import { useState } from "react";
import { fireEvent, screen, waitFor, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import { NewLoanDialog } from "./NewLoanDialog";

const mocks = vi.hoisted(() => ({
  accountList: vi.fn(),
  createAccount: vi.fn(),
}));
vi.mock("@/bindings", () => ({
  commands: { accountList: mocks.accountList, createAccount: mocks.createAccount },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

beforeEach(() => {
  mocks.accountList.mockReset();
  mocks.createAccount.mockReset();
  mocks.accountList.mockResolvedValue(ok([]));
  mocks.createAccount.mockResolvedValue(
    ok({ account_id: "loan-new", mutation: { op_seq: 1, replayed: false } }),
  );
});

function open(handlers: { onCreated?: () => void; onClose?: () => void } = {}) {
  const onClose = handlers.onClose ?? vi.fn();
  const onCreated = handlers.onCreated ?? vi.fn();
  renderWithClient(
    <div>
      <button>Opener</button>
      <NewLoanDialog currency="USD" onCreated={onCreated} onClose={onClose} />
    </div>,
  );
  return { onClose, onCreated };
}

describe("NewLoanDialog on the shared Dialog (personal-cfo-4d8.6.2)", () => {
  it("is named by its title, starts in the name field and keeps Tab inside", async () => {
    open();
    const dialog = await screen.findByRole("dialog", { name: "New loan" });
    const name = within(dialog).getByLabelText("Loan name");
    await waitFor(() => expect(name).toHaveFocus());
    const create = within(dialog).getByRole("button", { name: "Create loan" });
    create.focus();
    fireEvent.keyDown(create, { key: "Tab" });
    // Tab from the last control wraps to the first control inside, never out to the page.
    expect(dialog).toContainElement(document.activeElement as HTMLElement);
    // The page behind is hidden from assistive technology.
    expect(screen.queryByRole("button", { name: "Opener" })).not.toBeInTheDocument();
  });

  it("closes on Escape and on Cancel without creating anything", async () => {
    const { onClose } = open();
    const dialog = await screen.findByRole("dialog", { name: "New loan" });
    fireEvent.keyDown(dialog, { key: "Escape" });
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(onClose).toHaveBeenCalledTimes(2);
    expect(mocks.createAccount).not.toHaveBeenCalled();
  });

  it("Escape closes only this dialog: a listener on the window behind never sees the key", async () => {
    open();
    const behind = vi.fn();
    window.addEventListener("keydown", behind);
    const dialog = await screen.findByRole("dialog", { name: "New loan" });
    fireEvent.keyDown(dialog, { key: "Escape" });
    window.removeEventListener("keydown", behind);
    expect(behind).not.toHaveBeenCalled();
  });

  it("shows a refusal in the dialog's alert and stays open", async () => {
    mocks.createAccount.mockResolvedValue({ status: "error", error: { Validation: "Name taken." } });
    const { onClose } = open();
    const dialog = await screen.findByRole("dialog", { name: "New loan" });
    fireEvent.change(within(dialog).getByLabelText("Loan name"), { target: { value: "Car" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Create loan" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("Name taken.");
    expect(onClose).not.toHaveBeenCalled();
  });

  it("asks for a name first", async () => {
    open();
    const dialog = await screen.findByRole("dialog", { name: "New loan" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Create loan" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("Enter a name for the loan.");
    expect(mocks.createAccount).not.toHaveBeenCalled();
  });

  it("returns focus to the button that opened it", async () => {
    function Harness() {
      const [creating, setCreating] = useState(false);
      return (
        <div>
          <button onClick={() => setCreating(true)}>New loan</button>
          {creating && (
            <NewLoanDialog
              currency="USD"
              onCreated={() => undefined}
              onClose={() => setCreating(false)}
            />
          )}
        </div>
      );
    }
    renderWithClient(<Harness />);
    const opener = screen.getByRole("button", { name: "New loan" });
    opener.focus();
    fireEvent.click(opener);
    const dialog = await screen.findByRole("dialog", { name: "New loan" });
    fireEvent.keyDown(dialog, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    await waitFor(() => expect(screen.getByRole("button", { name: "New loan" })).toHaveFocus());
  });
});
