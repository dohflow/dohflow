import { fireEvent, screen, waitFor } from "@testing-library/react";

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
    expect(text).toMatch(/three Touch ID tries that fail or are cancelled/i);
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
    expect(await screen.findByText(/touch id is on for this vault/i)).toBeInTheDocument();
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
});
