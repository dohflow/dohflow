import { useState } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import type { BuildInfoDto } from "@/bindings";
import {
  UPDATE_HANDLE_QUERY_KEY,
  useSoftwareUpdate,
  type ApplyOutcome,
} from "./useSoftwareUpdate";

// personal-cfo-sk4xr: `useSoftwareUpdate`'s live `Update` handle used to live in a
// per-component `useRef`, but its `["software-update"]` queryFn is SHARED across
// every caller — TanStack Query dedupes concurrent/cached fetches, so only the ONE
// observer whose queryFn actually ran got a populated ref. A SECOND observer that
// mounted later (e.g. `UpdateAvailableNotice`'s always-mounted toast, then
// `SoftwareUpdateCard` in Settings) saw `status.available: true` from the shared
// cache while its OWN ref was still null, so its `apply()` incorrectly reported
// "No update is available to install." even though one genuinely was.

const mocks = vi.hoisted(() => ({
  buildInfo: vi.fn(),
  checkForUpdate: vi.fn(),
  applyUpdate: vi.fn(),
  relaunchApp: vi.fn(),
  pluginCheck: vi.fn(),
  pluginRelaunch: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    buildInfo: mocks.buildInfo,
    checkForUpdate: mocks.checkForUpdate,
    applyUpdate: mocks.applyUpdate,
    relaunchApp: mocks.relaunchApp,
  },
}));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: mocks.pluginCheck }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: mocks.pluginRelaunch }));

function buildInfo(over: Partial<BuildInfoDto> = {}): BuildInfoDto {
  return {
    version: "0.1.0",
    commit: "abc1234",
    channel: "release",
    built_at: "2026-07-20T14:32:05Z",
    dirty: false,
    os_version: "15.7.9",
    self_update_available: true,
    system_backup_device: null,
    ...over,
  };
}

/// A minimal plugin `Update` stub, matching `SoftwareUpdateCard.test.tsx`'s.
function pluginUpdate(over: Partial<Record<string, unknown>> = {}) {
  return {
    version: "0.2.0",
    currentVersion: "0.1.0",
    date: "2026-08-01",
    body: "Bug fixes.",
    downloadAndInstall: vi.fn().mockResolvedValue(undefined),
    ...over,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.buildInfo.mockResolvedValue(buildInfo());
});

/// A single hook observer, wired up like `SoftwareUpdateCard` for the parts this
/// test needs: a probe for whether `status` shows an available update, and a
/// button that calls `apply()` and records the outcome.
function Observer({ label }: { label: string }) {
  const { status, apply } = useSoftwareUpdate();
  const [result, setResult] = useState<ApplyOutcome | null>(null);
  return (
    <div>
      <div data-testid={`${label}-available`}>
        {String(status?.kind === "release" && status.available)}
      </div>
      <button
        onClick={() => {
          void apply().then(setResult);
        }}
      >
        apply-{label}
      </button>
      {result && (
        <div data-testid={`${label}-result`}>{JSON.stringify(result)}</div>
      )}
    </div>
  );
}

/// Two independent `useSoftwareUpdate()` observers sharing ONE `QueryClient` — the
/// exact shape of the real bug (`UpdateAvailableNotice` always mounted, then
/// `SoftwareUpdateCard` mounting later in the SAME session). `mountSecond` starts
/// `false` so the first observer's queryFn resolves and populates the cache BEFORE
/// the second observer ever mounts, matching the real timeline; the test mounts
/// the second one afterward via `rerender`, not both at once, so the second
/// observer's own `useQuery` call genuinely never runs its `queryFn` — it is
/// served straight from the shared cache.
function TwoObservers({ mountSecond }: { mountSecond: boolean }) {
  return (
    <div>
      <Observer label="first" />
      {mountSecond && <Observer label="second" />}
    </div>
  );
}

function renderTwoObservers() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const view = render(
    <QueryClientProvider client={queryClient}>
      <TwoObservers mountSecond={false} />
    </QueryClientProvider>,
  );
  return { queryClient, ...view };
}

