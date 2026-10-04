//! The home migration (`hermesd migrate-home`) on two paired daemons, as on
//! the Mac and the PC: each runs on a pre-rename `~/.gravity` with bots and
//! Claude Code transcripts, is stopped, migrated to `~/.thehermes`, and comes
//! back up with every bot, its history and the peer link intact.

mod common;

use std::path::{Path, PathBuf};

use common::peers::{pair, project};
use common::*;
use hermesd::migrate_home::{self, Plan};
use serde_json::json;

struct Machine {
    user: PathBuf,
    bots: Vec<(String, PathBuf)>,
}

impl Machine {
    fn old_home(&self) -> PathBuf {
        self.user.join(".gravity")
    }
    fn new_home(&self) -> PathBuf {
        self.user.join(".thehermes")
    }
    fn plan(&self) -> Plan {
        Plan::new(self.old_home(), self.new_home(), self.user.clone())
    }
}

/// Claude Code's name for a workspace's transcript dir.
fn transcript_dir(user: &Path, workspace: &Path) -> PathBuf {
    let key: String = workspace
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    user.join(".claude/projects").join(key)
}

fn on_home(user: PathBuf, home: PathBuf) -> impl FnOnce(&mut hermesd::config::Config) {
    move |cfg| {
        cfg.home = home;
        cfg.user_home = user;
    }
}

/// Two paired daemons on pre-rename homes, each with a project and bots and
/// a transcript per bot. Runs on a runtime of its own, dropped at the end, so
/// both daemons have let go of their databases before anything moves.
fn pre_rename_machines(root: &Path) -> (Machine, Machine) {
    let (mac_user, win_user) = (root.join("mac"), root.join("win"));
    let (mac_u, win_u) = (mac_user.clone(), win_user.clone());
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let (mac_bots, win_bots) = runtime.block_on(async move {
        let mac = spawn_daemon_with(on_home(mac_u.clone(), mac_u.join(".gravity"))).await;
        let win = spawn_daemon_with(on_home(win_u.clone(), win_u.join(".gravity"))).await;
        let mut mac_client = WsClient::connect(&mac).await;
        let mut win_client = WsClient::connect(&win).await;
        pair(&mac, &win, &mut mac_client, &mut win_client).await;
        let mut out = Vec::new();
        for (d, client, names) in [
            (&mac, &mut mac_client, ["lead", "architect"]),
            (&win, &mut win_client, ["windev", "tester"]),
        ] {
            let project_id = project(client, "app").await;
            let mut bots = Vec::new();
            for name in names {
                let created = create_bot(client, &project_id, name).await;
                let id = created["id"].as_str().expect("bot id").to_string();
                let bot = d.app.db.get_bot(&id).expect("db").expect("bot");
                bots.push((id, PathBuf::from(bot.workspace_path)));
            }
            out.push(bots);
        }
        (out.remove(0), out.remove(0))
    });
    drop(runtime);

    let machines = (
        Machine {
            user: mac_user,
            bots: mac_bots,
        },
        Machine {
            user: win_user,
            bots: win_bots,
        },
    );
    for m in [&machines.0, &machines.1] {
        std::fs::write(m.old_home().join("gravityd.toml"), "port = 49777\n").expect("config");
        for (id, workspace) in &m.bots {
            assert!(
                workspace.starts_with(m.old_home()),
                "{}",
                workspace.display()
            );
            let dir = transcript_dir(&m.user, workspace);
            std::fs::create_dir_all(dir.join("memory")).expect("transcript dir");
            std::fs::write(dir.join("memory/MEMORY.md"), format!("memory of {id}"))
                .expect("memory");
            let line = json!({
                "type": "assistant",
                "timestamp": "2026-10-01T00:00:00Z",
                "message": {"content": [{"type": "text", "text": format!("hello from {id}")}]}
            });
            std::fs::write(dir.join("session.jsonl"), format!("{line}\n")).expect("transcript");
        }
    }
    machines
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn both_daemons_come_back_on_the_new_home_with_bots_and_history() {
    let root = tempfile::tempdir().expect("root");
    let root_path = root.path().to_path_buf();
    let (mac, win) = tokio::task::spawn_blocking(move || pre_rename_machines(&root_path))
        .await
        .expect("fixture");

    for m in [&mac, &win] {
        let mut out = Vec::new();
        assert!(migrate_home::dry_run(&m.plan(), &mut out).expect("dry run"));
        let state = migrate_home::run(&m.plan(), &mut out).expect("migrate");
        assert!(state.is_complete(), "{}", String::from_utf8_lossy(&out));
        assert!(m.new_home().join("hermesd.toml").is_file());
        assert!(m.old_home().symlink_metadata().expect("link").is_symlink());
    }

    for m in [&mac, &win] {
        let d = spawn_daemon_with(on_home(m.user.clone(), m.new_home())).await;
        assert_eq!(d.app.db.list_peers().expect("peers").len(), 1);
        for (id, old_ws) in &m.bots {
            let bot = d.app.db.get_bot(id).expect("db").expect("bot survives");
            let new_ws = PathBuf::from(&bot.workspace_path);
            assert_eq!(
                new_ws,
                m.new_home()
                    .join(old_ws.strip_prefix(m.old_home()).expect("under home"))
            );
            assert!(new_ws.join("CLAUDE.md").is_file());

            // History and memory are where Claude Code will look for them.
            let dir = transcript_dir(&m.user, &new_ws);
            assert!(dir.join("session.jsonl").is_file());
            assert_eq!(
                std::fs::read_to_string(dir.join("memory/MEMORY.md")).expect("memory"),
                format!("memory of {id}")
            );
            assert!(!transcript_dir(&m.user, old_ws).exists());
            let activity = hermesd::activity::from_transcript(&m.user, &new_ws).expect("activity");
            assert_eq!(activity.text, format!("hello from {id}"));

            // Regenerated at boot from the rewritten database.
            let system = std::fs::read_to_string(new_ws.parent().expect("root").join("system.md"))
                .expect("system.md");
            assert!(
                !system.contains(&*m.old_home().to_string_lossy()),
                "{system}"
            );
            assert!(
                system.contains(&*m.new_home().to_string_lossy()),
                "{system}"
            );
        }
        let plan = m.plan();
        assert!(migrate_home::pending_for(&plan).expect("pending").is_none());
    }
}
