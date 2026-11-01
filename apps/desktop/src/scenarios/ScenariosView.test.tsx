import { fireEvent, screen, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { RevertPreviewDto, ScenarioDto } from "@/bindings";
import { ScenariosView } from "./ScenariosView";

const mocks = vi.hoisted(() => ({
  scenarioList: vi.fn(),
  createScenario: vi.fn(),
  updateScenario: vi.fn(),
  deleteScenario: vi.fn(),
  archiveScenario: vi.fn(),
  cloneScenario: vi.fn(),
  setScenarioExpiry: vi.fn(),
  applyScenario: vi.fn(),
  revertScenarioApply: vi.fn(),
  scenarioRevertPreview: vi.fn(),
  scenarioDeletedAppliedList: vi.fn(),
  recurringBillList: vi.fn(),
  incomeSourceList: vi.fn(),
  categoryList: vi.fn(),
  baseCurrency: vi.fn(),
  householdTimezone: vi.fn(),
}));
vi.mock("@/bindings", () => ({ commands: mocks }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

function scenario(over: Partial<ScenarioDto> = {}): ScenarioDto {
  return {
    id: "sce-1",
    name: "Maternity leave",
    description: null,
    applied_at: null,
    status: "draft",
    created_at: "2026-07-01T00:00:00Z",
    updated_at: "2026-07-01T00:00:00Z",
    expires_on: null,
    event_count: 3,
    ...over,
  };
}

function preview(over: Partial<RevertPreviewDto> = {}): RevertPreviewDto {
  return { lines: [], blocker_name: null, confirmed_descendants: [], ...over };
}

beforeEach(() => {
  for (const fn of Object.values(mocks)) fn.mockReset();
  mocks.scenarioList.mockResolvedValue(ok([scenario()]));
  mocks.archiveScenario.mockResolvedValue(ok(null));
  mocks.deleteScenario.mockResolvedValue(ok(null));
  mocks.cloneScenario.mockResolvedValue(ok("sce-2"));
  mocks.setScenarioExpiry.mockResolvedValue(ok(null));
  mocks.updateScenario.mockResolvedValue(ok(null));
  mocks.scenarioDeletedAppliedList.mockResolvedValue(ok([]));
  mocks.scenarioRevertPreview.mockResolvedValue(ok(preview()));
  mocks.recurringBillList.mockResolvedValue(ok([]));
  mocks.incomeSourceList.mockResolvedValue(ok([]));
  mocks.categoryList.mockResolvedValue(ok([]));
  mocks.baseCurrency.mockResolvedValue(ok("USD"));
  // The backend default (personal-cfo-q329); no test here needs a specific household
  // zone — the two boundary-testing expires_on fixtures (2020-01-01, 2026-12-31) are far
  // enough from "now" that any real-world date/zone resolves the same expired/not-expired
  // verdict.
  mocks.householdTimezone.mockResolvedValue(ok("UTC"));
});

describe("ScenariosView (personal-cfo-4d8.27.6.1 / .6.6)", () => {
  it("lists scenarios with their state and change count", async () => {
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
    expect(await screen.findByText("Maternity leave")).toBeInTheDocument();
    expect(screen.getByText("Draft")).toBeInTheDocument();
    expect(screen.getByText("3")).toBeInTheDocument();
  });

  it("archives without confirmation, because archiving keeps everything (ADR 0051 §1)", async () => {
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: /archive maternity leave/i }));
    await vi.waitFor(() => expect(mocks.archiveScenario).toHaveBeenCalledWith("sce-1"));
    // No dialog: it is reversible, so it does not interrupt.
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(mocks.deleteScenario).not.toHaveBeenCalled();
  });

  it("confirms before deleting, and says what is lost", async () => {
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: /delete maternity leave/i }));
    const dialog = await screen.findByRole("dialog", { name: /delete scenario/i });
    // The count makes "archive instead" an informed choice.
    expect(within(dialog).getByText(/3 changes/)).toBeInTheDocument();
    // The rows stay in the vault under a tombstone, so the copy must not claim permanence.
    expect(within(dialog).queryByText(/permanent/i)).not.toBeInTheDocument();
    expect(mocks.deleteScenario).not.toHaveBeenCalled();

    fireEvent.click(within(dialog).getByRole("button", { name: /^delete scenario$/i }));
    await vi.waitFor(() => expect(mocks.deleteScenario).toHaveBeenCalledWith("sce-1"));
  });

  it("cancelling the delete dialog deletes nothing", async () => {
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: /delete maternity leave/i }));
    const dialog = await screen.findByRole("dialog", { name: /delete scenario/i });
    fireEvent.click(within(dialog).getByRole("button", { name: /cancel/i }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(mocks.deleteScenario).not.toHaveBeenCalled();
  });

  it("clones into a copy", async () => {
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: /duplicate maternity leave/i }));
    await vi.waitFor(() =>
      expect(mocks.cloneScenario).toHaveBeenCalledWith({
        id: "sce-1",
        name: "Maternity leave (copy)",
      }),
    );
  });

  it("clones a scenario whose name already fills the limit, cutting the base so the suffix fits", async () => {
    const full = "x".repeat(100);
    mocks.scenarioList.mockResolvedValue(ok([scenario({ name: full })]));
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: /duplicate x+/i }));
    await vi.waitFor(() => expect(mocks.cloneScenario).toHaveBeenCalled());
    const [input] = mocks.cloneScenario.mock.calls[0] as [{ name: string }];
    const { name } = input;
    expect(name).toBe(`${"x".repeat(93)} (copy)`);
    expect(Array.from(name)).toHaveLength(100);
  });

  it("sets an expiry date on blur, not on every keystroke", async () => {
    // A segmented date input fires change for each intermediate COMPLETE value, so
    // typing a year emits 0002-, 0020-, 0202- … — each of which would persist a past
    // date and silently stop the scenario applying. Committing on blur avoids that.
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
    const expiry = await screen.findByLabelText(/expiry for maternity leave/i);
    fireEvent.change(expiry, { target: { value: "0002-12-31" } });
    expect(mocks.setScenarioExpiry).not.toHaveBeenCalled();
    fireEvent.change(expiry, { target: { value: "2026-12-31" } });
    fireEvent.blur(expiry);
    await vi.waitFor(() =>
      expect(mocks.setScenarioExpiry).toHaveBeenCalledWith({
        id: "sce-1",
        expires_on: "2026-12-31",
      }),
    );
  });

  it("clearing the date sends null, so the scenario stops expiring", async () => {
    // Starts WITH an expiry — emptying the field must send null, not "".
    mocks.scenarioList.mockResolvedValue(ok([scenario({ expires_on: "2026-12-31" })]));
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
    const expiry = await screen.findByLabelText(/expiry for maternity leave/i);
    expect(expiry).toHaveValue("2026-12-31");
    fireEvent.change(expiry, { target: { value: "" } });
    fireEvent.blur(expiry);
    await vi.waitFor(() =>
      expect(mocks.setScenarioExpiry).toHaveBeenCalledWith({
        id: "sce-1",
        expires_on: null,
      }),
    );
  });

  it("shows a past expiry as Expired without any stored status change", async () => {
    // ADR 0051 §3: expiry is derived at read time — the row is still a draft.
    mocks.scenarioList.mockResolvedValue(
      ok([scenario({ expires_on: "2020-01-01", status: "draft" })]),
    );
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
    expect(await screen.findByText("Expired")).toBeInTheDocument();
    expect(screen.queryByText("Draft")).not.toBeInTheDocument();
  });

  it("hides archived scenarios behind a toggle, and restores them", async () => {
    mocks.scenarioList.mockResolvedValue(
      ok([scenario(), scenario({ id: "sce-9", name: "Old plan", status: "archived" })]),
    );
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
    await screen.findByText("Maternity leave");
    expect(screen.queryByText("Old plan")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /show 1 archived/i }));
    expect(screen.getByText("Old plan")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /restore old plan/i }));
    await vi.waitFor(() =>
      expect(mocks.updateScenario).toHaveBeenCalledWith({
        id: "sce-9",
        status: "draft",
        name: null,
      }),
    );
  });

  it("opens a scenario on the Cash Flow screen", async () => {
    const onOpen = vi.fn();
    renderWithClient(<ScenariosView onOpenInCashFlow={onOpen} />);
    fireEvent.click(await screen.findByRole("button", { name: "Maternity leave" }));
    expect(onOpen).toHaveBeenCalledWith("sce-1");
  });

  it("creates a scenario", async () => {
    mocks.createScenario.mockResolvedValue(ok(scenario({ id: "sce-new" })));
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: /new scenario/i }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Raise" } });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await vi.waitFor(() =>
      expect(mocks.createScenario).toHaveBeenCalledWith({
        name: "Raise",
        description: null,
      }),
    );
  });
});

