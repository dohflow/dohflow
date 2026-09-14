import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import { ChangePasswordCard } from "./ChangePasswordCard";

const mocks = vi.hoisted(() => ({
  changePassword: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    changePassword: mocks.changePassword,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

function fill(current: string, next: string, confirm: string) {
  fireEvent.change(screen.getByLabelText(/current password/i), {
    target: { value: current },
  });
  fireEvent.change(screen.getByLabelText(/^new password/i), {
    target: { value: next },
  });
  fireEvent.change(screen.getByLabelText(/confirm new password/i), {
    target: { value: confirm },
  });
}

beforeEach(() => {
  mocks.changePassword.mockReset();
  mocks.changePassword.mockResolvedValue(
    ok({ state: "Unlocked", account_count: 1 }),
  );
});

describe("ChangePasswordCard", () => {
  it("changes the password and confirms", async () => {
    renderWithClient(<ChangePasswordCard />);
    fill("old password 1", "new password 12", "new password 12");

    fireEvent.click(screen.getByRole("button", { name: /change password/i }));

    await waitFor(() =>
      expect(mocks.changePassword).toHaveBeenCalledWith({
        old_password: "old password 1",
        new_password: "new password 12",
      }),
    );
    expect(await screen.findByText(/password changed/i)).toBeInTheDocument();
    // The fields are cleared so the passwords don't linger on screen.
    expect(screen.getByLabelText(/current password/i)).toHaveValue("");
  });

  it("blocks a mismatched confirmation without calling the command", () => {
    renderWithClient(<ChangePasswordCard />);
    fill("old password 1", "new password 12", "something else");

    expect(screen.getByText(/passwords do not match/i)).toBeInTheDocument();
    const button = screen.getByRole("button", { name: /change password/i });
    expect(button).toBeDisabled();
    fireEvent.click(button);
    expect(mocks.changePassword).not.toHaveBeenCalled();
  });

  it("blocks a too-short new password", () => {
    renderWithClient(<ChangePasswordCard />);
    fill("old password 1", "short", "short");

    expect(screen.getByText(/at least 8 characters/i)).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /change password/i }),
    ).toBeDisabled();
  });

  it("surfaces a wrong current password inline", async () => {
    mocks.changePassword.mockResolvedValue({
      status: "error",
      error: "VaultUnlockFailed",
    });
    renderWithClient(<ChangePasswordCard />);
    fill("wrong password", "new password 12", "new password 12");

    fireEvent.click(screen.getByRole("button", { name: /change password/i }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      /incorrect password/i,
    );
    expect(screen.queryByText(/password changed/i)).not.toBeInTheDocument();
  });

  it("repeats the no-reset reminder", () => {
    renderWithClient(<ChangePasswordCard />);
    expect(screen.getByText(/no password reset/i)).toBeInTheDocument();
  });
});
