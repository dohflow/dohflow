//! The deterministic mock connector (personal-cfo-1s2b).
//!
//! Test-only by construction: compiled solely under `cfg(test)` or the
//! non-default `mock` feature, enabled from dev-dependencies (CI greps that no
//! production `[dependencies]` enables it). It is **never** registered with
//! [`register_connector!`](crate::register_connector) — tests construct it
//! explicitly, so the production adapter roster can never contain it even when
//! the feature is on.
//!
//! Determinism: fixture data is a pure function of the configuration — no
//! wall clock, no randomness (DoD fixture-determinism policy). A seed knob is
//! deliberately absent until a test needs varied datasets (recorded on the
//! bead).

use chrono::NaiveDate;
use importer_core::{ParsedAccount, ParsedBalance, ParsedRecord, ParsedTransaction};
use semver::Version;

use crate::{
    Capability, CapabilitySet, Connection, ConnectorAdapter, ConnectorError, Credential,
    HealthStatus, LinkInput, LinkSession,
};

/// Error injection for exercising the ADR 0060 §5 taxonomy end to end
/// (connection health `ul5d`, connector-error inbox items `zfyo`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FailureMode {
    /// Behave: serve fixture data.
    #[default]
    None,
    /// Every call answers `RateLimited` — the *healthy* throttled state.
    RateLimited,
    /// Every call answers `Expired` — the re-link path.
    Expired,
    /// Every call answers `NeedsUserAction` (e.g. institution re-auth).
    NeedsUserAction,
}

/// A deterministic in-memory [`ConnectorAdapter`] for tests.
#[derive(Debug, Clone)]
pub struct MockConnector {
    capabilities: CapabilitySet,
    failure_mode: FailureMode,
    /// The token [`ConnectorAdapter::link`] accepts; anything else is a
    /// `NeedsUserAction` (mirrors a single-use/wrong setup token).
    accepted_token: &'static str,
    /// The adapter id this mock reports. Defaults to `"mock"`; integration
    /// tests that drive real staging use a `source_batches.source_type`
    /// schema token (`"other"`) via [`Self::with_id`].
    id_token: &'static str,
    /// When set, the fixture includes an account with no id or name, to
    /// exercise the default sync's skip-warning path.
    include_unnamed_account: bool,
    /// Raw provider account ids the mock reports retry-required
    /// (watermark-hold) on every sync.
    held_accounts: &'static [&'static str],
    /// When set, [`ConnectorAdapter::fetch_accounts`] returns an empty list —
    /// models a provider whose account list is not ready yet (the SimpleFIN
    /// Bridge right after an app connection is created, personal-cfo-k025).
    /// `sync` still returns the full fixture.
    empty_discovery: bool,
}

impl MockConnector {
    /// Accounts + transactions + balances, no failures — the common case.
    #[must_use]
    pub fn with_fixture() -> Self {
        Self {
            capabilities: CapabilitySet {
                accounts: true,
                transactions: true,
                balances: true,
                holdings: false,
                liabilities: false,
            },
            failure_mode: FailureMode::None,
            accepted_token: "mock-setup-token",
            id_token: "mock",
            include_unnamed_account: false,
            held_accounts: &[],
            empty_discovery: false,
        }
    }

