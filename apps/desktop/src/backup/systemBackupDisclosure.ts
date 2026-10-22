/// The system-backup disclosure sentence (personal-cfo-i93n3.9). The host that built this app
/// says whether ITS system backups include the encrypted vault and what to call the device
/// (`build_info.system_backup_device`); the desktop says nothing, so nothing is shown. The
/// sentence is the UI's, the device name is the host's, and no device is named in this repo.
export function systemBackupDisclosure(device: string): string {
  return `${device} backups include your encrypted vault; your password is still required.`;
}
