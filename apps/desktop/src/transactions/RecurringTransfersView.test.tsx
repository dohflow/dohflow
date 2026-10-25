import { fireEvent, screen, waitFor, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { AccountViewDto, RecurringTransferDto } from "@/bindings";
import { RecurringTransfersView } from "./RecurringTransfersView";

const mocks = vi.hoisted(() => ({
  recurringTransferList: vi.fn(),
  createRecurringTransfer: vi.fn(),
  deleteRecurringTransfer: vi.fn(),
  reinstateRecurringTransfer: vi.fn(),
  accountList: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    recurringTransferList: mocks.recurringTransferList,
    createRecurringTransfer: mocks.createRecurringTransfer,
    deleteRecurringTransfer: mocks.deleteRecurringTransfer,
    reinstateRecurringTransfer: mocks.reinstateRecurringTransfer,
    accountList: mocks.accountList,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

const TRANSFER: RecurringTransferDto = {
  id: "rt-1",
  source_account_id: "a-1",
  source_account_name: "Checking",
  dest_account_id: "a-2",
  dest_account_name: "Savings",
  amount: { minor_units: 30_000, currency: "USD" },
  frequency: "monthly",
  anchor_date: "2026-07-01",
  next_date: "2026-07-01",
  created_at: "2026-06-27T00:00:00Z",
};

beforeEach(() => {
  vi.clearAllMocks();
  mocks.recurringTransferList.mockResolvedValue(ok([TRANSFER]));
  mocks.deleteRecurringTransfer.mockResolvedValue(ok({ op_seq: 1, replayed: false }));
  mocks.reinstateRecurringTransfer.mockResolvedValue(ok({ op_seq: 3, replayed: false }));
  mocks.createRecurringTransfer.mockResolvedValue(ok({ op_seq: 2, replayed: false }));
  mocks.accountList.mockResolvedValue(
    ok([
      {
        id: "a-1",
        name: "Checking",
        cashflow_role: "liquid_cash",
        subtype: null,
        active: true,
        balance: { minor_units: 500_000, currency: "USD" },
      } as AccountViewDto,
      {
        id: "a-2",
        name: "Savings",
        cashflow_role: "liquid_cash",
        subtype: null,
        active: true,
        balance: { minor_units: 100_000, currency: "USD" },
      } as AccountViewDto,
    ]),
  );
});

describe("RecurringTransfersView", () => {
  it("lists a scheduled transfer with its accounts, amount, and cadence", async () => {
    renderWithClient(<RecurringTransfersView />);
    expect(await screen.findByText("Checking")).toBeInTheDocument();
    expect(screen.getByText("Savings")).toBeInTheDocument();
    expect(screen.getByText(/\$300/)).toBeInTheDocument();
    expect(screen.getByText(/Monthly/)).toBeInTheDocument();
  });

  it("shows the empty state with no scheduled transfers", async () => {
    mocks.recurringTransferList.mockResolvedValue(ok([]));
    renderWithClient(<RecurringTransfersView />);
    expect(
      await screen.findByText(/no scheduled transfers/i),
    ).toBeInTheDocument();
  });

  it("adds a recurring transfer right from the tab, custom interval included (4d8.25.12 + ADR 0048)", async () => {
    renderWithClient(<RecurringTransfersView />);
    fireEvent.click(
      await screen.findByRole("button", { name: /add recurring transfer/i }),
    );
    // The form is LOCKED to recurring mode: no one-off/recurring choice.
    expect(await screen.findByLabelText("Amount")).toBeInTheDocument();
    expect(screen.queryByText("Repeat")).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Amount"), {
      target: { value: "150.00" },
    });
    // Pick a custom cadence: every 6 weeks (ADR 0048).
    fireEvent.change(screen.getByLabelText("Frequency"), {
      target: { value: "custom" },
    });
    fireEvent.change(screen.getByLabelText("Interval count"), {
      target: { value: "6" },
    });
    fireEvent.change(screen.getByLabelText("Interval unit"), {
      target: { value: "weeks" },
    });
    fireEvent.click(screen.getByRole("button", { name: /schedule transfer/i }));
    await waitFor(() =>
      expect(mocks.createRecurringTransfer).toHaveBeenCalledWith(
        expect.objectContaining({
          source_account_id: "a-1",
          dest_account_id: "a-2",
          frequency: "every_6_weeks",
          amount: { minor_units: 15_000, currency: "USD" },
        }),
      ),
    );
  });

  // Remove with undo (docs/agent/FRONTEND.md section 6.2, personal-cfo-egn67).
  it("removes a scheduled transfer behind a confirm that says it can be brought back", async () => {
    renderWithClient(<RecurringTransfersView />);
    fireEvent.click(
      await screen.findByRole("button", { name: /^remove transfer from checking to savings$/i }),
    );
    // A transfer used to be removed on the first click; now it is confirmed first.
    expect(mocks.deleteRecurringTransfer).not.toHaveBeenCalled();
    const confirm = screen.getByRole("group", { name: "Remove transfer from Checking to Savings?" });
    expect(within(confirm).getByText("You can bring it back.")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /confirm remove transfer/i }));
    await waitFor(() =>
      // The key is minted per action (personal-cfo-3fdd.5) — non-empty, not "".
      expect(mocks.deleteRecurringTransfer).toHaveBeenCalledWith(
        "rt-1",
        expect.stringMatching(/.+/),
      ),
    );
  });

  it("after a removal a notice says so, and Undo reinstates that transfer", async () => {
    renderWithClient(<RecurringTransfersView />);
    fireEvent.click(
      await screen.findByRole("button", { name: /^remove transfer from checking to savings$/i }),
    );
    fireEvent.click(screen.getByRole("button", { name: /confirm remove transfer/i }));

    const region = await screen.findByRole("region", { name: "Notifications" });
    expect(
      await within(region).findByText("Removed transfer from Checking to Savings."),
    ).toBeInTheDocument();
    fireEvent.click(within(region).getByRole("button", { name: "Undo" }));
    await waitFor(() =>
      expect(mocks.reinstateRecurringTransfer).toHaveBeenCalledWith(
        "rt-1",
        expect.stringMatching(/.+/),
      ),
    );
  });

  it("after a removal focus moves to the nearest surviving row, not the page body", async () => {
    const second = {
      ...TRANSFER,
      id: "rt-2",
      source_account_name: "Savings",
      dest_account_name: "Brokerage",
    };
    mocks.recurringTransferList.mockResolvedValue(ok([TRANSFER, second]));
    mocks.deleteRecurringTransfer.mockImplementation(async () => {
      mocks.recurringTransferList.mockResolvedValue(ok([TRANSFER]));
      return ok({ op_seq: 1, replayed: false });
    });
    renderWithClient(<RecurringTransfersView />);
    fireEvent.click(
      await screen.findByRole("button", { name: /^remove transfer from savings to brokerage$/i }),
    );
    fireEvent.click(screen.getByRole("button", { name: /confirm remove transfer/i }));

    await screen.findByRole("region", { name: "Notifications" });
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: /^remove transfer from checking to savings$/i }),
      ).toHaveFocus(),
    );
  });

  it("a failed removal shows its error and no notice", async () => {
    mocks.deleteRecurringTransfer.mockResolvedValue({
      status: "error",
      error: { Validation: "recurring transfer does not exist" },
    });
    renderWithClient(<RecurringTransfersView />);
    fireEvent.click(
      await screen.findByRole("button", { name: /^remove transfer from checking to savings$/i }),
    );
    fireEvent.click(screen.getByRole("button", { name: /confirm remove transfer/i }));
    expect(await screen.findByText("recurring transfer does not exist")).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Notifications" })).toBeNull();
  });

  it("cancelling the confirm removes nothing", async () => {
    renderWithClient(<RecurringTransfersView />);
    fireEvent.click(
      await screen.findByRole("button", { name: /^remove transfer from checking to savings$/i }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(mocks.deleteRecurringTransfer).not.toHaveBeenCalled();
  });
});
