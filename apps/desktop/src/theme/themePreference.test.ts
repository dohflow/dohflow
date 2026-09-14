import {
  applyResolvedTheme,
  readStoredThemePreference,
  resolveTheme,
  systemPrefersDark,
  writeStoredThemePreference,
} from "./themePreference";

beforeEach(() => {
  window.localStorage.clear();
  document.documentElement.classList.remove("dark");
});

describe("resolveTheme", () => {
  it("an explicit light preference always resolves to light, regardless of the system", () => {
    expect(resolveTheme("light", true)).toBe("light");
    expect(resolveTheme("light", false)).toBe("light");
  });

  it("an explicit dark preference always resolves to dark, regardless of the system", () => {
    expect(resolveTheme("dark", true)).toBe("dark");
    expect(resolveTheme("dark", false)).toBe("dark");
  });

  it("system follows the live system query", () => {
    expect(resolveTheme("system", true)).toBe("dark");
    expect(resolveTheme("system", false)).toBe("light");
  });
});

describe("applyResolvedTheme", () => {
  it("toggles the .dark class on the document element", () => {
    applyResolvedTheme("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    applyResolvedTheme("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });
});

describe("readStoredThemePreference / writeStoredThemePreference", () => {
  it("defaults to system when nothing is stored", () => {
    expect(readStoredThemePreference()).toBe("system");
  });

  it("round-trips a written preference", () => {
    writeStoredThemePreference("dark");
    expect(readStoredThemePreference()).toBe("dark");
    writeStoredThemePreference("light");
    expect(readStoredThemePreference()).toBe("light");
  });

  it("falls back to system for a garbage stored value (a future release removing a choice)", () => {
    window.localStorage.setItem("pcfo.theme", "solarized");
    expect(readStoredThemePreference()).toBe("system");
  });

  it("never throws when storage is unavailable", () => {
    const original = window.localStorage;
    Object.defineProperty(window, "localStorage", {
      value: {
        getItem: () => {
          throw new Error("storage disabled");
        },
        setItem: () => {
          throw new Error("storage disabled");
        },
      },
      configurable: true,
    });
    try {
      expect(readStoredThemePreference()).toBe("system");
      expect(() => writeStoredThemePreference("dark")).not.toThrow();
    } finally {
      Object.defineProperty(window, "localStorage", {
        value: original,
        configurable: true,
      });
    }
  });
});

describe("systemPrefersDark", () => {
  it("never throws even if matchMedia is unavailable", () => {
    const original = window.matchMedia;
    // @ts-expect-error -- deliberately simulating an unusual WebView build
    delete window.matchMedia;
    try {
      expect(systemPrefersDark()).toBe(false);
    } finally {
      window.matchMedia = original;
    }
  });
});
