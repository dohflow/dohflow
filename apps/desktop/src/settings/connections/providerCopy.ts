// Plain-language lines the provider picker shows for one registry entry
// (personal-cfo-dto2j): what it fetches, what it costs and to whom, and where
// it works. Descriptive only (ADR 0018) — it states facts, it never recommends.

import type {
  ConnectorAdapterDto,
  ConnectorBillingPeriodDto,
} from "@/bindings";
import { formatMoney } from "@/lib/format";

/// The capabilities a picker chip names, in a fixed order.
export function capabilityChips(adapter: ConnectorAdapterDto): string[] {
  const { accounts, transactions, balances, holdings } = adapter.capabilities;
  return [
    accounts ? "Accounts" : null,
    transactions ? "Transactions" : null,
    balances ? "Balances" : null,
    holdings ? "Holdings" : null,
  ].filter((chip): chip is string => chip !== null);
}

function per(period: ConnectorBillingPeriodDto | null): string {
  return period === "Monthly" ? " a month" : period === "Annual" ? " a year" : "";
}

function money(minorUnits: number, currency: string): string {
  return formatMoney({ minor_units: minorUnits, currency });
}

/// Who pays, how much, and what it includes — e.g. "You pay SimpleFIN
/// directly: $15.00 a year, up to 25 connections included."
export function costLine(adapter: ConnectorAdapterDto): string {
  const economics = adapter.economics;
  if (economics.payer === "None") {
    return `${adapter.display_name} is free to use.`;
  }
  const payer =
    economics.payer === "UserDirect"
      ? `You pay ${adapter.display_name} directly`
      : "Billed through DohFlow";
  const currency = economics.currency;
  if (economics.base_cost_minor_units === null || currency === null) {
    return `${payer}.`;
  }
  let line = `${payer}: ${money(economics.base_cost_minor_units, currency)}${per(economics.billing_period)}`;
  if (economics.included_connections !== null) {
    line += `, up to ${economics.included_connections} connections included`;
  }
  if (economics.extra_connection_cost_minor_units !== null) {
    line += `, then ${money(economics.extra_connection_cost_minor_units, currency)}${per(economics.extra_connection_period)} for each extra connection`;
  }
  return `${line}.`;
}

/// Country names for the registry's ISO 3166-1 codes, falling back to the
/// code where the runtime has no name for it.
export function countryNames(adapter: ConnectorAdapterDto): string {
  let names: Intl.DisplayNames | null = null;
  try {
    names = new Intl.DisplayNames(undefined, { type: "region" });
  } catch {
    names = null;
  }
  return adapter.regions
    .map((code) => names?.of(code) ?? code)
    .join(", ");
}

/// The first sentence of a disclosure point, set in bold as a lead-in, and
/// the rest — `lead` is null when the point is a single sentence.
export function splitLead(text: string): { lead: string | null; rest: string } {
  const end = text.indexOf(". ");
  if (end === -1) return { lead: null, rest: text };
  return { lead: text.slice(0, end + 1), rest: text.slice(end + 2) };
}
