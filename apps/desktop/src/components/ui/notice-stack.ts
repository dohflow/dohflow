import { MAX_NOTICES, type ActiveNotice } from "@/components/ui/notice-context";

/// The notices on screen, and those waiting for room (docs/agent/FRONTEND.md section 6.2, "When the
/// stack is full"). Pure, so the rule is tested without a DOM.
export type NoticeStack = { shown: ActiveNotice[]; waiting: ActiveNotice[] };

export const EMPTY_STACK: NoticeStack = { shown: [], waiting: [] };

/// Add a notice. With room it shows (newest last). Full, it evicts the OLDEST UNDO notice (its timer
/// was running anyway) and shows. An ERROR notice is never evicted, because the user has not closed
/// it and it must not vanish before it is read: when all of the shown notices are errors, the new
/// one waits, in order, and nothing is lost.
export function addNotice(stack: NoticeStack, notice: ActiveNotice): NoticeStack {
  if (stack.shown.length < MAX_NOTICES) {
    return { ...stack, shown: [...stack.shown, notice] };
  }
  const evict = stack.shown.findIndex((shown) => shown.undo !== undefined);
  if (evict === -1) {
    return { ...stack, waiting: [...stack.waiting, notice] };
  }
  return {
    ...stack,
    shown: [...stack.shown.filter((_, index) => index !== evict), notice],
  };
}

/// Remove a notice (closed, expired, or undone) and let waiting ones in, oldest first, while there is
/// room.
export function removeNotice(stack: NoticeStack, key: number): NoticeStack {
  const shown = stack.shown.filter((notice) => notice.key !== key);
  const waiting = stack.waiting.filter((notice) => notice.key !== key);
  while (shown.length < MAX_NOTICES && waiting.length > 0) {
    shown.push(waiting.shift()!);
  }
  return { shown, waiting };
}
