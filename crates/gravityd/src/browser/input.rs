//! The owner's mouse and keyboard on a bot's browser: what a client sends
//! (`browser_input`), checked and turned into DevTools `Input` calls. This is
//! how the owner signs a bot in, or gets it past a page it cannot handle.

use serde_json::{json, Value};

use super::cdp::Command;

/// The most text one paste may carry.
const MAX_TEXT: usize = 100_000;

/// The DevTools calls for one input event from a client. Coordinates are in
/// the page's CSS pixels, the size `browser_frame` reports.
pub fn commands(event: &Value) -> anyhow::Result<Vec<Command>> {
    let modifiers = event["modifiers"].as_u64().unwrap_or(0) & 0b1111;
    let command = match event["kind"].as_str().unwrap_or_default() {
        "mouse" => {
            let kind = match event["action"].as_str().unwrap_or_default() {
                "down" => "mousePressed",
                "up" => "mouseReleased",
                "move" => "mouseMoved",
                other => anyhow::bail!("unknown mouse action '{other}'"),
            };
            let button = match event["button"].as_str().unwrap_or("none") {
                button @ ("left" | "middle" | "right" | "none") => button,
                other => anyhow::bail!("unknown mouse button '{other}'"),
            };
            let (x, y) = point(event)?;
            let clicks = event["clicks"].as_u64().unwrap_or(1).clamp(1, 3);
            (
                "Input.dispatchMouseEvent",
                json!({ "type": kind, "x": x, "y": y, "button": button,
                        "clickCount": clicks, "modifiers": modifiers }),
            )
        }
        "wheel" => {
            let (x, y) = point(event)?;
            (
                "Input.dispatchMouseEvent",
                json!({ "type": "mouseWheel", "x": x, "y": y,
                        "deltaX": finite(&event["dx"])?, "deltaY": finite(&event["dy"])?,
                        "modifiers": modifiers }),
            )
        }
        "key" => {
            let key = event["key"].as_str().unwrap_or_default();
            anyhow::ensure!(!key.is_empty() && key.len() <= 32, "'key' is required");
            let code = event["code"].as_str().unwrap_or_default();
            let key_code = event["key_code"].as_u64().unwrap_or(0).min(255);
            let mut params = json!({
                "key": key, "code": code, "modifiers": modifiers,
                "windowsVirtualKeyCode": key_code, "nativeVirtualKeyCode": key_code
            });
            match event["action"].as_str().unwrap_or_default() {
                "down" => match event["text"].as_str().filter(|t| !t.is_empty()) {
                    // A key that types: Chrome inserts the text as it would
                    // for a real keystroke, events and all.
                    Some(text) => {
                        anyhow::ensure!(text.chars().count() <= 4, "a key types a character");
                        params["type"] = json!("keyDown");
                        params["text"] = json!(text);
                        params["unmodifiedText"] = json!(text);
                    }
                    None => params["type"] = json!("rawKeyDown"),
                },
                "up" => params["type"] = json!("keyUp"),
                other => anyhow::bail!("unknown key action '{other}'"),
            }
            ("Input.dispatchKeyEvent", params)
        }
        "text" => {
            let text = event["text"].as_str().unwrap_or_default();
            anyhow::ensure!(text.len() <= MAX_TEXT, "too much text to paste");
            ("Input.insertText", json!({ "text": text }))
        }
        other => anyhow::bail!("unknown input kind '{other}'"),
    };
    Ok(vec![command])
}

fn point(event: &Value) -> anyhow::Result<(f64, f64)> {
    Ok((finite(&event["x"])?.max(0.0), finite(&event["y"])?.max(0.0)))
}

fn finite(value: &Value) -> anyhow::Result<f64> {
    value
        .as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| anyhow::anyhow!("coordinates must be numbers"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(event: Value) -> Command {
        let mut commands = commands(&event).expect("valid");
        assert_eq!(commands.len(), 1);
        commands.remove(0)
    }

    #[test]
    fn a_click_is_a_press_at_the_page_point() {
        let (method, params) = one(json!({
            "kind": "mouse", "action": "down", "x": 120.5, "y": -3, "button": "left", "clicks": 2
        }));
        assert_eq!(method, "Input.dispatchMouseEvent");
        assert_eq!(params["type"], "mousePressed");
        assert_eq!(params["x"], 120.5);
        assert_eq!(params["y"], 0.0);
        assert_eq!(params["clickCount"], 2);
    }

    #[test]
    fn a_typing_key_carries_its_text_and_others_do_not() {
        let (_, typed) = one(json!({
            "kind": "key", "action": "down", "key": "a", "code": "KeyA", "key_code": 65, "text": "a"
        }));
        assert_eq!(typed["type"], "keyDown");
        assert_eq!(typed["text"], "a");
        let (_, backspace) = one(json!({
            "kind": "key", "action": "down", "key": "Backspace", "code": "Backspace", "key_code": 8
        }));
        assert_eq!(backspace["type"], "rawKeyDown");
        assert_eq!(backspace["windowsVirtualKeyCode"], 8);
        assert!(backspace.get("text").is_none());
    }

    #[test]
    fn a_paste_is_inserted_text() {
        let (method, params) = one(json!({ "kind": "text", "text": "hunter2" }));
        assert_eq!(method, "Input.insertText");
        assert_eq!(params["text"], "hunter2");
    }

    #[test]
    fn nonsense_is_refused() {
        for event in [
            json!({ "kind": "mouse", "action": "down", "x": 1, "y": 1, "button": "thumb" }),
            json!({ "kind": "mouse", "action": "down", "x": "far", "y": 1 }),
            json!({ "kind": "key", "action": "down" }),
            json!({ "kind": "eval", "text": "1" }),
        ] {
            assert!(commands(&event).is_err(), "{event}");
        }
    }
}
