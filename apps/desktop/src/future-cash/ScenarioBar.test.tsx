import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { ScenarioDto } from "@/bindings";
import { todayInTimezone } from "@/lib/format";
import { ScenarioBar } from "./ScenarioBar";

const mocks = vi.hoisted(() => ({
  householdTimezone: vi.fn(),
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
