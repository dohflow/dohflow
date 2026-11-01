import * as React from "react";
import * as DialogPrimitive from "@radix-ui/react-dialog";

import { cn } from "@/lib/utils";

/// The shared modal dialog (personal-cfo-4d8.6.1; docs/design-system/recipes.md section 3).
///
/// shadcn's Dialog on `@radix-ui/react-dialog`, themed only through `globals.css` tokens. Radix
/// supplies what a hand-rolled overlay does not: Tab and Shift+Tab stay inside, everything outside
/// is `aria-hidden` and the page does not scroll behind it, Escape closes, and the title and
/// description name the dialog for assistive technology.
///
/// What this layer adds, because a confirmation about real money has to be reachable and readable:
///
/// - **Bounded.** The content never exceeds the window (`max-h-[calc(100dvh-3rem)]`). The title, the
///   error slot and the footer stay in view; [`DialogBody`] is the region that scrolls, so the
///   actions are always reachable on a short window. Data text wraps anywhere
///   (`overflow-wrap:anywhere`) and is never clamped or truncated.
/// - **A persistent error slot.** [`DialogError`] is a `role="alert"` region that is always mounted,
///   so an error that appears is announced; it stays until the caller clears it, and the dialog
///   never closes because of it. It is capped at a quarter of the window height (and scrolls) so a
///   long error cannot push the body out of a short window.
/// - **Focus.** `initialFocusRef` picks what is focused on open (put the safe action there for a
///   confirmation). On close, focus returns to `restoreFocusTo` when given (the caller decides, and
///   an empty or removed target means `fallbackFocusTo`), else to the element that had focus when
///   the dialog opened, else to `fallbackFocusTo` when that element is gone.
const Dialog = DialogPrimitive.Root;
const DialogTrigger = DialogPrimitive.Trigger;
const DialogClose = DialogPrimitive.Close;
const DialogPortal = DialogPrimitive.Portal;

const DialogOverlay = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Overlay>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Overlay>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Overlay
    ref={ref}
    className={cn("fixed inset-0 z-50 bg-foreground/40", className)}
    {...props}
  />
));
DialogOverlay.displayName = "DialogOverlay";

function focusable(element: HTMLElement | null | undefined): element is HTMLElement {
  return (
    element !== null &&
    element !== undefined &&
    element.isConnected &&
    !(element as HTMLButtonElement).disabled
  );
}

interface DialogContentProps
  extends React.ComponentPropsWithoutRef<typeof DialogPrimitive.Content> {
  /// Focused when the dialog opens. For a confirmation, the safe action (Cancel).
  initialFocusRef?: React.RefObject<HTMLElement | null>;
  /// Focused when the dialog closes, when it is still on the page. The caller decides: pass it where
  /// the opener does not take focus on click (Safari and the macOS web view do not focus a clicked
  /// button), or to send focus elsewhere because the opener is about to go (an empty or removed
  /// target then means `fallbackFocusTo`).
  restoreFocusTo?: React.RefObject<HTMLElement | null>;
  /// Focused when the dialog closes and the opener is gone (its row was removed by the action).
  fallbackFocusTo?: React.RefObject<HTMLElement | null>;
}

const DialogContent = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Content>,
  DialogContentProps
