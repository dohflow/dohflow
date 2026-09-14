import { useRef, useState } from "react";
import { Plus, Trash2 } from "lucide-react";

import type {
  CategoryDto,
  IpcError,
  SplitLineDto,
  SplitLineInputDto,
  TagViewDto,
  TransactionRowDto,
} from "@/bindings";
import { Button } from "@/components/ui/button";
import { describeIpcError } from "@/vault/useVault";
import { dollarsToMinorUnits, formatMoney } from "@/lib/format";
import { categoryLabels } from "@/categories/labels";
import { TagChip } from "@/tags/TagChip";

const INPUT_CLASS =
  "flex h-10 rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background";

type DraftLine = {
  id: number;
  amount: string;
  categoryId: string;
  note: string;
  tagIds: string[];
};

/// The split editor (ADR 0034, personal-cfo-4d8.18 / -4d8.19). The user works in positive
/// magnitudes; the transaction's sign is applied on save. Lines must sum to the
/// transaction amount before saving. Each line carries its own category, note, and tags.
export function SplitEditor({
  transaction,
  categories,
  tags,
  existing,
  onCreateTag,
  onSave,
  onClear,
  onClose,
}: {
  transaction: TransactionRowDto;
  categories: CategoryDto[] | null;
  tags: TagViewDto[] | null;
  existing: SplitLineDto[];
  onCreateTag: (name: string) => Promise<string | IpcError>;
  onSave: (lines: SplitLineInputDto[]) => Promise<IpcError | null>;
  onClear: () => Promise<IpcError | null>;
  onClose: () => void;
}) {
  const currency = transaction.amount.currency;
  const sign = transaction.amount.minor_units < 0 ? -1 : 1;
  const txnMagnitude = Math.abs(transaction.amount.minor_units);

  const [lines, setLines] = useState<DraftLine[]>(() => {
    let id = 0;
    if (existing.length > 0) {
      return existing.map((line) => ({
        id: id++,
        amount: (Math.abs(line.amount.minor_units) / 100).toFixed(2),
        categoryId: line.category_id ?? "",
        note: line.note ?? "",
        tagIds: line.tag_ids,
      }));
    }
    return [
      {
        id: 0,
        amount: (txnMagnitude / 100).toFixed(2),
        categoryId: "",
        note: "",
        tagIds: [],
      },
      { id: 1, amount: "", categoryId: "", note: "", tagIds: [] },
    ];
  });
  const nextId = useRef(lines.length);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { options } = categoryLabels(categories ?? []);

  const lineMinor = (line: DraftLine) =>
    Math.abs(dollarsToMinorUnits(line.amount) ?? 0);
  const sumMinor = lines.reduce((acc, line) => acc + lineMinor(line), 0);
  const remaining = txnMagnitude - sumMinor;
  const balanced =
    remaining === 0 && lines.length >= 2 && lines.every((l) => lineMinor(l) > 0);

  function update(id: number, patch: Partial<DraftLine>) {
    setLines((prev) =>
      prev.map((line) => (line.id === id ? { ...line, ...patch } : line)),
    );
  }
  function addLine() {
    // Prefill with whatever is still unallocated, so the split tends to balance.
    const fill = remaining > 0 ? (remaining / 100).toFixed(2) : "";
    setLines((prev) => [
      ...prev,
      { id: nextId.current++, amount: fill, categoryId: "", note: "", tagIds: [] },
    ]);
  }
  function removeLine(id: number) {
    setLines((prev) => prev.filter((line) => line.id !== id));
  }

  async function save() {
    setSaving(true);
    setError(null);
    const payload: SplitLineInputDto[] = lines.map((line) => ({
      amount: { minor_units: sign * lineMinor(line), currency },
      category_id: line.categoryId || null,
      note: line.note.trim() || null,
      tag_ids: line.tagIds,
    }));
    const failure = await onSave(payload);
    setSaving(false);
    if (failure) setError(describeIpcError(failure));
    else onClose();
  }

  async function clear() {
    setSaving(true);
    setError(null);
    const failure = await onClear();
    setSaving(false);
    if (failure) setError(describeIpcError(failure));
    else onClose();
  }

  return (
    <div className="flex flex-col gap-3">
      {lines.map((line, index) => (
        <div key={line.id} className="flex flex-col gap-2 rounded-md border p-2">
          <div className="flex items-center gap-2">
            <input
              inputMode="decimal"
              value={line.amount}
              onChange={(event) =>
                update(line.id, { amount: event.target.value })
              }
              placeholder="0.00"
              aria-label={`Split ${index + 1} amount`}
              className={`${INPUT_CLASS} w-24`}
            />
            <select
              value={line.categoryId}
              onChange={(event) =>
                update(line.id, { categoryId: event.target.value })
              }
              aria-label={`Split ${index + 1} category`}
              className={`${INPUT_CLASS} flex-1`}
            >
              <option value="">Uncategorized</option>
              {options.map((option) => (
                <option key={option.id} value={option.id}>
                  {option.label}
                </option>
              ))}
            </select>
            {lines.length > 1 && (
              <button
                type="button"
                onClick={() => removeLine(line.id)}
                aria-label={`Remove split ${index + 1}`}
                className="rounded-md p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
              >
                <Trash2 className="size-4" aria-hidden />
              </button>
            )}
          </div>
          <input
            value={line.note}
            onChange={(event) => update(line.id, { note: event.target.value })}
            maxLength={4096}
            placeholder="Note (optional)"
            aria-label={`Split ${index + 1} note`}
            className={`${INPUT_CLASS} w-full`}
          />
          <LineTags
            label={`Split ${index + 1}`}
            tagIds={line.tagIds}
            tags={tags}
            onCreateTag={onCreateTag}
            onChange={(tagIds) => update(line.id, { tagIds })}
          />
        </div>
      ))}

      <datalist id="split-line-tag-options">
        {(tags ?? [])
          .filter((tag) => !tag.archived)
          .map((tag) => (
            <option key={tag.id} value={tag.name} />
          ))}
      </datalist>

      <button
        type="button"
        onClick={addLine}
        className="inline-flex items-center gap-1 self-start text-sm text-muted-foreground hover:text-foreground"
      >
        <Plus className="size-4" aria-hidden />
        Add line
      </button>

      <div
        className={`text-sm ${balanced ? "text-gain" : "text-muted-foreground"}`}
        role="status"
      >
        {formatMoney({ minor_units: sumMinor, currency })} of{" "}
        {formatMoney({ minor_units: txnMagnitude, currency })}
        {remaining !== 0 &&
          ` · ${formatMoney({ minor_units: Math.abs(remaining), currency })} ${
            remaining > 0 ? "left" : "over"
          }`}
      </div>

      {error && (
        <p role="alert" className="text-sm text-loss">
          {error}
        </p>
      )}

      <div className="flex justify-end gap-2">
        {existing.length > 0 && (
          <Button variant="ghost" size="sm" disabled={saving} onClick={clear}>
            Remove split
          </Button>
        )}
        <Button variant="ghost" size="sm" disabled={saving} onClick={onClose}>
          Cancel
        </Button>
        <Button size="sm" disabled={saving || !balanced} onClick={save}>
          {saving ? "Saving…" : "Save split"}
        </Button>
      </div>
    </div>
  );
}

