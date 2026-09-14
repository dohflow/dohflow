/// Band folding for the per-debt paydown chart (personal-cfo-hnba, ADR 0054).
///
/// Lives in its own module so the sum invariant can be unit-tested directly — asserting
/// it through rendered Recharts paths would test the label, not the arithmetic.

/// One plotted band: a single debt, or the folded tail.
export type Band = { label: string; owed: number[] };

export type PerDebt = { label: string; monthly_owed_minor: number[] };

/// Fold a per-debt series list down to at most `slots` bands.
///
/// Categorical colours are assigned in order and never cycled (ADR 0054), so a debt past
/// the slot count cannot get its own band. It is FOLDED rather than dropped because this
/// chart's promise is that the bands sum to the total owed — dropping the tail would
/// quietly understate the debt, and on a debt chart that is the one direction the error
/// must never go.
///
/// The largest debts by starting balance keep their own band: those are the ones a reader
/// tracks individually, and the long tail of small balances is what reads as "the rest".
export function foldDebts(debts: PerDebt[], slots: number): Band[] {
  const asBand = (d: PerDebt): Band => ({ label: d.label, owed: d.monthly_owed_minor });
  if (slots < 1) return [];
  if (debts.length <= slots) return debts.map(asBand);
  const ranked = [...debts].sort(
    (a, b) => (b.monthly_owed_minor[0] ?? 0) - (a.monthly_owed_minor[0] ?? 0),
  );
  const kept = ranked.slice(0, slots - 1);
  const tail = ranked.slice(slots - 1);
  const span = Math.max(0, ...debts.map((d) => d.monthly_owed_minor.length));
  return [
    ...kept.map(asBand),
    {
      label: `${tail.length} smaller debts`,
      owed: Array.from({ length: span }, (_, month) =>
        tail.reduce((sum, d) => sum + (d.monthly_owed_minor[month] ?? 0), 0),
      ),
    },
  ];
}
