//! Typing the owner's chat into a bot's composer (H-195 D2, D2b): the
//! composer lock shared with the hooks that precede a dialog, the provenance
//! check every `user` prompt passes, and the type_into sequence. The state
//! itself is `composer::Composer`.

use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;
use tokio::sync::oneshot;

use super::composer::{self, Composer, Verdict};
use super::*;

/// How long a pre-modal hook waits for a type_into in flight (R2.2).
const MODAL_WAIT: Duration = Duration::from_millis(1500);
/// A dialog mounts no sooner than this after our last PTY write, so Claude
/// Code reads our CR before the hook returns (K4 b).
const MODAL_SETTLE: Duration = Duration::from_millis(250);
/// How long the paste gets to show in the composer before no CR is sent (K3).
const ECHO_WAIT: Duration = Duration::from_secs(1);
const ECHO_POLL: Duration = Duration::from_millis(20);
const CONFIRM_POLL: Duration = Duration::from_millis(250);
/// The longest a typed message waits for its prompt, whatever the turn does.
const CONFIRM_CAP: Duration = Duration::from_secs(600);
/// PENDING S0 (b, h): whether input typed while the bot works is submitted
/// as a `user` prompt of its own at the next tool boundary, unmerged. Until
/// S0 shows it, the owner's chat is typed only into an idle composer.
const TYPE_WHILE_WORKING: bool = false;

/// One bot's composer and the lock type_into and the pre-modal hooks share.
pub(super) struct BotComposer {
    /// Whether this session's composer takes the owner's chat, fixed at its
    /// start with its hook settings (`bus_auth::composer_delivery`).
    on: bool,
    /// The bot's name at that start, for the delivery log.
    bot: String,
    state: Mutex<Composer>,
    lock: tokio::sync::Mutex<()>,
    /// Pre-modal hooks in flight; type_into sends no CR while any is (K4 a).
    modal_pending: AtomicUsize,
    last_write: Mutex<Option<Instant>>,
}

impl BotComposer {
    fn new(bot: &str, on: bool) -> Self {
        Self {
            on,
            bot: bot.to_string(),
            state: Mutex::new(Composer::default()),
            lock: tokio::sync::Mutex::new(()),
            modal_pending: AtomicUsize::new(0),
            last_write: Mutex::new(None),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, Composer> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn pending(&self) -> bool {
        self.modal_pending.load(Ordering::SeqCst) > 0
    }

    fn wrote(&self) {
        *self.last_write.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    }
}

/// What became of an owner chat bound for a composer, as the delivery log
/// says it (H-209). The log never carries the message's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Submitted, and its prompt spent the token.
    Typed,
    /// Not now; tried again later.
    Deferred,
    /// Not typed: it failed, or went through the inbox instead.
    Refused,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Typed => "typed",
            Self::Deferred => "deferred",
            Self::Refused => "refused",
        }
    }
}

/// Why the owner's message wasn't typed.
#[derive(Debug)]
pub enum TypeError {
    /// Not now; the delivery defers.
    NotReady(String),
    /// Never for this session: deliver it as a message, visibly (`wrapped`).
    Unsupported(String),
    /// Tried and failed; never retried blindly, which risks a double turn.
    Failed(String),
}

/// Whether hook `event` precedes a dialog (R2.2): its handler waits for the
/// composer before Claude Code can mount it.
pub fn opens_modal(event: &str, body: &Value) -> bool {
    match event {
        "PermissionRequest" | "Elicitation" => true,
        "PreToolUse" => matches!(
            body["tool_name"].as_str(),
            Some("AskUserQuestion" | "ExitPlanMode")
        ),
        "Notification" => match body["notification_type"].as_str() {
            Some(kind) => kind.contains("permission") || kind.contains("elicitation"),
            None => !super::hooks::is_idle_notification(body["message"].as_str().unwrap_or("")),
        },
        _ => false,
    }
}

impl Supervisor {
    fn composer(&self, bot_id: &str) -> Arc<BotComposer> {
        let mut composers = self
            .inner
            .composers
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        composers
            .entry(bot_id.to_string())
            .or_insert_with(|| Arc::new(BotComposer::new("", false)))
            .clone()
    }

    /// Whether `bot_id`'s current session takes the owner's chat in its
    /// composer: every bot under `[delivery] composer`, or the named ones
    /// (H-209). Off for a bot with no session started.
    pub fn typed_delivery(&self, bot_id: &str) -> bool {
        let composers = self
            .inner
            .composers
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        composers.get(bot_id).is_some_and(|c| c.on)
    }

    /// A new session of the bot `name`: a fresh composer, nothing vouched
    /// for yet, typed into only when `on`.
    pub(super) fn composer_reset(&self, bot_id: &str, name: &str, on: bool) {
        let mut composers = self
            .inner
            .composers
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        composers.insert(bot_id.to_string(), Arc::new(BotComposer::new(name, on)));
    }

