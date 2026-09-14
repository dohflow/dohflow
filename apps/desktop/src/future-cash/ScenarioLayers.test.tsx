import { fireEvent, screen, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import { ScenarioLayers } from "./ScenarioLayers";

const mocks = vi.hoisted(() => ({
  scenarioList: vi.fn(),
  forecastAssumptionList: vi.fn(),
  getSetting: vi.fn(),
}));
vi.mock("@/bindings", () => ({ commands: mocks }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

const scenario = (id: string, name: string) => ({
  id,
  name,
  description: null,
  status: "draft",
  created_at: "2026-08-01",
  updated_at: "2026-08-01",
  expires_on: null,
  event_count: 1,
  applied_at: null,
});

const event = (id: string, scenarioId: string | null, amountMinor: number) => ({
  id,
  kind: "bill_amount",
  target_entity_id: "rent",
  scenario_id: scenarioId,
  params_json: JSON.stringify({ new_amount_minor: amountMinor }),
  created_at: "2026-08-01",
  promoted_from_scenario_id: null,
});

beforeEach(() => {
  vi.clearAllMocks();
  mocks.getSetting.mockResolvedValue(ok(null));
  mocks.scenarioList.mockResolvedValue(
    ok([scenario("a", "Rent hike"), scenario("b", "Pay rise")]),
  );
  // Both scenarios change the same bill; base has nothing, so the only contest is a vs b.
  mocks.forecastAssumptionList.mockImplementation((id: string | null) => {
    if (id === "a") return Promise.resolve(ok([event("a1", "a", 250_000)]));
    if (id === "b") return Promise.resolve(ok([event("b1", "b", 300_000)]));
    return Promise.resolve(ok([]));
  });
});

describe("ScenarioLayers (personal-cfo-88o4)", () => {
  it("puts the winning scenario at the top of the pile", async () => {
    // "a" is primary (weakest), "b" is stacked above it — so "b" wins and must be the TOP
    // sheet. Rendering the precedence array un-reversed would put "a" there and name the
    // wrong winner.
    renderWithClient(
      <ScenarioLayers primaryId="a" stacked={["b"]} onReorder={() => {}} />,
    );

    const pile = await screen.findByRole("list", { name: /strongest first/i });
    const sheets = within(pile).getAllByRole("listitem");
    // Pile order is top → bottom: Pay rise (winner), Rent hike, Your forecast.
    expect(sheets[0]).toHaveTextContent("Pay rise");
    expect(sheets[0]).toHaveTextContent(/wins ties/i);
    expect(sheets[sheets.length - 1]).toHaveTextContent("Your forecast");
  });

  it("strikes the losing change inside the sheet that lost it, naming the winner", async () => {
    renderWithClient(
      <ScenarioLayers primaryId="a" stacked={["b"]} onReorder={() => {}} />,
    );

    const overruled = await screen.findByText(/overruled by Pay rise/i);
    expect(overruled).toBeInTheDocument();
    // The mark belongs to the LOSER's own sheet, not the winner's.
    const sheet = overruled.closest("li");
    expect(sheet?.textContent).toContain("Rent hike");
  });

  it("names the contested item and the value the forecast uses", async () => {
    renderWithClient(
      <ScenarioLayers primaryId="a" stacked={["b"]} onReorder={() => {}} />,
    );

    expect(await screen.findByText(/where they collide/i)).toBeInTheDocument();
    // $3,000.00 wins over $2,500.00 — the later-stacked scenario.
    expect(screen.getByText(/the forecast uses \$3,000\.00/i)).toBeInTheDocument();
    expect(screen.getByText(/over \$2,500\.00/i)).toBeInTheDocument();
  });

  it("moves a sheet toward the top by making it LATER in precedence", async () => {
    // The arrows are in pile terms (up = stronger) while the array is in precedence terms
    // (later = stronger), so this asserts the flip is applied to the callback too — not
    // only to the render. An inverted callback would move the sheet the wrong way while
    // looking right until the next paint.
    const onReorder = vi.fn();
    renderWithClient(
      <ScenarioLayers primaryId="a" stacked={["b", "c"]} onReorder={onReorder} />,
    );

    fireEvent.click(await screen.findByLabelText(/move Pay rise up/i));
    expect(onReorder).toHaveBeenCalledWith(["c", "b"]);
  });

  it("renders nothing until every sheet's events have arrived", () => {
    // A partly-loaded stack would compute conflicts against events that have not arrived
    // and report a change as surviving that a still-loading sheet overrules.
    mocks.forecastAssumptionList.mockReturnValue(new Promise(() => {}));
    const { container } = renderWithClient(
      <ScenarioLayers primaryId="a" stacked={["b"]} onReorder={() => {}} />,
    );
    expect(container.textContent).not.toMatch(/top sheet wins/i);
  });

  it("renders nothing when there is no stack to explain", () => {
    const { container } = renderWithClient(
      <ScenarioLayers primaryId="a" stacked={[]} onReorder={() => {}} />,
    );
    expect(container.textContent).not.toMatch(/top sheet wins/i);
  });
});
