use std::fmt;

/// Identifies one boot of the device (on macOS, `kern.bootsessionuuid`, the per-boot random UUID, 16
/// bytes). Equal for every call during one boot and different after a restart. **Not secret.**
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BootId([u8; 16]);

impl BootId {
    #[must_use]
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Debug for BootId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BootId(..)")
    }
}

/// The platform could not say. A fixed value with no text: a caller treats ANY error as "unknown" and
/// fails closed (it asks for the vault password).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootClockError;

impl fmt::Display for BootClockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the boot clock is unavailable")
    }
}

impl std::error::Error for BootClockError {}

/// Boot identity and a monotonic clock, for the rules that decide when the vault password is required
/// again (`personal-cfo-xuu` AC K1 and K3). **It is not the anti-rollback witness** (ADR 0017).
///
/// * [`BootClock::boot_id`] changes on every restart, so a value recorded at the last password unlock
///   that no longer matches means "the Mac restarted since".
/// * [`BootClock::monotonic_ns`] counts nanoseconds since this boot, never goes backwards and keeps
///   counting while the Mac sleeps (macOS `mach_continuous_time`). A change of the wall clock cannot
///   move it, which is why the 30-day rule uses it. It is comparable only within one boot, and that is
///   enough: any restart already requires the password.
pub trait BootClock: Send + Sync {
    /// # Errors
    /// [`BootClockError`] when the platform cannot answer.
    fn boot_id(&self) -> Result<BootId, BootClockError>;

    /// # Errors
    /// [`BootClockError`] when the platform cannot answer.
    fn monotonic_ns(&self) -> Result<u64, BootClockError>;
}
