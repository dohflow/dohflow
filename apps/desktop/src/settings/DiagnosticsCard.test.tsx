import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";

import { DiagnosticsCard } from "./DiagnosticsCard";

const mocks = vi.hoisted(() => ({
  save: vi.fn(),
  diagnosticsPreview: vi.fn(),
  diagnosticsSave: vi.fn(),
  diagnosticsDiscard: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: mocks.save }));
vi.mock("@/bindings", () => ({
  commands: {
    diagnosticsPreview: mocks.diagnosticsPreview,
    diagnosticsSave: mocks.diagnosticsSave,
    diagnosticsDiscard: mocks.diagnosticsDiscard,
  },
}));
vi.mock("@/vault/useVault", () => ({
  describeIpcError: (e: unknown) =>
    e === "VaultLocked" ? "The vault is locked." : "Something went wrong.",
}));

const CHOSEN_PATH = "/Users/jane/Private Folder/dohflow-diagnostics-2026-09-28.json";

function previewOf(records: number, extra: Partial<Record<"dropped" | "rejected", number>> = {}) {
  return {
    status: "ok" as const,
    data: {
      snapshot_id: 7,
      text: `{\n  "format": "dohflow-diagnostics",\n  "records_retained": ${records}\n}\n`,
      records,
      dropped: extra.dropped ?? 0,
      rejected: extra.rejected ?? 0,
      suggested_file_name: "dohflow-diagnostics-2026-09-28.json",
    },
  };
}

async function openPreview() {
  fireEvent.click(screen.getByRole("button", { name: /preview diagnostics/i }));
  return screen.findByLabelText(/exactly what will be saved/i);
}

function noteText(): string {
  return (screen.queryByRole("status") ?? screen.queryByRole("alert"))?.textContent ?? "";
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.diagnosticsDiscard.mockResolvedValue({ status: "ok", data: null });
});

test("idle: explains the policy and offers only a preview", () => {
  render(<DiagnosticsCard />);
  expect(screen.getByText(/never leaves this device unless you save it/i)).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /save/i })).not.toBeInTheDocument();
  expect(mocks.diagnosticsPreview).not.toHaveBeenCalled();
});

test("ready: shows the exact bundle text and counts, and moves focus to it", async () => {
  mocks.diagnosticsPreview.mockResolvedValue(previewOf(3, { dropped: 2, rejected: 1 }));
  render(<DiagnosticsCard />);
  const region = await openPreview();
  expect(region).toHaveTextContent('"records_retained": 3');
  expect(document.activeElement).toBe(region);
  expect(screen.getByText(/3 records · 2 older records dropped · 1 rejected/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: /save/i })).toBeEnabled();
});

test("empty: says nothing was captured and cannot save", async () => {
  mocks.diagnosticsPreview.mockResolvedValue(previewOf(0));
  render(<DiagnosticsCard />);
  await openPreview();
  expect(screen.getByText(/nothing has been captured this session/i)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: /save/i })).toBeDisabled();
});

test("preview failure: a safe alert, no preview", async () => {
  mocks.diagnosticsPreview.mockResolvedValue({ status: "error", error: "VaultLocked" });
  render(<DiagnosticsCard />);
  fireEvent.click(screen.getByRole("button", { name: /preview diagnostics/i }));
  expect(await screen.findByRole("alert")).toHaveTextContent("The vault is locked.");
  expect(screen.queryByLabelText(/exactly what will be saved/i)).not.toBeInTheDocument();
});

test("cancel: closing the Save dialog writes nothing and keeps the preview", async () => {
  mocks.diagnosticsPreview.mockResolvedValue(previewOf(1));
  mocks.save.mockResolvedValue(null);
  render(<DiagnosticsCard />);
  await openPreview();
  fireEvent.click(screen.getByRole("button", { name: /save/i }));
  expect(await screen.findByRole("status")).toHaveTextContent("Not saved.");
  expect(mocks.diagnosticsSave).not.toHaveBeenCalled();
  expect(screen.getByLabelText(/exactly what will be saved/i)).toBeInTheDocument();
  expect(mocks.save.mock.calls[0]![0]).toMatchObject({
    defaultPath: "dohflow-diagnostics-2026-09-28.json",
    filters: [{ extensions: ["json"] }],
  });
});

