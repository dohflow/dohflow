# Surface audit — every list, table and card

- **Bead:** `personal-cfo-4d8.27.4.3`
- **Date:** 2026-08-01
- **Feeds:** `4d8.27.4.4` (the convergence decision) and `4d8.27.4.2` (the DataTable primitive)

An inventory of every surface that renders a *collection*, so convergence is decided from
what is actually there rather than from impression. Forms, single cards, modals and charts
are out of scope.

## The headline: the shared state primitives are barely used

`EmptyState` and `Skeleton` exist in `components/ui/`, and almost nothing consumes them.

| Primitive | Consumers (non-test) |
| --- | --- |
| `EmptyState` | `accounts/AccountDetailView`, `dashboard/DashboardView`, `search/GlobalSearch` |
| `Skeleton` | `accounts/AccountDetailView` — **one** |
| `PaginationControls` | `AccountDetailView`, `BillDetailDrawer`, `ProjectedActivityTable`, `MoneyInboxView`, `TransactionsView` |

Meanwhile **all four screens named in `4d8.27.4.4` hand-roll a `Loader2` spinner** —
`BillsView` (×3), `AccountsView` (×2), `IncomeView`, `CategoriesView` — and none of them
uses `EmptyState` or `Skeleton` at all.

That is the finding that matters most. The inconsistency people notice is not really
"table vs list"; it is that the same screen states — *loading*, *empty*, *error* — are
re-invented per surface, so the app flickers between a centred spinner, a bare sentence,
and nothing at all. A shared table primitive is worth building mainly because it makes
those three states impossible to get wrong, not because tables are prettier than lists.

## Real tables

Everything below renders `<table>` via `components/ui/table`.

| Surface | File | Header? | Sort | Pagination | Verdict |
| --- | --- | --- | --- | --- | --- |
| Transactions (Activity) | `transactions/TransactionsView.tsx` | yes, 8 cols | filter bar (server-side, whole result set) | shared, server-paged | **on DataTable, states included** ✅ |
| Projected Activity | `future-cash/ProjectedActivityTable.tsx` | yes, dynamic cols | **forbidden** — running balances are order-dependent | shared, client-paged | **on DataTable** ✅ (`trailingRow`) |
| Money Inbox | `money-inbox/MoneyInboxView.tsx` | **deliberately none** | none | shared | **stays** — polymorphic rows with shared state; see below |
| Scenarios | `scenarios/ScenariosView.tsx` | yes, 5 cols | none | none (bounded) | **on DataTable** ✅ |
| Debt payoff compare | `accounts/DebtPayoffCompare.tsx` | yes | none | none (bounded) | **on DataTable** ✅ |
| Account detail activity | `accounts/AccountDetailView.tsx` | yes | none | shared | **on DataTable** ✅ (`leadingRow`) |

**Money Inbox does NOT migrate. Decided 2026-08-02 (`personal-cfo-9krd`).**

It renders *four polymorphic row components* — one per inbox item kind — rather than one
row shape with varying data. It was scoped to go last on the assumption that this was a
harder swap. Grounding it found something else: it is not a swap at all.

The bead was filed on a narrower question — a Money Inbox row can emit **two** full-width
rows (an always-visible error that survives a collapsed row, plus the expander) where
`expandedContent` offers one. That question has a clean answer: fold both into a single
`expandedContent` return, which the caller controls, so the error still shows while
collapsed. No primitive change needed.

**The actual blocker is state.** Each row component holds its own `useState` — thirteen
across the four — and each row's `pending` / `error` is **shared between its action cell
and its full-width error row**. `DataTable` renders every cell as `columns[].cell(row)`: a
plain function that cannot hold hooks, each separately wrapped in its own `<TableCell>`,
so no per-kind component can span cells. The column-def API assumes cells are independent
pure renders. These are not.

Migrating anyway would mean lifting four different state shapes into one `item_id`-keyed
map in a file already 1,319 lines long, against a 621-line test suite. What it would buy
is three hand-written `colSpan={5}` literals becoming derived. That trade is not worth it,
so the literals are removed directly instead (a shared `INBOX_COLUMN_COUNT`), and the
surface keeps the raw `ui/table` primitives.

**`hideHeader` is therefore deleted from `DataTable`.** It existed solely for this surface,
which is not coming — and the rule below says to delete unproven slots rather than keep
them as decoration.

*Revisit if* Money Inbox's rows ever converge on one shape, or if a second genuinely
headerless collection appears.

Two properties recur and are exactly what a primitive should own:

- **Expander rows whose span is hand-maintained.** Transactions carried a literal
  `colSpan={8}` with a comment asking the next author to keep it in sync — it had to be
  edited by hand every time the column count changed, and nothing would have caught it if
  someone hadn't. (Projected Activity avoids the literal by computing
  `3 + columns.length`, which is the same maintenance burden expressed arithmetically.)
  The primitive derives the span from the column list, so neither form is needed.
