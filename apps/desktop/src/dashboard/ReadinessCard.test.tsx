import { fireEvent, screen, waitFor, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { ForecastReadinessDto } from "@/bindings";
import { ReadinessCard } from "./ReadinessCard";

const mocks = vi.hoisted(() => ({
  forecastReadiness: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: { forecastReadiness: mocks.forecastReadiness },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

function readiness(over: Partial<ForecastReadinessDto> = {}): ForecastReadinessDto {
  return {
    score: 85,
    factors: [
      { key: "coverage", label: "Coverage", score: 100, detail: "All set." },
      { key: "freshness", label: "Balance freshness", score: 100, detail: "Current." },
      {
        key: "explained",
        label: "Explained activity",
        score: 55,
        detail: "Record transactions to explain the balance.",
      },
    ],
    ...over,
  };
}

beforeEach(() => {
  mocks.forecastReadiness.mockReset();
});

describe("ReadinessCard", () => {
  it("shows the score, band, and the weakest factor's next action", async () => {
    mocks.forecastReadiness.mockResolvedValue(ok(readiness()));
    renderWithClient(<ReadinessCard />);

    expect(await screen.findByText("85")).toBeInTheDocument();
    expect(screen.getByText("/100")).toBeInTheDocument();
    expect(screen.getByText("Good")).toBeInTheDocument();
    // Collapsed headline = the lowest-scoring factor's detail (explained, 55).
    expect(
      screen.getByText(/record transactions to explain the balance/i),
    ).toBeInTheDocument();
  });

  it("expands to the per-factor breakdown", async () => {
    mocks.forecastReadiness.mockResolvedValue(ok(readiness()));
    renderWithClient(<ReadinessCard />);

    fireEvent.click(await screen.findByRole("button", { name: /details/i }));
    expect(screen.getByText("Coverage")).toBeInTheDocument();
    expect(screen.getByText("Balance freshness")).toBeInTheDocument();
    expect(screen.getByText("Explained activity")).toBeInTheDocument();
    expect(screen.getByText("All set.")).toBeInTheDocument();
  });

  it("shows a positive headline when every factor is maxed", async () => {
    mocks.forecastReadiness.mockResolvedValue(
      ok(
        readiness({
          score: 100,
          factors: [
            { key: "coverage", label: "Coverage", score: 100, detail: "All set." },
            { key: "freshness", label: "Balance freshness", score: 100, detail: "Current." },
            { key: "explained", label: "Explained activity", score: 100, detail: "Explained." },
          ],
        }),
      ),
    );
    renderWithClient(<ReadinessCard />);

    expect(await screen.findByText("100")).toBeInTheDocument();
    expect(screen.getByText(/trustworthy as today's inputs allow/i)).toBeInTheDocument();
  });

  it("surfaces a load error", async () => {
    mocks.forecastReadiness.mockRejectedValue(new Error("ipc down"));
    renderWithClient(<ReadinessCard />);
    await waitFor(() => expect(screen.getByRole("alert")).toBeInTheDocument());
  });

  it("keeps the card compact and opens the breakdown in a dialog (4d8.27.2)", async () => {
    mocks.forecastReadiness.mockResolvedValue(ok(readiness()));
    renderWithClient(<ReadinessCard />);
    await screen.findByText("85");
    // Compact: the per-factor breakdown is NOT in the flow — it must not push the
    // dashboard down, which is what the owner objected to.
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.queryByText("Balance freshness")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Details" }));
    const dialog = await screen.findByRole("dialog", {
      name: /forecast readiness detail/i,
    });
    expect(within(dialog).getByText("Balance freshness")).toBeInTheDocument();

    // Escape closes it.
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});