test("saving then success: busy while writing, then saved, closed, focus returned", async () => {
  mocks.diagnosticsPreview.mockResolvedValue(previewOf(2));
  mocks.save.mockResolvedValue(CHOSEN_PATH);
  let finish!: (value: unknown) => void;
  mocks.diagnosticsSave.mockReturnValue(new Promise((resolve) => (finish = resolve)));
  render(<DiagnosticsCard />);
  await openPreview();
  fireEvent.click(screen.getByRole("button", { name: /save/i }));

  const busy = await screen.findByRole("button", { name: /saving/i });
  expect(busy).toBeDisabled();
  expect(busy).toHaveAttribute("aria-busy", "true");
  expect(screen.getByRole("button", { name: /close/i })).toBeDisabled();
  expect(mocks.diagnosticsSave).toHaveBeenCalledWith(7, CHOSEN_PATH);

  await act(async () => finish({ status: "ok", data: "saved" }));
  expect(await screen.findByRole("status")).toHaveTextContent("Diagnostics saved.");
  expect(screen.queryByLabelText(/exactly what will be saved/i)).not.toBeInTheDocument();
  expect(document.activeElement).toBe(
    screen.getByRole("button", { name: /preview diagnostics/i }),
  );
  expect(noteText()).not.toContain("jane");
});

test.each([
  ["permission_denied", /permission to save there/i],
  ["disk_full", /enough disk space/i],
  ["invalid_destination", /\.json file in a folder that exists/i],
  ["failed", /couldn't be saved/i],
])("a %s save shows safe copy and keeps the preview for another try", async (result, copy) => {
  mocks.diagnosticsPreview.mockResolvedValue(previewOf(1));
  mocks.save.mockResolvedValue(CHOSEN_PATH);
  mocks.diagnosticsSave.mockResolvedValue({ status: "ok", data: result });
  render(<DiagnosticsCard />);
  await openPreview();
  fireEvent.click(screen.getByRole("button", { name: /save/i }));
  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent(copy);
  expect(alert.textContent).not.toContain("jane");
  expect(alert.textContent).not.toContain("Private Folder");
  expect(screen.getByLabelText(/exactly what will be saved/i)).toBeInTheDocument();
});

test.each([
  ["an expired preview", { status: "ok", data: "preview_expired" }],
  ["a vault lock during the save", { status: "error", error: "VaultLocked" }],
])("%s closes the preview with the expired message", async (_label, response) => {
  mocks.diagnosticsPreview.mockResolvedValue(previewOf(1));
  mocks.save.mockResolvedValue(CHOSEN_PATH);
  mocks.diagnosticsSave.mockResolvedValue(response);
  render(<DiagnosticsCard />);
  await openPreview();
  fireEvent.click(screen.getByRole("button", { name: /save/i }));
  expect(await screen.findByRole("alert")).toHaveTextContent(/preview has expired/i);
  expect(screen.queryByLabelText(/exactly what will be saved/i)).not.toBeInTheDocument();
});

test("keyboard: Escape closes the preview, discards it, and returns focus", async () => {
  mocks.diagnosticsPreview.mockResolvedValue(previewOf(1));
  render(<DiagnosticsCard />);
  const region = await openPreview();
  fireEvent.keyDown(region, { key: "Escape" });
  await waitFor(() =>
    expect(screen.queryByLabelText(/exactly what will be saved/i)).not.toBeInTheDocument(),
  );
  expect(mocks.diagnosticsDiscard).toHaveBeenCalledWith(7);
  expect(document.activeElement).toBe(
    screen.getByRole("button", { name: /preview diagnostics/i }),
  );
});

test("Close discards the pending preview", async () => {
  mocks.diagnosticsPreview.mockResolvedValue(previewOf(1));
  render(<DiagnosticsCard />);
  await openPreview();
  fireEvent.click(screen.getByRole("button", { name: /close/i }));
  expect(mocks.diagnosticsDiscard).toHaveBeenCalledWith(7);
});

test("leaving Settings (lock/switch unmounts it) discards a pending preview", async () => {
  mocks.diagnosticsPreview.mockResolvedValue(previewOf(1));
  const { unmount } = render(<DiagnosticsCard />);
  await openPreview();
  unmount();
  expect(mocks.diagnosticsDiscard).toHaveBeenCalledWith(7);
});
