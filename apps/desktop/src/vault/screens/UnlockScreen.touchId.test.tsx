import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import type { TouchIdStatusDto } from "@/bindings";

import { UnlockScreen } from "./UnlockScreen";

// personal-cfo-xuu (4 of 4): the unlock screen's Touch ID button and what it says for each rule.

const mocks = vi.hoisted(() => ({
  touchIdStatus: vi.fn(),
  unlockVault: vi.fn(),
  unlockWithTouchId: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: { touchIdStatus: mocks.touchIdStatus },
}));
vi.mock("@/vault/useVault", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/vault/useVault")>()),
  useVault: () => ({
    unlockVault: mocks.unlockVault,
    unlockWithTouchId: mocks.unlockWithTouchId,
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
});

describe("Touch ID on the unlock screen", () => {
  it("offers the button when Touch ID is on and no rule asks for the password", async () => {
    mocks.touchIdStatus.mockResolvedValue(status({}));
    render(<UnlockScreen />);
    expect(await screen.findByRole("button", { name: /unlock with touch id/i })).toBeEnabled();
    // The password field is still the first thing: the button is an addition, not a replacement.
    expect(screen.getByLabelText(/master password/i)).toBeInTheDocument();
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it.each([
    ["after_restart", /your mac restarted/i], // K1
    ["too_many_failed_prompts", /three times in a row/i], // K2
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

  it("a cancelled prompt falls back to the password field, focused, with a calm note", async () => {
    mocks.touchIdStatus.mockResolvedValue(status({ failed_prompts: 1 }));
    mocks.unlockWithTouchId.mockResolvedValue({
      kind: "password",
      reason: "prompt_not_completed",
    });
    render(<UnlockScreen />);
    fireEvent.click(await screen.findByRole("button", { name: /unlock with touch id/i }));
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
    expect(screen.getByRole("status")).toHaveTextContent(/three times in a row/i);
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
});
