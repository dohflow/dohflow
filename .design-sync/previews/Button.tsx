import { Button } from "@personal-cfo/desktop";
import { Download, Plus, Trash2 } from "lucide-react";

export function Variants() {
  return (
    <div className="flex flex-wrap items-center gap-3">
      <Button>Add account</Button>
      <Button variant="secondary">Secondary</Button>
      <Button variant="outline">Outline</Button>
      <Button variant="ghost">Ghost</Button>
      <Button variant="destructive">Delete</Button>
      <Button variant="link">Learn more</Button>
    </div>
  );
}

export function Sizes() {
  return (
    <div className="flex flex-wrap items-center gap-3">
      <Button size="sm">Small</Button>
      <Button size="default">Default</Button>
      <Button size="lg">Large</Button>
      <Button size="icon" aria-label="Add account">
        <Plus />
      </Button>
    </div>
  );
}

export function IconsAndStates() {
  return (
    <div className="flex flex-wrap items-center gap-3">
      <Button>
        <Plus /> New transaction
      </Button>
      <Button variant="outline">
        <Download /> Export
      </Button>
      <Button variant="destructive">
        <Trash2 /> Remove
      </Button>
      <Button disabled>Disabled</Button>
    </div>
  );
}
