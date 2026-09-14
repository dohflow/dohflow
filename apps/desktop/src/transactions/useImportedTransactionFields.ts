import { useQuery } from "@tanstack/react-query";

import { commands, type ImportedTransactionFieldsDto } from "@/bindings";
import { ipcQuery } from "@/lib/query";

/// Query key for a transaction's raw imported source fields (ADR 0045 §2). Scoped
/// by id so each transaction caches independently.
function importedFieldsKey(transactionId: string) {
  return ["imported-fields", "transaction", transactionId] as const;
}

/// Loads the raw imported source fields behind a transaction — every column the
/// importer captured, resolved via the provenance link (ADR 0045, 4d8.24.1.4).
/// `null` for a manually-entered transaction (no import).
export function useImportedTransactionFields(transactionId: string) {
  const query = useQuery({
    queryKey: importedFieldsKey(transactionId),
    queryFn: () =>
      ipcQuery(
        commands.importedTransactionFields(transactionId),
        "Could not load imported details.",
      ),
  });

  return {
    imported: (query.data ?? null) as ImportedTransactionFieldsDto | null,
    error: query.error?.message ?? null,
    isLoading: query.isLoading,
  };
}
