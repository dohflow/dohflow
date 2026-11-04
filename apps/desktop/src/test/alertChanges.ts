/// Counts the DOM changes an alert element sees, to prove that a screen reader would be told again
/// (a live region announces a change, and an alert kept on screen through a retry that fails the same
/// way would otherwise change nothing; personal-cfo-4d8.39.2). `stop()` returns the changes recorded.
export function watchAlert(alert: HTMLElement): { stop: () => number } {
  let seen = 0;
  const observer = new MutationObserver((records) => {
    seen += records.length;
  });
  observer.observe(alert, { childList: true, characterData: true, subtree: true });
  return {
    stop: () => {
      seen += observer.takeRecords().length;
      observer.disconnect();
      return seen;
    },
  };
}
