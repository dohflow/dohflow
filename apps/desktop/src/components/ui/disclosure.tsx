import { ChevronDown } from "lucide-react";
import * as React from "react";

import { Card } from "@/components/ui/card";
import { cn } from "@/lib/utils";

/// The shared disclosure (Paper contract A2, docs/design-system/settings-layout-review.md,
/// docs/agent/FRONTEND.md section 7.5; personal-cfo-4d8.38).
///
/// A card whose heading holds ONE native trigger button. The body stays mounted when it is
/// collapsed, so ordinary drafts survive, but it is `hidden` and `inert`: it leaves the Tab
/// order and the accessibility tree. Errors, complete remedies and the one relevant action go in
/// `persistent`, which sits OUTSIDE the body and is never hidden. Collapsing only hides: it never
/// submits, resets or cancels anything.

/// What a descendant uses to pin its ancestors open while a destructive confirmation is open,
/// armed or irreversibly busy.
type PinContextValue = { pin: () => () => void } | null;

const PinContext = React.createContext<PinContextValue>(null);

/// Pins EVERY enclosing disclosure open while `active` is true. Call it from the component that
/// owns a destructive confirmation (open, armed or busy past the point of no return). A pinned
/// trigger stays focusable and announces `aria-disabled`, and ignores collapse attempts. The pin
/// releases when `active` turns false or the caller unmounts.
export function usePinDisclosures(active: boolean): void {
  const ctx = React.useContext(PinContext);
  React.useEffect(() => {
    if (!active || ctx === null) return;
    return ctx.pin();
  }, [active, ctx]);
}

type HeadingLevel = 2 | 3 | 4;

export type DisclosureProps = {
  /// The heading text, and the trigger's stable accessible name.
  title: React.ReactNode;
  /// A line under the title shown only while collapsed, from real data (never a fixed value, and
  /// never "healthy" or "verified" while loading or failed). It describes the trigger; it is not
  /// part of its name.
  summary?: React.ReactNode;
  /// Errors, complete remedies and the one relevant action. Rendered once, outside the body, and
  /// visible whether the section is open or collapsed.
  persistent?: React.ReactNode;
  /// Uncontrolled initial state.
  defaultOpen?: boolean;
  /// Controlled state. When set, `onOpenChange` is how a toggle is requested.
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  /// Heading level; pick the one that fits the page outline.
  headingLevel?: HeadingLevel;
  /// Names the body as a region (`role="region"`, labelled by the heading). Use selectively.
  region?: boolean;
  className?: string;
  bodyClassName?: string;
  children?: React.ReactNode;
};

export function Disclosure({
  title,
  summary,
  persistent,
  defaultOpen = false,
  open: openProp,
  onOpenChange,
  headingLevel = 2,
  region = false,
  className,
  bodyClassName,
  children,
}: DisclosureProps) {
  const baseId = React.useId();
  const triggerId = `${baseId}-trigger`;
  const bodyId = `${baseId}-body`;
  const summaryId = `${baseId}-summary`;

  const controlled = openProp !== undefined;
  const [openState, setOpenState] = React.useState(defaultOpen);
  const open = controlled ? openProp : openState;

  // Descendants pin this section (and, through the parent context, every ancestor) open.
  const parentPin = React.useContext(PinContext);
  const [pinCount, setPinCount] = React.useState(0);
  const pin = React.useCallback(() => {
    setPinCount((n) => n + 1);
    const releaseParent = parentPin?.pin();
    return () => {
      setPinCount((n) => n - 1);
      releaseParent?.();
    };
  }, [parentPin]);
  const pinContext = React.useMemo(() => ({ pin }), [pin]);
  const pinned = pinCount > 0;
  const expanded = open || pinned;

  const triggerRef = React.useRef<HTMLButtonElement>(null);
  const bodyRef = React.useRef<HTMLDivElement>(null);

  // Focus goes back to the trigger BEFORE the body is hidden, so it is never stranded on an
  // element that leaves the page. `toggle` does it ahead of the state change; the layout effect
  // below covers a controlled parent that collapses the section without a trigger click.
  const returnFocusIfInsideBody = React.useCallback(() => {
    const body = bodyRef.current;
    if (body !== null && body.contains(document.activeElement)) triggerRef.current?.focus();
  }, []);

  // A controlled parent can collapse the section without a click on the trigger.
  React.useLayoutEffect(() => {
    if (!expanded) returnFocusIfInsideBody();
  }, [expanded, returnFocusIfInsideBody]);

  function toggle() {
    if (pinned) return;
    const next = !open;
    if (!next) returnFocusIfInsideBody();
    if (!controlled) setOpenState(next);
    onOpenChange?.(next);
  }

  const Heading = `h${headingLevel}` as const;

  return (
    <Card className={cn("flex flex-col", className)}>
      <Heading className="m-0 text-base font-semibold leading-6">
        <button
          ref={triggerRef}
          id={triggerId}
          type="button"
          aria-expanded={expanded}
          aria-controls={bodyId}
          aria-disabled={pinned || undefined}
          aria-describedby={summary !== undefined && !expanded ? summaryId : undefined}
          onClick={toggle}
          className={cn(
            "flex min-h-11 w-full items-start gap-3 rounded-lg px-6 py-2.5 text-left",
            "outline-none focus-visible:ring-2 focus-visible:ring-ring",
            pinned && "cursor-not-allowed",
          )}
        >
          <span className="flex h-6 w-4 shrink-0 items-center justify-center">
            <ChevronDown
              aria-hidden
              className={cn(
                "size-4 transition-transform motion-reduce:transition-none",
                !expanded && "-rotate-90",
              )}
            />
          </span>
          <span className="min-w-0 break-words">{title}</span>
        </button>
      </Heading>
      {summary !== undefined && !expanded && (
        <p id={summaryId} className="px-6 pb-3 pl-13 text-sm text-muted-foreground">
          {summary}
        </p>
      )}
      {persistent !== undefined && <div className="px-6 pb-3">{persistent}</div>}
      <PinContext.Provider value={pinContext}>
        <div
          ref={bodyRef}
          id={bodyId}
          role={region ? "region" : undefined}
          aria-labelledby={region ? triggerId : undefined}
          hidden={!expanded}
          inert={!expanded}
          className={cn("px-6 pb-6", bodyClassName)}
        >
          {children}
        </div>
      </PinContext.Provider>
    </Card>
  );
}
