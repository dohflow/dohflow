import { QueryClient, onlineManager } from "@tanstack/react-query";
import { fireEvent, screen, waitFor } from "@testing-library/react";
import { useState } from "react";

import { renderWithClient } from "@/test/renderWithClient";

import type { ScenarioDto } from "@/bindings";
import { todayInTimezone } from "@/lib/format";
import { ScenarioBar } from "./ScenarioBar";

const mocks = vi.hoisted(() => ({
  householdTimezone: vi.fn(),
  setHouseholdTimezone: vi.fn(),
  scenarioList: vi.fn(),
  createScenario: vi.fn(),
  updateScenario: vi.fn(),
  archiveScenario: vi.fn(),
}));
vi.mock("@/bindings", () => ({ commands: mocks }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
// Two zones 26 hours apart, so their calendar dates always differ. A scenario that expires
// on one zone's "today" is expired for the other, whatever the machine running the test
// thinks the date is: that is how these tests tell the household date from the device's.
const AHEAD = "Pacific/Kiritimati";
const BEHIND = "Etc/GMT+12";

const scenario = (id: string, name: string, status = "draft"): ScenarioDto => ({
  id,
  name,
  description: null,
  applied_at: null,
  status,
  created_at: "2026-07-01T00:00:00Z",
  updated_at: "2026-07-01T00:00:00Z",
  expires_on: null,
  event_count: 0,
});

beforeEach(() => {
  vi.clearAllMocks();
  mocks.householdTimezone.mockResolvedValue(ok("UTC"));
  mocks.scenarioList.mockResolvedValue(ok([scenario("s1", "Raise", "draft")]));
  mocks.createScenario.mockResolvedValue(ok(scenario("s2", "Big move")));
  mocks.updateScenario.mockResolvedValue(ok(scenario("s1", "Raise + rent", "active")));
  mocks.archiveScenario.mockResolvedValue(ok(null));
});

test("creates a named scenario and selects it", async () => {
  const onSelect = vi.fn();
  renderWithClient(<ScenarioBar selectedId={null} onSelect={onSelect} />);

  fireEvent.click(screen.getByRole("button", { name: /new scenario/i }));
  fireEvent.change(screen.getByLabelText(/new scenario name/i), {
    target: { value: "Big move" },
  });
  fireEvent.click(screen.getByRole("button", { name: /^create$/i }));

  await waitFor(() =>
    expect(mocks.createScenario).toHaveBeenCalledWith({
      name: "Big move",
      description: null,
    }),
  );
  await waitFor(() => expect(onSelect).toHaveBeenCalledWith("s2"));
});

test("renames the selected scenario and changes its status", async () => {
  renderWithClient(<ScenarioBar selectedId="s1" onSelect={vi.fn()} />);

  fireEvent.click(await screen.findByRole("button", { name: /edit Raise/i }));
  fireEvent.change(screen.getByLabelText(/^name$/i), {
    target: { value: "Raise + rent" },
  });
  fireEvent.change(screen.getByLabelText(/^status$/i), { target: { value: "active" } });
  fireEvent.click(screen.getByRole("button", { name: /^save$/i }));

  await waitFor(() =>
    expect(mocks.updateScenario).toHaveBeenCalledWith({
      id: "s1",
      name: "Raise + rent",
      status: "active",
    }),
  );
});

test("archives the selected scenario and falls back to base", async () => {
  const onSelect = vi.fn();
  renderWithClient(<ScenarioBar selectedId="s1" onSelect={onSelect} />);

  fireEvent.click(await screen.findByRole("button", { name: /archive Raise/i }));

  await waitFor(() => expect(mocks.archiveScenario).toHaveBeenCalledWith("s1"));
  await waitFor(() => expect(onSelect).toHaveBeenCalledWith(null));
});

test("rejects an empty rename", async () => {
  renderWithClient(<ScenarioBar selectedId="s1" onSelect={vi.fn()} />);

  fireEvent.click(await screen.findByRole("button", { name: /edit Raise/i }));
  fireEvent.change(screen.getByLabelText(/^name$/i), { target: { value: "  " } });
  fireEvent.click(screen.getByRole("button", { name: /^save$/i }));

  expect(await screen.findByRole("alert")).toHaveTextContent(/name your scenario/i);
  expect(mocks.updateScenario).not.toHaveBeenCalled();
});

test("a scenario is expired by the household date, not the device's (g3m 5ie.18)", async () => {
  // Expires on the date it is in BEHIND right now: still live there, expired in AHEAD.
  const expiresOn = todayInTimezone(BEHIND);
  mocks.scenarioList.mockResolvedValue(
    ok([{ ...scenario("s1", "Raise"), expires_on: expiresOn }, scenario("s2", "Move")]),
  );
  mocks.householdTimezone.mockResolvedValue(ok(AHEAD));
  const { unmount } = renderWithClient(<ScenarioBar selectedId={null} onSelect={vi.fn()} />);
  await screen.findByRole("option", { name: "Move" });
  await waitFor(() => expect(screen.queryByRole("option", { name: "Raise" })).toBeNull());
  unmount();

  mocks.householdTimezone.mockResolvedValue(ok(BEHIND));
  renderWithClient(<ScenarioBar selectedId={null} onSelect={vi.fn()} />);
  expect(await screen.findByRole("option", { name: "Raise" })).toBeInTheDocument();
});

// 2026-09-02 00:30 UTC is still September 1 in Los Angeles, so a scenario that expires on
// September 1 is live there and expired in UTC (5ie.18, AC8).
describe("the picker waits for the household date (5ie.18, AC8)", () => {
  const LA = "America/Los_Angeles";
  const expiring = () => ({ ...scenario("s1", "Raise"), expires_on: "2026-09-01" });
  const client = () => new QueryClient({ defaultOptions: { queries: { retry: false } } });

  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-09-02T00:30:00Z"));
    mocks.scenarioList.mockResolvedValue(ok([expiring(), scenario("s2", "Move")]));
  });
  afterEach(() => {
    vi.useRealTimers();
    onlineManager.setOnline(true);
  });

  test("scenarios that arrive before the timezone are not judged on the UTC fallback", async () => {
    let resolveTz: (value: unknown) => void = () => {};
    mocks.householdTimezone.mockReturnValue(new Promise((resolve) => (resolveTz = resolve)));
    const onSelect = vi.fn();
    renderWithClient(<ScenarioBar selectedId="s1" onSelect={onSelect} />, client());
    // The scenarios are in (the selected one is named), the timezone is not.
    await waitFor(() => expect(screen.getByLabelText("Scenario")).toHaveValue("s1"));
    expect(screen.getByLabelText("Scenario")).toBeDisabled();
    expect(screen.getByRole("status")).toHaveTextContent("Loading…");
    expect(onSelect).not.toHaveBeenCalled();

    resolveTz(ok(LA));
    expect(await screen.findByRole("option", { name: "Move" })).toBeInTheDocument();
    expect(screen.getByLabelText("Scenario")).toBeEnabled();
    expect(screen.getByLabelText("Scenario")).toHaveValue("s1"); // still live in Los Angeles
    expect(onSelect).not.toHaveBeenCalled();
  });

  test("a read that is paused (offline, or a key rotation) leaves the selection alone", async () => {
    onlineManager.setOnline(false);
    mocks.householdTimezone.mockResolvedValue(ok(LA));
    const onSelect = vi.fn();
    renderWithClient(<ScenarioBar selectedId="s1" onSelect={onSelect} />, client());
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Loading…"));
    expect(mocks.householdTimezone).not.toHaveBeenCalled();
    expect(onSelect).not.toHaveBeenCalled();

    onlineManager.setOnline(true);
    expect(await screen.findByRole("option", { name: "Move" })).toBeInTheDocument();
    expect(onSelect).not.toHaveBeenCalled();
  });

  test("a failed read says so, keeps the selection, and Retry reads it again without writing", async () => {
    mocks.householdTimezone.mockRejectedValueOnce(new Error("host down"));
    mocks.householdTimezone.mockResolvedValue(ok(LA));
    const onSelect = vi.fn();
    renderWithClient(<ScenarioBar selectedId="s1" onSelect={onSelect} />, client());
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not load the household timezone.");
    expect(screen.queryByRole("option", { name: "Move" })).toBeNull();
    expect(onSelect).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByRole("option", { name: "Move" })).toBeInTheDocument();
    expect(mocks.householdTimezone).toHaveBeenCalledTimes(2);
    expect(mocks.setHouseholdTimezone).not.toHaveBeenCalled();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(onSelect).not.toHaveBeenCalled();
  });

  test("Retry reads Retrying… and is disabled while the read runs", async () => {
    mocks.householdTimezone.mockRejectedValueOnce(new Error("host down"));
    let finish: (value: unknown) => void = () => {};
    mocks.householdTimezone.mockReturnValueOnce(new Promise((resolve) => (finish = resolve)));
    renderWithClient(<ScenarioBar selectedId={null} onSelect={vi.fn()} />, client());
    await screen.findByRole("alert");
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    const pending = await screen.findByRole("button", { name: "Retrying…" });
    expect(pending).toBeDisabled();
    // The failure being retried stays on screen meanwhile.
    expect(screen.getByRole("alert")).toHaveTextContent("Could not load the household timezone.");
    finish(ok(LA));
    await waitFor(() => expect(screen.queryByRole("button", { name: /retry/i })).toBeNull());
  });

  test("a cached timezone whose refresh failed is not a fresh read: the failure shows and nothing is judged", async () => {
    const c = client();
    mocks.householdTimezone.mockResolvedValue(ok(LA));
    const onSelect = vi.fn();
    renderWithClient(<ScenarioBar selectedId="s1" onSelect={onSelect} />, c);
    await screen.findByRole("option", { name: "Move" });

    mocks.householdTimezone.mockRejectedValue(new Error("host down"));
    await c.refetchQueries({ queryKey: ["household-timezone"] });
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not load the household timezone.");
    expect(screen.getByLabelText("Scenario")).toBeDisabled();
    expect(onSelect).not.toHaveBeenCalled();
  });

  test("nothing falls back to base for any reason before the timezone is read", async () => {
    // The selected scenario is not in the list at all; that is judged once the read is in.
    mocks.householdTimezone.mockReturnValue(new Promise(() => {}));
    mocks.scenarioList.mockResolvedValue(ok([scenario("s2", "Move")]));
    const onSelect = vi.fn();
    renderWithClient(<ScenarioBar selectedId="s1" onSelect={onSelect} />, client());
    await waitFor(() => expect(mocks.scenarioList).toHaveBeenCalled());
    await screen.findByRole("status");
    expect(onSelect).not.toHaveBeenCalled();
  });

  test("a timezone that was read as UTC is a real answer: the scenario that expired falls back to base", async () => {
    mocks.householdTimezone.mockResolvedValue(ok("UTC"));
    const onSelect = vi.fn();
    renderWithClient(<ScenarioBar selectedId="s1" onSelect={onSelect} />, client());
    await waitFor(() => expect(onSelect).toHaveBeenCalledWith(null));
  });

  test("after a good read, a scenario that was deleted elsewhere still falls back to base", async () => {
    mocks.householdTimezone.mockResolvedValue(ok(LA));
    mocks.scenarioList.mockResolvedValue(ok([scenario("s2", "Move")]));
    const onSelect = vi.fn();
    renderWithClient(<ScenarioBar selectedId="s1" onSelect={onSelect} />, client());
    await waitFor(() => expect(onSelect).toHaveBeenCalledWith(null));
  });
});

