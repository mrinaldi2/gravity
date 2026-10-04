use super::*;

#[test]
fn lsof_output_names_each_holder_once_preferring_its_cwd() {
    let home = PathBuf::from("/Users/u/.gravity");
    let output = "p100\ng100\ncpython3\nfcwd\nn/Users/u/.gravity/projects/p/serve\n\
                  p200\ng200\ncnode\nftxt\nn/usr/bin/node\nf12\nn/Users/u/.gravity/browser/x\n\
                  fcwd\nn/Users/u/.gravity/projects\n\
                  p300\ng300\ncvim\nfcwd\nn/Users/u/.gravity-old/x\n\
                  p400\ng1\nchermesd\nfcwd\nn/Users/u/.gravity\n";
    let holders = parse_lsof(output, &[home], 400);
    assert_eq!(holders.len(), 2, "{holders:?}");
    assert_eq!(
        holders[0].to_string(),
        "pid 100 python3 (cwd /Users/u/.gravity/projects/p/serve)"
    );
    assert_eq!(holders[1].pid, 200);
    assert_eq!(holders[1].pgid, Some(200));
    assert!(holders[1].cwd);
    assert_eq!(holders[1].path, PathBuf::from("/Users/u/.gravity/projects"));
}

/// A process working in a directory, in its own group like a bot session,
/// reaped on a thread so it never lingers as a zombie.
#[cfg(unix)]
pub(crate) fn fake_holder(dir: &Path) -> u32 {
    use std::os::unix::process::CommandExt;
    let mut child = std::process::Command::new("/bin/sleep")
        .arg("60")
        .current_dir(dir)
        .process_group(0)
        .spawn()
        .expect("spawn sleep");
    let pid = child.id();
    std::thread::spawn(move || child.wait());
    pid
}

#[cfg(unix)]
#[test]
fn a_process_working_in_the_home_is_listed_and_only_an_owned_one_stopped() {
    let tmp = tempfile::tempdir().expect("tmp");
    let home = tmp.path().join("home");
    std::fs::create_dir_all(home.join("serve")).expect("home");
    let pid = fake_holder(&home.join("serve"));

    let holders = list(&spellings(&home)).expect("lsof");
    let held = holders
        .iter()
        .find(|h| h.pid == pid)
        .unwrap_or_else(|| panic!("{pid} not in {holders:?}"));
    assert!(held.cwd);
    assert_eq!(held.pgid, Some(pid));
    assert!(held.to_string().contains("sleep (cwd "), "{held}");

    // Not ours: reported, never killed.
    assert!(stop_owned(&home, &BTreeSet::from([1_000_000]))
        .expect("stop")
        .is_empty());
    assert!(group_alive(pid));

    assert_eq!(
        stop_owned(&home, &BTreeSet::from([pid])).expect("stop"),
        vec![pid]
    );
    assert!(!group_alive(pid));
}

#[cfg(unix)]
#[test]
fn descendants_in_their_own_groups_are_found() {
    let tmp = tempfile::tempdir().expect("tmp");
    let pid = fake_holder(tmp.path());
    assert!(descendant_groups(std::process::id()).contains(&pid));
    // SAFETY: the group was created above, by this test.
    unsafe { libc::killpg(pid as libc::pid_t, libc::SIGKILL) };
}

#[cfg(unix)]
#[test]
fn recorded_sessions_round_trip_through_the_file() {
    let tmp = tempfile::tempdir().expect("tmp");
    std::fs::write(tmp.path().join(SESSIONS_FILE), "12\n34\nnot a pid\n").expect("file");
    assert_eq!(recorded_sessions(tmp.path()), BTreeSet::from([12, 34]));
    assert!(recorded_sessions(&tmp.path().join("missing")).is_empty());
}
