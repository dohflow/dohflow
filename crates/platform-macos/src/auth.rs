//! `LocalAuthentication`: availability and a pre-flight prompt. **Never releases a secret.**
//!
//! The Objective-C binding marks message sends unsafe; each block below says why it is sound.
#![allow(unsafe_code)]

use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use core_foundation::base::{CFType, TCFType};
use core_foundation_sys::base::CFTypeRef;
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2_foundation::{NSError, NSString};
use objc2_local_authentication::{LAContext, LAPolicy};
use platform_traits::{
    AuthPrompt, BiometricAuth, BiometricAvailability, PreflightResult, SecretError,
    UnavailableReason,
};

use crate::availability::availability_from_la_code;

/// Whether a biometric can be used on this Mac right now, asked of the system each time (never
/// cached: a fingerprint can be enrolled or removed while the app runs).
pub(crate) fn current_availability() -> BiometricAvailability {
    current_availability_with_code().0
}

/// [`current_availability`] with the raw `LAError` code that produced it (`None` when a biometric can be
/// used), so a harness or a log line can say WHICH unmapped code a Mac answered.
pub(crate) fn current_availability_with_code() -> (BiometricAvailability, Option<i64>) {
    // SAFETY: a fresh context and a valid policy constant; the context is invalidated before it drops.
    let context = unsafe { LAContext::new() };
    let outcome = unsafe {
        context.canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthenticationWithBiometrics)
    };
    // SAFETY: invalidating a context we own and no longer use.
    unsafe { context.invalidate() };
    let code = outcome.err().map(|error| error.code() as i64);
    (availability_from_la_code(code), code)
}

/// `LAError` codes we map (stable public values).
const LA_AUTHENTICATION_FAILED: i64 = -1;
const LA_USER_CANCEL: i64 = -2;
const LA_BIOMETRY_NOT_AVAILABLE: i64 = -6;
const LA_BIOMETRY_NOT_ENROLLED: i64 = -7;
const LA_BIOMETRY_LOCKOUT: i64 = -8;

const PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(120);

fn map_la_code(code: i64) -> SecretError {
    match code {
        LA_AUTHENTICATION_FAILED => SecretError::AuthFailed,
        LA_USER_CANCEL => SecretError::AuthCancelled,
        LA_BIOMETRY_NOT_AVAILABLE => SecretError::Unavailable(UnavailableReason::Unsupported),
        LA_BIOMETRY_NOT_ENROLLED => SecretError::BiometryNotEnrolled,
        LA_BIOMETRY_LOCKOUT => SecretError::BiometryLockedOut,
        other => SecretError::Platform { code: other },
    }
}

/// A fresh `LAContext` carrying the prompt's reason, as the CF object the keychain query takes
/// (`kSecUseAuthenticationContext`). One context per load: a context that has already authenticated
/// is reused by the system for a short time, so sharing one would skip the prompt.
pub(crate) fn authentication_context(prompt: &AuthPrompt) -> CFType {
    // SAFETY: `LAContext` has a plain `init`; `setLocalizedReason:` takes an `NSString` we own.
    let context = unsafe { LAContext::new() };
    unsafe { context.setLocalizedReason(&NSString::from_str(prompt.reason())) };
    let pointer: *const LAContext = Retained::into_raw(context);
    // SAFETY: an Objective-C object is a valid CFTypeRef for retain/release (toll-free); we hand the
    // +1 reference we just took out of `Retained` to `wrap_under_create_rule`, which releases it.
    unsafe { CFType::wrap_under_create_rule(pointer as CFTypeRef) }
}

/// Invalidates a borrowed `LAContext` when dropped.
struct InvalidateOnDrop<'a>(&'a Retained<LAContext>);

impl Drop for InvalidateOnDrop<'_> {
    fn drop(&mut self) {
        // SAFETY: `invalidate` is safe to call on a live context, and on one whose evaluation has
        // already finished it does nothing.
        unsafe { self.0.invalidate() };
    }
}

/// [`BiometricAuth`] on `LocalAuthentication` (biometrics only, no passcode fallback).
pub struct MacBiometricAuth;

impl MacBiometricAuth {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// The availability and the raw `LAError` code behind it (`None` when a biometric can be used).
    #[must_use]
    pub fn availability_with_code(&self) -> (BiometricAvailability, Option<i64>) {
        current_availability_with_code()
    }
}

impl Default for MacBiometricAuth {
    fn default() -> Self {
        Self::new()
    }
}

impl BiometricAuth for MacBiometricAuth {
    fn availability(&self) -> BiometricAvailability {
        current_availability()
    }

    fn preflight_prompt(&self, prompt: &AuthPrompt) -> Result<PreflightResult, SecretError> {
        // SAFETY: a fresh context.
        let context = unsafe { LAContext::new() };
        // However this call ends (a match, a cancel, a failure, a timeout or an early return), the
        // context is invalidated, so an abandoned prompt is torn down and no authenticated context
        // lingers for the system to reuse (xuu N2).
        let _invalidate = InvalidateOnDrop(&context);
        // SAFETY: as above.
        if let Err(error) = unsafe {
            context.canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthenticationWithBiometrics)
        } {
            return Err(map_la_code(error.code() as i64));
        }
        let (sender, receiver) = mpsc::channel::<Result<(), i64>>();
        let reply = RcBlock::new(move |success: Bool, error: *mut NSError| {
            let outcome = if success.as_bool() {
                Ok(())
            } else {
                // SAFETY: on failure the framework passes a valid NSError (or null).
                Err(unsafe { error.as_ref() }.map_or(LA_AUTHENTICATION_FAILED, |e| e.code() as i64))
            };
            let _ = sender.send(outcome);
        });
        // SAFETY: the block and reason outlive the call; the reply runs on an arbitrary queue, which
        // is why the result is handed back through a channel.
        unsafe {
            context.evaluatePolicy_localizedReason_reply(
                LAPolicy::DeviceOwnerAuthenticationWithBiometrics,
                &NSString::from_str(prompt.reason()),
                &reply,
            );
        }
        match receiver.recv_timeout(PREFLIGHT_TIMEOUT) {
            Ok(Ok(())) => Ok(PreflightResult::Matched),
            Ok(Err(code)) => Err(map_la_code(code)),
            Err(_) => Err(SecretError::AuthFailed),
        }
    }
}
