import { useCategories } from "@/categories/useCategories";
import { useMarkReviewedBulk } from "@/money-inbox/useMoneyInbox";
import { useTags } from "@/tags/useTags";
import { BulkTransactionActions } from "./BulkTransactionActions";
import { useTransactions } from "./useTransactions";
import type { TransactionSelection } from "./useTransactionSelection";

/// The bulk-action bar for ONE transaction surface's selection (ADR 0049 §3).
///
/// Extracted from the former unified Transactions hub, which hosted a single bar over a
/// cross-list selection. With the Money Inbox and the Activity list on separate
/// destinations, each surface owns its own selection and renders its own bar — the
/// selection stays *controlled* by the shell (which is what keeps the inbox's
/// "Select all N in inbox" control and its visible-row freshness handling alive), while
/// the bar moves out of the dissolved hub to here.
export function TransactionBulkBar({
  selection,
}: {
  selection: TransactionSelection;
}) {
  const { recategorize, setReviewed, deleteTransaction } = useTransactions({
    list: false,
  });
  const { tags, setTags, createTag } = useTags();
  const { categories } = useCategories();
  const markReviewedBulk = useMarkReviewedBulk();

  if (selection.selectedCount === 0) return null;

  return (
    <BulkTransactionActions
      selected={selection.selectedRows}
      total={selection.visibleCount}
      categories={categories}
      tags={tags}
      recategorize={recategorize}
      setReviewed={setReviewed}
      deleteTransaction={deleteTransaction}
      setTags={setTags}
      createTag={createTag}
      bulkMarkReviewed={markReviewedBulk}
      onSelectAll={selection.selectAllVisible}
      onSelectionChange={selection.setSelectionIds}
    />
  );
}
