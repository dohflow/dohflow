import { render, screen } from "@testing-library/react";

import { PasswordStrengthMeter } from "./PasswordStrengthMeter";

describe("PasswordStrengthMeter", () => {
  it("shows no strength label for an empty or too-short password", () => {
    const { rerender } = render(<PasswordStrengthMeter password="" />);
    expect(screen.queryByText(/password strength/i)).not.toBeInTheDocument();

    rerender(<PasswordStrengthMeter password="abc" />);
    expect(screen.queryByText(/password strength/i)).not.toBeInTheDocument();
  });

  it("labels each strength bucket", () => {
    // 8 chars, letters only → Weak.
    render(<PasswordStrengthMeter password="password" />);
    expect(screen.getByText("Weak")).toBeInTheDocument();
  });

  it("rates a 12-char letter password Fair", () => {
    render(<PasswordStrengthMeter password="passwordword" />);
    expect(screen.getByText("Fair")).toBeInTheDocument();
  });

  it("rates a 12-char password with letters and digits Good", () => {
    render(<PasswordStrengthMeter password="password1234" />);
    expect(screen.getByText("Good")).toBeInTheDocument();
  });

  it("rates a long mixed password with a symbol Strong", () => {
    render(<PasswordStrengthMeter password="password123!@" />);
    expect(screen.getByText("Strong")).toBeInTheDocument();
  });
});
