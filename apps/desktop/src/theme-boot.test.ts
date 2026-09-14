/// theme-boot.ts runs its effect as a side effect of being imported (it has to — it must
/// apply the theme before React ever mounts, see its own doc comment), so each case here
/// resets the module registry and re-imports fresh rather than calling an exported
/// function. The underlying resolution logic (an explicit preference always winning, the
/// system fallback, storage/matchMedia failure handling) is already covered by
/// theme/themePreference.test.ts — this file only proves theme-boot.ts actually wires
/// that logic together and applies it to the real document.

beforeEach(() => {
  vi.resetModules();
  window.localStorage.clear();
  document.documentElement.classList.remove("dark");
});

afterEach(() => {
  vi.unstubAllGlobals();
});

test("applies dark when the stored preference is dark, before anything else runs", async () => {
  window.localStorage.setItem("pcfo.theme", "dark");
  await import("./theme-boot");
  expect(document.documentElement.classList.contains("dark")).toBe(true);
});

test("applies light when the stored preference is light", async () => {
  window.localStorage.setItem("pcfo.theme", "light");
  await import("./theme-boot");
  expect(document.documentElement.classList.contains("dark")).toBe(false);
});

test("with no stored preference, follows the system query", async () => {
  vi.stubGlobal(
    "matchMedia",
    ((query: string) => ({
      matches: true,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    })) as unknown as typeof window.matchMedia,
  );
  await import("./theme-boot");
  expect(document.documentElement.classList.contains("dark")).toBe(true);
});
