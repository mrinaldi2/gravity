//! What a paired phone says it runs (H-241, UX-043 §5): the app version and
//! build its hello carries (H-230), kept per device in `meta` under
//! `app_version:<device id>` with when it last said so.
//!
//! Display only. A phone can say anything, so a reported version never
//! creates or confirms a deploy record: those stay with the tester's and
//! DevOps' confirmation.

use bus::contract::home::InstallDevice;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app::AppState;
use crate::attention::timestamp;
use crate::db::Db;

/// The longest version or build kept, in characters.
const MAX_LEN: usize = 32;

/// A device's latest report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reported {
    pub version: String,
    /// Empty when the hello carried no usable build.
    #[serde(default)]
    pub build: String,
    pub seen_at: DateTime<Utc>,
}

impl Reported {
    /// "0.6.1 (12)", or "0.6.1" without a build.
    pub fn label(&self) -> String {
        if self.build.is_empty() || self.build == self.version {
            self.version.clone()
        } else {
            format!("{} ({})", self.version, self.build)
        }
    }
}

/// A reported string as kept: trimmed, 1 to 32 printable ASCII characters,
/// or nothing.
fn clean(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    let printable = text.chars().all(|c| c == ' ' || c.is_ascii_graphic());
    (printable && !text.is_empty() && text.chars().count() <= MAX_LEN).then(|| text.to_string())
}

fn key(device_id: &str) -> String {
    format!("app_version:{device_id}")
}

/// Keeps what a paired device's hello says it runs. A hello without a usable
/// version leaves the last report as it was.
pub fn note(db: &Db, device_id: &str, hello: &Value) -> anyhow::Result<()> {
    let Some(version) = clean(hello.get("app_version")) else {
        return Ok(());
    };
    let reported = Reported {
        version,
        build: clean(hello.get("app_build")).unwrap_or_default(),
        seen_at: bus::now(),
    };
    db.set_meta(&key(device_id), &serde_json::to_string(&reported)?)
}

/// The device's latest report, if it has made one.
pub fn reported(db: &Db, device_id: &str) -> anyhow::Result<Option<Reported>> {
    Ok(db
        .get_meta(&key(device_id))?
        .and_then(|s| serde_json::from_str(&s).ok()))
}

/// The paired devices that may be sent a link, last seen first, with what
/// each last said it runs.
pub fn install_devices(app: &AppState) -> anyhow::Result<Vec<InstallDevice>> {
    let mut list: Vec<_> = app
        .db
        .list_devices()?
        .into_iter()
        .filter(|d| d.revoked_at.is_none())
        .collect();
    list.sort_by_key(|d| std::cmp::Reverse(d.last_seen_at));
    list.into_iter()
        .map(|d| {
            let reported = reported(&app.db, &d.id)?;
            Ok(InstallDevice {
                connected: app.live_devices.connected(&d.id),
                last_seen_at: d.last_seen_at.map(timestamp),
                app_version: reported.as_ref().map(Reported::label).unwrap_or_default(),
                app_version_seen_at: reported.map(|r| timestamp(r.seen_at)),
                device_id: d.id,
                name: d.name,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn only_short_printable_strings_are_kept() {
        let long = "1".repeat(MAX_LEN + 1);
        let at_limit = "1".repeat(MAX_LEN);
        for (given, kept) in [
            (json!(" 0.6.1 "), Some("0.6.1")),
            (json!("0.6.1-beta 2"), Some("0.6.1-beta 2")),
            (json!(at_limit), Some(at_limit.as_str())),
            (json!(long), None),
            (json!(""), None),
            (json!("   "), None),
            (json!("0.6\n.1"), None),
            (json!("0.6.1\u{202e}"), None),
            (json!("0.6.1\u{0}"), None),
            (json!("é"), None),
            (json!(12), None),
            (Value::Null, None),
        ] {
            assert_eq!(clean(Some(&given)).as_deref(), kept, "{given}");
        }
        assert_eq!(clean(None), None);
    }

    #[test]
    fn the_label_names_the_build_when_there_is_one() {
        let at = bus::now();
        let r = |version: &str, build: &str| Reported {
            version: version.into(),
            build: build.into(),
            seen_at: at,
        };
        assert_eq!(r("0.6.1", "12").label(), "0.6.1 (12)");
        assert_eq!(r("0.6.1", "").label(), "0.6.1");
        assert_eq!(r("0.6.1", "0.6.1").label(), "0.6.1");
    }
}