>(
  (
    {
      className,
      children,
      initialFocusRef,
      restoreFocusTo,
      fallbackFocusTo,
      onOpenAutoFocus,
      onCloseAutoFocus,
      ...props
    },
    ref,
  ) => {
    const opener = React.useRef<HTMLElement | null>(null);
    return (
      <DialogPortal>
        <DialogOverlay />
        <DialogPrimitive.Content
          ref={ref}
          onOpenAutoFocus={(event) => {
            const active = document.activeElement;
            opener.current =
              active instanceof HTMLElement && active !== document.body ? active : null;
            onOpenAutoFocus?.(event);
            if (!event.defaultPrevented && initialFocusRef?.current) {
              event.preventDefault();
              initialFocusRef.current.focus();
            }
          }}
          onCloseAutoFocus={(event) => {
            onCloseAutoFocus?.(event);
            if (event.defaultPrevented) return;
            // A caller that passes `restoreFocusTo` decides: its target if it is still there, else
            // the fallback (never the element that happened to have focus, which the action may be
            // about to remove). Without it, the element that had focus on open, then the fallback.
            const first = restoreFocusTo !== undefined ? restoreFocusTo.current : opener.current;
            const target = [first, fallbackFocusTo?.current].find(focusable);
            if (target) {
              event.preventDefault();
              target.focus();
            }
          }}
          className={cn(
            "fixed left-1/2 top-1/2 z-50 flex max-h-[calc(100dvh-3rem)] w-[calc(100vw-2rem)] max-w-md -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-lg border bg-popover text-popover-foreground shadow-lg focus-visible:outline-none",
            className,
          )}
          {...props}
        >
          {children}
        </DialogPrimitive.Content>
      </DialogPortal>
    );
  },
);
DialogContent.displayName = "DialogContent";

const DialogHeader = ({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) => (
  <div className={cn("flex shrink-0 flex-col gap-1.5 px-5 pt-5", className)} {...props} />
);
DialogHeader.displayName = "DialogHeader";

const DialogTitle = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Title>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Title>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Title
    ref={ref}
    className={cn("text-base font-semibold [overflow-wrap:anywhere]", className)}
    {...props}
  />
));
DialogTitle.displayName = "DialogTitle";

const DialogDescription = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Description>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Description>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Description
    ref={ref}
    className={cn("text-sm text-muted-foreground [overflow-wrap:anywhere]", className)}
    {...props}
  />
));
DialogDescription.displayName = "DialogDescription";

/// The region that scrolls when the content is taller than the window. Everything else (title,
/// error, footer) stays in view. A scroll container with nothing focusable inside cannot be
/// scrolled from the keyboard, so it is made focusable (`tabIndex={0}`) only while it overflows.
const DialogBody = React.forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>(
  ({ className, ...props }, forwarded) => {
    const own = React.useRef<HTMLDivElement | null>(null);
    const [overflows, setOverflows] = React.useState(false);
    React.useLayoutEffect(() => {
      const node = own.current;
      if (node === null) return;
      const measure = () => setOverflows(node.scrollHeight > node.clientHeight + 1);
      measure();
      if (typeof ResizeObserver === "undefined") return;
      const observer = new ResizeObserver(measure);
      observer.observe(node);
      return () => observer.disconnect();
    }, []);
    return (
      <div
        ref={(node) => {
          own.current = node;
          if (typeof forwarded === "function") forwarded(node);
          else if (forwarded) forwarded.current = node;
        }}
        tabIndex={overflows ? 0 : undefined}
        className={cn(
          "min-h-0 flex-1 overflow-y-auto px-5 py-3 text-sm [overflow-wrap:anywhere] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring",
          className,
        )}
        {...props}
      />
    );
  },
);
DialogBody.displayName = "DialogBody";

/// The persistent, announced error slot. Always mounted so a new error is announced; empty, it is
/// visually hidden. It stays until the caller clears it (on the next attempt, or when the dialog
/// closes) and never closes the dialog itself.
const DialogError = ({
  error,
  className,
  ...props
}: { error?: string | null } & Omit<React.HTMLAttributes<HTMLDivElement>, "children">) => (
  <div
    role="alert"
    className={cn(
      error
        ? "mx-5 mb-2 max-h-[25dvh] shrink-0 overflow-y-auto rounded-md border border-loss/40 bg-loss/10 p-3 text-sm text-foreground [overflow-wrap:anywhere]"
        : "sr-only",
      className,
    )}
    {...props}
  >
    {error ?? null}
  </div>
);
DialogError.displayName = "DialogError";

const DialogFooter = ({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) => (
  <div
    className={cn("flex shrink-0 flex-wrap justify-end gap-2 px-5 pb-5 pt-3", className)}
    {...props}
  />
);
DialogFooter.displayName = "DialogFooter";

export {
  Dialog,
  DialogBody,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogError,
  DialogFooter,
  DialogHeader,
  DialogOverlay,
  DialogPortal,
  DialogTitle,
  DialogTrigger,
};
