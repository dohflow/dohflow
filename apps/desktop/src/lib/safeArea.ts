/// Padding that keeps a full-screen container clear of the device chrome (the notch and the home
/// indicator) and of the on-screen keyboard (personal-cfo-i93n3.25). Add it to the outermost element of
/// a screen that does not already pad itself; where it does, add the same variables to its own padding
/// with `calc()`.
///
/// Every variable is zero on a desktop window (see `globals.css`), so this changes nothing there. The
/// bottom edge takes the larger of the home-indicator inset and the keyboard, never their sum: the
/// keyboard already covers the indicator.
export const SAFE_AREA_PADDING =
  "pt-(--safe-area-top) pr-(--safe-area-right) pb-[max(var(--safe-area-bottom),var(--keyboard-inset))] pl-(--safe-area-left)";