// While the timezone is unverified (or the list has not come) the select holds the selection
// instead of reading "Base (no scenario)" (5ie.18, Studio round 3).
describe("the picker never shows Base for a selection it cannot judge yet (5ie.18)", () => {
  const client = () => new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const shown = () => {
    const select = screen.getByLabelText("Scenario") as HTMLSelectElement;
    return { value: select.value, text: select.selectedOptions[0]?.textContent };
  };

  test("a list that has not come yet shows Loading… for the selection", async () => {
    mocks.householdTimezone.mockReturnValue(new Promise(() => {}));
    mocks.scenarioList.mockReturnValue(new Promise(() => {}));
    renderWithClient(<ScenarioBar selectedId="s1" onSelect={vi.fn()} />, client());
    await screen.findByRole("status");
    expect(shown()).toEqual({ value: "s1", text: "Loading…" });
  });

  test("a row missing from the list shows Loading… while the timezone is unread", async () => {
    mocks.householdTimezone.mockReturnValue(new Promise(() => {}));
    mocks.scenarioList.mockResolvedValue(ok([scenario("s2", "Move")]));
    const onSelect = vi.fn();
    renderWithClient(<ScenarioBar selectedId="s1" onSelect={onSelect} />, client());
    await waitFor(() => expect(mocks.scenarioList).toHaveBeenCalled());
    await screen.findByRole("status");
    expect(shown()).toEqual({ value: "s1", text: "Loading…" });
    expect(onSelect).not.toHaveBeenCalled();
  });

  test("an archived selected row shows its name while the timezone is unread", async () => {
    mocks.householdTimezone.mockReturnValue(new Promise(() => {}));
    mocks.scenarioList.mockResolvedValue(ok([scenario("s1", "Old plan", "archived")]));
    const onSelect = vi.fn();
    renderWithClient(<ScenarioBar selectedId="s1" onSelect={onSelect} />, client());
    await waitFor(() =>
      expect(shown()).toEqual({ value: "s1", text: "Old plan" }),
    );
    expect(onSelect).not.toHaveBeenCalled();
  });

  test("a failed list shows its own error for the selection", async () => {
    mocks.householdTimezone.mockReturnValue(new Promise(() => {}));
    mocks.scenarioList.mockRejectedValue(new Error("boom"));
    renderWithClient(<ScenarioBar selectedId="s1" onSelect={vi.fn()} />, client());
    await waitFor(() =>
      expect(shown()).toEqual({ value: "s1", text: "Could not load your scenarios." }),
    );
  });

  test("Base shows as selected for no selection, with or without the timezone", async () => {
    mocks.householdTimezone.mockReturnValue(new Promise(() => {}));
    renderWithClient(<ScenarioBar selectedId={null} onSelect={vi.fn()} />, client());
    await screen.findByRole("status");
    expect(shown()).toEqual({ value: "", text: "Base (no scenario)" });
    const select = screen.getByLabelText("Scenario") as HTMLSelectElement;
    expect([...select.options].map((o) => o.textContent)).toEqual(["Base (no scenario)"]);
  });

  test("a read timezone with a list still on its way holds the selection as Loading…", async () => {
    mocks.householdTimezone.mockResolvedValue(ok("UTC"));
    mocks.scenarioList.mockReturnValue(new Promise(() => {}));
    const onSelect = vi.fn();
    renderWithClient(<ScenarioBar selectedId="s1" onSelect={onSelect} />, client());
    await waitFor(() => expect(screen.getByLabelText("Scenario")).toBeEnabled());
    expect(shown()).toEqual({ value: "s1", text: "Loading…" });
    expect(onSelect).not.toHaveBeenCalled();
  });

  test("once the reads are in, a live scenario is kept and its name shows", async () => {
    let resolveTz: (value: unknown) => void = () => {};
    mocks.householdTimezone.mockReturnValue(new Promise((resolve) => (resolveTz = resolve)));
    const onSelect = vi.fn();
    renderWithClient(<ScenarioBar selectedId="s1" onSelect={onSelect} />, client());
    await waitFor(() => expect(shown()).toEqual({ value: "s1", text: "Raise" }));
    resolveTz(ok("UTC"));
    await screen.findByRole("option", { name: "Raise" });
    await waitFor(() => expect(screen.getByLabelText("Scenario")).toBeEnabled());
    expect(shown()).toEqual({ value: "s1", text: "Raise" });
    expect(onSelect).not.toHaveBeenCalled();
  });

  test("once the reads are in, a row that is really gone prunes to Base", async () => {
    mocks.householdTimezone.mockResolvedValue(ok("UTC"));
    mocks.scenarioList.mockResolvedValue(ok([scenario("s2", "Move")]));
    function Parent() {
      const [id, setId] = useState<string | null>("s1");
      return <ScenarioBar selectedId={id} onSelect={setId} />;
    }
    renderWithClient(<Parent />, client());
    await waitFor(() =>
      expect(shown()).toEqual({ value: "", text: "Base (no scenario)" }),
    );
  });
});
