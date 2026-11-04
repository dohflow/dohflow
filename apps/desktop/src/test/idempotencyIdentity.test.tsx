import { fireEvent, render, screen } from "@testing-library/react";
import { useRef, useState } from "react";

import { expectDraftKeyIdentity } from "./idempotencyIdentity";

/// A tiny form with a per-draft key, to prove the helper passes a correct identity and FAILS one that
/// leaves a field out. `identityFields` is what the form believes identifies a draft.
function Form({
  identityFields,
  send,
  resetOnSuccess = true,
  keepOnRefusal = false,
}: {
  identityFields: ("amount" | "date")[];
  send: (key: string) => Promise<"unknown" | "refused" | "ok">;
  /// False models the bug: the key outlives the write it belonged to.
  resetOnSuccess?: boolean;
  /// True models a form that keeps its key through a typed refusal (a valid choice, FRONTEND section 8).
  keepOnRefusal?: boolean;
}) {
  const [amount, setAmount] = useState("5");
  const [date, setDate] = useState("2026-01-01");
  const [error, setError] = useState("");
  const [open, setOpen] = useState(true);
  const draft = useRef<{ identity: string; key: string } | null>(null);
  if (!open) return <button onClick={() => setOpen(true)}>Reopen</button>;
  return (
    <div>
      <input aria-label="amount" value={amount} onChange={(e) => setAmount(e.target.value)} />
      <input aria-label="date" value={date} onChange={(e) => setDate(e.target.value)} />
      <button
        onClick={async () => {
          const identity = JSON.stringify(
            identityFields.map((f) => (f === "amount" ? amount : date)),
          );
          if (draft.current?.identity !== identity) {
            draft.current = { identity, key: crypto.randomUUID() };
          }
          const outcome = await send(draft.current.key);
          // A refusal is a confirmed failure: the next try is a new draft.
          if (outcome === "refused" && !keepOnRefusal) draft.current = null;
          // A success is final: the form closes, and (unless the bug is modelled) the draft is over.
          if (outcome === "ok") {
            if (resetOnSuccess) draft.current = null;
            setOpen(false);
          }
          setError(outcome);
        }}
      >
        Send
      </button>
      <p>{error}</p>
    </div>
  );
}

function harness(identityFields: ("amount" | "date")[], resetOnSuccess = true, keepOnRefusal = false) {
  const keys: string[] = [];
  let refuseNext = false;
  let succeedNext = false;
  const spec = {
    name: "fake form",
    open: () => {
      render(
        <Form
          identityFields={identityFields}
          resetOnSuccess={resetOnSuccess}
          keepOnRefusal={keepOnRefusal}
          send={async (key) => {
            keys.push(key);
            if (succeedNext) {
              succeedNext = false;
              return "ok";
            }
            if (refuseNext) {
              refuseNext = false;
              return "refused";
            }
            return "unknown";
          }}
        />,
      );
    },
    send: () => {
      fireEvent.click(screen.getByRole("button", { name: "Send" }));
    },
    keys: () => keys,
    fields: [
      { name: "amount", change: () => { fireEvent.change(screen.getByLabelText("amount"), { target: { value: String(Math.random()) } }); } },
      { name: "date", change: () => { fireEvent.change(screen.getByLabelText("date"), { target: { value: String(Math.random()) } }); } },
    ],
    refusal: { arrange: () => { refuseNext = true; } },
    success: {
      arrange: () => { succeedNext = true; },
      reopen: async () => { fireEvent.click(await screen.findByRole("button", { name: "Reopen" })); },
    },
  };
  return spec;
}

describe("expectDraftKeyIdentity (personal-cfo-56w.8)", () => {
  it("passes a form whose draft identity covers every listed field", async () => {
    await expectDraftKeyIdentity(harness(["amount", "date"]));
  });

  it("fails a form that leaves a field out of its draft identity, and names the field", async () => {
    await expect(expectDraftKeyIdentity(harness(["amount"]))).rejects.toThrow(/changing "date" must give a new key/);
  });

  it("fails a form whose key changes on every send", async () => {
    const spec = harness(["amount", "date"]);
    // A form that mints a key per send never reuses one: model it by changing the draft in `send`.
    const send = spec.send;
    let n = 0;
    spec.send = () => {
      fireEvent.change(screen.getByLabelText("amount"), { target: { value: `x${n++}` } });
      send();
    };
    await expect(expectDraftKeyIdentity(spec)).rejects.toThrow(/unchanged retry reuses the key/);
  });

  it("fails a form that keeps its key after a confirmed success, and names the step", async () => {
    await expect(expectDraftKeyIdentity(harness(["amount", "date"], false))).rejects.toThrow(
      /after a confirmed success the same draft gets a new key/,
    );
  });

  it("passes when no success step is given, even for a form that keeps its key after success", async () => {
    const { success: _success, ...withoutSuccess } = harness(["amount", "date"], false);
    void _success;
    await expectDraftKeyIdentity(withoutSuccess);
  });

  it("fails when no identity field is listed", async () => {
    await expect(expectDraftKeyIdentity({ ...harness(["amount"]), fields: [] })).rejects.toThrow(/list the draft's identity fields/);
  });

  it("fails a form that keeps its key after a refusal", async () => {
    const keys: string[] = [];
    let refuseNext = false;
    void refuseNext;
    function Sticky() {
      const [amount, setAmount] = useState("5");
      const key = useRef(crypto.randomUUID());
      return (
        <div>
          <input aria-label="amount" value={amount} onChange={(e) => { setAmount(e.target.value); key.current = crypto.randomUUID(); }} />
          <button
            onClick={() => {
              keys.push(key.current);
              refuseNext = false;
            }}
          >
            Send
          </button>
        </div>
      );
    }
    await expect(
      expectDraftKeyIdentity({
        name: "sticky",
        open: () => { render(<Sticky />); },
        send: () => { fireEvent.click(screen.getByRole("button", { name: "Send" })); },
        keys: () => keys,
        fields: [{ name: "amount", change: () => { fireEvent.change(screen.getByLabelText("amount"), { target: { value: "9" } }); } }],
        refusal: { arrange: () => { refuseNext = true; } },
      }),
    ).rejects.toThrow(/after a refusal is a new draft/);
  });

  describe("a refusal that keeps the draft (mode: same-draft)", () => {
    const sameDraft = (spec: ReturnType<typeof harness>) => ({
      ...spec,
      refusal: { arrange: spec.refusal.arrange, mode: "same-draft" as const },
    });

    it("passes a form that keeps its key after a refusal and still re-keys an edited field", async () => {
      await expectDraftKeyIdentity(sameDraft(harness(["amount", "date"], true, true)));
    });

    it("fails a form that mints a new key after a refusal, and names the step", async () => {
      await expect(expectDraftKeyIdentity(sameDraft(harness(["amount", "date"])))).rejects.toThrow(
        /after a refusal keeps the draft, so it reuses the key/,
      );
    });

    it("still fails a form that keeps its key after a refusal when new-draft is expected", async () => {
      await expect(expectDraftKeyIdentity(harness(["amount", "date"], true, true))).rejects.toThrow(
        /after a refusal is a new draft/,
      );
    });

    it("still names a field left out of the identity", async () => {
      await expect(expectDraftKeyIdentity(sameDraft(harness(["amount"], true, true)))).rejects.toThrow(
        /changing "date" must give a new key/,
      );
    });
  });
});
