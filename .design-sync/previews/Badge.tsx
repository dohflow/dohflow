import { Badge } from "@personal-cfo/desktop";

export function Variants() {
  return (
    <div className="flex flex-wrap items-center gap-2">
      <Badge>Active</Badge>
      <Badge variant="secondary">Draft</Badge>
      <Badge variant="outline">Manual</Badge>
      <Badge variant="gain">+2.4%</Badge>
      <Badge variant="warning">Due soon</Badge>
      <Badge variant="info">Pending</Badge>
      <Badge variant="loss">−1.2%</Badge>
    </div>
  );
}

export function StatusChips() {
  return (
    <div className="flex flex-wrap items-center gap-2">
      <Badge variant="gain">Paid</Badge>
      <Badge variant="info">Pending</Badge>
      <Badge variant="warning">Due soon</Badge>
      <Badge variant="loss">Overdue</Badge>
    </div>
  );
}

export function Categories() {
  return (
    <div className="flex flex-wrap items-center gap-2">
      <Badge variant="secondary">Groceries</Badge>
      <Badge variant="secondary">Dining</Badge>
      <Badge variant="secondary">Rent</Badge>
      <Badge variant="secondary">Utilities</Badge>
      <Badge variant="outline">Uncategorized</Badge>
    </div>
  );
}
