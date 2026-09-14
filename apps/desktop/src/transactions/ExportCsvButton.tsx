import { useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { FileDown, Loader2 } from "lucide-react";

import { commands } from "@/bindings";
import { Button } from "@/components/ui/button";
import { describeIpcError } from "@/vault/useVault";

/// Export every transaction as plaintext CSV via the native save dialog
/// (personal-cfo-hbd8, the Launch gate's portability guarantee). The file is
/// deliberately UNENCRYPTED — the dialog title and the result line both say so.
export function ExportCsvButton() {
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);

  async function onExport() {
    setNote(null);
    const path = await save({
      title: "Export transactions as UNENCRYPTED CSV",
      defaultPath: "transactions.csv",
      filters: [{ name: "CSV", extensions: ["csv"] }],
    });
    if (!path) return; // the user cancelled the dialog
    setBusy(true);
    const result = await commands.exportTransactionsCsv(path);
    setBusy(false);
    if (result.status === "ok") {
      setNote(
        `Exported ${result.data} transaction${result.data === 1 ? "" : "s"} — plaintext, unencrypted.`,
      );
    } else {
      setNote(describeIpcError(result.error));
    }
  }

  return (
    <span className="inline-flex items-center gap-2">
      <Button size="sm" variant="outline" disabled={busy} onClick={() => void onExport()}>
        {busy ? <Loader2 className="animate-spin" aria-hidden /> : <FileDown aria-hidden />}
        Export CSV
      </Button>
      {note && (
        <span role="status" className="text-xs text-muted-foreground">
          {note}
        </span>
      )}
    </span>
  );
}
