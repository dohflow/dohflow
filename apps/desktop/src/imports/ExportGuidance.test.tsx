import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { ExportGuidance } from "./ExportGuidance";

describe("ExportGuidance", () => {
  it("starts on the generic guide and switches to a bank's landmarks", () => {
    render(<ExportGuidance />);
    expect(
      screen.getByText(/mobile apps rarely export/i),
    ).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText(/where is the money/i), {
      target: { value: "chase" },
    });
    expect(screen.getByText(/download account activity/i)).toBeInTheDocument();
    // The caveat and the honest menus-move note ride along.
    expect(screen.getByText(/a year or two/i)).toBeInTheDocument();
    expect(screen.getByText(/menus move/i)).toBeInTheDocument();
  });
});
