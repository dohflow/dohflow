import { act, renderHook } from "@testing-library/react";

import globalsCss from "virtual:globals-css-raw";

import { COARSE_POINTER_QUERY, COMPACT_QUERY, matches, useCompactLayout } from "./compact";

/// A `matchMedia` stub that answers per query and can be driven, like a real MediaQueryList.
function installMatchMedia(initial: Record<string, boolean>) {
  const state = { ...initial };
  const listeners = new Map<string, Set<() => void>>();
  vi.stubGlobal("matchMedia", ((query: string) => ({
    get matches() {
      return state[query] ?? false;
    },
    media: query,
    addEventListener: (_type: string, l: () => void) => {
      if (!listeners.has(query)) listeners.set(query, new Set());
      listeners.get(query)?.add(l);
    },
    removeEventListener: (_type: string, l: () => void) => listeners.get(query)?.delete(l),
  })) as unknown as typeof window.matchMedia);
  return {
    set(query: string, value: boolean) {
      state[query] = value;
      for (const l of listeners.get(query) ?? []) l();
    },
    listeners: (query: string) => listeners.get(query)?.size ?? 0,
  };
}

afterEach(() => vi.unstubAllGlobals());

describe("the compact layout signal (personal-cfo-i93n3.25)", () => {
  it("is declared once in TypeScript, and the Tailwind `compact:` variant uses the same query", () => {
    const declared = /@custom-variant compact \(@media (.+)\);/.exec(globalsCss);
    expect(declared, "globals.css declares the compact variant").not.toBeNull();
    expect(declared?.[1]).toBe(COMPACT_QUERY);
  });

  it("needs a touch pointer as well as a narrow screen, so a narrowed desktop window is not compact", () => {
    expect(COMPACT_QUERY).toContain("(pointer: coarse)");
    expect(COMPACT_QUERY).toContain("max-width");
    expect(COARSE_POINTER_QUERY).toBe("(pointer: coarse)");
  });

  it("is false where matchMedia does not exist", () => {
    vi.stubGlobal("matchMedia", undefined);
    expect(matches(COMPACT_QUERY)).toBe(false);
    expect(renderHook(() => useCompactLayout()).result.current).toBe(false);
  });

  it("follows the query as it changes, and unsubscribes on unmount", () => {
    const media = installMatchMedia({ [COMPACT_QUERY]: false });
    const { result, unmount } = renderHook(() => useCompactLayout());
    expect(result.current).toBe(false);

    act(() => media.set(COMPACT_QUERY, true));
    expect(result.current).toBe(true);

    act(() => media.set(COMPACT_QUERY, false));
    expect(result.current).toBe(false);

    unmount();
    expect(media.listeners(COMPACT_QUERY)).toBe(0);
  });

  it("reads only the compact query, not the pointer alone", () => {
    installMatchMedia({ [COARSE_POINTER_QUERY]: true, [COMPACT_QUERY]: false });
    expect(renderHook(() => useCompactLayout()).result.current).toBe(false);
  });
});
