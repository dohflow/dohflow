import type { ScenarioDto } from "@/bindings";

/// How an APPLIED scenario reads in the list (personal-cfo-kkxu, from the Scenarios mock).
///
/// # Applied is brand, never warning
///
/// This is a semantic decision, not styling taste. Applying a scenario is something the
/// user **chose**, and ADR 0055 makes it reversible by construction — promotion inserts
/// rows and revert clears them, destroying nothing. Painting it amber would read as a
/// problem to fix and push people to revert something working exactly as intended.
///
/// So the treatment says *live and deliberate*: a solid brand spine down the left edge and
/// a faint brand tint. The badge beside it already uses the `gain` token, and
/// `appliedRow.test.ts` asserts no warning or loss token appears anywhere in the applied
/// treatment — the rule is easy to break later by copying a row style from elsewhere.
///
/// # The spine carries it; the tint reinforces
///
/// Measured against the shipped surfaces, the tint at 5% separated an applied row from a
/// plain one by only ΔE 2.8 light / 2.4 dark — dark the weaker, so unlike the forecast
/// band (personal-cfo-s0iz) the mock's dark-mode concern DOES hold here. Raised to 10%
/// (5.6 / 4.7) — also a standard Tailwind step, so it cannot silently emit nothing the way
/// an off-scale value can.
///
/// It is still a modest number, and deliberately so. Unlike the band, the tint is not the
/// whole mechanism: the SPINE is solid `--primary` at full opacity and is unmissable, with
/// the gain badge and since-line beside it. A row wash that shouted would make a list of
/// applied scenarios unreadable.
///
/// That is also why `appliedRow.test.ts` uses a lower floor than the band guard rather
/// than reusing 5. Perceptual thresholds depend on the size of the field being judged — a
/// 1px stipple and a full table row are different stimuli, and a large area is easier to
/// discriminate at the same ΔE. The test asserts the spine is present first, because that
/// is the signal doing the work.
///
/// # It composes with archived rather than replacing it
///
/// Applied-ness is orthogonal to the lifecycle (ADR 0055 §4): a scenario can be applied
/// *and* archived, and that combination is exactly when someone needs to see both facts —
/// the effect is still in their forecast while the scenario is filed away. So this returns
/// only the applied layer, to be composed with whatever the row's state already carries.
export function appliedRowProps(scenario: ScenarioDto): { className?: string } {
  if (scenario.applied_at === null) return {};
  return {
    // `border-l-2` rather than a full border: a spine marks the row's edge without boxing
    // it, so a run of applied rows still reads as one table.
    className: "border-l-2 border-l-primary bg-primary/10",
  };
}
