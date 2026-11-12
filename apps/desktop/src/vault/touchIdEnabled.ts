/// Whether the app shows any Touch ID control: the Settings card and the unlock screen's button.
///
/// v0.4.1 ships with this off (`personal-cfo-p3f7r`). Turning Touch ID on failed on every Mac with a
/// sensor in the 0.4.0 build (`personal-cfo-xuu`, being fixed), so no Touch ID item can be created
/// from the UI until the fix is released. The kernel and the host code are untouched and still tested,
/// so the release that carries the fix (v0.4.2) flips this one constant and nothing else.
///
/// It is a plain constant, not a setting: nothing reads it from the vault, the host or the network,
/// and a build either has the controls or does not. `as boolean` keeps the compiler from treating the
/// guarded code as unreachable, so the code that comes back with the flip is still type-checked.
export const TOUCH_ID_ENABLED = false as boolean;