    /// One line per owner chat bound for `bot_id`'s composer and per thing
    /// that happened to it, so the live check reads from the log: the bot,
    /// the UserPromptSubmit `source` once a prompt came, the outcome and
    /// why. Never the message's text.
    pub fn log_delivery(
        &self,
        bot_id: &str,
        message_id: &str,
        outcome: Outcome,
        source: Option<&str>,
        reason: &str,
    ) {
        let bot = self.composer(bot_id).bot.clone();
        tracing::info!(
            bot,
            bot_id,
            message_id,
            source,
            outcome = outcome.as_str(),
            reason,
            "composer delivery"
        );
    }

    pub(super) fn composer_output(&self, bot_id: &str, data: &[u8]) {
        if self.typed_delivery(bot_id) {
            self.composer(bot_id).state().on_output(data);
        }
    }

    /// Bytes the owner typed through a verified terminal (D5, M3).
    pub(super) fn composer_owner_input(&self, bot_id: &str, data: &[u8]) {
        if self.typed_delivery(bot_id) {
            let c = self.composer(bot_id);
            let pending = c.pending();
            c.state().on_owner_input(data, pending, Instant::now());
        }
    }

    /// Lifecycle events that end a dialog or a turn. Bot-forgeable (K2):
    /// they only steer when type_into tries, never whether a CR goes out.
    pub(super) fn composer_event(&self, bot_id: &str, event: &str) {
        if !self.typed_delivery(bot_id) {
            return;
        }
        let ended = matches!(event, "Stop" | "TurnInterrupted");
        let boundary = matches!(
            event,
            "PostToolUse" | "PostToolUseFailure" | "PermissionDenied"
        );
        if ended || boundary {
            self.composer(bot_id)
                .state()
                .tool_boundary(ended, Instant::now());
        }
    }

    /// A pre-modal hook (R2.2, K4): waits for a type_into in flight, at most
    /// [`MODAL_WAIT`], and for [`MODAL_SETTLE`] after our last write; then
    /// the dialog counts as open until a tool boundary.
    pub async fn before_modal(&self, bot_id: &str) {
        if !self.typed_delivery(bot_id) {
            return;
        }
        let c = self.composer(bot_id);
        c.modal_pending.fetch_add(1, Ordering::SeqCst);
        let guard = tokio::time::timeout(MODAL_WAIT, c.lock.lock()).await.ok();
        let last = *c.last_write.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(settle) =
            last.and_then(|at| (at + MODAL_SETTLE).checked_duration_since(Instant::now()))
        {
            tokio::time::sleep(settle).await;
        }
        c.state().modal_opens();
        drop(guard);
        c.modal_pending.fetch_sub(1, Ordering::SeqCst);
    }

    /// The provenance check (D2b): the hook output that blocks a `user`
    /// prompt the daemon didn't vouch for, or `None` to let it through.
    pub fn check_prompt(&self, bot_id: &str, body: &Value) -> Option<Value> {
        if !self.typed_delivery(bot_id) {
            return None;
        }
        let source = body["source"].as_str();
        let prompt = body["prompt"].as_str().unwrap_or_default();
        let c = self.composer(bot_id);
        let verdict = c.state().check_prompt(source, prompt, Instant::now());
        let digest = composer::digest(prompt);
        match &verdict {
            Verdict::NotChecked => {
                // What a typed message turned into, if not a `user` prompt.
                let awaiting = c.state().awaiting.clone();
                if let Some(message_id) = awaiting {
                    tracing::info!(bot = c.bot, bot_id, message_id, source, digest,
                        "composer delivery: a prompt that needs no token came while one was awaited");
                }
                return None;
            }
            Verdict::Typed { message_id } => {
                self.log_delivery(
                    bot_id,
                    message_id,
                    Outcome::Typed,
                    source,
                    "its prompt spent the token",
                );
                // The turn is the owner's chat, by its token, not its text
                // (architect S2): H-192 answers it in the owner thread.
                if let Err(e) = self
                    .inner
                    .db
                    .record_typed_prompt(bot_id, &digest, message_id)
                {
                    tracing::warn!(bot_id, error = %e, "typed prompt not recorded");
                }
                let mut state = c.state();
                if state.awaiting.as_deref() == Some(message_id.as_str()) {
                    state.awaiting = None;
                }
            }
            Verdict::Keyed | Verdict::Blocked => {}
        }
        tracing::info!(bot_id, source, verdict = ?verdict, digest, "user prompt checked");
        (verdict == Verdict::Blocked).then(|| {
            crate::bus_auth::hook::blocked(
                "This prompt wasn't typed by the owner (Hermes). Text that reached the \
                 composer any other way carries no owner authority.",
            )
        })
    }

