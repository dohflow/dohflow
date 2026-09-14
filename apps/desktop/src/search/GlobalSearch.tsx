import { useEffect, useState } from "react";
import { Loader2, Search, SearchX } from "lucide-react";

import { Input } from "@/components/ui/input";
import { EmptyState } from "@/components/ui/empty-state";
import {
  useTransactionPage,
  useTransactions,
} from "@/transactions/useTransactions";
import { useTags } from "@/tags/useTags";
import { useCategories } from "@/categories/useCategories";
import { TransactionDetailDrawer } from "@/transactions/TransactionDetailDrawer";
import { formatDate, formatSignedMoney, signedAmountClass } from "@/lib/format";
import { cn } from "@/lib/utils";

/// Results are capped so a one-letter query doesn't paint the whole ledger; the
/// palette is a jump-to surface, not a browse surface (that's the Transactions tab).
const MAX_RESULTS = 20;

/// The app-wide search palette (personal-cfo-z5lj): a Cmd+K overlay that matches
/// transactions on memo / counterparty / note / account name — the same semantics
/// as the Transactions filter bar, evaluated server-side over ALL history
/// (personal-cfo-3fdd.1) — and opens the full TransactionDetailDrawer on a hit,
/// from any tab. The host (UnlockedHome) owns the open state; this component owns
/// the query + drawer state.
export function GlobalSearch({ onClose }: { onClose: () => void }) {
  // Mutations only; the rows come from the server-side search below.
  const { recategorize, deleteTransaction, setReviewed } = useTransactions({
    list: false,
  });
  const { tags, createTag, setTags, setNote } = useTags();
  const { categories } = useCategories();
  const [query, setQuery] = useState("");
  const [detailId, setDetailId] = useState<string | null>(null);

  const trimmed = query.trim();
  const searchQuery = useTransactionPage(
    {
      query: trimmed === "" ? null : trimmed,
      account_ids: [],
      category_id: null,
      tag_id: null,
      recurring_event_id: null,
      from_date: null,
      to_date: null,
      unreviewed_only: false,
      sort: "newest",
      limit: MAX_RESULTS,
      offset: 0,
    },
    { enabled: trimmed !== "" },
  );
  const results = trimmed === "" ? [] : (searchQuery.data?.rows ?? []);

  // The drawer prop is a snapshot looked up per render, so edits made inside the
  // drawer (category, tags, note) reflect once the page cache refetches — the same
  // pattern MoneyInboxView uses.
  const detailTxn =
    detailId === null
      ? null
      : (results.find((t) => t.transaction_id === detailId) ?? null);

  // Esc peels one layer at a time: drawer first, then the palette. The drawer
  // registers its own Escape listener that closes *itself*, so while it's open
  // this handler only clears the local drawer state (idempotent with the
  // drawer's own close) and leaves the palette up.
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      if (detailId !== null) setDetailId(null);
      else onClose();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [detailId, onClose]);

  return (
    <>
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Search transactions"
        className="fixed inset-0 z-50 flex items-start justify-center bg-foreground/50 px-4 pt-[15vh] backdrop-blur-[3px]"
        onClick={onClose}
      >
        <div
          className="flex w-full max-w-xl flex-col overflow-hidden rounded-2xl border bg-popover shadow-2xl"
          onClick={(event) => event.stopPropagation()}
        >
          <div className="relative border-b">
            <Search
              className="pointer-events-none absolute left-4 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
              aria-hidden
            />
            <Input
              type="search"
              autoFocus
              placeholder="Search transactions…"
              aria-label="Search transactions"
              // The palette's chrome is the card itself — strip the input's own
              // border/ring so it reads as one surface.
              className="h-12 rounded-none border-0 bg-transparent pl-11 text-base shadow-none focus-visible:ring-0 focus-visible:ring-offset-0"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />
          </div>

          <div className="max-h-[50vh] overflow-y-auto p-2">
            {trimmed === "" ? (
              <p className="px-3 py-6 text-center text-sm text-muted-foreground">
                Search all your transactions
              </p>
            ) : searchQuery.data === undefined ? (
              <p className="flex items-center justify-center gap-2 px-3 py-6 text-sm text-muted-foreground">
                <Loader2 className="size-4 animate-spin" aria-hidden />
                Loading transactions…
              </p>
            ) : results.length === 0 ? (
              <EmptyState
                icon={SearchX}
                title="No matching transactions"
                description="Try a merchant, note, or account name."
                className="py-6"
              />
            ) : (
              <ul aria-label="Search results">
                {results.map((txn) => (
                  <li key={txn.transaction_id}>
                    <button
                      type="button"
                      onClick={() => setDetailId(txn.transaction_id)}
                      className="flex w-full items-center gap-3 rounded-md px-3 py-2 text-left transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    >
                      <span className="flex min-w-0 flex-1 flex-col">
                        <span className="truncate text-sm font-medium">
                          {txn.memo ?? txn.counterparty ?? txn.note ?? txn.account_name}
                        </span>
                        <span className="truncate text-xs text-muted-foreground">
                          {txn.account_name} · {formatDate(txn.occurred_at)}
                        </span>
                      </span>
                      <span
                        className={cn(
                          "shrink-0 text-sm font-medium tabular-nums",
                          signedAmountClass(txn.amount),
                        )}
                      >
                        {formatSignedMoney(txn.amount)}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
        </div>
      </div>

      {/* Rendered as a sibling (not inside the backdrop) so clicks in the drawer
          never bubble into the palette's close-on-backdrop handler. */}
      {detailTxn && (
        <TransactionDetailDrawer
          transaction={detailTxn}
          categories={categories}
          onRecategorize={recategorize}
          onDelete={deleteTransaction}
          onSetReviewed={setReviewed}
          tags={tags}
          onCreateTag={createTag}
          onSetTags={setTags}
          onSetNote={setNote}
          onClose={() => setDetailId(null)}
        />
      )}
    </>
  );
}
