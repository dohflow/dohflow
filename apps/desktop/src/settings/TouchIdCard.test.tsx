import { act, fireEvent, screen, waitFor } from "@testing-library/react";

import type { TouchIdStatusDto } from "@/bindings";
import { renderWithClient } from "@/test/renderWithClient";

import { TouchIdCard } from "./TouchIdCard";

const mocks = vi.hoisted(() => ({
  touchIdStatus: vi.fn(),
  touchIdEnable: vi.fn(),
  touchIdDisable: vi.fn(),
}));

vi.mock("@/bindings", () => ({ commands: mocks }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

function status(partial: Partial<TouchIdStatusDto>): TouchIdStatusDto {
  return { state: "off", offered: false, reason: "not_enabled", failed_prompts: 0, ...partial };
}

beforeEach(() => {
  for (const mock of Object.values(mocks)) mock.mockReset();
});

describe("TouchIdCard", () => {
  it("says the master password stays the way in, with the three rules", async () => {
    mocks.touchIdStatus.mockResolvedValue(status({}));
    renderWithClient(<TouchIdCard />);
    await screen.findByLabelText(/master password/i);
    const text = document.body.textContent ?? "";
    expect(text).toMatch(/after you restart your Mac/i);
    expect(text).toMatch(/three Touch ID tries that fail or are canceled/i);
    expect(text).toMatch(/every 30 days/i);
  });

  it("turns Touch ID on with the password entered again, then clears the field", async () => {
    mocks.touchIdStatus
      .mockResolvedValueOnce(status({}))
      .mockResolvedValue(status({ state: "on", offered: true, reason: null }));
    mocks.touchIdEnable.mockResolvedValue(ok({ enabled: true, reason: null }));
    renderWithClient(<TouchIdCard />);
    const button = await screen.findByRole("button", { name: /turn on touch id/i });
    expect(button).toBeDisabled(); // no password yet
    fireEvent.change(screen.getByLabelText(/master password/i), {
      target: { value: "synthetic-password" },
    });
    fireEvent.click(button);
    await waitFor(() =>
      expect(mocks.touchIdEnable).toHaveBeenCalledWith({ password: "synthetic-password" }),
    );
    // The visible line and the one announcement for a screen reader.
    expect(await screen.findAllByText(/touch id is on for this vault/i)).toHaveLength(2);
    expect(screen.queryByLabelText(/master password/i)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /turn off touch id/i })).toBeInTheDocument();
  });

  it("a wrong password says so and turns nothing on", async () => {
    mocks.touchIdStatus.mockResolvedValue(status({}));
    mocks.touchIdEnable.mockResolvedValue({ status: "error", error: "VaultUnlockFailed" });
    renderWithClient(<TouchIdCard />);
    fireEvent.change(await screen.findByLabelText(/master password/i), {
      target: { value: "nope" },
    });
    fireEvent.click(screen.getByRole("button", { name: /turn on touch id/i }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Incorrect password");
    expect(screen.getByRole("button", { name: /turn on touch id/i })).toBeInTheDocument();
  });

  it("an enable the device refuses stays off and explains why in plain words", async () => {
    mocks.touchIdStatus.mockResolvedValue(status({}));
    mocks.touchIdEnable.mockResolvedValue(
      ok({ enabled: false, reason: "biometry_not_enrolled" }),
    );
    renderWithClient(<TouchIdCard />);
    fireEvent.change(await screen.findByLabelText(/master password/i), {
      target: { value: "synthetic-password" },
    });
    fireEvent.click(screen.getByRole("button", { name: /turn on touch id/i }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/no fingerprint is set up/i);
    expect(screen.getByLabelText(/master password/i)).toHaveValue("");
  });

  it("turns off with one click and no password", async () => {
    mocks.touchIdStatus.mockResolvedValue(status({ state: "on", offered: true, reason: null }));
    mocks.touchIdDisable.mockResolvedValue(ok(status({})));
    renderWithClient(<TouchIdCard />);
    fireEvent.click(await screen.findByRole("button", { name: /turn off touch id/i }));
    await waitFor(() => expect(mocks.touchIdDisable).toHaveBeenCalledTimes(1));
    expect(await screen.findByRole("button", { name: /turn on touch id/i })).toBeInTheDocument();
  });

  it("while on, explains a rule that wants the password next time", async () => {
    mocks.touchIdStatus.mockResolvedValue(
      status({ state: "on", offered: false, reason: "password_too_old" }),
    );
    renderWithClient(<TouchIdCard />);
    expect(await screen.findByRole("status")).toHaveTextContent(/30 days/i);
  });

  it.each([
    ["signed_build_needed", /signed release/i],
    ["biometry_not_present", /isn't available on this mac/i],
    ["biometry_not_enrolled", /no fingerprint is set up/i],
  ])("when unavailable (%s) it says why and offers no control", async (reason, words) => {
    mocks.touchIdStatus.mockResolvedValue(status({ state: "unavailable", reason }));
    renderWithClient(<TouchIdCard />);
    expect(await screen.findByText(words)).toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
    expect(screen.queryByLabelText(/master password/i)).not.toBeInTheDocument();
  });

  it("an unreadable status leaves the password working and offers nothing", async () => {
    mocks.touchIdStatus.mockRejectedValue(new Error("no host"));
    renderWithClient(<TouchIdCard />);
    expect(await screen.findByText(/status could not be read/i)).toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  describe("a call that throws (the outcome is unknown)", () => {
    const UNKNOWN = "Couldn’t confirm the Touch ID change. Check its status before trying again.";
    const announcement = () => screen.getByTestId("touch-id-announcement");

    it("a rejected enable releases the button, keeps the password and offers a read-only check", async () => {
      mocks.touchIdStatus.mockResolvedValue(status({}));
      mocks.touchIdEnable.mockRejectedValue(new Error("host went away"));
      renderWithClient(<TouchIdCard />);
      fireEvent.change(await screen.findByLabelText(/master password/i), {
        target: { value: "synthetic-password" },
      });
      fireEvent.click(screen.getByRole("button", { name: /turn on touch id/i }));
      expect(await screen.findByRole("alert")).toHaveTextContent(UNKNOWN);
      // No on/off claim until the status is read again.
      expect(screen.queryByText(/touch id is on for this vault/i)).not.toBeInTheDocument();
      const check = screen.getByRole("button", { name: "Check Touch ID status" });
      expect(check).toBeEnabled();
      await waitFor(() => expect(check).toHaveFocus());

      fireEvent.click(check);
      // The check only reads: nothing is enabled again and nothing prompts.
      await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
      expect(mocks.touchIdEnable).toHaveBeenCalledTimes(1);
      expect(mocks.touchIdDisable).not.toHaveBeenCalled();
      // Still off, and the typed password is still there.
      expect(screen.getByLabelText(/master password/i)).toHaveValue("synthetic-password");
    });

    it("a rejected disable is unknown too, and the check shows what the host says", async () => {
      mocks.touchIdStatus
        .mockResolvedValueOnce(status({ state: "on", offered: true, reason: null }))
        .mockResolvedValue(status({}));
      mocks.touchIdDisable.mockRejectedValue(new Error("host went away"));
      renderWithClient(<TouchIdCard />);
      fireEvent.click(await screen.findByRole("button", { name: /turn off touch id/i }));
      expect(await screen.findByRole("alert")).toHaveTextContent(UNKNOWN);
      expect(screen.queryByText(/touch id is on for this vault/i)).not.toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: "Check Touch ID status" }));
      expect(await screen.findByRole("button", { name: /turn on touch id/i })).toBeInTheDocument();
      expect(mocks.touchIdDisable).toHaveBeenCalledTimes(1);
    });

    it("an enable that worked but whose status read rejects is unknown, not on", async () => {
      mocks.touchIdStatus
        .mockResolvedValueOnce(status({}))
        .mockRejectedValueOnce(new Error("host went away"))
        .mockResolvedValue(status({ state: "on", offered: true, reason: null }));
      mocks.touchIdEnable.mockResolvedValue(ok({ enabled: true, reason: null }));
      renderWithClient(<TouchIdCard />);
      fireEvent.change(await screen.findByLabelText(/master password/i), {
        target: { value: "synthetic-password" },
      });
      fireEvent.click(screen.getByRole("button", { name: /turn on touch id/i }));
      expect(await screen.findByRole("alert")).toHaveTextContent(UNKNOWN);
      expect(screen.queryByText(/touch id is on for this vault/i)).not.toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: "Check Touch ID status" }));
      expect(await screen.findByRole("button", { name: /turn off touch id/i })).toBeInTheDocument();
    });

    it("a status check that also fails keeps the alert and the check", async () => {
      mocks.touchIdStatus
        .mockResolvedValueOnce(status({ state: "on", offered: true, reason: null }))
        .mockRejectedValue(new Error("host went away"));
      mocks.touchIdDisable.mockRejectedValue(new Error("host went away"));
      renderWithClient(<TouchIdCard />);
      fireEvent.click(await screen.findByRole("button", { name: /turn off touch id/i }));
      await screen.findByRole("alert");
      fireEvent.click(screen.getByRole("button", { name: "Check Touch ID status" }));
      await waitFor(() => expect(mocks.touchIdStatus).toHaveBeenCalledTimes(2));
      expect(await screen.findByRole("alert")).toHaveTextContent(UNKNOWN);
      expect(screen.getByRole("button", { name: "Check Touch ID status" })).toBeEnabled();
    });

    describe("a status check that fails (xuu repair round 2)", () => {
      const FAILED = "Couldn’t check Touch ID status. Try again.";

      /// Every text the live region shows, in order, so a repeat is told from a text that never changed.
      function recordAnnouncements() {
        const region = screen.getByTestId("touch-id-announcement");
        const seen: string[] = [];
        new MutationObserver(() => {
          const text = region.textContent ?? "";
          if (seen.at(-1) !== text) seen.push(text);
        }).observe(region, { childList: true, characterData: true, subtree: true });
        return seen;
      }

      async function unknownState() {
        mocks.touchIdStatus.mockResolvedValueOnce(status({ state: "on", offered: true, reason: null }));
        mocks.touchIdDisable.mockRejectedValue(new Error("host went away"));
        renderWithClient(<TouchIdCard />);
        fireEvent.click(await screen.findByRole("button", { name: /turn off touch id/i }));
        await screen.findByRole("alert");
      }

      it("is announced after each failure, including a second identical one", async () => {
        await unknownState();
        mocks.touchIdStatus.mockRejectedValue(new Error("host went away"));
        const seen = recordAnnouncements();
        for (const times of [1, 2]) {
          fireEvent.click(screen.getByRole("button", { name: "Check Touch ID status" }));
          await waitFor(() => expect(seen.filter((t) => t === FAILED)).toHaveLength(times));
          await waitFor(() =>
            expect(screen.getByRole("button", { name: "Check Touch ID status" })).toBeEnabled(),
          );
        }
        // The text changed in between (the pending announcement), so a polite region says it again.
        expect(seen).toEqual(["Checking Touch ID…", FAILED, "Checking Touch ID…", FAILED]);
      });

      it("a pending check announces once, then on failure keeps the alert and Check, and refocuses Check", async () => {
        await unknownState();
        let fail: (reason: Error) => void = () => {};
        mocks.touchIdStatus.mockReturnValue(new Promise((_, reject) => (fail = reject)));
        fireEvent.click(screen.getByRole("button", { name: "Check Touch ID status" }));
        await waitFor(() =>
          expect(screen.getByTestId("touch-id-announcement")).toHaveTextContent("Checking Touch ID…"),
        );
        expect(screen.getByRole("button", { name: "Check Touch ID status" })).toBeDisabled();
        await act(async () => fail(new Error("host went away")));
        expect(screen.getByTestId("touch-id-announcement")).toHaveTextContent(FAILED);
        expect(screen.getByRole("alert")).toHaveTextContent(UNKNOWN);
        const check = screen.getByRole("button", { name: "Check Touch ID status" });
        expect(check).toBeEnabled();
        await waitFor(() => expect(check).toHaveFocus());
      });

      it("guesses no state and makes no enable or disable call", async () => {
        await unknownState();
        mocks.touchIdStatus.mockRejectedValue(new Error("host went away"));
        fireEvent.click(screen.getByRole("button", { name: "Check Touch ID status" }));
        await waitFor(() =>
          expect(screen.getByTestId("touch-id-announcement")).toHaveTextContent(FAILED),
        );
        expect(screen.queryByText(/touch id is (on|off) for this vault/i)).not.toBeInTheDocument();
        expect(mocks.touchIdDisable).toHaveBeenCalledTimes(1);
        expect(mocks.touchIdEnable).not.toHaveBeenCalled();
      });
    });

    it("says nothing in the live region just because the card loaded", async () => {
      mocks.touchIdStatus.mockResolvedValue(status({ state: "on", offered: true, reason: null }));
      renderWithClient(<TouchIdCard />);
      await screen.findByRole("button", { name: /turn off touch id/i });
      expect(announcement()).toBeEmptyDOMElement();
    });

    it("turning on announces the wait, then the result once, and focus lands on Turn off", async () => {
      mocks.touchIdStatus
        .mockResolvedValueOnce(status({}))
        .mockResolvedValue(status({ state: "on", offered: true, reason: null }));
      let finish: (value: ReturnType<typeof ok>) => void = () => {};
      mocks.touchIdEnable.mockReturnValue(new Promise((resolve) => (finish = resolve)));
      renderWithClient(<TouchIdCard />);
      fireEvent.change(await screen.findByLabelText(/master password/i), {
        target: { value: "synthetic-password" },
      });
      fireEvent.click(screen.getByRole("button", { name: /turn on touch id/i }));
      await waitFor(() => expect(announcement()).toHaveTextContent("Turning on…"));
      finish(ok({ enabled: true, reason: null }));
      await waitFor(() =>
        expect(announcement()).toHaveTextContent(/^Touch ID is on for this vault\.$/),
      );
      await waitFor(() =>
        expect(screen.getByRole("button", { name: /turn off touch id/i })).toHaveFocus(),
      );
      // One persistent region, not a new one per change.
      expect(screen.getAllByTestId("touch-id-announcement")).toHaveLength(1);
    });

    it("turning off announces the result once, and focus lands on the master password field", async () => {
      mocks.touchIdStatus.mockResolvedValue(status({ state: "on", offered: true, reason: null }));
      mocks.touchIdDisable.mockResolvedValue(ok(status({})));
      renderWithClient(<TouchIdCard />);
      fireEvent.click(await screen.findByRole("button", { name: /turn off touch id/i }));
      await waitFor(() =>
        expect(announcement()).toHaveTextContent(/^Touch ID is off for this vault\.$/),
      );
      await waitFor(() => expect(screen.getByLabelText(/master password/i)).toHaveFocus());
    });
  });
});
