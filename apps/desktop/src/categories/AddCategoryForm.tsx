import { useState, type FormEvent } from "react";

import type { CategoryDto, CreateCategoryInput } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { EmojiPicker } from "@/components/ui/emoji-picker";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { mintIdempotencyKey } from "@/lib/idempotency";

export const CATEGORY_TYPES = [
  { value: "expense", label: "Expense" },
  { value: "income", label: "Income" },
  { value: "transfer", label: "Transfer" },
  { value: "adjustment", label: "Adjustment" },
] as const;

const SELECT_CLASS =
  "flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background";

/// The add-category form (extracted from CategoriesView for reuse by the
/// create-from-picker dialog, personal-cfo-4d8.25.19). The icon is chosen via the
/// click-only EmojiPicker — free text can no longer become an icon
/// (personal-cfo-4d8.25.20).
export function AddCategoryForm({
  parents,
  initialName = "",
  submitLabel = "Add category",
  onCancel,
  onCreate,
}: {
  parents: CategoryDto[];
  /// Prefill for the name field (the picker passes its search text).
  initialName?: string;
  submitLabel?: string;
  onCancel: () => void;
  onCreate: (input: CreateCategoryInput) => Promise<string | null>;
}) {
  const [name, setName] = useState(initialName);
  const [categoryType, setCategoryType] = useState("expense");
  const [parentId, setParentId] = useState("");
  const [color, setColor] = useState("");
  const [icon, setIcon] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    const trimmed = name.trim();
    if (!trimmed) {
      setError("Enter a category name.");
      return;
    }
    setBusy(true);
    setError(null);
    const failure = await onCreate({
      parent_id: parentId || null,
      name: trimmed,
      category_type: categoryType,
      color: color.trim() || null,
      icon: icon || null,
      idempotency_key: mintIdempotencyKey(),
    });
    setBusy(false);
    if (failure) setError(failure);
  }

  return (
    <Card>
      <CardContent className="pt-6">
        <form onSubmit={submit} className="flex flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="category-name">Category name</Label>
            <Input
              id="category-name"
              autoFocus
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder="e.g. Coffee shops"
              aria-invalid={!!error}
            />
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="category-type">Type</Label>
              <select
                id="category-type"
                className={SELECT_CLASS}
                value={categoryType}
                onChange={(event) => setCategoryType(event.target.value)}
              >
                {CATEGORY_TYPES.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="category-parent">Parent (optional)</Label>
              <select
                id="category-parent"
                className={SELECT_CLASS}
                value={parentId}
                onChange={(event) => setParentId(event.target.value)}
              >
                <option value="">Top level</option>
                {parents.map((parent) => (
                  <option key={parent.id} value={parent.id}>
                    {parent.name}
                  </option>
                ))}
              </select>
            </div>
          </div>

          {/* Appearance: a pick-only emoji + a native color picker, set at creation
              (personal-cfo-kogu; picker-only per personal-cfo-4d8.25.20). */}
          <div className="grid grid-cols-2 gap-3">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="category-icon">Icon (optional)</Label>
              <EmojiPicker
                id="category-icon"
                aria-label="Category icon"
                value={icon}
                onSelect={setIcon}
                onClear={() => setIcon("")}
              />
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="category-color">Color (optional)</Label>
              <div className="flex items-center gap-2">
                <input
                  type="color"
                  id="category-color"
                  aria-label="Category color"
                  value={color || "#808080"}
                  onChange={(event) => setColor(event.target.value)}
                  className="h-9 w-10 shrink-0 cursor-pointer rounded-md border border-input bg-background p-1"
                />
                <Input
                  value={color}
                  readOnly
                  aria-label="Category color hex"
                  placeholder="#RRGGBB"
                  className="w-28"
                />
                {color !== "" && (
                  <Button
                    size="sm"
                    variant="ghost"
                    type="button"
                    aria-label="Clear category color"
                    onClick={() => setColor("")}
                  >
                    Clear
                  </Button>
                )}
              </div>
            </div>
          </div>

          {error && (
            <p role="alert" className="text-sm text-loss">
              {error}
            </p>
          )}

          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={onCancel}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy}>
              {busy ? "Adding…" : submitLabel}
            </Button>
          </div>
        </form>
      </CardContent>
    </Card>
  );
}
