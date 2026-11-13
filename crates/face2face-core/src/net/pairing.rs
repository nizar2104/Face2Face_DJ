//! The pairing code: what stops anyone else on the network from reading the decks.
//!
//! A Sender that listens beyond this computer has a code, and a Viewer must
//! send it in `hello.pair_code`. The DJs exchange it by reading it off each
//! other's screen (docs/SETUP.md).
//!
//! The code is short, so guessing has to be slow: every attempt from one
//! address waits for the previous one's turn plus [`ATTEMPT_SPACING`]. At one
//! attempt a second, trying every 6-digit code takes eleven days. A correct
//! code clears the address, so a partner who reconnects is never held up.
//!
//! The code is sent as plain text and is not encryption: someone who can
//! capture traffic on the network can read it, and the deck data itself.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::lock;

/// The shortest code a Sender on the network may have.
pub const MIN_CODE_LENGTH: usize = 6;
const ATTEMPT_SPACING: Duration = Duration::from_secs(1);
/// An address with this much waiting queued up is refused outright, so that
/// held-open connections cannot pile up.
const MAX_QUEUE: Duration = Duration::from_secs(10);

pub struct Pairing {
    code: String,
    /// Per address: when its next attempt may be looked at.
    next_turn: Mutex<HashMap<IpAddr, Instant>>,
}

impl Pairing {
    pub fn new(code: String) -> Self {
        Self { code, next_turn: Mutex::new(HashMap::new()) }
    }

    /// Takes a turn for an attempt from `peer`: the instant at which its code
    /// may be checked, or `None` if that address already has too many waiting.
    pub fn take_turn(&self, peer: IpAddr, now: Instant) -> Option<Instant> {
        let mut next_turn = lock(&self.next_turn);
        next_turn.retain(|_, turn| *turn > now); // forget addresses whose wait is over
        let turn = next_turn.get(&peer).copied().unwrap_or(now);
        if turn.saturating_duration_since(now) > MAX_QUEUE {
            return None;
        }
        next_turn.insert(peer, turn + ATTEMPT_SPACING);
        Some(turn)
    }

    /// Whether `offered` is the code. A correct one from `peer` ends its waiting.
    pub fn check(&self, peer: IpAddr, offered: Option<&str>) -> bool {
        // Compare every byte, so the time taken says nothing about how many matched.
        let matches = offered.is_some_and(|offered| {
            offered.len() == self.code.len()
                && offered.bytes().zip(self.code.bytes()).fold(0, |differences, (a, b)| differences | (a ^ b)) == 0
        });
        if matches {
            lock(&self.next_turn).remove(&peer);
        }
        matches
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    fn peer(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(192, 168, 1, last))
    }

    #[test]
    fn only_the_exact_code_is_accepted() {
        let pairing = Pairing::new("483920".to_string());
        assert!(pairing.check(peer(2), Some("483920")));
        for wrong in [None, Some(""), Some("483921"), Some("48392"), Some("4839200"), Some("483 920")] {
            assert!(!pairing.check(peer(2), wrong), "{wrong:?}");
        }
    }

    #[test]
    fn attempts_from_one_address_are_spaced_out() {
        let pairing = Pairing::new("483920".to_string());
        let now = Instant::now();
        assert_eq!(pairing.take_turn(peer(2), now), Some(now), "the first attempt is looked at immediately");
        assert_eq!(pairing.take_turn(peer(2), now), Some(now + ATTEMPT_SPACING));
        assert_eq!(pairing.take_turn(peer(2), now), Some(now + 2 * ATTEMPT_SPACING), "also when sent in parallel");
        assert_eq!(pairing.take_turn(peer(3), now), Some(now), "another address is not held up");
    }

    #[test]
    fn a_long_queue_is_refused_and_waiting_ends() {
        let pairing = Pairing::new("483920".to_string());
        let now = Instant::now();
        let granted = std::iter::from_fn(|| pairing.take_turn(peer(2), now)).take(100).count();
        assert_eq!(granted, 11, "turns at 0 to 10 s, then refusal");
        assert_eq!(pairing.take_turn(peer(2), now), None);
        let later = now + MAX_QUEUE + 2 * ATTEMPT_SPACING;
        assert_eq!(pairing.take_turn(peer(2), later), Some(later), "once the wait is over, the address starts afresh");
    }

    #[test]
    fn the_right_code_clears_the_wait() {
        let pairing = Pairing::new("483920".to_string());
        let now = Instant::now();
        pairing.take_turn(peer(2), now);
        assert!(!pairing.check(peer(2), Some("000000")));
        assert_eq!(pairing.take_turn(peer(2), now), Some(now + ATTEMPT_SPACING));
        assert!(pairing.check(peer(2), Some("483920")));
        assert_eq!(pairing.take_turn(peer(2), now), Some(now), "a partner who reconnects does not wait");
    }
}
