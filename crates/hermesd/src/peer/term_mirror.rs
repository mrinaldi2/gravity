//! The mirror of a peer bot's terminal: where its feed stands and which of
//! the peer's frames it has already pushed into the stand-in's terminal.

/// Where a mirror stands.
pub(super) enum State {
    /// No feed: nobody watches, or the link is down.
    Stopped,
    /// Asked for a feed; frames that overtake the reply wait here.
    Starting(Vec<(u64, Vec<u8>)>),
    Running,
}

/// A peer bot's terminal mirrored into its stand-in's buffer.
pub(super) struct Mirror {
    pub(super) peer_id: String,
    pub(super) remote_bot_id: String,
    pub(super) viewers: usize,
    pub(super) state: State,
    /// The newest of the peer's sequence numbers pushed into the mirror.
    pub(super) last_remote: u64,
}

impl Mirror {
    /// Whether a frame from the peer is newer than what the mirror holds,
    /// moving the mirror's cursor to it if so.
    pub(super) fn accept(&mut self, seq: u64) -> bool {
        if seq <= self.last_remote {
            return false;
        }
        self.last_remote = seq;
        true
    }

    /// What an attach reply's replay, and the frames that overtook it, push
    /// into the stand-in's terminal.
    ///
    /// A reply that could not resume replays the peer's whole buffer: its
    /// daemon restarted and numbers frames from 1 again, or the cursor fell
    /// out of its ring. The mirror then forgets its cursor, or it would drop
    /// every frame until the new numbers passed the old ones, and starts
    /// with a terminal reset so the replay paints a clean screen.
    pub(super) fn take_replay(
        &mut self,
        resumed: bool,
        frames: impl IntoIterator<Item = (u64, Vec<u8>)>,
    ) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        if !resumed && self.last_remote > 0 {
            self.last_remote = 0;
            out.push(b"\x1bc".to_vec());
        }
        for (seq, data) in frames {
            if self.accept(seq) {
                out.push(data);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mirror(last_remote: u64) -> Mirror {
        Mirror {
            peer_id: "peer".into(),
            remote_bot_id: "bot".into(),
            viewers: 1,
            state: State::Running,
            last_remote,
        }
    }

    fn frames(seqs: &[u64]) -> Vec<(u64, Vec<u8>)> {
        seqs.iter()
            .map(|s| (*s, format!("f{s}").into_bytes()))
            .collect()
    }

    #[test]
    fn a_resumed_replay_skips_what_the_mirror_already_holds() {
        let mut m = mirror(5);
        let out = m.take_replay(true, frames(&[4, 5, 6, 7]));
        assert_eq!(out, vec![b"f6".to_vec(), b"f7".to_vec()]);
        assert_eq!(m.last_remote, 7);
    }

    #[test]
    fn a_restarted_peer_numbering_from_one_again_is_mirrored() {
        // The peer fed up to 90 000, then its daemon restarted: the attach
        // reply cannot resume and replays its new buffer from seq 1.
        let mut m = mirror(90_000);
        let out = m.take_replay(false, frames(&[1, 2, 3]));
        assert_eq!(
            out,
            vec![
                b"\x1bc".to_vec(),
                b"f1".to_vec(),
                b"f2".to_vec(),
                b"f3".to_vec()
            ]
        );
        assert_eq!(m.last_remote, 3);
        // Live frames after the replay are applied, not dropped as old.
        assert!(m.accept(4));
        assert!(m.accept(5));
        assert!(!m.accept(5));
        assert_eq!(m.last_remote, 5);
    }

    #[test]
    fn a_first_replay_needs_no_reset() {
        let mut m = mirror(0);
        let out = m.take_replay(false, frames(&[1, 2]));
        assert_eq!(out, vec![b"f1".to_vec(), b"f2".to_vec()]);
    }
}
