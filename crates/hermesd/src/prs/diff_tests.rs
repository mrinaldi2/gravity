//! Reading git's file lists, and the 2 MB cap.

use super::*;

#[test]
fn numstat_counts_lines_and_marks_binaries_and_renames() {
    let raw = b"3\t1\tsrc/a.rs\0-\t-\timg.png\x002\t0\t\0old.txt\0new.txt\0";
    let counts = numstat(raw);
    assert_eq!(counts["src/a.rs"], Some((3, 1)));
    assert_eq!(counts["img.png"], None);
    assert_eq!(counts["new.txt"], Some((2, 0)));
    assert_eq!(counts.len(), 3);
}

#[test]
fn name_status_reads_each_kind() {
    use p::FileStatus as S;
    let raw = b"M\0src/a.rs\0A\0b.txt\0D\0c.txt\0R087\0old.txt\0new.txt\0T\0link\0";
    let files: Vec<(String, String, i32)> = name_status(raw)
        .into_iter()
        .map(|f| (f.path, f.old_path, f.status))
        .collect();
    let want = [
        ("src/a.rs", "", S::Modified),
        ("b.txt", "", S::Added),
        ("c.txt", "", S::Deleted),
        ("new.txt", "old.txt", S::Renamed),
        ("link", "", S::Modified),
    ];
    let want: Vec<(String, String, i32)> = want
        .iter()
        .map(|(p, o, s)| (p.to_string(), o.to_string(), *s as i32))
        .collect();
    assert_eq!(files, want);
}

#[test]
fn the_diff_stops_at_two_megabytes_on_a_whole_line() {
    let small = b"+one\n+two\n";
    assert_eq!(capped(small), ("+one\n+two\n".to_string(), false));
    let line = "+".repeat(99) + "\n";
    let big = line.repeat(MAX_DIFF_BYTES / 100 + 10);
    let (text, truncated) = capped(big.as_bytes());
    assert!(truncated);
    assert!(text.len() <= MAX_DIFF_BYTES);
    assert!(text.ends_with('\n'));
    assert_eq!(text.len() % 100, 0);
    // Exactly at the cap is whole.
    let exact = "a".repeat(MAX_DIFF_BYTES);
    assert!(!capped(exact.as_bytes()).1);
}
