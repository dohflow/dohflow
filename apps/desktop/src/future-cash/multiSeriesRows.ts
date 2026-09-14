import type { AccountHistoryDto, AccountSeriesDto, GroupSeriesDto } from "@/bindings";

/// One chart row: `date` plus, per plotted series `k`, the forward `k` (P50 minor
/// units), the realized `hist_k`, the forward band `band_k` = `[p10, p90]`, and the
/// `spendableWash` value that keeps the hero gradient continuous across both halves.
export type MultiSeriesRow = { date: string } & Record<
  string,
  number | string | [number, number] | null | undefined
>;

export type AssembledRows = {
  rows: MultiSeriesRow[];
  anyNegative: boolean;
  /// Realized days strictly BEFORE the forecast's first day.
  historyDayCount: number;
  forwardDayCount: number;
  /// Tiers whose forward band actually has width somewhere (worth an Area).
  bandedTiers: string[];
};

/// A plotted individual-account series (already filtered + keyed by the chart).
export type PlottedAccount = {
  key: string;
  account_id: string;
  days: AccountSeriesDto["days"];
};

/// The tiers whose realized history is ever nonzero — so a tier that is flat-zero
/// FORWARD (e.g. reserve just drained to checking) still earns its line + legend chip
/// for the past it actually had (adversarial review of 4d8.27.5.3). Cards excluded.
export function historyTiersWithData(
  history: AccountHistoryDto[] | null | undefined,
): Set<string> {
  const out = new Set<string>();
  for (const account of history ?? []) {
    if (account.tier === "card") continue;
    if (account.days.some((d) => d.closing.minor_units !== 0)) {
      out.add(account.tier === "reserve" ? "reserve" : "spendable");
      out.add("net");
    }
  }
  return out;
}

/// Assemble the date-keyed union of realized history and the forward projection
/// (personal-cfo-4d8.27.5.3). History tiers are rolled up client-side from the
/// per-account realized series: an account contributes from its own honest start
/// (before it, it did not exist, so it adds nothing — 0); `card` series are excluded
/// (this is the LIQUID chart; a card's owed history lives on its Account Detail
/// view). `net` = spendable + reserve, matching the forward rollup. The today row
/// carries BOTH halves so the realized and projected lines meet.
export function assembleMultiSeriesRows(args: {
  groups: GroupSeriesDto[];
  tiers: string[];
  plottedAccounts: PlottedAccount[];
  history: AccountHistoryDto[] | null | undefined;
  todayIso: string | undefined;
}): AssembledRows {
  const { groups, tiers, plottedAccounts, history, todayIso } = args;
  const spine = groups.find((g) => g.tier === "net");
  const rows = new Map<string, MultiSeriesRow>();
  let anyNegative = false;
  const seen = (v: number) => {
    if (v < 0) anyNegative = true;
    return v;
  };

  // --- Forward rows (positional over the net spine — every forward series shares
  //     the horizon's daily dates). ---
  const bandWidth = new Map<string, boolean>();
  for (const [i, c] of (spine?.closings ?? []).entries()) {
    const row: MultiSeriesRow = { date: c.date };
    for (const tier of tiers) {
      const closing = groups.find((g) => g.tier === tier)?.closings[i]?.closing;
      if (!closing) continue;
      row[tier] = seen(closing.p50.minor_units);
      row[`band_${tier}`] = [closing.p10.minor_units, closing.p90.minor_units];
      // The cone's low edge extends the Y domain, so a band-only dip below zero
      // must still ground the zero line (adversarial review of 4d8.27.5.3).
      if (closing.p10.minor_units < 0) anyNegative = true;
      if (closing.p10.minor_units !== closing.p90.minor_units)
        bandWidth.set(tier, true);
    }
    for (const acc of plottedAccounts) {
      const value = acc.days[i]?.closing.p50.minor_units;
      if (value !== undefined) row[acc.key] = seen(value);
    }
    if (typeof row.spendable === "number") row.spendableWash = row.spendable;
    rows.set(c.date, row);
  }

  // --- Realized history (dates ≤ today; the backend already clamps each account to
  //     its earliest real data). ---
  let historyDayCount = 0;
  const liquidHistory = (history ?? []).filter((a) => a.tier !== "card");
  const byDateTier = new Map<string, { spendable: number; reserve: number }>();
  for (const account of liquidHistory) {
    for (const day of account.days) {
      if (todayIso !== undefined && day.date > todayIso) continue;
      const bucket = byDateTier.get(day.date) ?? { spendable: 0, reserve: 0 };
      if (account.tier === "reserve") bucket.reserve += day.closing.minor_units;
      else bucket.spendable += day.closing.minor_units;
      byDateTier.set(day.date, bucket);
    }
  }
  const accountById = new Map(plottedAccounts.map((a) => [a.account_id, a.key]));
  for (const [date, bucket] of byDateTier) {
    const row = rows.get(date) ?? { date };
    const tierValue: Record<string, number> = {
      spendable: bucket.spendable,
      reserve: bucket.reserve,
      net: bucket.spendable + bucket.reserve,
    };
    for (const tier of tiers) {
      // Unallocated is a synthetic forward-only bucket — drawing a solid "realized 0"
      // line for it would present fabricated history (adversarial review).
      if (tier === "unallocated") continue;
      row[`hist_${tier}`] = seen(tierValue[tier] ?? 0);
    }
    // The band opens from ZERO WIDTH at the seam (personal-cfo-7c7a). Today's balance is
    // the one number the projection is ANCHORED to — a band already wide there would say
    // the anchor itself is in doubt. The engine's day-0 row legitimately carries a day of
    // spend uncertainty (it is END of today), so the zero-width point is the realized
    // close, and the range opens from it on the following day.
    if (todayIso !== undefined && date === todayIso) {
      for (const tier of tiers) {
        if (tier === "unallocated") continue;
        const realized = tierValue[tier];
        if (realized !== undefined) row[`band_${tier}`] = [realized, realized];
      }
    }
    if (typeof row[`hist_spendable`] === "number")
      row.spendableWash = row[`hist_spendable`] as number;
    rows.set(date, row);
    if (todayIso === undefined || date < todayIso) historyDayCount++;
  }
  for (const account of liquidHistory) {
    const key = accountById.get(account.account_id);
    if (!key) continue;
    for (const day of account.days) {
      if (todayIso !== undefined && day.date > todayIso) continue;
      const row = rows.get(day.date);
      if (row) row[`hist_${key}`] = seen(day.closing.minor_units);
    }
  }

  const sorted = [...rows.values()].sort((a, b) =>
    a.date.localeCompare(b.date),
  );
  return {
    rows: sorted,
    anyNegative,
    historyDayCount,
    forwardDayCount: spine?.closings.length ?? 0,
    bandedTiers: tiers.filter((t) => bandWidth.get(t)),
  };
}