    /// Same fixture, reporting these raw account ids retry-required so
    /// watermark-hold plumbing can be tested end to end.
    #[must_use]
    pub fn with_held_accounts(mut self, held: &'static [&'static str]) -> Self {
        self.held_accounts = held;
        self
    }

    /// Same fixture, but link-time account discovery finds nothing —
    /// `fetch_accounts` returns an empty list while `sync` still carries the
    /// full fixture (the provider-not-ready-yet shape, personal-cfo-k025).
    #[must_use]
    pub fn with_empty_discovery(mut self) -> Self {
        self.empty_discovery = true;
        self
    }

    /// Same fixture, reporting a different adapter id.
    #[must_use]
    pub fn with_id(mut self, id: &'static str) -> Self {
        self.id_token = id;
        self
    }

    /// Same fixture, different declared capabilities.
    #[must_use]
    pub fn with_capabilities(capabilities: CapabilitySet) -> Self {
        Self {
            capabilities,
            ..Self::with_fixture()
        }
    }

    /// Same fixture, every call failing with `mode`.
    #[must_use]
    pub fn failing_with(mode: FailureMode) -> Self {
        Self {
            failure_mode: mode,
            ..Self::with_fixture()
        }
    }

    /// The fixture plus one account with no external id or name.
    #[must_use]
    pub fn with_unnamed_account() -> Self {
        Self {
            include_unnamed_account: true,
            ..Self::with_fixture()
        }
    }

    fn check_failure(&self) -> Result<(), ConnectorError> {
        match self.failure_mode {
            FailureMode::None => Ok(()),
            FailureMode::RateLimited => Err(ConnectorError::RateLimited(
                "mock: try again tomorrow".to_owned(),
            )),
            FailureMode::Expired => Err(ConnectorError::Expired(
                "mock: access URL no longer valid".to_owned(),
            )),
            FailureMode::NeedsUserAction => Err(ConnectorError::NeedsUserAction {
                code: "con.auth".to_owned(),
                message: "mock: institution requires reauthorization".to_owned(),
                help_url: Some("https://example.invalid/reauth".to_owned()),
            }),
        }
    }

    fn fixture_accounts(&self) -> Vec<ParsedAccount> {
        let mut accounts = vec![
            ParsedAccount {
                external_id: Some("mock-acct-checking".to_owned()),
                external_name: Some("Mock Checking".to_owned()),
                external_number_hash: Some("sha256:mock-checking".to_owned()),
                proposed_subtype: Some("checking".to_owned()),
            },
            ParsedAccount {
                external_id: Some("mock-acct-card".to_owned()),
                external_name: Some("Mock Card".to_owned()),
                external_number_hash: Some("sha256:mock-card".to_owned()),
                proposed_subtype: Some("credit_card".to_owned()),
            },
        ];
        if self.include_unnamed_account {
            accounts.push(ParsedAccount {
                external_id: None,
                external_name: None,
                external_number_hash: None,
                proposed_subtype: None,
            });
        }
        accounts
    }

    /// Three deterministic transactions per account, dated relative to a fixed
    /// anchor so `since` filtering is exercised.
    fn fixture_transactions(account: &str, since: Option<NaiveDate>) -> Vec<ParsedRecord> {
        use core_money::{Currency, Money};
        let anchor = NaiveDate::from_ymd_opt(2026, 7, 15).expect("valid fixture anchor");
        [(0_u64, -1250_i64), (10, -4599), (20, 250_000)]
            .into_iter()
            .enumerate()
            .filter_map(|(i, (day_offset, minor))| {
                let posted = anchor + chrono::Days::new(day_offset);
                if since.is_some_and(|s| posted < s) {
                    return None;
                }
                let merchant = format!("mock merchant {i}");
                Some(ParsedRecord {
                    external_id: Some(format!("mock-{account}-{i}")),
                    source_hash: format!("sha256:mock-{account}-{i}"),
                    normalized_json: format!("{{\"mock_row\":{i}}}"),
                    parse_confidence_bps: Some(10_000),
                    transaction: Some(ParsedTransaction {
                        posted_date: posted,
                        transaction_date: None,
                        raw_date: posted.to_string(),
                        date_confidence_bps: 10_000,
                        amount: Money::new(minor, Currency::Usd),
                        description: Some(format!("Mock purchase {i}")),
                        category: None,
                        normalized_merchant: Some(merchant.clone()),
                        external_account: Some(account.to_owned()),
                        txn_fingerprint: format!("{posted}|{minor}|{merchant}|{account}"),
                    }),
                    balance: None,
                })
            })
            .collect()
    }
}

