/// The file-system path of what the native file picker returned (personal-cfo-i93n3.26, restore on iOS).
///
/// On desktop the dialog plugin returns a plain path. On iOS it returns the `file://` URL of the copy
/// the system picker made inside the app's own storage, and the restore commands open a path, not a
/// URL: handed the URL they fail with "No such file or directory". A `file://` URL is turned into its
/// decoded path; anything else (every desktop path, including one that merely contains a colon or a
/// percent sign) is returned unchanged.
export function pickedFilePath(selected: string): string {
  if (!/^file:\/\//i.test(selected)) return selected;
  try {
    const url = new URL(selected);
    if (url.protocol !== "file:") return selected;
    // `file://localhost/x` and `file:///x` are the same local file; any other host is not a local path.
    if (url.hostname !== "" && url.hostname.toLowerCase() !== "localhost") return selected;
    return decodeURIComponent(url.pathname);
  } catch {
    return selected;
  }
}
