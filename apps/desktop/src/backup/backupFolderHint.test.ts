import { backupFolderHint } from "./backupFolderHint";

const HOME = "/Users/alex";

describe("backupFolderHint", () => {
  it("recognizes iCloud Drive and Dropbox folders beneath the home directory", () => {
    expect(
      backupFolderHint(
        `${HOME}/Library/Mobile Documents/com~apple~CloudDocs/Backups`,
        HOME,
      ),
    ).toBe("iCloud Drive");
    expect(backupFolderHint(`${HOME}/Dropbox/Finance`, HOME)).toBe("Dropbox");
  });

  it("recognizes cloud-storage containers without exposing account names", () => {
    expect(
      backupFolderHint(
        `${HOME}/Library/CloudStorage/GoogleDrive-alex@example.com/Backups`,
        HOME,
      ),
    ).toBe("Google Drive");
    expect(
      backupFolderHint(
        `${HOME}/Library/CloudStorage/OneDrive-Personal/Backups`,
        HOME,
      ),
    ).toBe("OneDrive");
  });

  it("does not classify similarly named folders outside a known root", () => {
    expect(backupFolderHint(`${HOME}/Dropbox-old/Finance`, HOME)).toBeNull();
    expect(backupFolderHint("/Volumes/External/Backups", HOME)).toBeNull();
    expect(
      backupFolderHint(`${HOME}/Library/CloudStorage`, HOME),
    ).toBe("your cloud provider");
  });
});
