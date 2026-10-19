//! `IssuedAt` (personal-cfo-t2s4b; ADR 0013 Addendum A): a command's issue instant as
//! microseconds since the Unix epoch, strictly increasing within a process.

use std::collections::HashSet;
use std::sync::Arc;
use std::thread;

use core_ids::IssuedAt;

#[test]
fn it_is_exactly_the_wire_number() {
    // The membership `issued_at` sample in vault-format.md C.14 and the ADR 0013-A
    // worked example (unix_ms 1700000000123).
    let sample = IssuedAt::from_unix_micros(1_790_000_001_000_000);
    assert_eq!(sample.as_unix_micros(), 1_790_000_001_000_000);
    assert_eq!(sample.as_unix_millis(), 1_790_000_001_000);

    let adr = IssuedAt::from_unix_micros(1_700_000_000_123_000);
    assert_eq!(adr.as_unix_millis(), 1_700_000_000_123);
    assert_eq!(
        IssuedAt::from_unix_micros(i64::MAX).as_unix_micros(),
        i64::MAX
    );
    assert_eq!(
        IssuedAt::from_unix_micros(i64::MIN).as_unix_micros(),
        i64::MIN
    );
}

#[test]
fn milliseconds_round_toward_negative_infinity() {
    for (micros, millis) in [
        (0, 0),
        (999, 0),
        (1_000, 1),
        (1_999, 1),
        (-1, -1),
        (-1_000, -1),
        (-1_001, -2),
    ] {
        assert_eq!(
            IssuedAt::from_unix_micros(micros).as_unix_millis(),
            millis,
            "{micros}"
        );
    }
}

#[test]
fn values_order_by_instant() {
    let early = IssuedAt::from_unix_micros(10);
    let late = IssuedAt::from_unix_micros(11);
    assert!(early < late);
    assert_eq!(early, IssuedAt::from_unix_micros(10));
    assert_eq!(early.max(late), late);
}

#[test]
fn now_is_a_plausible_current_time_in_microseconds() {
    // After 2025-01-01 and before 2100-01-01, in microseconds (not milliseconds or
    // nanoseconds): a unit mix-up lands far outside this window.
    let micros = IssuedAt::now().as_unix_micros();
    assert!(micros > 1_735_689_600_000_000, "{micros}");
    assert!(micros < 4_102_444_800_000_000, "{micros}");
}

#[test]
fn now_is_strictly_increasing_within_a_process() {
    let mut previous = IssuedAt::now();
    for _ in 0..50_000 {
        let next = IssuedAt::now();
        assert!(next > previous, "{next:?} must follow {previous:?}");
        previous = next;
    }
}

#[test]
fn now_never_repeats_across_threads() {
    let threads = 8;
    let per_thread = 5_000;
    let handles: Vec<_> = (0..threads)
        .map(|_| {
            thread::spawn(move || {
                (0..per_thread)
                    .map(|_| IssuedAt::now().as_unix_micros())
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let mut seen = HashSet::new();
    for handle in handles {
        for value in handle.join().unwrap() {
            assert!(seen.insert(value), "{value} was issued twice");
        }
    }
    assert_eq!(seen.len(), threads * per_thread);
}

#[test]
fn a_value_is_copy_and_usable_as_a_shared_key() {
    let shared = Arc::new(IssuedAt::from_unix_micros(5));
    let copy = *shared;
    assert_eq!(copy, *shared);
}
