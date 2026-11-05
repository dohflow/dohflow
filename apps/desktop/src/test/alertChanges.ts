/// Counts the DOM changes an alert element sees, to prove that a screen reader would be told again
/// (a live region announces a change, and an alert kept on screen through a retry that fails the same
/// way would otherwise change nothing; personal-cfo-4d8.39.2). `count()` reads the changes so far and
/// keeps watching (use it inside `waitFor`); `stop()` reads them and stops.
export function watchAlert(alert: HTMLElement): { count: () => number; stop: () => number } {
  let seen = 0;
  const observer = new MutationObserver((records) => {
    seen += records.length;
  });
  observer.observe(alert, { childList: true, characterData: true, subtree: true });
  const count = () => {
    seen += observer.takeRecords().length;
    return seen;
  };
  return {
    count,
    stop: () => {
      const total = count();
      observer.disconnect();
      return total;
    },
  };
}
