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

import { useAttemptCount, useReannounced } from "./attemptKey";
import { PersistentNotice } from "./PersistentSlot";
import { useKeepFocusAcrossMove } from "./useKeepFocusAcrossMove";

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
  // Each settled attempt that fails clears and restores its alert text, so an identical repeat is
  // announced again (the error is kept on screen through the retry).
  const [noteAttempt, nextNoteAttempt] = useAttemptCount();
  const alertText = useReannounced(note?.kind === "alert" ? note.text : null, noteAttempt);
  function failWith(text: string) {
    setNote({ kind: "alert", text });
    nextNoteAttempt();
  }
  const previewRegion = useRef<HTMLPreElement>(null);
  // Preview and Save each move between the body and the persistent slot (rendered once, wherever
  // the error needs them), so each keeps focus across the move.
  const previewFocus = useKeepFocusAcrossMove();
  const previewButton = previewFocus.ref;
  const saveFocus = useKeepFocusAcrossMove({
    settled: !saving,
    // The preview closed (saved, or expired): the Preview button is where the person goes next.
    fallback: () => previewRegion.current ?? previewButton.current,
  });
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
      // Not into a collapsed section's hidden body: there the Preview's hand-over chooses the heading.
      const region = previewRegion.current;
      if (region && region.closest("[hidden],[inert]") === null) region.focus();
    } else if (returnFocus.current) {
      returnFocus.current = false;
      // Not into a collapsed section's hidden body: there the Save's hand-over chooses the heading.
      const button = previewButton.current;
      if (button && button.closest("[hidden],[inert]") === null) button.focus();
    }
  }, [preview, previewButton]);

  function closePreview(message?: Note) {
    if (pending.current !== null) void commands.diagnosticsDiscard(pending.current);
    pending.current = null;
    returnFocus.current = true;
    setPreview(null);
    setNote(message ?? null);
    if (message?.kind === "alert") nextNoteAttempt();
  }

  async function onPreview() {
    previewFocus.remember();
    // An earlier error stays, with its Preview diagnostics, until this try settles.
    setLoading(true);
    const result = await commands.diagnosticsPreview();
    setLoading(false);
    if (result.status === "ok") {
      pending.current = result.data.snapshot_id;
      setNote(null);
      setPreview(result.data);
    } else {
      failWith(describeIpcError(result.error));
    }
  }

  async function onSave() {
    if (!preview) return;
    saveFocus.remember();
    // An earlier error stays, with its Save, until this try settles.
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
      failWith(SAVE_MESSAGES[result.data]);
    }
  }

  const empty = preview !== null && preview.records === 0;

  // An error, its full message and its one recovery action stay in the section's persistent slot,
  // open or closed; the action is rendered once, there while the error shows, else in the body.
  // With the preview still pending (a save was refused) the action is Save, to pick another
  // destination; with no preview (it failed, or it expired) it is Preview diagnostics.
  const alert = note?.kind === "alert" ? note : null;
  const previewInSlot = alert !== null && preview === null;
  const saveInSlot = alert !== null && preview !== null;
  const previewDiagnosticsButton = (
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
  );
  const saveButton = (
    <Button
      ref={saveFocus.ref}
      size="sm"
      disabled={saving || empty}
      aria-busy={saving}
      onClick={() => void onSave()}
    >
      {saving ? <Loader2 className="animate-spin" aria-hidden /> : <FileDown aria-hidden />}
      {saving ? "Saving…" : "Save…"}
    </Button>
  );

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
        {!preview && !previewInSlot && <div>{previewDiagnosticsButton}</div>}

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
              {!saveInSlot && saveButton}
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

        <PersistentNotice>
          {alert && (
            <div className="flex flex-col gap-2">
              <p role="alert" className="text-xs text-loss">
                {alertText}
              </p>
              <div>{previewInSlot ? previewDiagnosticsButton : saveButton}</div>
            </div>
          )}
        </PersistentNotice>
        {note?.kind === "status" && (
          <p role="status" className="text-xs text-muted-foreground">
            {note.text}
          </p>
        )}
      </CardContent>
    </Card>
  );
}