impl ConnectorAdapter for MockConnector {
    fn id(&self) -> &'static str {
        self.id_token
    }

    fn display_name(&self) -> &'static str {
        "Mock connector (tests only)"
    }

    fn version(&self) -> Version {
        Version::new(0, 1, 0)
    }

    fn capabilities(&self) -> CapabilitySet {
        self.capabilities
    }

    fn link(&self, input: &LinkInput) -> Result<LinkSession, ConnectorError> {
        self.check_failure()?;
        let token = input
            .user_token
            .as_ref()
            .ok_or_else(|| ConnectorError::NeedsUserAction {
                code: "token.missing".to_owned(),
                message: "mock: paste a setup token".to_owned(),
                help_url: None,
            })?;
        if token.expose_secret() != self.accepted_token {
            return Err(ConnectorError::NeedsUserAction {
                code: "setup_token_used".to_owned(),
                message: "mock: setup token already claimed — generate a new one".to_owned(),
                help_url: None,
            });
        }
        Ok(LinkSession::Established {
            credential: Credential::new("mock-access-url"),
            display_hint: Some("Mock bridge, 2 accounts".to_owned()),
        })
    }

    fn fetch_accounts(&self, _conn: &Connection) -> Result<Vec<ParsedAccount>, ConnectorError> {
        self.check_failure()?;
        self.capabilities.ensure(Capability::Accounts)?;
        if self.empty_discovery {
            return Ok(Vec::new());
        }
        Ok(self.fixture_accounts())
    }

    fn fetch_transactions(
        &self,
        _conn: &Connection,
        account_external_id: &str,
        since: Option<NaiveDate>,
    ) -> Result<Vec<ParsedRecord>, ConnectorError> {
        self.check_failure()?;
        self.capabilities.ensure(Capability::Transactions)?;
        Ok(Self::fixture_transactions(account_external_id, since))
    }

    fn fetch_balances(
        &self,
        _conn: &Connection,
        account_external_id: &str,
    ) -> Result<Vec<ParsedBalance>, ConnectorError> {
        use core_money::{Currency, Money};
        self.check_failure()?;
        self.capabilities.ensure(Capability::Balances)?;
        Ok(vec![ParsedBalance {
            observed_at: NaiveDate::from_ymd_opt(2026, 8, 4).expect("valid fixture date"),
            amount: Money::new(123_456, Currency::Usd),
            external_account: Some(account_external_id.to_owned()),
        }])
    }

    fn sync(
        &self,
        conn: &Connection,
        since: Option<chrono::NaiveDate>,
    ) -> Result<crate::SyncBatch, ConnectorError> {
        // Compose exactly like the default implementation, then stamp the
        // configured retry-required scopes so watermark-hold plumbing is
        // testable end to end.
        let capabilities = self.capabilities();
        capabilities.ensure(crate::Capability::Accounts)?;
        let accounts = self.fetch_accounts(conn)?;
        let mut records = Vec::new();
        let mut warnings = Vec::new();
        let mut balance_index = 0_usize;
        for account in &accounts {
            let Some(key) = account
                .external_id
                .as_deref()
                .or(account.external_name.as_deref())
            else {
                warnings.push(importer_core::ParseWarning {
                    row: None,
                    message: "account with no external id or name was skipped during sync"
                        .to_owned(),
                });
                continue;
            };
            if capabilities.supports(crate::Capability::Transactions) {
                records.extend(self.fetch_transactions(conn, key, since)?);
            }
            if capabilities.supports(crate::Capability::Balances) {
                for balance in self.fetch_balances(conn, key)? {
                    records.push(crate::balance_record(balance_index, balance));
                    balance_index += 1;
                }
            }
        }
        Ok(crate::SyncBatch {
            adapter_id: self.id().to_owned(),
            adapter_version: self.version().to_string(),
            since,
            held_account_ids: self.held_accounts.iter().map(|s| (*s).to_owned()).collect(),
            hold_all_watermarks: false,
            batch: importer_core::ParsedBatch {
                source_format: self.id().to_owned(),
                accounts,
                records,
                warnings,
            },
        })
    }

    fn health(&self, _conn: &Connection) -> Result<HealthStatus, ConnectorError> {
        Ok(match self.failure_mode {
            FailureMode::None => HealthStatus::Healthy,
            FailureMode::RateLimited => HealthStatus::RateLimited,
            FailureMode::Expired => HealthStatus::Expired,
            FailureMode::NeedsUserAction => HealthStatus::NeedsUserAction {
                code: "con.auth".to_owned(),
                message: "mock: institution requires reauthorization".to_owned(),
                help_url: Some("https://example.invalid/reauth".to_owned()),
            },
        })
    }
}
