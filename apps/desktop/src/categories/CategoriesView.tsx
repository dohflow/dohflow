import { useMemo, useState, type FormEvent } from "react";
import { Loader2, Plus, Tags } from "lucide-react";

import type {
  CategoryDto,
  IpcError,
  MoveCategoryInput,
  UpdateCategoryInput,
} from "@/bindings";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card, CardContent } from "@/components/ui/card";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { describeIpcError } from "@/vault/useVault";
import { useCategories } from "./useCategories";
import { AddCategoryForm, CATEGORY_TYPES } from "./AddCategoryForm";
import { EmojiPicker } from "@/components/ui/emoji-picker";

const TYPE_LABEL: Record<string, string> = Object.fromEntries(
  CATEGORY_TYPES.map((t) => [t.value, t.label]),
);

const SELECT_CLASS =
  "flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background";

/// What every category row needs to act. Threaded down the recursive tree so a
/// node can edit/move/archive itself and offer a cycle-safe set of new parents.
type Handlers = {
  update: (input: UpdateCategoryInput) => Promise<IpcError | null>;
  move: (input: MoveCategoryInput) => Promise<IpcError | null>;
  archive: (id: string) => Promise<IpcError | null>;
  reinstate: (id: string) => Promise<IpcError | null>;
  /// Valid new parents for `id`: every non-archived category except itself and
  /// its descendants (a cycle), so the reparent picker can't offer an illegal
  /// move. Rust is still the hard guard (ADR 0030).
  parentChoices: (id: string) => CategoryDto[];
};

/// The category-management surface (plan §9.6, ADR 0030, personal-cfo-bac): the
/// seeded taxonomy as a tree plus full CRUD. System defaults carry a badge; their
/// identity (name/parent) is fixed but their appearance (color + icon) is editable
/// (kogu), and they can be archived. User categories can be renamed, recolored,
/// re-parented, and archived. The picker that assigns a category to a transaction
/// is a later slice (PR5).
export function CategoriesView() {
  const {
    categories,
    error,
    addCategory,
    updateCategory,
    moveCategory,
    archiveCategory,
    reinstateCategory,
  } = useCategories();
  const [adding, setAdding] = useState(false);
  const [showArchived, setShowArchived] = useState(false);

  // Build the parent → children index from the visible set. When archived are
  // hidden, an active child of a hidden parent is promoted to the top level so it
  // can never disappear from the list.
  const { roots, byParent, parentChoices } = useMemo(() => {
    const all = categories ?? [];
    const visible = showArchived ? all : all.filter((c) => !c.archived);
    const visibleIds = new Set(visible.map((c) => c.id));
    const index = new Map<string | null, CategoryDto[]>();
    for (const category of visible) {
      const key =
        category.parent_id && visibleIds.has(category.parent_id)
          ? category.parent_id
          : null;
      const siblings = index.get(key) ?? [];
      siblings.push(category);
      index.set(key, siblings);
    }
    // Descendant set of a node (over ALL categories, so a hidden subtree still
    // counts as off-limits when picking a new parent).
    const childrenOf = new Map<string | null, CategoryDto[]>();
    for (const category of all) {
      const list = childrenOf.get(category.parent_id) ?? [];
      list.push(category);
      childrenOf.set(category.parent_id, list);
    }
    const descendantsOf = (id: string): Set<string> => {
      const out = new Set<string>();
      const stack = [...(childrenOf.get(id) ?? [])];
      while (stack.length) {
        const next = stack.pop()!;
        if (out.has(next.id)) continue;
        out.add(next.id);
        stack.push(...(childrenOf.get(next.id) ?? []));
      }
      return out;
    };
    const choices = (id: string): CategoryDto[] => {
      const banned = descendantsOf(id);
      return all.filter(
        (c) => !c.archived && c.id !== id && !banned.has(c.id),
      );
    };
    return {
      roots: index.get(null) ?? [],
      byParent: index,
      parentChoices: choices,
    };
  }, [categories, showArchived]);

  const handlers: Handlers = {
    update: updateCategory,
    move: moveCategory,
    archive: archiveCategory,
    reinstate: reinstateCategory,
    parentChoices,
  };

  return (
    <div className="mx-auto flex w-full max-w-2xl flex-col gap-4">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold tracking-tight">Categories</h2>
        {!adding && (
          <Button size="sm" onClick={() => setAdding(true)}>
            <Plus aria-hidden />
            Add category
          </Button>
        )}
      </div>

      <p className="text-sm text-muted-foreground">
        Organize spending and income. The defaults are ready to use; add your own
        or hide the ones you don&apos;t need. Hidden categories keep their history.
      </p>

      {adding && (
        <AddCategoryForm
          parents={categories?.filter((c) => !c.archived) ?? []}
          onCancel={() => setAdding(false)}
          onCreate={async (input) => {
            const { error: failure } = await addCategory(input);
            if (!failure) setAdding(false);
            return failure ? describeIpcError(failure) : null;
          }}
        />
      )}

      {error && (
        <p role="alert" className="text-sm text-loss">
          {error}
        </p>
      )}

      {categories === null ? (
        <div className="flex items-center justify-center gap-2 py-10 text-muted-foreground">
          <Loader2 className="size-5 animate-spin" aria-hidden />
          Loading categories…
        </div>
      ) : roots.length === 0 ? (
        !adding && (
          <Card>
            <CardContent className="flex flex-col items-center gap-2 py-10 text-center">
              <Tags className="size-8 text-muted-foreground" aria-hidden />
              <p className="font-medium">No categories</p>
              <p className="text-sm text-muted-foreground">
                Add a category to start organizing transactions.
              </p>
            </CardContent>
          </Card>
        )
      ) : (
        <>
          <label className="flex items-center gap-2 text-sm text-muted-foreground">
            <input
              type="checkbox"
              className="size-4 rounded border-input"
              checked={showArchived}
              onChange={(event) => setShowArchived(event.target.checked)}
            />
            Show archived
          </label>
          <Card>
            <CardContent className="p-0">
              <ul>
                {roots.map((node) => (
                  <CategoryNode
                    key={node.id}
                    node={node}
                    depth={0}
                    byParent={byParent}
                    handlers={handlers}
                  />
                ))}
              </ul>
            </CardContent>
          </Card>
        </>
      )}
    </div>
  );
}

