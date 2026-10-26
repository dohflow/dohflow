/// The approved messages about a bill-to-loan link that is not doing its job (personal-cfo-6wk.22,
/// planning). One place, so the bill detail and the Bills / Cash Flow notices cannot drift.
export function cadenceChangedMessage(loanName: string): string {
  return `This bill no longer repeats like ${loanName}'s payment, so both now count.`;
}

export function belowScheduledMessage(loanName: string): string {
  return `This linked bill is less than ${loanName}'s scheduled payment.`;
}

/// Shown when a bill was added but the link the user asked for did not work (personal-cfo-6wk.22
/// item 9, planning-approved). The bill exists; the warning still offers "Link it".
export function linkFailedMessage(loanName: string): string {
  return `The bill was added, but linking it to ${loanName} didn't work. Use "Link it" on the warning to try again.`;
}

/// Shown when a bill was added but the answer "this isn't that loan's payment" could not be saved.
export const ANSWER_NOT_SAVED_MESSAGE =
  "The bill was added, but your answer wasn't saved, so DohFlow may ask again.";
