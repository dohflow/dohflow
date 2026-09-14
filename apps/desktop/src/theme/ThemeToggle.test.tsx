import { fireEvent, render, screen } from "@testing-library/react";

import { ThemeProvider } from "./ThemeProvider";
import { ThemeToggle } from "./ThemeToggle";

const mocks = vi.hoisted(() => ({ setTheme: vi.fn().mockResolvedValue(undefined) }));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ setTheme: mocks.setTheme }),
}));

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  document.documentElement.classList.remove("dark");
});

function renderToggle(props: { floating?: boolean } = {}) {
  return render(
    <ThemeProvider>
      <ThemeToggle {...props} />
    </ThemeProvider>,
  );
}

test("shows the System label by default", () => {
  renderToggle();
  expect(screen.getByRole("button", { name: /Appearance: System/ })).toBeInTheDocument();
});

test("one click cycles System -> Light -> Dark -> System", () => {
  renderToggle();
  const button = () => screen.getByRole("button", { name: /^Appearance:/ });

  expect(button()).toHaveAccessibleName(/Appearance: System/);

  fireEvent.click(button());
  expect(button()).toHaveAccessibleName(/Appearance: Light/);
  expect(document.documentElement.classList.contains("dark")).toBe(false);

  fireEvent.click(button());
  expect(button()).toHaveAccessibleName(/Appearance: Dark/);
  expect(document.documentElement.classList.contains("dark")).toBe(true);

  fireEvent.click(button());
  expect(button()).toHaveAccessibleName(/Appearance: System/);
});

test("reflects an already-set preference from a shared provider instance", () => {
  render(
    <ThemeProvider>
      <ThemeToggle />
    </ThemeProvider>,
  );
  fireEvent.click(screen.getByRole("button", { name: /^Appearance:/ })); // -> Light
  expect(
    screen.getByRole("button", { name: /Appearance: Light/ }),
  ).toBeInTheDocument();
});

test("floating renders with fixed positioning classes", () => {
  renderToggle({ floating: true });
  expect(screen.getByRole("button", { name: /^Appearance:/ }).className).toMatch(
    /fixed/,
  );
});

test("not floating omits the fixed positioning classes (for embedding in normal flow)", () => {
  renderToggle({ floating: false });
  expect(screen.getByRole("button", { name: /^Appearance:/ }).className).not.toMatch(
    /fixed/,
  );
});
