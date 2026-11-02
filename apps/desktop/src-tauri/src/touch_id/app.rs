//! The flows the IPC commands call (`personal-cfo-xuu`). Key-bearing code stays here: the commands in
//! `src/ipc` take and return only plain values (a password in a `Zeroizing`, a status, a reason code).

use finance_kernel::{Kernel, KernelError, UnlockOptions, VaultState};
use platform_traits::SecretError;
use zeroize::Zeroizing;

use super::{Attempt, Reason, Status};
use crate::ipc::commands::active_vault_id;
use crate::state::AppState;

fn poisoned() -> KernelError {
    KernelError::Persistence("application state lock was poisoned".to_owned())
}

/// The keychain account of the active vault: its registry id, or a fixed name for a host with no
/// registry. One item per vault, so switching vaults never reads another vault's item.
fn account(state: &AppState) -> String {
    active_vault_id(state).unwrap_or_else(|| super::default_account().to_owned())
}

/// The fingerprint of the wrapped key in the active vault's envelope, read without unlocking.
fn fingerprint(state: &AppState) -> Option<[u8; 32]> {
    let path = state.lock_controller().ok()?.path().to_path_buf();
    Kernel::envelope_fingerprint(path).ok()
}

/// Touch ID's status for the active vault. Reads no secret and shows no prompt.
#[must_use]
pub fn status(state: &AppState) -> Status {
    state.touch_id().status(&account(state), fingerprint(state))
}

/// How enabling ended when the password was right.
#[derive(Debug, PartialEq, Eq)]
pub enum EnableOutcome {
    Enabled,
    /// Touch ID could not be enabled, and why. The vault stays password-only and any older item is gone.
    NotEnabled(Reason),
}

/// Enables Touch ID for the unlocked active vault. The password is checked again (a standing unlock path
/// is worth one more entry); a wrong password changes nothing.
///
/// # Errors
/// The kernel's error: a wrong password, a vault that is not unlocked, an envelope that changed.
pub fn enable(
    state: &AppState,
    password: &Zeroizing<String>,
) -> Result<EnableOutcome, KernelError> {
    let material = {
        let guard = state.lock_controller().map_err(|_| poisoned())?;
        guard.enrollment_unlock_material(password.as_bytes())?
    };
    Ok(match state.touch_id().install(&account(state), &material) {
        Ok(()) => EnableOutcome::Enabled,
        Err(reason) => EnableOutcome::NotEnabled(reason),
    })
}

/// Turns Touch ID off for the active vault (idempotent; a build with no keychain has nothing to remove).
///
/// # Errors
/// The store's error when an item exists and cannot be removed.
pub fn disable(state: &AppState) -> Result<(), SecretError> {
    state.touch_id().retire(&account(state))
}

/// One Touch ID unlock attempt for the locked active vault, with the host's per-vault options.
pub fn unlock(state: &AppState, options: UnlockOptions) -> Attempt {
    // A vault that is already unlocked (or in any state the unlock would refuse) gets the same typed
    // refusal the unlock itself would give, BEFORE anything is prompted or counted.
    match state.lock_controller() {
        Ok(guard) => {
            let from = guard.state();
            if !from.can_transition_to(VaultState::Unlocking) {
                return Attempt::Failed(KernelError::IllegalVaultTransition {
                    from,
                    to: VaultState::Unlocking,
                });
            }
        }
        Err(_) => return Attempt::Failed(poisoned()),
    }
    state
        .touch_id()
        .unlock(&account(state), fingerprint(state), |material| {
            let mut guard = state.lock_controller().map_err(|_| poisoned())?;
            guard.unlock_with_released(material, options)
        })
}

/// After a successful vault PASSWORD unlock: the markers are re-stamped (this boot, now, no failed
/// prompts), or the item is removed if it no longer matches the envelope.
pub fn note_password_unlock(state: &AppState) {
    state
        .touch_id()
        .note_password_unlock(&account(state), fingerprint(state));
}

/// Removes the active vault's item if its markers are unreadable or no longer match the envelope. Needs
/// no prompt and no unlock; run at launch.
pub fn sweep(state: &AppState) {
    state.touch_id().sweep(&account(state), fingerprint(state));
}

/// Before the vault's key material changes (a password change, a key rotation) or the vault is deleted:
/// the item is removed first. A failure stops the change: an item that cannot be removed would be left
/// able to unwrap a key that is about to be retired.
///
/// # Errors
/// The store's error when an item exists and cannot be removed.
pub fn retire_before_change(state: &AppState) -> Result<(), SecretError> {
    state.touch_id().retire(&account(state))
}

/// After the change succeeded: the item is removed again (a Touch ID enable that raced the change cannot
/// survive it). It cannot be undone, so a failure is only logged; the fingerprint check retires the item
/// at the next launch or unlock anyway.
pub fn retire_after_change(state: &AppState) {
    if state.touch_id().retire(&account(state)).is_err() {
        tracing::warn!(
            outcome = "failed",
            "could not remove the Touch ID item after a key change"
        );
    }
}
