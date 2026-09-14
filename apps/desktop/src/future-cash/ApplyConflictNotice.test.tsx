import { screen } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import { ApplyConflictNotice } from "./ApplyConflictNotice";

const mocks = vi.hoisted(() => ({
  forecastAssumptionList: vi.fn(),
  baseCurrency: vi.fn(),
}));
vi.mock("@/bindings", () => ({ commands: mocks }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

const event = (over: Record<string, unknown> = {}) => ({
  id: "e1",
  kind: "bill_amount",
  target_entity_id: "rent",
  scenario_id: null,
  params_json: JSON.stringify({ new_amount_minor: 250_000 }),
  created_at: "2026-08-01",
  promoted_from_scenario_id: null,
  ...over,
});

beforeEach(() => {
  vi.clearAllMocks();
  mocks.baseCurrency.mockResolvedValue(ok("USD"));
  mocks.forecastAssumptionList.mockResolvedValue(ok([]));
});

describe("ApplyConflictNotice (personal-cfo-4d8.27.6.5, ADR 0059 §3)", () => {
  it("names the value the forecast uses now and the one it would use after", async () => {
    mocks.forecastAssumptionList.mockImplementation((scenarioId: string | null) =>
      Promise.resolve(
        ok(
          scenarioId === null
            ? [event({ id: "base1" })]
            : [
                event({
                  id: "scn1",
                  scenario_id: "scn-1",
                  params_json: JSON.stringify({ new_amount_minor: 300_000 }),
                }),
              ],
        ),
      ),
    );
    renderWithClient(<ApplyConflictNotice scenarioId="scn-1" />);

    const notice = await screen.findByRole("status");
    expect(notice).toHaveTextContent("This replaces one assumption you already have");
    expect(notice).toHaveTextContent("$2,500.00");
    expect(notice).toHaveTextContent("$3,000.00");
    // Descriptive per ADR 0018: it says what happens and how to undo it, and does not
    // block or scold.
    expect(notice).toHaveTextContent(/reverting this scenario restores it/i);
  });

  it("renders nothing when the scenario touches nothing the forecast already assumes", async () => {
    mocks.forecastAssumptionList.mockImplementation((scenarioId: string | null) =>
      Promise.resolve(
        ok(
          scenarioId === null
            ? [event({ id: "base1", target_entity_id: "phone" })]
            : [event({ id: "scn1", scenario_id: "scn-1", target_entity_id: "rent" })],
        ),
      ),
    );
    const { container } = renderWithClient(<ApplyConflictNotice scenarioId="scn-1" />);
    await vi.waitFor(() =>
      expect(mocks.forecastAssumptionList).toHaveBeenCalledTimes(2),
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("does not flash an all-clear before both sides have loaded", async () => {
    // Reporting "no conflicts" from a half-loaded comparison would be an all-clear the
    // check has not actually performed — and this one gates a durable write.
    let resolveScenario: ((v: unknown) => void) | undefined;
    mocks.forecastAssumptionList.mockImplementation((scenarioId: string | null) =>
      scenarioId === null
        ? Promise.resolve(ok([event({ id: "base1" })]))
        : new Promise((r) => {
            resolveScenario = r;
          }),
    );
    const { container } = renderWithClient(<ApplyConflictNotice scenarioId="scn-1" />);
    await vi.waitFor(() => expect(mocks.forecastAssumptionList).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();

    resolveScenario?.(
      ok([
        event({
          id: "scn1",
          scenario_id: "scn-1",
          params_json: JSON.stringify({ new_amount_minor: 300_000 }),
        }),
      ]),
    );
    expect(await screen.findByRole("status")).toBeInTheDocument();
  });
});
