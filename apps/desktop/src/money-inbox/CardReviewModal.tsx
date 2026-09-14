import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useQuery } from "@tanstack/react-query";
import { Calendar, Check, FileText, RotateCcw, X } from "lucide-react";

import type { CategoryDto, IpcError, TagViewDto, TransactionRowDto } from "@/bindings";
import { commands } from "@/bindings";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ipcQuery } from "@/lib/query";
import { formatIsoDate, formatMoney } from "@/lib/format";
import { cn } from "@/lib/utils";
import { describeIpcError } from "@/vault/useVault";
import { CategoryCombobox } from "@/categories/CategoryCombobox";
import { CreateCategoryDialog } from "@/categories/CreateCategoryDialog";

/// One-at-a-time review of the Money Inbox — the "Card Review" mode
/// (personal-cfo-4d8.25.17, built to the approved Claude Design mock in the bead's
/// design field). A centered modal shows one transaction with editable category /
/// tags / notes; right-arrow (or "Mark reviewed") reviews and advances, left-arrow
/// (or "Later") sends the card to the back of the queue with a "Resurfaced" badge
/// when it comes around again. Reviewing is the SAME explicit `MarkReviewed` the
/// list rows dispatch (ADR 0032 — presentation only, no new review semantics).
///
/// Queue model (matches the mock's counters): the header is the position within the
/// remaining queue — "X of Y to review" where Y shrinks only on review; deferring
/// advances X (wrapping to 1 past the end). The subline carries total progress
/// ("N of TOTAL reviewed · M set aside").
export interface CardReviewModalProps {
  /// The review queue: transaction ids in inbox order (ADR 0014 default sort).
  queueIds: string[];
  /// Full row DTOs for the queue, keyed by transaction id (fetched by the caller
  /// via `transactionRowsByIds`, so cards outside the recent window still render).
  rowsById: ReadonlyMap<string, TransactionRowDto>;
  categories: CategoryDto[] | null;
  tags: TagViewDto[] | null;
  onMarkReviewed: (transactionId: string) => Promise<IpcError | null>;
  onRecategorize: (
    transactionId: string,
    categoryId: string | null,
  ) => Promise<IpcError | null>;
  onCreateTag: (name: string) => Promise<string | IpcError>;
  onSetTags: (transactionId: string, tagIds: string[]) => Promise<IpcError | null>;
  onSetNote: (transactionId: string, note: string | null) => Promise<IpcError | null>;
  onClose: () => void;
}

