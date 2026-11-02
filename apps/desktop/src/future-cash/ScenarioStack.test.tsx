import { act, fireEvent, screen, waitFor } from "@testing-library/react";

import { todayInTimezone } from "@/lib/format";
import { renderWithClient } from "@/test/renderWithClient";

import { ScenarioStack } from "./ScenarioStack";

const mocks = vi.hoisted(() => ({ scenarioList: vi.fn(), householdTimezone: vi.fn() }));
vi.mock("@/bindings", () => ({ commands: mocks }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
// Two zones 26 hours apart, so their calendar dates always differ. A scenario that expires
// on one zone's "today" is expired for the other, whatever the machine running the test
// thinks the date is: that is how these tests tell the household date from the device's.
const AHEAD = "Pacific/Kiritimati";
const BEHIND = "Etc/GMT+12";


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
  mocks.householdTimezone.mockResolvedValue(ok("UTC"));
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

describe("which scenarios can be stacked (5ie.18)", () => {
  it("leaves out an expired one, by the household date", async () => {
    // Expires on the date it is in BEHIND right now: live there, expired in AHEAD.
    const expiresOn = todayInTimezone(BEHIND);
    mocks.scenarioList.mockResolvedValue(
      ok([
        scenario("a", "Rent hike"),
        { ...scenario("late", "Lapsed plan"), expires_on: expiresOn },
        { ...scenario("long", "Long gone"), expires_on: "2001-01-01" },
        scenario("b", "Pay rise"),
      ]),
    );
    mocks.householdTimezone.mockResolvedValue(ok(AHEAD));
    const { unmount } = renderWithClient(
      <ScenarioStack primaryId="a" stacked={[]} onChange={vi.fn()} />,
    );
    await screen.findByRole("option", { name: "Pay rise" });
    // The household zone arrives after the first paint, so wait for it to apply.
    await waitFor(() => expect(screen.queryByRole("option", { name: "Lapsed plan" })).toBeNull());
    expect(screen.queryByRole("option", { name: "Long gone" })).toBeNull();
    unmount();

    mocks.householdTimezone.mockResolvedValue(ok(BEHIND));
    renderWithClient(<ScenarioStack primaryId="a" stacked={[]} onChange={vi.fn()} />);
    expect(await screen.findByRole("option", { name: "Lapsed plan" })).toBeInTheDocument();
    expect(screen.queryByRole("option", { name: "Long gone" })).toBeNull();
  });
});

describe("the stack waits for the household date (5ie.18, AC8)", () => {
  it("offers and lists nothing until the timezone has been read", async () => {
    let resolveTz: (value: unknown) => void = () => {};
    mocks.householdTimezone.mockReturnValue(new Promise((resolve) => (resolveTz = resolve)));
    renderWithClient(<ScenarioStack primaryId="a" stacked={["b"]} onChange={vi.fn()} />);
    await waitFor(() => expect(mocks.scenarioList).toHaveBeenCalled());
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
    expect(screen.queryByLabelText("Add a scenario to the stack")).toBeNull();
    expect(screen.queryByText("Pay rise")).toBeNull();
    resolveTz(ok("UTC"));
    expect(await screen.findByText("Pay rise")).toBeInTheDocument();
  });
});

