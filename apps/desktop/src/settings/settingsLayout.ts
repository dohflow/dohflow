/// The Settings page's span and lane primitives (personal-cfo-4d8.39.1; docs/agent/FRONTEND.md
/// section 7.4 and 7.5).
///
/// Everything here is a function of the width of the MAIN PANE, not the viewport: `C` is the window
/// width minus the sidebar (224px expanded, 56px collapsed) minus the 48px of main-pane insets. The
/// page's root is a CSS container, and these classes are container queries against it, so a narrow
/// pane next to an expanded sidebar lays out as narrow whatever the window is.
///
/// The grid has SIX equal tracks and a 16px gap, so a "third" is two tracks, a "half" is three, and
/// both are equal parts of the whole width (a half is not two thirds of anything):
///
///   - three columns from C = 992, two from C = 656, otherwise one (a closed tile needs 320px);
///   - a span is written as its narrowest layout first and widens at the thresholds, so the
///     DOM order, which is also the reading and Tab order, never changes.
///
/// The class strings are literals on purpose: Tailwind finds a class by scanning the source for its
/// complete name, so they cannot be assembled at run time.

/// The container widths the page layout turns over at.
export const THREE_COLUMNS_FROM = 992;
export const TWO_COLUMNS_FROM = 656;

/// A section's width in the six-track grid, per container width.
export const SPAN = {
  /// The whole width at every C.
  full: "col-span-6",
  /// One of three columns from 992, otherwise the whole width.
  oneOfThree: "col-span-6 @min-[992px]:col-span-2",
  /// Two of three columns from 992, otherwise the whole width.
  twoOfThree: "col-span-6 @min-[992px]:col-span-4",
  /// Half the width from 656 (and from 992: two equal tracks across the whole width), otherwise the
  /// whole width.
  half: "col-span-6 @min-[656px]:col-span-3",
} as const;

export type SpanName = keyof typeof SPAN;

/// A preference card's lanes, by the CARD's own width (not the pane's): three from 866, two from
/// 586, otherwise one. Its own container, so it lays out the same wherever the card sits.
export const PREFERENCE_LANES = {
  three: 866,
  two: 586,
  classes: "grid grid-cols-1 gap-4 @min-[586px]:grid-cols-2 @min-[866px]:grid-cols-3",
} as const;

/// The Comfort editor's two fields sit side by side from 480px of usable editor body width and stack
/// below it. Its own container, for the card to apply to its fields.
export const EDITOR_STACK_BELOW = 480;
export const EDITOR_FIELDS_CLASSES = "flex flex-col gap-4 @min-[480px]:flex-row";
