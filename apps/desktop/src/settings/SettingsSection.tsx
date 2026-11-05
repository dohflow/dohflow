import { useCallback, useMemo, useRef, useState, type ReactNode } from "react";

import { Disclosure } from "@/components/ui/disclosure";
import { cn } from "@/lib/utils";

import { PersistentSlotContext } from "./persistentSlotContext";
import { SectionFocusContext } from "./sectionFocusContext";
import { PREFERENCE_LANES, SPAN, type SpanName } from "./settingsLayout";

/// The Settings page grid (personal-cfo-4d8.39.1): six equal tracks, 16px gap, laid out against the
/// width of the main pane (`@container`), with `items-start` so an open editor keeps its natural
/// height instead of stretching to a taller neighbour. A collapsed section opts into stretching
/// (see `SettingsSection`) so collapsed siblings share the tallest natural height of their row.
export function SettingsGrid({ children }: { children: ReactNode }) {
  return (
    <div className="@container w-full">
      <div className="grid grid-cols-6 items-start gap-4">{children}</div>
    </div>
  );
}

/// One Settings section: an existing card inside the shared disclosure, with a wrapper title from
/// the approved list, an optional summary shown only while collapsed, and a span in the page grid.
///
/// Every section starts OPEN. Its body stays mounted when collapsed, so a draft survives, and is
/// `hidden` and `inert`, so it leaves the Tab order. Nothing is stored: the next visit opens the
/// sections again.
///
/// The summary must come from real data or be left out: it is never a fixed value, and never
/// reads as healthy or verified while a read is loading or has failed. Pass `undefined` then.
///
/// Errors inside the card body would be hidden by collapsing, so a card puts its error, remedy and
/// one recovery action in `PersistentNotice`: it lands in a slot the section keeps outside the hidden
/// body (personal-cfo-4d8.39.2), visible and announced open or collapsed. A card that has not been
/// moved yet keeps its error in its body. `persistent` lets a section pass its own content as well;
/// the slot is added after it.
export function SettingsSection({
  title,
  summary,
  persistent,
  span,
  children,
}: {
  title: string;
  summary?: ReactNode;
  persistent?: ReactNode;
  span: SpanName;
  children: ReactNode;
}) {
  const [open, setOpen] = useState(true);
  // Where the cards in this section put what must outlive a collapse. It is set once the slot has
  // mounted (a callback ref, so the portal target exists before the first paint).
  const [slot, setSlot] = useState<HTMLElement | null>(null);
  // The section's own heading trigger is the one control that is always visible; a card hands focus
  // to it when its own targets are hidden by a collapse (useKeepFocusAcrossMove).
  const sectionRef = useRef<HTMLDivElement>(null);
  const trigger = useCallback(
    () => sectionRef.current?.querySelector<HTMLElement>("h3 > button[aria-expanded]") ?? null,
    [],
  );
  const focus = useMemo(() => ({ open, trigger }), [open, trigger]);
  return (
    // The grid item. The disclosure fills it, so a collapsed section is as tall as its row.
    <div
      ref={sectionRef}
      data-settings-section={title}
      className={cn("min-w-0", SPAN[span], open ? "self-start" : "self-stretch")}
    >
      <Disclosure
        title={title}
        summary={summary}
        persistent={
          <>
            {persistent}
            <div ref={setSlot} data-settings-persistent={title} className="flex flex-col gap-2" />
          </>
        }
        open={open}
        onOpenChange={setOpen}
        headingLevel={3}
        className="h-full"
        bodyClassName="px-0 pb-0"
      >
        <SectionFocusContext.Provider value={focus}>
          <PersistentSlotContext.Provider value={slot}>{children}</PersistentSlotContext.Provider>
        </SectionFocusContext.Provider>
      </Disclosure>
    </div>
  );
}

/// The lanes a preference card lays its controls out in, by the card's own width: one, two from
/// 586px, three from 866px. For the combined preferences group; not used by a card yet.
export function PreferenceLanes({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div className="@container w-full">
      <div className={cn(PREFERENCE_LANES.classes, className)}>{children}</div>
    </div>
  );
}
