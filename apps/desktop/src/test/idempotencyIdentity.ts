/// A shared check for a per-draft idempotency key (personal-cfo-56w.8; docs/agent/FRONTEND.md
/// section 8).
///
/// A write that can be retried after an unknown outcome carries one key per DRAFT: the same draft
/// sent again reuses the key (so the host replays instead of writing twice), and any change to the
/// draft is a new draft with a new key. The code is easy to get right and easy to get subtly wrong:
/// a field left out of the draft's identity still sends the right amount, and a retry after editing
/// that field silently replays the OLD write. Nothing in an ordinary test notices, so this helper
/// pins it: it names every field of the draft's identity and fails when changing any one of them
/// leaves the key unchanged.
///
/// It is framework-neutral: the caller supplies the steps (render and fill, send once, read the keys
/// the mock received, change one field), so it works for a form, a modal or a batch row alike. The
/// mock the caller sends to must make each send leave the form open and the draft in place (reject
/// the call, as a lost connection does: that is the unknown outcome a retry follows).

export type DraftField = {
  /// The field, as the person names it ("amount", "From", "date"). Appears in the failure message.
  name: string;
  /// Changes this one field to a different valid value, from wherever the form is now.
  change: () => void | Promise<void>;
};

export type DraftKeySpec = {
  /// What is under test, for failure messages.
  name: string;
  /// Puts a valid draft on screen, ready to send.
  open: () => void | Promise<void>;
  /// Sends the current draft once (a click on its submit control). The helper waits for the call.
  send: () => void | Promise<void>;
  /// Every idempotency key the mock has received so far, oldest first.
  keys: () => readonly string[];
  /// EVERY field that is part of the draft's identity. A field missing here is a field nothing pins.
  fields: readonly DraftField[];
  /// Where a typed refusal counts as a new draft (the convention for a refused write: nothing was
  /// saved, so the next try is a new draft). `arrange` makes the NEXT send come back refused.
  refusal?: { arrange: () => void };
  /// Where a confirmed success closes the draft: the same figures sent again afterwards are a NEW
  /// draft and must carry a new key (the bug is a key kept after the write it belonged to succeeded,
  /// so a second, identical write replays the first). `arrange` makes the NEXT send succeed; the
  /// helper then sends once, calls `reopen` if the success closed the form (it must put the SAME
  /// draft back on screen, ready to send), and sends again.
  success?: { arrange: () => void; reopen?: () => void | Promise<void> };
};

async function sendOnce(spec: DraftKeySpec): Promise<string> {
  const before = spec.keys().length;
  await spec.send();
  await vi.waitFor(() => expect(spec.keys().length, `${spec.name}: a send reaches the host`).toBe(before + 1));
  return spec.keys()[before] as string;
}

export async function expectDraftKeyIdentity(spec: DraftKeySpec): Promise<void> {
  expect(spec.fields.length, `${spec.name}: list the draft's identity fields`).toBeGreaterThan(0);
  await spec.open();

  // The same draft, sent again, replays: one key.
  const first = await sendOnce(spec);
  expect(first, `${spec.name}: a key is sent`).toBeTruthy();
  const again = await sendOnce(spec);
  expect(again, `${spec.name}: an unchanged retry reuses the key`).toBe(first);

  // A confirmed success is final: the same figures sent again are a new draft. This runs on the
  // draft as first opened, so `reopen` only has to rebuild what `open` built.
  const seen = new Set([first]);
  let previousKey = first;
  if (spec.success) {
    spec.success.arrange();
    const succeeded = await sendOnce(spec);
    expect(succeeded, `${spec.name}: the send that succeeds is the same draft`).toBe(first);
    await spec.success.reopen?.();
    const next = await sendOnce(spec);
    expect(next, `${spec.name}: after a confirmed success the same draft gets a new key`).not.toBe(succeeded);
    seen.add(next);
    previousKey = next;
  }

  // Changing ANY one field is a new draft, and none of the keys so far may come back.
  let previous = previousKey;
  for (const field of spec.fields) {
    await field.change();
    const key = await sendOnce(spec);
    expect(seen.has(key), `${spec.name}: changing "${field.name}" must give a new key`).toBe(false);
    seen.add(key);
    previous = key;
    // Sent again unchanged, it is the same draft once more.
    expect(await sendOnce(spec), `${spec.name}: a retry after changing "${field.name}" reuses its key`).toBe(key);
  }

  if (spec.refusal) {
    // The next send is refused. The draft is unchanged, so it still carries the previous key...
    spec.refusal.arrange();
    const refused = await sendOnce(spec);
    expect(refused, `${spec.name}: the refused send is the same draft`).toBe(previous);
    // ...but a refusal is a confirmed failure, so the try after it is a NEW draft.
    const next = await sendOnce(spec);
    expect(next, `${spec.name}: the send after a refusal is a new draft with a new key`).not.toBe(refused);
  }
}
