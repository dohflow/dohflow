/**
 * personal-cfo-4d8.6.2 (AC3): a hand-rolled `role="dialog"` overlay has no focus containment, inert
 * background, Escape or focus return. Modals are built on the shared Dialog (`components/ui/dialog.tsx`),
 * so this scan fails on a NEW `role="dialog"` outside `components/ui/`.
 *
 * The allowlist is the views that have not moved yet, with how many `role="dialog"` each still has. It
 * only shrinks: a view that no longer has one must be taken off the list (the second test fails
 * otherwise), and the list reaches zero when the last modal has moved.
 */

import sourceTree from "virtual:source-tree";

const ALLOWLIST: Record<string, number> = {
  "accounts/AccountEditorDrawer.tsx": 1,
  "accounts/NewLoanDialog.tsx": 1,
  "accounts/ReconcileBalanceModal.tsx": 1,
  "accounts/SetBalanceModal.tsx": 1,
  "bills/BillDetailDrawer.tsx": 1,
  "categories/CreateCategoryDialog.tsx": 1,
  "dashboard/ReadinessCard.tsx": 1,
  "future-cash/SeriesPicker.tsx": 1,
  "money-inbox/CardReviewModal.tsx": 1,
  "money-inbox/DuplicateReviewPanel.tsx": 1,
  "money-inbox/ImportFileDialog.tsx": 1,
  "search/GlobalSearch.tsx": 1,
  "settings/NewMappedAccountDialog.tsx": 1,
  "transactions/TransactionDetailDrawer.tsx": 1,
  "vault/screens/VaultPickerScreen.tsx": 2,
};

const ROLE_DIALOG = /role="dialog"/g;

/// Production sources that declare a `role="dialog"`, by path under `src/`, with the count. The shared
/// primitives in `components/ui/`, tests and the dev harness pages are out of scope.
function declared(): Record<string, number> {
  const found: Record<string, number> = {};
  for (const file of sourceTree) {
    if (!file.path.endsWith(".tsx")) continue;
    if (file.path.startsWith("components/ui/")) continue;
    if (file.path.startsWith("dev/") || /\.test\.tsx$/.test(file.path)) continue;
    const count = file.text.match(ROLE_DIALOG)?.length ?? 0;
    if (count > 0) found[file.path] = count;
  }
  return found;
}

describe("hand-rolled dialogs (personal-cfo-4d8.6.2)", () => {
  it("reads the source tree", () => {
    expect(sourceTree.length).toBeGreaterThan(100); // guards the guard
  });

  it("no new role=\"dialog\" appears outside components/ui; use the shared Dialog", () => {
    const found = declared();
    const added = Object.entries(found).filter(([path, count]) => count > (ALLOWLIST[path] ?? 0));
    expect(
      added,
      "these views declare a hand-rolled role=\"dialog\"; build the modal on components/ui/dialog.tsx instead (or, for a drawer, a Sheet)",
    ).toEqual([]);
  });

  it("the allowlist only holds views that still have one: take a view off it when it moves", () => {
    const found = declared();
    const stale = Object.entries(ALLOWLIST).filter(([path, count]) => (found[path] ?? 0) < count);
    expect(stale, "these are allowlisted but no longer have that many role=\"dialog\"; remove or lower them").toEqual([]);
  });

  it("the scenario Apply and Delete confirmations are on the shared Dialog", () => {
    const text = sourceTree.find((f) => f.path === "scenarios/ScenariosView.tsx")?.text ?? "";
    expect(text).not.toMatch(ROLE_DIALOG);
    expect(text).toContain('from "@/components/ui/dialog"');
  });
});
