import { useEffect, useRef, useState, type ChangeEvent } from "react";
import {
  CheckCircle2,
  ChevronRight,
  FileUp,
  Loader2,
  Upload,
  X,
} from "lucide-react";

import type {
  AccountViewDto,
  BatchResultDto,
  ColumnMappingDto,
  SourcePresetDto,
} from "@/bindings";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { describeIpcError } from "@/vault/useVault";
import { ExportGuidance } from "@/imports/ExportGuidance";
import { migrateGuideUrl, openExternal } from "@/lib/openExternal";

import { useImportBatch } from "./useImportBatch";

const SELECT_CLASS =
  "flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background";

/// The canonical CSV fields a user can remap, in display order (ADR 0045 slice 3,
/// personal-cfo-4d8.24.1.2; `account` added personal-cfo-gvidg for sources whose
/// export spans multiple accounts, e.g. a preset's own mapping). Each maps to a
/// `ColumnMappingDto` key; the "Date" field sets the posted (primary) date.
/// Unmapped fields fall back to header auto-detect. `category_group` (also
/// personal-cfo-gvidg) is deliberately NOT here — only a preset sets it; a
/// generic import has no safe name to guess it from, and hand-mapping a
/// second category column is a rare enough need to defer rather than widen
/// this grid for it now.
type MapField =
  | "date"
  | "description"
  | "amount"
  | "debit"
  | "credit"
  | "account"
  | "category"
  | "currency";
const MAP_FIELDS: { key: MapField; label: string }[] = [
  { key: "date", label: "Date (posted)" },
  { key: "description", label: "Description" },
  { key: "amount", label: "Amount" },
  { key: "debit", label: "Debit" },
  { key: "credit", label: "Credit" },
  { key: "account", label: "Account" },
  { key: "category", label: "Category" },
  { key: "currency", label: "Currency" },
];

/// Build a `ColumnMappingDto` from the user's selections, or `null` when nothing was
/// overridden (so the importer auto-detects, or a chosen preset's own mapping
/// applies unmodified — personal-cfo-gvidg).
function toColumnMapping(
  selections: Partial<Record<MapField, string>>,
): ColumnMappingDto | null {
  const pick = (key: MapField) => selections[key] || null;
  if (!MAP_FIELDS.some((f) => pick(f.key))) return null;
  return {
    date: pick("date"),
    description: pick("description"),
    amount: pick("amount"),
    debit: pick("debit"),
    credit: pick("credit"),
    account: pick("account"),
    category: pick("category"),
    category_group: null,
    currency: pick("currency"),
    memo: null,
  };
}

/// Every non-empty field a preset's own mapping declares, as `[MapField, name]`
/// pairs — `category_group` excluded (not a MapField; applied server-side via
/// `preset_id` regardless of what the mapping UI shows).
function presetMapEntries(preset: SourcePresetDto): [MapField, string][] {
  const m = preset.column_mapping;
  const entries: [MapField, string | null][] = [
    ["date", m.date],
    ["description", m.description],
    ["amount", m.amount],
    ["debit", m.debit],
    ["credit", m.credit],
    ["account", m.account],
    ["category", m.category],
    ["currency", m.currency],
  ];
  return entries.filter((e): e is [MapField, string] => e[1] !== null);
}

/// Whether every column `preset` declares is actually present in `headers`
/// (case-insensitive, matching the importer's own `find()`) — when true, the
/// mapping step can be skipped entirely; the preset's own hints already
/// cover the file completely (personal-cfo-gvidg AC #2).
function presetFullyMatches(preset: SourcePresetDto, headers: string[]): boolean {
  const lower = headers.map((h) => h.trim().toLowerCase());
  return presetMapEntries(preset).every(([, name]) =>
    lower.includes(name.trim().toLowerCase()),
  );
}

/// Human-readable outcome of an import (ADR 0014 auto-commit-clean): a whole-file
/// re-upload is skipped; otherwise N commit and M (if any) are flagged for triage.
/// When merchant memory auto-categorized some of the new rows (ADR 0030 addendum,
/// personal-cfo-5n4.2), that count is appended so the auto-apply is surfaced.
function summaryMessage(batch: BatchResultDto): string {
  if (batch.status === "already_imported") {
    return "This file was already imported — nothing new was added.";
  }
  const committed = `Imported ${batch.committed} ${
    batch.committed === 1 ? "transaction" : "transactions"
  }.`;
  const autoCategorized =
    batch.auto_categorized > 0
      ? ` Auto-categorized ${batch.auto_categorized} from merchant memory.`
      : "";
  if (batch.flagged > 0) {
    return `${committed} ${batch.flagged} ${
      batch.flagged === 1 ? "needs" : "need"
    } review in the Money Inbox.${autoCategorized}`;
  }
  return `${committed} All clean — nothing to review.${autoCategorized}`;
}

