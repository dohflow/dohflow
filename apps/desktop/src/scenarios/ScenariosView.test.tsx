import { QueryClient } from "@tanstack/react-query";
import { act, fireEvent, screen, within } from "@testing-library/react";

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
    const dialog = await screen.findByRole("dialog", { name: /^Delete “Maternity leave”\?$/ });
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
    const dialog = await screen.findByRole("dialog", { name: /^Delete “Maternity leave”\?$/ });
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

describe("a refused row action says so (personal-cfo-5ie.20)", () => {
  const REASON = "That scenario can't be changed right now.";
  const refusal = { status: "error", error: { Validation: REASON } } as const;

  type Row = {
    name: string;
    arrange: () => void;
    act: () => Promise<void>;
    mock: () => ReturnType<typeof vi.fn>;
  };
  const rows: Row[] = [
    {
      name: "Archive",
      arrange: () => {},
      act: async () => {
        fireEvent.click(await screen.findByRole("button", { name: /archive maternity leave/i }));
      },
      mock: () => mocks.archiveScenario,
    },
    {
      name: "Duplicate",
      arrange: () => {},
      act: async () => {
        fireEvent.click(await screen.findByRole("button", { name: /duplicate maternity leave/i }));
      },
      mock: () => mocks.cloneScenario,
    },
    {
      name: "Restore",
      arrange: () => {
        mocks.scenarioList.mockResolvedValue(
          ok([scenario(), scenario({ id: "sce-9", name: "Old plan", status: "archived" })]),
        );
      },
      act: async () => {
        await screen.findByText("Maternity leave");
        fireEvent.click(screen.getByRole("button", { name: /show 1 archived/i }));
        fireEvent.click(screen.getByRole("button", { name: /restore old plan/i }));
      },
      mock: () => mocks.updateScenario,
    },
    {
      name: "Expiry change",
      arrange: () => {},
      act: async () => {
        const expiry = await screen.findByLabelText(/expiry for maternity leave/i);
        fireEvent.change(expiry, { target: { value: "2026-12-31" } });
        fireEvent.blur(expiry);
      },
      mock: () => mocks.setScenarioExpiry,
    },
  ];

  describe.each(rows)("$name", (row) => {
    it("shows the reason when the app refuses it, and releases the row", async () => {
      row.arrange();
      row.mock().mockResolvedValue(refusal);
      renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
      await row.act();
      await vi.waitFor(() => expect(row.mock()).toHaveBeenCalledTimes(1));
      expect(await screen.findByRole("alert")).toHaveTextContent(REASON);
      // Busy is released: the row's own actions are usable again.
      await vi.waitFor(() =>
        expect(screen.getByRole("button", { name: /^open maternity leave/i })).toBeEnabled(),
      );
      expect(screen.getByLabelText(/expiry for maternity leave/i)).toBeEnabled();
      expect(screen.getByRole("button", { name: /duplicate maternity leave/i })).toBeEnabled();
    });

    it("shows no error when it succeeds", async () => {
      row.arrange();
      renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
      await row.act();
      await vi.waitFor(() => expect(row.mock()).toHaveBeenCalledTimes(1));
      await act(async () => {});
      expect(screen.queryByRole("alert")).toBeNull();
    });

    it("keeps today's message when the call itself throws, and releases the row", async () => {
      row.arrange();
      row.mock().mockRejectedValue(new Error("connection lost"));
      renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
      await row.act();
      expect(await screen.findByRole("alert")).toHaveTextContent("That didn't work. Please try again.");
      await vi.waitFor(() =>
        expect(screen.getByRole("button", { name: /duplicate maternity leave/i })).toBeEnabled(),
      );
    });
  });

  it("a later success clears the earlier refusal", async () => {
    mocks.archiveScenario.mockResolvedValueOnce(refusal);
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: /archive maternity leave/i }));
    expect(await screen.findByRole("alert")).toHaveTextContent(REASON);
    fireEvent.click(await screen.findByRole("button", { name: /archive maternity leave/i }));
    await vi.waitFor(() => expect(mocks.archiveScenario).toHaveBeenCalledTimes(2));
    await vi.waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  });

  it("the deleted-but-applied Undo is locked while another row action is pending, and opens when it settles", async () => {
    mocks.scenarioDeletedAppliedList.mockResolvedValue(
      ok([scenario({ id: "sce-gone", name: "Old plan", applied_at: "2026-08-02T10:00:00Z" })]),
    );
    let settle: (value: unknown) => void = () => {};
    mocks.archiveScenario.mockReturnValue(new Promise((resolve) => (settle = resolve)));
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
    const undo = await screen.findByRole("button", { name: /undo applying deleted scenario old plan/i });
    expect(undo).toBeEnabled();

    fireEvent.click(await screen.findByRole("button", { name: /archive maternity leave/i }));
    await vi.waitFor(() => expect(mocks.archiveScenario).toHaveBeenCalledTimes(1));
    await vi.waitFor(() => expect(undo).toBeDisabled());
    fireEvent.click(undo);
    expect(mocks.scenarioRevertPreview).not.toHaveBeenCalled();

    await act(async () => settle(ok(null)));
    await vi.waitFor(() => expect(undo).toBeEnabled());
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
    const dialog = await screen.findByRole("dialog", { name: /^Apply “Maternity leave”\?$/ });
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
    // The undone row stops showing Undo, so focus goes to the Scenarios heading.
    await vi.waitFor(() => expect(document.activeElement).toBe(screen.getByText("Scenarios")));
  });

  describe("undoing an apply that was changed afterwards (personal-cfo-5ie.12)", () => {
    const applied = () =>
      mocks.scenarioList.mockResolvedValue(
        ok([scenario({ applied_at: "2026-08-02T10:00:00Z" })]),
      );
    const rate = (bps: number) => `{"annual_bps":${bps}}`;
    const line = (after: string | null, change: "edited" | "cleared" = "edited") => ({
      change,
      kind: "inflation_rate",
      target_entity_id: null,
      before_params_json: rate(400),
      after_params_json: after,
    });
    const CONFIRM_TITLE = /^Undo “Maternity leave”\?$/;
    const INTRO =
      "Undoing removes the changes this scenario added to your forecast, including later edits to them. If applying replaced earlier assumptions, they return unless a newer assumption is kept instead. Review what happens below:";

    async function openConfirmation(
      previewValue = preview({ lines: [line(rate(450))], confirmed_descendants: ["evt-2"] }),
    ) {
      applied();
      mocks.scenarioRevertPreview.mockResolvedValue(ok(previewValue));
      renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
      const opener = await screen.findByRole("button", { name: /undo applying/i });
      opener.focus();
      fireEvent.click(opener);
      const dialog = await screen.findByRole("dialog", { name: CONFIRM_TITLE });
      return { opener, dialog };
    }

    it("lists each change it will undo, with Studio's intro, and sends back the ids it showed", async () => {
      mocks.revertScenarioApply.mockResolvedValue(ok(null));
      const { dialog } = await openConfirmation(
        preview({
          lines: [line(null, "cleared")],
          confirmed_descendants: ["evt-2"],
        }),
      );
      expect(dialog).toHaveTextContent(INTRO);
      expect(dialog).toHaveTextContent(/you removed/i);
      // Nothing is written until the person confirms.
      expect(mocks.revertScenarioApply).not.toHaveBeenCalled();

      fireEvent.click(within(dialog).getByRole("button", { name: "Undo scenario" }));
      await vi.waitFor(() =>
        expect(mocks.revertScenarioApply).toHaveBeenCalledWith("sce-1", ["evt-2"]),
      );
      await vi.waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    });

    it("starts on Cancel even when the scrolling body is the first tab stop", async () => {
      // A body that overflows becomes focusable; without the initial-focus option Radix would put
      // focus there, ahead of the footer.
      const scroll = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollHeight");
      const client = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
      Object.defineProperty(HTMLElement.prototype, "scrollHeight", {
        configurable: true,
        get: () => 900,
      });
      Object.defineProperty(HTMLElement.prototype, "clientHeight", {
        configurable: true,
        get: () => 200,
      });
      try {
        const { dialog } = await openConfirmation();
        const body = dialog.querySelector('[tabindex="0"]');
        expect(body).not.toBeNull();
        expect(document.activeElement).toBe(within(dialog).getByRole("button", { name: "Cancel" }));
      } finally {
        if (scroll) Object.defineProperty(HTMLElement.prototype, "scrollHeight", scroll);
        else Reflect.deleteProperty(HTMLElement.prototype, "scrollHeight");
        if (client) Object.defineProperty(HTMLElement.prototype, "clientHeight", client);
        else Reflect.deleteProperty(HTMLElement.prototype, "clientHeight");
      }
    });

    it("cannot be cancelled while the undo is in flight, and a failure shows in the open dialog", async () => {
      let resolve: (value: unknown) => void = () => undefined;
      mocks.revertScenarioApply.mockImplementation(
        () =>
          new Promise((r) => {
            resolve = r;
          }),
      );
      const { dialog } = await openConfirmation();
      fireEvent.click(within(dialog).getByRole("button", { name: "Undo scenario" }));
      await vi.waitFor(() => expect(mocks.revertScenarioApply).toHaveBeenCalledTimes(1));

      // Cancel is inert and Escape does nothing: the write is still on its way.
      const cancel = within(dialog).getByRole("button", { name: "Cancel" });
      expect(cancel).toBeDisabled();
      fireEvent.click(cancel);
      fireEvent.keyDown(dialog, { key: "Escape" });
      expect(screen.getByRole("dialog", { name: CONFIRM_TITLE })).toBeInTheDocument();

      resolve({ status: "error", error: { Validation: "The database is busy." } });
      const alert = await within(screen.getByRole("dialog")).findByRole("alert");
      await vi.waitFor(() => expect(alert).toHaveTextContent("The database is busy."));
      // Cancel works again once it has finished.
      expect(within(screen.getByRole("dialog")).getByRole("button", { name: "Cancel" })).toBeEnabled();
    });

    it("starts on Cancel, the safe action", async () => {
      const { dialog } = await openConfirmation();
      expect(document.activeElement).toBe(within(dialog).getByRole("button", { name: "Cancel" }));
    });

    it("keeps Tab inside the dialog", async () => {
      const { dialog } = await openConfirmation();
      const cancel = within(dialog).getByRole("button", { name: "Cancel" });
      const confirm = within(dialog).getByRole("button", { name: "Undo scenario" });
      confirm.focus();
      fireEvent.keyDown(confirm, { key: "Tab" });
      expect(document.activeElement).toBe(cancel);
      fireEvent.keyDown(cancel, { key: "Tab", shiftKey: true });
      expect(document.activeElement).toBe(confirm);
    });

    it("cancels on Escape before anything is sent, and focus returns to the Undo button", async () => {
      const { opener, dialog } = await openConfirmation();
      fireEvent.keyDown(dialog, { key: "Escape" });
      await vi.waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
      expect(mocks.revertScenarioApply).not.toHaveBeenCalled();
      await vi.waitFor(() => expect(document.activeElement).toBe(opener));
    });

    it("writes nothing when the person cancels", async () => {
      const { dialog } = await openConfirmation();
      expect(dialog).toHaveTextContent(/you changed/i);
      fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
      await vi.waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
      expect(mocks.revertScenarioApply).not.toHaveBeenCalled();
    });

    it("sends focus to the Scenarios heading once the undo succeeds and the Undo button is gone", async () => {
      mocks.revertScenarioApply.mockImplementation(async () => {
        // The scenario stops being applied, so its row stops showing Undo.
        mocks.scenarioList.mockResolvedValue(ok([scenario()]));
        return ok(null);
      });
      const { dialog } = await openConfirmation();
      fireEvent.click(within(dialog).getByRole("button", { name: "Undo scenario" }));
      await vi.waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
      await vi.waitFor(() =>
        expect(document.activeElement).toBe(screen.getByText("Scenarios")),
      );
    });

    it("shows a failed undo inside the dialog, which stays open until the next attempt", async () => {
      mocks.revertScenarioApply.mockResolvedValueOnce({
        status: "error",
        error: { Validation: "the database is busy" },
      });
      // The list did not move, so this is a real failure and not a stale confirmation.
      const { dialog } = await openConfirmation();
      fireEvent.click(within(dialog).getByRole("button", { name: "Undo scenario" }));

      const alert = await within(dialog).findByRole("alert");
      await vi.waitFor(() =>
        expect(alert).toHaveTextContent(
          "That didn’t work, and nothing was undone. The database is busy. Please try again.",
        ),
      );
      expect(screen.getByRole("dialog", { name: CONFIRM_TITLE })).toBeInTheDocument();
      // Not the page-level banner, and not an automatic close.
      expect(screen.queryByText(/couldn.t load your scenarios/i)).not.toBeInTheDocument();

      // The next attempt clears it and, on success, closes.
      mocks.revertScenarioApply.mockResolvedValueOnce(ok(null));
      fireEvent.click(within(dialog).getByRole("button", { name: "Undo scenario" }));
      await vi.waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    });

    it("does not claim nothing was undone when the call itself failed", async () => {
      mocks.revertScenarioApply.mockRejectedValueOnce(new Error("connection lost"));
      const { dialog } = await openConfirmation();
      fireEvent.click(within(dialog).getByRole("button", { name: "Undo scenario" }));
      const alert = await within(dialog).findByRole("alert");
      await vi.waitFor(() =>
        expect(alert).toHaveTextContent(
          "Couldn’t confirm whether the scenario was undone. Check the forecast before trying again.",
        ),
      );
      expect(alert).not.toHaveTextContent(/nothing was undone/i);
      // Still open, retryable and cancellable; nothing dismisses it by itself.
      expect(screen.getByRole("dialog", { name: CONFIRM_TITLE })).toBeInTheDocument();
      expect(within(dialog).getByRole("button", { name: "Undo scenario" })).toBeEnabled();
      expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeEnabled();
    });

    it("re-reads the scenarios after an unknown outcome on the one-click undo, which has no dialog", async () => {
      applied();
      mocks.scenarioRevertPreview.mockResolvedValue(ok(preview({})));
      mocks.revertScenarioApply.mockRejectedValueOnce(new Error("connection lost"));
      renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
      const opener = await screen.findByRole("button", { name: /undo applying/i });
      const listReads = mocks.scenarioList.mock.calls.length;
      // The call failed, but the undo had in fact landed: the list now shows the scenario unapplied.
      mocks.scenarioList.mockResolvedValue(ok([scenario()]));
      fireEvent.click(opener);
      await vi.waitFor(() =>
        expect(
          screen.getByText(
            /Couldn’t confirm whether the scenario was undone\. Check the forecast before trying again\./,
          ),
        ).toBeInTheDocument(),
      );
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
      await vi.waitFor(() =>
        expect(mocks.scenarioList.mock.calls.length).toBeGreaterThan(listReads),
      );
      await vi.waitFor(() =>
        expect(screen.queryByRole("button", { name: /undo applying/i })).not.toBeInTheDocument(),
      );
    });

    it("re-reads the scenarios and the preview after an unknown outcome, so the screen shows what is stored", async () => {
      mocks.revertScenarioApply.mockRejectedValueOnce(new Error("connection lost"));
      const { dialog } = await openConfirmation();
      const listReads = mocks.scenarioList.mock.calls.length;
      const previewReads = mocks.scenarioRevertPreview.mock.calls.length;
      // The call failed, but the undo had in fact landed: the list now shows nothing to undo.
      mocks.scenarioRevertPreview.mockResolvedValueOnce(
        ok(preview({ lines: [], confirmed_descendants: [] })),
      );
      mocks.scenarioList.mockResolvedValue(ok([scenario()]));
      fireEvent.click(within(dialog).getByRole("button", { name: "Undo scenario" }));
      const alert = await within(dialog).findByRole("alert");
      await vi.waitFor(() => expect(alert).toHaveTextContent(/couldn.t confirm/i));
      await vi.waitFor(() =>
        expect(mocks.scenarioRevertPreview.mock.calls.length).toBeGreaterThan(previewReads),
      );
      await vi.waitFor(() =>
        expect(mocks.scenarioList.mock.calls.length).toBeGreaterThan(listReads),
      );
      // The fresh list replaced the stale one: no change lines remain in the open dialog.
      await vi.waitFor(() => expect(within(dialog).queryByText(/you changed/i)).not.toBeInTheDocument());
    });

    it("still reports a typed refusal as nothing undone when re-reading the list fails", async () => {
      mocks.revertScenarioApply.mockResolvedValueOnce({
        status: "error",
        error: { Validation: "The database is busy." },
      });
      const { dialog } = await openConfirmation();
      mocks.scenarioRevertPreview.mockRejectedValueOnce(new Error("offline"));
      fireEvent.click(within(dialog).getByRole("button", { name: "Undo scenario" }));
      const alert = await within(dialog).findByRole("alert");
      await vi.waitFor(() =>
        expect(alert).toHaveTextContent(
          "That didn’t work, and nothing was undone. The database is busy. Please try again.",
        ),
      );
      expect(alert).not.toHaveTextContent(/couldn.t confirm/i);
      expect(within(dialog).getByRole("button", { name: "Undo scenario" })).toBeEnabled();
      expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeEnabled();
      expect(screen.getByRole("dialog", { name: CONFIRM_TITLE })).toBeInTheDocument();
    });

    it("keeps the intro, the status and the lines in the scrolling body, with only the title pinned", async () => {
      const { dialog } = await openConfirmation(
        preview({ lines: [line(rate(450))], confirmed_descendants: ["evt-2"] }),
      );
      const body = dialog.querySelector<HTMLElement>(".overflow-y-auto");
      expect(body).not.toBeNull();
      expect(body).toHaveTextContent(INTRO);
      expect(body).toHaveTextContent(/you changed/i);
      expect(body).toHaveTextContent("Your bills, income and transactions are not edited.");
      // The header holds only the title; the actions are outside the scrolling region.
      const heading = within(dialog).getByRole("heading", { name: CONFIRM_TITLE });
      expect(body).not.toContainElement(heading);
      expect(heading.parentElement).not.toHaveTextContent(INTRO);
      expect(body).not.toContainElement(
        within(dialog).getByRole("button", { name: "Undo scenario" }),
      );
    });

    it("keeps the refreshed-list status in the scrolling body too", async () => {
      applied();
      mocks.scenarioRevertPreview
        .mockResolvedValueOnce(
          ok(preview({ lines: [line(rate(450))], confirmed_descendants: ["evt-2"] })),
        )
        .mockResolvedValueOnce(
          ok(preview({ lines: [line(rate(500))], confirmed_descendants: ["evt-3"] })),
        );
      mocks.revertScenarioApply.mockResolvedValue({
        status: "error",
        error: { Validation: "the changes are not the ones confirmed" },
      });
      renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
      fireEvent.click(await screen.findByRole("button", { name: /undo applying/i }));
      const first = await screen.findByRole("dialog", { name: CONFIRM_TITLE });
      fireEvent.click(within(first).getByRole("button", { name: "Undo scenario" }));
      const status = await screen.findByRole("status");
      expect(status).toHaveTextContent(/changed while you were looking/i);
      expect(status.closest(".overflow-y-auto")).not.toBeNull();
    });

    it("shows a fresh list to confirm again when the changes moved, and writes nothing", async () => {
      applied();
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
      const first = await screen.findByRole("dialog", { name: CONFIRM_TITLE });
      fireEvent.click(within(first).getByRole("button", { name: "Undo scenario" }));

      await vi.waitFor(() =>
        expect(mocks.revertScenarioApply).toHaveBeenCalledWith("sce-1", ["evt-2"]),
      );
      const again = await screen.findByRole("dialog", { name: CONFIRM_TITLE });
      await vi.waitFor(() =>
        expect(again).toHaveTextContent(/changed while you were looking/i),
      );
      // The fresh list is what the next confirm sends.
      mocks.revertScenarioApply.mockResolvedValue(ok(null));
      fireEvent.click(within(again).getByRole("button", { name: "Undo scenario" }));
      await vi.waitFor(() =>
        expect(mocks.revertScenarioApply).toHaveBeenLastCalledWith("sce-1", ["evt-3"]),
      );
    });

    it("explains a later apply that appeared meanwhile instead of a generic error", async () => {
      applied();
      mocks.scenarioRevertPreview
        .mockResolvedValueOnce(
          ok(preview({ lines: [line(rate(450))], confirmed_descendants: ["evt-2"] })),
        )
        // The same changes, but another scenario was applied on top in the meantime.
        .mockResolvedValueOnce(
          ok(
            preview({
              lines: [line(rate(450))],
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
      const first = await screen.findByRole("dialog", { name: CONFIRM_TITLE });
      fireEvent.click(within(first).getByRole("button", { name: "Undo scenario" }));

      const blocked = await screen.findByRole("dialog", { name: /Undo “Second plan” first/ });
      expect(blocked).toHaveTextContent(/Second plan/);
      expect(screen.queryByText(/couldn.t load your scenarios/i)).not.toBeInTheDocument();
    });

    it("says a later scenario has to be undone first, and offers no undo", async () => {
      applied();
      mocks.scenarioRevertPreview.mockResolvedValue(
        ok(preview({ blocker_name: "Second plan" })),
      );
      renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
      const opener = await screen.findByRole("button", { name: /undo applying/i });
      opener.focus();
      fireEvent.click(opener);
      const dialog = await screen.findByRole("dialog", { name: /Undo “Second plan” first/ });
      expect(dialog).toHaveTextContent(
        "“Second plan” was applied after “Maternity leave” and builds on it, so it has to be undone first. Then you can undo “Maternity leave”.",
      );
      expect(
        within(dialog).queryByRole("button", { name: "Undo scenario" }),
      ).not.toBeInTheDocument();
      expect(mocks.revertScenarioApply).not.toHaveBeenCalled();
      // The explanation scrolls with the body, and OK stays outside it.
      const body = dialog.querySelector<HTMLElement>(".overflow-y-auto");
      expect(body).toHaveTextContent(/builds on it/);
      expect(body).not.toContainElement(within(dialog).getByRole("button", { name: "OK" }));
      // OK closes it and focus goes back to the Undo button.
      fireEvent.click(within(dialog).getByRole("button", { name: "OK" }));
      await vi.waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
      await vi.waitFor(() => expect(document.activeElement).toBe(opener));
    });

    it("keeps a deleted but applied scenario reachable, and undoes it from there", async () => {
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

  describe("deleting the last applied scenario (personal-cfo-5ie.19, Studio R2)", () => {
    const gone = () =>
      scenario({ id: "sce-gone", name: "Old plan", applied_at: "2026-08-02T10:00:00Z" });

    it("keeps Deleted, but still applied and its Undo next to the empty state", async () => {
      mocks.scenarioList.mockResolvedValue(ok([]));
      mocks.scenarioDeletedAppliedList.mockResolvedValue(ok([gone()]));
      renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);

      expect(await screen.findByText(/no scenarios yet/i)).toBeInTheDocument();
      expect(await screen.findByText("Deleted, but still applied")).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: /undo applying deleted scenario old plan/i }),
      ).toBeEnabled();
    });

    it("after the delete of the only scenario, the deleted-but-applied entry appears and Undo works", async () => {
      mocks.scenarioList.mockResolvedValueOnce(
        ok([scenario({ id: "sce-gone", name: "Old plan", applied_at: "2026-08-02T10:00:00Z" })]),
      );
      mocks.scenarioList.mockResolvedValue(ok([]));
      mocks.scenarioDeletedAppliedList.mockResolvedValueOnce(ok([]));
      mocks.scenarioDeletedAppliedList.mockResolvedValue(ok([gone()]));
      mocks.revertScenarioApply.mockResolvedValue(ok(null));
      renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);

      fireEvent.click(await screen.findByRole("button", { name: /delete old plan/i }));
      const dialog = await screen.findByRole("dialog", { name: /^Delete “Old plan”\?$/ });
      fireEvent.click(within(dialog).getByRole("button", { name: /^delete scenario$/i }));
      await vi.waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());

      expect(await screen.findByText(/no scenarios yet/i)).toBeInTheDocument();
      expect(await screen.findByText("Deleted, but still applied")).toBeInTheDocument();
      fireEvent.click(
        await screen.findByRole("button", { name: /undo applying deleted scenario old plan/i }),
      );
      await vi.waitFor(() =>
        expect(mocks.revertScenarioApply).toHaveBeenCalledWith("sce-gone", []),
      );
    });

    it("with nothing deleted-but-applied the empty state stands alone", async () => {
      mocks.scenarioList.mockResolvedValue(ok([]));
      renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
      expect(await screen.findByText(/no scenarios yet/i)).toBeInTheDocument();
      expect(screen.queryByText("Deleted, but still applied")).toBeNull();
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
    return screen.findByRole("dialog", { name: /^Apply “Maternity leave”\?$/ });
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

// The shared Dialog keyboard contract (personal-cfo-4d8.6.1) on the Apply and Delete confirmations
// (personal-cfo-4d8.6.2): focus starts on the safe action and stays inside, the background is hidden,
// Escape closes without acting, and focus returns to where it was.
describe.each([
  {
    name: "Apply",
    openerName: /apply .* to the forecast/i,
    title: /^Apply “Maternity leave”\?$/,
    confirmName: /^apply to my forecast$/i,
    action: () => mocks.applyScenario,
  },
  {
    name: "Delete",
    openerName: /delete maternity leave/i,
    title: /^Delete “Maternity leave”\?$/,
    confirmName: /^delete scenario$/i,
    action: () => mocks.deleteScenario,
  },
])("the $name scenario dialog keyboard contract (personal-cfo-4d8.6.2)", (spec) => {
  async function open() {
    mocks.scenarioList.mockResolvedValue(ok([scenario()]));
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
    const opener = await screen.findByRole("button", { name: spec.openerName });
    opener.focus();
    fireEvent.click(opener);
    const dialog = await screen.findByRole("dialog", { name: spec.title });
    return { opener, dialog };
  }

  it("starts on Cancel, the safe action", async () => {
    const { dialog } = await open();
    expect(within(dialog).getByRole("button", { name: "Cancel" })).toHaveFocus();
  });

  it("keeps Tab and Shift+Tab inside the dialog and hides the page behind it", async () => {
    const { dialog } = await open();
    const cancel = within(dialog).getByRole("button", { name: "Cancel" });
    const confirm = within(dialog).getByRole("button", { name: spec.confirmName });
    confirm.focus();
    fireEvent.keyDown(confirm, { key: "Tab" });
    expect(cancel).toHaveFocus();
    fireEvent.keyDown(cancel, { key: "Tab", shiftKey: true });
    expect(confirm).toHaveFocus();
    // The page behind is out of reach of assistive technology.
    expect(screen.queryByRole("button", { name: spec.openerName })).not.toBeInTheDocument();
  });

  it("closes on Escape without acting", async () => {
    await open();
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    await vi.waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(spec.action()).not.toHaveBeenCalled();
  });

  it("returns focus to the button that opened it when Cancel closes it", async () => {
    const { opener, dialog } = await open();
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await vi.waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    await vi.waitFor(() => expect(opener).toHaveFocus());
  });
});


// Studio's lines (personal-cfo-5ie.19): the intro and the Delete dialog never say the real forecast is
// untouched when it is not, and Apply and Delete keep their dialog open, locked, until the write settles.
describe("the intro and the Delete dialog tell the truth (personal-cfo-5ie.19)", () => {
  const INTRO =
    "Try changes and compare them on Cash Flow before you apply them. Comparing does not change your real forecast. Apply makes those changes part of your real forecast.";
  const ARCHIVE = "To keep the plan but stop seeing it, archive it instead.";
  const UNAPPLIED = (n: string) =>
    `This deletes the scenario and its ${n}. Your real forecast is not affected.`;
  const APPLIED = (n: string) =>
    `This deletes the scenario and its ${n}. It does not undo the changes already applied to your real forecast. Use Undo apply to reverse those changes; it remains available after deletion.`;

  async function openDelete(over: Partial<ScenarioDto>, showArchived = false) {
    mocks.scenarioList.mockResolvedValue(
      ok(showArchived ? [scenario({ id: "sce-0", name: "Other" }), scenario(over)] : [scenario(over)]),
    );
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
    if (showArchived) fireEvent.click(await screen.findByRole("button", { name: /show 1 archived/i }));
    fireEvent.click(await screen.findByRole("button", { name: /delete maternity leave/i }));
    return within(await screen.findByRole("dialog", { name: /^Delete “Maternity leave”\?$/ }));
  }

  it("the intro says what a scenario does until it is applied, and that Apply changes the real forecast", async () => {
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
    expect(await screen.findByText(INTRO)).toBeInTheDocument();
    expect(screen.queryByText(/never changes your/i)).toBeNull();
  });

  it("an unapplied scenario keeps the existing sentence, with the archive alternative", async () => {
    const dialog = await openDelete({ applied_at: null, event_count: 3 });
    expect(dialog.getByText(`${UNAPPLIED("3 changes")} ${ARCHIVE}`)).toBeInTheDocument();
  });

  it("an applied scenario says deleting does not undo it, and that Undo apply reverses it", async () => {
    const dialog = await openDelete({ applied_at: "2026-07-02T00:00:00Z", event_count: 3 });
    expect(dialog.getByText(`${APPLIED("3 changes")} ${ARCHIVE}`)).toBeInTheDocument();
    expect(dialog.queryByText(/real forecast is not affected/i)).toBeNull();
  });

  it.each([
    [0, "0 changes"],
    [1, "1 change"],
    [2, "2 changes"],
  ])("a count of %i reads “%s”", async (n, phrase) => {
    const dialog = await openDelete({ applied_at: null, event_count: n });
    expect(dialog.getByText(`${UNAPPLIED(phrase)} ${ARCHIVE}`)).toBeInTheDocument();
  });

  it("an archived scenario omits the whole archive sentence, applied or not", async () => {
    const unapplied = await openDelete({ status: "archived", applied_at: null }, true);
    expect(unapplied.getByText(UNAPPLIED("3 changes"))).toBeInTheDocument();
    expect(unapplied.queryByText(/archive it instead/i)).toBeNull();
  });

  it("an applied archived scenario also omits it", async () => {
    const dialog = await openDelete({ status: "archived", applied_at: "2026-07-02T00:00:00Z" }, true);
    expect(dialog.getByText(APPLIED("3 changes"))).toBeInTheDocument();
    expect(dialog.queryByText(/archive it instead/i)).toBeNull();
  });

  it("applied is read from applied_at, not the lifecycle: an applied draft and an applied active scenario both say it", async () => {
    const dialog = await openDelete({ status: "active", applied_at: "2026-07-02T00:00:00Z" });
    expect(dialog.getByText(`${APPLIED("3 changes")} ${ARCHIVE}`)).toBeInTheDocument();
  });

  it("when the scenarios read fails the row is not known: no reassurance and no way to confirm", async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    mocks.scenarioList.mockResolvedValue(ok([scenario()]));
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />, client);
    fireEvent.click(await screen.findByRole("button", { name: /delete maternity leave/i }));
    const dialog = within(await screen.findByRole("dialog", { name: /^Delete “Maternity leave”\?$/ }));
    expect(dialog.getByText(/real forecast is not affected/i)).toBeInTheDocument();

    mocks.scenarioList.mockResolvedValue({ status: "error", error: { kind: "Internal", message: "read failed" } });
    await act(async () => {
      await client.refetchQueries({ queryKey: ["scenarios"] });
    });
    await vi.waitFor(() => expect(dialog.queryByText(/real forecast is not affected/i)).toBeNull());
    expect(dialog.queryByText(/does not undo/i)).toBeNull();
    expect(dialog.getByRole("button", { name: /^delete scenario$/i })).toBeDisabled();
    expect(dialog.getByRole("button", { name: "Cancel" })).toBeEnabled();
  });

  it("deleting an applied scenario still only tombstones it: nothing is reverted, and its Undo stays reachable", async () => {
    mocks.scenarioList.mockResolvedValue(ok([scenario({ applied_at: "2026-07-02T00:00:00Z" })]));
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: /delete maternity leave/i }));
    const dialog = await screen.findByRole("dialog", { name: /^Delete “Maternity leave”\?$/ });
    fireEvent.click(within(dialog).getByRole("button", { name: /^delete scenario$/i }));
    await vi.waitFor(() => expect(mocks.deleteScenario).toHaveBeenCalledWith("sce-1"));
    expect(mocks.revertScenarioApply).not.toHaveBeenCalled();
  });
});

describe("Apply and Delete lock while their write runs, and report in the dialog (personal-cfo-5ie.19)", () => {
  const specs = [
    {
      name: "Apply",
      opener: /apply .* to the forecast/i,
      title: /^Apply “Maternity leave”\?$/,
      confirm: /^apply to my forecast$/i,
      write: () => mocks.applyScenario,
      unknown:
        "Couldn’t confirm whether the scenario was applied. Close this dialog and check its status in Scenarios before applying it again.",
    },
    {
      name: "Delete",
      opener: /delete maternity leave/i,
      title: /^Delete “Maternity leave”\?$/,
      confirm: /^delete scenario$/i,
      write: () => mocks.deleteScenario,
      unknown:
        "Couldn’t confirm whether the scenario was deleted. Close this dialog and check Scenarios before trying to delete it again.",
    },
  ];

  describe.each(specs)("$name", (spec) => {
    async function openPending() {
      let settle: (value: unknown) => void = () => {};
      spec.write().mockReturnValue(new Promise((resolve) => (settle = resolve)));
      renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
      const opener = await screen.findByRole("button", { name: spec.opener });
      opener.focus();
      fireEvent.click(opener);
      const dialog = await screen.findByRole("dialog", { name: spec.title });
      fireEvent.click(within(dialog).getByRole("button", { name: spec.confirm }));
      await vi.waitFor(() => expect(spec.write()).toHaveBeenCalledTimes(1));
      return { dialog, settle };
    }

    it("Escape, Cancel and a click outside do nothing while the write is running", async () => {
      const { dialog, settle } = await openPending();
      expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeDisabled();
      expect(within(dialog).getByRole("button", { name: spec.confirm })).toBeDisabled();
      fireEvent.keyDown(dialog, { key: "Escape" });
      fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
      fireEvent.pointerDown(document.body);
      fireEvent.click(document.body);
      await act(async () => {});
      expect(screen.getByRole("dialog", { name: spec.title })).toBeInTheDocument();
      expect(spec.write()).toHaveBeenCalledTimes(1);
      await act(async () => settle(ok(null)));
      await vi.waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    });

    it("a refusal shows in the dialog, which stays open and unlocks", async () => {
      const { dialog, settle } = await openPending();
      await act(async () => settle({ status: "error", error: { Validation: "Nothing to do." } }));
      expect(await within(dialog).findByRole("alert")).toHaveTextContent("Nothing to do.");
      expect(screen.getByRole("dialog", { name: spec.title })).toBeInTheDocument();
      expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeEnabled();
      fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
      await vi.waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    });

    it("a call that threw reports in the dialog, which stays open and unlocks", async () => {
      let reject: (error: unknown) => void = () => {};
      spec.write().mockReturnValue(new Promise((_, r) => (reject = r)));
      renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
      fireEvent.click(await screen.findByRole("button", { name: spec.opener }));
      const dialog = within(await screen.findByRole("dialog", { name: spec.title }));
      fireEvent.click(dialog.getByRole("button", { name: spec.confirm }));
      await vi.waitFor(() => expect(spec.write()).toHaveBeenCalledTimes(1));
      await act(async () => reject(new Error("connection lost")));
      expect(await dialog.findByRole("alert")).toHaveTextContent(spec.unknown);
      expect(dialog.getByRole("alert")).not.toHaveTextContent("That didn't work");
      expect(dialog.getByRole("button", { name: "Cancel" })).toBeEnabled();
    });

    it("a call that threw reads the scenarios again, and stays locked until that read settles", async () => {
      let reject: (error: unknown) => void = () => {};
      spec.write().mockReturnValue(new Promise((_, r) => (reject = r)));
      renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
      fireEvent.click(await screen.findByRole("button", { name: spec.opener }));
      const dialog = within(await screen.findByRole("dialog", { name: spec.title }));
      fireEvent.click(dialog.getByRole("button", { name: spec.confirm }));
      await vi.waitFor(() => expect(spec.write()).toHaveBeenCalledTimes(1));

      // The read that follows the throw is held, so the lock can be looked at while it runs.
      const readsBefore = mocks.scenarioList.mock.calls.length;
      let listed: (value: unknown) => void = () => {};
      mocks.scenarioList.mockReturnValue(new Promise((resolve) => (listed = resolve)));
      await act(async () => reject(new Error("connection lost")));

      await vi.waitFor(() => expect(mocks.scenarioList.mock.calls.length).toBeGreaterThan(readsBefore));
      expect(await dialog.findByRole("alert")).toHaveTextContent(spec.unknown);
      expect(dialog.getByRole("button", { name: "Cancel" })).toBeDisabled();
      expect(dialog.getByRole("button", { name: spec.confirm })).toBeDisabled();
      fireEvent.keyDown(screen.getByRole("dialog", { name: spec.title }), { key: "Escape" });
      fireEvent.click(dialog.getByRole("button", { name: "Cancel" }));
      fireEvent.pointerDown(document.body);
      fireEvent.click(document.body);
      await act(async () => {});
      expect(screen.getByRole("dialog", { name: spec.title })).toBeInTheDocument();
      expect(spec.write()).toHaveBeenCalledTimes(1);

      await act(async () => listed(ok([scenario()])));
      await vi.waitFor(() => expect(dialog.getByRole("button", { name: "Cancel" })).toBeEnabled());
      expect(dialog.getByRole("alert")).toHaveTextContent(spec.unknown);
    });

    it("the row's own actions are locked while it runs", async () => {
      const { settle } = await openPending();
      await act(async () => {});
      // The page behind is hidden, so look at the row through the DOM.
      const rowButtons = [...document.querySelectorAll("table button[aria-label]")].filter(
        // Opening Cash Flow is navigation, not a write.
        (button) => !button.getAttribute("aria-label")?.startsWith("Open "),
      );
      expect(rowButtons.length).toBeGreaterThan(3);
      for (const button of rowButtons) expect(button).toBeDisabled();
      await act(async () => settle(ok(null)));
    });
  });

  it("after a confirmed Delete focus lands on the Scenarios heading, not on a button that is gone", async () => {
    mocks.scenarioList.mockResolvedValueOnce(ok([scenario()]));
    // The list read after the delete is held: the dialog is released only once the row is really gone.
    let listed: (value: unknown) => void = () => {};
    mocks.scenarioList.mockReturnValue(new Promise((resolve) => (listed = resolve)));
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
    const opener = await screen.findByRole("button", { name: /delete maternity leave/i });
    opener.focus();
    fireEvent.click(opener);
    const dialog = await screen.findByRole("dialog", { name: /^Delete “Maternity leave”\?$/ });
    fireEvent.click(within(dialog).getByRole("button", { name: /^delete scenario$/i }));
    await vi.waitFor(() => expect(mocks.deleteScenario).toHaveBeenCalledTimes(1));
    await vi.waitFor(() => expect(mocks.scenarioList.mock.calls.length).toBeGreaterThan(1));
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    await act(async () => listed(ok([])));
    await vi.waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    await vi.waitFor(() => expect(screen.getByText("Scenarios", { selector: "[tabindex='-1']" })).toHaveFocus());
  });

  it("after a confirmed Apply focus stays on that row's action, now Undo, never lost to the page", async () => {
    mocks.scenarioList.mockResolvedValueOnce(ok([scenario()]));
    mocks.scenarioList.mockResolvedValue(ok([scenario({ applied_at: "2026-07-02T00:00:00Z" })]));
    mocks.applyScenario.mockResolvedValue(ok(null));
    renderWithClient(<ScenariosView onOpenInCashFlow={vi.fn()} />);
    const opener = await screen.findByRole("button", { name: /apply .* to the forecast/i });
    opener.focus();
    fireEvent.click(opener);
    const dialog = await screen.findByRole("dialog", { name: /^Apply “Maternity leave”\?$/ });
    fireEvent.click(within(dialog).getByRole("button", { name: /^apply to my forecast$/i }));
    await vi.waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    await vi.waitFor(() =>
      expect(screen.getByRole("button", { name: "Undo applying Maternity leave" })).toHaveFocus(),
    );
    expect(document.activeElement).not.toBe(document.body);
  });
});
