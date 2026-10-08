//! The one toast for bots the watchdog gave up on (H-041, H-183).

use super::*;

/// H-041 (ARCH-R20 F5): a mass restart where several bots never connect
/// raises one toast that names them, not one each.
#[tokio::test(flavor = "multi_thread")]
async fn bots_that_dont_connect_together_are_told_in_one_toast() {
    let f = Fixture::new(|_| {});
    let mut pushes = f.sup.inner.events.subscribe_push();
    let ids: Vec<String> = ["alice", "bob", "carol"]
        .iter()
        .map(|name| f.bot(name, bus::BotRuntime::ClaudeCode))
        .collect();
    f.tick_until("the watchdog to give up on all three", |f| {
        ids.iter().all(|id| gave_up(&f.sup, id))
    })
    .await;
    f.sup.reconcile();
    let mut toasts = Vec::new();
    while let Ok(push) = pushes.try_recv() {
        if let Push::Notify { title, body, .. } = push {
            toasts.push((title, body));
        }
    }
    assert_eq!(toasts.len(), 1, "{toasts:?}");
    assert_eq!(toasts[0].0, "3 bots didn't connect");
    assert!(
        toasts[0].1.starts_with("alice, bob and carol"),
        "{toasts:?}"
    );
}

#[test]
fn the_toast_names_one_bot_or_counts_many() {
    let bots = |names: &[&str]| -> Vec<(String, bool)> {
        names.iter().map(|n| ((*n).to_string(), false)).collect()
    };
    assert_eq!(gave_up_toast(&bots(&["alice"])).0, "alice didn't connect");
    assert_eq!(
        gave_up_toast(&[("w-1".to_string(), true)]).1,
        "Its task was cancelled and its worker slot freed."
    );
    let (title, body) = gave_up_toast(&bots(&["a", "b", "c", "d", "e", "f"]));
    assert_eq!(title, "6 bots didn't connect");
    assert!(body.starts_with("a, b, c and 3 more:"), "{body}");
    let mixed = [("a".to_string(), false), ("w".to_string(), true)];
    assert!(gave_up_toast(&mixed).1.contains("Workers among them"));
}

/// H-183: a bot the watchdog is restarting, between its old session and the
/// next, is still connecting: the toast waits for it rather than going out
/// without it (a split toast under load).
#[tokio::test(flavor = "multi_thread")]
async fn a_bot_between_watchdog_restarts_holds_the_toast() {
    let f = Fixture::new(|_| {});
    let mut pushes = f.sup.inner.events.subscribe_push();
    {
        let mut restarting = BotHandle::new(f.sup.inner.cfg.scrollback_bytes);
        restarting.connect_restarts = 1;
        f.sup.lock_bots().insert("bob".into(), restarting);
        let mut gave_up = f.sup.inner.gave_up.lock().expect("gave up");
        gave_up.bots.push(("alice".into(), false));
        gave_up.since = Some(Instant::now());
    }

    f.sup.tell_gave_up();
    assert!(pushes.try_recv().is_err(), "told before bob was done");

    f.sup
        .lock_bots()
        .get_mut("bob")
        .expect("bob")
        .connect_gave_up = true;
    f.sup.tell_gave_up();
    assert!(matches!(pushes.try_recv(), Ok(Push::Notify { .. })));
}
