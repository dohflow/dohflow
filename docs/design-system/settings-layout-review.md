# Settings design review

Status: **Alternatives v3 frozen; independent review pending.**

Freeze: **October 27, 2026, 8:36:39 AM Pacific**
(**2026-10-27 15:36:39 UTC**).
[V3 narrative/specification](https://app.paper.design/file/01M5VA9BZKEQPM8F7S5DHHWC84/p-7-0),
[A](https://app.paper.design/file/01M5VA9BZKEQPM8F7S5DHHWC84/p-8-0),
[B](https://app.paper.design/file/01M5VA9BZKEQPM8F7S5DHHWC84/p-9-0),
[C](https://app.paper.design/file/01M5VA9BZKEQPM8F7S5DHHWC84/p-A-0).

The owner’s layout corrections apply to A, B and C. The owner reviewed A only;
this does not select A or approve the proposed headings and summaries. The v2
Paper pages, their independent review and the RC2 record below are preserved.

## v3 layout craft contract

Reuse the independently reviewed Paper-only **LCR-1 v1**, frozen
**2026-10-27 15:21:48 UTC**, exact frames
`DS-LCR-1 · Layout craft · light · candidate` and
`DS-LCR-1 · Layout craft · dark · candidate` on
[the shared system page](https://app.paper.design/file/01M5TMWNYW8618DTPFRYTQ0CGN/p-8-0).
Its PASS covers that Paper contract; it does not approve these alternatives or
claim that the frontend convention has shipped. The Lead validated the wording
below before the candidate freeze.

- Collapsed siblings in each actual row stretch to the tallest natural content.
  Keep full titles and summaries. Do not use fixed short heights, clipping or
  equal heights across unrelated rows. Expanded editors retain natural height.
  An exposed error grows naturally; a healthy collapsed sibling may grow with
  it, but must never constrain its message or retry.
- Align control top edges in a shared track while retaining each source
  control’s specified size. Labels, help and the containing row grow naturally
  without clipping; stack when lanes cannot fit. Help follows the controls.
  Do not put independent descriptions above parallel controls or insert
  arbitrary spacers. Each feature retains its DOM and focus grouping.
- At two columns, healthy three-item groups use two half-width siblings then
  the named full-row last item. Expanded and mixed states use the explicit
  spans below. Do not strand a half-width final tile or use dense packing,
  masonry, CSS order or visual/focus-order divergence.

`C` is usable content width after sidebar and 24px insets. Minimum closed-tile
width is 320px with 16px gaps: three columns at C992, two at C656, one below.
Preferences use three lanes when their card is at least 866px, two at 586px,
otherwise one; at two lanes Appearance spans the next row. The label/control
tracks retain source 36px and 40px controls, with the row growing for full help.

| Group/state | Three-column fit | Two-column fit | One column |
| --- | --- | --- | --- |
| A Backups / Comfort / Auto, healthy | 1 / 1 / 1 | half / half / full | all full |
| A Comfort open | 1 / 2 / full | all full | all full |
| A Comfort closed with partial error | half / half / full | half / half / full | all full |
| B recovery / preferences | 2 / 1 | full / full | full / full |
| B Forecast/imports / Troubleshooting / Setup, healthy | 1 / 1 / 1 | half / half / full | all full |
| B groups with open editors or problems | all full | all full | all full |
| C preferences / Backups / Comfort, healthy | 1 / 1 / 1 | half / half / full | all full |
| C preferences open | full / half / half | full / half / half | all full |
| C Comfort open or partial error | half / half / full | half / half / full | all full |
| C Auto / Vaults / Setup, healthy | 1 / 1 / 1 | half / half / full | all full |
| C Auto open | full / half / half | full / half / half | all full |
| A/C Health / Update / Diagnostics, healthy | 1 / 1 / 1 | half / half / full | all full |
| A/C Health problem | full / half / half | full / half / half | all full |
| A Vaults / Setup | half / half | half / half | all full |

Connections, About and the destructive parent remain full width. Spans retain
source order. All v2 outside retries, retained drafts, complete partial-save
alerts, busy states and A2 destructive/focus guards continue to apply. No new
product copy, token or component is introduced by these corrections.

## v3 evidence and verification

| Named frames | Evidence |
| --- | --- |
| A/B/C01–06,14–15 · v3 | Default Light/Dark, C1008/C1176 wide and C696/C528 sidebar fit, C368 stress |
| A/B/C10–13 · v3 | Open/mixed editors, full errors/remedies, long amounts, busy and two distinct partial-save results |
| A/B/C16–19 · v3 | Closed Comfort partial failure, natural row growth, persistent alert and outside retry, both themes/wide/narrow |
| A/B/C30–37 · v3 | Light/Dark at C992, C991, C656, C655; exact three/two/one-column boundaries; wrapped title and missing summary |
| A/B/C40–41 · v3 | Light/Dark C696 mixed error/partial layouts with explicit spans |
| C20–23 · v3 | Open preferences, Light/Dark C1008, Light C696, Dark C368 |
| B42–43 · v3 | Light/Dark C368, long synthetic destination and last-backup paths wrap in full |
| X01–16, unchanged v2 | Empty/loading, 24 connections and long names, closed failures/retries/busy, mapping/Forget and destructive guards |

The original 66 v2 root exports remain byte-identical. All 48 copied A/B/C
layouts retain the same full text-node inventory as v2; only layout and the
specification annotations changed. The two added long-path fixtures are fake.

All 102 candidate root exports were checked for oversized fixed-width
descendants and Light token references in Dark frames; none were found.
All 30 open/closed/mixed Comfort partial-save specimens retain the complete
visible alert, both draft values and exactly one visible Save in its scope.
All 129 token names/values still match the shared baseline.

Screenshots cover the corrected wide/narrow defaults, closed failures,
preferences, all new transition/mixed specimens and the long-path pair.
Exported CSS demonstrates row stretching and label/control tracks; inactive-page
Paper node geometry is unavailable, so no fabricated measured-height table is
claimed. These are static design checks, not runtime resize or accessibility
tests. Source input-boundary contrast remains the recorded production gate.
No application code or dependency changed; application test/build suites were
not run. Documentation passes `git diff --check`.

Independent review must bind the new freeze, exact manifest below and the new
PR commit. The historical v2 verdict does not apply to v3. Any Paper or tracked
specification edit requires a new review. Owner option selection and proposed
copy approval remain pending; no owner comment is resolved merely by intake.

## Exact alternatives v3 manifest

This freeze includes 86 new v3 roots and 16 unchanged shared v2 roots.
Original v2 narrative/A/B/C pages and the workbench are excluded from the v3
candidate; their historical manifest is preserved below.

| Page | Frame |
| --- | --- |
| Shared states · Connections and safety | X-01 · Connections · 24 compact rows · Light C1008 |
| Shared states · Connections and safety | X-02 · Connections · mapping and open Forget · Light C1008 |
| Shared states · Connections and safety | X-03 · Destructive guard · armed attempted-collapse Cancel busy · Light C1008 |
| Shared states · Connections and safety | X-04 · Connections · loading empty error and closed alerts · Light C1008 |
| Shared states · Connections and safety | X-05 · Connections · 24 compact rows · Dark C1008 |
| Shared states · Connections and safety | X-06 · Connections · 24 compact rows · Light C368 · stress |
| Shared states · Connections and safety | X-07 · Connections · 24 compact rows · Dark C368 · stress |
| Shared states · Connections and safety | X-08 · Connections · mapping and open Forget · Dark C1008 |
| Shared states · Connections and safety | X-09 · Connections · mapping and open Forget · Light C368 · stress |
| Shared states · Connections and safety | X-10 · Connections · mapping and open Forget · Dark C368 · stress |
| Shared states · Connections and safety | X-11 · Destructive guard · armed attempted-collapse Cancel busy · Dark C1008 |
| Shared states · Connections and safety | X-12 · Destructive guard · armed attempted-collapse Cancel busy · Light C368 · stress |
| Shared states · Connections and safety | X-13 · Destructive guard · armed attempted-collapse Cancel busy · Dark C368 · stress |
| Shared states · Connections and safety | X-14 · Connections · loading empty error and closed alerts · Dark C1008 |
| Shared states · Connections and safety | X-15 · Connections · loading empty error and closed alerts · Light C368 · stress |
| Shared states · Connections and safety | X-16 · Connections · loading empty error and closed alerts · Dark C368 · stress |
| v3 · A · Landscape grid | A-01 · Default · Light · C1008 · 1280 expanded-sidebar · v3 |
| v3 · A · Landscape grid | A-02 · Default · Dark · C1008 · 1280 expanded-sidebar · v3 |
| v3 · A · Landscape grid | A-03 · Default · Light · C368 · 472 collapsed-sidebar · stress · v3 |
| v3 · A · Landscape grid | A-04 · Default · Dark · C368 · 472 collapsed-sidebar · stress · v3 |
| v3 · A · Landscape grid | A-05 · Default · Light · C696 · 800 collapsed-sidebar · v3 |
| v3 · A · Landscape grid | A-06 · Default · Light · C1176 · 1280 collapsed-sidebar · v3 |
| v3 · A · Landscape grid | A-10 · Full layout · independent open · partial save · problems · Light C1008 · v3 |
| v3 · A · Landscape grid | A-11 · Full layout · long values and problems · Dark C368 · stress · v3 |
| v3 · A · Landscape grid | A-12 · Full layout · long values and problems · Light C368 · stress · v3 |
| v3 · A · Landscape grid | A-13 · Full layout · long values and problems · Dark C1008 · v3 |
| v3 · A · Landscape grid | A-14 · Default · Light C528 · 800 expanded-sidebar · v3 |
| v3 · A · Landscape grid | A-15 · Default · Dark C528 · 800 expanded-sidebar · v3 |
| v3 · A · Landscape grid | A-16 · Closed Comfort partial failure · Light C1008 · v3 |
| v3 · A · Landscape grid | A-17 · Closed Comfort partial failure · Dark C368 · stress · v3 |
| v3 · A · Landscape grid | A-18 · Closed Comfort partial failure · Light C368 · stress · v3 |
| v3 · A · Landscape grid | A-19 · Closed Comfort partial failure · Dark C1008 · v3 |
| v3 · A · Landscape grid | A-30 · LCR-1 transition · Light C992 · v3 |
| v3 · A · Landscape grid | A-31 · LCR-1 transition · Dark C992 · v3 |
| v3 · A · Landscape grid | A-32 · LCR-1 transition · Light C991 · v3 |
| v3 · A · Landscape grid | A-33 · LCR-1 transition · Dark C991 · v3 |
| v3 · A · Landscape grid | A-34 · LCR-1 transition · Light C656 · v3 |
| v3 · A · Landscape grid | A-35 · LCR-1 transition · Dark C656 · v3 |
| v3 · A · Landscape grid | A-36 · LCR-1 transition · Light C655 · v3 |
| v3 · A · Landscape grid | A-37 · LCR-1 transition · Dark C655 · v3 |
| v3 · A · Landscape grid | A-40 · LCR-1 mixed error partial · Light C696 · v3 |
| v3 · A · Landscape grid | A-41 · LCR-1 mixed error partial · Dark C696 · v3 |
| v3 · B · Task groups | B-01 · Default · Light · C1008 · 1280 expanded-sidebar · v3 |
| v3 · B · Task groups | B-02 · Default · Dark · C1008 · 1280 expanded-sidebar · v3 |
| v3 · B · Task groups | B-03 · Default · Light · C368 · 472 collapsed-sidebar · stress · v3 |
| v3 · B · Task groups | B-04 · Default · Dark · C368 · 472 collapsed-sidebar · stress · v3 |
| v3 · B · Task groups | B-05 · Default · Light · C696 · 800 collapsed-sidebar · v3 |
| v3 · B · Task groups | B-06 · Default · Light · C1176 · 1280 collapsed-sidebar · v3 |
| v3 · B · Task groups | B-10 · Full layout · independent open · partial save · problems · Light C1008 · v3 |
| v3 · B · Task groups | B-11 · Full layout · long values and problems · Dark C368 · stress · v3 |
| v3 · B · Task groups | B-12 · Full layout · long values and problems · Light C368 · stress · v3 |
| v3 · B · Task groups | B-13 · Full layout · long values and problems · Dark C1008 · v3 |
| v3 · B · Task groups | B-14 · Default · Light C528 · 800 expanded-sidebar · v3 |
| v3 · B · Task groups | B-15 · Default · Dark C528 · 800 expanded-sidebar · v3 |
| v3 · B · Task groups | B-16 · Closed Comfort partial failure · Light C1008 · v3 |
| v3 · B · Task groups | B-17 · Closed Comfort partial failure · Dark C368 · stress · v3 |
| v3 · B · Task groups | B-18 · Closed Comfort partial failure · Light C368 · stress · v3 |
| v3 · B · Task groups | B-19 · Closed Comfort partial failure · Dark C1008 · v3 |
| v3 · B · Task groups | B-30 · LCR-1 transition · Light C992 · v3 |
| v3 · B · Task groups | B-31 · LCR-1 transition · Dark C992 · v3 |
| v3 · B · Task groups | B-32 · LCR-1 transition · Light C991 · v3 |
| v3 · B · Task groups | B-33 · LCR-1 transition · Dark C991 · v3 |
| v3 · B · Task groups | B-34 · LCR-1 transition · Light C656 · v3 |
| v3 · B · Task groups | B-35 · LCR-1 transition · Dark C656 · v3 |
| v3 · B · Task groups | B-36 · LCR-1 transition · Light C655 · v3 |
| v3 · B · Task groups | B-37 · LCR-1 transition · Dark C655 · v3 |
| v3 · B · Task groups | B-40 · LCR-1 mixed error partial · Light C696 · v3 |
| v3 · B · Task groups | B-41 · LCR-1 mixed error partial · Dark C696 · v3 |
| v3 · B · Task groups | B-42 · Long demo backup path · Light C368 · v3 |
| v3 · B · Task groups | B-43 · Long demo backup path · Dark C368 · v3 |
| v3 · C · Compact overview | C-01 · Default · Light · C1008 · 1280 expanded-sidebar · v3 |
| v3 · C · Compact overview | C-02 · Default · Dark · C1008 · 1280 expanded-sidebar · v3 |
| v3 · C · Compact overview | C-03 · Default · Light · C368 · 472 collapsed-sidebar · stress · v3 |
| v3 · C · Compact overview | C-04 · Default · Dark · C368 · 472 collapsed-sidebar · stress · v3 |
| v3 · C · Compact overview | C-05 · Default · Light · C696 · 800 collapsed-sidebar · v3 |
| v3 · C · Compact overview | C-06 · Default · Light · C1176 · 1280 collapsed-sidebar · v3 |
| v3 · C · Compact overview | C-10 · Full layout · independent open · partial save · problems · Light C1008 · v3 |
| v3 · C · Compact overview | C-11 · Full layout · long values and problems · Dark C368 · stress · v3 |
| v3 · C · Compact overview | C-12 · Full layout · long values and problems · Light C368 · stress · v3 |
| v3 · C · Compact overview | C-13 · Full layout · long values and problems · Dark C1008 · v3 |
| v3 · C · Compact overview | C-14 · Default · Light C528 · 800 expanded-sidebar · v3 |
| v3 · C · Compact overview | C-15 · Default · Dark C528 · 800 expanded-sidebar · v3 |
| v3 · C · Compact overview | C-16 · Closed Comfort partial failure · Light C1008 · v3 |
| v3 · C · Compact overview | C-17 · Closed Comfort partial failure · Dark C368 · stress · v3 |
| v3 · C · Compact overview | C-18 · Closed Comfort partial failure · Light C368 · stress · v3 |
| v3 · C · Compact overview | C-19 · Closed Comfort partial failure · Dark C1008 · v3 |
| v3 · C · Compact overview | C-20 · Preferences open · Light C1008 · v3 |
| v3 · C · Compact overview | C-21 · Preferences open · Dark C1008 · v3 |
| v3 · C · Compact overview | C-22 · Preferences open · Light C696 · v3 |
| v3 · C · Compact overview | C-23 · Preferences open · Dark C368 · v3 |
| v3 · C · Compact overview | C-30 · LCR-1 transition · Light C992 · v3 |
| v3 · C · Compact overview | C-31 · LCR-1 transition · Dark C992 · v3 |
| v3 · C · Compact overview | C-32 · LCR-1 transition · Light C991 · v3 |
| v3 · C · Compact overview | C-33 · LCR-1 transition · Dark C991 · v3 |
| v3 · C · Compact overview | C-34 · LCR-1 transition · Light C656 · v3 |
| v3 · C · Compact overview | C-35 · LCR-1 transition · Dark C656 · v3 |
| v3 · C · Compact overview | C-36 · LCR-1 transition · Light C655 · v3 |
| v3 · C · Compact overview | C-37 · LCR-1 transition · Dark C655 · v3 |
| v3 · C · Compact overview | C-40 · LCR-1 mixed error partial · Light C696 · v3 |
| v3 · C · Compact overview | C-41 · LCR-1 mixed error partial · Dark C696 · v3 |
| v3 · Read first and craft evidence | OPTIONS-00 · Three alternatives · narrative and recommendation · v3 |
| v3 · Read first and craft evidence | OPTIONS-SPEC · Behavior copy components and coverage · v3 |

## Historical alternatives v2 — preserved record

Status: **v2 independently reviewed: HUMAN_DECISION_REQUIRED.**
The verdict binds to commit `234493133be8f3f53ae4e8ded7a84364aa900ac5`
and the freeze below:
[independent v2 review](https://github.com/dohflow/dohflow/pull/187#issuecomment-6410000147).
Both v1 findings were resolved. The subsequent owner corrections are addressed
in v3; this historical verdict does not approve later edits.
This is a design review package, not an implementation handoff or accepted
frontend convention. The earlier RC2 artifact remains unchanged.

- Task: `personal-cfo-twpo.3`.
- [Paper narrative and specification](https://app.paper.design/file/01M5VA9BZKEQPM8F7S5DHHWC84/p-1-0).
- Freeze: **October 25, 2026, 11:41:58 PM Pacific**
  (**2026-10-26 06:41:58 UTC**).
- Source baseline: `e8e6ed00389278bb14af8b88d92097b5f61cda4d`.
- Shared system: v0.2, token hash `0afec932`; copied local hash
  `a378c7e0`. All 129 token names and values match.
- Shared accordion: independently reviewed **A2**, frozen
  **2026-10-26 05:24:16 UTC**, exact
  `DS-A1 · Simple accordion contract · Light · A2` and
  `DS-A2 · Simple accordion contract · Dark · A2`, in the
  [shared system file](https://app.paper.design/file/01M5TMWNYW8618DTPFRYTQ0CGN/p-7-0).
  The superseded A1 verdict does not authorize reuse.
- The owner has approved D3–D6 within their recorded terms. D1/D2 require these
  three new alternatives. New headings and summary compositions below remain
  **PROPOSED**. Unchanged approvals are not being re-asked.

## v1 review and v2 repairs

The independent v1 verdict was **CHANGES_REQUIRED**, bound to freeze
2026-10-26 06:18:19 UTC and commit
`80712dd715a0ab0c38ec2b00c1d25f63564c434e`:
[review findings](https://github.com/dohflow/dohflow/pull/187#issuecomment-6409727965).
That commit preserves the v1 manifest.

- **F-V1-1:** eight failed compact connection02 rows now expose one separate
  Refresh now control outside the hidden details. Its accessible name remains
  “Refresh now, Demo connection 02” and it uses the same source refresh callback.
  Four closed timezone-error layouts now retain the Los Angeles draft and expose
  the source Save in a Timezone group, associated with the draft and error.
- **F-V1-2:** combined-preference partial success is now distinguished from
  Comfort band's persisted-lower/failed-upper result. All twelve open-editor
  layouts show the complete source Comfort alert, both retained drafts and one
  40px source Save in CardFooter, outside the collapsible body. The former body
  Save is suppressed. Twelve closed-parent counterparts cover both themes at
  C1008/C368 without hiding that alert or retry.

Comfort's retry group uses the existing “Comfort band” text and describes Save
with the full partial-save alert. The source Save name, validation, disabled and
pending rules remain; pending uses the approved 16px Loader2 at this same outside
location. Expanding details must not introduce a second active retry control.
Collapse never submits, resets or rolls back either partial result. These repairs
reuse approved source words and existing CardFooter; no new copy or primitive is
introduced. The independent v2 verdict above reviewed these repairs.

## Decision narrative

### A — Landscape grid

[Page A](https://app.paper.design/file/01M5VA9BZKEQPM8F7S5DHHWC84/p-2-0),
starting at `A-01`, responds most directly to the landscape-layout request.
Household, General and Appearance share one initially open composition. Other
healthy settings begin closed and remain easy to find by their existing names.
Connections and About span the full content width. Healthy Vault health occupies
a small summary; a current problem exposes its full remedy and action.

This is the recommended starting point because it balances familiar setting
names with less scrolling. Dense editors gain two column spans rather than
shrinking controls. The trade-off is visible row-height change and some open
space when several independent sections expand. The layout preserves reading
order instead of packing later items into those spaces.

### B — Task groups

[Page B](https://app.paper.design/file/01M5VA9BZKEQPM8F7S5DHHWC84/p-3-0),
starting at `B-01`, organizes maintenance around a task. Backups and Vaults form
a wide, initially open recovery area beside compact, initially open preferences.
Forecast and imports groups Comfort band and auto-categorization; Troubleshooting
contains Vault health, software update and diagnostics. Setup guide remains
conditional. Connections and About stay full width.

The benefit is enough room to finish related maintenance without moving among
many small cards. The cost is a taller initial view, especially when narrow
windows stack the recovery and preference areas. New group names also require
owner approval. This option uses a shared 24px recovery work area around existing
controls; it deliberately recomposes the former separate cards.

### C — Compact overview

[Page C](https://app.paper.design/file/01M5VA9BZKEQPM8F7S5DHHWC84/p-4-0),
starting at `C-01`, begins with healthy sections closed and concise summaries.
Opening a section creates a full-width editor at that point in normal flow.
Sections remain independent; there are no tabs, drawers or single-open rules.

This gives the quietest initial scan and the most space for one editing task.
Routine changes require an extra expansion, and several open editors create a
taller page. Choose it when occasional Settings visits and focused editing matter
more than immediately exposing common preference controls.

`OPTIONS-00` presents the same narrative in Paper. `OPTIONS-SPEC` records the
behavior, component, copy and state index. Reviewer and Decisions desk can use
the exact `A-01`, `B-01`, and `C-01` screenshots for comparison; these are
main-content specimens, not full-window or runtime screenshots.

## Responsive and interaction contract

Let `C` be remaining main content width after the sidebar and 24px padding on
each side. A closed tile needs at least 320px, with 16px gutters: three columns
fit at `C >= 992`, two at `C >= 656`, otherwise one. The photographed
specimens cover C1008, C1176, C696, C528 and C368. C1008/C1176 correspond to a
1280px window with the 224px/56px sidebar; C528/C696 correspond to an 800px
window. **C368 is a desktop stress case, not a supported-minimum claim.**

Preferences use three lanes when wide, two at C696, and one when narrow.
Dense A/B editors take two spans or the full available width; C editors take
a full row. Comfort fields stack below 480px of usable editor body width.
Default group bodies use 24px padding; Delete retains 16px. Source controls
retain 40px/36px/32px heights. A2 retains a minimum 44px trigger, fixed 16px
chevron lane, 12px gap, and 16px semibold title with 24px line height.

Expansion is independent. Reading and focus order stay in source order, without
dense packing, masonry or CSS ordering. Collapsing does not save or discard
ordinary drafts. Hidden panels leave the keyboard and accessibility tree while
their draft state survives. Move focus to the trigger before hiding focused
content. No library or durable expansion-storage choice is prescribed; Engineering
must record the shared implementation approach. No `ui/Accordion` exists at
the pinned source, and no per-screen substitute is authorized.

Current errors, complete actionable remedies and their controls remain outside
hidden bodies. Counts or dots alone are insufficient. Named busy operations,
decorative 16px Loader2 indicators and polite announcements remain visible.
Relevant source actions stay disabled. Save retains its name plus spinner.
Errors remain persistent alerts, with no animation-tick or resize announcements.

All ancestor disclosures remain expanded when a destructive confirmation is open
or armed. Focusable `aria-disabled=true` triggers ignore collapse attempts.
Explicit existing Cancel resets the feature's confirmation/password/error as
applicable, returns action focus, and leaves the section open; only a later
deliberate collapse hides it. Irreversible busy work prevents collapse and Cancel.
Blocking rotation remains whole-app inert and non-dismissible, without Escape or
Cancel. The new parent behavior is illustrated in X02/X03 and their counterparts.

## Coverage, copy and component index

A/B/C01–06 and 14–15 cover default/theme/container fit. A/B/C10–13 retain the
full layout order and distinguish two partial results: combined preferences show
timezone failure with base-currency success; Comfort band shows the complete
“Saved the lower edge, but couldn't update the upper edge: Something went wrong.”
alert with retained lower `123456789012.34` and upper `987654321098.76` drafts.
Its single Save remains outside the hidden editor body.

A/B/C16–19 show that Comfort failure with its enclosing editor/group closed,
paired Light/Dark at C1008/C368. The hidden fields preserve the same drafts as
10–13; reopening restores them. The full alert and one Save remain reachable.
This is source partial persistence, not a whole-form rollback or just combined
preference success.

X01–16 cover 24 synthetic connections, long provider/account labels, compact and
expanded mapping, Forget, loading, empty, closed errors/busy, and destructive
open/armed/attempted-collapse/Cancel/busy states in both themes at C1008/C368.
X01/X02/X05–X10 now include the outside failed-refresh retry. All data, paths and
names are synthetic.

The shared X specimens apply identically to all three options. Unchanged source
controls and state deltas reference the exact historical RC2 freeze
**2026-10-26 01:36:05 UTC**, S01–S16 Light/Dark, F00–F05 and R11/R12. These cover
password-strength buckets, provider disclosure/credential, create-map/shared-feed
dialogs, diagnostics preview, source retry/error/success and explicit N/A cases.
Blocking rotation is **F04**; F05 is the credential step. References retain source
content and behavior, not the rejected RC2 layout.

The S01–S16 source mapping in the historical table below still applies. Local
Card, CardFooter, Button, Input, Label, NativeSelect, Combobox, Skeleton, EmptyState, Badge and
Notice patterns supply the compositions. Appearance radios and native checkbox
remain feature controls. EmptyState uses its existing Cable icon; manual export
retains its Download icon. Icon geometry was checked against the official
[Lucide Cable](https://github.com/lucide-icons/lucide/blob/main/icons/cable.svg)
and [Download](https://github.com/lucide-icons/lucide/blob/main/icons/download.svg)
sources. No package was installed.

**PROPOSED new product copy**, pending owner approval:

- Group headings: “Household · General · Appearance”; “Backups and vaults”;
  “Forecast and imports”; “Troubleshooting”; “Password, encryption and deletion”.
- Summary compositions/templates: “Weekly · last backup verified”;
  “[lower]–[upper]” money range; “[timezone] · [currency] · [appearance]”; “On”;
  “[vault name] · active”; “[count] connections”; “DohFlow · version [version]”;
  “Comfort band · auto-categorization”.
- Named progress compositions: “Back up now · Backing up…”;
  “Refresh now · Refreshing…”.

Other retained words use the approved D6 baseline, including its historical
provider fixtures; they are not current commercial claims. Synthetic values and
specification annotations are distinguished from product copy. Approved accessible
names remain “Refresh now, [connection name]” and “Rename [vault name]”.
Account mapping's “Account for [full account name]” label stays visually hidden.

A2 heading/button semantics, linked panels, Enter/Space, Tab/Shift+Tab and focus
return apply. Label open regions selectively rather than creating 24 landmarks.
No new shortcuts or arrow-key model is introduced. Existing global Search
shortcuts and the `settings-backups` destination remain. Notifications and the
existing persistent connection attention precede Settings; unchanged shell,
modal containment, inertness and focus-return states use RC2 references.

## Verification and gates

- Screenshot checkpoints covered spacing, type, source control geometry,
  alignment, long values, wrapping, theme and clipping, with targeted repairs.
- Export audit covered all **66** candidate roots: no oversized fixed-width
  descendants and no light-theme color references in dark frames.
- All **40** targeted static action/draft checks pass: one visible retry per
  checked connection, timezone or Comfort scope; the full Comfort alert and both
  retained values remain present, including hidden-body counterparts.
- All **129** copied token names/values match the pinned system baseline.
- Baseline text/card contrast is 17.63:1 light and 15.34:1 dark; muted text/card
  is 5.97:1 and 6.69:1. A2 focus ring/card is 3.76:1 and 5.17:1, with a 2px ring
  and 2px offset. Status also carries readable words, not color alone.
- Required input boundaries remain the known 1.27:1/1.46:1 source gap where 3:1
  is required. `personal-cfo-4d8.35` remains a production gate; no waiver or
  blanket AA claim is made. Convention/source drift remains
  `personal-cfo-4d8.34`.
- Static design and documentation only: no application code, dependency or
  runtime behavior changed. Application test/build suites were not run.

Fresh independent review binds to this freeze, manifest and the PR's exact
commit. Any edit requires a new freeze/review. Owner selection and new-copy
approval remain with the Decisions desk. Only the owner merges; merge closes
the bead. The workbench page is excluded from this freeze.

## Exact alternatives v2 manifest

| Page | Frame |
| --- | --- |
| Read first · narrative and spec | OPTIONS-00 · Three alternatives · narrative and recommendation · v2 |
| Read first · narrative and spec | OPTIONS-SPEC · Behavior copy components and coverage · v2 |
| A · Landscape grid | A-01 · Default · Light · C1008 · 1280 expanded-sidebar |
| A · Landscape grid | A-02 · Default · Dark · C1008 · 1280 expanded-sidebar |
| A · Landscape grid | A-03 · Default · Light · C368 · 472 collapsed-sidebar · stress |
| A · Landscape grid | A-04 · Default · Dark · C368 · 472 collapsed-sidebar · stress |
| A · Landscape grid | A-05 · Default · Light · C696 · 800 collapsed-sidebar |
| A · Landscape grid | A-06 · Default · Light · C1176 · 1280 collapsed-sidebar |
| A · Landscape grid | A-10 · Full layout · independent open · partial save · problems · Light C1008 |
| A · Landscape grid | A-11 · Full layout · long values and problems · Dark C368 · stress |
| A · Landscape grid | A-12 · Full layout · long values and problems · Light C368 · stress |
| A · Landscape grid | A-13 · Full layout · long values and problems · Dark C1008 |
| A · Landscape grid | A-14 · Default · Light C528 · 800 expanded-sidebar |
| A · Landscape grid | A-15 · Default · Dark C528 · 800 expanded-sidebar |
| A · Landscape grid | A-16 · Closed Comfort partial failure · Light C1008 |
| A · Landscape grid | A-17 · Closed Comfort partial failure · Dark C368 · stress |
| A · Landscape grid | A-18 · Closed Comfort partial failure · Light C368 · stress |
| A · Landscape grid | A-19 · Closed Comfort partial failure · Dark C1008 |
| B · Task groups | B-01 · Default · Light · C1008 · 1280 expanded-sidebar |
| B · Task groups | B-02 · Default · Dark · C1008 · 1280 expanded-sidebar |
| B · Task groups | B-03 · Default · Light · C368 · 472 collapsed-sidebar · stress |
| B · Task groups | B-04 · Default · Dark · C368 · 472 collapsed-sidebar · stress |
| B · Task groups | B-05 · Default · Light · C696 · 800 collapsed-sidebar |
| B · Task groups | B-06 · Default · Light · C1176 · 1280 collapsed-sidebar |
| B · Task groups | B-10 · Full layout · independent open · partial save · problems · Light C1008 |
| B · Task groups | B-11 · Full layout · long values and problems · Dark C368 · stress |
| B · Task groups | B-12 · Full layout · long values and problems · Light C368 · stress |
| B · Task groups | B-13 · Full layout · long values and problems · Dark C1008 |
| B · Task groups | B-14 · Default · Light C528 · 800 expanded-sidebar |
| B · Task groups | B-15 · Default · Dark C528 · 800 expanded-sidebar |
| B · Task groups | B-16 · Closed Comfort partial failure · Light C1008 |
| B · Task groups | B-17 · Closed Comfort partial failure · Dark C368 · stress |
| B · Task groups | B-18 · Closed Comfort partial failure · Light C368 · stress |
| B · Task groups | B-19 · Closed Comfort partial failure · Dark C1008 |
| C · Compact overview | C-01 · Default · Light · C1008 · 1280 expanded-sidebar |
| C · Compact overview | C-02 · Default · Dark · C1008 · 1280 expanded-sidebar |
| C · Compact overview | C-03 · Default · Light · C368 · 472 collapsed-sidebar · stress |
| C · Compact overview | C-04 · Default · Dark · C368 · 472 collapsed-sidebar · stress |
| C · Compact overview | C-05 · Default · Light · C696 · 800 collapsed-sidebar |
| C · Compact overview | C-06 · Default · Light · C1176 · 1280 collapsed-sidebar |
| C · Compact overview | C-10 · Full layout · independent open · partial save · problems · Light C1008 |
| C · Compact overview | C-11 · Full layout · long values and problems · Dark C368 · stress |
| C · Compact overview | C-12 · Full layout · long values and problems · Light C368 · stress |
| C · Compact overview | C-13 · Full layout · long values and problems · Dark C1008 |
| C · Compact overview | C-14 · Default · Light C528 · 800 expanded-sidebar |
| C · Compact overview | C-15 · Default · Dark C528 · 800 expanded-sidebar |
| C · Compact overview | C-16 · Closed Comfort partial failure · Light C1008 |
| C · Compact overview | C-17 · Closed Comfort partial failure · Dark C368 · stress |
| C · Compact overview | C-18 · Closed Comfort partial failure · Light C368 · stress |
| C · Compact overview | C-19 · Closed Comfort partial failure · Dark C1008 |
| Shared states · Connections and safety | X-01 · Connections · 24 compact rows · Light C1008 |
| Shared states · Connections and safety | X-02 · Connections · mapping and open Forget · Light C1008 |
| Shared states · Connections and safety | X-03 · Destructive guard · armed attempted-collapse Cancel busy · Light C1008 |
| Shared states · Connections and safety | X-04 · Connections · loading empty error and closed alerts · Light C1008 |
| Shared states · Connections and safety | X-05 · Connections · 24 compact rows · Dark C1008 |
| Shared states · Connections and safety | X-06 · Connections · 24 compact rows · Light C368 · stress |
| Shared states · Connections and safety | X-07 · Connections · 24 compact rows · Dark C368 · stress |
| Shared states · Connections and safety | X-08 · Connections · mapping and open Forget · Dark C1008 |
| Shared states · Connections and safety | X-09 · Connections · mapping and open Forget · Light C368 · stress |
| Shared states · Connections and safety | X-10 · Connections · mapping and open Forget · Dark C368 · stress |
| Shared states · Connections and safety | X-11 · Destructive guard · armed attempted-collapse Cancel busy · Dark C1008 |
| Shared states · Connections and safety | X-12 · Destructive guard · armed attempted-collapse Cancel busy · Light C368 · stress |
| Shared states · Connections and safety | X-13 · Destructive guard · armed attempted-collapse Cancel busy · Dark C368 · stress |
| Shared states · Connections and safety | X-14 · Connections · loading empty error and closed alerts · Dark C1008 |
| Shared states · Connections and safety | X-15 · Connections · loading empty error and closed alerts · Light C368 · stress |
| Shared states · Connections and safety | X-16 · Connections · loading empty error and closed alerts · Dark C368 · stress |

## Historical RC2 — preserved record

RC2's independent outcome was **HUMAN_DECISION_REQUIRED** at commit
`035daa4359046e081e33eb5d97d7423d917a7bb9`:
[review comment](https://github.com/dohflow/dohflow/pull/187#issuecomment-6407383533).
The earlier dark wordmark and Loader2 findings were resolved. D3–D6 were later
approved; the layout was not selected and does not count as one of the three
new options. The historical submission details below remain as provenance.

Status: **RC2 proposal; independent screen review and owner decisions pending.**
This record describes a design artifact, not an accepted frontend convention or
an implementation handoff.

- Task: `personal-cfo-twpo.3`.
- [Paper candidate](https://app.paper.design/file/01M5TNGF0SVKWW6WQCTF8Z41H0/p-1-0).
- Frozen: **October 25, 2026, 6:36:05 PM Pacific** (2026-10-26 01:36:05 UTC).
- Source: `e8e6ed00389278bb14af8b88d92097b5f61cda4d`.
- Design-system audit baseline: v0.2, frozen October 25, 2026,
  4:18:31 PM Pacific (23:18:31 UTC).
- Baseline token hash: `0afec932`; candidate-local token hash: `a378c7e0`.
  All 129 copied token names and values were compared and match. The files do
  not have synchronized component instances.

## Proposed layout

Recovery and connection health come first, preferences next, maintenance and
help below, and security actions last. All sixteen current cards remain.
The artifact uses synthetic demo data and retains the source help and
irreversible-action explanations.

The proposed container policy uses remaining main content width `C`, after
the sidebar and the existing 24px horizontal main padding on each side:

| Content width | Layout |
| --- | --- |
| Below 480px | One column; Comfort band fields stack. |
| 480–735px | One column; Comfort band fields may share a row. |
| 736–1007px | Small preference/help cards can pair; recovery cards remain full width. |
| 1008px and above | Backups and Connections can also pair, each at least 496px. |

The proposal retains 16px gaps, 24px default card padding, the existing 16px
Vaults/Delete padding, and the source control sizes. It uses stable DOM order,
wrapping actions, and no dense grid packing or masonry. Full names remain
available without hover. It does not introduce a new shared primitive.

## Coverage and component mapping

ST frames contain complete default cards. S frames show state deltas to those
cards; their unchanged descriptions, warnings, and confirmation requirements
remain present. The paired state sheets index 126 coverage variants per theme,
including four existing password-strength buckets and explicit N/A cases.
These are static design specifications, not runtime test results.

Paths below are relative to `apps/desktop/src`.

| State index | Existing composition |
| --- | --- |
| S01 | `backup/BackupScheduleCard.tsx`, `backup/BackupView.tsx` |
| S02 | `settings/ConnectionsCard.tsx`, `settings/connections/*`, `settings/NewMappedAccountDialog.tsx` |
| S03 | `settings/VaultHealthCard.tsx` |
| S04 | `settings/HouseholdCard.tsx`, `components/ui/combobox.tsx` |
| S05 | General in `settings/SettingsView.tsx` |
| S06 | `settings/AppearanceCard.tsx` |
| S07 | `settings/ComfortBandCard.tsx` |
| S08 | `settings/AutoCategorizeOnImportCard.tsx` |
| S09 | `settings/SoftwareUpdateCard.tsx` |
| S10 | `settings/DiagnosticsCard.tsx` |
| S11 | `settings/VaultsCard.tsx` |
| S12 | Setup guide in `settings/SettingsView.tsx` |
| S13 | `settings/AboutCard.tsx`, `brand/Brand.tsx` |
| S14 | `settings/ChangePasswordCard.tsx`, `vault/PasswordStrengthMeter.tsx` |
| S15 | `settings/RotateKeyCard.tsx` |
| S16 | `settings/DeleteVaultCard.tsx` |

The compositions use the existing local Card, Button, Input, Label,
NativeSelect, Combobox, Skeleton, EmptyState, Badge and Notice patterns.
Appearance radios and the native auto-categorize checkbox remain feature
compositions. Connection dialogs remain the existing feature compositions.

R frames prove the remaining-width layouts at 1280×800 and 800×600 with
224px/56px sidebar space, plus a 640×600 stress case. The latter is **not a
supported-minimum claim**. The sidebar is schematic context in these proofs:
its icons, collapse control, support action and build footer are omitted.
No sidebar change is proposed. ST frames supply the complete scrolled content.

## Copy and accessibility gates

Source strings are evidence, not blanket copy approval. ST-00 records the
provenance and proposed decisions. The new visible section headings are marked
PROPOSED. Contextual accessible names, long-name wrapping, consistent title
size, and retaining the existing Save label while busy are also explicit
proposals. Account mapping labels remain visually hidden in the product;
the state sheets print their accessible names as annotations.

Notifications remain first in main. The existing persistent connection
attention notice precedes the page title. The Backups focus destination remains
`settings-backups`. Reading order and keyboard focus order agree at every
column count. The specification preserves existing picker, native dialog and
blocking-rotation behavior and requires modal focus containment, background
inertness and focus return. It does not claim the current implementation has
passed those checks.

RC1 received CHANGES_REQUIRED for dark wordmark paint and omitted busy indicators.
RC2 corrects 98 wordmark-path bindings; an export audit across all 31 dark frames
finds no light-foreground SVG references. All 15 logo instances retain matching
path geometry and seven dark-foreground wordmark paths. The fixed mark colors
were not changed.

RC2 also adds verified 16px decorative Loader2 icons alongside retained button
names in S04/saving, S07/saving, S02/refresh-busy and S10/loading in both themes.
Captions specify runtime animation, a static reduced-motion cue and polite busy
announcements without per-tick repetition. Paper itself is static. This repair
does not approve the pending C08 adaptation or other owner decisions.
Diagnostics preview text uses the theme foreground token.
Token calculations give text/card contrast of 17.63:1 light and 15.34:1 dark;
muted text/card is 5.97:1 and 6.69:1. Primary and destructive button text also
clear 4.5:1 in both themes. These calculations are **not a blanket accessibility
PASS**. Required input boundaries remain a known source gap: 1.27:1 light and
1.46:1 dark against the input background. The proposed neutral text plus
semantic mark treatment needs independent review and approval.

Provider wording and historical economics in F00/F01/F05 come from the
source registry fixture; they are not current commercial claims. Diagnostics
uses synthetic test-fixture JSON. No real financial data, paths or credentials
were used.

## Exact frozen frame manifest

A review binds to this freeze and these names. Any Paper edit requires a new
freeze and review. The owner approves layout/copy and performs merges; an
artifact review does not authorize implementation.

| Page | Frame |
| --- | --- |
| Page 1 | ST-00 · RC2 review manifest and specifications |
| Page 1 | ST-01 · Settings full content · Wide Light · RC2 |
| Page 1 | ST-02 · Settings full content · Wide Dark · RC2 |
| Page 1 | ST-03 · Settings full content · C528 Light · RC2 |
| Page 1 | ST-04 · Settings full content · C528 Dark · RC2 |
| Page 1 | ST-05 · Settings full content · C368 Light · stress · RC2 |
| Page 1 | ST-06 · Settings full content · C368 Dark · stress · RC2 |
| 02 · States and accessibility | S01 · Backups state deltas · Light · RC2 |
| 02 · States and accessibility | S02 · Connections state deltas · Light · RC2 |
| 02 · States and accessibility | S03 · Vault health state deltas · Light · RC2 |
| 02 · States and accessibility | S04 · Household state deltas · Light · RC2 |
| 02 · States and accessibility | S05 · General state deltas · Light · RC2 |
| 02 · States and accessibility | S06 · Appearance state deltas · Light · RC2 |
| 02 · States and accessibility | S07 · Comfort band state deltas · Light · RC2 |
| 02 · States and accessibility | S08 · Auto-categorize imported transactions state deltas · Light · RC2 |
| 02 · States and accessibility | S09 · Software update state deltas · Light · RC2 |
| 02 · States and accessibility | S10 · Diagnostics state deltas · Light · RC2 |
| 02 · States and accessibility | S11 · Vaults state deltas · Light · RC2 |
| 02 · States and accessibility | S12 · Setup guide state deltas · Light · RC2 |
| 02 · States and accessibility | S13 · About state deltas · Light · RC2 |
| 02 · States and accessibility | S14 · Change master password state deltas · Light · RC2 |
| 02 · States and accessibility | S15 · Rotate encryption key state deltas · Light · RC2 |
| 02 · States and accessibility | S16 · Delete this vault state deltas · Light · RC2 |
| 01 · Responsive windows | R01 · 1280×800 · Sidebar 224 · Light · RC2 |
| 01 · Responsive windows | R02 · 1280×800 · Sidebar 56 · Light · RC2 |
| 01 · Responsive windows | R03 · 800×600 · Sidebar 224 · Light · RC2 |
| 01 · Responsive windows | R11 · 800×600 · Existing persistent attention · Light · RC2 |
| 01 · Responsive windows | R12 · 800×600 · Existing persistent attention · Dark · RC2 |
| 01 · Responsive windows | R04 · 800×600 · Sidebar 56 · Light · RC2 |
| 01 · Responsive windows | R05 · 1280×800 · Sidebar 224 · Dark · RC2 |
| 01 · Responsive windows | R06 · 1280×800 · Sidebar 56 · Dark · RC2 |
| 01 · Responsive windows | R07 · 800×600 · Sidebar 224 · Dark · RC2 |
| 01 · Responsive windows | R09 · 640×600 · Sidebar 224 · Light · stress, not supported minimum · RC2 |
| 01 · Responsive windows | R10 · 640×600 · Sidebar 224 · Dark · stress, not supported minimum · RC2 |
| 01 · Responsive windows | R08 · 800×600 · Sidebar 56 · Dark · RC2 |
| 03 · Dark state counterparts | S01 · Backups state deltas · Dark · RC2 |
| 03 · Dark state counterparts | S02 · Connections state deltas · Dark · RC2 |
| 03 · Dark state counterparts | S03 · Vault health state deltas · Dark · RC2 |
| 03 · Dark state counterparts | S04 · Household state deltas · Dark · RC2 |
| 03 · Dark state counterparts | S05 · General state deltas · Dark · RC2 |
| 03 · Dark state counterparts | S06 · Appearance state deltas · Dark · RC2 |
| 03 · Dark state counterparts | S07 · Comfort band state deltas · Dark · RC2 |
| 03 · Dark state counterparts | S08 · Auto-categorize imported transactions state deltas · Dark · RC2 |
| 03 · Dark state counterparts | S09 · Software update state deltas · Dark · RC2 |
| 03 · Dark state counterparts | S10 · Diagnostics state deltas · Dark · RC2 |
| 03 · Dark state counterparts | S11 · Vaults state deltas · Dark · RC2 |
| 03 · Dark state counterparts | S12 · Setup guide state deltas · Dark · RC2 |
| 03 · Dark state counterparts | S13 · About state deltas · Dark · RC2 |
| 03 · Dark state counterparts | S14 · Change master password state deltas · Dark · RC2 |
| 03 · Dark state counterparts | S15 · Rotate encryption key state deltas · Dark · RC2 |
| 03 · Dark state counterparts | S16 · Delete this vault state deltas · Dark · RC2 |
| 04 · Connection and blocking flows | F01 · Provider disclosure · Light · RC2 |
| 04 · Connection and blocking flows | F05 · Credential step after disclosure · Light · RC2 |
| 04 · Connection and blocking flows | F02 · Create and map dialog · Light · RC2 |
| 04 · Connection and blocking flows | F03 · Shared-feed confirmation · Light · RC2 |
| 04 · Connection and blocking flows | F04 · Blocking rotation · Light · RC2 |
| 04 · Connection and blocking flows | F01 · Provider disclosure · Dark · RC2 |
| 04 · Connection and blocking flows | F05 · Credential step after disclosure · Dark · RC2 |
| 04 · Connection and blocking flows | F02 · Create and map dialog · Dark · RC2 |
| 04 · Connection and blocking flows | F03 · Shared-feed confirmation · Dark · RC2 |
| 04 · Connection and blocking flows | F04 · Blocking rotation · Dark · RC2 |
| 04 · Connection and blocking flows | F00 · Provider picker · source registry fixture · Light · RC2 |
| 04 · Connection and blocking flows | F00 · Provider picker · source registry fixture · Dark · RC2 |

## Verification

- Screenshot checkpoints covered layout, wrapping, alignment, type, theme and
  clipping, with targeted repairs before this freeze.
- Token name/value parity was checked against the pinned system baseline.
- Text-color and SVG-theme binding audits covered the candidate roots.
- The application was not launched. No application code, package or dependency
  changed, so application test/build suites were not run.
