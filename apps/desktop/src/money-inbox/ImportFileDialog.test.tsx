import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { AccountViewDto } from "@/bindings";
import { ImportFileDialog } from "./ImportFileDialog";

const mocks = vi.hoisted(() => ({
  importBatch: vi.fn(),
  importPreviewColumns: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    importBatch: mocks.importBatch,
    importPreviewColumns: mocks.importPreviewColumns,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

function account(over: Partial<AccountViewDto> = {}): AccountViewDto {
  return {
    id: "0190a000-0000-7000-8000-000000000001",
    name: "Checking",
    cashflow_role: "liquid_cash",
    subtype: null,
    active: true,
    balance: { minor_units: 125_000, currency: "USD" },
    notes: null,
    linked_account_id: null,
    linked_account_name: null,
    ...over,
  };
}

/// A `File` whose `arrayBuffer()` resolves in jsdom (which doesn't implement it).
function csvFile(): File {
  const text = "Date,Description,Amount\n2026-06-20,Coffee,-12.99\n";
  const file = new File([text], "statement.csv", { type: "text/csv" });
  Object.defineProperty(file, "arrayBuffer", {
    value: () => Promise.resolve(new TextEncoder().encode(text).buffer),
  });
  return file;
}

beforeEach(() => {
  vi.clearAllMocks();
  // Default: no mappable columns (auto-detect); the mapping test overrides this.
  mocks.importPreviewColumns.mockResolvedValue(ok([]));
});

it("imports a chosen file into the selected account and shows the outcome", async () => {
  mocks.importBatch.mockResolvedValue(
    ok({
      source_batch_id: "0190b000-0000-7000-8000-000000000001",
      status: "partially_committed",
      staged: 3,
      committed: 2,
      flagged: 1,
      auto_categorized: 0,
    }),
  );

  const { container } = renderWithClient(
    <ImportFileDialog accounts={[account()]} onClose={vi.fn()} />,
  );

  // Pick a file via the hidden input.
  const input = container.querySelector(
    'input[type="file"]',
  ) as HTMLInputElement;
  Object.defineProperty(input, "files", { value: [csvFile()] });
  fireEvent.change(input);
  expect(await screen.findByText("statement.csv")).toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: /^import$/i }));

  await waitFor(() => expect(mocks.importBatch).toHaveBeenCalledTimes(1));
  expect(mocks.importBatch).toHaveBeenCalledWith(
    expect.objectContaining({
      filename: "statement.csv",
      target_account_id: "0190a000-0000-7000-8000-000000000001",
      default_currency: "USD",
      data: expect.any(Array),
    }),
  );

  // The summary reflects the batch result + points at the Money Inbox.
  expect(
    await screen.findByText(/Imported 2 transactions\. 1 needs review/i),
  ).toBeInTheDocument();
});

it("surfaces how many rows were auto-categorized on import (5n4.2)", async () => {
  mocks.importBatch.mockResolvedValue(
    ok({
      source_batch_id: "0190b000-0000-7000-8000-000000000002",
      status: "committed",
      staged: 4,
      committed: 4,
      flagged: 0,
      auto_categorized: 3,
    }),
  );

  const { container } = renderWithClient(
    <ImportFileDialog accounts={[account()]} onClose={vi.fn()} />,
  );
  const input = container.querySelector(
    'input[type="file"]',
  ) as HTMLInputElement;
  Object.defineProperty(input, "files", { value: [csvFile()] });
  fireEvent.change(input);
  fireEvent.click(screen.getByRole("button", { name: /^import$/i }));

  expect(
    await screen.findByText(/Auto-categorized 3 from merchant memory/i),
  ).toBeInTheDocument();
});

it("lets the user remap a column and passes the mapping to import (4d8.24.1.2)", async () => {
  mocks.importPreviewColumns.mockResolvedValue(
    ok(["Txn Date", "Details", "Value"]),
  );
  mocks.importBatch.mockResolvedValue(
    ok({
      source_batch_id: "0190b000-0000-7000-8000-000000000003",
      status: "committed",
      staged: 1,
      committed: 1,
      flagged: 0,
      auto_categorized: 0,
    }),
  );

  const { container } = renderWithClient(
    <ImportFileDialog accounts={[account()]} onClose={vi.fn()} />,
  );
  const input = container.querySelector(
    'input[type="file"]',
  ) as HTMLInputElement;
  Object.defineProperty(input, "files", { value: [csvFile()] });
  fireEvent.change(input);

  // The mapping section appears once the file's headers load; expand it and remap.
  const toggle = await screen.findByRole("button", { name: /column mapping/i });
  fireEvent.click(toggle);
  fireEvent.change(await screen.findByLabelText("Map Amount"), {
    target: { value: "Value" },
  });
  fireEvent.click(screen.getByRole("button", { name: /^import$/i }));

  await waitFor(() => expect(mocks.importBatch).toHaveBeenCalledTimes(1));
  expect(mocks.importBatch).toHaveBeenCalledWith(
    expect.objectContaining({
      column_mapping: expect.objectContaining({ amount: "Value", date: null }),
    }),
  );
});

it("imports with no mapping (auto-detect) when the user overrides nothing", async () => {
  mocks.importPreviewColumns.mockResolvedValue(ok(["Date", "Amount"]));
  mocks.importBatch.mockResolvedValue(
    ok({
      source_batch_id: "0190b000-0000-7000-8000-000000000004",
      status: "committed",
      staged: 1,
      committed: 1,
      flagged: 0,
      auto_categorized: 0,
    }),
  );

  const { container } = renderWithClient(
    <ImportFileDialog accounts={[account()]} onClose={vi.fn()} />,
  );
  const input = container.querySelector(
    'input[type="file"]',
  ) as HTMLInputElement;
  Object.defineProperty(input, "files", { value: [csvFile()] });
  fireEvent.change(input);
  await screen.findByText("statement.csv");
  fireEvent.click(screen.getByRole("button", { name: /^import$/i }));

  await waitFor(() => expect(mocks.importBatch).toHaveBeenCalledTimes(1));
  expect(mocks.importBatch).toHaveBeenCalledWith(
    expect.objectContaining({ column_mapping: null }),
  );
});

it("reports an already-imported file without claiming new rows", async () => {
  mocks.importBatch.mockResolvedValue(
    ok({
      source_batch_id: null,
      status: "already_imported",
      staged: 0,
      committed: 0,
      flagged: 0,
      auto_categorized: 0,
    }),
  );

  const { container } = renderWithClient(
    <ImportFileDialog accounts={[account()]} onClose={vi.fn()} />,
  );
  const input = container.querySelector(
    'input[type="file"]',
  ) as HTMLInputElement;
  Object.defineProperty(input, "files", { value: [csvFile()] });
  fireEvent.change(input);
  fireEvent.click(screen.getByRole("button", { name: /^import$/i }));

  expect(
    await screen.findByText(/already imported/i),
  ).toBeInTheDocument();
});

it("explains how to export from common banks, on demand (rfsc)", async () => {
  renderWithClient(<ImportFileDialog accounts={[account()]} onClose={() => {}} />);
  const summary = await screen.findByText(/how do i export from my bank/i);
  fireEvent.click(summary);
  // On demand: the disclosure actually opened.
  expect(summary.closest("details")).toHaveAttribute("open");
  const picker = screen.getByLabelText(/where is the money/i);
  expect(picker).toBeInTheDocument();
  fireEvent.change(picker, { target: { value: "american-express" } });
  expect(screen.getByText(/statements & activity/i)).toBeInTheDocument();
});
