import { SUBTYPE_LABELS } from "@/accounts/subtypes";
import type { AccountSeriesDto } from "@/bindings";

/// A selection key for an individual account series in the Future Cash chart
/// (personal-cfo-4d8.25.26). Kept out of the component file so it can be shared by
/// the chart, the picker, and the settings hook without a fast-refresh warning.
export const accountSeriesKey = (accountId: string) => `acct:${accountId}`;

/// A display label for one account that is UNIQUE within `all` (adversarial review of
/// 4d8.25.26): account names are free-text and two accounts can share one (e.g. two
/// "Savings"). When a name repeats, disambiguate by subtype if that separates them,
/// else by a 1-based ordinal — so the legend + picker rows are never indistinguishable.
/// The minimum an account needs for a unique label. Structural rather than tied to one
/// DTO so the Debt selector (`AccountViewDto`, personal-cfo-4d8.27.9.3) and the Future
/// Cash picker (`AccountSeriesDto`) share ONE implementation — two copies of a
/// disambiguation rule drift, and then the same two accounts read differently on two
/// screens.
export type LabelledAccount = {
  id: string | null;
  name: string;
  subtype: string | null;
};

export function accountLabel(
  account: LabelledAccount,
  all: LabelledAccount[],
): string {
  const sameName = all.filter((a) => a.name === account.name);
  if (sameName.length <= 1) return account.name;
  const subtypesDistinct =
    new Set(sameName.map((a) => a.subtype)).size === sameName.length;
  if (subtypesDistinct && account.subtype) {
    return `${account.name} · ${SUBTYPE_LABELS[account.subtype] ?? account.subtype}`;
  }
  const ordinal = sameName.findIndex((a) => a.id === account.id) + 1;
  return `${account.name} (${ordinal})`;
}

/// Adapt an `AccountSeriesDto` (which names its id `account_id`) to [`LabelledAccount`].
export const seriesAsLabelled = (a: AccountSeriesDto): LabelledAccount => ({
  id: a.account_id,
  name: a.name,
  subtype: a.subtype,
});
