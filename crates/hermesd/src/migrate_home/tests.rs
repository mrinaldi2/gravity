use super::*;
use crate::activity::{claude_project_key, transcript_dir};

/// A pre-rename home under a temporary user home: a database with one bot,
/// the daemon's old-named files, and a Claude Code transcript dir for the
/// bot's workspace.
pub(super) struct Fixture {
    _tmp: tempfile::TempDir,
    pub(super) plan: Plan,
    pub(super) old_ws: PathBuf,
    pub(super) new_ws: PathBuf,
}

pub(super) fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().expect("tmp");
    let user = tmp.path().join("user");
    let from = user.join(".gravity");
    let to = user.join(".thehermes");
    let old_ws = from.join("projects/app/bots/lead/workspace");
    let new_ws = to.join("projects/app/bots/lead/workspace");
    std::fs::create_dir_all(&old_ws).expect("workspace");
    std::fs::write(old_ws.join("FACTS.md"), "facts").expect("facts");
    std::fs::create_dir_all(from.join("logs")).expect("logs");
    std::fs::write(from.join("gravityd.toml"), "port = 7777\n").expect("config");
    std::fs::write(from.join("logs/gravityd.err.log"), "old log\n").expect("log");
    std::fs::create_dir_all(from.join("secrets")).expect("secrets");
    std::fs::write(from.join("secrets/client.token"), "tok").expect("token");

    let db = crate::db::Db::open(&from.join("bus.sqlite")).expect("db");
    drop(db);
    let conn = rusqlite::Connection::open(from.join("bus.sqlite")).expect("conn");
    conn.execute(
        "INSERT INTO project(id, name, created_at) VALUES ('p', 'app', 'now')",
        [],
    )
    .expect("project");
    conn.execute(
        "INSERT INTO bot(id, project_id, name, instructions, workspace_path, created_at)
         VALUES ('b', 'p', 'lead', ?1, ?2, 'now')",
        rusqlite::params![
            format!("Write to {}/projects/app/artifacts.", from.display()),
            old_ws.to_string_lossy()
        ],
    )
    .expect("bot");
    drop(conn);

    let transcripts = transcript_dir(&user, &old_ws);
    std::fs::create_dir_all(transcripts.join("memory")).expect("transcripts");
    std::fs::write(transcripts.join("s1.jsonl"), "{}\n").expect("session");
    // Not ours: a project that merely shares the prefix up to `.gravity`.
    let unrelated = crate::activity::claude_projects_dir(&user)
        .join(claude_project_key(&user.join(".gravity-old/x")));
    std::fs::create_dir_all(&unrelated).expect("unrelated");

    Fixture {
        plan: Plan::new(from, to, user),
        _tmp: tmp,
        old_ws,
        new_ws,
    }
}

pub(super) fn workspace_path(db: &Path) -> String {
    let conn = rusqlite::Connection::open(db).expect("conn");
    conn.query_row("SELECT workspace_path FROM bot WHERE id = 'b'", [], |r| {
        r.get(0)
    })
    .expect("bot row")
}

#[test]
fn a_dry_run_lists_the_work_and_changes_nothing() {
    let f = fixture();
    let mut out = Vec::new();
    assert!(dry_run(&f.plan, &mut out).expect("dry run"));
    let out = String::from_utf8(out).expect("utf8");
    assert!(out.contains("gravityd.toml -> hermesd.toml"), "{out}");
    assert!(out.contains("bot.workspace_path: 1 row(s)"), "{out}");
    assert!(out.contains("bot.instructions: 1 row(s)"), "{out}");
    assert!(out.contains(&claude_project_key(&f.new_ws)), "{out}");
    assert!(!out.contains("gravity-old"), "{out}");
    assert!(f.plan.from.join("gravityd.toml").exists());
    assert!(!f.plan.to.exists());
    assert!(!f.plan.from.join(STATE_FILE).exists());
}

