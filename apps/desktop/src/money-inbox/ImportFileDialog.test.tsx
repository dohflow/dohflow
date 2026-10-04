import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { AccountViewDto, SourcePresetDto } from "@/bindings";
import { ImportFileDialog } from "./ImportFileDialog";

const mocks = vi.hoisted(() => ({
  importBatch: vi.fn(),
  importPreviewColumns: vi.fn(),
  importPreviewAccounts: vi.fn(),
  listSourcePresets: vi.fn(),
  createAccount: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    importBatch: mocks.importBatch,
    importPreviewColumns: mocks.importPreviewColumns,
    importPreviewAccounts: mocks.importPreviewAccounts,
    listSourcePresets: mocks.listSourcePresets,
    createAccount: mocks.createAccount,
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

/// A minimal preset shape, standing in for `SourcePresetDto` (personal-cfo-gvidg).
function preset(over: Partial<SourcePresetDto> = {}): SourcePresetDto {
  return {
    id: "ynab",
    display_name: "YNAB",
    source_app_url: "https://www.ynab.com",
    column_mapping: {
      date: "Date",
      description: "Payee",
      amount: null,
      debit: "Outflow",
      credit: "Inflow",
      account: "Account",
      category: "Category",
      category_group: "Category Group",
      currency: null,
      memo: "Memo",
    },
    help_slug: "move-from-ynab",
    // Matches the real YNAB preset's own default (personal-cfo-gvidg review
    // finding F1, PR #15): a guide slug existing does not mean the page is
    // published. Tests that specifically cover the guide link opt in with
    // `preset({ help_published: true })`.
    help_published: false,
    importer_id: "ynab-register",
    ...over,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  // Default: no mappable columns (auto-detect); the mapping test overrides this.
  mocks.importPreviewColumns.mockResolvedValue(ok([]));
  // Default: no account column — the single "Import into" path (personal-cfo-tulv
  // tests override this).
  mocks.importPreviewAccounts.mockResolvedValue(ok([]));
  // Default: no presets registered — the picker stays hidden, matching every
  // test written before personal-cfo-gvidg existed. Preset-specific tests
  // override this.
  mocks.listSourcePresets.mockResolvedValue([]);
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
      skipped_rows: 0,
      warnings: [],
      skipped_unmapped: 0,
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
      skipped_rows: 0,
      warnings: [],
      skipped_unmapped: 0,
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

/// Pick the CSV fixture and press Import (personal-cfo-pxi.10 helpers).
async function importWith(result: unknown) {
  mocks.importBatch.mockResolvedValue(ok(result));
  const { container } = renderWithClient(
    <ImportFileDialog accounts={[account()]} onClose={vi.fn()} />,
  );
  const input = container.querySelector(
    'input[type="file"]',
  ) as HTMLInputElement;
  Object.defineProperty(input, "files", { value: [csvFile()] });
  fireEvent.change(input);
  fireEvent.click(screen.getByRole("button", { name: /^import$/i }));
  await screen.findByText(/Imported \d+ transaction/i);
}

it("shows no skipped-rows note when every row was imported (pxi.10)", async () => {
  await importWith({
    source_batch_id: "0190b000-0000-7000-8000-000000000010",
    status: "committed",
    staged: 2,
    committed: 2,
    flagged: 0,
    auto_categorized: 0,
    skipped_rows: 0,
    // A note on an imported row is not a skip and adds nothing here.
    warnings: [
      { row: 1, message: "ambiguous date (assumed US M/D/Y)", skipped: false },
    ],
  });
  expect(
    screen.getByText("Imported 2 transactions. All clean — nothing to review."),
  ).toBeInTheDocument();
  expect(screen.queryByText(/skipped/i)).not.toBeInTheDocument();
  expect(screen.queryByText(/ambiguous/i)).not.toBeInTheDocument();
  expect(screen.queryByRole("list", { name: /skipped rows/i })).not.toBeInTheDocument();
});

it("says how many rows were skipped and why, by row (pxi.10)", async () => {
  await importWith({
    source_batch_id: "0190b000-0000-7000-8000-000000000011",
    status: "committed",
    staged: 3,
    committed: 3,
    flagged: 0,
    auto_categorized: 0,
    skipped_rows: 2,
    warnings: [
      { row: 2, message: "unparseable / missing amount", skipped: true },
      { row: 5, message: "unparseable date", skipped: true },
      { row: 1, message: "ambiguous date (assumed US M/D/Y)", skipped: false },
    ],
  });
  expect(screen.getByText("Imported 3 transactions.")).toBeInTheDocument();
  expect(
    screen.getByText(/2 rows skipped — nothing was imported for them\./),
  ).toBeInTheDocument();
  const reasons = screen.getByRole("list", { name: /skipped rows/i });
  expect(
    Array.from(reasons.querySelectorAll("li")).map((li) => li.textContent),
  ).toEqual(["Row 2: unparseable / missing amount", "Row 5: unparseable date"]);
});

it("counts every skipped row even when the reason list is cut short (pxi.10)", async () => {
  await importWith({
    source_batch_id: "0190b000-0000-7000-8000-000000000012",
    status: "committed",
    staged: 1,
    committed: 1,
    flagged: 0,
    auto_categorized: 0,
    skipped_rows: 23,
    warnings: Array.from({ length: 20 }, (_, i) => ({
      row: i + 2,
      message: "unparseable / missing amount",
      skipped: true,
    })),
  });
  expect(screen.getByText(/23 rows skipped/)).toBeInTheDocument();
  const items = screen
    .getByRole("list", { name: /skipped rows/i })
    .querySelectorAll("li");
  expect(items).toHaveLength(21);
  expect(items[20]).toHaveTextContent("…and 3 more");
});

it("names one skipped row in the singular (pxi.10)", async () => {
  await importWith({
    source_batch_id: "0190b000-0000-7000-8000-000000000013",
    status: "committed",
    staged: 1,
    committed: 1,
    flagged: 0,
    auto_categorized: 0,
    skipped_rows: 1,
    warnings: [{ row: null, message: "unreadable row", skipped: true }],
  });
  expect(
    screen.getByText(/1 row skipped — nothing was imported for it\./),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("list", { name: /skipped rows/i }).querySelector("li"),
  ).toHaveTextContent(/^unreadable row$/);
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
      skipped_rows: 0,
      warnings: [],
      skipped_unmapped: 0,
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
      skipped_rows: 0,
      warnings: [],
      skipped_unmapped: 0,
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
      skipped_rows: 0,
      warnings: [],
      skipped_unmapped: 0,
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

// --- personal-cfo-gvidg: "Import from <app>" preset picker ---

it("offers no preset picker when the registry is empty (unchanged default behavior)", async () => {
  renderWithClient(<ImportFileDialog accounts={[account()]} onClose={() => {}} />);
  await waitFor(() => expect(mocks.listSourcePresets).toHaveBeenCalled());
  expect(screen.queryByLabelText("Import from")).not.toBeInTheDocument();
});

it("skips the mapping step when a chosen preset's columns fully match the file", async () => {
  mocks.listSourcePresets.mockResolvedValue([preset()]);
  mocks.importPreviewColumns.mockResolvedValue(
    ok(["Date", "Payee", "Category Group", "Category", "Memo", "Outflow", "Inflow", "Account"]),
  );
  mocks.importBatch.mockResolvedValue(
    ok({
      source_batch_id: "0190b000-0000-7000-8000-000000000005",
      status: "committed",
      staged: 1,
      committed: 1,
      flagged: 0,
      auto_categorized: 0,
      skipped_rows: 0,
      warnings: [],
      skipped_unmapped: 0,
    }),
  );

  const { container } = renderWithClient(
    <ImportFileDialog accounts={[account()]} onClose={vi.fn()} />,
  );
  const presetPicker = await screen.findByLabelText("Import from");
  fireEvent.change(presetPicker, { target: { value: "ynab" } });

  const input = container.querySelector(
    'input[type="file"]',
  ) as HTMLInputElement;
  Object.defineProperty(input, "files", { value: [csvFile()] });
  fireEvent.change(input);
  await screen.findByText("statement.csv");

  // Every column the preset declared was found — nothing to review, the
  // mapping section stays collapsed (no "Map Amount" etc. visible).
  expect(screen.queryByLabelText("Map Date (posted)")).not.toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: /^import$/i }));
  await waitFor(() => expect(mocks.importBatch).toHaveBeenCalledTimes(1));
  expect(mocks.importBatch).toHaveBeenCalledWith(
    expect.objectContaining({ preset_id: "ynab" }),
  );
});

// personal-cfo-gvidg review finding F1 (PR #15): a preset's help_slug
// existing does not mean the page is published — the app must never offer a
// user-reachable link to a page marked draft on the site.
it("never shows a guide link for a preset whose guide is not published", async () => {
  mocks.listSourcePresets.mockResolvedValue([preset({ help_published: false })]);
  const { container } = renderWithClient(
    <ImportFileDialog accounts={[account()]} onClose={vi.fn()} />,
  );
  const presetPicker = await screen.findByLabelText("Import from");
  fireEvent.change(presetPicker, { target: { value: "ynab" } });

  const input = container.querySelector(
    'input[type="file"]',
  ) as HTMLInputElement;
  Object.defineProperty(input, "files", { value: [csvFile()] });
  fireEvent.change(input);
  await screen.findByText("statement.csv");

  expect(
    screen.queryByRole("button", { name: /full guide/i }),
  ).not.toBeInTheDocument();
});

it("shows the guide link once a preset's guide is published", async () => {
  mocks.listSourcePresets.mockResolvedValue([preset({ help_published: true })]);
  renderWithClient(<ImportFileDialog accounts={[account()]} onClose={vi.fn()} />);
  const presetPicker = await screen.findByLabelText("Import from");
  fireEvent.change(presetPicker, { target: { value: "ynab" } });

  expect(
    await screen.findByRole("button", { name: /full guide: moving from ynab/i }),
  ).toBeInTheDocument();
});

it("pre-fills the mapping and leaves a gap visible when a preset's columns partially match", async () => {
  mocks.listSourcePresets.mockResolvedValue([preset()]);
  // "Account" is missing from this file — a real MapField gap (unlike
  // category_group, which is preset-only and deliberately not part of the
  // full-match check — see presetMapEntries). Everything else matches.
  mocks.importPreviewColumns.mockResolvedValue(
    ok(["Date", "Payee", "Category Group", "Category", "Memo", "Outflow", "Inflow"]),
  );
  mocks.importBatch.mockResolvedValue(
    ok({
      source_batch_id: "0190b000-0000-7000-8000-000000000006",
      status: "committed",
      staged: 1,
      committed: 1,
      flagged: 0,
      auto_categorized: 0,
      skipped_rows: 0,
      warnings: [],
      skipped_unmapped: 0,
    }),
  );

  const { container } = renderWithClient(
    <ImportFileDialog accounts={[account()]} onClose={vi.fn()} />,
  );
  const presetPicker = await screen.findByLabelText("Import from");
  fireEvent.change(presetPicker, { target: { value: "ynab" } });

  const input = container.querySelector(
    'input[type="file"]',
  ) as HTMLInputElement;
  Object.defineProperty(input, "files", { value: [csvFile()] });
  fireEvent.change(input);
  await screen.findByText("statement.csv");

  // A gap exists — the mapping section auto-expands rather than hiding a
  // preset guess the user should see.
  const dateField = (await screen.findByLabelText(
    "Map Date (posted)",
  )) as HTMLSelectElement;
  expect(dateField.value).toBe("Date");
  const categoryField = screen.getByLabelText(
    "Map Category",
  ) as HTMLSelectElement;
  expect(categoryField.value).toBe("Category");

  fireEvent.click(screen.getByRole("button", { name: /^import$/i }));
  await waitFor(() => expect(mocks.importBatch).toHaveBeenCalledTimes(1));
  expect(mocks.importBatch).toHaveBeenCalledWith(
    expect.objectContaining({
      preset_id: "ynab",
      column_mapping: expect.objectContaining({
        date: "Date",
        category: "Category",
        account: null,
      }),
    }),
  );
});

// --- personal-cfo-tulv: a file spanning several accounts (YNAB) ---

const CHECKING_ID = "0190a000-0000-7000-8000-000000000001";
const CARD_ID = "0190a000-0000-7000-8000-000000000002";
const YNAB_HEADERS = [
  "Account",
  "Flag",
  "Date",
  "Payee",
  "Category Group/Category",
  "Category Group",
  "Category",
  "Memo",
  "Outflow",
  "Inflow",
  "Cleared",
];

function ynabAccounts(): AccountViewDto[] {
  return [
    account(),
    account({ id: CARD_ID, name: "Rewards Card", cashflow_role: "credit_facility" }),
  ];
}

/// Choose "Import from YNAB", then pick a file whose YNAB accounts are `labels`.
async function pickYnabFile(accounts: AccountViewDto[], labels: string[]) {
  mocks.listSourcePresets.mockResolvedValue([preset()]);
  mocks.importPreviewColumns.mockResolvedValue(ok(YNAB_HEADERS));
  mocks.importPreviewAccounts.mockResolvedValue(ok(labels));
  const view = renderWithClient(<ImportFileDialog accounts={accounts} onClose={vi.fn()} />);
  fireEvent.change(await screen.findByLabelText("Import from"), {
    target: { value: "ynab" },
  });
  const input = view.container.querySelector('input[type="file"]') as HTMLInputElement;
  Object.defineProperty(input, "files", { value: [csvFile()] });
  fireEvent.change(input);
  await screen.findByRole("region", { name: /accounts in this file/i });
  return view;
}

it("reads the file through the preset's own importer", async () => {
  await pickYnabFile(ynabAccounts(), ["Checking", "Credit Card"]);
  expect(mocks.importPreviewColumns).toHaveBeenCalledWith(
    expect.any(Array),
    "statement.csv",
    "ynab-register",
  );
  expect(mocks.importPreviewAccounts).toHaveBeenCalledWith(
    expect.any(Array),
    "statement.csv",
    null,
    "ynab",
    null,
  );
});

it("asks where each account in the file goes and imports with that map", async () => {
  mocks.importBatch.mockResolvedValue(
    ok({
      source_batch_id: "0190b000-0000-7000-8000-000000000020",
      status: "partially_committed",
      staged: 10,
      committed: 9,
      flagged: 1,
      auto_categorized: 0,
      skipped_rows: 0,
      warnings: [
        {
          row: null,
          message:
            "2 rows are each a transfer between YNAB accounts — each side was imported as its own transaction, not linked as a transfer.",
          skipped: false,
        },
        { row: 4, message: "ambiguous date (assumed US M/D/Y)", skipped: false },
      ],
      skipped_unmapped: 1,
    }),
  );
  await pickYnabFile(ynabAccounts(), ["Checking", "Credit Card", "Savings"]);

  // One "Import into" for the whole file would put every account's rows in one
  // account — it gives way to a choice per account.
  expect(screen.queryByLabelText("Import into")).not.toBeInTheDocument();
  // Pre-selected by matching name; the rest start as "Don't import".
  expect((screen.getByLabelText("Import Checking into") as HTMLSelectElement).value).toBe(
    CHECKING_ID,
  );
  const card = screen.getByLabelText("Import Credit Card into") as HTMLSelectElement;
  expect(card.value).toBe("");
  expect((screen.getByLabelText("Import Savings into") as HTMLSelectElement).value).toBe("");
  fireEvent.change(card, { target: { value: CARD_ID } });

  fireEvent.click(screen.getByRole("button", { name: /^import$/i }));
  await waitFor(() => expect(mocks.importBatch).toHaveBeenCalledTimes(1));
  expect(mocks.importBatch).toHaveBeenCalledWith(
    expect.objectContaining({
      preset_id: "ynab",
      target_account_id: null,
      account_map: [
        { source_account: "Checking", account_id: CHECKING_ID },
        { source_account: "Credit Card", account_id: CARD_ID },
        { source_account: "Savings", account_id: null },
      ],
      default_currency: "USD",
    }),
  );

  // The outcome names what was left out and the file's own notes; a note
  // about one row is still not listed (pxi.10).
  expect(await screen.findByText(/Imported 9 transactions/)).toBeInTheDocument();
  const notes = screen.getByRole("list", { name: /import notes/i });
  expect(Array.from(notes.querySelectorAll("li")).map((li) => li.textContent)).toEqual([
    "1 row from accounts you chose not to import was left out.",
    "2 rows are each a transfer between YNAB accounts — each side was imported as its own transaction, not linked as a transfer.",
  ]);
  expect(screen.queryByText(/ambiguous/i)).not.toBeInTheDocument();
  expect(screen.queryByText(/All clean/)).not.toBeInTheDocument();
});

it("won't import until at least one account in the file is mapped", async () => {
  await pickYnabFile([account({ name: "Main" })], ["Checking", "Savings"]);
  const importButton = screen.getByRole("button", { name: /^import$/i });
  expect(importButton).toBeDisabled();
  fireEvent.change(screen.getByLabelText("Import Savings into"), {
    target: { value: CHECKING_ID },
  });
  expect(importButton).toBeEnabled();
});

it("refuses to mix currencies in one import", async () => {
  await pickYnabFile(
    [
      account(),
      account({ id: CARD_ID, name: "Euro Card", balance: { minor_units: 0, currency: "EUR" } }),
    ],
    ["Checking", "Credit Card"],
  );
  fireEvent.change(screen.getByLabelText("Import Credit Card into"), {
    target: { value: CARD_ID },
  });
  expect(screen.getByRole("alert")).toHaveTextContent(/different currencies/i);
  expect(screen.getByRole("button", { name: /^import$/i })).toBeDisabled();
});

it("creates a new account from the mapping step and maps it", async () => {
  const NEW_ID = "0190a000-0000-7000-8000-000000000009";
  mocks.createAccount.mockResolvedValue(ok({ account_id: NEW_ID, mutation: {} }));
  await pickYnabFile(ynabAccounts(), ["Checking", "Savings"]);

  fireEvent.change(screen.getByLabelText("Import Savings into"), {
    target: { value: "__create_new__" },
  });
  const dialog = await screen.findByRole("dialog", { name: /new account for this import/i });
  expect(dialog).toHaveTextContent(/“Savings” from this file/);
  expect(dialog).not.toHaveTextContent(/refresh/i);
  fireEvent.click(screen.getByRole("button", { name: /^create$/i }));

  await waitFor(() =>
    expect(mocks.createAccount).toHaveBeenCalledWith(
      expect.objectContaining({ name: "Savings", currency: "USD", opening_balance: null }),
    ),
  );
  await waitFor(() =>
    expect((screen.getByLabelText("Import Savings into") as HTMLSelectElement).value).toBe(
      NEW_ID,
    ),
  );
});

it("keeps the single Import-into choice for a file with one account", async () => {
  mocks.importPreviewColumns.mockResolvedValue(ok(YNAB_HEADERS));
  mocks.importPreviewAccounts.mockResolvedValue(ok(["Checking"]));
  const { container } = renderWithClient(
    <ImportFileDialog accounts={[account()]} onClose={vi.fn()} />,
  );
  const input = container.querySelector('input[type="file"]') as HTMLInputElement;
  Object.defineProperty(input, "files", { value: [csvFile()] });
  fireEvent.change(input);
  await screen.findByText("statement.csv");
  await waitFor(() => expect(mocks.importPreviewAccounts).toHaveBeenCalled());
  expect(screen.getByLabelText("Import into")).toBeInTheDocument();
  expect(
    screen.queryByRole("region", { name: /accounts in this file/i }),
  ).not.toBeInTheDocument();
});
