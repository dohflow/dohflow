import { onlineManager } from "@tanstack/react-query";

import { createQueryClient } from "./queryClient";

/// A mounted client, as `QueryClientProvider` has it: only a mounted client
/// listens to `onlineManager`, so only then does an offline event reach it.
const mounted: Array<{ unmount: () => void }> = [];
function mountedClient() {
  const client = createQueryClient();
  client.mount();
  mounted.push(client);
  return client;
}

/// What the OS does when the machine loses its network: the `offline` event, and
/// `navigator.onLine` going false.
function goOffline() {
  vi.spyOn(navigator, "onLine", "get").mockReturnValue(false);
  window.dispatchEvent(new Event("offline"));
}

afterEach(() => {
  mounted.splice(0).forEach((client) => client.unmount());
  vi.restoreAllMocks();
  onlineManager.setOnline(true);
});

test("a read still resolves when the OS reports no network", async () => {
  const client = mountedClient();
  goOffline();
  await expect(
    client.fetchQuery({ queryKey: ["accounts"], queryFn: async () => "rows" }),
  ).resolves.toBe("rows");
  expect(onlineManager.isOnline()).toBe(true);
});

test("a query observed while the OS reports no network is not paused", async () => {
  const client = mountedClient();
  goOffline();
  const observer = client.getQueryCache().build(client, {
    queryKey: ["observed"],
    queryFn: async () => "rows",
  });
  await observer.fetch();
  expect(observer.state.fetchStatus).toBe("idle");
  expect(observer.state.data).toBe("rows");
});

test("a write still runs when the OS reports no network", async () => {
  const client = mountedClient();
  goOffline();
  const mutation = client
    .getMutationCache()
    .build(client, { mutationFn: async (n: number) => n + 1 });
  await expect(mutation.execute(1)).resolves.toBe(2);
  expect(mutation.state.isPaused).toBe(false);
});

test("the app can still pause queries itself, as a key rotation does", async () => {
  const client = mountedClient();
  onlineManager.setOnline(false);
  const queryFn = vi.fn(async () => "rows");
  const pending = client.fetchQuery({ queryKey: ["paused"], queryFn });
  await Promise.resolve();
  expect(client.getQueryState(["paused"])?.fetchStatus).toBe("paused");
  onlineManager.setOnline(true);
  await expect(pending).resolves.toBe("rows");
});
