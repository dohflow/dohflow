import { screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { BuildInfoDto } from "@/bindings";
import { CreateVaultScreen } from "./CreateVaultScreen";

const mocks = vi.hoisted(() => ({ buildInfo: vi.fn() }));

vi.mock("@/bindings", () => ({ commands: { buildInfo: mocks.buildInfo } }));
vi.mock("@/vault/useVault", () => ({
  useVault: () => ({ createVault: vi.fn() }),
  describeIpcError: (error: unknown) => String(error),
}));
// The restore control has its own tests and its own IPC; only the footer copy is under test.
vi.mock("@/backup/RestoreFromBackup", () => ({ RestoreFromBackup: () => null }));

function buildInfo(systemBackupDevice: string | null): BuildInfoDto {
  return {
    version: "0.3.0",
    commit: "abc1234",
    channel: "release",
    built_at: "2026-07-20T14:32:05Z",
    dirty: false,
    os_version: null,
    self_update_available: false,
    system_backup_device: systemBackupDevice,
  };
}

const DESKTOP_LINE = "Encrypted on this device. Never uploaded.";
const HOST_LINE = "Encrypted on this device. DohFlow never uploads it.";

describe("create-vault footer (personal-cfo-i93n3.9)", () => {
  test("a host with no system-backup device keeps the line exactly as it was", async () => {
    mocks.buildInfo.mockResolvedValue(buildInfo(null));
    renderWithClient(<CreateVaultScreen />);
    await waitFor(() => expect(mocks.buildInfo).toHaveBeenCalled());
    expect(screen.getByText(DESKTOP_LINE)).toBeInTheDocument();
    expect(screen.queryByText(HOST_LINE)).not.toBeInTheDocument();
    expect(screen.queryByTestId("system-backup-disclosure")).not.toBeInTheDocument();
  });

  test("the line shows the desktop wording until the host answers", () => {
    mocks.buildInfo.mockReturnValue(new Promise(() => undefined));
    renderWithClient(<CreateVaultScreen />);
    expect(screen.getByText(DESKTOP_LINE)).toBeInTheDocument();
  });

  test("a host whose system backups include the vault changes the line and adds the disclosure", async () => {
    mocks.buildInfo.mockResolvedValue(buildInfo("Test Device"));
    renderWithClient(<CreateVaultScreen />);
    expect(await screen.findByText(HOST_LINE)).toBeInTheDocument();
    expect(screen.queryByText(DESKTOP_LINE)).not.toBeInTheDocument();
    const note = screen.getByTestId("system-backup-disclosure");
    expect(note.textContent).toBe(
      "Test Device backups include your encrypted vault; your password is still required.",
    );
    expect(note).toHaveAttribute("role", "note");
  });
});
