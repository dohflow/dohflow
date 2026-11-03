import { fireEvent, render, screen } from "@testing-library/react";
import { useRef, useState } from "react";

import { expectDraftKeyIdentity } from "./idempotencyIdentity";

/// A tiny form with a per-draft key, to prove the helper passes a correct identity and FAILS one that
/// leaves a field out. `identityFields` is what the form believes identifies a draft.
function Form({
  identityFields,
  send,
}: {
  identityFields: ("amount" | "date")[];
  send: (key: string) => Promise<"unknown" | "refused">;
}) {
  const [amount, setAmount] = useState("5");
  const [date, setDate] = useState("2026-01-01");
  const [error, setError] = useState("");
  const draft = useRef<{ identity: string; key: string } | null>(null);
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
          if (outcome === "refused") draft.current = null;
          setError(outcome);
        }}
      >
        Send
      </button>
      <p>{error}</p>
    </div>
  );
}

function harness(identityFields: ("amount" | "date")[]) {
  const keys: string[] = [];
  let refuseNext = false;
  const spec = {
    name: "fake form",
    open: () => {
      render(
        <Form
          identityFields={identityFields}
          send={async (key) => {
            keys.push(key);
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
});
