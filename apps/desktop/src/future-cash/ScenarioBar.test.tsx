import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { ScenarioDto } from "@/bindings";
import { ScenarioBar } from "./ScenarioBar";

const mocks = vi.hoisted(() => ({
  scenarioList: vi.fn(),
  createScenario: vi.fn(),
  updateScenario: vi.fn(),
  archiveScenario: vi.fn(),
}));
vi.mock("@/bindings", () => ({ commands: mocks }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
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
