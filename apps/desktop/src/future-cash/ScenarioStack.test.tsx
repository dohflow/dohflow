import { fireEvent, screen } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import { ScenarioStack } from "./ScenarioStack";

const mocks = vi.hoisted(() => ({ scenarioList: vi.fn() }));
vi.mock("@/bindings", () => ({ commands: mocks }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

const scenario = (id: string, name: string, status = "draft") => ({
  id,
  name,
  description: null,
  status,
  created_at: "2026-08-01",
  updated_at: "2026-08-01",
  expires_on: null,
  event_count: 1,
  applied_at: null,
});

beforeEach(() => {
  vi.clearAllMocks();
  mocks.scenarioList.mockResolvedValue(
    ok([
      scenario("a", "Rent hike"),
      scenario("b", "Pay rise"),
      scenario("old", "Last year", "archived"),
    ]),
  );
});

describe("ScenarioStack (personal-cfo-4d8.27.6.4, ADR 0059)", () => {
  it("stacks a scenario on top of the selected one", async () => {
    const onChange = vi.fn();
    renderWithClient(
      <ScenarioStack primaryId="a" stacked={[]} onChange={onChange} />,
    );
    const picker = await screen.findByLabelText("Add a scenario to the stack");
    fireEvent.change(picker, { target: { value: "b" } });
    expect(onChange).toHaveBeenCalledWith(["b"]);
  });

  it("shows the stack in precedence order", async () => {
    // Order is the precedence (ADR 0059 §1), so it has to be VISIBLE — a control that hid
    // it would leave the user unable to predict which change is in effect. The primary is
    // position 1, so a stacked entry starts at 2.
    renderWithClient(
      <ScenarioStack primaryId="a" stacked={["b"]} onChange={vi.fn()} />,
    );
    // Wait on the RESOLVED name: the chip renders before the scenario list loads, so a
    // findByText("2.") would pass while the label still reads "a deleted scenario".
    expect(
      await screen.findByRole("button", { name: /Remove Pay rise from the stack/i }),
    ).toBeInTheDocument();
    // The position number is the point: it says which change wins without opening anything.
    expect(screen.getByText("2.")).toBeInTheDocument();
    expect(
      screen.getByText(/the later one is what the forecast uses/i),
    ).toBeInTheDocument();
  });

  it("does not offer the selected scenario, an already-stacked one, or an archived one", async () => {
    // Offering an archived scenario would promise a change the backend drops (ADR 0051).
    mocks.scenarioList.mockResolvedValue(
      ok([
        scenario("a", "Rent hike"),
        scenario("b", "Pay rise"),
        scenario("c", "New job"),
        scenario("old", "Last year", "archived"),
      ]),
    );
    renderWithClient(
      <ScenarioStack primaryId="a" stacked={["b"]} onChange={vi.fn()} />,
    );
    const picker = await screen.findByLabelText("Add a scenario to the stack");
    const options = Array.from(picker.querySelectorAll("option")).map(
      (o) => o.textContent,
    );
    // Only the one remaining live scenario — not the primary, not the stacked one, not
    // the archived one.
    expect(options).toEqual(["Add a scenario…", "New job"]);
  });

  it("hides the picker once nothing selectable is left", async () => {
    // A control offering no options is furniture; the stack itself stays visible.
    renderWithClient(
      <ScenarioStack primaryId="a" stacked={["b"]} onChange={vi.fn()} />,
    );
    expect(await screen.findByText("2.")).toBeInTheDocument();
    expect(
      screen.queryByLabelText("Add a scenario to the stack"),
    ).not.toBeInTheDocument();
  });

  it("removes one from the stack without touching the rest", async () => {
    const onChange = vi.fn();
    mocks.scenarioList.mockResolvedValue(
      ok([scenario("a", "Rent hike"), scenario("b", "Pay rise"), scenario("c", "New job")]),
    );
    renderWithClient(
      <ScenarioStack primaryId="a" stacked={["b", "c"]} onChange={onChange} />,
    );
    fireEvent.click(
      await screen.findByRole("button", { name: /Remove Pay rise from the stack/i }),
    );
    expect(onChange).toHaveBeenCalledWith(["c"]);
  });

  it("renders nothing when there is nothing to stack", async () => {
    mocks.scenarioList.mockResolvedValue(ok([scenario("a", "Rent hike")]));
    const { container } = renderWithClient(
      <ScenarioStack primaryId="a" stacked={[]} onChange={vi.fn()} />,
    );
    await vi.waitFor(() => expect(mocks.scenarioList).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });
});
