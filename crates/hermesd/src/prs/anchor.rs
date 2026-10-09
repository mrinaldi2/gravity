//! Where a line comment shows on a newer head (H-261 §1.4): its line,
//! carried through the zero-context diff of its file from the commit it was
//! written on, or `outdated` when that line was changed or removed. A
//! comment never shows on a line it wasn't written about.

use std::path::Path;

use crate::board::release::git_cache::{self, Hunk};

/// The new-side line for old line `line`, through `hunks` in order; `None`
/// when a hunk replaced or removed it.
pub fn remap(line: u32, hunks: &[Hunk]) -> Option<u32> {
    let mut shift: i64 = 0;
    for h in hunks {
        if h.old_len == 0 {
            // Pure insertion after old line `old_start`.
            if line > h.old_start {
                shift += i64::from(h.new_len);
            }
            continue;
        }
        let end = h.old_start + h.old_len; // first old line after the hunk
        if line >= h.old_start && line < end {
            return None;
        }
        if line >= end {
            shift += i64::from(h.new_len) - i64::from(h.old_len);
        }
    }
    u32::try_from(i64::from(line) + shift).ok()
}

/// Where a comment on `path:line` at `from` shows at `to`. `side` `old`
/// lines belong to the diff they were written on and never move: they are
/// outdated on any other head.
pub fn anchor(
    cache: &Path,
    from: &str,
    to: &str,
    path: &str,
    line: u32,
    side: &str,
) -> anyhow::Result<Option<u32>> {
    if from == to {
        return Ok(Some(line));
    }
    if side == "old" {
        return Ok(None);
    }
    Ok(git_cache::hunks(cache, from, to, path)?.and_then(|h| remap(line, &h)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(old_start: u32, old_len: u32, new_len: u32) -> Hunk {
        Hunk {
            old_start,
            old_len,
            new_len,
        }
    }

    #[test]
    fn lines_before_a_change_stay_and_after_it_shift() {
        let hunks = [h(3, 0, 2)]; // two lines inserted after line 3
        assert_eq!(remap(2, &hunks), Some(2));
        assert_eq!(remap(3, &hunks), Some(3));
        assert_eq!(remap(4, &hunks), Some(6));
    }

    #[test]
    fn a_changed_or_removed_line_is_outdated() {
        let hunks = [h(5, 2, 1)]; // lines 5-6 became one line
        assert_eq!(remap(5, &hunks), None);
        assert_eq!(remap(6, &hunks), None);
        assert_eq!(remap(7, &hunks), Some(6));
        assert_eq!(remap(4, &hunks), Some(4));
        let removed = [h(2, 1, 0)];
        assert_eq!(remap(2, &removed), None);
        assert_eq!(remap(3, &removed), Some(2));
    }

    #[test]
    fn hunks_add_up() {
        let hunks = [h(1, 0, 3), h(10, 1, 1), h(20, 0, 1)];
        assert_eq!(remap(5, &hunks), Some(8));
        assert_eq!(remap(10, &hunks), None);
        assert_eq!(remap(25, &hunks), Some(29));
    }
}

#[cfg(test)]
#[path = "anchor_git_tests.rs"]
mod git_tests;
