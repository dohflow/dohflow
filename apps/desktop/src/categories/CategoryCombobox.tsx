import { useMemo } from "react";

import type { CategoryDto } from "@/bindings";
import { Combobox, type ComboboxItem } from "@/components/ui/combobox";

interface CategoryComboboxProps {
  categories: CategoryDto[];
  value: string | null;
  onSelect: (id: string | null) => void;
  "aria-label": string;
  id?: string;
  placeholder?: string;
  /// Offer an explicit empty choice (omit on surfaces where clearing makes no
  /// sense, e.g. the bulk bar's fire-once action).
  clearLabel?: string;
  /// Renders the persistent "Create new category…" footer (personal-cfo-4d8.25.19);
  /// receives the current search text so the dialog can prefill the name.
  onCreateNew?: (query: string) => void;
  buttonClassName?: string;
  disabled?: boolean;
}

/// The searchable category picker (personal-cfo-4d8.25.18): typing a LEAF name
/// ("Groceries") matches directly — no parent-path prefix needed — while every
/// result still shows its parent context because leaf names are not unique
/// (ADR 0045 §3: "Maintenance" exists under both Housing and Transportation).
export function CategoryCombobox({
  categories,
  value,
  onSelect,
  id,
  placeholder = "Category",
  clearLabel,
  onCreateNew,
  buttonClassName,
  disabled,
  "aria-label": ariaLabel,
}: CategoryComboboxProps) {
  const items = useMemo<ComboboxItem[]>(() => {
    const byId = new Map(categories.map((c) => [c.id, c]));
    const path = (category: CategoryDto): string[] => {
      const names: string[] = [];
      let cursor: CategoryDto | undefined = category;
      // Bounded by taxonomy depth; guards a (never-expected) parent cycle.
      for (let hops = 0; cursor && hops < 8; hops += 1) {
        names.unshift(cursor.name);
        cursor = cursor.parent_id === null ? undefined : byId.get(cursor.parent_id);
      }
      return names;
    };
    return categories
      .filter((c) => !c.archived)
      .map((c) => {
        const names = path(c);
        const parents = names.slice(0, -1).join(" / ");
        return {
          id: c.id,
          label: c.name,
          hint: parents === "" ? undefined : parents,
          searchText: names.join(" / "),
        };
      })
      .sort((a, b) =>
        (a.searchText ?? a.label).localeCompare(b.searchText ?? b.label),
      );
  }, [categories]);

  return (
    <Combobox
      items={items}
      value={value}
      onSelect={onSelect}
      placeholder={placeholder}
      clearLabel={clearLabel}
      footerAction={
        onCreateNew ? { label: "Create new category…", onAction: onCreateNew } : undefined
      }
      buttonClassName={buttonClassName}
      disabled={disabled}
      id={id}
      aria-label={ariaLabel}
    />
  );
}