test("a second observer mounting after the first's check still has an update to apply", async () => {
  const update = pluginUpdate();
  mocks.pluginCheck.mockResolvedValue(update);
  mocks.pluginRelaunch.mockResolvedValue(undefined);
  const { queryClient, rerender } = renderTwoObservers();

  // First observer's queryFn runs and resolves — this is the ONLY check that
  // happens; the shared cache now has `available: true` AND the live handle.
  await waitFor(() =>
    expect(screen.getByTestId("first-available")).toHaveTextContent("true"),
  );
  expect(mocks.pluginCheck).toHaveBeenCalledTimes(1);

  // Mount the second observer now, after the check already resolved and cached
  // (`staleTime: 5min`) — its own `useQuery` call is served from cache, never
  // running its own queryFn.
  rerender(
    <QueryClientProvider client={queryClient}>
      <TwoObservers mountSecond={true} />
    </QueryClientProvider>,
  );
  await waitFor(() =>
    expect(screen.getByTestId("second-available")).toHaveTextContent("true"),
  );
  // Confirms the second observer really did NOT re-check — the fix must be
  // reading the shared handle, not silently falling back to a second check
  // every time (which would still "work" but isn't what's being proven here).
  expect(mocks.pluginCheck).toHaveBeenCalledTimes(1);

  // The actual regression: apply() from the SECOND observer must succeed.
  fireEvent.click(screen.getByRole("button", { name: "apply-second" }));
  await waitFor(() =>
    expect(screen.getByTestId("second-result")).toHaveTextContent(
      JSON.stringify({ ok: true }),
    ),
  );
  expect(update.downloadAndInstall).toHaveBeenCalledTimes(1);
});

test("apply() re-checks rather than refuses when the shared handle has been evicted", async () => {
  const firstUpdate = pluginUpdate();
  mocks.pluginCheck.mockResolvedValue(firstUpdate);
  const { queryClient, rerender } = renderTwoObservers();
  await waitFor(() =>
    expect(screen.getByTestId("first-available")).toHaveTextContent("true"),
  );
  expect(mocks.pluginCheck).toHaveBeenCalledTimes(1);

  // Simulate the handle having fallen out of the cache independently of the
  // status data (e.g. `gcTime` elapsed with no observer of the second key, or
  // some future refactor race) — `status.available` is untouched, only the
  // handle itself is gone, which is exactly the case `apply()`'s fallback must
  // cover per the owner's fix direction ("re-check rather than refuse").
  queryClient.removeQueries({ queryKey: UPDATE_HANDLE_QUERY_KEY });

  const secondUpdate = pluginUpdate({ version: "0.3.0" });
  mocks.pluginCheck.mockResolvedValue(secondUpdate);
  rerender(
    <QueryClientProvider client={queryClient}>
      <TwoObservers mountSecond={true} />
    </QueryClientProvider>,
  );
  await waitFor(() =>
    expect(screen.getByTestId("second-available")).toHaveTextContent("true"),
  );

  fireEvent.click(screen.getByRole("button", { name: "apply-second" }));
  await waitFor(() =>
    expect(screen.getByTestId("second-result")).toHaveTextContent(
      JSON.stringify({ ok: true }),
    ),
  );
  // The re-check ran (a second `check()` call) rather than refusing outright,
  // and installed whatever it found — never a silent no-op.
  expect(mocks.pluginCheck).toHaveBeenCalledTimes(2);
  expect(firstUpdate.downloadAndInstall).not.toHaveBeenCalled();
  expect(secondUpdate.downloadAndInstall).toHaveBeenCalledTimes(1);
});

