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
let notifyError: (message: string) => void = () => {};
function Harness() {
  ({ notify, notifyError } = useNotices());
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
/// The notice's card (the region's child), which holds the status line and the buttons.
const card = (index = 0) => region()!.children[index] as HTMLElement;
/// One animation frame: the status element gets its message on the frame after the notice mounts.
const frame = () => act(() => vi.advanceTimersByTime(16));

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
  frame();
  const inRegion = within(screen.getByRole("region", { name: "Notifications" }));
  expect(inRegion.getByRole("status")).toHaveTextContent("Removed Rent.");
  expect(inRegion.getByRole("button", { name: "Undo" })).toBeInTheDocument();
});

// A live region that mounts together with its text is not reliably announced (review F1): the status
// element is in the page EMPTY first, and the message arrives on the next frame.
test("the status element mounts empty and receives its message on the next frame", () => {
  renderShell();
  act(() => notify({ message: "Removed Rent.", undo: undoOk() }));
  const status = screen.getByRole("status");
  expect(status).toBeEmptyDOMElement();
  expect(status).not.toHaveTextContent("Removed Rent.");
  frame();
  // The SAME element: the live region was already registered when the text arrived.
  expect(screen.getByRole("status")).toBe(status);
  expect(status).toHaveTextContent("Removed Rent.");
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
  const status = card();
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
  // Its own alert element inside the notice, a sibling of the status line (never nested in it).
  expect(within(card()).getByRole("alert")).toHaveTextContent(UNDO_FAILED_MESSAGE);
  expect(screen.getByRole("status")).not.toContainElement(screen.getByRole("alert"));
  expect(within(card()).getByRole("button", { name: "Undo" })).toBeEnabled();
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
  frame();
  const messages = screen.getAllByRole("status").map((node) => node.textContent);
  expect(messages[0]).toContain("Removed Rent.");
  expect(messages[1]).toContain("Removed Gym.");
});

test("a fourth notice closes the oldest early", () => {
  renderShell();
  for (const name of ["One", "Two", "Three", "Four"]) {
    act(() => notify({ message: `Removed ${name}.`, undo: undoOk() }));
  }
  frame();
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

// ----- the error notice: an action partly worked (FRONTEND.md 6.2, personal-cfo-6wk.22 item 9) -----

test("an error notice is an alert with no Undo, filled on the frame after it mounts", () => {
  renderShell();
  act(() => notifyError("The bill was added, but linking it didn't work."));
  const alert = screen.getByRole("alert");
  expect(alert).toBeEmptyDOMElement();
  frame();
  expect(screen.getByRole("alert")).toBe(alert);
  expect(alert).toHaveTextContent("The bill was added, but linking it didn't work.");
  expect(within(region()!).queryByRole("button", { name: "Undo" })).toBeNull();
  expect(screen.queryByRole("status")).toBeNull();
});

test("an error notice never closes on a timer", () => {
  renderShell();
  act(() => notifyError("The bill was added, but linking it didn't work."));
  act(() => vi.advanceTimersByTime(NOTICE_DURATION_MS * 100));
  expect(region()).not.toBeNull();
});

test("an error notice closes with its close button", () => {
  renderShell();
  act(() => notifyError("The bill was added, but linking it didn't work."));
  fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
  expect(region()).toBeNull();
});

test("an error notice closes with Escape while focus is inside it, and not otherwise", () => {
  renderShell();
  act(() => notifyError("The bill was added, but linking it didn't work."));
  fireEvent.keyDown(screen.getByRole("button", { name: "page content" }), { key: "Escape" });
  expect(region()).not.toBeNull();
  const dismiss = screen.getByRole("button", { name: "Dismiss" });
  act(() => dismiss.focus());
  fireEvent.keyDown(dismiss, { key: "Escape" });
  expect(region()).toBeNull();
});

test("an error notice appearing never moves focus", () => {
  renderShell();
  const page = screen.getByRole("button", { name: "page content" });
  page.focus();
  act(() => notifyError("The bill was added, but linking it didn't work."));
  expect(page).toHaveFocus();
});

// ----- when the stack is full (FRONTEND.md 6.2, personal-cfo-4d8.32 item 4) -----

test("a fourth notice evicts the oldest UNDO notice, never an error", () => {
  renderShell();
  act(() => notifyError("First failure."));
  act(() => notify({ message: "Removed Rent.", undo: undoOk() }));
  act(() => notify({ message: "Removed Gym.", undo: undoOk() }));
  act(() => notify({ message: "Removed Spa.", undo: undoOk() }));
  frame();
  const text = region()!.textContent ?? "";
  // The error is oldest of all but stays; the oldest UNDO notice made room.
  expect(text).toContain("First failure.");
  expect(text).not.toContain("Removed Rent.");
  expect(text).toContain("Removed Gym.");
  expect(text).toContain("Removed Spa.");
});

test("a new error also evicts the oldest Undo notice, not an older error", () => {
  renderShell();
  act(() => notifyError("First failure."));
  act(() => notify({ message: "Removed Rent.", undo: undoOk() }));
  act(() => notifyError("Second failure."));
  act(() => notifyError("Third failure."));
  frame();
  const text = region()!.textContent ?? "";
  expect(text).toContain("First failure.");
  expect(text).toContain("Second failure.");
  expect(text).toContain("Third failure.");
  expect(text).not.toContain("Removed Rent.");
});

test("with the stack full of errors a new notice waits, and shows when one is closed", () => {
  renderShell();
  for (const name of ["One", "Two", "Three"]) act(() => notifyError(`${name} failure.`));
  act(() => notify({ message: "Removed Rent.", undo: undoOk() }));
  frame();
  expect(region()!.textContent).not.toContain("Removed Rent.");
  expect(region()!.textContent).toContain("One failure.");
  // Nothing was lost: closing the first error lets the waiting notice in.
  fireEvent.click(within(card(0)).getByRole("button", { name: "Dismiss" }));
  frame();
  const text = region()!.textContent ?? "";
  expect(text).not.toContain("One failure.");
  expect(text).toContain("Removed Rent.");
  expect(text).toContain("Three failure.");
});

test("waiting notices come in oldest first", () => {
  renderShell();
  for (const name of ["One", "Two", "Three"]) act(() => notifyError(`${name} failure.`));
  act(() => notify({ message: "Removed A.", undo: undoOk() }));
  act(() => notify({ message: "Removed B.", undo: undoOk() }));
  fireEvent.click(within(card(0)).getByRole("button", { name: "Dismiss" }));
  frame();
  expect(region()!.textContent).toContain("Removed A.");
  expect(region()!.textContent).not.toContain("Removed B.");
});

test("an Undo notice still times out, and an error beside it does not", () => {
  renderShell();
  act(() => notifyError("Only failure."));
  act(() => notify({ message: "Removed Rent.", undo: undoOk() }));
  frame();
  act(() => vi.advanceTimersByTime(NOTICE_DURATION_MS));
  const after = region()!.textContent ?? "";
  expect(after).not.toContain("Removed Rent.");
  expect(after).toContain("Only failure.");
});

test("useNotices outside a provider throws, so an Undo is never silently lost", () => {
  const spy = vi.spyOn(console, "error").mockImplementation(() => {});
  expect(() => render(<Harness />)).toThrow(/NoticeProvider/);
  spy.mockRestore();
});