// A single category row plus its subtree. System categories show a "Default" badge;
// their identity (name/parent) is fixed, so their inline Edit exposes appearance
// (color + icon) only (ADR 0030 amendment, kogu). User categories add inline Edit for
// rename + recolor + re-icon + reparent. Archived rows are dimmed.
function CategoryNode({
  node,
  depth,
  byParent,
  handlers,
}: {
  node: CategoryDto;
  depth: number;
  byParent: Map<string | null, CategoryDto[]>;
  handlers: Handlers;
}) {
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const children = byParent.get(node.id) ?? [];

  async function run(
    action: () => Promise<IpcError | null>,
  ): Promise<IpcError | null> {
    setBusy(true);
    setError(null);
    const failure = await action();
    setBusy(false);
    if (failure) setError(describeIpcError(failure));
    return failure;
  }

  return (
    <>
      <li
        className={`flex flex-col gap-1 border-b px-5 py-3 last:border-0 ${
          node.archived ? "opacity-60" : ""
        }`}
      >
        {editing ? (
          <CategoryEditor
            node={node}
            parents={handlers.parentChoices(node.id)}
            onCancel={() => {
              setEditing(false);
              setError(null);
            }}
            onSave={async ({ name, color, icon, parentId }) => {
              const newColor = color || null;
              const newIcon = icon || null;
              const fieldsChanged =
                name !== node.name ||
                newColor !== (node.color ?? null) ||
                newIcon !== (node.icon ?? null);
              if (fieldsChanged) {
                const failure = await run(() =>
                  handlers.update({
                    id: node.id,
                    name,
                    color: newColor,
                    icon: newIcon,
                    idempotency_key: mintIdempotencyKey(),
                  }),
                );
                if (failure) return;
              }
              // A system category can't be re-parented (its identity is fixed); only user
              // categories reach MoveCategory (personal-cfo-kogu).
              if (!node.is_system && parentId !== (node.parent_id ?? "")) {
                const failure = await run(() =>
                  handlers.move({
                    id: node.id,
                    new_parent_id: parentId || null,
                    idempotency_key: mintIdempotencyKey(),
                  }),
                );
                if (failure) return;
              }
              setEditing(false);
            }}
          />
        ) : (
          <div
            className="flex items-center justify-between gap-3"
            style={{ paddingLeft: depth * 16 }}
          >
            <div className="flex items-center gap-2">
              {node.color && (
                <span
                  aria-hidden
                  className="size-3 shrink-0 rounded-full border"
                  style={{ backgroundColor: node.color }}
                />
              )}
              <span className="font-medium">{node.name}</span>
              <span className="text-xs text-muted-foreground">
                {TYPE_LABEL[node.category_type] ?? node.category_type}
              </span>
              {node.is_system && (
                <span className="rounded-full bg-muted px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                  Default
                </span>
              )}
              {node.archived && (
                <span className="rounded-full bg-muted px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                  Archived
                </span>
              )}
            </div>
            <div className="flex gap-1">
              {node.archived ? (
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy}
                  onClick={() => run(() => handlers.reinstate(node.id))}
                  aria-label={`Reinstate ${node.name}`}
                >
                  Reinstate
                </Button>
              ) : (
                <>
                  {/* Every category is editable; for a system ("Default") category the
                      editor only exposes appearance (color + icon) — its name/parent stay
                      fixed (ADR 0030 amendment, personal-cfo-kogu). */}
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => setEditing(true)}
                    aria-label={`Edit ${node.name}`}
                  >
                    Edit
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={busy}
                    onClick={() => run(() => handlers.archive(node.id))}
                    aria-label={`Archive ${node.name}`}
                  >
                    Archive
                  </Button>
                </>
              )}
            </div>
          </div>
        )}
        {error && (
          <p role="alert" className="text-xs text-loss">
            {error}
          </p>
        )}
      </li>
      {children.map((child) => (
        <CategoryNode
          key={child.id}
          node={child}
          depth={depth + 1}
          byParent={byParent}
          handlers={handlers}
        />
      ))}
    </>
  );
}

