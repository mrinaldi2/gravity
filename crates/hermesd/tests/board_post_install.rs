//! Acceptance criteria provable only after install (H-116): flagged, they
//! don't hold a package back, but nothing reaches Done until they are
//! ticked. The lead ticks with evidence; a move into Verify lists what is
//! still unticked.

mod common;

use common::board::version;
use common::releases::{releases, rule};
use common::tasks::{drain_until, error_text};
use common::team::{get, item, team, OWNER};
use common::WsClient;
use hermesd::db::{ItemEdit, Write};
use serde_json::json;

fn with_criteria(db: &hermesd::db::Db, id: &str, texts: &[&str]) {
    let current = db.get_item(id).unwrap().unwrap();
    let texts: Vec<String> = texts.iter().map(|t| (*t).to_string()).collect();
    let edit = ItemEdit {
        acceptance_criteria: Some(&texts),
        ..ItemEdit::default()
    };
    let written = db.update_item(id, current.version, &edit, &OWNER).unwrap();
    assert!(matches!(written, Write::Done(_)));
}

#[tokio::test]
async fn a_post_install_criterion_skips_submit_and_holds_done() {
    let mut r = releases(1).await;
    let item = r.items[0].clone();
    with_criteria(&r.pair.d.app.db, &item, &["builds", "survives a reboot"]);
    let tester = &mut r.bots[2];
    let current = get(tester, &item).await;
    tester
        .call(
            "item_check_ac",
            json!({"id": item, "expected_version": version(&current), "index": 0, "result": "pass"}),
        )
        .await;

    // Unflagged, the second criterion holds the package back.
    let ops = &mut r.bots[1];
    let created = ops
        .call("release_create", json!({"name": "0.16.3", "items": [item]}))
        .await;
    let id = created["release"]["id"].as_str().unwrap().to_string();
    ops.call(
        "release_attach_build",
        json!({"release_id": id, "platform": "daemon", "version": "0.16.3",
               "artifact": "/builds/hermesd", "sha256": "a".repeat(64)}),
    )
    .await;
    r.passed(&id).await;
    let raw = r.bots[1]
        .call_raw("release_submit", json!({"release_id": id}))
        .await;
    assert!(error_text(&raw).contains("not checked"), "{raw}");

    // Flagged post-install by the lead, it doesn't.
    let lead = &mut r.bots[0];
    let current = get(lead, &item).await;
    let flagged = lead
        .call(
            "item_flag_ac",
            json!({"id": item, "expected_version": version(&current), "index": 1,
                   "post_install": true}),
        )
        .await;
    assert_eq!(
        flagged["item"]["acceptance_criteria"][1]["post_install"],
        true
    );
    let release = r.bots[1]
        .call("release_submit", json!({"release_id": id}))
        .await["release"]
        .clone();
    // The owner sees what is approved unproven (S1).
    assert_eq!(
        release["post_install"],
        json!([{"item_id": item, "index": 1, "text": "survives a reboot",
                "checked": false, "checked_by": null}]),
        "{release}"
    );
    let mut owner = WsClient::connect(&r.pair.d).await;
    rule(
        &mut owner,
        &release,
        json!([{"item_id": item, "verdict": "ship"}]),
    )
    .await;
    r.bots[1]
        .call(
            "release_deploy",
            json!({"release_id": id, "machine": "mac"}),
        )
        .await;

    // Installed everywhere, but the post-install criterion is open: the
    // confirm is refused and the deployment stays open for later.
    let confirm = json!({"release_id": id, "machine": "mac", "result": "ok", "smoke": "pass"});
    let raw = r.bots[2].call_raw("deploy_confirm", confirm.clone()).await;
    let text = error_text(&raw);
    assert!(
        text.contains("post-install") && text.contains("survives a reboot"),
        "{raw}"
    );
    assert_eq!(r.column(&item), "deploying");

    // In a submitted package the flag stays (ARCH-R53 M1).
    let lead = &mut r.bots[0];
    let current = get(lead, &item).await;
    let raw = lead
        .call_raw(
            "item_flag_ac",
            json!({"id": item, "expected_version": version(&current), "index": 1,
                   "post_install": false}),
        )
        .await;
    assert!(error_text(&raw).contains("ac.post_install_locked"), "{raw}");

    // The lead ticks it on its own evidence: the package's history says so
    // (S2), and the tester holding the deploy is told to confirm again (S3).
    let checked = lead
        .call(
            "item_check_ac",
            json!({"id": item, "expected_version": version(&current), "index": 1,
                   "result": "pass", "evidence": "rebooted mac, still serving"}),
        )
        .await;
    assert_eq!(checked["item"]["acceptance_criteria"][1]["checked"], true);
    let told = drain_until(&mut r.bots[2], "send it again").await;
    assert!(!told.is_empty());
    let got = r.bots[1]
        .call("release_get", json!({"release_id": id}))
        .await;
    let events = got["release"]["events"].as_array().expect("events");
    let tick = events
        .iter()
        .find(|e| e["kind"] == "lead_ticked")
        .unwrap_or_else(|| panic!("{got}"));
    assert_eq!(tick["note"], "rebooted mac, still serving");
    assert_eq!(tick["detail"]["text"], "survives a reboot");
    let done = r.bots[2].call("deploy_confirm", confirm).await;
    assert_eq!(done["release"]["status"], "deployed", "{done}");
    assert_eq!(r.column(&item), "done");
}

