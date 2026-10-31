/* eslint-disable react-refresh/only-export-components -- an entry module: it renders itself and nothing imports it, so fast refresh has no component boundary to preserve. */
// Contrast audit (personal-cfo-4d8.35). DEV ONLY and FAKE DATA ONLY: this page is not part of the app
// bundle (nothing imports it; `index.html` is the only build entry). It renders the REAL components
// plus every `border-input` class string found in the source, derives their hover and focus states from
// those same class strings, and reports what was actually PAINTED: composited backgrounds, tinted
// fills, borders, rings and badge marks, with the engine's version and the pass/fail verdicts.
//
// Run:  pnpm -C apps/desktop exec vite --port 5199 --strictPort
// Open: http://127.0.0.1:5199/contrast-audit.html   (any browser; Safari is the closest to the app's
//       WKWebView). The summary, the table and `window.__contrastAudit` carry the results.
// See `src/dev/README.md`.
import { RotateCcw } from "lucide-react";
import React, { useEffect, useMemo, useState } from "react";
import ReactDOM from "react-dom/client";

import "@fontsource-variable/ibm-plex-sans/index.css";
import "@/styles/globals.css";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Combobox } from "@/components/ui/combobox";
import { EmojiPicker } from "@/components/ui/emoji-picker";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { PaginationControls } from "@/components/ui/pagination";
import { usePagination } from "@/lib/usePagination";
import { cn } from "@/lib/utils";
import { AppearanceCard } from "@/settings/AppearanceCard";
import { ThemeProvider } from "@/theme/ThemeProvider";
import { ThemeToggle } from "@/theme/ThemeToggle";

import { BADGE_CASES, badgeVerdict, type Verdict } from "./badge-verdict";
import { clears, HEADROOM, NON_TEXT_FLOOR, solidRings, TEXT_FLOOR, toHex } from "./contrast-math";
import { backgroundChain, captureMeta, composite, engine, postResultIfAsked, ratio } from "./measure";

type Theme = "light" | "dark";
type Surface = "page" | "card";

type Row = {
  theme: Theme;
  surface: Surface;
  section: string;
  id: string;
  state: string;
  verdict: Verdict;
  /// The floors this row was held to, headroom included (`label`, `mark`, `boundary`, `ring`). An `info`
  /// row carries the floors it would have been held to, and `required` says whether it was.
  thresholds: Record<string, number>;
  required: boolean;
  detail: string;
  raw: Record<string, string | number | boolean>;
};

const TEXT_NEED = TEXT_FLOOR + HEADROOM;
const NON_TEXT_NEED = NON_TEXT_FLOOR + HEADROOM;

