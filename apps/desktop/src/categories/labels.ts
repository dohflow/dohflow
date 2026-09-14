import type { CategoryDto } from "@/bindings";

/// Display metadata for one category — its own name plus the optional emoji icon +
/// color used for the row chip (personal-cfo-4d8.24.10).
export interface CategoryMeta {
  name: string;
  icon: string | null;
  color: string | null;
}

/// Display helpers for assigning + showing a category (personal-cfo-bac / -4d8.24.9).
///
/// - `label(id)` → the full "Parent / Leaf" path (top-level groups show just their own
///   name). Used where a category is shown out of context and needs disambiguation.
/// - `leafLabel(id)` → the category's OWN name only ("Groceries", never "Food and Drink /
///   Groceries"). Used for the transaction-row chip + inline renders, where the row's own
///   context makes the parent redundant (personal-cfo-4d8.24.9).
/// - `options` → the non-archived taxonomy as flat "Parent / Leaf" picker entries, sorted
///   by that label (retains parent context for pickers that stay flat).
/// - `groups` → the same taxonomy shaped for `<optgroup>`: each top-level category, with
///   its active leaves nested. A native `<select>` built from these shows the leaf's own
///   name when collapsed but disambiguates same-named leaves under their parent header
///   when open — the best of both for the row's inline category control.
///
/// Built from the `useCategories` list.
export function categoryLabels(categories: CategoryDto[]) {
  const byId = new Map(categories.map((category) => [category.id, category]));

  const label = (id: string | null): string | null => {
    if (id === null) return null;
    const category = byId.get(id);
    if (!category) return null;
    const parent = category.parent_id ? byId.get(category.parent_id) : null;
    return parent ? `${parent.name} / ${category.name}` : category.name;
  };

  // The category's own (leaf) name, with no parent prefix — `null` when unset/unknown.
  const leafLabel = (id: string | null): string | null => {
    if (id === null) return null;
    return byId.get(id)?.name ?? null;
  };

  const options = categories
    .filter((category) => !category.archived)
    .map((category) => ({ id: category.id, label: label(category.id) ?? category.name }))
    .sort((a, b) => a.label.localeCompare(b.label));

  // Grouped taxonomy for `<optgroup>` rendering. Every active category is bucketed under
  // its "display root" — the topmost active ancestor reached by climbing `parent_id`
  // through active categories (a category whose parent is null or archived IS its own
  // root). So a group root's bucket holds ALL its active descendants at any depth, not just
  // direct children — nothing can vanish from the picker even in a 3+-level tree
  // (personal-cfo-4d8.24.9). Every active category appears exactly once (a root, or a
  // descendant of one).
  const active = categories.filter((category) => !category.archived);
  const activeById = new Map(active.map((category) => [category.id, category]));
  const byName = (a: { name: string }, b: { name: string }) =>
    a.name.localeCompare(b.name);
  const displayRoot = (category: CategoryDto): CategoryDto => {
    let current = category;
    const seen = new Set<string>();
    while (
      current.parent_id &&
      activeById.has(current.parent_id) &&
      !seen.has(current.id)
    ) {
      seen.add(current.id); // guard against any (backend-blocked) cycle
      current = activeById.get(current.parent_id)!;
    }
    return current;
  };
  const descendantsByRoot = new Map<string, { id: string; name: string }[]>();
  for (const category of active) {
    const root = displayRoot(category);
    if (root.id === category.id) continue; // the root itself is not its own descendant
    const bucket = descendantsByRoot.get(root.id) ?? [];
    bucket.push({ id: category.id, name: category.name });
    descendantsByRoot.set(root.id, bucket);
  }
  const groups = active
    .filter((category) => displayRoot(category).id === category.id)
    .sort(byName)
    .map((root) => ({
      id: root.id,
      name: root.name,
      children: (descendantsByRoot.get(root.id) ?? []).sort(byName),
    }));

  // Flat id → display metadata (icon/color/name) for the row chip (personal-cfo-4d8.24.10).
  // Includes archived categories so a transaction on one still resolves its swatch.
  const metaById = new Map<string, CategoryMeta>(
    categories.map((category) => [
      category.id,
      { name: category.name, icon: category.icon, color: category.color },
    ]),
  );
  const meta = (id: string | null): CategoryMeta | null =>
    id === null ? null : (metaById.get(id) ?? null);

  return { label, leafLabel, options, groups, meta };
}
