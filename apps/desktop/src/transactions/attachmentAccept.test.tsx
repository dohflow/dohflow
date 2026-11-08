import { render, screen } from "@testing-library/react";

import { COARSE_POINTER_QUERY } from "@/lib/compact";
import { TOUCH_ATTACHMENT_ACCEPT, useAttachmentAccept } from "./attachmentAccept";

function stub(coarse: boolean) {
  vi.stubGlobal("matchMedia", ((query: string) => ({
    matches: coarse && query === COARSE_POINTER_QUERY,
    media: query,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
  })) as unknown as typeof window.matchMedia);
}
afterEach(() => vi.unstubAllGlobals());

function Probe() {
  const accept = useAttachmentAccept();
  return <input type="file" aria-label="probe" accept={accept} />;
}

describe("attachment picker accept (personal-cfo-i93n3.28 slice 5)", () => {
  it("on a touch device accepts images, documents and text, and no video or audio (so no microphone is ever offered)", () => {
    stub(true);
    render(<Probe />);
    const accept = screen.getByLabelText("probe").getAttribute("accept")!;
    expect(accept).toBe(TOUCH_ATTACHMENT_ACCEPT);
    expect(accept.split(",")).toEqual(["image/*", "application/*", "text/*"]);
    expect(accept).not.toMatch(/video|audio/);
  });

  it("on a desktop pointer sets no accept at all: any type attaches, as before", () => {
    stub(false);
    render(<Probe />);
    expect(screen.getByLabelText("probe")).not.toHaveAttribute("accept");
  });
});
