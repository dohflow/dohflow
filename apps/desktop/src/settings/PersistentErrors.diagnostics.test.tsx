/**
 * personal-cfo-4d8.39.2 (slice b, part 3): Diagnostics puts each error, its full message and its one
 * recovery action in the section's persistent slot. The action is rendered once, in the slot while
 * the error shows, and nothing runs because the section was collapsed.
 */

import { fireEvent, screen, waitFor, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import { DiagnosticsCard } from "./DiagnosticsCard";
import { SettingsSection } from "./SettingsSection";

const mocks = vi.hoisted(() => ({
  save: vi.fn(),
  diagnosticsPreview: vi.fn(),
  diagnosticsSave: vi.fn(),
  diagnosticsDiscard: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: mocks.save }));
vi.mock("@/bindings", () => ({ commands: mocks }));
vi.mock("@/vault/useVault", () => ({
  describeIpcError: (error: unknown) =>
    error === "VaultLocked" ? "The vault is locked." : "Something went wrong.",
}));

const CHOSEN_PATH = "/Users/jane/Private Folder/dohflow-diagnostics-2026-09-28.json";
const previewOf = (records = 2) => ({
  status: "ok" as const,
  data: {
    snapshot_id: 7,
    text: `{ "records_retained": ${records} }`,
    records,
    dropped: 0,
    rejected: 0,
    suggested_file_name: "dohflow-diagnostics-2026-09-28.json",
  },
});

beforeEach(() => {
  vi.clearAllMocks();
  mocks.diagnosticsDiscard.mockResolvedValue({ status: "ok", data: null });
  mocks.diagnosticsPreview.mockResolvedValue(previewOf());
  mocks.save.mockResolvedValue(CHOSEN_PATH);
});

const render = () =>
  renderWithClient(
    <SettingsSection title="Diagnostics" span="full">
      <DiagnosticsCard />
    </SettingsSection>,
  );
const section = () => document.querySelector(`[data-settings-section="Diagnostics"]`) as HTMLElement;
const slot = () => section().querySelector("[data-settings-persistent]") as HTMLElement;
const everyButton = (name: string | RegExp) =>
  within(section()).queryAllByRole("button", { name, hidden: true });
/// Exactly one such button in the section, and it is in the slot.
function expectOnlyInSlot(name: string | RegExp) {
  const found = everyButton(name);
  expect(found).toHaveLength(1);
  expect(slot().contains(found[0]!)).toBe(true);
}
const collapse = () => fireEvent.click(screen.getByRole("button", { name: "Diagnostics" }));
const openPreview = async () => {
  fireEvent.click(screen.getByRole("button", { name: "Preview diagnostics" }));
  await screen.findByLabelText(/exactly what will be saved/i);
};

describe("Diagnostics", () => {
  it("a failed preview keeps its message and its one Preview diagnostics while collapsed", async () => {
    mocks.diagnosticsPreview.mockResolvedValueOnce({ status: "error", error: "VaultLocked" });
    render();
    fireEvent.click(screen.getByRole("button", { name: "Preview diagnostics" }));
    const alert = await screen.findByText("The vault is locked.");
    expect(slot().contains(alert)).toBe(true);
    expectOnlyInSlot("Preview diagnostics");

    collapse();
    expect(screen.getByText("The vault is locked.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Preview diagnostics" })).toBeVisible();
    expect(mocks.diagnosticsPreview).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: "Preview diagnostics" }));
    await waitFor(() => expect(mocks.diagnosticsPreview).toHaveBeenCalledTimes(2));
  });

  it.each([
    ["permission_denied", "DohFlow doesn't have permission to save there. Choose another folder. Nothing was saved."],
    ["disk_full", "There isn't enough disk space to save diagnostics. Nothing was saved."],
    ["invalid_destination", "Choose a .json file in a folder that exists. Nothing was saved."],
    ["failed", "Diagnostics couldn't be saved. Nothing was saved."],
  ])(
    "a %s save keeps its full message and its one Save in the slot, and Save picks another destination",
    async (result, message) => {
      mocks.diagnosticsSave.mockResolvedValueOnce({ status: "ok", data: result });
      render();
      await openPreview();
      fireEvent.click(screen.getByRole("button", { name: "Save…" }));
      const alert = await screen.findByText(message);
      expect(slot().contains(alert)).toBe(true);
      expectOnlyInSlot("Save…");

      collapse();
      expect(screen.getByText(message)).toBeVisible();
      expect(screen.getByRole("button", { name: "Save…" })).toBeVisible();
      // Collapsing saved nothing and discarded nothing.
      expect(mocks.diagnosticsSave).toHaveBeenCalledTimes(1);
      expect(mocks.diagnosticsDiscard).not.toHaveBeenCalled();

      mocks.diagnosticsSave.mockResolvedValueOnce({ status: "ok", data: "saved" });
      fireEvent.click(screen.getByRole("button", { name: "Save…" }));
      await waitFor(() => expect(mocks.diagnosticsSave).toHaveBeenCalledTimes(2));
      expect(mocks.save).toHaveBeenCalledTimes(2);
    },
  );

  it("an expired preview keeps its message and offers its one Preview diagnostics in the slot", async () => {
    mocks.diagnosticsSave.mockResolvedValueOnce({ status: "ok", data: "preview_expired" });
    render();
    await openPreview();
    fireEvent.click(screen.getByRole("button", { name: "Save…" }));
    const alert = await screen.findByText("This preview has expired. Preview diagnostics again.");
    expect(slot().contains(alert)).toBe(true);
    expectOnlyInSlot("Preview diagnostics");
    expect(everyButton("Save…")).toHaveLength(0);
  });

  it("focus stays on Save when a refused save moves it into the slot", async () => {
    mocks.diagnosticsSave.mockResolvedValueOnce({ status: "ok", data: "failed" });
    render();
    await openPreview();
    const save = screen.getByRole("button", { name: "Save…" });
    save.focus();
    fireEvent.click(save);
    await screen.findByText("Diagnostics couldn't be saved. Nothing was saved.");
    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByRole("button", { name: "Save…" })),
    );
    expect(slot().contains(document.activeElement)).toBe(true);
  });

  it("focus stays on Preview diagnostics when a failed preview moves it into the slot", async () => {
    mocks.diagnosticsPreview.mockResolvedValueOnce({ status: "error", error: "VaultLocked" });
    render();
    const preview = screen.getByRole("button", { name: "Preview diagnostics" });
    preview.focus();
    fireEvent.click(preview);
    await screen.findByText("The vault is locked.");
    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByRole("button", { name: "Preview diagnostics" })),
    );
    expect(slot().contains(document.activeElement)).toBe(true);
  });

  it("a status note (cancelled save) stays in the body, not the slot", async () => {
    mocks.save.mockResolvedValueOnce(null);
    render();
    await openPreview();
    fireEvent.click(screen.getByRole("button", { name: "Save…" }));
    const status = await screen.findByText("Not saved.");
    expect(slot().contains(status)).toBe(false);
    expect(slot()).toBeEmptyDOMElement();
  });
});
