# LunchFlow wire fixtures

Synthetic responses shaped exactly like LunchFlow's published Personal API
schemas (lunchflow.app/docs/api/personal-api, read 2026-09-27): field names,
types and envelopes are the documented ones. Every value is invented — no
real account, institution, merchant, amount or id. Where the documentation is
silent (the amount sign convention), the fixtures follow the convention the
owner confirmed on a live account (2026-09-27, personal-cfo-r2pow): money
leaving the account is negative.

Live deltas from the docs, observed on the owner's account (2026-09-27) and
mirrored here: account objects carry **no `currency`** (account 101 omits
it; account 102 keeps the documented field so both shapes stay covered), a
`provider` value the docs don't list (`quiltt`), and a wrong key answered
**403** `{error, message}`, not 401.