#[test]
fn a_run_moves_the_home_rewrites_paths_and_keeps_transcripts() {
    let f = fixture();
    let state = run(&f.plan, &mut Vec::new()).expect("migrate");
    assert!(state.is_complete());
    assert_eq!(state.steps, Step::ALL.to_vec());

    let to = &f.plan.to;
    assert_eq!(
        std::fs::read_to_string(to.join("hermesd.toml")).expect("config"),
        "port = 7777\n"
    );
    assert!(to.join("logs/hermesd.err.log").exists());
    assert!(!to.join("gravityd.toml").exists());
    assert!(to.join("secrets/client.token").exists());
    assert!(f.new_ws.join("FACTS.md").exists());

    assert_eq!(
        workspace_path(&to.join("bus.sqlite")),
        f.new_ws.to_string_lossy()
    );
    assert!(is_migrated(to));

    let moved = transcript_dir(&f.plan.user_home, &f.new_ws);
    assert!(moved.join("s1.jsonl").exists());
    assert!(moved.join("memory").is_dir());
    assert!(!transcript_dir(&f.plan.user_home, &f.old_ws).exists());

    // The old path still resolves, through the compatibility link.
    let link = f.plan.from.symlink_metadata().expect("link");
    assert!(link.file_type().is_symlink());
    assert!(f.old_ws.join("FACTS.md").exists());

    let backup = state.backup.expect("backup");
    assert!(backup.starts_with(to));
    assert!(backup.join("bus.sqlite").exists());
    assert!(backup.join("gravityd.toml").exists());
    assert!(backup
        .join("claude-projects")
        .join(claude_project_key(&f.old_ws))
        .join("s1.jsonl")
        .exists());
}

#[test]
fn a_second_run_is_a_no_op_and_nothing_is_pending() {
    let f = fixture();
    run(&f.plan, &mut Vec::new()).expect("migrate");
    let mut out = Vec::new();
    run(&f.plan, &mut out).expect("again");
    assert!(String::from_utf8(out)
        .expect("utf8")
        .contains("already migrated"));
    assert!(!f.plan.source_is_real_home());
    assert!(f.plan.state().expect("state").expect("found").is_complete());
}

#[test]
fn an_interrupted_run_resumes_after_its_last_finished_step() {
    let f = fixture();
    run(&f.plan, &mut Vec::new()).expect("migrate");
    // As if it had stopped right after the database step.
    let mut state = f.plan.state().expect("state").expect("found");
    state.completed_at = None;
    state.steps.truncate(4);
    state
        .actions
        .retain(|action| !matches!(action, Action::Symlink { .. }));
    // A junction on Windows, which `remove_file` refuses.
    steps::remove_link(&f.plan.from).expect("drop link");
    state.save().expect("save");

    let state = run(&f.plan, &mut Vec::new()).expect("resume");
    assert!(state.is_complete());
    assert!(f
        .plan
        .from
        .symlink_metadata()
        .expect("link again")
        .is_symlink());
    assert_eq!(
        workspace_path(&f.plan.to.join("bus.sqlite")),
        f.new_ws.to_string_lossy()
    );
}

#[test]
fn rollback_restores_the_old_layout() {
    let f = fixture();
    run(&f.plan, &mut Vec::new()).expect("migrate");
    rollback(&f.plan, &mut Vec::new()).expect("rollback");

    let from = &f.plan.from;
    assert!(from.symlink_metadata().expect("home").is_dir());
    assert!(!f.plan.to.exists());
    assert!(from.join("gravityd.toml").exists());
    assert!(from.join("logs/gravityd.err.log").exists());
    assert!(!from.join(STATE_FILE).exists());
    assert_eq!(
        workspace_path(&from.join("bus.sqlite")),
        f.old_ws.to_string_lossy()
    );
    assert!(!is_migrated(from));
    assert!(transcript_dir(&f.plan.user_home, &f.old_ws)
        .join("s1.jsonl")
        .exists());

    // And it can be migrated again afterwards.
    run(&f.plan, &mut Vec::new()).expect("migrate again");
}

