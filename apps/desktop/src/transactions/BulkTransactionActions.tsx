import { useState } from "react";
import { Check, Loader2, Trash2, X } from "lucide-react";

import type {
  CategoryDto,
  IpcError,
  TagViewDto,
  TransactionRowDto,
} from "@/bindings";
import { Button } from "@/components/ui/button";
import { CategoryCombobox } from "@/categories/CategoryCombobox";
import { commands } from "@/bindings";

const SELECT_CLASS =
  "h-8 rounded-md border bg-background px-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

/// A bulk-action bar for the selected transactions (personal-cfo-j0cg.4): recategorize, add a tag,
/// mark reviewed, or delete them all in one pass. Composes the existing single-transaction
/// operations over the selection and reports a partial-failure summary. Delete asks to confirm.
export function BulkTransactionActions({
  selected,
  total,
  categories,
  tags,
  recategorize,
  setReviewed,
  deleteTransaction,
  setTags,
  createTag,
  bulkMarkReviewed,
  onSelectAll,
  onSelectionChange,
}: {
  selected: TransactionRowDto[];
  total: number;
  categories: CategoryDto[] | null;
  tags: TagViewDto[] | null;
  recategorize: (id: string, categoryId: string | null) => Promise<IpcError | null>;
  setReviewed: (id: string, reviewed: boolean) => Promise<IpcError | null>;
  deleteTransaction: (id: string) => Promise<IpcError | null>;
  setTags: (id: string, tagIds: string[]) => Promise<IpcError | null>;
  /// Create a tag by name (or resolve an existing one) — powers the always-available
  /// create-and-apply tag control (personal-cfo-4d8.24.8). Returns the new tag id, or an
  /// `IpcError`.
  createTag: (name: string) => Promise<string | IpcError>;
  /// One-round-trip bulk mark-reviewed (personal-cfo-4d8.25.16). When provided, the
  /// Mark-reviewed action uses it instead of the per-row fan-out.
  bulkMarkReviewed?: (
    ids: string[],
  ) => Promise<{ count: number } | { error: IpcError }>;
  onSelectAll: () => void;
  /// Replace the current selection — called with `[]` to clear, or with the ids that FAILED after a
  /// bulk op so the succeeded (and now-changed/removed) rows drop out and a retry targets only them.
  onSelectionChange: (ids: string[]) => void;
}) {
  const activeTags = (tags ?? []).filter((t) => !t.archived);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [tagDraft, setTagDraft] = useState("");
  const count = selected.length;

  /// Apply a tag by name to the whole selection (personal-cfo-4d8.24.8): resolve an existing
  /// tag case-insensitively, else mint one ONCE via `createTag`, then merge it into each
  /// row's tags. The selection is de-duplicated by id, so a transaction present in both lists
  /// is tagged exactly once.
  async function applyTag(rawName: string) {
    const name = rawName.trim();
    if (!name || busy) return;
    const existing = activeTags.find(
      (t) => t.name.toLowerCase() === name.toLowerCase(),
    );
    let tagId: string;
    if (existing) {
      tagId = existing.id;
    } else {
      setBusy(true);
      const created = await createTag(name);
      setBusy(false);
      if (typeof created !== "string") {
        setNote(`Could not create tag "${name}".`);
        return;
      }
      tagId = created;
    }
    setTagDraft("");
    // Merge against FRESH tag sets: selection snapshots can be stale for rows that
    // were never re-registered as visible (whole-inbox select-all), and set_tags
    // REPLACES the set — a stale merge would silently drop tags added since the
    // snapshot (adversarial review of 4d8.25.16). Falls back to the snapshot rows
    // when the refresh fails.
    let freshTagsById = new Map<string, string[]>();
    try {
      const fresh = await commands.transactionRowsByIds(
        selected.map((t) => t.transaction_id),
      );
      if (fresh.status === "ok") {
        freshTagsById = new Map(fresh.data.map((t) => [t.transaction_id, t.tag_ids]));
      }
    } catch {
      // snapshot fallback below
    }
    await runAll(
      (t) =>
        setTags(t.transaction_id, [
          ...new Set([...(freshTagsById.get(t.transaction_id) ?? t.tag_ids), tagId]),
        ]),
      "Tagged",
    );
  }

  /// Mark the whole selection reviewed: one bulk round-trip when the fast path is
  /// available (personal-cfo-4d8.25.16), else the per-row fan-out.
  async function markAllReviewed() {
    if (!bulkMarkReviewed) {
      await runAll((t) => setReviewed(t.transaction_id, true), "Marked reviewed");
      return;
    }
    setBusy(true);
    setNote(null);
    const result = await bulkMarkReviewed(selected.map((t) => t.transaction_id));
    setBusy(false);
    if ("error" in result) {
      setNote("Could not mark the selection reviewed.");
      return;
    }
    onSelectionChange([]);
  }

  /// Apply `op` to every selected transaction; clear the selection on a clean run, else keep it and
  /// surface how many failed so the user can retry.
  async function runAll(
    op: (txn: TransactionRowDto) => Promise<IpcError | null>,
    verb: string,
  ) {
    setBusy(true);
    setNote(null);
    const results = await Promise.all(selected.map(op));
    setBusy(false);
    // Keep only the rows that failed selected, so the succeeded (and now-changed or removed) rows
    // drop out and a retry acts only on the failures. An all-clean run leaves `[]` → cleared.
    const failedIds = selected
      .filter((_, i) => results[i] !== null)
      .map((t) => t.transaction_id);
    onSelectionChange(failedIds);
    if (failedIds.length > 0) {
      setNote(`${verb} ${count - failedIds.length} of ${count} — ${failedIds.length} failed.`);
    }
  }

  return (
    // A floating bar pinned to the bottom of the viewport (feedback 2026-07-03): the user can
    // keep scrolling and selecting without losing the actions off-screen.
    <div className="fixed bottom-5 left-1/2 z-40 flex max-w-[calc(100vw-3rem)] -translate-x-1/2 flex-wrap items-center gap-2 rounded-xl border bg-card px-3 py-2 text-sm shadow-lg">
      <span className="font-medium">{count} selected</span>
      {count < total && (
        // Scope: every selectable row currently visible across BOTH lists — the inbox's
        // selectable items on its page + the Activity list's current page, deduped by id.
        <Button size="sm" variant="ghost" disabled={busy} onClick={onSelectAll}>
          Select all {total} shown
        </Button>
      )}

      <div className="w-44">
        {/* Fire-once action: value stays null so the trigger always reads
            "Categorize…" and each pick fires the bulk run (j0cg.4 reset pattern). */}
        <CategoryCombobox
          categories={categories ?? []}
          aria-label="Categorize selected"
          value={null}
          placeholder="Categorize…"
          disabled={busy}
          onSelect={(next) => {
            if (next !== null) {
              void runAll((t) => recategorize(t.transaction_id, next), "Recategorized");
            }
          }}
          buttonClassName="h-8"
        />
      </div>

      {/* Always available, even with zero tags (personal-cfo-4d8.24.8): type an existing
          tag (suggested via the datalist) or a new name — Enter/Add creates + applies it to
          the whole selection. set_tags REPLACES the tag set, so applyTag merges with each
          row's current tags from the list snapshot (a tag edited elsewhere in the same
          instant could be missed until the list refetches — j0cg.4 notes). */}
      <div className="flex items-center gap-1">
        <input
          aria-label="Add or create tag"
          list="bulk-tag-options"
          className={SELECT_CLASS}
          placeholder="Tag…"
          disabled={busy}
          value={tagDraft}
          onChange={(e) => setTagDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              void applyTag(tagDraft);
            }
          }}
        />
        <datalist id="bulk-tag-options">
          {activeTags.map((t) => (
            <option key={t.id} value={t.name} />
          ))}
        </datalist>
        <Button
          size="sm"
          variant="outline"
          disabled={busy || tagDraft.trim() === ""}
          onClick={() => void applyTag(tagDraft)}
        >
          Add
        </Button>
      </div>

      <Button
        size="sm"
        variant="outline"
        disabled={busy}
        onClick={() => void markAllReviewed()}
      >
        <Check aria-hidden />
        Mark reviewed
      </Button>

      {confirmingDelete ? (
        <>
          <span className="text-loss">Delete {count}?</span>
          <Button
            size="sm"
            variant="destructive"
            disabled={busy}
            onClick={() => {
              setConfirmingDelete(false);
              void runAll((t) => deleteTransaction(t.transaction_id), "Deleted");
            }}
          >
            Confirm delete
          </Button>
          <Button
            size="sm"
            variant="ghost"
            disabled={busy}
            onClick={() => setConfirmingDelete(false)}
          >
            Cancel
          </Button>
        </>
      ) : (
        <Button
          size="sm"
          variant="outline"
          disabled={busy}
          onClick={() => setConfirmingDelete(true)}
        >
          <Trash2 aria-hidden />
          Delete
        </Button>
      )}

      {busy && <Loader2 className="size-4 animate-spin" aria-hidden />}
      {note && (
        <span role="alert" className="text-loss">
          {note}
        </span>
      )}

      <Button
        size="sm"
        variant="ghost"
        className="ml-auto"
        disabled={busy}
        onClick={() => onSelectionChange([])}
      >
        <X aria-hidden />
        Clear
      </Button>
    </div>
  );
}
