import { createContext, useCallback, useContext, useRef, useState, useSyncExternalStore } from "react";

/// The view settings that survive a visit to another destination, for the unlocked session only
/// (personal-cfo-i93n3.27 slice D; Mobile design 2, "Proposed unlocked-session preservation").
///
/// A closed union, not free-form strings, so a typo is a type error and the whole list is one place to read.
/// Each key's value type is what the view already held in local state:
export type SessionViewStateKey =
  | "futureCash.horizon" // number: the forecast horizon in days
  | "futureCash.lookback" // number: how many days of history the chart shows
  | "futureCash.stackedIds" // string[]: the ordered secondary scenario stack
  | "activity.query" // string: the projected-activity search text (memory only; never logged)
  | "activity.kind" // string: the projected-activity type filter
  | "activity.page" // number: the zero-based page of projected activity
  | "activity.accountColumns" // string[]: the account ids added as balance columns
  | "activity.openKey"; // string | null: the expanded event row

export type SessionViewStateStore = {
  values: Map<SessionViewStateKey, unknown>;
  listeners: Set<() => void>;
};

export function createSessionViewStateStore(): SessionViewStateStore {
  return { values: new Map(), listeners: new Set() };
}

export const SessionViewStateContext = createContext<SessionViewStateStore | null>(null);

/// `useState` backed by the session store: the same value, the same setter (a value or an updater), seeded
/// with `initial` on first read. Without a provider (a screen rendered alone, a unit test) it is plain local
/// state, so no view requires the provider; the shell test asserts every lifted view renders under it.
///
/// A `null` key is always local state: a caller that lifts its state only when it is given a key
/// (`usePagination`) passes `null` otherwise.
export function useSessionState<T>(
  key: SessionViewStateKey | null,
  initial: T,
): [T, (next: T | ((previous: T) => T)) => void] {
  const provided = useContext(SessionViewStateContext);
  const store = key === null ? null : provided;
  const [local, setLocal] = useState<T>(initial);
  const initialRef = useRef(initial);

  const subscribe = useCallback(
    (listener: () => void) => {
      store?.listeners.add(listener);
      return () => {
        store?.listeners.delete(listener);
      };
    },
    [store],
  );
  const getSnapshot = useCallback(
    () => (store && key !== null && store.values.has(key) ? (store.values.get(key) as T) : initialRef.current),
    [store, key],
  );
  const stored = useSyncExternalStore(subscribe, getSnapshot);

  const set = useCallback(
    (next: T | ((previous: T) => T)) => {
      if (!store || key === null) {
        setLocal(next);
        return;
      }
      const previous = store.values.has(key) ? (store.values.get(key) as T) : initialRef.current;
      const value = typeof next === "function" ? (next as (previous: T) => T)(previous) : next;
      if (Object.is(value, previous) && store.values.has(key)) return;
      store.values.set(key, value);
      for (const listener of [...store.listeners]) listener();
    },
    [store, key],
  );

  return [store ? stored : local, set];
}
