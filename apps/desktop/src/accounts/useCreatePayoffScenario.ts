import { useCallback } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";

import { commands, type IpcError } from "@/bindings";
import { queryKeys } from "@/lib/query";

/// The first day of next month, `YYYY-MM-DD` — the anchor for a "starts next month" recurring
/// extra payment. Uses the browser clock (UI-only; the pure forecast engine stays clock-free).
function firstOfNextMonth(): string {
  const now = new Date();
  const next = new Date(now.getFullYear(), now.getMonth() + 1, 1);
  const month = String(next.getMonth() + 1).padStart(2, "0");
  return `${next.getFullYear()}-${month}-01`;
}

/// Turn the entered extra debt payment into a named Future Cash scenario (personal-cfo-6wk.15):
/// creates a scenario and attaches a `recurring_debt_payment` overlay, so the extra flows as a
/// monthly liquid outflow (ADR 0035 §3) that the Future Cash view can compare against the base.
/// The extra's cash impact is the same whichever strategy allocates it, so it's one scenario per
/// amount. Rolls the (empty) scenario back if the overlay fails to attach.
export function useCreatePayoffScenario() {
  const queryClient = useQueryClient();
  const mutation = useMutation({
    mutationFn: async (input: {
      name: string;
      extraBudgetMinor: number;
      currency: string;
    }) => {
      const created = await commands.createScenario({
        name: input.name,
        description: null,
      });
      if (created.status !== "ok") return created;

      // Roll the (empty) scenario back if the overlay can't attach — whether the command returns
      // a structured error or throws a transport-level Error.
      try {
        const attached = await commands.createForecastAssumption({
          kind: "recurring_debt_payment",
          scenario_id: created.data.id,
          target_entity_id: null,
          amount: { minor_units: input.extraBudgetMinor, currency: input.currency },
          date: null,
          label: "Extra debt payment",
          new_amount_minor: null,
          new_anchor_date: firstOfNextMonth(),
          effective_date: null,
          end_date: null,
        });
        if (attached.status !== "ok") {
          await commands.deleteScenario(created.data.id).catch(() => undefined);
          return attached;
        }
        return created;
      } catch (e) {
        await commands.deleteScenario(created.data.id).catch(() => undefined);
        throw e;
      }
    },
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({ queryKey: queryKeys.scenarios });
        void queryClient.invalidateQueries({ queryKey: ["forecast"] });
      }
    },
  });

  const createPayoffScenario = useCallback(
    async (input: {
      name: string;
      extraBudgetMinor: number;
      currency: string;
    }): Promise<IpcError | null> => {
      const result = await mutation.mutateAsync(input);
      return result.status === "ok" ? null : result.error;
    },
    [mutation],
  );

  return { createPayoffScenario, pending: mutation.isPending };
}