// Inline editor. For a user category: rename + recolor + re-icon + reparent. For a
// system ("Default") category: appearance only — the name is read-only and the parent
// picker is hidden (its identity is immutable, ADR 0030 amendment / personal-cfo-kogu).
// Save composes an UpdateCategory (name/color/icon) and, for user categories, a
// MoveCategory (parent) only for the fields that changed.
function CategoryEditor({
  node,
  parents,
  onCancel,
  onSave,
}: {
  node: CategoryDto;
  parents: CategoryDto[];
  onCancel: () => void;
  onSave: (values: {
    name: string;
    color: string;
    icon: string;
    parentId: string;
  }) => Promise<void>;
}) {
  const [name, setName] = useState(node.name);
  const [color, setColor] = useState(node.color ?? "");
  const [icon, setIcon] = useState(node.icon ?? "");
  const [parentId, setParentId] = useState(node.parent_id ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(event: FormEvent) {
    event.preventDefault();
    const trimmed = name.trim();
    if (!trimmed) {
      setError("Enter a category name.");
      return;
    }
    setBusy(true);
    await onSave({ name: trimmed, color: color.trim(), icon: icon.trim(), parentId });
    setBusy(false);
  }

  return (
    <form onSubmit={submit} className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        {/* A system category's name is fixed — shown read-only (personal-cfo-kogu). */}
        <Input
          value={name}
          onChange={(event) => setName(event.target.value)}
          autoFocus
          readOnly={node.is_system}
          aria-label="Category name"
          className={`w-44 ${node.is_system ? "text-muted-foreground" : ""}`}
        />
        {/* Emoji icon: pick-only (personal-cfo-4d8.25.20) — free text can no
            longer become an icon. */}
        <EmojiPicker
          aria-label="Category icon"
          value={icon}
          onSelect={setIcon}
          onClear={() => setIcon("")}
        />
        {/* Color PICKER (native, dep-free). The swatch value falls back to grey for
            display only; `color` state stays "" until the user picks, so an untouched
            null-color category is not silently assigned a color. */}
        <input
          type="color"
          aria-label="Category color"
          value={color || "#808080"}
          onChange={(event) => setColor(event.target.value)}
          className="h-9 w-10 shrink-0 cursor-pointer rounded-md border border-input bg-background p-1"
        />
        {/* Read-only hex, so the picked value is visible + copyable for reuse. */}
        <Input
          value={color}
          readOnly
          aria-label="Category color hex"
          placeholder="#RRGGBB"
          className="w-28"
        />
        {/* A native color picker can't represent "no color", so a Clear button is the
            only way to set the color back to null (the DTO's "null to clear it"
            contract) — parallel to emptying the emoji field. */}
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
        {!node.is_system && (
          <select
            className={`${SELECT_CLASS} w-auto`}
            value={parentId}
            aria-label="Parent category"
            onChange={(event) => setParentId(event.target.value)}
          >
            <option value="">Top level</option>
            {parents.map((parent) => (
              <option key={parent.id} value={parent.id}>
                {parent.name}
              </option>
            ))}
          </select>
        )}
        <Button size="sm" type="submit" disabled={busy}>
          Save
        </Button>
        <Button size="sm" variant="ghost" type="button" onClick={onCancel}>
          Cancel
        </Button>
      </div>
      {error && (
        <p role="alert" className="text-xs text-loss">
          {error}
        </p>
      )}
    </form>
  );
}

// Add a new user category at the top level or under a parent. Type defaults to
// expense (the common case); Rust validates the token set (ADR 0030 / 0003).
