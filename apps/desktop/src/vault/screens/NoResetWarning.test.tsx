import { fireEvent, screen } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import { NoResetWarning } from "./NoResetWarning";

// The canonical ADR 0002 copy. The backend pins this to the ADR byte-for-byte
// (vault-crypto::CANONICAL_NO_RESET_WARNING) and returns it from
// `no_reset_warning`; here we assert the component renders exactly what it fetches.
const CANONICAL =
  "DohFlow has no cloud password reset. If you forget your password, your data is unrecoverable. Save your password somewhere safe.";

const mocks = vi.hoisted(() => ({ noResetWarning: vi.fn() }));

vi.mock("@/bindings", () => ({
  commands: { noResetWarning: mocks.noResetWarning },
}));

beforeEach(() => {
  mocks.noResetWarning.mockReset();
  mocks.noResetWarning.mockResolvedValue(CANONICAL);
});

describe("NoResetWarning", () => {
  it("renders the fetched warning verbatim (byte-for-byte)", async () => {
    renderWithClient(
      <NoResetWarning acknowledged={false} onAcknowledgedChange={() => {}} />,
    );

    const rendered = await screen.findByTestId("no-reset-warning");
    expect(rendered.textContent).toBe(CANONICAL);
  });

  it("reports acknowledgement-checkbox toggles to the parent", () => {
    const onChange = vi.fn();
    renderWithClient(
      <NoResetWarning acknowledged={false} onAcknowledgedChange={onChange} />,
    );

    fireEvent.click(screen.getByRole("checkbox"));
    expect(onChange).toHaveBeenCalledWith(true);
  });
});
