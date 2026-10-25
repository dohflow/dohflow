import { createContext, useContext } from "react";

/// How long a notice stays before it closes itself (docs/agent/FRONTEND.md section 6.2).
export const NOTICE_DURATION_MS = 10_000;
/// At most this many notices at once; a further one closes the oldest early.
export const MAX_NOTICES = 3;
/// What a failed Undo says (section 6.2 item 5; the same words as the Cash Flow undo bar).
export const UNDO_FAILED_MESSAGE = "Couldn't undo — try again.";

export type NoticeInput = {
  /// What happened, as a sentence the notice reads before its Undo button: "Removed Rent."
  message: string;
  /// Reverses it. Resolves `true` when it worked and `false` when it did not (the notice then
  /// stays, with its Undo button, so the user can try again).
  undo: () => Promise<boolean>;
};

/// A notice on screen. One WITHOUT `undo` is an error notice (FRONTEND.md section 6.2): an action
/// partly worked and a step after it failed.
export type ActiveNotice = { key: number; message: string; undo?: NoticeInput["undo"] };

export type NoticeApi = {
  notify: (input: NoticeInput) => void;
  /// Tell the user a step failed after the thing they asked for was done. No Undo; it does not close
  /// on a timer, only when the user closes it (its button, or Escape while focus is inside).
  notifyError: (message: string) => void;
};
export type NoticeState = {
  notices: ActiveNotice[];
  dismiss: (key: number) => void;
};

export const NoticeContext = createContext<NoticeApi | null>(null);
export const NoticeStateContext = createContext<NoticeState | null>(null);

/// Show a transient notice with an Undo (section 6.2). Throws outside a `NoticeProvider`,
/// like `useTheme`, so a screen cannot silently lose its Undo.
export function useNotices(): NoticeApi {
  const api = useContext(NoticeContext);
  if (!api) throw new Error("useNotices must be used inside a NoticeProvider");
  return api;
}
