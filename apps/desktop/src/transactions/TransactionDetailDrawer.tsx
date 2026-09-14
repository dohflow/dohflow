import { useEffect, useRef, useState, type ChangeEvent } from "react";
import {
  Check,
  ChevronRight,
  FileText,
  Loader2,
  Paperclip,
  Repeat,
  Trash2,
  X,
} from "lucide-react";

import type {
  CategoryDto,
  IpcError,
  TagViewDto,
  TransactionRowDto,
} from "@/bindings";
import { Button } from "@/components/ui/button";
import { describeIpcError } from "@/vault/useVault";
import {
  formatBytes,
  formatDate,
  formatIsoDate,
  formatSignedMoney,
  signedAmountClass,
} from "@/lib/format";
import { cn } from "@/lib/utils";
import { categoryLabels } from "@/categories/labels";
import { CategoryCombobox } from "@/categories/CategoryCombobox";
import { CreateCategoryDialog } from "@/categories/CreateCategoryDialog";
import { TagChip } from "@/tags/TagChip";
import { MakeRecurringBillForm } from "@/bills/BillsView";
import { SplitEditor } from "./SplitEditor";
import { useSetSplits, useTransactionSplits } from "./useSplits";
import { useTransactionAttachments } from "./useTransactionAttachments";
import { useImportedTransactionFields } from "./useImportedTransactionFields";

const SELECT_CLASS =
  "flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background";

