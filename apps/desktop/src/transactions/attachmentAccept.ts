import { useEffect, type RefObject } from "react";

import { useCoarsePointer } from "@/lib/compact";

/// What the attachment pickers accept on a touch device (personal-cfo-i93n3.28 slice 5, AC5).
///
/// iOS WebKit offers Photo Library, Take Photo or Video and Choose Files for a file input with no `accept`. Take
/// Video needs a microphone usage description the app does not declare (and does not want), and a file picker that
/// offers it can end the app. `image/*` keeps Photo Library and Take Photo; `application/*` and `text/*` keep Choose Files
/// for PDFs and other documents; video and audio are not accepted, so they are not offered. A desktop attachment accepts any
/// type, so the desktop picker is left alone (`undefined`). Keyed to the touch pointer, not the width: a phone turned to
/// landscape is wider than the compact breakpoint and must not reach the unrestricted picker.
export const TOUCH_ATTACHMENT_ACCEPT = "image/*,application/*,text/*";

export function useAttachmentAccept(): string | undefined {
  return useCoarsePointer() ? TOUCH_ATTACHMENT_ACCEPT : undefined;
}

/// Cancelling the system file sheet returns focus to the control that opened it (personal-cfo-i93n3.28 slice 5, AC5).
/// The picker's input is visually hidden, and a dismissed sheet leaves focus on nothing (the page behind it), so a screen
/// reader user would lose their place. WebKit fires `cancel` on the file input when the sheet is dismissed without a file.
export function useFileCancelFocus(
  input: RefObject<HTMLInputElement | null>,
  trigger: RefObject<HTMLElement | null>,
): void {
  // Every render: the input may mount later than the component (the Add form shows it under "Add details").
  useEffect(() => {
    const element = input.current;
    if (element === null) return;
    const onCancel = () => trigger.current?.focus();
    element.addEventListener("cancel", onCancel);
    return () => element.removeEventListener("cancel", onCancel);
  });
}
