//! Protected secret buffers (personal-cfo-1t0).
//!
//! [`SecretBytes`] is the storage primitive for app-controlled key material —
//! the KEK today, and the DEK / per-attachment content keys / password buffer
//! as `personal-cfo-vhv` introduces them. It provides three defences:
//!
//! 1. **Zeroize on drop.** The backing bytes are overwritten with zeroes before
//!    the allocation is freed, so freed key material does not linger in the heap.
//! 2. **Best-effort `mlock`.** The page(s) backing the secret are locked into
//!    RAM so they are not written to swap. This is *best-effort* (see limits
//!    below) and never fatal.
//! 3. **No accidental copies.** The type is deliberately **not** `Copy` and
//!    **not** `Clone`, so the compiler rejects silent duplication of a key into
//!    an un-scrubbed buffer (the type-system half of 1t0's "lints catch direct
//!    copies of key types").
//!
//! # OS / library limits (documented per the 1t0 acceptance criteria)
//!
//! - `mlock` locks whole pages, and the OS bounds the total locked memory via
//!   `RLIMIT_MEMLOCK`. If the limit is exhausted (or the platform is not Unix),
//!   the lock silently fails and the buffer is still zeroized — confidentiality
//!   at rest is unaffected, only the anti-swap hardening is skipped.
//! - Because locking is page-granular, two secrets sharing a page share a lock;
//!   dropping one `munlock`s the shared page. A dedicated guard-page allocator
//!   would avoid this and is tracked as a deeper hardening follow-up.
//! - `mlock` does **not** protect against core dumps, hibernation images, or an
//!   attacker with the process's address space. Those are out of scope here.
//! - [`SecretBytes::new`] takes the key by value, so the caller's array (and the argument
//!   slot) can leave a copy on the stack that nothing scrubs. Prefer
//!   [`SecretBytes::init`] and [`SecretBytes::from_slice`], which hand the closure the
//!   protected buffer itself: the key is generated or decrypted **in place** and no
//!   un-scrubbed stack array ever holds it (personal-cfo-xzpra). `new` stays for
//!   callers that already hold a value they must keep (tests).

use zeroize::Zeroize;

/// A fixed-size secret byte buffer: heap-backed, best-effort page-locked, and
/// zeroized on drop. Not `Copy`, not `Clone`.
pub struct SecretBytes<const N: usize> {
    /// Heap allocation so the address is stable for `mlock`/`munlock`.
    bytes: Box<[u8; N]>,
    /// Whether `mlock` succeeded, so we only `munlock` what we locked.
    locked: bool,
}

impl<const N: usize> SecretBytes<N> {
    /// Move `bytes` into a protected, page-locked, zeroizing allocation.
    ///
    /// The caller's original `bytes` is `Copy` array data; scrub it after the
    /// call if it held real key material.
    #[must_use]
    pub fn new(bytes: [u8; N]) -> Self {
        let boxed = Box::new(bytes);
        let locked = lock_pages(boxed.as_ptr(), N);
        Self {
            bytes: boxed,
            locked,
        }
    }

    /// Build a secret **in place**: allocate the protected buffer zeroed, lock it, and let
    /// `fill` write the key straight into it. No intermediate array holds the key, so there is
    /// no stack copy to scrub. If `fill` fails the partly written buffer is zeroized and
    /// released by the normal drop.
    ///
    /// # Errors
    /// Whatever `fill` returns.
    pub fn init<E>(fill: impl FnOnce(&mut [u8; N]) -> Result<(), E>) -> Result<Self, E> {
        let boxed = Box::new([0_u8; N]);
        let locked = lock_pages(boxed.as_ptr(), N);
        let mut secret = Self {
            bytes: boxed,
            locked,
        };
        fill(&mut secret.bytes)?;
        Ok(secret)
    }

    /// Copy `bytes` into a protected buffer in place. `None` unless `bytes` is exactly `N` long.
    /// The caller still owns (and must scrub) the slice it passed.
    #[must_use]
    pub fn from_slice(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != N {
            return None;
        }
        Self::init::<core::convert::Infallible>(|buffer| {
            buffer.copy_from_slice(bytes);
            Ok(())
        })
        .ok()
    }

    /// Borrow the protected bytes. Callers must not copy these into an
    /// un-scrubbed buffer.
    #[must_use]
    pub fn expose(&self) -> &[u8; N] {
        &self.bytes
    }

