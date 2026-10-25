import { act, fireEvent, render, screen, within } from "@testing-library/react";

import { NoticeProvider, NotificationsRegion } from "@/components/ui/notice";
import {
  NOTICE_DURATION_MS,
  UNDO_FAILED_MESSAGE,
  useNotices,
  type NoticeInput,
} from "@/components/ui/notice-context";

// docs/agent/FRONTEND.md section 6.2 (personal-cfo-egn67): the shell's one notifications region and
// the rules every Undo notice follows.

let notify: (input: NoticeInput) => void = () => {};
function Harness() {
  notify = useNotices().notify;
  return <button>page content</button>;
}
function renderShell() {
  return render(
    <NoticeProvider>
      <NotificationsRegion />
      <Harness />
    </NoticeProvider>,
  );
}
const undoOk = () => vi.fn(async () => true);
const region = () => screen.queryByRole("region", { name: "Notifications" });

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

test("there is no landmark until there is a notice", () => {
  renderShell();
  expect(region()).toBeNull();
});

test("a notice is a polite status inside the Notifications region, with an Undo button", () => {
  renderShell();
  act(() => notify({ message: "Removed Rent.", undo: undoOk() }));
  const inRegion = within(screen.getByRole("region", { name: "Notifications" }));
  const status = inRegion.getByRole("status");
  expect(status).toHaveTextContent("Removed Rent.");
  expect(within(status).getByRole("button", { name: "Undo" })).toBeInTheDocument();
});

test("a notice appearing never moves focus", () => {
  renderShell();
  const page = screen.getByRole("button", { name: "page content" });
  page.focus();
  act(() => notify({ message: "Removed Rent.", undo: undoOk() }));
  expect(page).toHaveFocus();
});

test("the region comes before the page content in the tab order", () => {
  renderShell();
  act(() => notify({ message: "Removed Rent.", undo: undoOk() }));
  const undo = screen.getByRole("button", { name: "Undo" });
  const page = screen.getByRole("button", { name: "page content" });
  expect(undo.compareDocumentPosition(page) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
});

test("it closes itself after ten seconds", () => {
  renderShell();
  act(() => notify({ message: "Removed Rent.", undo: undoOk() }));
  act(() => vi.advanceTimersByTime(NOTICE_DURATION_MS - 1));
  expect(region()).not.toBeNull();
  act(() => vi.advanceTimersByTime(1));
  expect(region()).toBeNull();
});

test("hovering pauses the timer and keeps the time that was left", () => {
  renderShell();
  act(() => notify({ message: "Removed Rent.", undo: undoOk() }));
  const status = screen.getByRole("status");
  act(() => vi.advanceTimersByTime(6_000));
  fireEvent.mouseEnter(status);
  act(() => vi.advanceTimersByTime(60_000));
  expect(region()).not.toBeNull();
  fireEvent.mouseLeave(status);
  act(() => vi.advanceTimersByTime(3_999));
  expect(region()).not.toBeNull();
  act(() => vi.advanceTimersByTime(1));
  expect(region()).toBeNull();
});

test("focus inside pauses the timer; leaving resumes it", () => {
  renderShell();
  act(() => notify({ message: "Removed Rent.", undo: undoOk() }));
  const undo = screen.getByRole("button", { name: "Undo" });
  act(() => undo.focus());
  act(() => vi.advanceTimersByTime(60_000));
  expect(region()).not.toBeNull();
  act(() => undo.blur());
  act(() => vi.advanceTimersByTime(NOTICE_DURATION_MS));
  expect(region()).toBeNull();
});

test("the close button dismisses it", () => {
  renderShell();
  act(() => notify({ message: "Removed Rent.", undo: undoOk() }));
  fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
  expect(region()).toBeNull();
});

test("Escape closes the notice while focus is inside it", () => {
  renderShell();
  act(() => notify({ message: "Removed Rent.", undo: undoOk() }));
  const undo = screen.getByRole("button", { name: "Undo" });
  act(() => undo.focus());
  fireEvent.keyDown(undo, { key: "Escape" });
  expect(region()).toBeNull();
});

test("Escape elsewhere is never taken", () => {
  renderShell();
  act(() => notify({ message: "Removed Rent.", undo: undoOk() }));
  fireEvent.keyDown(screen.getByRole("button", { name: "page content" }), { key: "Escape" });
  expect(region()).not.toBeNull();
});

test("Escape inside a notice does not reach a window listener behind it", () => {
  const behind = vi.fn();
  window.addEventListener("keydown", behind);
  renderShell();
  act(() => notify({ message: "Removed Rent.", undo: undoOk() }));
  const undo = screen.getByRole("button", { name: "Undo" });
  act(() => undo.focus());
  fireEvent.keyDown(undo, { key: "Escape" });
  expect(behind).not.toHaveBeenCalled();
  window.removeEventListener("keydown", behind);
});

test("a successful Undo calls undo once and closes the notice", async () => {
  renderShell();
  const undo = undoOk();
  act(() => notify({ message: "Removed Rent.", undo }));
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
  });
  expect(undo).toHaveBeenCalledTimes(1);
  expect(region()).toBeNull();
});

