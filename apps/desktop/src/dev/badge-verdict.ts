/// The badge section of the contrast audit (`personal-cfo-4d8.35`), as data and a pure verdict, so
/// it is unit tested without a browser. Every badge the page renders is DECLARED here, with its
/// variant, whether its mark is the status cue (`semantic`) and whether it must pass (`required`).
/// The verdict reads that declaration; it never derives a badge's class from its id. The first
/// version parsed `badge-long-narrow` as the variant "long", which is not semantic, and so reported
/// the two long badges as `info` whatever they measured (Studio finding F1 on 0565232 and 598873d).
import { clears, NON_TEXT_FLOOR, TEXT_FLOOR } from "./contrast-math";

export type Verdict = "pass" | "FAIL" | "info";

export type BadgeVariant = "default" | "secondary" | "outline" | "gain" | "warning" | "info" | "loss";

export type BadgeSpec = {
  /// The `data-case` of the rendered element.
  id: string;
  variant: BadgeVariant;
  /// The mark (dot or icon) carries the status, so it must clear the non-text floor too.
  semantic: boolean;
  /// Held to the floors. A badge that is deliberately only reported says so, with a reason.
  required: boolean;
  exemption?: string;
  /// What the fixture renders inside it.
  body: "label" | "autopay" | "resurfaced" | "long";
  /// Rendered in the 220px column that makes the long labels wrap.
  narrow?: boolean;
};

const VARIANTS: readonly BadgeVariant[] = ["default", "secondary", "outline", "gain", "warning", "info", "loss"];
const SEMANTIC_VARIANTS = new Set<BadgeVariant>(["gain", "warning", "info", "loss"]);

export const BADGE_CASES: readonly BadgeSpec[] = [
  ...VARIANTS.map<BadgeSpec>((variant) => ({
    id: `badge-${variant}`,
    variant,
    semantic: SEMANTIC_VARIANTS.has(variant),
    required: true,
    body: "label",
  })),
  { id: "badge-autopay", variant: "gain", semantic: true, required: true, body: "autopay" },
  { id: "badge-resurfaced", variant: "warning", semantic: true, required: true, body: "resurfaced" },
  { id: "badge-long-narrow", variant: "warning", semantic: true, required: true, body: "long", narrow: true },
  { id: "badge-long-narrow-gain", variant: "gain", semantic: true, required: true, body: "long", narrow: true },
];

export type BadgeMeasure = {
  /// Contrast of the label on the badge's composited fill.
  label: number;
  /// Contrast of the mark on that fill, or `null` when the badge has no mark.
  markOnFill: number | null;
  /// The label does not fit its box.
  overflows: boolean;
};

/// The verdict for one rendered badge. A badge with no declaration FAILS: an undeclared case must not
/// be able to read as `info` and hide.
export function badgeVerdict(spec: BadgeSpec | undefined, m: BadgeMeasure): Verdict {
  if (!spec) return "FAIL";
  if (!spec.required) return "info";
  const labelOk = clears(m.label, TEXT_FLOOR);
  const markOk = m.markOnFill === null ? !spec.semantic : clears(m.markOnFill, NON_TEXT_FLOOR);
  return labelOk && markOk && !m.overflows ? "pass" : "FAIL";
}