// personal-cfo-md8h2.4: the minimum-macOS guard. An update whose manifest floor is newer than
// this Mac's macOS is never offered, never kept as an installable handle, and apply() refuses it.
test("an update needing a newer macOS is held back and cannot be installed", async () => {
  mocks.buildInfo.mockResolvedValue(buildInfo({ os_version: "14.8.9" }));
  const close = vi.fn().mockResolvedValue(undefined);
  const update = pluginUpdate({
    rawJson: { version: "0.4.0", minimumSystemVersion: "15.0" },
    version: "0.4.0",
    close,
  });
  mocks.pluginCheck.mockResolvedValue(update);
  const { queryClient } = renderTwoObservers();

  await waitFor(() => expect(mocks.pluginCheck).toHaveBeenCalledTimes(1));
  await waitFor(() => expect(close).toHaveBeenCalledTimes(1));
  expect(screen.getByTestId("first-available")).toHaveTextContent("false");
  expect(queryClient.getQueryData(UPDATE_HANDLE_QUERY_KEY)).toBeNull();

  fireEvent.click(screen.getByRole("button", { name: "apply-first" }));
  await waitFor(() =>
    expect(screen.getByTestId("first-result")).toHaveTextContent(
      "DohFlow 0.4.0 needs macOS 15.0 or later.",
    ),
  );
  expect(update.downloadAndInstall).not.toHaveBeenCalled();
});

test("an update whose macOS floor this Mac meets is offered as before", async () => {
  mocks.buildInfo.mockResolvedValue(buildInfo({ os_version: "14.8.9" }));
  const update = pluginUpdate({ rawJson: { minimumSystemVersion: "14.0" } });
  mocks.pluginCheck.mockResolvedValue(update);
  mocks.pluginRelaunch.mockResolvedValue(undefined);
  renderTwoObservers();

  await waitFor(() =>
    expect(screen.getByTestId("first-available")).toHaveTextContent("true"),
  );
  fireEvent.click(screen.getByRole("button", { name: "apply-first" }));
  await waitFor(() =>
    expect(screen.getByTestId("first-result")).toHaveTextContent(JSON.stringify({ ok: true })),
  );
  expect(update.downloadAndInstall).toHaveBeenCalledTimes(1);
});

// personal-cfo-i93n3.12: a host whose build cannot update itself reports it in build_info,
// and the hook then invokes nothing — neither the plugin nor any update command.
describe("a host without self-update", () => {
  function Probe() {
    const { status, selfUpdate, check, apply, relaunch } = useSoftwareUpdate();
    return (
      <div>
        <div data-testid="supported">{String(selfUpdate)}</div>
        <div data-testid="status">{String(status)}</div>
        <button
          onClick={() => {
            void check();
            void apply().then((r) => {
              document.title = JSON.stringify(r);
            });
            void relaunch();
          }}
        >
          try-everything
        </button>
      </div>
    );
  }

  function renderProbe() {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    return render(
      <QueryClientProvider client={client}>
        <Probe />
      </QueryClientProvider>,
    );
  }

  function expectNoUpdateCall() {
    expect(mocks.pluginCheck).not.toHaveBeenCalled();
    expect(mocks.pluginRelaunch).not.toHaveBeenCalled();
    expect(mocks.checkForUpdate).not.toHaveBeenCalled();
    expect(mocks.applyUpdate).not.toHaveBeenCalled();
    expect(mocks.relaunchApp).not.toHaveBeenCalled();
  }

  test.each(["dev", "release"])("%s channel: no check, apply or relaunch ever runs", async (channel) => {
    mocks.buildInfo.mockResolvedValue(buildInfo({ channel, self_update_available: false }));
    renderProbe();
    await waitFor(() => expect(mocks.buildInfo).toHaveBeenCalled());
    await waitFor(() => expect(screen.getByTestId("supported")).toHaveTextContent("false"));
    expect(screen.getByTestId("status")).toHaveTextContent("null");

    fireEvent.click(screen.getByRole("button", { name: "try-everything" }));
    await waitFor(() =>
      expect(document.title).toBe(
        JSON.stringify({ ok: false, message: "This build can't update itself." }),
      ),
    );
    expectNoUpdateCall();
  });

  test("while build info is still unknown nothing runs either", async () => {
    mocks.buildInfo.mockReturnValue(new Promise(() => undefined));
    renderProbe();
    expect(screen.getByTestId("supported")).toHaveTextContent("false");
    fireEvent.click(screen.getByRole("button", { name: "try-everything" }));
    expectNoUpdateCall();
  });
});