describe("applying a scenario (ADR 0055, personal-cfo-4d8.27.6.3)", () => {
  it("confirms first, and says what does and does not change", async () => {
    // "Apply" could reasonably be read as "edit my bills". It is not — it promotes the
    // scenario's assumption events into base. The dialog has to say so, because the user
    // cannot see the difference from the button.
    mocks.scenarioList.mockResolvedValue(ok([scenario({ event_count: 3 })]));
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);

    fireEvent.click(
      await screen.findByRole("button", { name: /apply .* to the forecast/i }),
    );
    const dialog = await screen.findByRole("dialog", { name: "Apply scenario" });
    expect(dialog).toHaveTextContent(/3 changes/);
    expect(dialog).toHaveTextContent(/bills, income and transactions are not edited/i);
    // Nothing has happened yet — confirming is what applies.
    expect(mocks.applyScenario).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: /apply to my forecast/i }));
    await vi.waitFor(() => expect(mocks.applyScenario).toHaveBeenCalledWith("sce-1"));
  });

  it("offers undo instead of apply once applied, and marks the row", async () => {
    mocks.scenarioList.mockResolvedValue(
      ok([scenario({ applied_at: "2026-08-02T10:00:00Z" })]),
    );
    mocks.revertScenarioApply.mockResolvedValue(ok(null));
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);

    expect(await screen.findByText("Applied")).toBeInTheDocument();
    // Applied-ness is orthogonal to the lifecycle (ADR 0055 §4), so the draft badge stays.
    expect(screen.getByText("Draft")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /apply .* to the forecast/i }),
    ).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /undo applying/i }));
    // Nothing was changed after applying, so undoing needs no confirmation.
    await vi.waitFor(() =>
      expect(mocks.revertScenarioApply).toHaveBeenCalledWith("sce-1", []),
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  describe("undoing an apply that was changed afterwards (personal-cfo-5ie.12)", () => {
    const applied = () =>
      mocks.scenarioList.mockResolvedValue(
        ok([scenario({ applied_at: "2026-08-02T10:00:00Z" })]),
      );
    const rate = (bps: number) => `{"annual_bps":${bps}}`;

    it("lists each change it will undo, and sends back the ids it showed", async () => {
      applied();
      mocks.revertScenarioApply.mockResolvedValue(ok(null));
      mocks.scenarioRevertPreview.mockResolvedValue(
        ok(
          preview({
            lines: [
              {
                change: "cleared",
                kind: "inflation_rate",
                target_entity_id: null,
                before_params_json: rate(400),
                after_params_json: null,
              },
            ],
            confirmed_descendants: ["evt-2"],
          }),
        ),
      );
      renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);

      fireEvent.click(await screen.findByRole("button", { name: /undo applying/i }));
      const dialog = await screen.findByRole("dialog", { name: "Undo scenario" });
      expect(dialog).toHaveTextContent(/you removed/i);
      // Nothing is written until the person confirms.
      expect(mocks.revertScenarioApply).not.toHaveBeenCalled();

      fireEvent.click(within(dialog).getByRole("button", { name: "Undo scenario" }));
      await vi.waitFor(() =>
        expect(mocks.revertScenarioApply).toHaveBeenCalledWith("sce-1", ["evt-2"]),
      );
    });

    it("writes nothing when the person cancels", async () => {
      applied();
      mocks.scenarioRevertPreview.mockResolvedValue(
        ok(
          preview({
            lines: [
              {
                change: "edited",
                kind: "inflation_rate",
                target_entity_id: null,
                before_params_json: rate(400),
                after_params_json: rate(450),
              },
            ],
            confirmed_descendants: ["evt-2"],
          }),
        ),
      );
      renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
      fireEvent.click(await screen.findByRole("button", { name: /undo applying/i }));
      const dialog = await screen.findByRole("dialog", { name: "Undo scenario" });
      expect(dialog).toHaveTextContent(/you changed/i);
      fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
      expect(mocks.revertScenarioApply).not.toHaveBeenCalled();
    });

    it("shows the fresh list to confirm again when the changes moved, and writes nothing", async () => {
      applied();
      const line = (after: string) => ({
        change: "edited" as const,
        kind: "inflation_rate",
        target_entity_id: null,
        before_params_json: rate(400),
        after_params_json: after,
      });
      mocks.scenarioRevertPreview
        .mockResolvedValueOnce(
          ok(preview({ lines: [line(rate(450))], confirmed_descendants: ["evt-2"] })),
        )
        .mockResolvedValueOnce(
          ok(preview({ lines: [line(rate(500))], confirmed_descendants: ["evt-3"] })),
        );
      // The backend refuses the stale list.
      mocks.revertScenarioApply.mockResolvedValue({
        status: "error",
        error: { Validation: "the changes are not the ones confirmed" },
      });
      renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);

      fireEvent.click(await screen.findByRole("button", { name: /undo applying/i }));
      const first = await screen.findByRole("dialog", { name: "Undo scenario" });
      fireEvent.click(within(first).getByRole("button", { name: "Undo scenario" }));

      await vi.waitFor(() =>
        expect(mocks.revertScenarioApply).toHaveBeenCalledWith("sce-1", ["evt-2"]),
      );
      const again = await screen.findByRole("dialog", { name: "Undo scenario" });
      expect(again).toHaveTextContent(/changed while you were looking/i);
      // The fresh list is what the next confirm sends.
      mocks.revertScenarioApply.mockResolvedValue(ok(null));
      fireEvent.click(within(again).getByRole("button", { name: "Undo scenario" }));
      await vi.waitFor(() =>
        expect(mocks.revertScenarioApply).toHaveBeenLastCalledWith("sce-1", ["evt-3"]),
      );
    });

    it("explains a later apply that appeared meanwhile instead of a generic error", async () => {
      applied();
      const line = {
        change: "edited" as const,
        kind: "inflation_rate",
        target_entity_id: null,
        before_params_json: rate(400),
        after_params_json: rate(450),
      };
      mocks.scenarioRevertPreview
        .mockResolvedValueOnce(
          ok(preview({ lines: [line], confirmed_descendants: ["evt-2"] })),
        )
        // The same changes, but another scenario was applied on top in the meantime.
        .mockResolvedValueOnce(
          ok(
            preview({
              lines: [line],
              confirmed_descendants: ["evt-2"],
              blocker_name: "Second plan",
            }),
          ),
        );
      mocks.revertScenarioApply.mockResolvedValue({
        status: "error",
        error: { Validation: "a scenario applied later has to be undone first" },
      });
      renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);

      fireEvent.click(await screen.findByRole("button", { name: /undo applying/i }));
      const first = await screen.findByRole("dialog", { name: "Undo scenario" });
      fireEvent.click(within(first).getByRole("button", { name: "Undo scenario" }));

      const blocked = await screen.findByRole("dialog", {
        name: "Undo another scenario first",
      });
      expect(blocked).toHaveTextContent(/Second plan/);
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    });

    it("says a later scenario has to be undone first, and offers no undo", async () => {
      applied();
      mocks.scenarioRevertPreview.mockResolvedValue(
        ok(preview({ blocker_name: "Second plan" })),
      );
      renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
      fireEvent.click(await screen.findByRole("button", { name: /undo applying/i }));
      const dialog = await screen.findByRole("dialog", {
        name: "Undo another scenario first",
      });
      expect(dialog).toHaveTextContent(/Second plan/);
      expect(
        within(dialog).queryByRole("button", { name: "Undo scenario" }),
      ).not.toBeInTheDocument();
      expect(mocks.revertScenarioApply).not.toHaveBeenCalled();
    });

    it("keeps a deleted but applied scenario reachable, and undoes it from there", async () => {
      mocks.scenarioList.mockResolvedValue(ok([]));
      mocks.scenarioList.mockResolvedValue(ok([scenario({ id: "sce-9", name: "Other" })]));
      mocks.scenarioDeletedAppliedList.mockResolvedValue(
        ok([
          scenario({
            id: "sce-gone",
            name: "Old plan",
            applied_at: "2026-08-02T10:00:00Z",
          }),
        ]),
      );
      mocks.revertScenarioApply.mockResolvedValue(ok(null));
      renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);

      expect(await screen.findByText("Deleted, but still applied")).toBeInTheDocument();
      fireEvent.click(
        screen.getByRole("button", { name: /undo applying deleted scenario old plan/i }),
      );
      await vi.waitFor(() =>
        expect(mocks.revertScenarioApply).toHaveBeenCalledWith("sce-gone", []),
      );
    });
  });

  it("does not offer apply for a scenario with no changes", async () => {
    // Applying nothing would mark it applied and offer an undo that undoes nothing.
    mocks.scenarioList.mockResolvedValue(ok([scenario({ event_count: 0 })]));
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
    expect(
      await screen.findByRole("button", { name: /apply .* to the forecast/i }),
    ).toBeDisabled();
  });
});

