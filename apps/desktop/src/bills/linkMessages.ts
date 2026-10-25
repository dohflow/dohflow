/// The approved messages about a bill-to-loan link that is not doing its job (personal-cfo-6wk.22,
/// planning). One place, so the bill detail and the Bills / Cash Flow notices cannot drift.
export function cadenceChangedMessage(loanName: string): string {
  return `This bill no longer repeats like ${loanName}'s payment, so both now count.`;
}

export function belowScheduledMessage(loanName: string): string {
  return `This linked bill is less than ${loanName}'s scheduled payment.`;
}
