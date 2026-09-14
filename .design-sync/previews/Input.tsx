import { Input, Label } from "@personal-cfo/desktop";

export function Default() {
  return (
    <div className="flex flex-col gap-2" style={{ width: 280 }}>
      <Label htmlFor="amount">Amount</Label>
      <Input id="amount" type="text" defaultValue="1,240.00" />
    </div>
  );
}

export function Types() {
  return (
    <div className="grid grid-cols-2 gap-4">
      <div className="flex flex-col gap-2" style={{ width: 220 }}>
        <Label htmlFor="payee">Payee</Label>
        <Input id="payee" type="text" defaultValue="Whole Foods Market" />
      </div>
      <div className="flex flex-col gap-2" style={{ width: 220 }}>
        <Label htmlFor="amt">Amount</Label>
        <Input id="amt" type="number" defaultValue="86.42" />
      </div>
      <div className="flex flex-col gap-2" style={{ width: 220 }}>
        <Label htmlFor="search">Search transactions</Label>
        <Input id="search" type="search" placeholder="Search transactions" />
      </div>
      <div className="flex flex-col gap-2" style={{ width: 220 }}>
        <Label htmlFor="posted">Posted date</Label>
        <Input id="posted" type="date" defaultValue="2026-06-14" />
      </div>
    </div>
  );
}

export function States() {
  return (
    <div className="grid grid-cols-3 gap-4">
      <div className="flex flex-col gap-2" style={{ width: 200 }}>
        <Label htmlFor="s-empty">Memo</Label>
        <Input id="s-empty" type="text" placeholder="Add a memo" />
      </div>
      <div className="flex flex-col gap-2" style={{ width: 200 }}>
        <Label htmlFor="s-filled">Account name</Label>
        <Input id="s-filled" type="text" defaultValue="Chase Checking" />
      </div>
      <div className="flex flex-col gap-2" style={{ width: 200 }}>
        <Label htmlFor="s-disabled">Institution</Label>
        <Input id="s-disabled" type="text" defaultValue="Read-only" disabled />
      </div>
    </div>
  );
}