export function CardReviewModal({
  queueIds,
  rowsById,
  categories,
  tags,
  onMarkReviewed,
  onRecategorize,
  onCreateTag,
  onSetTags,
  onSetNote,
  onClose,
}: CardReviewModalProps) {
  const total = queueIds.length;
  const [queue, setQueue] = useState<string[]>(queueIds);
  const [position, setPosition] = useState(0);
  const [reviewedCount, setReviewedCount] = useState(0);
  // Cards sent to the back at least once: drives the "Resurfaced" badge and the
  // finished panel's "set aside, then cleared" stat.
  const [setAside, setSetAside] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  // Synchronous re-entrancy guard: two arrow presses in one tick both read the
  // stale `busy === false` closure — the ref flips before any re-render, so the
  // second press bails instead of double-reviewing (double count, double IPC).
  const busyRef = useRef(false);
  const [error, setError] = useState<string | null>(null);
  // Create-from-picker dialog (personal-cfo-4d8.25.19); null = closed.
  const [creatingCategory, setCreatingCategory] = useState<string | null>(null);

  const currentId = queue[position] ?? null;
  const current = currentId === null ? null : (rowsById.get(currentId) ?? null);
  const finished = queue.length === 0;
  // Tracks the card an async edit belonged to, so a late IPC failure that resolves
  // after the user advanced doesn't roll back the NEW card's state (adversarial
  // review of 4d8.25.17).
  const currentIdRef = useRef(currentId);
  useEffect(() => {
    currentIdRef.current = currentId;
  }, [currentId]);

  // ----- per-card edit state (seeded from the row when the card changes) -----
  // rowsById is a snapshot taken when the modal opened; edits persisted during this
  // session are layered over it, so a deferred card that resurfaces seeds from what
  // the user actually saved — never the stale snapshot (kernel SetTags is
  // exact-replace, so a stale tag seed would silently DELETE first-visit tags;
  // adversarial review of 4d8.25.17).
  const editsRef = useRef(
    new Map<string, { categoryId?: string | null; tagIds?: string[]; note?: string }>(),
  );
  const recordEdit = useCallback(
    (id: string, patch: { categoryId?: string | null; tagIds?: string[]; note?: string }) => {
      editsRef.current.set(id, { ...editsRef.current.get(id), ...patch });
    },
    [],
  );
  const [categoryId, setCategoryId] = useState<string | null>(null);
  const [tagIds, setTagIds] = useState<string[]>([]);
  const [tagInput, setTagInput] = useState("");
  const [note, setNote] = useState("");
  useEffect(() => {
    const persisted = currentId === null ? undefined : editsRef.current.get(currentId);
    setCategoryId(persisted?.categoryId !== undefined ? persisted.categoryId : (current?.category_id ?? null));
    setTagIds(persisted?.tagIds ?? current?.tag_ids ?? []);
    setTagInput("");
    setNote(persisted?.note ?? current?.note ?? "");
    setError(null);
  }, [currentId]); // eslint-disable-line react-hooks/exhaustive-deps -- seed per card

  const attachments = useQuery({
    queryKey: ["transactions", "attachments", currentId],
    enabled: currentId !== null,
    queryFn: () =>
      ipcQuery(
        commands.transactionAttachments(currentId ?? ""),
        "Could not load attachments.",
      ),
  });

  // Persist a changed note before leaving the card, so an edit-then-arrow never
  // silently drops the text.
  const flushNote = useCallback(async (): Promise<IpcError | null> => {
    if (currentId === null || current === null) return null;
    const baseline = editsRef.current.get(currentId)?.note ?? current.note ?? "";
    const trimmed = note.trim();
    if (trimmed === baseline.trim()) return null;
    const failure = await onSetNote(currentId, trimmed === "" ? null : trimmed);
    if (!failure) recordEdit(currentId, { note: trimmed });
    return failure;
  }, [current, currentId, note, onSetNote, recordEdit]);

  const review = useCallback(async () => {
    if (currentId === null || busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setError(null);
    const noteFailure = await flushNote();
    const failure = noteFailure ?? (await onMarkReviewed(currentId));
    busyRef.current = false;
    setBusy(false);
    if (failure) {
      setError(describeIpcError(failure));
      return;
    }
    setQueue((prev) => {
      const next = prev.filter((id) => id !== currentId);
      setPosition((p) => (p >= next.length ? 0 : p));
      return next;
    });
    setReviewedCount((n) => n + 1);
  }, [currentId, flushNote, onMarkReviewed]);

  const later = useCallback(async () => {
    if (currentId === null || busyRef.current || queue.length === 0) return;
    busyRef.current = true;
    setBusy(true);
    const noteFailure = await flushNote();
    busyRef.current = false;
    setBusy(false);
    if (noteFailure) {
      setError(describeIpcError(noteFailure));
      return;
    }
    setSetAside((prev) => new Set(prev).add(currentId));
    setQueue((prev) => {
      const rest = prev.filter((id) => id !== currentId);
      return [...rest, currentId];
    });
    // Advance the pass position; wrap to the front past the end. The card moved to
    // the back, so the same index already shows the next card — except at the end.
    setPosition((p) => (p >= queue.length - 1 ? 0 : p));
  }, [currentId, flushNote, queue.length]);

  // Keyboard-first (mock): ← Later, → Reviewed, Esc close — inert while a text
  // input has focus so arrows never fight the caret.
  //
  // useLayoutEffect, not useEffect (personal-cfo-bv6w2): this listener closes
  // over `review`/`later`, which close over `currentId` — every one of those
  // needs to be re-bound the instant currentId changes, and `useEffect` does
  // not guarantee that. Passive effects are scheduled AFTER paint, but a
  // DOM-observing consumer (a MutationObserver — exactly what Testing
  // Library's `findBy*`/`waitFor` use, and structurally the same primitive a
  // real assistive-tech tool or a fast-typing user's browser could race
  // against) can see the new DOM the instant it commits, before that passive
  // effect has run. Confirmed directly: instrumented this effect and `review`
  // to log on every call, reproduced under load, and caught the listener
  // still bound to the PREVIOUS card's id at the moment `screen.findByText`
  // had already resolved for the NEW card — so the next arrow press reviewed
  // (or misfiled an edit against) the wrong transaction. `useLayoutEffect`
  // runs synchronously as part of the same commit that updates the DOM, so
  // the listener is guaranteed current before anything outside React can
  // observe the change and react to it.
  useLayoutEffect(() => {
    function onKey(event: KeyboardEvent) {
      // While the create-category dialog is open it owns the keyboard — the modal's
      // review/defer shortcuts (and Escape) must not fire under it (adversarial
      // review of 4d8.25.19). The dialog's own capture-phase Escape closes it.
      if (creatingCategory !== null) return;
      const target = event.target as HTMLElement | null;
      const typing =
        target !== null &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.tagName === "SELECT" ||
          target.isContentEditable);
      if (event.key === "Escape") {
        // First Escape inside a field leaves the field (blur flushes the note);
        // Escape outside flushes any dirty note, then closes — typed text is
        // never silently dropped (adversarial review of 4d8.25.17).
        if (typing) {
          target?.blur();
          return;
        }
        void flushNote();
        onClose();
        return;
      }
      if (typing || finished) return;
      if (event.key === "ArrowRight") {
        event.preventDefault();
        void review();
      } else if (event.key === "ArrowLeft") {
        event.preventDefault();
        void later();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [creatingCategory, finished, flushNote, later, onClose, review]);

  // ----- edit handlers (drawer parity: optimistic with rollback) -----
  // The edit's card id is captured so a late resolution that lands after the user
  // advanced records/rolls back the RIGHT card, never the one now on screen.
  async function pickCategory(next: string | null) {
    const id = currentId;
    if (id === null) return;
    const previous = categoryId;
    setCategoryId(next);
    const failure = await onRecategorize(id, next);
    if (failure) {
      if (currentIdRef.current === id) {
        setCategoryId(previous);
        setError(describeIpcError(failure));
      }
      return;
    }
    recordEdit(id, { categoryId: next });
  }

  async function commitTags(next: string[]) {
    const id = currentId;
    if (id === null) return;
    const previous = tagIds;
    setTagIds(next);
    const failure = await onSetTags(id, next);
    if (failure) {
      if (currentIdRef.current === id) {
        setTagIds(previous);
        setError(describeIpcError(failure));
      }
      return;
    }
    recordEdit(id, { tagIds: next });
  }

  async function addTagFromInput() {
    const name = tagInput.trim();
    if (!name) return;
    setTagInput("");
    const existing = (tags ?? []).find(
      (t) => t.name.toLowerCase() === name.toLowerCase(),
    );
    let tagId = existing?.id ?? null;
    if (tagId === null) {
      const created = await onCreateTag(name);
      if (typeof created !== "string") {
        setError(describeIpcError(created));
        return;
      }
      tagId = created;
    }
    if (tagIds.includes(tagId)) return;
    await commitTags([...tagIds, tagId]);
  }

  const tagNameById = useMemo(
    () => new Map((tags ?? []).map((t) => [t.id, t.name] as const)),
    [tags],
  );

  const activeSetAside = queue.filter((id) => setAside.has(id)).length;
  const clearedSetAside = setAside.size - activeSetAside;
  const resurfaced = currentId !== null && setAside.has(currentId);
  const progressPct = total === 0 ? 100 : Math.round((reviewedCount / total) * 100);

  // Focus the dialog on mount so keyboard review works immediately.
  const dialogRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    dialogRef.current?.focus();
  }, []);

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/40 pt-[8vh]"
      role="presentation"
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-label="Card Review"
        tabIndex={-1}
        className="flex max-h-[84vh] w-[30rem] max-w-[calc(100vw-2rem)] flex-col overflow-hidden rounded-xl border bg-background shadow-xl outline-none"
      >
        {/* Thin progress bar (mock): total review progress. */}
        <div className="h-1 w-full bg-muted" aria-hidden>
          <div
            className="h-full bg-primary transition-[width]"
            style={{ width: `${progressPct}%` }}
          />
        </div>

        {finished ? (
          <FinishedPanel
            reviewed={reviewedCount}
            clearedSetAside={clearedSetAside}
            onClose={onClose}
          />
        ) : (
          <>
            <header className="flex items-start justify-between gap-2 border-b px-5 py-3">
              <div>
                <p className="flex items-center gap-2 text-sm font-semibold">
                  {position + 1} of {queue.length} to review
                  {resurfaced && (
                    <Badge variant="warning" className="gap-1">
                      <RotateCcw className="size-3" aria-hidden /> Resurfaced
                    </Badge>
                  )}
                </p>
                <p className="text-xs text-muted-foreground">
                  {reviewedCount} of {total} reviewed
                  {activeSetAside > 0 && <> · {activeSetAside} set aside</>}
                </p>
              </div>
              <button
                type="button"
                aria-label="Close Card Review"
                onClick={onClose}
                className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
              >
                <X className="size-4" aria-hidden />
              </button>
            </header>

            {current === null ? (
              <div className="px-5 py-8 text-sm text-muted-foreground">
                This item's details are unavailable.
              </div>
            ) : (
              <div className="flex-1 overflow-y-auto px-5 py-4">
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <h3 className="truncate text-lg font-semibold">
                      {current.counterparty ?? current.memo ?? "Transaction"}
                    </h3>
                    {current.memo !== null &&
                      current.counterparty !== null &&
                      current.memo !== current.counterparty && (
                        <p className="truncate text-xs uppercase text-muted-foreground">
                          {current.memo}
                        </p>
                      )}
                  </div>
                  <span
                    className={cn(
                      "shrink-0 text-lg font-semibold tabular-nums",
                      current.amount.minor_units < 0 ? "text-loss" : "text-gain",
                    )}
                  >
                    {formatMoney(current.amount)}
                  </span>
                </div>

                <p className="mt-2 flex items-center gap-2 text-xs text-muted-foreground">
                  <Calendar className="size-3.5" aria-hidden />
                  {formatIsoDate(current.occurred_at.slice(0, 10))}
                  <span aria-hidden>·</span>
                  <span className="rounded-full bg-muted px-2 py-0.5">
                    {current.account_name}
                  </span>
                </p>

                <label
                  className="mt-4 block text-xs font-medium text-muted-foreground"
                  htmlFor="card-review-category"
                >
                  Category
                </label>
                <div className="mt-1">
                  <CategoryCombobox
                    categories={categories ?? []}
                    id="card-review-category"
                    aria-label="Category"
                    value={categoryId}
                    onSelect={(next) => void pickCategory(next)}
                    clearLabel="Uncategorized"
                    onCreateNew={(query) => setCreatingCategory(query)}
                  />
                </div>

                <label
                  className="mt-3 block text-xs font-medium text-muted-foreground"
                  htmlFor="card-review-tags"
                >
                  Tags
                </label>
                {tagIds.length > 0 && (
                  <div className="mt-1 flex flex-wrap gap-1">
                    {tagIds.map((id) => (
                      <Badge key={id} variant="secondary" className="gap-1">
                        {tagNameById.get(id) ?? "Tag"}
                        <button
                          type="button"
                          aria-label={`Remove tag ${tagNameById.get(id) ?? ""}`}
                          onClick={() =>
                            void commitTags(tagIds.filter((t) => t !== id))
                          }
                        >
                          <X className="size-3" aria-hidden />
                        </button>
                      </Badge>
                    ))}
                  </div>
                )}
                <Input
                  id="card-review-tags"
                  className="mt-1 h-9"
                  placeholder="Add tags (Enter to confirm)"
                  value={tagInput}
                  onChange={(event) => setTagInput(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      event.preventDefault();
                      void addTagFromInput();
                    }
                  }}
                />

                <label
                  className="mt-3 block text-xs font-medium text-muted-foreground"
                  htmlFor="card-review-notes"
                >
                  Notes
                </label>
                <textarea
                  id="card-review-notes"
                  rows={2}
                  className="mt-1 w-full rounded-md border bg-background px-2 py-1.5 text-sm"
                  placeholder="Add context for future you…"
                  value={note}
                  onChange={(event) => setNote(event.target.value)}
                  onBlur={() => void flushNote()}
                />

                {(attachments.data?.length ?? 0) > 0 && (
                  <>
                    <p className="mt-3 text-xs font-medium text-muted-foreground">
                      Attachment
                    </p>
                    <ul className="mt-1 space-y-1">
                      {(attachments.data ?? []).map((attachment) => (
                        <li
                          key={attachment.id}
                          className="flex items-center gap-2 rounded-md border px-2 py-1.5 text-sm"
                        >
                          <FileText
                            className="size-4 shrink-0 text-muted-foreground"
                            aria-hidden
                          />
                          <span className="truncate">{attachment.original_filename ?? "Attachment"}</span>
                        </li>
                      ))}
                    </ul>
                  </>
                )}

                {error && (
                  <p role="alert" className="mt-3 text-xs text-loss">
                    {error}
                  </p>
                )}
              </div>
            )}

            <footer className="flex items-center justify-between gap-3 border-t px-5 py-3">
              <p className="flex items-center gap-3 text-xs text-muted-foreground">
                <span className="flex items-center gap-1">
                  <kbd className="rounded border px-1">←</kbd> Later
                </span>
                <span className="flex items-center gap-1">
                  <kbd className="rounded border px-1">→</kbd> Reviewed
                </span>
              </p>
              <div className="flex items-center gap-2">
                <Button variant="outline" disabled={busy} onClick={() => void later()}>
                  Later
                </Button>
                <Button disabled={busy} onClick={() => void review()}>
                  <Check aria-hidden />
                  Mark reviewed
                </Button>
              </div>
            </footer>
          </>
        )}
      </div>
      {creatingCategory !== null && (
        <CreateCategoryDialog
          initialName={creatingCategory}
          onCreated={(id) => void pickCategory(id)}
          onClose={() => setCreatingCategory(null)}
        />
      )}
    </div>
  );
}

