// SuggestedIncome (personal-cfo-gmnk): detected deposits render as prefilled
// approve / deny rows — approving creates the income source from the prefill,
// dismissing records the shared suppression; nothing is auto-created.

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { renderWithClient } from "@/test/renderWithClient";

const mocks = vi.hoisted(() => ({
  incomeCandidates: vi.fn(),
  dismissRecurringSuggestion: vi.fn(),
  createIncomeSource: vi.fn(),
  incomeSourceList: vi.fn(),
  accountList: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: { ...mocks },
}));

vi.mock("@/vault/useVault", () => ({
  describeIpcError: (error: unknown) =>
    typeof error === "string" ? error : "Something went wrong.",
}));

import { SuggestedIncome } from "./SuggestedIncome";

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const CHECKING = "0190a000-0000-7000-8000-000000000001";

function candidate() {
  return {
    merchant_key: "acme payroll",
    display: "ACME PAYROLL",
    amount_minor: 250_000,
    amount_min_minor: 250_000,
    amount_max_minor: 250_000,
    currency: "USD",
    frequency: "biweekly",
    last_seen: "2026-08-21",
    next_date: "2026-09-04",
    occurrence_count: 4,
    confidence_bps: 9000,
    dominant_category_id: null,
    source_account_names: ["Checking"],
    source_account_id: CHECKING,
    typical_day_of_month: null,
    observations: [],
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.incomeCandidates.mockResolvedValue(ok([candidate()]));
  mocks.incomeSourceList.mockResolvedValue(ok([]));
  mocks.accountList.mockResolvedValue(
    ok([
      {
        id: CHECKING,
        name: "Checking",
        cashflow_role: "liquid_cash",
        subtype: null,
        active: true,
        balance: { minor_units: 500_000, currency: "USD" },
      },
    ]),
  );
  mocks.createIncomeSource.mockResolvedValue(ok({ op_seq: 1, replayed: false }));
  mocks.dismissRecurringSuggestion.mockResolvedValue(ok(null));
});

describe("SuggestedIncome", () => {
  it("renders nothing when there is nothing to suggest", async () => {
    mocks.incomeCandidates.mockResolvedValue(ok([]));
    const { container } = renderWithClient(<SuggestedIncome />);
    await waitFor(() => expect(mocks.incomeCandidates).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });

  it("lists a detected deposit with its cadence and account", async () => {
    renderWithClient(<SuggestedIncome />);
    expect(await screen.findByText("Acme Payroll")).toBeInTheDocument();
    expect(screen.getByText(/every 2 weeks/)).toBeInTheDocument();
    expect(screen.getByText(/into Checking/)).toBeInTheDocument();
  });

  it("approves through the prefilled income form", async () => {
    renderWithClient(<SuggestedIncome />);
    fireEvent.click(await screen.findByRole("button", { name: /add as income/i }));
    // Prefilled from the candidate — the user only confirms.
    expect(screen.getByDisplayValue("Acme Payroll")).toBeInTheDocument();
    expect(screen.getByDisplayValue("2500")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /^add income$/i }));
    await waitFor(() => expect(mocks.createIncomeSource).toHaveBeenCalledTimes(1));
    expect(mocks.createIncomeSource.mock.calls[0]?.[0]).toMatchObject({
      name: "Acme Payroll",
      net_amount: { minor_units: 250_000, currency: "USD" },
      frequency: "biweekly",
      anchor_date: "2026-08-21",
      deposit_account_id: CHECKING,
    });
  });

  it("dismisses into the shared suppression store", async () => {
    renderWithClient(<SuggestedIncome />);
    fireEvent.click(await screen.findByRole("button", { name: /dismiss/i }));
    await waitFor(() =>
      expect(mocks.dismissRecurringSuggestion).toHaveBeenCalledWith(
        expect.objectContaining({
          merchant_key: "acme payroll",
          currency: "USD",
          amount_minor: 250_000,
          frequency: "biweekly",
          reason: null,
        }),
      ),
    );
  });
});
