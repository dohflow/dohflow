/// How the shared modal dialog presents itself on a compact screen (personal-cfo-i93n3.25 slice B): a
/// bottom sheet. Every utility here is behind `compact:`, so a desktop window renders the centered card
/// exactly as before.
///
/// - Anchored to the bottom edge and full width (`inset-x-0 bottom-0`), undoing the card's centering
///   (`top-auto`, no translate) and its side margins and width cap.
/// - Never taller than the visible screen minus the device chrome at the top
///   (`100dvh - --safe-area-top`); the dialog's body is the region that scrolls, so a tall form fills
///   the screen and a short confirmation stays short.
/// - Its own bottom padding is the larger of the home-indicator inset and the keyboard, never their sum
///   (`--keyboard-inset` is set by `useKeyboardInset()`), so the actions stay above the keyboard. The
///   sides clear the left and right insets in landscape.
/// - Rounded on top only, with no border on the other three edges.
export const COMPACT_SHEET = [
  "compact:inset-x-0",
  "compact:top-auto",
  "compact:bottom-0",
  "compact:left-0",
  "compact:w-full",
  "compact:max-w-none",
  "compact:translate-x-0",
  "compact:translate-y-0",
  "compact:max-h-[calc(100dvh-var(--safe-area-top))]",
  "compact:rounded-b-none",
  "compact:rounded-t-2xl",
  "compact:border-x-0",
  "compact:border-b-0",
  "compact:pl-(--safe-area-left)",
  "compact:pr-(--safe-area-right)",
  "compact:pb-[max(var(--safe-area-bottom),var(--keyboard-inset))]",
].join(" ");

/// The hand-rolled side drawers (transaction detail, bill detail, account editor) are already full
/// height and full width on a phone; what they lack is clearance from the device chrome. Add these to
/// the drawer's panel: the first for a panel that pads itself with `p-6` (the inset is added to that
/// padding), the second for one whose own padding is on its inner rows (the panel just clears the
/// chrome). The bottom edge is the larger of the home indicator and the keyboard, never their sum.
/// Every variable is 0px on a desktop window and behind `compact:`, so nothing moves there.
export const COMPACT_DRAWER_PADDED_6 = [
  "compact:pt-[calc(1.5rem+var(--safe-area-top))]",
  "compact:pr-[calc(1.5rem+var(--safe-area-right))]",
  "compact:pb-[calc(1.5rem+max(var(--safe-area-bottom),var(--keyboard-inset)))]",
].join(" ");

export const COMPACT_DRAWER_CLEARS_CHROME = [
  "compact:pt-(--safe-area-top)",
  "compact:pr-(--safe-area-right)",
  "compact:pb-[max(var(--safe-area-bottom),var(--keyboard-inset))]",
].join(" ");

/// A full-screen presentation of the shared dialog on a compact screen, for the navigation dialog
/// (personal-cfo-i93n3.25 slice C): pass it as `className` to `DialogContent`, after `COMPACT_SHEET`'s
/// classes, so each of these wins over the sheet's (same variant, same property; tailwind-merge keeps
/// the last). It fills the screen, with the device chrome cleared on every edge (the sheet already
/// pads the sides and the bottom; this adds the top) and no rounded corners.
export const COMPACT_FULLSCREEN = [
  "compact:top-0",
  "compact:h-dvh",
  "compact:max-h-dvh",
  "compact:rounded-none",
  "compact:border-0",
  "compact:pt-(--safe-area-top)",
].join(" ");

/// On a phone every button inside a surface may wrap its label and grow taller (personal-cfo-i93n3.28 slice 8). The shared `Button` is nowrap
/// and fixed-height, so a long label ("Run the setup guide again", "Split transaction") runs past the screen at the largest text size.
/// One rule on the surface's root instead of a class at every call site; the 44 pt floor in `globals.css` (a `min-height`, unlayered) still holds.
export const COMPACT_WRAP_BUTTONS =
  "compact:[&_button]:h-auto compact:[&_button]:max-w-full compact:[&_button]:whitespace-normal";
