/// Return a local-only hint for the macOS cloud-sync folder roots we recognize.
/// This helper has no IPC or telemetry side effects by design.
export function backupFolderHint(
  selectedPath: string,
  homeDirectory: string,
): string | null {
  const path = trimTrailingSlashes(selectedPath);
  const home = trimTrailingSlashes(homeDirectory);
  const iCloudRoot = `${home}/Library/Mobile Documents/com~apple~CloudDocs`;
  const dropboxRoot = `${home}/Dropbox`;
  const cloudStorageRoot = `${home}/Library/CloudStorage`;

  if (isWithin(path, iCloudRoot)) {
    return "iCloud Drive";
  }
  if (isWithin(path, dropboxRoot)) {
    return "Dropbox";
  }
  if (isWithin(path, cloudStorageRoot)) {
    const relative = path.slice(cloudStorageRoot.length).replace(/^\/+/, "");
    const containerName = relative.split("/")[0] ?? "";
    return displayProviderName(containerName);
  }
  return null;
}

function trimTrailingSlashes(path: string): string {
  return path.replace(/\/+$/, "") || "/";
}

function isWithin(path: string, root: string): boolean {
  return path === root || path.startsWith(`${root}/`);
}

function displayProviderName(containerName: string): string {
  const normalized = containerName.toLowerCase();
  if (normalized.startsWith("googledrive")) return "Google Drive";
  if (normalized.startsWith("onedrive")) return "OneDrive";
  if (normalized.startsWith("dropbox")) return "Dropbox";
  if (normalized.startsWith("box")) return "Box";
  if (normalized.startsWith("pcloud")) return "pCloud";
  return "your cloud provider";
}
