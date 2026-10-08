//! Installing an iOS package on a phone over the WebSocket (H-229, UX-043;
//! `hermes.home.v1` ReleaseInstall*): the desktop reads what a package
//! offers and sends the link to a paired device; the device reads its
//! waiting offer and dismisses it.

use serde_json::{json, Value};

use crate::board::release::model::Release;
use crate::board::release::phone;
use crate::decisions::{forbidden, not_found};

use super::Conn;

impl Conn {
    /// `{release_id, check_site?}`: the Install box's facts. With
    /// `check_site`, the build site is asked for the page first.
    pub(super) fn release_install(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let release = self.install_release(req)?;
        let info = phone::info(&self.app, &release)?;
        if req.get("check_site").and_then(Value::as_bool) != Some(true) || info.page_url.is_empty()
        {
            self.send(json!({ "type": "release_install", "req_id": req_id, "install": info }));
            return Ok(());
        }
        self.spawn_reply(req_id, move |req_id| async move {
            let mut info = info;
            info.site = Some(phone::check_site(&info.page_url).await);
            json!({ "type": "release_install", "req_id": req_id, "install": info })
        });
        Ok(())
    }

    /// `{release_id, device_id}`: the install link to a paired device.
    pub(super) fn release_send_to_device(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let release = self.install_release(req)?;
        let device_id = Self::str_field(req, "device_id")?;
        let offer = phone::send(&self.app, &release, device_id)?;
        self.send(json!({ "type": "install_offer", "req_id": req_id, "offer": offer }));
        Ok(())
    }

    /// This device's waiting offer; none on a connection that isn't a device.
    pub(super) fn install_offers(&self, req_id: &Value) -> anyhow::Result<()> {
        let offers = match &self.device_id {
            Some(id) => phone::waiting(&self.app, id)?.into_iter().collect(),
            None => Vec::new(),
        };
        self.send(json!({ "type": "install_offers", "req_id": req_id, "offers": offers }));
        Ok(())
    }

    /// `{release_id}`: Not now, from the device the offer was sent to.
    pub(super) fn install_offer_dismiss(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let device_id = self
            .device_id
            .as_deref()
            .ok_or_else(|| forbidden("only a paired device dismisses its install offer"))?;
        let release_id = Self::str_field(req, "release_id")?;
        phone::dismiss(&self.app, device_id, release_id)?;
        let offers: Vec<_> = phone::waiting(&self.app, device_id)?.into_iter().collect();
        self.send(json!({ "type": "install_offers", "req_id": req_id, "offers": offers }));
        Ok(())
    }

    /// The package a request names, on the board's home (B9).
    fn install_release(&self, req: &Value) -> anyhow::Result<Release> {
        let id = Self::str_field(req, "release_id")?;
        let release = self
            .app
            .db
            .board_read(|t| t.release(id))?
            .ok_or_else(|| not_found(format!("no release {id}")))?;
        if let Some(home) = self.app.board_mirror.home_peer(&release.project_id) {
            return Err(forbidden(format!(
                "This project's releases are on {}, which holds its board; install from there.",
                crate::peer::board::home_name(&self.app, &home)
            )));
        }
        Ok(release)
    }
}
