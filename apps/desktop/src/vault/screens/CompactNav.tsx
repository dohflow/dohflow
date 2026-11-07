import { useRef, useState } from "react";
import { Check, LockKeyhole, PanelLeftOpen, Search } from "lucide-react";

import { BuildBadge } from "@/settings/BuildBadge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogClose,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useLargeText } from "@/lib/largeText";
import { COMPACT_FULLSCREEN } from "@/lib/sheet";
import { cn } from "@/lib/utils";
import { GROUPS, NAV, type NavItem, type Tab } from "./Sidebar";

/// Focus the current destination's heading, so a screen reader announces where it landed
/// (personal-cfo-i93n3.25 slice C). A view's heading is not focusable by default, so it is made
/// programmatically focusable; the views render their heading as soon as they mount, but a read-model
/// query may mount a frame later, so it tries for a few frames.
export function focusDestinationHeading(attempts = 6): void {
  const heading = document.querySelector<HTMLElement>("main h1, main h2");
  if (heading) {
    heading.tabIndex = -1;
    heading.focus({ preventScroll: false });
    return;
  }
  if (attempts > 0) requestAnimationFrame(() => focusDestinationHeading(attempts - 1));
}

/// The phone's navigation (personal-cfo-i93n3.25 slice C; Mobile design 2, "Navigation and deep
/// state"). On a compact screen the persistent sidebar is replaced by this bar above the page, and
/// by a full-screen Sections dialog that lists every destination in the sidebar's groups and order.
///
/// - **Sections, Search and Lock** stay above the page. At an ordinary text size they are word
///   buttons; at an accessibility text size (`useLargeText`) they are icon buttons in at least 48-point
///   hit regions, named for VoiceOver by the same words.
/// - **The Sections dialog** is the shared `Dialog` presented full screen: the page behind it is inert
///   and hidden from assistive technology, focus starts at Close, and Escape or Close dismiss it
///   WITHOUT navigating and restore focus to the Sections button. Choosing a destination dismisses it,
///   changes the destination, and moves focus to that destination's heading. The current destination
///   carries a check mark, a selected surface and `aria-current="page"`.
export function CompactNavBar({
  tab,
  onSelect,
  onOpenSearch,
  onLock,
  locking,
  inboxCount = 0,
}: {
  tab: Tab;
  onSelect: (tab: Tab) => void;
  onOpenSearch: () => void;
  onLock: () => void;
  locking: boolean;
  /// Items waiting in the Money Inbox, shown on its row and spoken with its name.
  inboxCount?: number;
}) {
  const [open, setOpen] = useState(false);
  const sectionsRef = useRef<HTMLButtonElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  // Set when a destination was chosen, so the close moves focus to its heading instead of back to the
  // Sections button (which is where Close and Escape return it).
  const chose = useRef(false);
  const largeText = useLargeText();

  const barButton = cn("shrink-0", largeText ? "min-h-12 min-w-12 px-0" : "px-3");

  return (
    <>
      <div
        role="group"
        aria-label="Page actions"
        className="flex shrink-0 items-center gap-1 border-b bg-background px-2 py-1"
      >
        <Button
          ref={sectionsRef}
          type="button"
          variant="ghost"
          className={barButton}
          aria-haspopup="dialog"
          aria-expanded={open}
          aria-label={largeText ? "Sections" : undefined}
          onClick={() => setOpen(true)}
        >
          {largeText ? <PanelLeftOpen aria-hidden /> : "Sections"}
        </Button>
        <div className="flex-1" aria-hidden />
        <Button
          type="button"
          variant="ghost"
          className={barButton}
          aria-label={largeText ? "Search" : undefined}
          onClick={onOpenSearch}
        >
          {largeText ? <Search aria-hidden /> : "Search"}
        </Button>
        <Button
          type="button"
          variant="ghost"
          className={barButton}
          disabled={locking}
          aria-label={largeText ? "Lock" : undefined}
          onClick={onLock}
        >
          {largeText ? <LockKeyhole aria-hidden /> : locking ? "Locking…" : "Lock"}
        </Button>
      </div>

      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent
          // The spec names no description for this dialog and adds no copy for one.
          aria-describedby={undefined}
          className={cn("max-w-none", COMPACT_FULLSCREEN)}
          initialFocusRef={closeRef}
          restoreFocusTo={sectionsRef}
          fallbackFocusTo={sectionsRef}
          onCloseAutoFocus={(event) => {
            if (!chose.current) return;
            chose.current = false;
            event.preventDefault();
            requestAnimationFrame(() => focusDestinationHeading());
          }}
        >
          <DialogHeader className="flex-row items-center justify-between gap-2 px-4 pt-3">
            <DialogTitle>Sections</DialogTitle>
            <DialogClose asChild>
              <Button ref={closeRef} type="button" variant="ghost" className="shrink-0">
                Close
              </Button>
            </DialogClose>
          </DialogHeader>
          <DialogBody className="flex flex-col gap-4 px-4">
            {GROUPS.map((group) => {
              const items = NAV.filter((item) => item.group === group.key);
              const headingId = `sections-group-${group.key}`;
              return (
                <section key={group.key} aria-labelledby={headingId} className="flex flex-col gap-1">
                  <p
                    id={headingId}
                    className="px-2 pb-0.5 text-[0.75rem] font-semibold uppercase tracking-wider text-muted-foreground"
                  >
                    {group.label}
                  </p>
                  {items.map((item) => (
                    <DestinationRow
                      key={item.tab}
                      item={item}
                      active={tab === item.tab}
                      badge={item.tab === "money-inbox" ? inboxCount : 0}
                      onChoose={() => {
                        chose.current = true;
                        onSelect(item.tab);
                        setOpen(false);
                      }}
                    />
                  ))}
                </section>
              );
            })}
            {NAV.filter((item) => item.group === "utility").map((item) => (
              <DestinationRow
                key={item.tab}
                item={item}
                active={tab === item.tab}
                badge={0}
                onChoose={() => {
                  chose.current = true;
                  onSelect(item.tab);
                  setOpen(false);
                }}
              />
            ))}
            {/* Which build am I running? (the same badge the sidebar carries). */}
            <BuildBadge collapsed={false} />
          </DialogBody>
        </DialogContent>
      </Dialog>
    </>
  );
}

function DestinationRow({
  item,
  active,
  badge,
  onChoose,
}: {
  item: NavItem;
  active: boolean;
  badge: number;
  onChoose: () => void;
}) {
  // The count is part of the accessible name, as in the sidebar.
  const label = badge > 0 ? `${item.label}, ${badge} waiting` : item.label;
  return (
    <Button
      type="button"
      variant="ghost"
      aria-current={active ? "page" : undefined}
      aria-label={label}
      onClick={onChoose}
      className={cn(
        "min-h-12 w-full justify-start gap-3 px-3 text-base [&_svg]:size-5",
        active && "bg-primary/10 text-primary hover:bg-primary/10",
      )}
    >
      {item.icon}
      <span className="flex-1 text-left">{item.label}</span>
      {badge > 0 && (
        <span
          aria-hidden
          className="min-w-6 rounded-full bg-primary/15 px-1.5 py-0.5 text-center text-xs font-semibold tabular-nums text-primary"
        >
          {badge > 99 ? "99+" : badge}
        </span>
      )}
      {active && <Check aria-hidden className="shrink-0" />}
    </Button>
  );
}
