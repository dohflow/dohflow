import { screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { CashAvailabilityDto, MoneyDto } from "@/bindings";
import { SafeToSpendCard } from "./SafeToSpendCard";

const mocks = vi.hoisted(() => ({
  cashAvailability: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: { cashAvailability: mocks.cashAvailability },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const usd = (minor: number): MoneyDto => ({ minor_units: minor, currency: "USD" });

function availability(over: Partial<CashAvailabilityDto> = {}): CashAvailabilityDto {
  return {
    currency: "USD",
    accounts: [],
    net_available: usd(500_000),
    net_committed: usd(180_000),
    net_headroom: usd(320_000),
    floor: usd(0),
    below_floor: false,
    ...over,
  };
}

beforeEach(() => {
  mocks.cashAvailability.mockReset();
});

describe("SafeToSpendCard", () => {
  it("shows net headroom and the available/committed breakdown", async () => {
    mocks.cashAvailability.mockResolvedValue(ok(availability()));
    renderWithClient(<SafeToSpendCard />);

    expect(await screen.findByText("$3,200.00")).toBeInTheDocument();
    expect(screen.getByText(/\$5,000\.00 available/)).toBeInTheDocument();
    expect(screen.getByText(/\$1,800\.00 committed/)).toBeInTheDocument();
  });

  // personal-cfo-i93n3.27 (found by measuring the populated Dashboard at the largest text size): the hero figure never outgrows the
  // window on a phone; a desktop window keeps its size.
  it("caps the hero figure to the window width on a phone and wraps its breakdown line (i93n3.27)", async () => {
    mocks.cashAvailability.mockResolvedValue(ok(availability()));
    renderWithClient(<SafeToSpendCard />);
    const figure = await screen.findByText("$3,200.00");
    expect(figure).toHaveClass("text-4xl", "compact:text-[min(2.25rem,9vw)]", "compact:[overflow-wrap:anywhere]");
    expect(screen.getByText(/\$5,000\.00 available/)).toHaveClass("compact:[overflow-wrap:anywhere]");
  });

  it("warns when net headroom is below the household floor", async () => {
    mocks.cashAvailability.mockResolvedValue(
      ok(availability({ net_headroom: usd(50_000), floor: usd(100_000), below_floor: true })),
    );
    renderWithClient(<SafeToSpendCard />);

    const alert = await screen.findByText(/below your \$1,000\.00 floor/i);
    expect(alert).toBeInTheDocument();
    // On a phone the message may shrink to the window and break long words (i93n3.27), above-floor pill included.
    expect(alert.parentElement).toHaveClass("compact:max-w-full", "compact:[overflow-wrap:anywhere]");
  });

  it("reassures when headroom is above the floor", async () => {
    mocks.cashAvailability.mockResolvedValue(
      ok(availability({ floor: usd(100_000), below_floor: false })),
    );
    renderWithClient(<SafeToSpendCard />);

    const pill = await screen.findByText(/above your \$1,000\.00 floor/i);
    expect(pill).toBeInTheDocument();
    expect(pill.parentElement).toHaveClass("compact:max-w-full", "compact:[overflow-wrap:anywhere]");
  });

  it("hides the floor line when no floor is set", async () => {
    mocks.cashAvailability.mockResolvedValue(ok(availability({ floor: usd(0) })));
    renderWithClient(<SafeToSpendCard />);

    await screen.findByText("$3,200.00");
    expect(screen.queryByText(/floor/i)).not.toBeInTheDocument();
  });

  it("surfaces a load error", async () => {
    mocks.cashAvailability.mockRejectedValue(new Error("ipc down"));
    renderWithClient(<SafeToSpendCard />);

    await waitFor(() => expect(screen.getByRole("alert")).toBeInTheDocument());
  });
});
