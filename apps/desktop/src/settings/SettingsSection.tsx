import { useState, type ReactNode } from "react";

import { Disclosure } from "@/components/ui/disclosure";
import { cn } from "@/lib/utils";

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
/// Errors inside the card body can be hidden by collapsing; lifting them outside the body is a
/// separate slice, which is why this one opens every section.
export function SettingsSection({
  title,
  summary,
  span,
  children,
}: {
  title: string;
  summary?: ReactNode;
  span: SpanName;
  children: ReactNode;
}) {
  const [open, setOpen] = useState(true);
  return (
    // The grid item. The disclosure fills it, so a collapsed section is as tall as its row.
    <div
      data-settings-section={title}
      className={cn("min-w-0", SPAN[span], open ? "self-start" : "self-stretch")}
    >
      <Disclosure
        title={title}
        summary={summary}
        open={open}
        onOpenChange={setOpen}
        headingLevel={3}
        className="h-full"
        bodyClassName="px-0 pb-0"
      >
        {children}
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
