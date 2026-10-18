// Why a provider account can't be mapped (personal-cfo-049p6; ADR 0076
// decision 7). The message is the Rust currency guard's own text, carried on
// the link DTO, so the UI says exactly what the kernel enforces. The link
// opens the help page that states the limitation, in the system browser
// (the only opener grant, https://dohflow.app/*).

import { openExternal, DOHFLOW_LINKS } from "@/lib/openExternal";

export function CurrencyRefusal({
  message,
  id,
  className,
}: {
  message: string;
  /// For `aria-describedby` from the control it explains.
  id?: string;
  className?: string;
}) {
  return (
    <p id={id} className={className ?? "text-xs text-warning"}>
      {message}{" "}
      <button
        type="button"
        className="underline underline-offset-2 hover:text-foreground"
        onClick={() => void openExternal(DOHFLOW_LINKS.currencyLimitation)}
      >
        About currencies
      </button>
    </p>
  );
}
