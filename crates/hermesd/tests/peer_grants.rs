//! Grants from an owner's ruling for a bot on a linked computer (H-163):
//! kept until that computer applies or refuses them, retried only while it
//! is offline or restarting, and every outcome said on the decision. Extras
//! a computer won't take from another one (ARCH-R51 S1c) are refused one by
//! one, each with where to set it instead, so the rest still apply.

mod common;

use bus::PermissionExtra;
use common::grants::{comments, drop_link, extras, grant, said};
use common::peers::{team_trusting, wait_until};
use common::WsClient;
use serde_json::json;

/// What Tester Win got on 0.16.4: install, quiesce and build_installers in
/// one ruling, all turned away together. Now install and quiesce apply, and
/// build_installers, which another computer can't grant, is refused on its
/// own with where to set it.
#[tokio::test]
async fn the_grantable_extras_apply_and_each_other_one_says_where_to_set_it() {
    let mut t = team_trusting().await;
    let id = grant(&mut t, &["install", "quiesce", "build_installers"], |_| {}).await;
    wait_until("the PC grants install and quiesce", || {
        extras(&t) == vec![PermissionExtra::Install, PermissionExtra::Quiesce]
    })
    .await;
    said(
        &t.mac,
        &id,
        "Granted on win: windev now has install, quiesce.",
    )
    .await;
    said(
        &t.mac,
        &id,
        "Not granted on win: windev's build_installers can't be granted from another \
         computer; set it on win or from your phone, in the app there under windev's permissions.",
    )
    .await;
    assert!(
        comments(&t.mac, &id)
            .iter()
            .any(|c| c.contains("windev: install, quiesce (sending to win)")),
        "{:?}",
        comments(&t.mac, &id)
    );
    assert!(t.mac.app.db.pending_peer_grants(None).unwrap().is_empty());
}

/// A ruling while the PC is away waits, says so, and lands when the link
/// comes back, when nobody changed the bot there meanwhile (CE-020 d).
#[tokio::test]
async fn a_grant_sent_while_the_pc_is_offline_applies_when_it_is_back() {
    let mut t = team_trusting().await;
    let id = grant(&mut t, &["install"], drop_link).await;
    said(&t.mac, &id, "Waiting for win").await;
    wait_until("the PC grants it once back", || {
        extras(&t) == vec![PermissionExtra::Install]
    })
    .await;
    said(&t.mac, &id, "Granted on win").await;
    assert!(t.mac.app.db.pending_peer_grants(None).unwrap().is_empty());
}

/// Nothing grantable from here: nothing is sent, and the decision says so.
#[tokio::test]
async fn a_grant_only_its_own_computer_may_give_is_never_sent() {
    let mut t = team_trusting().await;
    let id = grant(&mut t, &["publish"], |_| {}).await;
    said(&t.mac, &id, "Not granted on win: windev's publish").await;
    assert!(
        comments(&t.mac, &id)
            .iter()
            .any(|c| c.contains("windev: not granted here, see below")),
        "{:?}",
        comments(&t.mac, &id)
    );
    assert!(t.mac.app.db.pending_peer_grants(None).unwrap().is_empty());
    assert!(extras(&t).is_empty());
}

/// The PC itself turns away, one by one, any extra another computer can't
/// grant, and applies the rest: a ruling's computer on an older version, or
/// with a different list, can't widen what it gives here.
#[tokio::test]
async fn the_pc_refuses_each_extra_it_may_not_take_and_applies_the_rest() {
    let t = team_trusting().await;
    let frame = json!({"type": "grant_extras", "bot_id": t.windev_id,
                       "extras": ["install", "build_installers"], "decision": "d"});
    let answer = hermesd::decisions::grants::serve_grant(&t.win.app, &t.win_peer_id, &frame)
        .expect("install applies");
    assert_eq!(answer["extras"], json!(["install"]), "{answer}");
    assert_eq!(
        answer["refused"][0]["extra"], "build_installers",
        "{answer}"
    );
    assert_eq!(extras(&t), vec![PermissionExtra::Install]);

    let none = json!({"type": "grant_extras", "bot_id": t.windev_id,
                      "extras": ["publish"], "decision": "d"});
    let refused = hermesd::decisions::grants::serve_grant(&t.win.app, &t.win_peer_id, &none);
    assert!(refused.is_err(), "nothing grantable is refused outright");
}

/// The way to unblock today, before this ships: the PC's own owner grants
/// the extras in the app on the PC (the bot's permissions), as any local bot.
#[tokio::test]
async fn the_pcs_owner_can_grant_build_installers_there() {
    let t = team_trusting().await;
    let mut pc_owner = WsClient::connect(&t.win).await;
    let set = pc_owner
        .request(
            json!({"type": "set_bot_permission_extras", "bot_id": t.windev_id,
                        "extras": ["install", "quiesce", "build_installers"]}),
        )
        .await;
    assert_eq!(set["type"], "bot", "{set}");
    assert_eq!(
        extras(&t),
        vec![
            PermissionExtra::Install,
            PermissionExtra::Quiesce,
            PermissionExtra::BuildInstallers,
        ]
    );
}