- **A "filtered to nothing" row that is not an empty table.** Projected Activity keeps a
  pinned TODAY row and shows a muted sentence when the filter excludes every *activity*.
  The primitive's `empty` fires only when there are no rows at all, so that surface needs
  either a `trailingRow` slot or to fold TODAY into its rows — an open question for its
  migration, recorded here rather than discovered then.
  **Answered in `personal-cfo-wxy7`: both.** TODAY *is* one of the rows, because its cells
  are the same per-tier running balances every other row renders and therefore have to
  come from the column defs — pinning it in `leadingRow` would mean hand-rolling a cell
  per balance column, the exact drift the primitive exists to prevent. That makes
  `rows.length` never 0, so `empty` can never fire there, so the notice is a `trailingRow`
  (full-width, span owned by the primitive).
- **A row-level accent.** Projected Activity colours each row's left border by activity
  kind (`border-l-gain` / `border-l-loss` / `border-l-primary`). The primitive's
  `rowProps` returns a `className`, so this survives migration without a special case.
- **Sorting is not a table concern here.** Transactions sorts in the filter bar because
  it sorts *server-side over the whole result set*, not the page in view; Projected
  Activity forbids re-sorting outright because its rows carry running balances that would
  become false. So sorting is **opt-in per column** in the primitive and never assumed.

## List-card surfaces

`<ul>`/`<li>` or a `Card` per row.

| Surface | File | States today | Verdict |
| --- | --- | --- | --- |
| Bills | `bills/BillsView.tsx` | spinner ×4, bespoke empty | **migrate** (see ADR 0053) |
| Income | `income/IncomeView.tsx` | spinner ×2, bespoke empty | **migrate** |
| Categories | `categories/CategoriesView.tsx` | spinner ×2 | **stay** — it is a tree |
| Accounts | `accounts/AccountsView.tsx` | spinner ×3 | **stay** — two-column, grouped |
| Recurring Transfers | `transactions/RecurringTransfersView.tsx` | spinner | migrate (with Bills/Income) |
| Suggested Recurring | `bills/SuggestedRecurring.tsx` | inline | stay — a prompt, not a collection |
| Future Cash entries | `future-cash/FutureCashEntries.tsx` | inline | stay — small, bounded |
| Scenario events | `future-cash/ScenarioEvents.tsx` | inline | stay — small, bounded |
| Dashboard upcoming lists | `dashboard/DashboardView.tsx` | uses `EmptyState` | stay — glance surface |
| Vaults / vault health | `settings/VaultsCard.tsx`, `VaultHealthCard.tsx` | inline | stay |
| Global search results | `search/GlobalSearch.tsx` | uses `EmptyState` | stay — a palette |
| Card review queue | `money-inbox/CardReviewModal.tsx` | bespoke | stay — a queue, not a list |
| Loan double-count warning | `accounts/LoanDoubleCountWarning.tsx` | inline | stay — an alert |
| Detail drawers | `transactions/TransactionDetailDrawer.tsx`, `bills/BillDetailDrawer.tsx` | inline | stay — detail, not collection |

## Token deviations

Spot-checked rather than exhaustive; a full sweep is `4d8.27.4.7`.

- The build badge uses `var(--terracotta)` directly with an inline `style` because the
  token has no Tailwind color alias (`settings/BuildBadge.tsx`). Deliberate, documented.
- `ProjectedActivityTable` uses semantic border tokens per activity kind
  (`border-l-gain` / `border-l-loss` / `border-l-primary`) — correct, and worth copying.
- Loading spinners are `text-muted-foreground` everywhere they appear, so the deviation
  is structural (spinner vs skeleton), not chromatic.

## What this implies

1. Build the primitive so the four states and the expander span are structural. *(done —
   `4d8.27.4.2`)*
2. Migrate the six real tables onto it, one per change, starting with the two named
   consumers. *(**complete**: five migrated in `4d8.27.4.2` / `wxy7` — Transactions,
   Scenarios, Projected Activity, Debt payoff compare, Account detail. The sixth, Money
   Inbox, was examined in `9krd` and deliberately **stays** — its rows are polymorphic with
   state shared across cells, which a column-def API cannot host.)*
3. Decide list-card convergence per screen rather than wholesale — see ADR 0053.

## Ship code with a consumer, or do not ship it

`personal-cfo-4d8.27.4.2`'s review caught that the primitive's four-state code had **zero
production consumers** and wired it up. Three *other* slots shipped in that same change
with none: `leadingRow`, `footer` and `hideHeader` were exercised only by the primitive's
own tests. `wxy7` gave the first two real consumers (Account detail's pinned Upcoming
block; Projected Activity's and Account detail's pagination) and added `trailingRow` only
alongside the surface that needed it.

**`hideHeader` was deleted (`personal-cfo-9krd`, 2026-08-02).** It existed solely for the
Money Inbox, and that surface does not migrate (see above) — so the slot had no consumer
and never would. Deleted rather than kept as decoration, which is what this section says
to do. The rule held in both directions this time: the primitive gained `trailingRow` only
alongside the surface that needed it, and lost `hideHeader` when its surface said no.
