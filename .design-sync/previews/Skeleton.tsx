import { Skeleton } from "@personal-cfo/desktop";

export function LoadingCard() {
  return (
    <div style={{ width: 360 }} className="border rounded-lg p-6">
      <div className="flex flex-col gap-2">
        <Skeleton style={{ height: 20, width: 180 }} />
        <Skeleton style={{ height: 12, width: 260 }} />
        <Skeleton style={{ height: 12, width: 220 }} />
        <Skeleton style={{ height: 12, width: 140 }} />
      </div>
    </div>
  );
}

export function ListRows() {
  return (
    <div style={{ width: 360 }} className="border rounded-lg p-6">
      <div className="flex flex-col gap-4">
        {[220, 180, 200].map((w) => (
          <div key={w} className="flex items-center gap-3">
            <Skeleton className="size-10" style={{ borderRadius: 9999 }} />
            <div className="flex flex-col gap-2">
              <Skeleton style={{ height: 12, width: w }} />
              <Skeleton style={{ height: 10, width: w - 80 }} />
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