// ── the consumers: every `border-input` class string in the source, discovered, not hand-listed ──
const SOURCES = import.meta.glob(["/src/**/*.tsx", "!/src/**/*.test.tsx", "!/src/dev/**"], {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

type Consumer = { classes: string; where: string[]; exemption: string | null };

/// A consumer whose boundary is NOT what identifies it, with the reason. Everything else is a field
/// (input, select, textarea, combobox trigger, swatch, a custom checkbox box) and must clear 3:1. Reported
/// either way. Order matters: the specific cases first, the generic "color-only" last.
function exemption(classes: string): string | null {
  const hasWidth = /(^|\s)border(\s|$)/.test(classes);
  if (/border-[lrtb]\b/.test(classes)) return "divider between labelled segments";
  if (/hover:bg-muted/.test(classes)) return "chip: identified by its label";
  if (/rounded-full/.test(classes) && /size-8/.test(classes))
    return "icon button: identified by its icon and accessible name";
  if (/hover:bg-accent/.test(classes) && !/(^|\s)h-\d/.test(classes))
    return "labelled button: identified by its text";
  if (/!done && !current|text-muted-foreground$/.test(classes) && !/(^|\s)h-\d/.test(classes))
    return "status indicator, not interactive";
  if (/inline-flex w-fit rounded-md/.test(classes)) return "group container of labelled segments";
  if (/(^|\s)size-4(\s|$)/.test(classes) && /rounded/.test(classes) && !hasWidth)
    return "native checkbox: the platform draws it; the class sets only a color";
  // A bare `border-input` (the unchecked box of a custom checkbox, `SeriesPicker`) is NOT exempt: its
  // border is the visible state of the control. Its width comes from a sibling class, so the tile adds one.
  return null;
}

function discover(): Consumer[] {
  const byClass = new Map<string, string[]>();
  for (const [path, text] of Object.entries(SOURCES)) {
    text.split("\n").forEach((line, index) => {
      if (!line.includes("border-input")) return;
      for (const m of line.matchAll(/"([^"\n]*\bborder-input\b[^"\n]*)"/g)) {
        const where = byClass.get(m[1]!) ?? [];
        where.push(`${path.replace("/src/", "")}:${index + 1}`);
        byClass.set(m[1]!, where);
      }
    });
  }
  return [...byClass.entries()]
    .map(([classes, where]) => ({ classes, where, exemption: exemption(classes) }))
    .sort((a, b) => a.where[0]!.localeCompare(b.where[0]!));
}

/// Put `el` into its `hover` or `focus` state WITHOUT a pointer or a key press, using the stylesheet
/// the browser actually loaded: for each class on the element, find the rules written for
/// `.<class>:hover` (or `:focus-visible` / `:focus`) and apply their declarations inline. Inline wins
/// over the base rule exactly as the pseudo-class would, and nothing depends on Tailwind having
/// generated an un-prefixed twin of a `hover:` utility (it has not: that class only exists with its
/// prefix). A real mouse hover and a real Tab were also compared against this for the shared controls.
function applyState(el: HTMLElement, state: "hover" | "focus"): number {
  // Read the FINAL state, not a frame of the transition: the controls use `transition-colors`, and in a
  // hidden or background tab transitions do not advance, so a mid-transition read would be the old color.
  el.style.transition = "none";
  const pseudo = state === "hover" ? [":hover"] : [":focus-visible", ":focus"];
  const classSelectors = new Set([...el.classList].map((token) => `.${CSS.escape(token)}`));
  let applied = 0;
  // `cssText`, not property-by-property: a shorthand holding `var()` (Tailwind's `border-color`)
  // reads back as EMPTY longhands, so copying them one by one silently writes nothing. Appending the
  // serialized declarations keeps the cascade order (a later `@supports` declaration wins, an
  // unsupported one is dropped by the engine, as it would be in the stylesheet).
  const copy = (style: CSSStyleDeclaration) => {
    if (style.cssText) el.style.cssText = `${el.style.cssText} ${style.cssText}`;
  };
  /// A matched rule's declarations and everything nested in it. Inside `@media (hover: hover)` within a
  /// nested rule the declarations are `CSSNestedDeclarations` objects (a `style` and no selector), so read
  /// `style` from ANY rule that has one, not only from style rules.
  const collect = (rule: CSSRule) => {
    const any = rule as CSSRule & { style?: CSSStyleDeclaration; cssRules?: CSSRuleList };
    if (any.style) copy(any.style);
    if (any.cssRules) for (const nested of any.cssRules) collect(nested);
  };
  // Tailwind v4 nests variants: `.hover\:x { &:hover { @media (hover: hover) { ... } } }`. Older output
  // is flat: `.hover\:x:hover { ... }`. Both are handled.
  const visit = (rules: CSSRuleList, inClass: boolean) => {
    for (const rule of rules) {
      if (rule instanceof CSSStyleRule) {
        const selectors = rule.selectorText.split(",").map((x) => x.trim());
        const flat = selectors.some((sel) =>
          [...classSelectors].some((cls) => pseudo.some((p) => sel === cls + p)),
        );
        const nested = inClass && selectors.some((sel) => pseudo.some((p) => sel === `&${p}`));
        if (flat || nested) {
          collect(rule);
          applied++;
        } else if (inClass) {
          visit(rule.cssRules, true);
        } else if (selectors.some((sel) => classSelectors.has(sel))) {
          visit(rule.cssRules, true);
        }
      } else if ("cssRules" in rule) {
        visit((rule as CSSGroupingRule).cssRules, inClass);
      }
    }
  };
  for (const sheet of document.styleSheets) {
    try {
      visit(sheet.cssRules, false);
    } catch {
      // a cross-origin sheet cannot be read; the app ships none.
    }
  }
  return applied;
}

// ── measurement ─────────────────────────────────────────────────────────────────────────────────
function measureField(el: HTMLElement) {
  const cs = getComputedStyle(el);
  const outside = composite(backgroundChain(el, false));
  const fill = composite(backgroundChain(el, true));
  const border = composite([...backgroundChain(el, true), cs.borderTopColor]);
  const rings = solidRings(cs.boxShadow);
  const ring = rings.at(-1);
  return {
    outside,
    fill,
    border,
    borderWidth: cs.borderTopWidth,
    onOutside: ratio(border, outside),
    onFill: ratio(border, fill),
    ring: ring ? ring.color : null,
    ringOnOutside: ring ? ratio(ring.color, outside) : null,
  };
}

function measureBadge(el: HTMLElement) {
  const cs = getComputedStyle(el);
  const before = getComputedStyle(el, "::before");
  const fill = composite(backgroundChain(el, true));
  const text = composite([...backgroundChain(el, true), cs.color]);
  const hasMark = before.content !== "none" && before.display !== "none";
  const icon = el.querySelector("svg");
  const mark = hasMark
    ? composite([...backgroundChain(el, true), before.backgroundColor])
    : icon
      ? composite([...backgroundChain(el, true), getComputedStyle(icon).color])
      : null;
  return {
    fill,
    text,
    label: ratio(text, fill),
    mark,
    markKind: hasMark ? "dot" : icon ? "icon" : "none",
    markOnFill: mark ? ratio(mark, fill) : null,
    fontPx: cs.fontSize,
    overflows: el.scrollWidth > el.clientWidth + 1,
    lines: Math.round(el.getBoundingClientRect().height / (parseFloat(cs.lineHeight) || 16)),
  };
}

// ── fixture ─────────────────────────────────────────────────────────────────────────────────────
const LONG = "Statement balance carried over from the previous billing period";

function Pager() {
  const pagination = usePagination(
    Array.from({ length: 80 }, (_, i) => i),
    "contrast-audit",
  );
  return <PaginationControls pagination={pagination} noun="rows" />;
}

/// One declared badge (`badge-verdict.ts`) rendered the way the app renders it.
function FixtureBadge({ spec }: { spec: (typeof BADGE_CASES)[number] }) {
  switch (spec.body) {
    case "autopay":
      return (
        <Badge variant={spec.variant} className="text-[10px] uppercase tracking-wide" data-case={spec.id}>
          Autopay
        </Badge>
      );
    case "resurfaced":
      return (
        <Badge variant={spec.variant} className="gap-1 before:hidden" data-case={spec.id}>
          <RotateCcw className="size-3 text-warning" aria-hidden /> Resurfaced
        </Badge>
      );
    case "long":
      return (
        <Badge variant={spec.variant} data-case={spec.id}>
          {LONG}
        </Badge>
      );
    default:
      return (
        <Badge variant={spec.variant} data-case={spec.id}>
          {spec.variant} label
        </Badge>
      );
  }
}

function Fixture({ theme, surface }: { theme: Theme; surface: Surface }) {
  const id = (name: string) => ({ "data-case": `${name}` });
  return (
    <div className="flex flex-col gap-4 p-4">
      <div className="flex flex-wrap gap-2" data-group="badge">
        {BADGE_CASES.filter((c) => !c.narrow).map((c) => (
          <FixtureBadge key={c.id} spec={c} />
        ))}
      </div>
      <div className="flex w-[220px] flex-col gap-2" data-group="badge-narrow">
        {BADGE_CASES.filter((c) => c.narrow).map((c) => (
          <FixtureBadge key={c.id} spec={c} />
        ))}
      </div>
      <div className="flex flex-wrap items-center gap-2" data-group="controls">
        <Input placeholder="Input" aria-label={`input ${theme} ${surface}`} {...id("input")} />
        <NativeSelect aria-label={`select ${theme} ${surface}`} {...id("native-select")}>
          <option>Select</option>
        </NativeSelect>
        <NativeSelect size="sm" aria-label={`small select ${theme} ${surface}`} {...id("native-select-sm")}>
          <option>Small</option>
        </NativeSelect>
        <div {...id("combobox")} className="w-40">
          <Combobox
            items={[{ id: "a", label: "Alpha" }]}
            value={null}
            onSelect={() => {}}
            placeholder="Combobox"
            aria-label={`combobox ${theme} ${surface}`}
          />
        </div>
        <div {...id("emoji-picker")}>
          <EmojiPicker value="" onSelect={() => {}} aria-label={`emoji ${theme} ${surface}`} />
        </div>
        <ThemeToggle />
      </div>
      <div className="w-[220px]" data-group="controls-narrow">
        <NativeSelect aria-label="long option" {...id("native-select-long")}>
          <option>{LONG}</option>
        </NativeSelect>
      </div>
      <div className="flex flex-wrap gap-2" data-group="buttons">
        <Button variant="outline" {...id("button-outline")}>
          outline
        </Button>
        <Button variant="secondary" {...id("button-secondary")}>
          secondary
        </Button>
        <Button variant="ghost" {...id("button-ghost")}>
          ghost
        </Button>
        <Button {...id("button-default")}>default</Button>
      </div>
      <div data-case="pagination">
        <Pager />
      </div>
      <div data-case="segmented-appearance">
        <AppearanceCard />
      </div>
    </div>
  );
}

function Block({ theme, surface }: { theme: Theme; surface: Surface }) {
  const label = `${theme} / ${surface === "page" ? "--background" : "--card"} (fake data)`;
  const inner = (
    <>
      <h2 className="px-4 pt-4 text-sm font-semibold">{label}</h2>
      <Fixture theme={theme} surface={surface} />
    </>
  );
  return (
    <div className={theme === "dark" ? "dark" : ""} data-theme={theme}>
      <div className="bg-background p-4 text-foreground" data-surface={`${theme}.${surface}`}>
        {surface === "page" ? inner : <Card>{inner}</Card>}
      </div>
    </div>
  );
}

function Tiles({ theme, surface, consumers }: { theme: Theme; surface: Surface; consumers: Consumer[] }) {
  return (
    <div className={theme === "dark" ? "dark" : ""} data-theme={theme}>
      <div className="bg-background p-4 text-foreground" data-surface={`${theme}.${surface}.tiles`}>
        {(() => {
          const body = (
            <div className="grid grid-cols-[repeat(auto-fill,minmax(150px,1fr))] gap-3 p-4">
              {consumers.map((c, i) =>
                (["default", "hover", "focus"] as const).map((state) => (
                  <div key={`${i}-${state}`} className="flex flex-col gap-1 text-[10px] text-muted-foreground">
                    <div
                      data-consumer={i}
                      data-state={state}
                      className={cn(
                        // A color-only class string takes its width from a sibling class in the app; the tile
                        // has no sibling, so give it the 1px the real element has.
                        !/(^|\s)border(\s|$|-[lrtb]\b)/.test(c.classes) && "border",
                        c.classes,
                        "min-h-8 min-w-24 text-foreground",
                      )}
                    >
                      sample
                    </div>
                    <span className="truncate">
                      {state} · {c.where[0]}
                    </span>
                  </div>
                )),
              )}
            </div>
          );
          return surface === "page" ? body : <Card><CardContent className="p-0">{body}</CardContent></Card>;
        })()}
      </div>
    </div>
  );
}

// ── run ─────────────────────────────────────────────────────────────────────────────────────────
function runAudit(consumers: Consumer[]): Row[] {
  const rows: Row[] = [];
  // Hover and focus tiles carry their ORIGINAL classes; the state comes from the stylesheet's own rules.
  for (const tile of document.querySelectorAll<HTMLElement>("[data-consumer][data-state]")) {
    const state = tile.dataset.state;
    if (state === "hover" || state === "focus") tile.dataset.rules = String(applyState(tile, state));
  }
  const push = (r: Omit<Row, "verdict"> & { verdict: Verdict }) => rows.push(r);
  for (const theme of ["light", "dark"] as const) {
    for (const surface of ["page", "card"] as const) {
      const root = document.querySelector<HTMLElement>(`[data-surface="${theme}.${surface}"]`);
      if (!root) continue;
      // Badges: the declared label floor at 4.5 and the mark at 3 (both plus headroom). Every rendered
      // badge must be declared in `BADGE_CASES` (an undeclared one FAILS), and every declared badge must
      // be rendered here (a missing one FAILS), so a row can neither read `info` by accident nor vanish.
      const declared = new Map(BADGE_CASES.map((c) => [c.id, c]));
      const seen = new Set<string>();
      for (const el of root.querySelectorAll<HTMLElement>("[data-case^='badge-']")) {
        const id = el.dataset.case!;
        seen.add(id);
        const spec = declared.get(id);
        const m = measureBadge(el);
        push({
          theme, surface, section: "badge", id, state: "default",
          verdict: badgeVerdict(spec, { label: m.label, markOnFill: m.markOnFill, overflows: m.overflows }),
          thresholds: { label: TEXT_NEED, mark: NON_TEXT_NEED },
          required: spec?.required ?? true,
          detail: `${spec ? `variant ${spec.variant}${spec.semantic ? ", semantic" : ""}` : "NOT DECLARED in BADGE_CASES"}; label ${m.label}:1 (need ${TEXT_NEED}), mark ${m.markKind} ${m.markOnFill ?? "-"}:1 (need ${NON_TEXT_NEED}), ${m.fontPx}, ${m.lines} line(s)${m.overflows ? ", OVERFLOWS" : ""}`,
          raw: { variant: spec?.variant ?? "undeclared", semantic: spec?.semantic ?? false, fill: toHex(m.fill), text: toHex(m.text), mark: m.mark ? toHex(m.mark) : "none", markKind: m.markKind, label: m.label, markOnFill: m.markOnFill ?? -1, lines: m.lines, overflows: m.overflows },
        });
      }
      for (const spec of BADGE_CASES) {
        if (seen.has(spec.id)) continue;
        push({
          theme, surface, section: "badge", id: spec.id, state: "default", verdict: "FAIL",
          thresholds: { label: TEXT_NEED, mark: NON_TEXT_NEED }, required: spec.required,
          detail: "declared in BADGE_CASES but not rendered", raw: { variant: spec.variant },
        });
      }
      // Shared controls: default state on the real element.
      for (const el of root.querySelectorAll<HTMLElement>("[data-case]:not([data-case^='badge-'])")) {
        const id = el.dataset.case!;
        const target =
          el.matches("input,select,button") ? el : el.querySelector<HTMLElement>("input,select,button");
        if (!target || id === "pagination" || id === "segmented-appearance") continue;
        const m = measureField(target);
        const required = !id.startsWith("button-") ;
        const ok = clears(Math.min(m.onOutside, m.onFill), NON_TEXT_FLOOR);
        push({
          theme, surface, section: "shared control", id, state: "default",
          verdict: required ? (ok ? "pass" : "FAIL") : "info",
          thresholds: { boundary: NON_TEXT_NEED }, required,
          detail: `border ${toHex(m.border)} on outside ${m.onOutside}:1, on fill ${m.onFill}:1 (need ${NON_TEXT_FLOOR + HEADROOM})${required ? "" : " [labelled button: reported, not required]"}`,
          raw: { outside: toHex(m.outside), fill: toHex(m.fill), border: toHex(m.border), onOutside: m.onOutside, onFill: m.onFill, className: target.className.slice(0, 160) },
        });
      }
      // Pagination select and the segmented Appearance control (real components).
      const pager = root.querySelector<HTMLElement>("[data-case='pagination'] select");
      if (pager) {
        const m = measureField(pager);
        push({ theme, surface, section: "shared control", id: "pagination-select", state: "default",
          verdict: clears(Math.min(m.onOutside, m.onFill), NON_TEXT_FLOOR) ? "pass" : "FAIL",
          thresholds: { boundary: NON_TEXT_NEED }, required: true,
          detail: `border ${toHex(m.border)} on outside ${m.onOutside}:1, on fill ${m.onFill}:1`,
          raw: { border: toHex(m.border), onOutside: m.onOutside, onFill: m.onFill } });
      }
      const seg = root.querySelector<HTMLElement>("[data-case='segmented-appearance'] [role='radiogroup']");
      if (seg) {
        const m = measureField(seg);
        push({ theme, surface, section: "shared control", id: "segmented-appearance", state: "default", verdict: "info",
          thresholds: { boundary: NON_TEXT_NEED }, required: false,
          detail: `group border ${m.onOutside}:1 on outside [segments are identified by their labels: reported, not required]`,
          raw: { onOutside: m.onOutside } });
      }
      // Derived states for every discovered consumer, and for each shared control's real class string.
      const tiles = document.querySelector<HTMLElement>(`[data-surface="${theme}.${surface}.tiles"]`);
      if (tiles) {
        for (const el of tiles.querySelectorAll<HTMLElement>("[data-consumer]")) {
          const c = consumers[Number(el.dataset.consumer)]!;
          const state = el.dataset.state!;
          const m = measureField(el);
          const declares =
            state === "hover"
              ? /(^|\s)hover:/.test(c.classes)
              : state === "focus" && /(^|\s)focus(-visible)?:/.test(c.classes);
          const ruleless = declares && Number(el.dataset.rules ?? 0) === 0;
          const required = c.exemption === null;
          const boundaryOk = clears(Math.min(m.onOutside, m.onFill), NON_TEXT_FLOOR);
          const ringOk = state !== "focus" || m.ringOnOutside === null || clears(m.ringOnOutside, NON_TEXT_FLOOR);
          push({
            theme, surface, section: "consumer", id: c.where[0]!, state,
            verdict: ruleless ? "FAIL" : required ? (boundaryOk && ringOk ? "pass" : "FAIL") : "info",
            thresholds: state === "focus" ? { boundary: NON_TEXT_NEED, ring: NON_TEXT_NEED } : { boundary: NON_TEXT_NEED },
            required,
            detail: `${ruleless ? "NO STATE RULE FOUND IN THE STYLESHEET (the audit could not apply this state); " : ""}border on outside ${m.onOutside}:1, on fill ${m.onFill}:1${state === "focus" ? `, ring ${m.ringOnOutside ?? "none"}:1` : ""}${required ? "" : ` [exempt: ${c.exemption}]`}`,
            raw: { stateRules: Number(el.dataset.rules ?? 0), border: toHex(m.border), onOutside: m.onOutside, onFill: m.onFill, ringOnOutside: m.ringOnOutside ?? -1, exempt: c.exemption ?? "", classes: c.classes.slice(0, 160), where: c.where.join(", ") },
          });
        }
      }
    }
  }
  return rows;
}

/// The shared controls' REAL class strings (read off the rendered elements) put through the same
/// hover/focus derivation, so a hover override in a shared component is measured, not assumed.
function sharedStateTiles(): Consumer[] {
  const seen = new Map<string, Consumer>();
  for (const el of document.querySelectorAll<HTMLElement>("[data-surface='light.page'] [data-case]")) {
    const target = el.matches("input,select,button") ? el : el.querySelector<HTMLElement>("input,select,button");
    if (!target || !target.className.includes("border-input")) continue;
    const classes = target.className;
    if (!seen.has(classes)) seen.set(classes, { classes, where: [`shared:${el.dataset.case}`], exemption: el.dataset.case!.startsWith("button-") ? "labelled button: identified by its text" : null });
  }
  return [...seen.values()];
}

function Audit() {
  const discovered = useMemo(discover, []);
  const [shared, setShared] = useState<Consumer[] | null>(null);
  const [rows, setRows] = useState<Row[] | null>(null);
  const all = useMemo(() => [...(shared ?? []), ...discovered], [shared, discovered]);

  useEffect(() => {
    // A timer, not requestAnimationFrame: rAF does not run in a hidden or background tab, and the audit must
    // complete in any browser state.
    void document.fonts.ready.then(() => setTimeout(() => setShared(sharedStateTiles()), 50));
  }, []);
  useEffect(() => {
    if (!shared) return;
    const t = setTimeout(() => {
      const result = runAudit(all);
      setRows(result);
      const envelope = { engine: engine(), meta: captureMeta(), rows: result };
      (window as unknown as { __contrastAudit: unknown }).__contrastAudit = envelope;
      postResultIfAsked(envelope);
    }, 250);
    return () => clearTimeout(t);
  }, [shared, all]);

  const counts = rows ? { pass: rows.filter((r) => r.verdict === "pass").length, fail: rows.filter((r) => r.verdict === "FAIL").length, info: rows.filter((r) => r.verdict === "info").length } : null;
  return (
    <div className="bg-background text-foreground">
      <div className="p-4 text-sm" data-testid="summary">
        <h1 className="text-lg font-semibold">Contrast audit (dev only, fake data)</h1>
        <p className="text-muted-foreground">{engine()}</p>
        <p className="font-medium" id="audit-summary">
          {counts ? `PASS ${counts.pass} · FAIL ${counts.fail} · info ${counts.info} (floors: text ${TEXT_FLOOR}, non-text ${NON_TEXT_FLOOR}, plus ${HEADROOM} headroom)` : "measuring…"}
        </p>
      </div>
      {(["light", "dark"] as const).flatMap((theme) =>
        (["page", "card"] as const).map((surface) => <Block key={`${theme}${surface}`} theme={theme} surface={surface} />),
      )}
      {shared && (["light", "dark"] as const).flatMap((theme) =>
        (["page", "card"] as const).map((surface) => (
          <Tiles key={`t${theme}${surface}`} theme={theme} surface={surface} consumers={all} />
        )),
      )}
      {rows && (
        <div className="p-4">
          <h2 className="text-sm font-semibold">Failures</h2>
          <ul className="text-xs" id="audit-failures">
            {rows.filter((r) => r.verdict === "FAIL").map((r, i) => (
              <li key={i}>{r.theme}/{r.surface} · {r.section} · {r.id} · {r.state}: {r.detail}</li>
            ))}
          </ul>
          <h2 className="mt-4 text-sm font-semibold">All rows (JSON also in window.__contrastAudit)</h2>
          <pre id="audit-json" className="max-h-96 overflow-auto text-[10px]">{JSON.stringify({ engine: engine(), meta: captureMeta(), rows }, null, 1)}</pre>
        </div>
      )}
    </div>
  );
}

// A fixed theme: the real ThemeToggle / AppearanceCard talk to the ThemeProvider, which would otherwise
// follow the OS and flip <html>. Each block carries its own theme class instead.
try {
  window.localStorage.setItem("pcfo.theme", "light");
} catch {
  // storage unavailable: the provider falls back to the system preference; the blocks still carry their own class.
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ThemeProvider>
      <Audit />
    </ThemeProvider>
  </React.StrictMode>,
);
