//! Fractional ranks: keys that sort as plain strings, with room for a new key
//! between any two. Moving a card rewrites one row, never its neighbours.
//!
//! A key is a base-62 fraction (`0-9A-Za-z`, in ASCII order) read as `0.k`.
//! A generated key never ends in `0`, so there is always room before it.

const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const BASE: usize = DIGITS.len();

fn digit(key: &[u8], i: usize) -> usize {
    key.get(i)
        .and_then(|c| DIGITS.iter().position(|d| d == c))
        .unwrap_or(0)
}

/// A key strictly between `before` and `after`; `None` means the open end.
/// Returns `None` when the bounds are not ordered (or not valid keys).
pub fn between(before: Option<&str>, after: Option<&str>) -> Option<String> {
    let low = before.unwrap_or("").as_bytes();
    let high = after.map(str::as_bytes);
    if !low.iter().all(|c| DIGITS.contains(c))
        || high.is_some_and(|h| h.is_empty() || !h.iter().all(|c| DIGITS.contains(c)))
    {
        return None;
    }
    if let Some(high) = high {
        if trimmed(low) >= trimmed(high) {
            return None;
        }
    }
    let mut out = Vec::new();
    // Once the result has a digit below `high`'s, `high` no longer bounds it.
    let mut bounded = high.is_some();
    for i in 0.. {
        let lo = digit(low, i);
        let hi = match high {
            Some(h) if bounded => digit(h, i),
            _ => BASE,
        };
        if hi > lo + 1 {
            out.push(DIGITS[(lo + hi) / 2]);
            break;
        }
        out.push(DIGITS[lo]);
        if hi == lo + 1 {
            bounded = false;
        }
        if i > low.len() + high.map_or(0, <[u8]>::len) + 1 {
            return None;
        }
    }
    String::from_utf8(out).ok()
}

/// Trailing zeros do not change a key's value.
fn trimmed(key: &[u8]) -> &[u8] {
    let end = key.iter().rposition(|c| *c != b'0').map_or(0, |i| i + 1);
    &key[..end]
}

/// `count` evenly spread keys, for seeding an ordered list in one go.
pub fn spread(count: usize) -> Vec<String> {
    let mut keys = Vec::with_capacity(count);
    let mut last: Option<String> = None;
    for _ in 0..count {
        let next = between(last.as_deref(), None).expect("an open upper end always has room");
        keys.push(next.clone());
        last = Some(next);
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_key_sits_in_the_middle() {
        assert_eq!(between(None, None).as_deref(), Some("V"));
    }

    #[test]
    fn keys_land_strictly_between_their_bounds() {
        for (lo, hi) in [
            ("V", "W"),
            ("V", "V1"),
            ("0", "1"),
            ("a", "b"),
            ("Vz", "W"),
            ("z", "zz"),
        ] {
            let mid = between(Some(lo), Some(hi)).unwrap_or_else(|| panic!("{lo}..{hi}"));
            assert!(
                lo < mid.as_str() && mid.as_str() < hi,
                "{lo} < {mid} < {hi}"
            );
        }
        let before_first = between(None, Some("1")).expect("room before");
        assert!(before_first.as_str() < "1" && !before_first.ends_with('0'));
        let after_last = between(Some("zzz"), None).expect("room after");
        assert!(after_last.as_str() > "zzz");
    }

    #[test]
    fn unordered_or_invalid_bounds_are_refused() {
        assert_eq!(between(Some("W"), Some("V")), None);
        assert_eq!(between(Some("V"), Some("V")), None);
        assert_eq!(between(Some("V"), Some("V0")), None);
        assert_eq!(between(Some("-"), None), None);
        assert_eq!(between(None, Some("")), None);
    }

    /// Repeatedly inserting at the same spot, at the front and at the back
    /// keeps every key distinct and in order.
    #[test]
    fn many_insertions_stay_ordered() {
        let mut keys = vec![between(None, None).expect("first")];
        for round in 0..300 {
            let at = match round % 3 {
                0 => 0,
                1 => keys.len(),
                _ => keys.len() / 2,
            };
            let before = at.checked_sub(1).map(|i| keys[i].as_str());
            let after = keys.get(at).map(String::as_str);
            let key = between(before, after).expect("room");
            assert!(!key.ends_with('0'), "{key}");
            keys.insert(at, key);
        }
        let mut sorted = keys.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted, keys);
    }

    #[test]
    fn spread_keys_ascend() {
        let keys = spread(12);
        assert_eq!(keys.len(), 12);
        assert!(keys.windows(2).all(|w| w[0] < w[1]));
    }
}