describe("apply respects the scenario lifecycle", () => {
  it("does not offer apply for an archived or expired scenario", async () => {
    // Archiving means "stop offering this" (ADR 0051 §1) and expiry means "this has
    // passed" (§3). Offering Apply on either would contradict the state the row shows —
    // and the backend gate that keeps an archived scenario out of a forecast run does
    // not cover apply, which promotes to BASE.
    mocks.scenarioList.mockResolvedValue(
      ok([
        scenario({ id: "sce-arch", name: "Archived one", status: "archived" }),
        scenario({ id: "sce-exp", name: "Expired one", expires_on: "2020-01-01" }),
      ]),
    );
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);

    // Expired rows are listed; archived ones are hidden until asked for.
    expect(
      await screen.findByRole("button", { name: /apply Expired one to the forecast/i }),
    ).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: /show 1 archived/i }));
    expect(
      await screen.findByRole("button", { name: /apply Archived one to the forecast/i }),
    ).toBeDisabled();
  });

  it("still offers undo on an applied scenario that was later archived", async () => {
    // The asymmetry is deliberate: archiving must not un-apply anything (ADR 0055 §4),
    // so the way back has to survive archiving or the change becomes unreachable.
    mocks.scenarioList.mockResolvedValue(
      ok([scenario({ status: "archived", applied_at: "2026-08-02T10:00:00Z" })]),
    );
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
    // Archived rows are hidden until asked for.
    fireEvent.click(await screen.findByRole("button", { name: /show 1 archived/i }));
    expect(
      await screen.findByRole("button", { name: /undo applying/i }),
    ).toBeEnabled();
  });
});

