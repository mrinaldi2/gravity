//! Showing a bot's browser to a client. A connection watches one bot at a
//! time: it gets `browser_tabs` whenever the bot's tabs change and
//! `browser_frame` with each new screen of the tab it shows. That tab is the
//! one the bot used last, unless the owner picked another to look at. Every
//! connection watching the same browser reads the same shared stream (see
//! `streams`), so the desktop and a phone cost one screencast, not two.
//!
//! Frames do not queue with the connection's other traffic: each goes into a
//! one-frame slot the connection's writer empties when the link has room, so
//! a newer frame replaces one not yet sent. A slow link skips frames, and a
//! reply or a push never waits behind more than the one frame being written.

use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch;

use crate::app::AppState;

use super::streams::{Frame, TabStream};
use super::BotBrowser;

/// Where a watch sends what it shows: tab lists in order with everything
/// else on the connection, frames into the newest-wins slot.
#[derive(Clone)]
pub struct Viewer {
    pushes: UnboundedSender<Value>,
    frames: watch::Sender<Option<Value>>,
}

impl Viewer {
    /// A viewer sending through `pushes`, and the slot its frames wait in.
    pub fn new(pushes: UnboundedSender<Value>) -> (Self, watch::Receiver<Option<Value>>) {
        let (frames, slot) = watch::channel(None);
        (Self { pushes, frames }, slot)
    }

    /// Sends a push in order; false once the connection is gone.
    pub fn push(&self, push: Value) -> bool {
        self.pushes.send(push).is_ok()
    }

    /// Puts a frame in the slot, replacing one not yet sent; false once the
    /// connection is gone.
    pub fn frame(&self, frame: Value) -> bool {
        self.frames.send(Some(frame)).is_ok()
    }

    /// Empties the slot, so a frame of a watch that ended is not sent.
    pub fn clear(&self) {
        self.frames.send_replace(None);
    }
}

/// The profile directory of a bot's browser, when the bot runs here.
pub fn profile(app: &AppState, bot: &bus::Bot) -> Option<std::path::PathBuf> {
    if bot.is_linked() {
        return None;
    }
    let project = app.db.get_project(&bot.project_id).ok()??;
    let root = crate::paths::bot_dir(&app.cfg, &project.dir_name, &bot.dir_name);
    Some(BotBrowser::new(&root).profile())
}

/// Passes the owner's mouse and keyboard to a tab of a bot's browser that
/// someone here is watching, or to its machine for a linked bot.
pub fn input(app: &AppState, bot: &bus::Bot, tab_id: &str, event: &Value) -> anyhow::Result<()> {
    if bot.is_linked() {
        return crate::peer::browser::input(app, bot, tab_id, event);
    }
    let commands = super::input::commands(event)?;
    app.browsers.input(&bot.id, tab_id, commands)
}

/// Watches a bot's browser for one connection until the task is aborted,
/// through the stream every viewer of that browser shares.
pub async fn watch(app: Arc<AppState>, bot: bus::Bot, chosen: Option<String>, out: Viewer) {
    if bot.is_linked() {
        // Its browser is on its machine, which streams it here.
        crate::peer::browser::watch(app, bot, chosen, out).await;
        return;
    }
    let Some(profile) = profile(&app, &bot) else {
        return;
    };
    let stream = app.browsers.bot(&bot.id, profile);
    let mut tabs = stream.tabs();
    let mut showing: Option<(String, Arc<TabStream>, watch::Receiver<Option<Frame>>)> = None;
    loop {
        let frame_changed = async {
            match &mut showing {
                Some((_, _, frames)) => frames.changed().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            changed = tabs.changed() => {
                if changed.is_err() {
                    break;
                }
                let Some(open) = tabs.borrow_and_update().clone() else {
                    continue;
                };
                let active = chosen
                    .as_ref()
                    .and_then(|id| open.iter().find(|t| &t.id == id))
                    .or_else(|| open.first())
                    .cloned();
                let listing = json!({
                    "type": "browser_tabs", "bot_id": bot.id, "open": !open.is_empty(),
                    "active": active.as_ref().map(|t| t.id.clone()),
                    "following": chosen.is_none(),
                    "tabs": open.iter().map(|t| json!({ "id": t.id, "title": t.title, "url": t.url })).collect::<Vec<_>>()
                });
                if !out.push(listing) {
                    break;
                }
                let on_show = showing.as_ref().map(|(id, _, _)| id.clone());
                if on_show != active.as_ref().map(|t| t.id.clone()) {
                    showing = active.map(|tab| {
                        let screen = stream.screen(&tab);
                        let frames = screen.frames();
                        (tab.id, screen, frames)
                    });
                }
            }
            changed = frame_changed => {
                let Some((tab_id, _, frames)) = &mut showing else {
                    continue;
                };
                if changed.is_err() {
                    // The tab's screencast ended (the tab closed, or Chrome
                    // dropped it); the next tab list decides what to show.
                    showing = None;
                    tabs.mark_changed();
                    continue;
                }
                let Some(frame) = frames.borrow_and_update().clone() else {
                    continue;
                };
                let sent = out.frame(json!({
                    "type": "browser_frame", "bot_id": bot.id, "tab_id": tab_id,
                    "data": &*frame.data, "width": frame.width, "height": frame.height
                }));
                if !sent {
                    break;
                }
            }
        }
    }
}
