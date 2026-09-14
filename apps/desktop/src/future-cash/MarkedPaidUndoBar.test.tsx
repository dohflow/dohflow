import { fireEvent, render, screen } from "@testing-library/react";

import { MarkedPaidUndoBar } from "./MarkedPaidUndoBar";

test("shows the bill name and fires undo / dismiss", () => {
  const onUndo = vi.fn();
  const onDismiss = vi.fn();
  render(
    <MarkedPaidUndoBar
      name="Rent"
      onUndo={onUndo}
      onDismiss={onDismiss}
      pending={false}
      error={null}
    />,
  );
  expect(screen.getByText("Rent")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: /undo/i }));
  expect(onUndo).toHaveBeenCalledTimes(1);
  fireEvent.click(screen.getByRole("button", { name: /dismiss/i }));
  expect(onDismiss).toHaveBeenCalledTimes(1);
});

test("shows an error and disables undo while pending", () => {
  render(
    <MarkedPaidUndoBar
      name="Rent"
      onUndo={vi.fn()}
      onDismiss={vi.fn()}
      pending
      error="Couldn't undo — try again."
    />,
  );
  expect(screen.getByText("Couldn't undo — try again.")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: /undo/i })).toBeDisabled();
});
