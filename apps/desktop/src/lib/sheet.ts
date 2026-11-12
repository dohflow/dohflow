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

/// Reflow for a sheet that does not fit (personal-cfo-i93n3.25 slice B, Studio's review: at the largest text
/// size a maximum-length scenario name wrapped the title to about nine lines, the pinned title and footer left
/// the body a 51pt window, the title's first lines sat above the sheet's top edge and the destructive label was
/// cut off). Three behind-`compact:` rules, none of which changes a desktop window or a sheet that fits:
///
/// - **Cramped sheets scroll as a whole.** When the title, the footer and the error slot leave the body fewer
///   than `CRAMPED_BODY_LINES` lines of height (`bodyIsStarved`, measured by `DialogBody`), the sheet sets
///   `data-cramped`: the sheet itself scrolls (`COMPACT_SHEET_CRAMPED`) and the body stops being its own tiny
///   scroll region (`COMPACT_BODY_CRAMPED`), so the title, the description and the actions all read in full and
///   are reached by scrolling, as an iOS alert does at accessibility sizes. A sheet that fits keeps its pinned
///   footer and its own scrolling body, exactly as before.
/// - **Footer buttons may wrap** (`COMPACT_FOOTER_WRAPS`): the shared `Button` is `h-10 whitespace-nowrap`, which
///   cut "Delete scenario" off at both edges. Inside a footer a button takes `h-auto min-h-10 whitespace-normal
///   [overflow-wrap:anywhere]` (FRONTEND.md section 7.6, "Reflow, not clipping"); a label that fits renders the same.
export const COMPACT_SHEET_CRAMPED = "compact:data-[cramped]:overflow-y-auto";

export const COMPACT_BODY_CRAMPED = "compact:flex-none compact:overflow-visible";

export const COMPACT_FOOTER_WRAPS = [
  "compact:[&_button]:h-auto",
  "compact:[&_button]:min-h-10",
  "compact:[&_button]:whitespace-normal",
  "compact:[&_button]:[overflow-wrap:anywhere]",
].join(" ");

/// How many lines of body text a sheet must leave visible before the body is a usable scroll region.
export const CRAMPED_BODY_LINES = 3;

/// True when what is left for the body (the sheet's height cap, less its padding and border, less every other
/// child's height) is under `CRAMPED_BODY_LINES` lines of text. Pure, so a test can pin the threshold.
export function bodyIsStarved(input: {
  limit: number;
  chrome: number;
  others: number;
  lineHeight: number;
}): boolean {
  const available = input.limit - input.chrome - input.others;
  return available < CRAMPED_BODY_LINES * input.lineHeight;
}
