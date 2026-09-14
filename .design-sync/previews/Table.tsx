import {
  Badge,
  Table,
  TableBody,
  TableCell,
  TableFooter,
  TableHead,
  TableHeader,
  TableRow,
} from "@personal-cfo/desktop";

export function TransactionsTable() {
  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>Date</TableHead>
          <TableHead>Description</TableHead>
          <TableHead>Category</TableHead>
          <TableHead className="text-right">Amount</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        <TableRow>
          <TableCell className="tabular-nums text-muted-foreground">
            Jun 28, 2026
          </TableCell>
          <TableCell className="font-medium">Whole Foods Market</TableCell>
          <TableCell>
            <Badge variant="secondary">Groceries</Badge>
          </TableCell>
          <TableCell
            className="text-right tabular-nums"
            style={{ color: "var(--loss)" }}
          >
            −$142.68
          </TableCell>
        </TableRow>
        <TableRow>
          <TableCell className="tabular-nums text-muted-foreground">
            Jun 27, 2026
          </TableCell>
          <TableCell className="font-medium">Acme Payroll</TableCell>
          <TableCell>
            <Badge variant="secondary">Income</Badge>
          </TableCell>
          <TableCell
            className="text-right tabular-nums"
            style={{ color: "var(--gain)" }}
          >
            +$3,120.00
          </TableCell>
        </TableRow>
        <TableRow>
          <TableCell className="tabular-nums text-muted-foreground">
            Jun 25, 2026
          </TableCell>
          <TableCell className="font-medium">Pacific Gas &amp; Electric</TableCell>
          <TableCell>
            <Badge variant="secondary">Utilities</Badge>
          </TableCell>
          <TableCell
            className="text-right tabular-nums"
            style={{ color: "var(--loss)" }}
          >
            −$88.40
          </TableCell>
        </TableRow>
        <TableRow>
          <TableCell className="tabular-nums text-muted-foreground">
            Jun 24, 2026
          </TableCell>
          <TableCell className="font-medium">Blue Bottle Coffee</TableCell>
          <TableCell>
            <Badge variant="secondary">Dining</Badge>
          </TableCell>
          <TableCell
            className="text-right tabular-nums"
            style={{ color: "var(--loss)" }}
          >
            −$6.75
          </TableCell>
        </TableRow>
        <TableRow>
          <TableCell className="tabular-nums text-muted-foreground">
            Jun 22, 2026
          </TableCell>
          <TableCell className="font-medium">Chase Sapphire Refund</TableCell>
          <TableCell>
            <Badge variant="secondary">Reimbursement</Badge>
          </TableCell>
          <TableCell
            className="text-right tabular-nums"
            style={{ color: "var(--gain)" }}
          >
            +$54.20
          </TableCell>
        </TableRow>
        <TableRow>
          <TableCell className="tabular-nums text-muted-foreground">
            Jun 20, 2026
          </TableCell>
          <TableCell className="font-medium">Equinox Membership</TableCell>
          <TableCell>
            <Badge variant="secondary">Health</Badge>
          </TableCell>
          <TableCell
            className="text-right tabular-nums"
            style={{ color: "var(--loss)" }}
          >
            −$215.00
          </TableCell>
        </TableRow>
      </TableBody>
      <TableFooter>
        <TableRow>
          <TableCell>Net</TableCell>
          <TableCell />
          <TableCell />
          <TableCell
            className="text-right tabular-nums"
            style={{ color: "var(--gain)" }}
          >
            +$2,721.37
          </TableCell>
        </TableRow>
      </TableFooter>
    </Table>
  );
}

export function AccountsTable() {
  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>Account</TableHead>
          <TableHead>Type</TableHead>
          <TableHead>Status</TableHead>
          <TableHead className="text-right">Balance</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        <TableRow>
          <TableCell className="font-medium">Everyday Checking</TableCell>
          <TableCell className="text-muted-foreground">Depository</TableCell>
          <TableCell>
            <Badge variant="gain">Synced</Badge>
          </TableCell>
          <TableCell
            className="text-right tabular-nums"
            style={{ color: "var(--gain)" }}
          >
            +$5,300.00
          </TableCell>
        </TableRow>
        <TableRow>
          <TableCell className="font-medium">High-Yield Savings</TableCell>
          <TableCell className="text-muted-foreground">Depository</TableCell>
          <TableCell>
            <Badge variant="gain">Synced</Badge>
          </TableCell>
          <TableCell
            className="text-right tabular-nums"
            style={{ color: "var(--gain)" }}
          >
            +$18,240.00
          </TableCell>
        </TableRow>
        <TableRow>
          <TableCell className="font-medium">Sapphire Credit Card</TableCell>
          <TableCell className="text-muted-foreground">Credit</TableCell>
          <TableCell>
            <Badge variant="warning">Pending</Badge>
          </TableCell>
          <TableCell
            className="text-right tabular-nums"
            style={{ color: "var(--loss)" }}
          >
            −$1,642.19
          </TableCell>
        </TableRow>
        <TableRow>
          <TableCell className="font-medium">Auto Loan</TableCell>
          <TableCell className="text-muted-foreground">Loan</TableCell>
          <TableCell>
            <Badge variant="loss">Overdue</Badge>
          </TableCell>
          <TableCell
            className="text-right tabular-nums"
            style={{ color: "var(--loss)" }}
          >
            −$12,880.00
          </TableCell>
        </TableRow>
      </TableBody>
    </Table>
  );
}
