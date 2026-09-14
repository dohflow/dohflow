import { act, render, renderHook, screen } from "@testing-library/react";
import { waitFor } from "@testing-library/react";

import { ThemeProvider, useTheme } from "./ThemeProvider";

const mocks = vi.hoisted(() => ({ setTheme: vi.fn().mockResolvedValue(undefined) }));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ setTheme: mocks.setTheme }),
}));

/// A controllable `matchMedia` stub for "(prefers-color-scheme: dark)" — see
/// themePreference.test.ts's sibling for why a controllable stub is needed beyond the
/// benign default in test/setup.ts. `fireChange` drives every listener this stub has
/// ever handed out, mirroring how a real MediaQueryList notifies every subscriber.
function installControllableMatchMedia(initialMatches: boolean) {
  let matches = initialMatches;
  const listeners = new Set<() => void>();
  const stub = ((query: string) => ({
    get matches() {
      return matches;
    },
    media: query,
    onchange: null,
    addListener: (l: () => void) => listeners.add(l),
    removeListener: (l: () => void) => listeners.delete(l),
    addEventListener: (_type: string, l: () => void) => listeners.add(l),
    removeEventListener: (_type: string, l: () => void) => listeners.delete(l),
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
  vi.stubGlobal("matchMedia", stub);
  return {
    fireChange: (next: boolean) => {
      matches = next;
      for (const l of listeners) l();
    },
    listenerCount: () => listeners.size,
  };
}

const wrapper = ({ children }: { children: React.ReactNode }) => (
  <ThemeProvider>{children}</ThemeProvider>
);

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  document.documentElement.classList.remove("dark");
});

afterEach(() => {
  vi.unstubAllGlobals();
});

test("useTheme() throws when called outside a ThemeProvider", () => {
  // Guards the review's own concern about a second, independent resolver existing
  // anywhere — a caller that forgets the provider fails loudly instead of silently
  // getting its own disconnected state.
  const { result } = renderHook(() => {
    try {
      return useTheme();
    } catch (e) {
      return e;
    }
  });
  expect(result.current).toBeInstanceOf(Error);
  expect((result.current as Error).message).toContain("ThemeProvider");
});

test("defaults to system, resolving against the current system query", () => {
  installControllableMatchMedia(true);
  const { result } = renderHook(() => useTheme(), { wrapper });
  expect(result.current.preference).toBe("system");
  expect(result.current.resolved).toBe("dark");
});

test("an explicit override always wins over the system query", () => {
  installControllableMatchMedia(true); // system prefers dark
  const { result } = renderHook(() => useTheme(), { wrapper });
  act(() => result.current.setPreference("light"));
  expect(result.current.preference).toBe("light");
  expect(result.current.resolved).toBe("light");
  expect(document.documentElement.classList.contains("dark")).toBe(false);
});

test("setPreference applies the .dark class, persists, and syncs the native title bar", async () => {
  installControllableMatchMedia(false);
  const { result } = renderHook(() => useTheme(), { wrapper });
  act(() => result.current.setPreference("dark"));
  expect(result.current.resolved).toBe("dark");
  expect(document.documentElement.classList.contains("dark")).toBe(true);
  expect(window.localStorage.getItem("pcfo.theme")).toBe("dark");
  await waitFor(() => expect(mocks.setTheme).toHaveBeenCalledWith("dark"));
});

// ---------------------------------------------------------------------------
// The review's required verification items (Finding 1), addressed directly:
//   (a) the always-mounted provider follows a live system change with NO other
//       component (no AppearanceCard, no Settings) mounted at all.
//   (b) an explicit override is never reverted by a later system change.
// ---------------------------------------------------------------------------

test("REQUIRED (a): a live system appearance change is followed with only the provider mounted — no Settings, no AppearanceCard", () => {
  const media = installControllableMatchMedia(false);
  // A bare consumer, standing in for "any screen at all" — the whole point is that
  // this has nothing to do with Settings being open.
  function Probe() {
    const { resolved } = useTheme();
    return <div data-testid="resolved">{resolved}</div>;
  }
  render(
    <ThemeProvider>
      <Probe />
    </ThemeProvider>,
  );
  expect(screen.getByTestId("resolved")).toHaveTextContent("light");
  act(() => media.fireChange(true));
  expect(screen.getByTestId("resolved")).toHaveTextContent("dark");
  expect(document.documentElement.classList.contains("dark")).toBe(true);
});

test("REQUIRED (b): an explicit override is never reverted by a later system appearance change", () => {
  const media = installControllableMatchMedia(false);
  const { result } = renderHook(() => useTheme(), { wrapper });
  act(() => result.current.setPreference("light"));
  expect(result.current.resolved).toBe("light");
  // The system now disagrees with the override in BOTH directions — first flip to
  // "prefers dark" (same direction the override already opposes), then flip AGAIN
  // to "prefers light" (the direction that would vacuously "match" the override by
  // accident) — neither should ever touch `preference` or `resolved`.
  act(() => media.fireChange(true));
  expect(result.current.preference).toBe("light");
  expect(result.current.resolved).toBe("light");
  act(() => media.fireChange(false));
  expect(result.current.preference).toBe("light");
  expect(result.current.resolved).toBe("light");
  expect(document.documentElement.classList.contains("dark")).toBe(false);
});

// ---------------------------------------------------------------------------

test("an explicit override stops listening for system appearance changes", () => {
  const media = installControllableMatchMedia(false);
  const { result } = renderHook(() => useTheme(), { wrapper });
  act(() => result.current.setPreference("light"));
  // The "system" listener from mount must have been torn down — pinning the exact
  // count (not just ">= 0") so a leak (subscribing again without unsubscribing the
  // old one — precisely how a stale listener could later revert an override) fails
  // here rather than only under repeated remounts.
  expect(media.listenerCount()).toBe(0);
});

test("switching back to system re-subscribes and reads a fresh value", () => {
  const media = installControllableMatchMedia(false);
  const { result } = renderHook(() => useTheme(), { wrapper });
  act(() => result.current.setPreference("light"));
  expect(media.listenerCount()).toBe(0);
  // The system preference changed while an explicit override was active.
  act(() => result.current.setPreference("system"));
  expect(media.listenerCount()).toBe(1);
  expect(result.current.resolved).toBe("light"); // matches the (unchanged) stub value
  act(() => media.fireChange(true));
  expect(result.current.resolved).toBe("dark");
});

test("a rejecting native setTheme never surfaces as an error (best-effort)", () => {
  mocks.setTheme.mockRejectedValueOnce(new Error("not supported on this platform"));
  installControllableMatchMedia(false);
  const { result } = renderHook(() => useTheme(), { wrapper });
  expect(() => act(() => result.current.setPreference("dark"))).not.toThrow();
  // The web-rendered theme still applied even though the native sync failed.
  expect(result.current.resolved).toBe("dark");
  expect(document.documentElement.classList.contains("dark")).toBe(true);
});

test("two independent components under the SAME provider see the same state — proving there is one shared source, not two", () => {
  // The review's core worry: two independently-created resolvers could disagree. This
  // proves the fix — both consumers read the ONE provider instance.
  installControllableMatchMedia(false);
  function ConsumerA() {
    const { resolved, setPreference } = useTheme();
    return (
      <div>
        <div data-testid="a-resolved">{resolved}</div>
        <button onClick={() => setPreference("dark")}>set-dark-from-a</button>
      </div>
    );
  }
  function ConsumerB() {
    const { resolved } = useTheme();
    return <div data-testid="b-resolved">{resolved}</div>;
  }
  render(
    <ThemeProvider>
      <ConsumerA />
      <ConsumerB />
    </ThemeProvider>,
  );
  expect(screen.getByTestId("a-resolved")).toHaveTextContent("light");
  expect(screen.getByTestId("b-resolved")).toHaveTextContent("light");
  act(() => screen.getByRole("button", { name: "set-dark-from-a" }).click());
  expect(screen.getByTestId("a-resolved")).toHaveTextContent("dark");
  expect(screen.getByTestId("b-resolved")).toHaveTextContent("dark");
});