/// Import a statement file into the ledger (personal-cfo-zl8f): choose a target
/// account + a file, hand the bytes to the pipeline, and show the outcome. The
/// flagged rows land in the Money Inbox behind this dialog (ADR 0014 §2/§7).
export function ImportFileDialog({
  accounts,
  onClose,
}: {
  accounts: AccountViewDto[];
  onClose: () => void;
}) {
  const { importFile, previewColumns, listPresets } = useImportBatch();
  const fileInput = useRef<HTMLInputElement>(null);
  const [accountId, setAccountId] = useState(accounts[0]?.id ?? "");
  const [file, setFile] = useState<File | null>(null);
  const [columns, setColumns] = useState<string[]>([]);
  const [mapping, setMapping] = useState<Partial<Record<MapField, string>>>({});
  const [showMapping, setShowMapping] = useState(false);
  const [importing, setImporting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [summary, setSummary] = useState<BatchResultDto | null>(null);
  // "Import from <app>" (personal-cfo-gvidg): "" is the generic/auto-detect
  // path, unchanged from before this feature existed.
  const [presets, setPresets] = useState<SourcePresetDto[]>([]);
  const [presetId, setPresetId] = useState("");
  const selectedPreset = presets.find((p) => p.id === presetId) ?? null;

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  useEffect(() => {
    void listPresets().then(setPresets);
  }, [listPresets]);

  /// Pre-fill the mapping UI from `preset` for whichever of its declared
  /// columns are actually present in `headers`; a gap is left blank (falls
  /// back to auto-detect / a manual pick from the real column list) rather
  /// than shown pointing at a header that doesn't exist. Auto-expands the
  /// mapping section only when something needs a look — a fully-matching
  /// preset stays collapsed, nothing to review (personal-cfo-gvidg AC #2).
  function applyPreset(preset: SourcePresetDto | null, headers: string[]) {
    if (!preset || headers.length === 0) {
      setMapping({});
      setShowMapping(false);
      return;
    }
    const lower = headers.map((h) => h.trim().toLowerCase());
    const next: Partial<Record<MapField, string>> = {};
    for (const [key, name] of presetMapEntries(preset)) {
      const match = headers[lower.indexOf(name.trim().toLowerCase())];
      if (match) next[key] = match;
    }
    setMapping(next);
    setShowMapping(!presetFullyMatches(preset, headers));
  }

  async function onPickFile(event: ChangeEvent<HTMLInputElement>) {
    const picked = event.target.files?.[0] ?? null;
    setFile(picked);
    setError(null);
    setColumns([]);
    setMapping({});
    setShowMapping(false);
    // Seed the (optional) column-mapping UI with the file's real headers. Only CSV
    // has mappable columns; OFX/an unrecognized file returns none (auto-detect only).
    if (picked) {
      const bytes = Array.from(new Uint8Array(await picked.arrayBuffer()));
      const headers = await previewColumns(bytes, picked.name);
      setColumns(headers);
      applyPreset(selectedPreset, headers);
    }
  }

  function onPickPreset(event: ChangeEvent<HTMLSelectElement>) {
    const id = event.target.value;
    setPresetId(id);
    applyPreset(presets.find((p) => p.id === id) ?? null, columns);
  }

  async function onImport() {
    if (!file || !accountId) return;
    const account = accounts.find((a) => a.id === accountId);
    setImporting(true);
    setError(null);
    const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
    const outcome = await importFile({
      data: bytes,
      filename: file.name,
      target_account_id: accountId,
      plugin_id: null,
      preset_id: presetId || null,
      column_mapping: toColumnMapping(mapping),
      default_currency: account?.balance.currency ?? null,
      date_format: null,
      idempotency_key: mintIdempotencyKey(),
    });
    setImporting(false);
    if ("error" in outcome) {
      setError(describeIpcError(outcome.error));
    } else {
      setSummary(outcome.batch);
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      <div className="absolute inset-0 bg-foreground/40" aria-hidden onClick={onClose} />
      <div
        role="dialog"
        aria-label="Import file"
        className="relative flex w-full max-w-md flex-col gap-4 rounded-lg border bg-background p-6 shadow-xl"
      >
        <div className="flex items-start justify-between">
          <h3 className="font-semibold">Import a file</h3>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="rounded-md p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <X className="size-4" aria-hidden />
          </button>
        </div>

        {summary ? (
          <>
            <div className="flex items-start gap-2 rounded-md bg-gain/10 px-3 py-2 text-sm text-gain">
              <CheckCircle2 className="mt-0.5 size-4 shrink-0" aria-hidden />
              <span>{summaryMessage(summary)}</span>
            </div>
            <div className="flex justify-end">
              <Button onClick={onClose}>Done</Button>
            </div>
          </>
        ) : (
          <>
            <p className="text-sm text-muted-foreground">
              Bank/credit-card exports — CSV or OFX/QFX (Quicken). Clean rows
              are added automatically; anything that looks like a duplicate is
              held in the Money Inbox for you to review.
            </p>

            {presets.length > 0 && (
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="import-preset">Import from</Label>
                <select
                  id="import-preset"
                  className={SELECT_CLASS}
                  value={presetId}
                  onChange={onPickPreset}
                >
                  <option value="">Another bank or app (generic)</option>
                  {presets.map((preset) => (
                    <option key={preset.id} value={preset.id}>
                      {preset.display_name}
                    </option>
                  ))}
                </select>
              </div>
            )}

            <div className="flex flex-col gap-1.5">
              <Label htmlFor="import-account">Import into</Label>
              <select
                id="import-account"
                className={SELECT_CLASS}
                value={accountId}
                onChange={(event) => setAccountId(event.target.value)}
              >
                {accounts.map((account) => (
                  <option key={account.id} value={account.id}>
                    {account.name}
                  </option>
                ))}
              </select>
            </div>

            <div className="flex flex-col gap-1.5">
              <Label>File</Label>
              <input
                ref={fileInput}
                type="file"
                accept=".csv,text/csv,.ofx,.qfx,application/x-ofx,application/vnd.intu.qfx"
                className="hidden"
                onChange={onPickFile}
              />
              <Button
                type="button"
                variant="outline"
                onClick={() => fileInput.current?.click()}
              >
                <FileUp aria-hidden />
                {file ? file.name : "Choose file…"}
              </Button>
            </div>

              <details className="rounded-md border px-3 py-2">
                <summary className="cursor-pointer text-sm font-medium">
                  How do I export from my bank?
                </summary>
                <div className="pt-3">
                  <ExportGuidance id="import-export-guide" />
                </div>
              </details>

            {selectedPreset && (
              <p className="text-xs text-muted-foreground">
                <button
                  type="button"
                  className="underline underline-offset-2"
                  onClick={() => {
                    const url = migrateGuideUrl(selectedPreset.help_slug);
                    // Same handling as AboutCard's external links: nothing in
                    // the dialog can act on a browser-launch failure, but it
                    // must not vanish as an unhandled rejection either.
                    void openExternal(url).catch((err: unknown) => {
                      console.error(`Could not open ${url}`, err);
                    });
                  }}
                >
                  Full guide: moving from {selectedPreset.display_name}
                </button>
              </p>
            )}

            {columns.length > 0 && (

              <section className="flex flex-col gap-2" aria-label="Column mapping">
                <button
                  type="button"
                  onClick={() => setShowMapping((open) => !open)}
                  aria-expanded={showMapping}
                  className="flex items-center gap-1.5 self-start text-sm font-medium"
                >
                  <ChevronRight
                    className={`size-4 text-muted-foreground transition-transform ${
                      showMapping ? "rotate-90" : ""
                    }`}
                    aria-hidden
                  />
                  Column mapping
                  <span className="font-normal text-muted-foreground">(optional)</span>
                </button>
                {showMapping && (
                  <>
                    <p className="text-xs text-muted-foreground">
                      Only needed if the columns aren&apos;t detected correctly. Leave a
                      field on “Auto-detect” to keep the automatic choice.
                    </p>
                    <div className="grid grid-cols-2 gap-x-3 gap-y-2">
                      {MAP_FIELDS.map((field) => (
                        <div key={field.key} className="flex flex-col gap-1">
                          <Label htmlFor={`map-${field.key}`} className="text-xs">
                            {field.label}
                          </Label>
                          <select
                            id={`map-${field.key}`}
                            aria-label={`Map ${field.label}`}
                            className={SELECT_CLASS}
                            value={mapping[field.key] ?? ""}
                            onChange={(event) =>
                              setMapping((prev) => ({
                                ...prev,
                                [field.key]: event.target.value,
                              }))
                            }
                          >
                            <option value="">Auto-detect</option>
                            {columns.map((column) => (
                              <option key={column} value={column}>
                                {column}
                              </option>
                            ))}
                          </select>
                        </div>
                      ))}
                    </div>
                  </>
                )}
              </section>
            )}

            {error && (
              <p role="alert" className="text-sm text-loss">
                {error}
              </p>
            )}

            <div className="flex justify-end gap-2">
              <Button variant="ghost" onClick={onClose}>
                Cancel
              </Button>
              <Button onClick={onImport} disabled={!file || !accountId || importing}>
                {importing ? (
                  <Loader2 className="animate-spin" aria-hidden />
                ) : (
                  <Upload aria-hidden />
                )}
                {importing ? "Importing…" : "Import"}
              </Button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
