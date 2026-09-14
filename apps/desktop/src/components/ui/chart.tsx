import * as React from "react";
import { ResponsiveContainer } from "recharts";

import { formatMoney } from "@/lib/format";
import { cn } from "@/lib/utils";

/// shadcn-style `chart` primitive (ADR 0031 / personal-cfo-4d8.6), themed to our
/// full-hex chart tokens. `ChartContainer` exposes each series colour as a
/// `--color-<key>` CSS variable on its container (set via the `style` prop — CSP
/// allows inline styles) so Recharts children reference `var(--color-<key>)`.
/// `ChartTooltipContent` renders an on-brand hover card. Recipe: `recipes.md` §1.
export type ChartConfig = Record<string, { label: string; color?: string }>;

const ChartConfigContext = React.createContext<ChartConfig | null>(null);

function useChartConfig(): ChartConfig {
  const ctx = React.useContext(ChartConfigContext);
  if (!ctx) {
    throw new Error("useChartConfig must be used within a <ChartContainer>");
  }
  return ctx;
}

/// Wrap a single Recharts chart element. Set an explicit height on `className`
/// (e.g. `h-72`) — `ResponsiveContainer` fills the container.
export function ChartContainer({
  config,
  className,
  children,
}: {
  config: ChartConfig;
  className?: string;
  children: React.ReactElement;
}) {
  const styleVars: Record<string, string> = {};
  for (const [key, conf] of Object.entries(config)) {
    if (conf.color) styleVars[`--color-${key}`] = conf.color;
  }
  return (
    <ChartConfigContext.Provider value={config}>
      <div
        data-slot="chart"
        className={cn(
          "w-full text-xs [&_.recharts-cartesian-axis-tick_text]:fill-muted-foreground [&_.recharts-surface]:outline-none",
          className,
        )}
        style={styleVars as React.CSSProperties}
      >
        <ResponsiveContainer width="100%" height="100%">
          {children}
        </ResponsiveContainer>
      </div>
    </ChartConfigContext.Provider>
  );
}

export { Tooltip as ChartTooltip } from "recharts";

/// One series row Recharts passes to the tooltip `content`.
type TooltipEntry = {
  name?: string;
  dataKey?: string | number;
  value?: number;
  color?: string;
};

/// The hover card: the (formatted) label plus one row per series — colour swatch,
/// label from the config, money value. Recharts injects `active`/`payload`/`label`
/// at render time, so all props are optional.
export function ChartTooltipContent({
  active,
  payload,
  label,
  currency = "USD",
  labelFormatter,
}: {
  active?: boolean;
  payload?: TooltipEntry[];
  label?: string;
  currency?: string;
  labelFormatter?: (label: string) => string;
}) {
  const config = useChartConfig();
  if (!active || !payload?.length) return null;
  return (
    <div className="rounded-lg border bg-popover px-3 py-2 text-xs shadow-md">
      {label !== undefined && (
        <div className="mb-1 font-medium text-popover-foreground">
          {labelFormatter ? labelFormatter(label) : label}
        </div>
      )}
      <div className="flex flex-col gap-1">
        {payload.map((entry, i) => {
          const key = String(entry.dataKey ?? entry.name ?? i);
          const conf = config[key];
          return (
            <div key={key} className="flex items-center justify-between gap-3">
              <span className="flex items-center gap-1.5 text-muted-foreground">
                <span
                  aria-hidden
                  className="inline-block size-2 rounded-full"
                  style={{ backgroundColor: entry.color ?? `var(--color-${key})` }}
                />
                {conf?.label ?? entry.name ?? key}
              </span>
              <span className="font-medium tabular-nums text-popover-foreground">
                {entry.value === undefined
                  ? "—"
                  : formatMoney({ minor_units: entry.value, currency })}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