#[test]
fn a_non_empty_destination_blocks_the_move() {
    let f = fixture();
    std::fs::create_dir_all(f.plan.to.join("projects")).expect("taken");
    let error = run(&f.plan, &mut Vec::new()).expect_err("refused");
    assert!(format!("{error:#}").contains("not empty"), "{error:#}");
    assert!(f.plan.from.join("gravityd.toml").exists());
}

#[test]
fn a_running_daemon_blocks_the_move() {
    let f = fixture();
    let held = crate::home::lock_legacy(&f.plan.from).expect("old daemon");
    let mut out = Vec::new();
    assert!(!dry_run(&f.plan, &mut out).expect("dry run"));
    assert!(String::from_utf8(out)
        .expect("utf8")
        .contains("a daemon is running"));
    drop(held);
}

#[test]
fn a_transcript_dir_already_under_the_new_name_is_merged() {
    let f = fixture();
    let early = transcript_dir(&f.plan.user_home, &f.new_ws);
    std::fs::create_dir_all(&early).expect("early");
    std::fs::write(early.join("s2.jsonl"), "{}\n").expect("early session");

    let state = run(&f.plan, &mut Vec::new()).expect("migrate");
    assert!(early.join("s1.jsonl").exists());
    assert!(early.join("s2.jsonl").exists());
    assert!(early.join("memory").is_dir());
    assert!(state.warnings.is_empty(), "{:?}", state.warnings);
}

/// cmd.exe runs `mklink`, so a user name with `&` or `^` must not split it,
/// and removing the junction must leave its target alone.
#[cfg(windows)]
#[test]
fn a_junction_survives_shell_characters_in_the_path() {
    let root = tempfile::tempdir().expect("temporary dir");
    let target = root.path().join("Test&User^1");
    std::fs::create_dir(&target).expect("target");
    std::fs::write(target.join("bus.sqlite"), b"db").expect("file");
    let link = root.path().join("old&home");

    steps::make_link(&target, &link).expect("junction");
    assert!(link
        .symlink_metadata()
        .expect("link")
        .file_type()
        .is_symlink());
    assert_eq!(
        std::fs::read(link.join("bus.sqlite")).expect("through"),
        b"db"
    );

    steps::remove_link(&link).expect("remove junction");
    assert!(link.symlink_metadata().is_err());
    assert!(target.join("bus.sqlite").is_file());
    // A real directory is never taken for a link.
    assert!(steps::remove_link(&target).is_err());
    assert!(target.is_dir());
}

/// A live bot whose workspace the rewrite missed (here an upper-case
/// spelling) would start under the old transcript key. The Database step
/// fails instead, naming it, and the run can be rolled back. Windows
/// compares paths loosely, so there the rewrite takes the variant.
#[test]
fn a_workspace_left_under_the_old_home_fails_the_database_step() {
    let f = fixture();
    let variant = f.old_ws.to_string_lossy().replace(".gravity", ".GRAVITY");
    let conn = rusqlite::Connection::open(f.plan.from.join("bus.sqlite")).expect("conn");
    for (id, name, deleted) in [("c", "shouty", None), ("d", "gone", Some("then"))] {
        conn.execute(
            "INSERT INTO bot(id, project_id, name, workspace_path, created_at, deleted_at)
             VALUES (?1, 'p', ?2, ?3, 'now', ?4)",
            rusqlite::params![id, name, variant, deleted],
        )
        .expect("bot");
    }
    drop(conn);

    let result = run(&f.plan, &mut Vec::new());
    if cfg!(windows) {
        result.expect("migrated");
        return;
    }
    let error = format!("{:#}", result.expect_err("stale workspace"));
    assert!(error.contains("step Database"), "{error}");
    assert!(error.contains("shouty"), "{error}");
    assert!(!error.contains("gone"), "{error}");
    // Nothing committed: the database still says not migrated.
    assert!(!is_migrated(&f.plan.to));
    rollback(&f.plan, &mut Vec::new()).expect("rollback");
    assert_eq!(
        workspace_path(&f.plan.from.join("bus.sqlite")),
        f.old_ws.to_string_lossy()
    );
}