#[tokio::test]
async fn the_lead_ticks_with_evidence_posted_as_a_comment() {
    let (pair, mut bots, project) = team(&["Team Lead", "Tester", "Desktop Dev"]).await;
    let id = item(&pair, &project, "verify", Some(&pair.ids[2]));
    with_criteria(&pair.d.app.db, &id, &["builds", "lint is clean"]);
    let [lead, tester, dev] = &mut bots[..] else {
        unreachable!()
    };

    let current = get(lead, &id).await;
    let check = json!({"id": id, "expected_version": version(&current), "index": 0,
                       "result": "pass"});
    let raw = lead.call_raw("item_check_ac", check.clone()).await;
    assert!(error_text(&raw).contains("ac.evidence"), "{raw}");
    let mut with_evidence = check;
    with_evidence["evidence"] = json!("CI run 812 is green");
    let checked = lead.call("item_check_ac", with_evidence).await;
    assert_eq!(checked["item"]["acceptance_criteria"][0]["checked"], true);
    let comments = pair.d.app.db.board_tx(|t| t.item_comments(&id)).unwrap();
    assert!(
        comments
            .iter()
            .any(|c| c.body.contains("by the lead") && c.body.contains("CI run 812")),
        "{comments:?}"
    );

    // A tester needs no evidence; a dev can't check at all.
    let check = json!({"id": id, "expected_version": version(&checked["item"]), "index": 1,
                       "result": "pass"});
    let raw = dev.call_raw("item_check_ac", check.clone()).await;
    assert!(error_text(&raw).contains("role"), "{raw}");
    let checked = tester.call("item_check_ac", check).await;
    assert_eq!(checked["item"]["acceptance_criteria"][1]["checked"], true);
    assert_eq!(
        pair.d
            .app
            .db
            .board_tx(|t| t.item_comments(&id))
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn a_move_into_verify_lists_the_unticked_criteria() {
    let (pair, mut bots, project) = team(&["Team Lead", "Desktop Dev", "reviewer"]).await;
    let id = item(&pair, &project, "review", Some(&pair.ids[1]));
    with_criteria(&pair.d.app.db, &id, &["builds", "survives a reboot"]);
    let [lead, dev, reviewer] = &mut bots[..] else {
        unreachable!()
    };

    // The assignee flags before Verify, not after.
    let current = get(dev, &id).await;
    let flagged = dev
        .call(
            "item_flag_ac",
            json!({"id": id, "expected_version": version(&current), "index": 1,
                   "post_install": true}),
        )
        .await;
    lead.call(
        "send_message",
        json!({"to": "reviewer", "kind": "task", "body": "review it", "item": id}),
    )
    .await;
    let moved = reviewer
        .call(
            "item_move",
            json!({"id": id, "to": "verify", "expected_version": version(&flagged["item"])}),
        )
        .await;
    assert_eq!(moved["item"]["column_key"], "verify", "{moved}");
    let unticked = moved["unticked_ac"].as_array().expect("unticked_ac");
    assert_eq!(unticked.len(), 2, "{moved}");
    assert_eq!(unticked[1]["post_install"], true);
    assert!(moved["note"].as_str().unwrap().contains("item_check_ac"));

    let raw = dev
        .call_raw(
            "item_flag_ac",
            json!({"id": id, "expected_version": version(&moved["item"]), "index": 1,
                   "post_install": false}),
        )
        .await;
    assert!(error_text(&raw).contains("role.not_allowed"), "{raw}");
}
