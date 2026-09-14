import type { AccountViewDto } from "@/bindings";

/// Whether archiving this account needs to say what happens to the forecast first
/// (personal-cfo-tiqf).
///
/// ADR 0056 made archived **liquid** accounts leave the forecast, which is the safe
/// direction of error — counting a retired account overstates cash, and overstating cash
/// is what produces an overdraft. The accepted cost is that archiving an account which
/// still holds money now *understates* cash, and doing that silently is what this guards:
/// a household that archives a savings account and watches its projection drop with no
/// explanation will reasonably conclude the forecast is broken.
///
/// Scoped to `liquid_cash` deliberately. Those are the only accounts whose balance anchors
/// the projection, so they are the only ones where archiving moves the number. Prompting on
/// a card or a loan would be friction that explains nothing.
///
/// A zero balance changes no projection, so it archives with no extra friction.
export function archiveMovesTheForecast(account: AccountViewDto): boolean {
  return (
    account.cashflow_role === "liquid_cash" && account.balance.minor_units !== 0
  );
}
