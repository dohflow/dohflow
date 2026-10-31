# Evidence for personal-cfo-4d8.38 (shared disclosure)

Fake data only. Rendered from the dev-only page `apps/desktop/disclosure-example.html`
(`src/dev/disclosure-example.tsx`), which is not in the production bundle (the build input is
`index.html` alone). Engine: Chromium in the Claude desktop browser pane, not WKWebView.

| File | State |
|------|-------|
| `light-open.jpg`, `dark-open.jpg` | Long title wrapping; the error and its one Refresh now outside the body; Backups collapsed with its summary; the nested delete section open |
| `light-pinned.jpg`, `dark-pinned.jpg` | A delete confirmation open: both ancestor triggers are `aria-disabled` and stay open; Cancel is offered |

## Measured geometry (same page, `getComputedStyle`)

Header 44px minimum (68px for the two-line title), chevron lane 16px, gap 12px, title 16px/24px
semibold, card radius 8px. Matches the A2 numbers in `settings-layout-review.md`.

## Runtime checks in the real browser (programmatic, not VoiceOver)

- Collapsing with focus inside the body leaves `document.activeElement` on the trigger.
- The collapsed body is `hidden` and `inert`; the Tab list excludes it.
- The error and exactly one Refresh now stay visible and reachable while collapsed.
- While a delete confirmation is open both ancestor triggers report `aria-disabled="true"`, keep
  `tabindex` 0 and ignore a click; after Cancel both are enabled and still open, and focus is on
  the start button (which is unmounted while confirming; focus is set from a layout effect after
  the re-render, found by 04b's review).

04b's automated accessibility-tree check on the built page is the runtime gate for this bead; the
owner's VoiceOver run is part of `personal-cfo-4d8.39`. No production-AA claim is made: the dark
card edge in these screenshots is the known `--border` gap that `personal-cfo-4d8.35` owns.
