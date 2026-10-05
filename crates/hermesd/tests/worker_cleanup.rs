//! A retired worker's clone, scratch folder and build output are removed by
//! the daemon, which needs no bot permission to do it (H-109). The machine's
//! shared Cargo target goes once the last worker has retired. Commits no
//! remote has are bundled first (ARCH-R40).

mod common;

use common::peers::{bot_named, wait_until};
use common::repo::{book, git};
use hermesd::workers::bundle::SALVAGE_DIR;
use hermesd::workers::scratch::{remove_idle_target, shared_target, SCRATCH_DIR};
use serde_json::json;

#[tokio::test]
async fn a_retired_workers_clone_and_build_output_are_removed() {
    let mut b = book().await;
    let (_spawned, checkout, _) = b.spawn("ch-1").await;
    b.clone_as_worker(&checkout);
    let workspace = checkout.parent().expect("workspace").to_path_buf();
    let build = workspace.join(SCRATCH_DIR).join("other-clone/target/debug");
    std::fs::create_dir_all(&build).expect("build dir");
    std::fs::write(build.join("hermesd"), vec![0u8; 4096]).expect("build output");

    b.bus.call("cancel_worker", json!({ "name": "ch-1" })).await;
    wait_until("ch-1 retires", || bot_named(&b.d, &b.pid, "ch-1").is_none()).await;
    wait_until("its clone is removed", || {
        !checkout.exists() && !workspace.join(SCRATCH_DIR).exists()
    })
    .await;
}

#[tokio::test]
async fn the_shared_target_goes_when_the_last_worker_retires() {
    let mut b = book().await;
    let target = shared_target(&b.d.app.cfg.home);
    std::fs::create_dir_all(target.join("debug")).expect("target");
    let (_one, _, _) = b.spawn("ch-1").await;
    let (_two, _, _) = b.spawn("ch-2").await;

    remove_idle_target(&b.d.app).expect("checked");
    assert!(target.exists(), "kept while workers live");

    b.bus.call("cancel_worker", json!({ "name": "ch-1" })).await;
    wait_until("ch-1 retires", || bot_named(&b.d, &b.pid, "ch-1").is_none()).await;
    remove_idle_target(&b.d.app).expect("checked");
    assert!(target.exists(), "kept while ch-2 lives");

    b.bus.call("cancel_worker", json!({ "name": "ch-2" })).await;
    wait_until("ch-2 retires", || bot_named(&b.d, &b.pid, "ch-2").is_none()).await;
    remove_idle_target(&b.d.app).expect("checked");
    assert!(!target.exists(), "removed after the last worker");
}

#[tokio::test]
async fn commits_a_failed_push_left_are_bundled_before_the_clone_goes() {
    let mut b = book().await;
    let (_spawned, checkout, _) = b.spawn("ch-1").await;
    b.clone_as_worker(&checkout);
    std::fs::write(checkout.join("notes.md"), "Outline.\n").expect("write");
    git(&checkout, &["add", "notes.md"]);
    git(&checkout, &["commit", "-q", "-m", "outline"]);
    let gone = checkout.parent().expect("ws").join("no-such-remote");
    git(
        &checkout,
        &["remote", "set-url", "origin", gone.to_str().expect("utf8")],
    );
    let workspace = checkout.parent().expect("workspace").to_path_buf();

    b.bus.call("cancel_worker", json!({ "name": "ch-1" })).await;
    wait_until("its clone is removed", || !checkout.exists()).await;
    let bundle = workspace.join(SALVAGE_DIR).join("repo.bundle");
    assert!(bundle.is_file(), "the bundle exists once the clone is gone");
    assert!(
        !workspace.join(".gravity-salvaged").exists(),
        "a failed salvage is not marked salvaged"
    );
    // The daemon tells the parent, as itself: the worker is archived.
    let path = bundle.display().to_string();
    let mut seen = Vec::new();
    for _ in 0..100 {
        let inbox = b.bus.call("check_inbox", json!({})).await;
        seen.extend(inbox["messages"].as_array().expect("messages").clone());
        if seen
            .iter()
            .any(|m| m["body"].as_str().is_some_and(|s| s.contains(&path)))
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        seen.iter()
            .any(|m| m["body"].as_str().is_some_and(|s| s.contains(&path))),
        "{seen:?}"
    );
    let restored = workspace.join("restored");
    git(
        &workspace,
        &["clone", "-q", b.origin.to_str().expect("utf8"), "restored"],
    );
    git(
        &restored,
        &["fetch", "-q", bundle.to_str().expect("utf8"), "main:saved"],
    );
    assert_eq!(git(&restored, &["show", "saved:notes.md"]), "Outline.\n");
}