/// A line's tags: removable chips + an input (with a shared datalist) that assigns an
/// existing tag or mints a new one on Enter. Assignment is local — the tags persist with
/// the split on Save (personal-cfo-4d8.19).
function LineTags({
  label,
  tagIds,
  tags,
  onCreateTag,
  onChange,
}: {
  label: string;
  tagIds: string[];
  tags: TagViewDto[] | null;
  onCreateTag: (name: string) => Promise<string | IpcError>;
  onChange: (tagIds: string[]) => void;
}) {
  const [input, setInput] = useState("");
  const [error, setError] = useState<string | null>(null);
  const assigned = tagIds
    .map((id) => (tags ?? []).find((tag) => tag.id === id))
    .filter((tag): tag is TagViewDto => Boolean(tag));

  async function addFromInput() {
    const name = input.trim();
    if (!name) return;
    setInput("");
    setError(null);
    const existing = (tags ?? []).find(
      (tag) => tag.name.toLowerCase() === name.toLowerCase(),
    );
    let id = existing?.id ?? null;
    if (id === null) {
      const created = await onCreateTag(name);
      if (typeof created !== "string") {
        setError(describeIpcError(created));
        return;
      }
      id = created;
    }
    if (!tagIds.includes(id)) onChange([...tagIds, id]);
  }

  return (
    <div className="flex flex-col gap-1">
      <div className="flex flex-wrap items-center gap-1">
        {assigned.map((tag) => (
          <TagChip
            key={tag.id}
            tag={tag}
            onRemove={() => onChange(tagIds.filter((id) => id !== tag.id))}
          />
        ))}
        <input
          list="split-line-tag-options"
          value={input}
          onChange={(event) => setInput(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              void addFromInput();
            }
          }}
          placeholder="Add tag…"
          aria-label={`${label} tags`}
          className="h-8 min-w-24 flex-1 rounded-md border border-input bg-background px-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        />
      </div>
      {error && (
        <p role="alert" className="text-xs text-loss">
          {error}
        </p>
      )}
    </div>
  );
}