/// A slide-over panel showing a transaction's detail and its encrypted
/// attachments (ADR 0023). The app's first entity-detail surface. Attach / list /
/// remove only — decrypt-to-view is deferred to parser isolation (ADR 0022).
export function TransactionDetailDrawer({
  transaction,
  categories,
  onRecategorize,
  onDelete,
  onSetReviewed,
  tags,
  onCreateTag,
  onSetTags,
  onSetNote,
  onClose,
}: {
  transaction: TransactionRowDto;
  /// The taxonomy for the category picker (`null` while loading).
  categories: CategoryDto[] | null;
  /// Assign (or clear, with `null`) the transaction's category.
  onRecategorize: (
    transactionId: string,
    categoryId: string | null,
  ) => Promise<IpcError | null>;
  /// Delete (void) the transaction; the drawer closes on success.
  onDelete: (transactionId: string) => Promise<IpcError | null>;
  /// Mark the transaction reviewed / unreviewed (ADR 0032, personal-cfo-4d8.7).
  onSetReviewed: (
    transactionId: string,
    reviewed: boolean,
  ) => Promise<IpcError | null>;
  /// The tag vocabulary for the picker (`null` while loading; ADR 0033).
  tags: TagViewDto[] | null;
  /// Create a tag, returning its new id (or an error); used by the add-tag input.
  onCreateTag: (name: string) => Promise<string | IpcError>;
  /// Replace the transaction's tag set.
  onSetTags: (
    transactionId: string,
    tagIds: string[],
  ) => Promise<IpcError | null>;
  /// Set or clear the transaction's note (`null` clears it).
  onSetNote: (
    transactionId: string,
    note: string | null,
  ) => Promise<IpcError | null>;
  onClose: () => void;
}) {
  const { attachments, error, attachFile, removeAttachment, isAttaching } =
    useTransactionAttachments(transaction.transaction_id);
  const { imported } = useImportedTransactionFields(transaction.transaction_id);
  const [importedOpen, setImportedOpen] = useState(false);
  const splitsQuery = useTransactionSplits(transaction.transaction_id);
  const setSplits = useSetSplits();
  const [splitting, setSplitting] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  // Optimistic category so the picker reflects the choice immediately while the
  // list refetches in the background (the `transaction` prop is a snapshot).
  const [categoryId, setCategoryId] = useState<string | null>(
    transaction.category_id,
  );
  const [savingCategory, setSavingCategory] = useState(false);
  // The create-from-picker dialog, prefilled with the picker's search text
  // (personal-cfo-4d8.25.19); null = closed.
  const [creatingCategory, setCreatingCategory] = useState<string | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  // Optimistic reviewed-state (the `transaction` prop is a snapshot).
  const [reviewed, setReviewedState] = useState(transaction.reviewed);
  const [settingReviewed, setSettingReviewed] = useState(false);
  const [reviewError, setReviewError] = useState<string | null>(null);
  // Optimistic tags + note (the `transaction` prop is a snapshot).
  const [tagIds, setTagIds] = useState<string[]>(transaction.tag_ids);
  const [tagInput, setTagInput] = useState("");
  const [tagError, setTagError] = useState<string | null>(null);
  const [note, setNote] = useState(transaction.note ?? "");
  const [noteError, setNoteError] = useState<string | null>(null);
  const [makingRecurring, setMakingRecurring] = useState(false);
  const { options } = categoryLabels(categories ?? []);

  async function onConfirmDelete() {
    setDeleting(true);
    setDeleteError(null);
    const failure = await onDelete(transaction.transaction_id);
    setDeleting(false);
    if (failure) {
      setDeleteError(describeIpcError(failure));
      return;
    }
    onClose(); // the transaction is gone — close the drawer
  }

  async function toggleReviewed() {
    const next = !reviewed;
    setReviewedState(next);
    setSettingReviewed(true);
    setReviewError(null);
    const failure = await onSetReviewed(transaction.transaction_id, next);
    setSettingReviewed(false);
    if (failure) {
      setReviewedState(!next); // roll back the optimistic toggle
      setReviewError(describeIpcError(failure));
    }
  }

  // Replace the tag set optimistically, rolling back on failure.
  async function commitTags(next: string[]) {
    const previous = tagIds;
    setTagIds(next);
    setTagError(null);
    const failure = await onSetTags(transaction.transaction_id, next);
    if (failure) {
      setTagIds(previous);
      setTagError(describeIpcError(failure));
    }
  }

  function removeTag(tagId: string) {
    void commitTags(tagIds.filter((id) => id !== tagId));
  }

  // Add the typed tag: match an existing one by name (case-insensitive), else mint it.
  async function addTagFromInput() {
    const name = tagInput.trim();
    if (!name) return;
    setTagInput("");
    setTagError(null);
    const existing = (tags ?? []).find(
      (t) => t.name.toLowerCase() === name.toLowerCase(),
    );
    let tagId = existing?.id ?? null;
    if (tagId === null) {
      const created = await onCreateTag(name);
      if (typeof created !== "string") {
        setTagError(describeIpcError(created));
        return;
      }
      tagId = created;
    }
    if (tagIds.includes(tagId)) return; // already assigned — no-op
    await commitTags([...tagIds, tagId]);
  }

  // Persist the note on blur if it changed (`null` clears it).
  async function saveNote() {
    const trimmed = note.trim();
    if (trimmed === (transaction.note ?? "").trim()) return;
    setNoteError(null);
    const failure = await onSetNote(
      transaction.transaction_id,
      trimmed === "" ? null : trimmed,
    );
    if (failure) setNoteError(describeIpcError(failure));
  }

  async function onPickCategory(next: string | null) {
    const previous = categoryId;
    setCategoryId(next);
    setSavingCategory(true);
    setActionError(null);
    const failure = await onRecategorize(transaction.transaction_id, next);
    setSavingCategory(false);
    if (failure) {
      setCategoryId(previous); // roll back the optimistic choice
      setActionError(describeIpcError(failure));
    }
  }

  // Close on Escape — this is a modal surface over the app shell.
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  async function onPickFile(event: ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    event.target.value = ""; // let the user re-pick the same file later
    if (!file) return;
    setActionError(null);
    const failure = await attachFile(file);
    if (failure) setActionError(describeIpcError(failure));
  }

  async function onRemove(id: string) {
    setActionError(null);
    const failure = await removeAttachment(id);
    if (failure) setActionError(describeIpcError(failure));
  }

  return (
    <div className="fixed inset-0 z-50 flex justify-end">
      <div
        className="absolute inset-0 bg-foreground/40"
        aria-hidden
        onClick={onClose}
      />
      <div
        role="dialog"
        aria-label="Transaction detail"
        className="relative flex h-full w-full max-w-md flex-col gap-4 overflow-y-auto border-l bg-background p-6 shadow-xl"
      >
        <div className="flex items-start justify-between">
          <div className="min-w-0">
            <div className="truncate font-semibold">
              {transaction.memo ?? transaction.counterparty ?? transaction.account_name}
            </div>
            <div className="text-xs text-muted-foreground">
              {transaction.memo || transaction.counterparty
                ? `${transaction.account_name} · ${formatDate(transaction.occurred_at)}`
                : formatDate(transaction.occurred_at)}
            </div>
          </div>
          <div className="flex items-center gap-3">
            <div
              className={`font-medium tabular-nums ${signedAmountClass(transaction.amount)}`}
            >
              {formatSignedMoney(transaction.amount)}
            </div>
            <button
              type="button"
              onClick={onClose}
              aria-label="Close"
              className="rounded-md p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
            >
              <X className="size-4" aria-hidden />
            </button>
          </div>
        </div>

        <div className="flex flex-col gap-1.5">
          <button
            type="button"
            onClick={toggleReviewed}
            disabled={settingReviewed}
            className={cn(
              "inline-flex items-center gap-1.5 self-start rounded-full border px-3 py-1 text-xs font-medium transition-colors disabled:opacity-60",
              reviewed
                ? "border-gain/40 bg-gain/10 text-gain"
                : "border-input text-muted-foreground hover:bg-muted",
            )}
          >
            <Check className="size-3.5" aria-hidden />
            {reviewed ? "Reviewed" : "Mark reviewed"}
          </button>
          {reviewError && (
            <p role="alert" className="text-xs text-loss">
              {reviewError}
            </p>
          )}
        </div>

        {transaction.transaction_date && (
          <section className="flex flex-col gap-2" aria-label="Dates">
            <h3 className="text-sm font-semibold">Dates</h3>
            {/* An import carried both dates: the posted date (when the money moved,
                primary) and the transaction/authorization date (ADR 0045). */}
            <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
              <dt className="text-muted-foreground">Posted</dt>
              <dd className="tabular-nums">{formatDate(transaction.occurred_at)}</dd>
              <dt className="text-muted-foreground">Transaction</dt>
              <dd className="tabular-nums">
                {formatIsoDate(transaction.transaction_date)}
              </dd>
            </dl>
          </section>
        )}

        {imported && imported.fields.length > 0 && (
          <section className="flex flex-col gap-2" aria-label="Imported details">
            <button
              type="button"
              onClick={() => setImportedOpen((open) => !open)}
              aria-expanded={importedOpen}
              className="flex items-center gap-1.5 self-start text-sm font-semibold"
            >
              <ChevronRight
                className={cn(
                  "size-4 text-muted-foreground transition-transform",
                  importedOpen && "rotate-90",
                )}
                aria-hidden
              />
              Imported details
              <span className="font-normal text-muted-foreground">
                ({imported.source_type.toUpperCase()})
              </span>
            </button>
            {/* Every column the importer captured — nothing is dropped (ADR 0045 §2). */}
            {importedOpen && (
              <dl className="grid grid-cols-[minmax(0,auto)_1fr] gap-x-4 gap-y-1 text-sm">
                {imported.fields.map((field) => (
                  <div key={field.key} className="contents">
                    <dt className="truncate text-muted-foreground">{field.key}</dt>
                    <dd className="min-w-0 break-words">{field.value || "—"}</dd>
                  </div>
                ))}
              </dl>
            )}
          </section>
        )}

        <section className="flex flex-col gap-2">
          <h3 className="text-sm font-semibold">Category</h3>
          <CategoryCombobox
            categories={categories ?? []}
            aria-label="Category"
            value={categoryId}
            disabled={categories === null || savingCategory}
            onSelect={(next) => void onPickCategory(next)}
            clearLabel="Uncategorized"
            onCreateNew={(query) => setCreatingCategory(query)}
            buttonClassName="h-10"
          />
        </section>

        <section className="flex flex-col gap-2">
          <h3 className="text-sm font-semibold">Split</h3>
          {splitting ? (
            <SplitEditor
              transaction={transaction}
              categories={categories}
              tags={tags}
              existing={splitsQuery.data ?? []}
              onCreateTag={onCreateTag}
              onSave={(lines) => setSplits(transaction.transaction_id, lines)}
              onClear={() => setSplits(transaction.transaction_id, [])}
              onClose={() => setSplitting(false)}
            />
          ) : (splitsQuery.data?.length ?? 0) > 0 ? (
            <>
              <ul className="flex flex-col gap-1 text-sm">
                {splitsQuery.data?.map((line) => (
                  <li
                    key={line.id}
                    className="flex items-center justify-between gap-2"
                  >
                    <span className="truncate text-muted-foreground">
                      {options.find((option) => option.id === line.category_id)
                        ?.label ?? "Uncategorized"}
                    </span>
                    <span className="tabular-nums">
                      {formatSignedMoney(line.amount)}
                    </span>
                  </li>
                ))}
              </ul>
              <Button
                variant="ghost"
                size="sm"
                className="self-start"
                onClick={() => setSplitting(true)}
              >
                Edit split
              </Button>
            </>
          ) : (
            <Button
              variant="ghost"
              size="sm"
              className="self-start"
              onClick={() => setSplitting(true)}
            >
              Split transaction
            </Button>
          )}
        </section>

        <section className="flex flex-col gap-2">
          <h3 className="text-sm font-semibold">Tags</h3>
          {tagIds.length > 0 && (
            <div className="flex flex-wrap gap-1.5">
              {tagIds.map((id) => {
                const tag = tags?.find((candidate) => candidate.id === id);
                return tag ? (
                  <TagChip key={id} tag={tag} onRemove={() => removeTag(id)} />
                ) : null;
              })}
            </div>
          )}
          <input
            list="drawer-tag-options"
            value={tagInput}
            onChange={(event) => setTagInput(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                void addTagFromInput();
              }
            }}
            placeholder="Add a tag…"
            aria-label="Add a tag"
            className={SELECT_CLASS}
          />
          <datalist id="drawer-tag-options">
            {(tags ?? [])
              .filter((tag) => !tag.archived && !tagIds.includes(tag.id))
              .map((tag) => (
                <option key={tag.id} value={tag.name} />
              ))}
          </datalist>
          {tagError && (
            <p role="alert" className="text-sm text-loss">
              {tagError}
            </p>
          )}
        </section>

        <section className="flex flex-col gap-2">
          <h3 className="text-sm font-semibold">Note</h3>
          <textarea
            value={note}
            onChange={(event) => setNote(event.target.value)}
            onBlur={() => void saveNote()}
            maxLength={4096}
            rows={3}
            placeholder="Add a note…"
            aria-label="Note"
            className={cn(SELECT_CLASS, "h-auto resize-y")}
          />
          {noteError && (
            <p role="alert" className="text-sm text-loss">
              {noteError}
            </p>
          )}
        </section>

        <section className="flex flex-col gap-2">
          <h3 className="text-sm font-semibold">Recurring</h3>
          {makingRecurring ? (
            <MakeRecurringBillForm
              // Remount per transaction so the pre-fill can never go stale.
              key={transaction.transaction_id}
              transaction={transaction}
              onCancel={() => setMakingRecurring(false)}
              onCreated={() => setMakingRecurring(false)}
            />
          ) : (
            <>
              <p className="text-sm text-muted-foreground">
                Track this as a recurring bill in your forecast.
              </p>
              <Button
                variant="outline"
                size="sm"
                className="self-start"
                onClick={() => setMakingRecurring(true)}
              >
                <Repeat aria-hidden />
                Make recurring
              </Button>
            </>
          )}
        </section>

        <section className="flex flex-col gap-2">
          <h3 className="text-sm font-semibold">Attachments</h3>

          {error && (
            <p role="alert" className="text-sm text-loss">
              {error}
            </p>
          )}

          {attachments === null ? (
            <div className="flex items-center gap-2 py-4 text-sm text-muted-foreground">
              <Loader2 className="size-4 animate-spin" aria-hidden /> Loading…
            </div>
          ) : attachments.length === 0 ? (
            <p className="py-2 text-sm text-muted-foreground">
              No documents attached yet.
            </p>
          ) : (
            <ul className="flex flex-col gap-1">
              {attachments.map((doc) => (
                <li
                  key={doc.id}
                  className="flex items-center justify-between rounded-md border px-3 py-2"
                >
                  <div className="flex min-w-0 items-center gap-2">
                    <FileText
                      className="size-4 shrink-0 text-muted-foreground"
                      aria-hidden
                    />
                    <span className="truncate text-sm">
                      {doc.original_filename ?? "Document"}
                    </span>
                    <span className="shrink-0 text-xs text-muted-foreground">
                      {formatBytes(doc.plaintext_size)}
                    </span>
                  </div>
                  <button
                    type="button"
                    onClick={() => onRemove(doc.id)}
                    aria-label={`Remove ${doc.original_filename ?? "document"}`}
                    className="rounded-md p-1 text-muted-foreground hover:bg-muted hover:text-loss"
                  >
                    <Trash2 className="size-4" aria-hidden />
                  </button>
                </li>
              ))}
            </ul>
          )}

          <input
            ref={fileInput}
            type="file"
            className="sr-only"
            aria-label="Choose a document to attach"
            onChange={onPickFile}
          />
          <Button
            variant="outline"
            size="sm"
            className="self-start"
            disabled={isAttaching}
            onClick={() => fileInput.current?.click()}
          >
            <Paperclip aria-hidden />
            {isAttaching ? "Attaching…" : "Attach document"}
          </Button>

          {actionError && (
            <p role="alert" className="text-sm text-loss">
              {actionError}
            </p>
          )}
        </section>

        <section className="flex flex-col gap-2 border-t pt-4">
          {confirmingDelete ? (
            <>
              <p className="text-sm">
                Delete this transaction? It&apos;s removed from your lists and
                balances. The entry stays in your encrypted history.
              </p>
              {deleteError && (
                <p role="alert" className="text-sm text-loss">
                  {deleteError}
                </p>
              )}
              <div className="flex justify-end gap-2">
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={deleting}
                  onClick={() => setConfirmingDelete(false)}
                >
                  Cancel
                </Button>
                <Button
                  variant="destructive"
                  size="sm"
                  disabled={deleting}
                  onClick={onConfirmDelete}
                >
                  {deleting && (
                    <Loader2 className="size-4 animate-spin" aria-hidden />
                  )}
                  Delete
                </Button>
              </div>
            </>
          ) : (
            <Button
              variant="ghost"
              size="sm"
              className="self-start text-loss hover:text-loss"
              onClick={() => {
                setDeleteError(null);
                setConfirmingDelete(true);
              }}
            >
              <Trash2 aria-hidden />
              Delete transaction
            </Button>
          )}
        </section>
      </div>
      {creatingCategory !== null && (
        <CreateCategoryDialog
          initialName={creatingCategory}
          onCreated={(id) => void onPickCategory(id)}
          onClose={() => setCreatingCategory(null)}
        />
      )}
    </div>
  );
}
