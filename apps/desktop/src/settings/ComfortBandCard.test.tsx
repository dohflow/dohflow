import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { ComfortBandDto, MoneyDto } from "@/bindings";
import { ComfortBandCard } from "./ComfortBandCard";

const mocks = vi.hoisted(() => ({
  comfortBand: vi.fn(),
  setMinimumCashFloor: vi.fn(),
  setComfortBandUpper: vi.fn(),
}));

vi.mock("@/bindings", () => ({ commands: mocks }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const usd = (minor: number): MoneyDto => ({ minor_units: minor, currency: "USD" });
function band(lowerMinor: number, upperMinor: number | null): ComfortBandDto {
  return {
    currency: "USD",
    lower: usd(lowerMinor),
    upper: upperMinor === null ? null : usd(upperMinor),
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.comfortBand.mockResolvedValue(ok(band(0, null)));
  mocks.setMinimumCashFloor.mockResolvedValue(ok(null));
  mocks.setComfortBandUpper.mockResolvedValue(ok(null));
});

describe("ComfortBandCard", () => {
  it("loads the stored lower + upper edges", async () => {
    mocks.comfortBand.mockResolvedValue(ok(band(100_000, 1_000_000)));
    renderWithClient(<ComfortBandCard />);

    await waitFor(() =>
      expect(screen.getByLabelText(/lower edge/i)).toHaveValue("1000"),
    );
    expect(screen.getByLabelText(/upper edge/i)).toHaveValue("10000");
  });

  it("saves the lower + upper edges as minor units", async () => {
    renderWithClient(<ComfortBandCard />);
    await waitFor(() =>
      expect(screen.getByLabelText(/lower edge/i)).toHaveValue(""),
    );

    fireEvent.change(screen.getByLabelText(/lower edge/i), {
      target: { value: "5,000" },
    });
    fireEvent.change(screen.getByLabelText(/upper edge/i), {
      target: { value: "10000" },
    });
    fireEvent.click(screen.getByRole("button", { name: /save/i }));

    await waitFor(() =>
      expect(mocks.setMinimumCashFloor).toHaveBeenCalledWith({
        minor_units: 500_000,
        currency: "USD",
      }),
    );
    expect(mocks.setComfortBandUpper).toHaveBeenCalledWith({
      minor_units: 1_000_000,
      currency: "USD",
    });
    expect(await screen.findByText(/saved/i)).toBeInTheDocument();
  });

  it("clears the upper edge when left blank", async () => {
    mocks.comfortBand.mockResolvedValue(ok(band(100_000, 1_000_000)));
    renderWithClient(<ComfortBandCard />);
    await waitFor(() =>
      expect(screen.getByLabelText(/upper edge/i)).toHaveValue("10000"),
    );

    fireEvent.change(screen.getByLabelText(/upper edge/i), {
      target: { value: "" },
    });
    fireEvent.click(screen.getByRole("button", { name: /save/i }));

    await waitFor(() =>
      expect(mocks.setComfortBandUpper).toHaveBeenCalledWith(null),
    );
  });

  it("rejects an upper edge below the lower edge without calling the backend", async () => {
    renderWithClient(<ComfortBandCard />);
    await screen.findByLabelText(/lower edge/i);

    fireEvent.change(screen.getByLabelText(/lower edge/i), {
      target: { value: "5000" },
    });
    fireEvent.change(screen.getByLabelText(/upper edge/i), {
      target: { value: "1000" },
    });
    fireEvent.click(screen.getByRole("button", { name: /save/i }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      /at or above the lower/i,
    );
    expect(mocks.setMinimumCashFloor).not.toHaveBeenCalled();
  });
});
