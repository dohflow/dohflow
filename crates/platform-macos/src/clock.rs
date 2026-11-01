//! Boot identity and a monotonic clock for the rules that decide when the vault password is required
//! again (`personal-cfo-xuu` AC K1 and K3). **Not the anti-rollback witness** (ADR 0017).
//!
//! * `boot_id`: `sysctl kern.boottime` (the wall-clock time this boot began, as a `timeval`), encoded as
//!   seconds and microseconds, 16 bytes. Equal for the whole boot; a restart changes it.
//! * `monotonic_ns`: `mach_continuous_time` scaled by the timebase. It counts from this boot, never goes
//!   backwards, and keeps counting while the Mac sleeps; changing the wall clock cannot move it.
#![allow(unsafe_code)]

use platform_traits::{BootClock, BootClockError, BootId};

/// `mach_timebase_info`: ticks to nanoseconds is `ticks * numer / denom`.
#[repr(C)]
struct TimebaseInfo {
    numer: u32,
    denom: u32,
}

extern "C" {
    /// Mach ticks since boot INCLUDING time spent asleep (`mach_absolute_time` stops during sleep).
    fn mach_continuous_time() -> u64;
    fn mach_timebase_info(info: *mut TimebaseInfo) -> i32;
}

/// [`BootClock`] on `kern.boottime` and `mach_continuous_time`.
#[derive(Debug, Default, Clone, Copy)]
pub struct MacBootClock;

impl MacBootClock {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl BootClock for MacBootClock {
    fn boot_id(&self) -> Result<BootId, BootClockError> {
        let mut boot = libc::timeval {
            tv_sec: 0,
            tv_usec: 0,
        };
        let mut size = std::mem::size_of::<libc::timeval>();
        // SAFETY: the name is a NUL-terminated string; `boot` is a valid out-buffer of `size` bytes and
        // `size` is a valid in/out length; no new value is set (null, 0).
        let status = unsafe {
            libc::sysctlbyname(
                c"kern.boottime".as_ptr(),
                (&raw mut boot).cast::<libc::c_void>(),
                &raw mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if status != 0 || size != std::mem::size_of::<libc::timeval>() || boot.tv_sec == 0 {
            return Err(BootClockError);
        }
        let mut bytes = [0_u8; 16];
        bytes[..8].copy_from_slice(&boot.tv_sec.to_le_bytes());
        bytes[8..].copy_from_slice(&i64::from(boot.tv_usec).to_le_bytes());
        Ok(BootId::new(bytes))
    }

    fn monotonic_ns(&self) -> Result<u64, BootClockError> {
        let mut timebase = TimebaseInfo { numer: 0, denom: 0 };
        // SAFETY: `timebase` is a valid out-pointer for the call.
        let status = unsafe { mach_timebase_info(&raw mut timebase) };
        if status != 0 || timebase.denom == 0 {
            return Err(BootClockError);
        }
        // SAFETY: no arguments; reads the system's continuous clock.
        let ticks = unsafe { mach_continuous_time() };
        let nanoseconds =
            u128::from(ticks) * u128::from(timebase.numer) / u128::from(timebase.denom);
        u64::try_from(nanoseconds).map_err(|_| BootClockError)
    }
}
