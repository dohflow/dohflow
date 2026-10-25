import {
  useCallback,
  useId,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type FocusEvent,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { Loader2, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  MAX_NOTICES,
  NOTICE_DURATION_MS,
  NoticeContext,
  NoticeStateContext,
  UNDO_FAILED_MESSAGE,
  type ActiveNotice,
  type NoticeApi,
  type NoticeInput,
  type NoticeState,
} from "@/components/ui/notice-context";

/// Owns the transient notices (docs/agent/FRONTEND.md section 6.2). Mounted once, by the shell, so
/// a pending Undo survives leaving the screen the removal happened on.
export function NoticeProvider({ children }: { children: ReactNode }) {
  const [notices, setNotices] = useState<ActiveNotice[]>([]);
  const nextKey = useRef(0);

  const notify = useCallback((input: NoticeInput) => {
    nextKey.current += 1;
    const key = nextKey.current;
    // Newest last; a fourth closes the oldest early.
    setNotices((previous) => [...previous, { ...input, key }].slice(-MAX_NOTICES));
  }, []);
  const dismiss = useCallback((key: number) => {
    setNotices((previous) => previous.filter((notice) => notice.key !== key));
  }, []);

  const api = useMemo<NoticeApi>(() => ({ notify }), [notify]);
  const state = useMemo<NoticeState>(() => ({ notices, dismiss }), [notices, dismiss]);

  return (
    <NoticeContext.Provider value={api}>
      <NoticeStateContext.Provider value={state}>{children}</NoticeStateContext.Provider>
    </NoticeContext.Provider>
  );
}

/// The one notifications region (a landmark, found by its name), rendered by the shell FIRST inside
/// `main` so the tab order reaches an Undo before the page content. It appears only while there is a
/// notice, and a notice appearing never moves focus. Sticky, so it stays in view when the list the
/// removal came from is scrolled.
export function NotificationsRegion() {
  const state = useContext(NoticeStateContext);
  if (!state || state.notices.length === 0) return null;
  return (
    <div
      role="region"
      aria-label="Notifications"
      className="sticky top-0 z-20 -mt-2 mb-4 flex flex-col gap-2"
    >
      {state.notices.map((notice) => (
        <NoticeItem key={notice.key} notice={notice} dismiss={state.dismiss} />
      ))}
    </div>
  );
}

function NoticeItem({
  notice,
  dismiss,
}: {
  notice: ActiveNotice;
  dismiss: (key: number) => void;
}) {
  const messageId = useId();
  const ref = useRef<HTMLDivElement>(null);
  const returnFocusTo = useRef<HTMLElement | null>(null);
  const remaining = useRef(NOTICE_DURATION_MS);
  const [hovered, setHovered] = useState(false);
  const [focusedWithin, setFocusedWithin] = useState(false);
  const [pending, setPending] = useState(false);
  const [failed, setFailed] = useState(false);
  // A live region that mounts together with its text is not reliably announced. The status element
  // mounts EMPTY with the notice and gets its message on the next frame, so assistive technology
  // sees the region first and the text arrive after (personal-cfo-egn67 review F1).
  const [announced, setAnnounced] = useState(false);
  useEffect(() => {
    const frame = requestAnimationFrame(() => setAnnounced(true));
    return () => cancelAnimationFrame(frame);
  }, []);

  const { key } = notice;
  const close = useCallback(() => {
    // Give focus back to where it came from when it was inside this notice and that place is
    // still on the page; never leave it on a removed element.
    const origin = returnFocusTo.current;
    if (ref.current?.contains(document.activeElement) && origin?.isConnected) origin.focus();
    dismiss(key);
  }, [dismiss, key]);

  // The timer runs only while nothing holds the notice open: hover, focus inside, an Undo in
  // flight, and a failed Undo (which waits for the user) all pause it, keeping the time left.
  const paused = hovered || focusedWithin || pending || failed;
  useEffect(() => {
    if (paused) return;
    const startedAt = Date.now();
    const timer = setTimeout(close, remaining.current);
    return () => {
      clearTimeout(timer);
      remaining.current -= Date.now() - startedAt;
    };
  }, [paused, close]);

  async function undo() {
    setPending(true);
    setFailed(false);
    let worked = false;
    try {
      worked = await notice.undo();
    } catch {
      worked = false;
    }
    if (worked) {
      close();
    } else {
      setPending(false);
      setFailed(true);
    }
  }

  function onFocus(event: FocusEvent<HTMLDivElement>) {
    if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
      returnFocusTo.current = event.relatedTarget as HTMLElement | null;
    }
    setFocusedWithin(true);
  }
  function onBlur(event: FocusEvent<HTMLDivElement>) {
    if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
      setFocusedWithin(false);
    }
  }
  // Escape closes THIS notice, and only while focus is inside it; it never reaches a drawer or
  // dialog behind.
  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.stopPropagation();
      close();
    }
  }

  return (
    <div
      ref={ref}
      className="rounded-lg border border-primary/30 bg-background px-4 py-3 shadow-sm"
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onFocus={onFocus}
      onBlur={onBlur}
      onKeyDown={onKeyDown}
    >
      <div className="flex items-center gap-3">
        <p id={messageId} role="status" className="min-h-5 flex-1 text-sm">
          {announced ? notice.message : null}
        </p>
        <Button
          variant="outline"
          size="sm"
          aria-describedby={messageId}
          disabled={pending}
          onClick={() => void undo()}
        >
          {pending ? <Loader2 className="animate-spin" aria-hidden /> : null}
          Undo
        </Button>
        <Button
          variant="ghost"
          size="icon"
          className="size-8 shrink-0"
          aria-label="Dismiss"
          onClick={close}
        >
          <X className="size-4" aria-hidden />
        </Button>
      </div>
      {failed && (
        <p role="alert" className="mt-2 text-sm text-loss">
          {UNDO_FAILED_MESSAGE}
        </p>
      )}
    </div>
  );
}
