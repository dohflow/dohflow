import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { ManualFutureEntryDto } from "@/bindings";
import { FutureCashEntries } from "./FutureCashEntries";

const mocks = vi.hoisted(() => ({
  manualFutureEntryList: vi.fn(),
  createManualFutureEntry: vi.fn(),
  updateManualFutureEntry: vi.fn(),
  deleteManualFutureEntry: vi.fn(),
  accountList: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    manualFutureEntryList: mocks.manualFutureEntryList,
    createManualFutureEntry: mocks.createManualFutureEntry,
    updateManualFutureEntry: mocks.updateManualFutureEntry,
    deleteManualFutureEntry: mocks.deleteManualFutureEntry,
    accountList: mocks.accountList,
  },
}));

const CHECKING_ID = "0190a000-0000-7000-8000-000000000001";

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

function entry(over: Partial<ManualFutureEntryDto> = {}): ManualFutureEntryDto {
  return {
    id: "entry-1",
    amount: { minor_units: -180_000, currency: "USD" },
    date: "2026-07-01",
    label: "Rent",
    account_id: null,
    matched_transaction_id: null,
    ...over,
  };
}

beforeEach(() => {
  mocks.manualFutureEntryList.mockReset().mockResolvedValue(ok([]));
  mocks.createManualFutureEntry.mockReset();
  mocks.updateManualFutureEntry.mockReset();
  mocks.deleteManualFutureEntry.mockReset();
  mocks.accountList.mockReset().mockResolvedValue(
    ok([
      {
        id: CHECKING_ID,
        name: "Checking",
        cashflow_role: "liquid_cash",
        subtype: null,
        active: true,
        balance: { minor_units: 500_000, currency: "USD" },
        notes: null,
        linked_account_id: null,
        linked_account_name: null,
      },
    ]),
  );
});

describe("FutureCashEntries", () => {
  it("adds a money-in entry with the signed amount", async () => {
    mocks.createManualFutureEntry.mockResolvedValue(ok(entry({ id: "new" })));
    renderWithClient(<FutureCashEntries currency="USD" />);

    fireEvent.click(screen.getByRole("button", { name: /add entry/i }));
    fireEvent.change(screen.getByLabelText("Label"), {
      target: { value: "Bonus" },
    });
    fireEvent.change(screen.getByLabelText(/Amount/), {
      target: { value: "5000" },
    });
    fireEvent.change(screen.getByLabelText("Date"), {
      target: { value: "2026-08-01" },
    });
    // Direction defaults to "Money in" → a positive amount.
    fireEvent.click(screen.getByRole("button", { name: /add entry/i }));

    await waitFor(() =>
      expect(mocks.createManualFutureEntry).toHaveBeenCalledWith({
        amount: { minor_units: 500_000, currency: "USD" },
        date: "2026-08-01",
        label: "Bonus",
        // No account chosen → Unallocated (personal-cfo-4d8.24.3).
        account_id: null,
      }),
    );
  });

  it("attributes an entry to a chosen liquid account (4d8.24.3)", async () => {
    mocks.createManualFutureEntry.mockResolvedValue(ok(entry({ id: "new" })));
    renderWithClient(<FutureCashEntries currency="USD" />);

    fireEvent.click(screen.getByRole("button", { name: /add entry/i }));
    fireEvent.change(screen.getByLabelText("Label"), {
      target: { value: "Insurance" },
    });
    fireEvent.change(screen.getByLabelText(/Amount/), {
      target: { value: "1000" },
    });
    fireEvent.click(
      screen.getByRole("button", { name: /money out/i }),
    );
    // Wait for the liquid accounts to load into the from/to picker, then choose Checking.
    await screen.findByRole("option", { name: "Checking" });
    fireEvent.change(screen.getByLabelText("From account"), {
      target: { value: CHECKING_ID },
    });
    fireEvent.click(screen.getByRole("button", { name: /add entry/i }));

    await waitFor(() =>
      expect(mocks.createManualFutureEntry).toHaveBeenCalledWith(
        expect.objectContaining({
          label: "Insurance",
          amount: { minor_units: -100_000, currency: "USD" },
          account_id: CHECKING_ID,
        }),
      ),
    );
  });

  it("lists entries and edits one via supersede", async () => {
    mocks.manualFutureEntryList.mockResolvedValue(ok([entry()]));
    mocks.updateManualFutureEntry.mockResolvedValue(ok(entry({ id: "new" })));
    renderWithClient(<FutureCashEntries currency="USD" />);

    // The existing entry is listed.
    expect(await screen.findByText("Rent")).toBeInTheDocument();

    // Edit prefills the form; saving supersedes (update by id).
    fireEvent.click(screen.getByRole("button", { name: "Edit Rent" }));
    fireEvent.change(screen.getByLabelText("Label"), {
      target: { value: "Rent (raised)" },
    });
    fireEvent.click(screen.getByRole("button", { name: /save changes/i }));

    await waitFor(() =>
      expect(mocks.updateManualFutureEntry).toHaveBeenCalledWith({
        id: "entry-1",
        amount: { minor_units: -180_000, currency: "USD" },
        date: "2026-07-01",
        label: "Rent (raised)",
        // The existing entry had no account; editing preserves that (Unallocated).
        account_id: null,
      }),
    );
  });

  it("deletes an entry by id", async () => {
    mocks.manualFutureEntryList.mockResolvedValue(ok([entry()]));
    mocks.deleteManualFutureEntry.mockResolvedValue(ok(null));
    renderWithClient(<FutureCashEntries currency="USD" />);

    fireEvent.click(await screen.findByRole("button", { name: "Delete Rent" }));
    await waitFor(() =>
      expect(mocks.deleteManualFutureEntry).toHaveBeenCalledWith("entry-1"),
    );
  });

  it("shows a matched entry as cleared from the forecast (xtz5)", async () => {
    mocks.manualFutureEntryList.mockResolvedValue(
      ok([
        entry({
          matched_transaction_id: "99999999-9999-7999-8999-999999999999",
        }),
      ]),
    );
    const { findByText } = renderWithClient(<FutureCashEntries currency="USD" />);
    expect(await findByText("Matched")).toBeInTheDocument();
    expect(
      await findByText(/no longer counts in the forecast/),
    ).toBeInTheDocument();
  });
});
