import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@personal-cfo/desktop";

export function AccountCard() {
  return (
    <Card style={{ width: 320 }}>
      <CardHeader>
        <CardTitle>Checking</CardTitle>
        <CardDescription>Everyday spending account</CardDescription>
      </CardHeader>
      <CardContent>
        <p className="text-2xl font-semibold tabular-nums">$5,300.00</p>
        <p className="text-sm text-muted-foreground">Available balance</p>
      </CardContent>
      <CardFooter className="justify-between">
        <Badge variant="gain">+2.4% this month</Badge>
        <Button variant="outline" size="sm">
          Details
        </Button>
      </CardFooter>
    </Card>
  );
}

export function SummaryCard() {
  return (
    <Card style={{ width: 320 }}>
      <CardHeader>
        <CardTitle>June cash flow</CardTitle>
        <CardDescription>Income minus spending</CardDescription>
      </CardHeader>
      <CardContent className="flex items-center justify-between">
        <span className="text-sm text-muted-foreground">Net</span>
        <span className="text-2xl font-semibold tabular-nums text-gain">
          +$1,240.00
        </span>
      </CardContent>
    </Card>
  );
}
