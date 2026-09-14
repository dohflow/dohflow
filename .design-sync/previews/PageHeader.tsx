import { Button, PageHeader } from "@personal-cfo/desktop";

export function WithActions() {
  return (
    <div style={{ width: 560 }}>
      <PageHeader
        title="Transactions"
        subtitle="1,284 across 4 accounts"
        actions={
          <>
            <Button variant="outline">Import</Button>
            <Button>Add transaction</Button>
          </>
        }
      />
    </div>
  );
}

export function TitleOnly() {
  return (
    <div style={{ width: 560 }}>
      <PageHeader
        title="June cash flow"
        subtitle="Income minus spending, last 30 days"
      />
    </div>
  );
}
