import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import type { TouchIdStatusDto } from "@/bindings";

import { UnlockScreen } from "./UnlockScreen";

// v0.4.1 ships with the Touch ID controls off (`vault/touchIdEnabled.ts`); the cases here are about the controls themselves, so
// they run with the flag on, and the "flag off" cases below turn it off. `vault/touchIdEnabled.test.ts` pins the shipped value.
const flag = vi.hoisted(() => ({ on: true }));
vi.mock("@/vault/touchIdEnabled", () => ({
  get TOUCH_ID_ENABLED() {
    return flag.on;
  },
}));

// personal-cfo-xuu (4 of 4): the unlock screen's Touch ID button and what it says for each rule.

// A status the hook reports whatever the host says, for the cases that must prove the SCREEN's own checks of the flag: with
// the hook itself returning "on and offered", only the screen's gates stand between that status and the button.
const hook = vi.hoisted(() => ({ forced: null as null | { status: unknown; loaded: boolean; refresh: () => Promise<unknown>; setStatus: () => void } }));
vi.mock("@/vault/useTouchIdStatus", async (importOriginal) => {
  const real = await importOriginal<typeof import("@/vault/useTouchIdStatus")>();
  return { ...real, useTouchIdStatus: () => hook.forced ?? real.useTouchIdStatus() };
});

const mocks = vi.hoisted(() => ({
  touchIdStatus: vi.fn(),
  unlockVault: vi.fn(),
  unlockWithTouchId: vi.fn(),
  refresh: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: { touchIdStatus: mocks.touchIdStatus },
}));
vi.mock("@/vault/useVault", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/vault/useVault")>()),
  useVault: () => ({
    unlockVault: mocks.unlockVault,
    unlockWithTouchId: mocks.unlockWithTouchId,
    refresh: mocks.refresh,
  }),
}));
vi.mock("@/backup/RestoreFromBackup", () => ({
  RestoreFromBackup: () => <button type="button">Restore as a new vault…</button>,
}));

function status(partial: Partial<TouchIdStatusDto>): TouchIdStatusDto {
  return {
    state: "on",
    offered: true,
    reason: null,
    failed_prompts: 0,
    ...partial,
  };
}

const touchButton = () => screen.queryByRole("button", { name: /unlock with touch id/i });

beforeEach(() => {
  mocks.touchIdStatus.mockReset();
  mocks.unlockVault.mockReset();
  mocks.unlockWithTouchId.mockReset();
  mocks.refresh.mockReset();
});