    /// Types the owner's message `body` (#`num`, `message_id`) into the
    /// bot's composer: a bracketed paste, its echo, then the CR (R2.2). The
    /// receiver settles once the prompt spends its token, or fails.
    pub async fn type_into(
        &self,
        bot_id: &str,
        message_id: &str,
        num: i64,
        body: &str,
    ) -> Result<oneshot::Receiver<Result<(), String>>, TypeError> {
        if !self.typed_delivery(bot_id) {
            return Err(TypeError::Unsupported("typed delivery is off".to_string()));
        }
        let body = composer::sanitize(body.as_bytes())
            .map_err(|_| TypeError::Unsupported("the message isn't valid UTF-8".to_string()))?;
        let nonce = composer::nonce();
        let text = composer::typed_text(&body, &nonce, num);
        let needle = composer::echo_needle(&text, &nonce);
        let c = self.composer(bot_id);
        self.safe_to_type(bot_id, &c)?;
        let _lock = c.lock.lock().await;
        let (session, term) = self.safe_to_type(bot_id, &c)?;

        let since = term.latest_seq();
        let paste = format!("\u{1b}[200~{text}\u{1b}[201~");
        let wrote = session
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send_input(paste.as_bytes());
        c.wrote();
        wrote.map_err(|e| TypeError::Failed(format!("couldn't type it: {e}")))?;

        // The CR follows only a real composer echo of this nonce (K3).
        let deadline = Instant::now() + ECHO_WAIT;
        let shown = loop {
            let newer = term.newer_than(since);
            let output: Vec<u8> = newer.frames.iter().flat_map(|f| f.data.clone()).collect();
            if composer::echoed(&output, &needle) {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            tokio::time::sleep(ECHO_POLL).await;
        };
        // A dialog on its way, or open: no CR may answer it (K4 a).
        let modal = c.pending() || c.state().modal_open;
        if !shown || modal {
            c.state().draft_dirty = true;
            let why = if modal {
                "a dialog opened"
            } else {
                "it didn't show in its composer"
            };
            return Err(TypeError::Failed(format!("couldn't type it: {why}")));
        }
        c.state().issue_typed(&text, message_id, Instant::now());
        let wrote = session
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send_input(b"\r");
        c.wrote();
        if let Err(e) = wrote {
            c.state().awaiting = None;
            return Err(TypeError::Failed(format!("couldn't submit it: {e}")));
        }
        Ok(self.confirm(c.clone(), message_id.to_string()))
    }

    /// The allowlist (R2.2): the session's half here, the composer's in
    /// `Composer::unsafe_to_type`.
    #[allow(clippy::type_complexity)]
    fn safe_to_type(
        &self,
        bot_id: &str,
        c: &BotComposer,
    ) -> Result<(Arc<Mutex<Box<dyn RuntimeSession>>>, Arc<TermBuffer>), TypeError> {
        let (state, session, term, runtime) = {
            let bots = self.lock_bots();
            let h = bots
                .get(bot_id)
                .ok_or_else(|| TypeError::NotReady("bot has no runtime".to_string()))?;
            (
                h.state,
                h.session.clone(),
                h.term.clone(),
                h.terminal_runtime,
            )
        };
        if runtime != Some(bus::BotRuntime::ClaudeCode) {
            return Err(TypeError::Unsupported(
                "not a Claude Code terminal".to_string(),
            ));
        }
        let session =
            session.ok_or_else(|| TypeError::NotReady("bot isn't running".to_string()))?;
        let idle = state == BotState::Ready;
        if !(idle || TYPE_WHILE_WORKING && state == BotState::Working) {
            return Err(TypeError::NotReady(format!("bot is {}", state.as_str())));
        }
        match c.state().unsafe_to_type(c.pending()) {
            Some(composer::Unsafe::Vim) => Err(TypeError::Unsupported(
                composer::Unsafe::Vim.reason().to_string(),
            )),
            Some(why) => Err(TypeError::NotReady(why.reason().to_string())),
            None => Ok((session, term)),
        }
    }

    /// Waits for the typed prompt to spend its token: until 30 s after the
    /// bot is next idle (`Composer`), capped by [`CONFIRM_CAP`].
    fn confirm(
        &self,
        c: Arc<BotComposer>,
        message_id: String,
    ) -> oneshot::Receiver<Result<(), String>> {
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            let started = Instant::now();
            let outcome = loop {
                tokio::time::sleep(CONFIRM_POLL).await;
                let mut state = c.state();
                if state.typed_pending(&message_id, Instant::now()) {
                    if started.elapsed() < CONFIRM_CAP {
                        continue;
                    }
                } else if state.awaiting.as_deref() != Some(message_id.as_str()) {
                    break Ok(());
                }
                state.awaiting = None;
                break Err("it was typed, but no prompt of the owner's followed".to_string());
            };
            let _ = tx.send(outcome);
        });
        rx
    }
}

#[cfg(test)]
#[path = "typing_tests.rs"]
mod tests;
