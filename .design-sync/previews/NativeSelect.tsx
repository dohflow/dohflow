import { Label, NativeSelect } from "@personal-cfo/desktop";

export function Default() {
  return (
    <div className="flex flex-col gap-2" style={{ width: 280 }}>
      <Label htmlFor="account-picker">Account</Label>
      <NativeSelect id="account-picker" defaultValue="checking" className="w-full">
        <option value="checking">Chase Checking</option>
        <option value="savings">Ally Savings</option>
        <option value="credit">Sapphire Credit Card</option>
        <option value="brokerage">Fidelity Brokerage</option>
      </NativeSelect>
    </div>
  );
}

export function Small() {
  return (
    <div className="flex items-center gap-3">
      <span className="text-sm text-muted-foreground">Filter</span>
      <NativeSelect size="sm" defaultValue="all" style={{ width: 160 }}>
        <option value="all">All categories</option>
        <option value="dining">Dining out</option>
        <option value="groceries">Groceries</option>
        <option value="transport">Transport</option>
      </NativeSelect>
    </div>
  );
}

export function States() {
  return (
    <div className="grid grid-cols-2 gap-4">
      <div className="flex flex-col gap-2" style={{ width: 220 }}>
        <Label htmlFor="ns-active">Statement period</Label>
        <NativeSelect id="ns-active" defaultValue="jun" className="w-full">
          <option value="jun">June 2026</option>
          <option value="may">May 2026</option>
          <option value="apr">April 2026</option>
        </NativeSelect>
      </div>
      <div className="flex flex-col gap-2" style={{ width: 220 }}>
        <Label htmlFor="ns-disabled">Currency</Label>
        <NativeSelect id="ns-disabled" defaultValue="usd" disabled className="w-full">
          <option value="usd">USD — US Dollar</option>
        </NativeSelect>
      </div>
    </div>
  );
}
