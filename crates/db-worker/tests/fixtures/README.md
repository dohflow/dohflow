# Frozen SQLCipher logical state

`sqlcipher-4.5.7-state.json` is the complete logical state of all 70 application
tables (including migrations, audit/operation history and projections), plus
`user_version`, read from fresh copies of the synthetic outgoing-engine corpus
in `crates/finance-kernel/tests/fixtures/sqlcipher-4.5.7.json`. Rows are sorted
and SQLite values retain their type; text/blob bytes are base64 encoded. This
is test data, not a personal vault or export.

Capture was performed before upgrading, with linked SQLCipher `4.5.7 community`
and SQLite `3.45.3`. The corpus contains both a checkpointed database and a
commit existing only in WAL; no shared-memory index is retained. Tests verify
artifact hashes before decoding, then compare every logical row, run integrity
and foreign-key checks, and exercise the production password/backup paths.

Capture helpers are ignored, require explicit **new** output paths, and refuse
replacement. To reproduce using the outgoing engine, use the fixture checkpoint
commit (before the engine upgrade), not the incoming engine:

```sh
G3M5_CAPTURE_PATH=/tmp/new-outgoing-corpus.json cargo test -p finance-kernel --test engine_compatibility capture_outgoing_engine_corpus -- --ignored --exact
G3M5_STATE_PATH=/tmp/new-outgoing-state.json cargo test -p db-worker --test engine_contract capture_outgoing_sql_state -- --ignored --exact
```

The second command reads the checked-in corpus, not the first command's new
output. Random encryption salts/nonces and generated internal IDs mean a new
capture is not byte-identical. The checked-in capture is immutable evidence:
never regenerate expected state with the incoming engine to make a test pass.

SHA-256 at capture:

- encrypted corpus JSON: `3817d31fa86ef745eea4d9670874be555e6a4248ea2d951dcb3b00a6240db28a`
- logical state JSON: `977dd6866ef51558b3e792aa0d287c1990e0b0315dbf0d18bad555f4bebdba69`

The older frozen PCFOBK v1 fixture remains unchanged and is exercised by
`backup_v1_compat`; no real financial data is used in any of these fixtures.
