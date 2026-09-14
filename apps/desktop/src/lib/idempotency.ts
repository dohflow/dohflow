/// Mint a fresh idempotency key for ONE user action (personal-cfo-3fdd.5).
///
/// The backend replays — rather than re-applies — a mutation that arrives twice
/// with the same key, so a double-fired submit or retried IPC call posts money
/// exactly once. Passing `""` defeats that: the server mints a fresh key per
/// call, and a retry double-posts.
///
/// Call this where the mutation's input is BUILT — the submit handler, action
/// button, or `mutationFn` body — so the key is minted once per invocation.
/// Never mint at component render (a memoized key would make two *different*
/// submissions collide and silently drop the second), and in bulk loops mint
/// per row per invocation.
export function mintIdempotencyKey(): string {
  return crypto.randomUUID();
}
