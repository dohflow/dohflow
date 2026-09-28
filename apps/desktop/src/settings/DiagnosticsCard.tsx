import { useEffect, useRef, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { FileDown, Loader2 } from "lucide-react";

import {
  commands,
  type DiagnosticsPreviewDto,
  type DiagnosticsSaveResult,
} from "@/bindings";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { describeIpcError } from "@/vault/useVault";

/// Safe copy for every save result (personal-cfo-lyd). None of it echoes the
/// path or any OS error text.
const SAVE_MESSAGES: Record<Exclude<DiagnosticsSaveResult, "saved">, string> = {
  preview_expired: "This preview has expired. Preview diagnostics again.",
  invalid_destination:
    "Choose a .json file in a folder that exists. Nothing was saved.",
  permission_denied:
    "DohFlow doesn't have permission to save there. Choose another folder. Nothing was saved.",
  disk_full: "There isn't enough disk space to save diagnostics. Nothing was saved.",
  failed: "Diagnostics couldn't be saved. Nothing was saved.",
};

type Note = { kind: "status" | "alert"; text: string };

/// Local diagnostics (personal-cfo-lyd, docs/security/logging-policy.md §8,
/// ADR 0066-A §5): preview the exact redacted bundle, then save it — through the
/// native Save dialog — only if the user chooses to. Nothing is exported,
/// written or uploaded otherwise, and there is no unredacted mode.
export function DiagnosticsCard() {
  const [preview, setPreview] = useState<DiagnosticsPreviewDto | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [note, setNote] = useState<Note | null>(null);
  const previewButton = useRef<HTMLButtonElement>(null);
  const previewRegion = useRef<HTMLPreElement>(null);
  // The pending snapshot, for discarding it when the card goes away (vault lock
  // and switch unmount Settings; the backend has already dropped it by then).
  const pending = useRef<number | null>(null);
  // Set when a preview closes, so focus returns to the Preview button once it
  // is rendered again (it is hidden while a preview is open).
  const returnFocus = useRef(false);

  useEffect(
    () => () => {
      if (pending.current !== null) {
        void commands.diagnosticsDiscard(pending.current);
      }
    },
    [],
  );

  useEffect(() => {
    if (preview) {
      previewRegion.current?.focus();
    } else if (returnFocus.current) {
      returnFocus.current = false;
      previewButton.current?.focus();
    }
  }, [preview]);

  function closePreview(message?: Note) {
    if (pending.current !== null) void commands.diagnosticsDiscard(pending.current);
    pending.current = null;
    returnFocus.current = true;
    setPreview(null);
    setNote(message ?? null);
  }

  async function onPreview() {
    setNote(null);
    setLoading(true);
    const result = await commands.diagnosticsPreview();
    setLoading(false);
    if (result.status === "ok") {
      pending.current = result.data.snapshot_id;
      setPreview(result.data);
    } else {
      setNote({ kind: "alert", text: describeIpcError(result.error) });
    }
  }

  async function onSave() {
    if (!preview) return;
    setNote(null);
    const path = await save({
      title: "Save redacted diagnostics",
      defaultPath: preview.suggested_file_name,
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (!path) {
      setNote({ kind: "status", text: "Not saved." });
      return;
    }
    setSaving(true);
    const result = await commands.diagnosticsSave(preview.snapshot_id, path);
    setSaving(false);
    if (result.status === "error") {
      // A lock or switch during the save: the preview is gone with the session.
      closePreview({ kind: "alert", text: SAVE_MESSAGES.preview_expired });
      return;
    }
    if (result.data === "saved") {
      pending.current = null; // the backend consumed it
      closePreview({ kind: "status", text: "Diagnostics saved." });
    } else if (result.data === "preview_expired") {
      closePreview({ kind: "alert", text: SAVE_MESSAGES.preview_expired });
    } else {
      // The preview is still pending: the user can pick another destination.
      setNote({ kind: "alert", text: SAVE_MESSAGES[result.data] });
    }
  }

  const empty = preview !== null && preview.records === 0;

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-base">Diagnostics</CardTitle>
        <CardDescription>
          While this vault is unlocked, DohFlow keeps a small record of how
          operations performed: durations, counts and outcomes, never your
          financial data. It stays in memory, is cleared when you lock or switch
          vaults, and never leaves this device unless you save it here.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        {!preview && (
          <div>
            <Button
              ref={previewButton}
              size="sm"
              variant="outline"
              disabled={loading}
              onClick={() => void onPreview()}
            >
              {loading && <Loader2 className="animate-spin" aria-hidden />}
              Preview diagnostics
            </Button>
          </div>
        )}

        {preview && (
          <div
            className="flex flex-col gap-2"
            onKeyDown={(event) => {
              if (event.key === "Escape" && !saving) closePreview();
            }}
          >
            {empty ? (
              <p className="text-sm text-muted-foreground">
                Nothing has been captured this session yet.
              </p>
            ) : (
              <p className="text-sm text-muted-foreground">
                {preview.records} record{preview.records === 1 ? "" : "s"}
                {preview.dropped > 0 &&
                  ` · ${preview.dropped} older record${preview.dropped === 1 ? "" : "s"} dropped`}
                {preview.rejected > 0 && ` · ${preview.rejected} rejected`}. This
                is exactly what will be saved.
              </p>
            )}
            <pre
              ref={previewRegion}
              tabIndex={0}
              aria-label="Diagnostics preview, exactly what will be saved"
              className="max-h-64 overflow-auto rounded-md border bg-muted/40 p-3 text-xs"
            >
              {preview.text}
            </pre>
            <div className="flex items-center gap-2">
              <Button
                size="sm"
                disabled={saving || empty}
                aria-busy={saving}
                onClick={() => void onSave()}
              >
                {saving ? (
                  <Loader2 className="animate-spin" aria-hidden />
                ) : (
                  <FileDown aria-hidden />
                )}
                {saving ? "Saving…" : "Save…"}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                disabled={saving}
                onClick={() => closePreview()}
              >
                Close
              </Button>
            </div>
          </div>
        )}

        {note && (
          <p
            role={note.kind}
            className={
              note.kind === "alert" ? "text-xs text-loss" : "text-xs text-muted-foreground"
            }
          >
            {note.text}
          </p>
        )}
      </CardContent>
    </Card>
  );
}
