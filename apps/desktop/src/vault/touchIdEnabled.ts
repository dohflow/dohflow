/// Whether the app shows any Touch ID control: the Settings card and the unlock screen's button.
///
/// v0.4.1 shipped with this off (`personal-cfo-p3f7r`): turning Touch ID on failed on every Mac with a
/// sensor in the 0.4.0 build, because the unlock markers sat on the biometric keychain item and reading
/// them is refused without a prompt (`personal-cfo-xuu`). v0.4.2 turns it back on with the fix: the
/// markers live on a sibling keychain item, and a refused store says which step and status refused.
///
/// It is a plain constant, not a setting: nothing reads it from the vault, the host or the network,
/// and a build either has the controls or does not. `as boolean` keeps the compiler from treating the
/// guarded code as unreachable, so the code behind it is type-checked whichever way it is set.
export const TOUCH_ID_ENABLED = true as boolean;
