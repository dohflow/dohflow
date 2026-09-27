# LunchFlow wire fixtures

Synthetic responses shaped exactly like LunchFlow's published Personal API
schemas (lunchflow.app/docs/api/personal-api, read 2026-09-27): field names,
types and envelopes are the documented ones. Every value is invented — no
real account, institution, merchant, amount or id. Where the documentation is
silent (the amount sign convention), the fixtures follow the assumption stated
on `to_ledger_sign` in `src/lib.rs`, which the owner's live check on
personal-cfo-r2pow confirms or corrects.
