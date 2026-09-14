import { createContext, useContext } from "react";

/// The just-confirmed occurrence, enough to reverse it (personal-cfo-5ie.9).
export type ConfirmedObligation = {
  eventId: string;
  scheduledDate: string;
  name: string;
};

/// Publish "I just marked this occurrence paid" up to the Future Cash view. The confirmed row
/// leaves the forecast on success, so the Undo affordance can't live on the row — it surfaces as a
/// view-level bar instead. `null` outside a provider (e.g. in isolated tests), so callers use `?.`.
const PublishContext = createContext<
  ((info: ConfirmedObligation) => void) | null
>(null);

export const MarkObligationUndoProvider = PublishContext.Provider;

export function useMarkObligationUndoPublish() {
  return useContext(PublishContext);
}
