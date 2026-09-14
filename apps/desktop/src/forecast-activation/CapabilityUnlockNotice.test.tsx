import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { CapabilityUnlockDto } from "@/bindings";
import { CapabilityUnlockNotice } from "./CapabilityUnlockNotice";

const mocks = vi.hoisted(() => ({
  pendingCapabilityUnlocks: vi.fn(),
  acknowledgeCapability: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    pendingCapabilityUnlocks: mocks.pendingCapabilityUnlocks,
    acknowledgeCapability: mocks.acknowledgeCapability,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

const bandUnlock: CapabilityUnlockDto = {
  key: "forecast_band",
  title: "Your forecast now shows a likely range",
  body: "You've recorded enough categorized spending history.",
  factor_key: "spending_history",
};

beforeEach(() => {
  mocks.pendingCapabilityUnlocks.mockReset();
  mocks.acknowledgeCapability.mockReset();
  mocks.acknowledgeCapability.mockResolvedValue(ok(null));
});

describe("CapabilityUnlockNotice", () => {
  it("renders nothing when no capability has unlocked", async () => {
    mocks.pendingCapabilityUnlocks.mockResolvedValue(ok([]));
    const { container } = renderWithClient(<CapabilityUnlockNotice />);
    await waitFor(() =>
      expect(mocks.pendingCapabilityUnlocks).toHaveBeenCalled(),
    );
    expect(container.firstChild).toBeNull();
  });

  it("shows a pending unlock and acknowledges it on dismiss", async () => {
    mocks.pendingCapabilityUnlocks.mockResolvedValue(ok([bandUnlock]));
    renderWithClient(<CapabilityUnlockNotice />);
    await screen.findByText("Your forecast now shows a likely range");

    fireEvent.click(screen.getByRole("button", { name: /dismiss/i }));
    await waitFor(() =>
      expect(mocks.acknowledgeCapability).toHaveBeenCalledWith("forecast_band"),
    );
  });
});
