const mocks = vi.hoisted(() => ({ openUrl: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: mocks.openUrl }));

import {
  DOHFLOW_LINKS,
  DOHFLOW_ORIGIN,
  isAllowedExternalUrl,
  openExternal,
} from "./openExternal";

beforeEach(() => {
  mocks.openUrl.mockReset();
  mocks.openUrl.mockResolvedValue(undefined);
});

describe("openExternal (personal-cfo-n76x.18)", () => {
  it("hands a DohFlow page to the system browser verbatim", async () => {
    await openExternal(DOHFLOW_LINKS.sponsor);
    expect(mocks.openUrl).toHaveBeenCalledTimes(1);
    expect(mocks.openUrl).toHaveBeenCalledWith("https://dohflow.app/sponsor");
  });

  it("allows the site root itself", async () => {
    await openExternal(DOHFLOW_ORIGIN);
    expect(mocks.openUrl).toHaveBeenCalledWith("https://dohflow.app/");
  });

  it.each([
    // The binding decision: never a repo host, never a sponsor platform or processor.
    "https://github.com/example/example",
    "https://sponsor.example/dohflow",
    // Plain HTTP to the right host is still not the allow-listed origin.
    "http://dohflow.app/",
    // A look-alike host: the trailing slash in the prefix is what stops this.
    "https://dohflow.app.example.com/",
    // The origin appearing later in the URL means nothing.
    "https://evil.example/?next=https://dohflow.app/",
    // Non-web schemes never leave the app.
    "javascript:alert(1)",
    "file:///etc/hosts",
    "mailto:someone@example.com",
    "",
  ])("refuses %j without touching the opener plugin", async (url) => {
    await expect(openExternal(url)).rejects.toThrow(/Refused to open/);
    expect(mocks.openUrl).not.toHaveBeenCalled();
  });

  it("keeps every cataloged link inside the allow-list", () => {
    for (const url of Object.values(DOHFLOW_LINKS)) {
      expect(isAllowedExternalUrl(url)).toBe(true);
    }
  });
});
