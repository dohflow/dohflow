import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import { AutoCategorizeOnImportCard } from "./AutoCategorizeOnImportCard";

const mocks = vi.hoisted(() => ({
  autoCategorizeOnImport: vi.fn(),
  setAutoCategorizeOnImport: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    autoCategorizeOnImport: mocks.autoCategorizeOnImport,
    setAutoCategorizeOnImport: mocks.setAutoCategorizeOnImport,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

beforeEach(() => {
  mocks.autoCategorizeOnImport.mockReset();
  mocks.setAutoCategorizeOnImport.mockReset();
  mocks.autoCategorizeOnImport.mockResolvedValue(ok(true));
  mocks.setAutoCategorizeOnImport.mockResolvedValue(ok(null));
});

describe("AutoCategorizeOnImportCard", () => {
  // personal-cfo-i93n3.28 slice 8: the text column may shrink and breaks long words on a phone (it pushed the switch past the edge).
  it("lets the text beside the switch shrink and break long words on a phone", async () => {
    renderWithClient(<AutoCategorizeOnImportCard />);
    const label = await screen.findByText("Auto-categorize imported transactions");
    expect(label.parentElement).toHaveClass("compact:min-w-0", "compact:[overflow-wrap:anywhere]");
  });

  it("reflects the stored on state", async () => {
    renderWithClient(<AutoCategorizeOnImportCard />);
    await waitFor(() =>
      expect(
        screen.getByRole("switch", { name: /auto-categorize imported/i }),
      ).toBeChecked(),
    );
  });

  it("reflects the stored off state", async () => {
    mocks.autoCategorizeOnImport.mockResolvedValue(ok(false));
    renderWithClient(<AutoCategorizeOnImportCard />);
    await waitFor(() =>
      expect(
        screen.getByRole("switch", { name: /auto-categorize imported/i }),
      ).not.toBeChecked(),
    );
  });

  it("persists a toggle off via the setter", async () => {
    renderWithClient(<AutoCategorizeOnImportCard />);
    const toggle = await screen.findByRole("switch", {
      name: /auto-categorize imported/i,
    });
    await waitFor(() => expect(toggle).toBeChecked());

    fireEvent.click(toggle);
    await waitFor(() =>
      expect(mocks.setAutoCategorizeOnImport).toHaveBeenCalledWith(false),
    );
  });
});
