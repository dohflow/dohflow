//! Boot identity and a monotonic clock for the rules that decide when the vault password is required
//! again (`personal-cfo-xuu` AC K1 and K3). **Not the anti-rollback witness** (ADR 0017).
//!
//! * `boot_id`: `sysctl kern.bootsessionuuid`, the per-boot random UUID the kernel generates once at
//!   start-up (16 bytes). Equal for the whole boot and different after a restart.
//! * `monotonic_ns`: `mach_continuous_time` scaled by the timebase. It counts from this boot, never goes
//!   backwards, and keeps counting while the Mac sleeps; changing the wall clock cannot move it.
#![allow(unsafe_code)]

use platform_traits::{BootClock, BootClockError, BootId};

use crate::boot_session::parse_boot_session_uuid;

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

/// [`BootClock`] on `kern.bootsessionuuid` and `mach_continuous_time`.
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
        // `kern.bootsessionuuid` is a NUL-terminated string of 36 characters; the buffer has room for
        // a little more so an unexpected value is seen as the wrong length, not truncated into a valid one.
        let mut buffer = [0_u8; 64];
        let mut size = buffer.len();
        // SAFETY: the name is a NUL-terminated string; `buffer` is a valid out-buffer of `size` bytes and
        // `size` is a valid in/out length; no new value is set (null, 0).
        let status = unsafe {
            libc::sysctlbyname(
                c"kern.bootsessionuuid".as_ptr(),
                buffer.as_mut_ptr().cast::<libc::c_void>(),
                &raw mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if status != 0 || size == 0 || size > buffer.len() {
            return Err(BootClockError);
        }
        let text = std::str::from_utf8(&buffer[..size]).map_err(|_| BootClockError)?;
        parse_boot_session_uuid(text)
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