function FinishedPanel({
  reviewed,
  clearedSetAside,
  onClose,
}: {
  reviewed: number;
  clearedSetAside: number;
  onClose: () => void;
}) {
  return (
    <div className="flex flex-col items-center px-8 py-10 text-center">
      <span className="flex size-14 items-center justify-center rounded-full bg-gain/10">
        <Check className="size-7 text-gain" aria-hidden />
      </span>
      <h3 className="mt-4 text-lg font-semibold">Inbox reviewed</h3>
      <p className="mt-1 max-w-xs text-sm text-muted-foreground">
        Every imported transaction has been categorized and cleared from your Money
        Inbox.
      </p>
      <div className="mt-5 flex gap-3">
        <div className="w-32 rounded-lg border px-4 py-3">
          <p className="text-2xl font-semibold text-gain tabular-nums">{reviewed}</p>
          <p className="text-xs text-muted-foreground">Reviewed</p>
        </div>
        {clearedSetAside > 0 && (
          <div className="w-32 rounded-lg border px-4 py-3">
            <p className="text-2xl font-semibold tabular-nums">{clearedSetAside}</p>
            <p className="text-xs text-muted-foreground">Set aside, then cleared</p>
          </div>
        )}
      </div>
      <Button className="mt-6" onClick={onClose}>
        Back to inbox
      </Button>
      <p className="mt-2 text-xs text-muted-foreground">
        Press <kbd className="rounded border px-1">Esc</kbd> to close
      </p>
    </div>
  );
}
