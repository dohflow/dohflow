import { Input, Label } from "@personal-cfo/desktop";

export function FieldLabel() {
  return (
    <div className="flex flex-col gap-2" style={{ width: 280 }}>
      <Label htmlFor="account-name">Account name</Label>
      <Input id="account-name" type="text" defaultValue="Chase Sapphire" />
    </div>
  );
}

export function RequiredField() {
  return (
    <div className="flex flex-col gap-2" style={{ width: 280 }}>
      <Label htmlFor="budget-name">
        Budget name
        <span style={{ color: "var(--loss)" }}> *</span>
      </Label>
      <Input id="budget-name" type="text" placeholder="e.g. Groceries" required />
    </div>
  );
}

export function LabeledFields() {
  return (
    <div className="grid grid-cols-2 gap-4">
      <div className="flex flex-col gap-2" style={{ width: 220 }}>
        <Label htmlFor="lf-amount">Monthly limit</Label>
        <Input id="lf-amount" type="number" defaultValue="600" />
      </div>
      <div className="flex flex-col gap-2" style={{ width: 220 }}>
        <Label htmlFor="lf-category">Category</Label>
        <Input id="lf-category" type="text" defaultValue="Dining out" />
      </div>
    </div>
  );
}
