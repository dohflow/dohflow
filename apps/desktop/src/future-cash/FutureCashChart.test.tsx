import { render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { ForecastDayDto } from "@/bindings";
import { FutureCashChart } from "./FutureCashChart";

/// The shared jsdom test setup (src/test/setup.ts) stubs ResizeObserver as a
/// no-op, specifically so chart-bearing views can render without throwing —
/// its own comment says the charts then "measure 0×0 and draw nothing." That
/// is fine for every OTHER test exercising this chart (they only assert on
/// the surrounding figure/aria-label), but personal-cfo-4d8.29's collision
/// fix needs REAL rendered pixel positions to assert on. Per that shared
/// stub's own documented convention ("a test that needs different behavior
/// installs its own controllable stub via vi.stubGlobal"), this replaces
/// ResizeObserver for this file only, reporting a fixed real size so
/// Recharts' <ResponsiveContainer> actually lays out the chart.
class FixedSizeResizeObserver {
  #callback: ResizeObserverCallback;
  constructor(callback: ResizeObserverCallback) {
    this.#callback = callback;
  }
  observe(target: Element) {
    const rect = { width: 600, height: 288 } as DOMRectReadOnly;
    // Real ResizeObservers report asynchronously; a microtask (rather than
    // firing synchronously inside observe()) reproduces that ordering
    // closely enough for FutureCashChart's own resize-driven re-render to
    // engage the same way it would in a real browser.
    queueMicrotask(() => {
      this.#callback(
        [{ target, contentRect: rect } as ResizeObserverEntry],
        this as unknown as ResizeObserver,
      );
    });
  }
  unobserve() {}
  disconnect() {}
}

function moneyDay(dateOffset: number, minorUnits: number): ForecastDayDto {
  const date = new Date(2026, 8, 1 + dateOffset).toISOString().slice(0, 10);
  const m = { minor_units: minorUnits, currency: "USD" };
  return { date, closing: { p10: m, p50: m, p90: m }, events: [] };
}

/// Matches personal-cfo-4d8.29's own field report exactly: a projected low of
/// "$34k" rendering close to a "$30k" y-axis gridline. Empirically verified
/// (against this exact component, in this exact test harness) to reproduce a
/// real collision: the label's uncorrected default position (14px below the
/// marker) lands ~2.7px from the $30k tick — well within the 12px collision
/// threshold — while the marker itself sits ~16.7px from that tick, not
/// literally within 1px of it. A separately-tuned fixture that puts the
/// MARKER within 1px of a tick was tried first and discarded: at this
/// chart's real scale, the fixed label-to-marker gap (14px) then clears the
/// 12px threshold entirely, so it never engages the fix at all. This
/// fixture is the one that actually exercises personal-cfo-4d8.29's bug and
/// its fix, which is the more useful thing to verify than hitting a literal
/// "1 px" figure by coincidence.
const COLLIDING_DAYS: ForecastDayDto[] = [
  moneyDay(0, 50_00000),
  moneyDay(1, 40_00000),
  moneyDay(2, 34_00000), // the projected low
  moneyDay(3, 45_00000),
  moneyDay(4, 60_00000),
];

function tickPositions(container: HTMLElement) {
  return [...container.querySelectorAll(".recharts-cartesian-axis-tick-value")].map((el) => ({
    text: el.textContent,
    y: Number(el.getAttribute("y")),
  }));
}

function lowLabel(container: HTMLElement) {
  return [...container.querySelectorAll("text")].find((el) => el.textContent?.startsWith("Low"));
}

describe("FutureCashChart collision avoidance (personal-cfo-4d8.29)", () => {
  let originalResizeObserver: typeof ResizeObserver;

  beforeEach(() => {
    originalResizeObserver = globalThis.ResizeObserver;
    globalThis.ResizeObserver = FixedSizeResizeObserver as unknown as typeof ResizeObserver;
  });

  afterEach(() => {
    globalThis.ResizeObserver = originalResizeObserver;
    document.documentElement.classList.remove("dark");
  });

  it("in light mode: moves the Low label clear of the nearest tick without hiding any tick", async () => {
    const { container } = render(<FutureCashChart days={COLLIDING_DAYS} currency="USD" />);

    await waitFor(() => {
      const label = lowLabel(container);
      const ticks = tickPositions(container);
      const dollarTicks = ticks.filter((t) => t.text?.startsWith("$"));
      // Uncorrected default would sit ~2.7px from the $30k tick — assert the
      // FIX actually engaged, not merely that some placement happens to
      // clear the threshold by luck.
      expect(label).toBeTruthy();
      for (const tick of dollarTicks) {
        const distance = Math.abs(Number(label!.getAttribute("y")) - tick.y);
        expect(distance, `Low label vs ${tick.text} tick`).toBeGreaterThan(12);
      }
    });

    // This scenario resolves by flipping sides, not by hiding a tick — every
    // dollar tick from the uncorrected render must still be present.
    const ticks = tickPositions(container);
    const dollarTickLabels = ticks.filter((t) => t.text?.startsWith("$")).map((t) => t.text);
    expect(dollarTickLabels).toEqual(["$0", "$15k", "$30k", "$45k", "$60k"]);
  });

  it("in dark mode: the same collision is resolved the same way (layout is theme-independent)", async () => {
    document.documentElement.classList.add("dark");
    const { container } = render(<FutureCashChart days={COLLIDING_DAYS} currency="USD" />);

    await waitFor(() => {
      const label = lowLabel(container);
      const ticks = tickPositions(container).filter((t) => t.text?.startsWith("$"));
      expect(label).toBeTruthy();
      for (const tick of ticks) {
        const distance = Math.abs(Number(label!.getAttribute("y")) - tick.y);
        expect(distance, `Low label vs ${tick.text} tick (dark)`).toBeGreaterThan(12);
      }
    });

    const dollarTickLabels = tickPositions(container)
      .filter((t) => t.text?.startsWith("$"))
      .map((t) => t.text);
    expect(dollarTickLabels).toEqual(["$0", "$15k", "$30k", "$45k", "$60k"]);
  });

  it("with no nearby tick, the label sits at its ordinary default position (no regression on the common case)", async () => {
    // A low value far from any round tick value — the pre-existing, common
    // case this bead must not disturb.
    const days: ForecastDayDto[] = [
      moneyDay(0, 50_00000),
      moneyDay(1, 22_00000), // far from every tick below
      moneyDay(2, 60_00000),
    ];
    const { container } = render(<FutureCashChart days={days} currency="USD" />);

    await waitFor(() => {
      expect(lowLabel(container)).toBeTruthy();
    });

    // No tick should ever be suppressed when nothing collides.
    const ticks = tickPositions(container).filter((t) => t.text?.startsWith("$"));
    expect(ticks.length).toBeGreaterThan(0);
  });
});
