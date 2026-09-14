import { Button, EmptyState } from "@personal-cfo/desktop";
import { Inbox, Wallet } from "lucide-react";

export function NoTransactions() {
  return (
    <div style={{ width: 420 }} className="border rounded-lg">
      <EmptyState
        icon={Inbox}
        title="No transactions yet"
        description="Import a bank statement to see your spending here."
        action={
          <Button className="mt-2">Import statement</Button>
        }
      />
    </div>
  );
}

export function EmptyVault() {
  return (
    <div style={{ width: 420 }} className="border rounded-lg">
      <EmptyState
        icon={Wallet}
        title="No accounts linked"
        description="Add a checking, savings, or credit account to start tracking your balances."
      />
    </div>
  );
}
