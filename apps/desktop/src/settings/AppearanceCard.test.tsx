import { fireEvent, render, screen } from "@testing-library/react";

import { ThemeProvider } from "@/theme/ThemeProvider";
import { AppearanceCard } from "./AppearanceCard";

const mocks = vi.hoisted(() => ({ setTheme: vi.fn().mockResolvedValue(undefined) }));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ setTheme: mocks.setTheme }),
}));

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  document.documentElement.classList.remove("dark");
});

/// AppearanceCard reads useTheme() from context — every render needs a ThemeProvider
/// ancestor (personal-cfo-17u1 review, Finding 1: the provider is what now owns the
/// shared theme state so it can be mounted once at the app root; this card is just one
/// of its consumers, not its own independent instance).
function renderCard() {
  return render(
    <ThemeProvider>
      <AppearanceCard />
    </ThemeProvider>,
  );
}

test("System is selected by default", () => {
  renderCard();
  expect(screen.getByRole("radio", { name: "System" })).toHaveAttribute(
    "aria-checked",
    "true",
  );
  expect(screen.getByRole("radio", { name: "Light" })).toHaveAttribute(
    "aria-checked",
    "false",
  );
  expect(screen.getByRole("radio", { name: "Dark" })).toHaveAttribute(
    "aria-checked",
    "false",
  );
});

test("clicking Dark applies immediately", () => {
  renderCard();
  fireEvent.click(screen.getByRole("radio", { name: "Dark" }));
  expect(screen.getByRole("radio", { name: "Dark" })).toHaveAttribute(
    "aria-checked",
    "true",
  );
  expect(document.documentElement.classList.contains("dark")).toBe(true);
});

test("clicking Light then System cycles correctly and clears the dark class", () => {
  renderCard();
  fireEvent.click(screen.getByRole("radio", { name: "Dark" }));
  expect(document.documentElement.classList.contains("dark")).toBe(true);

  fireEvent.click(screen.getByRole("radio", { name: "Light" }));
  expect(screen.getByRole("radio", { name: "Light" })).toHaveAttribute(
    "aria-checked",
    "true",
  );
  expect(document.documentElement.classList.contains("dark")).toBe(false);

  fireEvent.click(screen.getByRole("radio", { name: "System" }));
  expect(screen.getByRole("radio", { name: "System" })).toHaveAttribute(
    "aria-checked",
    "true",
  );
});

test("the choice persists across a remount (localStorage)", () => {
  const { unmount } = renderCard();
  fireEvent.click(screen.getByRole("radio", { name: "Dark" }));
  unmount();

  renderCard();
  expect(screen.getByRole("radio", { name: "Dark" })).toHaveAttribute(
    "aria-checked",
    "true",
  );
});

test("exposes an accessible radiogroup labeled Appearance", () => {
  renderCard();
  expect(
    screen.getByRole("radiogroup", { name: "Appearance" }),
  ).toBeInTheDocument();
});