describe("applied reads as brand, alongside the lifecycle (personal-cfo-kkxu)", () => {
  it("marks the applied row and says since when", async () => {
    mocks.scenarioList.mockResolvedValue(
      ok([scenario({ applied_at: "2026-08-02T10:00:00Z" })]),
    );
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);

    expect(await screen.findByText("Applied")).toBeInTheDocument();
    expect(screen.getByText(/since/i)).toBeInTheDocument();
    // The spine + tint land on the row itself, not on the badge.
    const row = screen.getByText("Applied").closest("tr");
    expect(row?.className).toContain("border-l-primary");
  });

  it("shows BOTH facts when a scenario is applied AND archived", async () => {
    // Applied-ness is orthogonal to the lifecycle (ADR 0055 §4), and this pairing is
    // exactly when both matter: the effect is still in the forecast while the scenario
    // itself is filed away. A treatment that replaced the state badge would hide that.
    mocks.scenarioList.mockResolvedValue(
      ok([
        scenario({
          id: "sce-arch",
          name: "Old plan",
          status: "archived",
          applied_at: "2026-08-02T10:00:00Z",
        }),
      ]),
    );
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);

    fireEvent.click(await screen.findByRole("button", { name: /show 1 archived/i }));

    // Scoped to the row: the "Hide archived" toggle also matches /archived/i.
    const row = (await screen.findByText("Applied")).closest("tr");
    expect(row).not.toBeNull();
    expect(within(row!).getByText("Applied")).toBeInTheDocument();
    expect(within(row!).getByText(/^archived$/i)).toBeInTheDocument();
    expect(row?.className).toContain("border-l-primary");
  });

  it("leaves an unapplied row unmarked", async () => {
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
    const row = (await screen.findByText("Maternity leave")).closest("tr");
    expect(row?.className ?? "").not.toContain("border-l-primary");
  });
});