test("a failed Undo keeps the notice and its button, and says so in its own alert", async () => {
  renderShell();
  const undo = vi.fn(async () => false);
  act(() => notify({ message: "Removed Rent.", undo }));
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
  });
  const status = screen.getByRole("status");
  // Its own alert element, inside the notice; the notice's own role is unchanged.
  expect(within(status).getByRole("alert")).toHaveTextContent(UNDO_FAILED_MESSAGE);
  expect(within(status).getByRole("button", { name: "Undo" })).toBeEnabled();
  expect(UNDO_FAILED_MESSAGE).toBe("Couldn't undo — try again.");
});

test("a thrown Undo is a failed Undo", async () => {
  renderShell();
  act(() =>
    notify({
      message: "Removed Rent.",
      undo: async () => {
        throw new Error("boom");
      },
    }),
  );
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
  });
  expect(screen.getByRole("alert")).toHaveTextContent(UNDO_FAILED_MESSAGE);
});

test("a failed Undo stops the timer: the notice waits for the user", async () => {
  renderShell();
  act(() => notify({ message: "Removed Rent.", undo: async () => false }));
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
  });
  act(() => vi.advanceTimersByTime(NOTICE_DURATION_MS * 5));
  expect(region()).not.toBeNull();
});

test("retrying after a failure can succeed, and clears the error", async () => {
  renderShell();
  const undo = vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true);
  act(() => notify({ message: "Removed Rent.", undo }));
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
  });
  expect(screen.getByRole("alert")).toBeInTheDocument();
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
  });
  expect(undo).toHaveBeenCalledTimes(2);
  expect(region()).toBeNull();
});

test("notices stack, newest last, so a second removal keeps the first one's Undo", () => {
  renderShell();
  act(() => notify({ message: "Removed Rent.", undo: undoOk() }));
  act(() => notify({ message: "Removed Gym.", undo: undoOk() }));
  const messages = screen.getAllByRole("status").map((node) => node.textContent);
  expect(messages[0]).toContain("Removed Rent.");
  expect(messages[1]).toContain("Removed Gym.");
});

test("a fourth notice closes the oldest early", () => {
  renderShell();
  for (const name of ["One", "Two", "Three", "Four"]) {
    act(() => notify({ message: `Removed ${name}.`, undo: undoOk() }));
  }
  const shown = screen.getAllByRole("status").map((node) => node.textContent ?? "");
  expect(shown).toHaveLength(3);
  expect(shown.join(" ")).not.toContain("Removed One.");
  expect(shown.join(" ")).toContain("Removed Four.");
});

test("closing a notice that held focus gives focus back to where it came from", () => {
  renderShell();
  const page = screen.getByRole("button", { name: "page content" });
  page.focus();
  act(() => notify({ message: "Removed Rent.", undo: undoOk() }));
  // The user tabs from the page into the notice, then dismisses it with the keyboard.
  fireEvent.focus(screen.getByRole("button", { name: "Undo" }), { relatedTarget: page });
  screen.getByRole("button", { name: "Undo" }).focus();
  fireEvent.keyDown(screen.getByRole("button", { name: "Undo" }), { key: "Escape" });
  expect(region()).toBeNull();
  expect(page).toHaveFocus();
});

test("useNotices outside a provider throws, so an Undo is never silently lost", () => {
  const spy = vi.spyOn(console, "error").mockImplementation(() => {});
  expect(() => render(<Harness />)).toThrow(/NoticeProvider/);
  spy.mockRestore();
});
