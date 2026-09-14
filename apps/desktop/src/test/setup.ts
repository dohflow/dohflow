import { configure } from "@testing-library/react";
import "@testing-library/jest-dom";

// Root cause of personal-cfo-bv6w2 (money-inbox tests flaky under full-suite
// runs, stable in isolation): NOT shared state or a missing await — every
// candidate was checked and ruled out (renderWithClient gives each test a
// fresh QueryClient with retry:false, so no cross-test cache/backoff delay;
// RTL's auto-cleanup is active via the `globals: true` afterEach hook, so no
// leftover DOM between tests within a file; default per-file module
// isolation means no cross-FILE state either). It is a TIMER: RTL's default
// `waitFor`/`findBy*` timeout is 1000ms, and the observed failure
// (CardReviewModal.test.tsx, "keeps persisted edits when a deferred card
// resurfaces") took exactly 1070ms — a hair over that budget, not "broken."
// `pnpm test` alone already runs at 500%+ CPU (measured: 541%) from Vitest's
// own worker parallelism; a concurrent build, or another session's tests,
// tips already-marginal effect-flush timing over the 1000ms line — exactly
// the condition both flakes were reported under (PR 440 and PR 441 reviews).
// Raising the shared budget fixes every assertion of this shape at once,
// including whichever specific MoneyInboxView spy assertion flaked (never
// pinned to an exact line in the bead report) — a global, environment-
// appropriate timeout, not a retry: the same single check still runs once
// and must still pass.
configure({ asyncUtilTimeout: 3000 });

// jsdom lacks ResizeObserver, which Recharts' ResponsiveContainer (used by the shadcn
// chart primitive) constructs on mount. A no-op stub lets chart-bearing views render in
// tests; the charts measure 0×0 and draw nothing, but the surrounding figure/legend
// (plain HTML) still assert cleanly.
class ResizeObserverStub {
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}

globalThis.ResizeObserver ??=
  ResizeObserverStub as unknown as typeof ResizeObserver;

// jsdom does not implement matchMedia at all (personal-cfo-17u1's useTheme calls it to
// follow the system appearance). A benign default — never matches, listener
// registration is a no-op — lets any component that queries it render without throwing.
// A test that needs to simulate the system appearance actually changing installs its own
// controllable stub via `vi.stubGlobal("matchMedia", ...)` instead of relying on this one.
if (typeof window.matchMedia !== "function") {
  window.matchMedia = ((query: string) =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }) as unknown as MediaQueryList) as typeof window.matchMedia;
}

// jsdom here doesn't expose a functional localStorage; a tiny in-memory Storage lets
// sticky-preference code (e.g. usePagination's page size) read/write in tests.
function createMemoryStorage(): Storage {
  const store = new Map<string, string>();
  return {
    get length(): number {
      return store.size;
    },
    clear: (): void => store.clear(),
    getItem: (key: string): string | null => store.get(key) ?? null,
    key: (index: number): string | null => [...store.keys()][index] ?? null,
    removeItem: (key: string): void => {
      store.delete(key);
    },
    setItem: (key: string, value: string): void => {
      store.set(key, String(value));
    },
  } as Storage;
}

Object.defineProperty(window, "localStorage", {
  value: createMemoryStorage(),
  configurable: true,
});