    /// Whether the backing pages are currently `mlock`ed. Best-effort; used by
    /// tests and diagnostics, never a security guarantee.
    #[must_use]
    pub fn is_locked(&self) -> bool {
        self.locked
    }
}

impl<const N: usize> Zeroize for SecretBytes<N> {
    fn zeroize(&mut self) {
        self.bytes.zeroize();
    }
}

impl<const N: usize> Drop for SecretBytes<N> {
    fn drop(&mut self) {
        // Scrub first, then release the page lock, then free.
        self.bytes.zeroize();
        if self.locked {
            unlock_pages(self.bytes.as_ptr(), N);
            self.locked = false;
        }
    }
}

impl<const N: usize> core::fmt::Debug for SecretBytes<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SecretBytes([REDACTED])")
    }
}

/// Lock `len` bytes starting at `ptr` into RAM. Returns whether the lock was
/// established. Best-effort: a failure (e.g. `RLIMIT_MEMLOCK`) is not an error.
#[cfg(unix)]
fn lock_pages(ptr: *const u8, len: usize) -> bool {
    if len == 0 {
        return false;
    }
    // SAFETY: `ptr`/`len` describe a live allocation owned by the caller for the
    // lifetime of the lock; `mlock` only pins those pages in RAM.
    unsafe { libc::mlock(ptr.cast(), len) == 0 }
}

/// Release a lock previously established by [`lock_pages`].
#[cfg(unix)]
fn unlock_pages(ptr: *const u8, len: usize) {
    if len == 0 {
        return;
    }
    // SAFETY: same allocation that was locked; `munlock` only unpins the pages.
    unsafe {
        let _ = libc::munlock(ptr.cast(), len);
    }
}

/// Non-Unix fallback: page locking is unavailable, so this is a documented
/// no-op. Zeroize-on-drop and the no-copy guarantees still hold.
#[cfg(not(unix))]
fn lock_pages(_ptr: *const u8, _len: usize) -> bool {
    false
}

#[cfg(not(unix))]
fn unlock_pages(_ptr: *const u8, _len: usize) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expose_returns_stored_bytes() {
        let s = SecretBytes::new([0xABu8; 32]);
        assert_eq!(s.expose(), &[0xABu8; 32]);
    }

    #[test]
    fn zeroize_scrubs_the_buffer() {
        let mut s = SecretBytes::new([0x11u8; 16]);
        assert_ne!(s.expose(), &[0u8; 16]);
        s.zeroize();
        assert_eq!(s.expose(), &[0u8; 16]);
    }

    #[test]
    fn construction_never_panics_regardless_of_mlock_support() {
        // Whether or not mlock succeeds (CI may cap RLIMIT_MEMLOCK), this must
        // construct and expose cleanly.
        let s = SecretBytes::new([7u8; 64]);
        assert_eq!(s.expose()[0], 7);
        // `is_locked()` is informational; we don't assert its value because it
        // is environment-dependent.
        let _ = s.is_locked();
    }

    #[test]
    fn debug_is_redacted() {
        let s = SecretBytes::new([0u8; 8]);
        assert_eq!(format!("{s:?}"), "SecretBytes([REDACTED])");
    }

    #[test]
    fn init_fills_the_protected_buffer_in_place() {
        let s = SecretBytes::<32>::init::<()>(|buf| {
            for (i, b) in buf.iter_mut().enumerate() {
                *b = u8::try_from(i).unwrap();
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(s.expose()[31], 31);
        assert_eq!(s.expose().iter().map(|b| u32::from(*b)).sum::<u32>(), 496);
    }

    #[test]
    fn init_failure_returns_the_error_and_drops_the_partial_buffer() {
        // The closure sees a zeroed buffer, may write part of a key, and fails: the error
        // comes back and no secret exists.
        let result = SecretBytes::<16>::init(|buf| {
            assert!(buf.iter().all(|b| *b == 0), "the buffer starts zeroed");
            buf[0] = 0xFF;
            Err("rng unavailable")
        });
        assert_eq!(result.err(), Some("rng unavailable"));
    }

    #[test]
    fn from_slice_requires_the_exact_length() {
        assert!(SecretBytes::<8>::from_slice(&[1; 7]).is_none());
        assert!(SecretBytes::<8>::from_slice(&[1; 9]).is_none());
        assert!(SecretBytes::<8>::from_slice(&[]).is_none());
        let s = SecretBytes::<8>::from_slice(&[9; 8]).unwrap();
        assert_eq!(s.expose(), &[9; 8]);
    }
}
