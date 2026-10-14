//! Session derivation tests. Sessions are a query-time derivation from ordered
//! hero hands, so these are pure and fast.

use chrono::{DateTime, Duration, TimeZone, Utc};
use texture::stats::{self, SessionHand};

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.from_utc_datetime(&DateTime::from_timestamp(seconds, 0).unwrap().naive_utc())
}

fn hand(seconds: i64) -> SessionHand {
    SessionHand { key: seconds, t: at(seconds), net_minor: 100 }
}

const HALF_HOUR: i64 = 30 * 60;

#[test]
fn an_empty_sequence_has_no_sessions() {
    assert!(stats::derive_sessions(&[], HALF_HOUR).is_empty());
}

#[test]
fn a_single_hand_is_one_session() {
    assert_eq!(stats::derive_sessions(&[hand(0)], HALF_HOUR), vec![(0, 0, 1)]);
}

#[test]
fn a_gap_exactly_at_the_threshold_is_the_same_session() {
    // The boundary rule is strict: only a gap *greater* than the threshold starts
    // a new session.
    let hands = vec![hand(0), hand(HALF_HOUR)];
    assert_eq!(stats::derive_sessions(&hands, HALF_HOUR), vec![(0, 0, 2)]);
}

#[test]
fn a_gap_just_above_the_threshold_starts_a_new_session() {
    let hands = vec![hand(0), hand(HALF_HOUR + 1)];
    assert_eq!(
        stats::derive_sessions(&hands, HALF_HOUR),
        vec![(0, 0, 1), (1, 1, 2)]
    );
}

#[test]
fn a_gap_just_below_the_threshold_stays_in_one_session() {
    let hands = vec![hand(0), hand(HALF_HOUR - 1)];
    assert_eq!(stats::derive_sessions(&hands, HALF_HOUR), vec![(0, 0, 2)]);
}

#[test]
fn a_long_break_splits_a_sequence_into_three_sessions() {
    let hands = vec![
        hand(0),
        hand(60),
        hand(120),
        hand(120 + HALF_HOUR + 1),
        hand(120 + HALF_HOUR + 60),
        hand(120 + HALF_HOUR + 60 + HALF_HOUR + 1),
    ];
    let sessions = stats::derive_sessions(&hands, HALF_HOUR);
    assert_eq!(sessions.len(), 3);
    assert_eq!(sessions[0], (0, 0, 3));
    assert_eq!(sessions[1], (1, 3, 5));
    assert_eq!(sessions[2], (2, 5, 6));
    // Ranges are contiguous and cover every hand exactly once.
    assert_eq!(sessions[0].2, sessions[1].1);
    assert_eq!(sessions[1].2, sessions[2].1);
    assert_eq!(sessions.last().unwrap().2, hands.len());
}

#[test]
fn a_backwards_step_does_not_split_a_session() {
    // A record written under a different timezone can put a later hand slightly
    // before its predecessor. The gap is compared on its magnitude, so this is
    // still one session rather than a spurious boundary.
    let hands = vec![hand(1_000), hand(1_000 - 60), hand(1_000 + 60)];
    assert_eq!(stats::derive_sessions(&hands, HALF_HOUR), vec![(0, 0, 3)]);
}

#[test]
fn multi_tabling_interleaves_into_one_session() {
    // Two tables played at once: hands alternate between them, all within seconds
    // of each other. The gap is time-based, not table-based, so this is one
    // session, which is how a multi-tabling player thinks about it.
    let mut hands = Vec::new();
    for i in 0..40 {
        hands.push(hand(i * 20));
    }
    let sessions = stats::derive_sessions(&hands, HALF_HOUR);
    assert_eq!(sessions, vec![(0, 0, 40)]);
}

#[test]
fn session_length_is_measured_from_the_first_and_last_hand() {
    // Duration is derived from hand timestamps and is therefore approximate: it
    // excludes any time played without a hand being dealt.
    let hands = vec![hand(0), hand(600), hand(1200)];
    let sessions = stats::derive_sessions(&hands, HALF_HOUR);
    let (_, start, end) = sessions[0];
    let span = hands[end - 1].t - hands[start].t;
    assert_eq!(span, Duration::seconds(1200));
}
