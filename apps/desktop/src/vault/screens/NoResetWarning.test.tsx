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

  // At the largest system text size an unbreakable word ("unrecoverable") is wider than the card's
  // column. A flex child will not shrink below its content unless `min-width: 0`, and a long word only
  // breaks with `overflow-wrap: anywhere`, so both the warning and the acknowledgement must carry them
  // or the page grows sideways and the copy is clipped at the screen edge (personal-cfo-i93n3.25).
  it("lets the warning and the acknowledgement shrink and wrap at the largest text size", async () => {
    renderWithClient(
      <NoResetWarning acknowledged={false} onAcknowledgedChange={() => {}} />,
    );

    const warning = await screen.findByTestId("no-reset-warning");
    const acknowledgement = screen.getByText(
      "I understand and have saved my password somewhere safe.",
    );
    const label = screen.getByRole("checkbox").closest("label");
    for (const element of [warning, acknowledgement, label]) {
      expect(element).toHaveClass("min-w-0");
    }
    for (const element of [warning, acknowledgement]) {
      expect(element).toHaveClass("[overflow-wrap:anywhere]");
    }
    // The copy is untouched.
    expect(warning.textContent).toBe(CANONICAL);
  });
});
