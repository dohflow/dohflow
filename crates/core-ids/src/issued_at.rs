//! [`IssuedAt`]: the instant a command was issued (ADR 0013 Addendum A,
//! `docs/architecture/vault-format.md` Part C).
//!
//! `issued_at` is set once, where a command's metadata is built, and then travels with
//! the command: it is persisted on the operation-log row, carried in the envelope's
//! immutable metadata, and read by `apply` instead of the wall clock, so replaying a
//! command on another device reads the same instant. The wire form is a **signed 64-bit
//! count of microseconds since the Unix epoch, UTC**; this type is exactly that number,
//! so it cannot drift between a local value and its serialized form (a `chrono` value
//! carries nanoseconds that would be lost on the wire).
//!
//! Derived identifiers use the millisecond count ([`IssuedAt::as_unix_millis`]), the
//! `unix_ms` of ADR 0013-A's UUIDv7 layout.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The instant a command was issued: microseconds since the Unix epoch, UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IssuedAt(i64);

/// The last value [`IssuedAt::now`] returned in this process.
static LAST_ISSUED: AtomicI64 = AtomicI64::new(i64::MIN);

impl IssuedAt {
    /// The earliest `issued_at` a command may carry: 1970-01-01T00:00:00Z. Nothing legitimate
    /// predates the epoch (a stamp before it means a broken device clock), and a derived id's
    /// UUIDv7 timestamp cannot hold a negative count, so the receiver check (C.4), the apply
    /// check and the id mint all read this one bound (`vault-format.md` C.4 check 6).
    pub const MIN_SUPPORTED_MICROS: i64 = 0;

    /// The latest `issued_at` a command may carry: 9999-12-31T23:59:59.999999Z. Beyond it chrono
    /// would write `+10000-...` and a stored `*_at` text would stop ordering as text.
    pub const MAX_SUPPORTED_MICROS: i64 = 253_402_300_799_999_999;

    /// Whether `micros` lies within [`Self::MIN_SUPPORTED_MICROS`] to
    /// [`Self::MAX_SUPPORTED_MICROS`].
    #[must_use]
    pub const fn micros_supported(micros: i64) -> bool {
        micros >= Self::MIN_SUPPORTED_MICROS && micros <= Self::MAX_SUPPORTED_MICROS
    }

    /// Whether this instant is within the supported range.
    #[must_use]
    pub const fn is_supported(self) -> bool {
        Self::micros_supported(self.0)
    }

    /// The instant `micros microseconds after the Unix epoch (negative before it).
    #[must_use]
    pub const fn from_unix_micros(micros: i64) -> Self {
        Self(micros)
    }

    /// Microseconds since the Unix epoch: the wire value.
    #[must_use]
    pub const fn as_unix_micros(self) -> i64 {
        self.0
    }

    /// Whole milliseconds since the Unix epoch, rounded toward negative infinity. This
    /// is the `unix_ms` field of a derived identifier (ADR 0013-A).
    #[must_use]
    pub const fn as_unix_millis(self) -> i64 {
        self.0.div_euclid(1_000)
    }

    /// The current instant, truncated to microseconds.
    ///
    /// **Strictly increasing within a process**: two calls never return the same value,
    /// and a backward step of the system clock (NTP, a virtualized CI clock) cannot make
    /// a later command appear to be issued before an earlier one. Identifiers derived
    /// from `issued_at` therefore keep creation order (`ORDER BY id`), as
    /// [`crate::monotonic_v7`] does for ordinary identifiers.
    ///
    /// This is the one place the clock is read for a command's `issued_at`; callers build
    /// a command's metadata with it at the command boundary, never inside `apply`.
    #[must_use]
    pub fn now() -> Self {
        let clock = system_micros();
        let mut previous = LAST_ISSUED.load(Ordering::Relaxed);
        loop {
            let next = clock.max(previous.saturating_add(1));
            match LAST_ISSUED.compare_exchange_weak(
                previous,
                next,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Self(next),
                Err(observed) => previous = observed,
            }
        }
    }
}

/// Microseconds since the Unix epoch from the system clock (negative before the epoch).
fn system_micros() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_micros()).unwrap_or(i64::MAX),
        Err(before) => i64::try_from(before.duration().as_micros()).map_or(i64::MIN, |m| -m),
    }
}