describe("the three states are deliberate (personal-cfo-m1am)", () => {
  it("does NOT claim you have no scenarios while still loading", async () => {
    // The bug this fixes: `all` fell back to [] during the read, so the empty state
    // rendered first. That state invites an action, so a slow load could prompt someone to
    // create a scenario they already have.
    mocks.scenarioList.mockReturnValue(new Promise(() => {}));
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);

    expect(await screen.findByText(/loading your scenarios/i)).toBeInTheDocument();
    expect(screen.queryByText(/no scenarios yet/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/start from/i)).not.toBeInTheDocument();
  });

  it("explains what a scenario IS, and that nothing real changes", async () => {
    // A scenario is the one object with no equivalent outside the app, so a bare button
    // would ask the user to invent the concept.
    mocks.scenarioList.mockResolvedValue(ok([]));
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);

    expect(await screen.findByText(/no scenarios yet/i)).toBeInTheDocument();
    expect(screen.getByText(/a set of changes laid over your forecast/i)).toBeInTheDocument();
    expect(screen.getByText(/nothing real changes/i)).toBeInTheDocument();
  });

  it("offers named starting points that PREFILL the form", async () => {
    // The first scenario should be a choice, not an invention — so the starting point has
    // to actually carry into the form rather than just read as a suggestion.
    mocks.scenarioList.mockResolvedValue(ok([]));
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);

    fireEvent.click(await screen.findByRole("button", { name: /a rent increase/i }));

    const name = screen.getByLabelText(/name/i);
    expect(name).toHaveValue("A rent increase");
  });

  it("says the ledger is unaffected when the scenario read fails", async () => {
    mocks.scenarioList.mockResolvedValue({
      status: "error",
      error: { kind: "Internal", message: "read failed" },
    });
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/accounts, transactions and balances are unaffected/i);
  });
});

