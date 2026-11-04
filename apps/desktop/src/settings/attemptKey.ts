import { useCallback, useEffect, useRef, useState } from "react";

/// Counts the attempts that have settled as a failure (personal-cfo-4d8.39.2): call `next()` each
/// time one does. Pair it with `useReannounced`.
export function useAttemptCount(): readonly [number, () => void] {
  const [attempt, setAttempt] = useState(0);
  const next = useCallback(() => setAttempt((count) => count + 1), []);
  return [attempt, next] as const;
}

/// An alert that is kept on screen through a retry never changes when the retry fails the same way,
/// so a screen reader would hear nothing for the second failure. The text to render for it: when
/// `attempt` advances and the message is the SAME as the one already on screen, it is empty for one
/// render and then comes back, in the same element, so the repeat is announced again (the pattern of
/// the Debt payoff status, personal-cfo-6wk.26). A first or different message shows at once. While
/// the retry is pending `attempt` does not move, so the message and the slot's action stay put.
export function useReannounced(text: string | null, attempt: number): string | null {
  const [shownFor, setShownFor] = useState(attempt);
  const previous = useRef<string | null>(text);
  const repeat = attempt !== shownFor && text !== null && text === previous.current;
  useEffect(() => {
    setShownFor(attempt);
  }, [attempt]);
  useEffect(() => {
    previous.current = text;
  });
  return repeat ? "" : text;
}