describe("Touch ID switched off (v0.4.1)", () => {
  afterEach(() => {
    flag.on = true;
    hook.forced = null;
  });

  it("the screen's own checks hide the button and the rule text even when the status hook reports Touch ID on and offered", async () => {
    flag.on = false;
    hook.forced = { status: status({}), loaded: true, refresh: async () => status({}), setStatus: () => undefined };
    render(<UnlockScreen />);
    expect(await screen.findByLabelText(/master password/i)).toBeInTheDocument();
    expect(touchButton()).not.toBeInTheDocument();
    expect(screen.queryByText(/touch id/i)).not.toBeInTheDocument();
    hook.forced = { status: status({ offered: false, reason: "after_restart" }), loaded: true, refresh: async () => undefined, setStatus: () => undefined };
    render(<UnlockScreen />);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("the same forced status shows the button with the flag on (so the case above proves the gates, not a broken setup)", async () => {
    flag.on = true;
    hook.forced = { status: status({}), loaded: true, refresh: async () => status({}), setStatus: () => undefined };
    render(<UnlockScreen />);
    expect(await screen.findByRole("button", { name: /unlock with touch id/i })).toBeEnabled();
  });

  it("shows no Touch ID button and says nothing about Touch ID, even if the host would offer it", async () => {
    flag.on = false;
    mocks.touchIdStatus.mockResolvedValue(status({}));
    render(<UnlockScreen />);
    expect(await screen.findByLabelText(/master password/i)).toBeInTheDocument();
    expect(touchButton()).not.toBeInTheDocument();
    expect(screen.queryByText(/touch id/i)).not.toBeInTheDocument();
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("does not even ask the host for the Touch ID status, and never prompts", async () => {
    flag.on = false;
    mocks.touchIdStatus.mockResolvedValue(status({}));
    render(<UnlockScreen />);
    await screen.findByLabelText(/master password/i);
    expect(mocks.touchIdStatus).not.toHaveBeenCalled();
    expect(mocks.unlockWithTouchId).not.toHaveBeenCalled();
  });

  it("shows no rule explanation either (a restart, three misses, 30 days): there is no Touch ID to explain", async () => {
    flag.on = false;
    mocks.touchIdStatus.mockResolvedValue(status({ offered: false, reason: "after_restart" }));
    render(<UnlockScreen />);
    await screen.findByLabelText(/master password/i);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("still unlocks with the master password", async () => {
    flag.on = false;
    mocks.unlockVault.mockResolvedValue(null);
    render(<UnlockScreen />);
    fireEvent.change(await screen.findByLabelText(/master password/i), { target: { value: "synthetic-password" } });
    fireEvent.click(screen.getByRole("button", { name: /^unlock$/i }));
    await waitFor(() => expect(mocks.unlockVault).toHaveBeenCalledWith("synthetic-password"));
  });
});

describe("Touch ID on the unlock screen", () => {
  it("offers the button when Touch ID is on and no rule asks for the password", async () => {
    mocks.touchIdStatus.mockResolvedValue(status({}));
    render(<UnlockScreen />);
    expect(await screen.findByRole("button", { name: /unlock with touch id/i })).toBeEnabled();
    // Nothing is prompted on its own: the screen only offers it.
    expect(mocks.unlockWithTouchId).not.toHaveBeenCalled();
    // The password field is still the first thing: the button is an addition, not a replacement.
    expect(screen.getByLabelText(/master password/i)).toBeInTheDocument();
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it.each([
    ["after_restart", /your mac restarted/i], // K1
    ["too_many_failed_prompts", /three times since you last entered your master password/i], // K2
    ["biometry_locked_out", /unlock your mac with its login password/i],
    ["password_too_old", /30 days/i], // K3
  ])("K: %s hides the button and says why in plain words", async (reason, words) => {
    mocks.touchIdStatus.mockResolvedValue(status({ offered: false, reason }));
    render(<UnlockScreen />);
    expect(await screen.findByRole("status")).toHaveTextContent(words);
    expect(touchButton()).not.toBeInTheDocument();
    expect(screen.getByLabelText(/master password/i)).toBeInTheDocument();
  });

  it.each([["off"], ["unavailable"]] as const)(
    "shows nothing about Touch ID when it is %s",
    async (state) => {
      mocks.touchIdStatus.mockResolvedValue(
        status({ state, offered: false, reason: "not_enabled" }),
      );
      render(<UnlockScreen />);
      await waitFor(() => expect(mocks.touchIdStatus).toHaveBeenCalled());
      expect(touchButton()).not.toBeInTheDocument();
      expect(screen.queryByRole("status")).not.toBeInTheDocument();
    },
  );

  it("shows nothing when the status cannot be read, and the password still works", async () => {
    mocks.touchIdStatus.mockRejectedValue(new Error("no host"));
    mocks.unlockVault.mockResolvedValue(null);
    render(<UnlockScreen />);
    await waitFor(() => expect(mocks.touchIdStatus).toHaveBeenCalled());
    expect(touchButton()).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText(/master password/i), {
      target: { value: "synthetic-password" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^unlock$/i }));
    await waitFor(() => expect(mocks.unlockVault).toHaveBeenCalledWith("synthetic-password"));
  });

  it("asks the host once and leaves the screen to the unlocked vault on success", async () => {
    mocks.touchIdStatus.mockResolvedValue(status({}));
    mocks.unlockWithTouchId.mockResolvedValue({ kind: "unlocked" });
    render(<UnlockScreen />);
    fireEvent.click(await screen.findByRole("button", { name: /unlock with touch id/i }));
    await waitFor(() => expect(mocks.unlockWithTouchId).toHaveBeenCalledTimes(1));
    expect(mocks.unlockVault).not.toHaveBeenCalled();
  });

  it("a canceled prompt falls back to the password field, focused, with a calm note", async () => {
    mocks.touchIdStatus.mockResolvedValue(status({ failed_prompts: 1 }));
    mocks.unlockWithTouchId.mockResolvedValue({
      kind: "password",
      reason: "prompt_not_completed",
    });
    render(<UnlockScreen />);
    const button = await screen.findByRole("button", { name: /unlock with touch id/i });
    // A real click moves focus to the button (fireEvent does not), so the password field has to win
    // it back: without this the field's own autofocus would pass the assertion on its own.
    button.focus();
    expect(button).toHaveFocus();
    fireEvent.click(button);
    expect(await screen.findByRole("status")).toHaveTextContent(/didn't unlock the vault/i);
    await waitFor(() => expect(screen.getByLabelText(/master password/i)).toHaveFocus());
    // Not an error: nothing red, and the button is there for another try.
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(touchButton()).toBeEnabled();
  });

  it("the third miss swaps the button for the rule", async () => {
    mocks.touchIdStatus
      .mockResolvedValueOnce(status({ failed_prompts: 2 }))
      .mockResolvedValue(
        status({ offered: false, reason: "too_many_failed_prompts", failed_prompts: 3 }),
      );
    mocks.unlockWithTouchId.mockResolvedValue({
      kind: "password",
      reason: "too_many_failed_prompts",
    });
    render(<UnlockScreen />);
    fireEvent.click(await screen.findByRole("button", { name: /unlock with touch id/i }));
    await waitFor(() => expect(touchButton()).not.toBeInTheDocument());
    expect(screen.getByRole("status")).toHaveTextContent(
      /three times since you last entered your master password/i,
    );
  });

  it("a failure the password would also hit is shown as an error, with the restore for a damaged vault", async () => {
    mocks.touchIdStatus.mockResolvedValue(status({}));
    mocks.unlockWithTouchId.mockResolvedValue({
      kind: "error",
      error: "VaultIntegrityCheckFailed",
    });
    render(<UnlockScreen />);
    fireEvent.click(await screen.findByRole("button", { name: /unlock with touch id/i }));
    expect(await screen.findByRole("alert")).toHaveTextContent("found damage");
    expect(screen.getByRole("button", { name: /restore as a new vault/i })).toBeInTheDocument();
  });

  it("does not run a second attempt while one is waiting", async () => {
    mocks.touchIdStatus.mockResolvedValue(status({}));
    let finish: (value: { kind: "unlocked" }) => void = () => {};
    mocks.unlockWithTouchId.mockReturnValue(new Promise((resolve) => (finish = resolve)));
    render(<UnlockScreen />);
    fireEvent.click(await screen.findByRole("button", { name: /unlock with touch id/i }));
    const waiting = await screen.findByRole("button", { name: /waiting for touch id/i });
    expect(waiting).toBeDisabled();
    fireEvent.click(waiting);
    expect(mocks.unlockWithTouchId).toHaveBeenCalledTimes(1);
    finish({ kind: "unlocked" });
  });

  it("an unknown reason code from a newer host falls back to the password", async () => {
    mocks.touchIdStatus.mockResolvedValue(status({}));
    mocks.unlockWithTouchId.mockResolvedValue({ kind: "password", reason: "something_new" });
    render(<UnlockScreen />);
    fireEvent.click(await screen.findByRole("button", { name: /unlock with touch id/i }));
    expect(await screen.findByRole("status")).toHaveTextContent("Enter your master password.");
  });

  it("a Touch ID call that throws releases the screen, keeps the password and says it could not confirm", async () => {
    mocks.touchIdStatus.mockResolvedValue(status({}));
    mocks.unlockWithTouchId.mockRejectedValue(new Error("host went away"));
    render(<UnlockScreen />);
    const password = screen.getByLabelText(/master password/i);
    fireEvent.change(password, { target: { value: "synthetic-password" } });
    const button = await screen.findByRole("button", { name: /unlock with touch id/i });
    button.focus();
    fireEvent.click(button);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Couldn’t confirm the Touch ID result. Enter your master password.",
    );
    // Released: the password is still there and works, the button can be pressed again, and the
    // password field has the focus.
    await waitFor(() => expect(password).toHaveFocus());
    expect(password).toHaveValue("synthetic-password");
    expect(screen.getByRole("button", { name: /^unlock$/i })).toBeEnabled();
    expect(touchButton()).toBeEnabled();
    // Both the vault and Touch ID are asked again; nothing prompts on its own.
    expect(mocks.refresh).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(mocks.touchIdStatus).toHaveBeenCalledTimes(2));
    expect(mocks.unlockWithTouchId).toHaveBeenCalledTimes(1);
  });

  it("the alert clears when Touch ID is tried again", async () => {
    mocks.touchIdStatus.mockResolvedValue(status({}));
    mocks.unlockWithTouchId
      .mockRejectedValueOnce(new Error("host went away"))
      .mockResolvedValue({ kind: "password", reason: "prompt_not_completed" });
    render(<UnlockScreen />);
    fireEvent.click(await screen.findByRole("button", { name: /unlock with touch id/i }));
    await screen.findByRole("alert");
    fireEvent.click(await screen.findByRole("button", { name: /unlock with touch id/i }));
    await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
  });

  describe("the unconfirmed-result alert and a password attempt (xuu repair round 2)", () => {
    const UNKNOWN = "Couldn’t confirm the Touch ID result. Enter your master password.";

    async function unknownThenType() {
      mocks.touchIdStatus.mockResolvedValue(status({}));
      mocks.unlockWithTouchId.mockRejectedValue(new Error("host went away"));
      render(<UnlockScreen />);
      fireEvent.click(await screen.findByRole("button", { name: /unlock with touch id/i }));
      await screen.findByText(UNKNOWN);
      const password = screen.getByLabelText(/master password/i);
      fireEvent.change(password, { target: { value: "synthetic-password" } });
      return password;
    }

    it("clears when a password attempt starts, before it answers", async () => {
      await unknownThenType();
      mocks.unlockVault.mockReturnValue(new Promise(() => {}));
      fireEvent.click(screen.getByRole("button", { name: /^unlock$/i }));
      await waitFor(() => expect(screen.queryByText(UNKNOWN)).not.toBeInTheDocument());
    });

    it("a wrong password then shows only its own error", async () => {
      await unknownThenType();
      mocks.unlockVault.mockResolvedValue("VaultUnlockFailed");
      fireEvent.click(screen.getByRole("button", { name: /^unlock$/i }));
      const alert = await screen.findByRole("alert");
      expect(alert).toHaveTextContent("Incorrect password");
      expect(screen.getAllByRole("alert")).toHaveLength(1);
      expect(screen.queryByText(UNKNOWN)).not.toBeInTheDocument();
    });

    it("typing does not clear it, and an empty submit keeps it", async () => {
      const password = await unknownThenType();
      expect(screen.getByText(UNKNOWN)).toBeInTheDocument(); // typed, not submitted
      fireEvent.change(password, { target: { value: "" } });
      fireEvent.submit(password.closest("form")!);
      expect(screen.getByText(UNKNOWN)).toBeInTheDocument();
      expect(mocks.unlockVault).not.toHaveBeenCalled();
    });
  });
});