describe("the apply confirmation says what will happen (personal-cfo-c5en)", () => {
  async function openApply() {
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
    fireEvent.click(
      await screen.findByRole("button", { name: /apply .* to the forecast/i }),
    );
    return screen.findByRole("dialog", { name: /apply scenario/i });
  }

  it("names what moves, what does NOT, and how to undo — in that order", async () => {
    const dialog = await openApply();
    const text = dialog.textContent ?? "";

    const moves = text.indexOf("will start using");
    const doesNot = text.indexOf("are not edited");
    const undo = text.indexOf("To undo");
    expect(moves).toBeGreaterThan(-1);
    expect(doesNot).toBeGreaterThan(moves);
    expect(undo).toBeGreaterThan(doesNot);
  });

  it("says HOW to undo even when nothing is superseded", async () => {
    // The gap this closes: the only sentence naming the mechanism lived inside the
    // conflict notice, which renders only when there IS a conflict — so a clean apply
    // promised undo without ever saying how.
    const dialog = await openApply();
    expect(dialog).toHaveTextContent(/use Revert on this scenario/i);
    // No conflict notice in this fixture, so the instruction is standing on its own.
    expect(dialog).not.toHaveTextContent(/replaces .* assumptions? you already have/i);
  });

  it("neither urges nor discourages", async () => {
    // ADR 0018: present the consequence, let the user decide.
    const dialog = await openApply();
    expect(dialog).not.toHaveTextContent(/you should|recommend|be careful|warning/i);
  });
});
