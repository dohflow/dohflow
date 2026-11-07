import { pickedFilePath } from "./pickedPath";

describe("pickedFilePath (personal-cfo-i93n3.26)", () => {
  it("turns the iOS picker's file URL into the path the restore commands open", () => {
    expect(
      pickedFilePath("file:///private/var/mobile/Containers/Data/Application/ABC/tmp/app.dohflow.mobile-Inbox/backup.pcfobk"),
    ).toBe("/private/var/mobile/Containers/Data/Application/ABC/tmp/app.dohflow.mobile-Inbox/backup.pcfobk");
  });

  it("decodes percent escapes, so a name with spaces or non-ASCII letters opens", () => {
    expect(pickedFilePath("file:///tmp/My%20Backup%20%C3%A9.pcfobk")).toBe("/tmp/My Backup é.pcfobk");
  });

  it("accepts the localhost form", () => {
    expect(pickedFilePath("file://localhost/tmp/a.pcfobk")).toBe("/tmp/a.pcfobk");
  });

  it("leaves a desktop path unchanged, including a percent sign or a colon in it", () => {
    for (const path of ["/Users/me/a.pcfobk", "C:\\Users\\me\\a.pcfobk", "/tmp/100%/a.pcfobk", "/tmp/a:b.pcfobk"]) {
      expect(pickedFilePath(path)).toBe(path);
    }
  });

  it("does not treat a URL for another host, or a malformed one, as a local path", () => {
    expect(pickedFilePath("file://server/share/a.pcfobk")).toBe("file://server/share/a.pcfobk");
    expect(pickedFilePath("file:///tmp/%E0%A4%A.pcfobk")).toBe("file:///tmp/%E0%A4%A.pcfobk");
  });
});
