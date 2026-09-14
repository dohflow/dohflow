/// How strongly the forecast's uncertainty band is drawn (personal-cfo-s0iz).
///
/// Its own module rather than exports beside the chart: a non-component export from a
/// component file adds a `react-refresh/only-export-components` warning, and the repo
/// keeps that baseline flat (the `seriesKeys.ts` pattern).
///
/// These are not taste. The band is the whole mechanism by which the projected half
/// declares itself uncertain — if it washes out, the chart silently becomes a confident
/// line. `bandLegibility.test.ts` composites each value over the SHIPPED surfaces using
/// the SHIPPED chart tokens and asserts the result clears a perceptual floor, so darkening
/// a surface or dulling a token fails a test instead of quietly erasing the band.

/// Fill strength, both themes.
///
/// Raised from 0.10. The mock proposed raising DARK only, on the general principle that a
/// dark surface loses more contrast at the same alpha. Measured against this palette that
/// premise is false — the dark theme swaps in brighter chart tokens, so at 0.10 the
/// weakest case was LIGHT-mode terracotta (ΔE 3.4 vs the card), while dark ranged
/// 4.7–5.3. A dark-only bump would have strengthened the half that was already ahead and
/// left the actual floor untouched.
export const BAND_FILL_OPACITY = 0.15;

/// Stipple strength for the band's edge.
///
/// Deliberately far stronger than the fill: this is the part that survives when the wash
/// is lost against a gradient, a gridline, or a bright screen.
export const BAND_EDGE_OPACITY = 0.45;

/// Dotted, not solid.
///
/// A percentile edge is not a promise about where the range ends. A solid boundary would
/// claim it is — that outcomes stop there — which is exactly the false precision the band
/// exists to avoid.
export const BAND_EDGE_DASH = "2 3";

/// The perceptual floor the fill must clear against its own surface, in OKLab ΔE × 100.
///
/// Set just under the measured worst case (light terracotta at 5.14) so the guard passes
/// today and fails on a real regression rather than on rounding.
export const BAND_FILL_FLOOR = 5;
