//! Re-authentication notices (personal-cfo-pxi.22; ADR 0060 addendum 2026-10-23).
//!
//! A bank connection that needs the user to sign in again at the provider must stay visible until
//! the NEXT healthy refresh. The one-bank SimpleFIN `con.auth` is not even a failed sync (the other
//! banks refresh normally), and `last_error` is free text that every healthy sync clears, so the
//! state lives in its own device-local table (`connector_attention`, migration 58, class 3). This
//! module is the whole vocabulary of that table: what a refresh outcome means for it, and how it is
//! stored and read back. The UI renders ONLY from the structured entries, never from warning prose.
//!
//! Rules:
//! * only a FULLY healthy refresh (a sync that ingested, or a discovery that succeeded) replaces
//!   or clears the notice; a network error, a rate limit or an ingest failure leaves it as it was;
//! * a bank name only ever comes from the adapter's sanitized data; it is cleaned again here
//!   (defense in depth) because it is shown to the user and kept in the vault;
//! * stored JSON that does not parse reads as "no notice", never as an error that hides the
//!   connection.

use connector_core::{ConnectorError, ReauthRequired};
use serde::{Deserialize, Serialize};

/// The longest bank name kept, in characters (the adapters bound theirs at 200).
const MAX_INSTITUTION_CHARS: usize = 200;

/// What the user has to do about a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionKind {
    /// One bank inside the provider needs the user to sign in again there (SimpleFIN `con.auth`).
    ReauthInstitution,
    /// The whole connection no longer works and has to be set up again (a refused or expired
    /// credential, SimpleFIN `gen.auth`).
    RelinkConnection,
}

impl AttentionKind {
    /// The wire token, as the IPC DTO carries it.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::ReauthInstitution => "reauth_institution",
            Self::RelinkConnection => "relink_connection",
        }
    }
}

/// One thing a connection needs. `institution` is the bank's own name when the provider gave one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionEntry {
    pub kind: AttentionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub institution: Option<String>,
}

/// A bank name as shown and stored: printable text only (no control, bidirectional-override or
/// zero-width characters), trimmed, bounded; `None` when nothing usable is left.
fn clean_institution(raw: &str) -> Option<String> {
    let forged = |c: &char| {
        c.is_control()
            || matches!(
                c,
                '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}'
            )
    };
    let kept: String = raw.chars().filter(|c| !forged(c)).collect();
    let trimmed = kept.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(MAX_INSTITUTION_CHARS).collect())
}

/// What a FAILED refresh means for the notice: `Some(entries)` to record, `None` to leave the
/// notice as it is. Only the failures that need the user at the provider produce an entry.
#[must_use]
pub fn from_failure(err: &ConnectorError) -> Option<Vec<AttentionEntry>> {
    match err {
        // A refused or expired credential: the connection has to be set up again.
        ConnectorError::Expired(_) => Some(vec![AttentionEntry {
            kind: AttentionKind::RelinkConnection,
            institution: None,
        }]),
        ConnectorError::NeedsUserAction { code, .. } => {
            if code == "gen.auth" {
                // The provider refused the whole connection.
                Some(vec![AttentionEntry {
                    kind: AttentionKind::RelinkConnection,
                    institution: None,
                }])
            } else if code.starts_with("con.") {
                // A bank-scoped sign-in need on a path that treats it as fatal (link, account
                // discovery): the bank is not named there.
                Some(vec![AttentionEntry {
                    kind: AttentionKind::ReauthInstitution,
                    institution: None,
                }])
            } else {
                None
            }
        }
        _ => None,
    }
}

/// What a SUCCESSFUL sync means for the notice: one entry per bank the provider says needs a
/// sign-in (empty = the connection is healthy, so the notice is cleared). The same bank is never
/// listed twice.
#[must_use]
pub fn from_sync(reauth: &[ReauthRequired]) -> Vec<AttentionEntry> {
    let mut entries: Vec<AttentionEntry> = Vec::new();
    for item in reauth {
        let entry = AttentionEntry {
            kind: AttentionKind::ReauthInstitution,
            institution: item.institution.as_deref().and_then(clean_institution),
        };
        if !entries.contains(&entry) {
            entries.push(entry);
        }
    }
    entries
}

/// The stored form (a JSON array).
#[must_use]
pub fn encode(entries: &[AttentionEntry]) -> String {
    serde_json::to_string(entries).unwrap_or_else(|_| "[]".to_owned())
}

/// Read the stored form. A missing, empty or unparseable value is "no notice"; names are cleaned
/// again so a value written by anything else cannot put control characters in front of the user.
#[must_use]
pub fn decode(stored: Option<&str>) -> Vec<AttentionEntry> {
    let Some(stored) = stored else {
        return Vec::new();
    };
    let Ok(mut entries) = serde_json::from_str::<Vec<AttentionEntry>>(stored) else {
        tracing::warn!("connector attention: stored notice is not readable; showing none");
        return Vec::new();
    };
    for entry in &mut entries {
        entry.institution = entry.institution.as_deref().and_then(clean_institution);
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str) -> ReauthRequired {
        ReauthRequired {
            institution: Some(name.to_owned()),
        }
    }

    #[test]
    fn an_expired_credential_or_gen_auth_means_set_the_connection_up_again() {
        let relink = Some(vec![AttentionEntry {
            kind: AttentionKind::RelinkConnection,
            institution: None,
        }]);
        assert_eq!(from_failure(&ConnectorError::Expired("403".into())), relink);
        let gen_auth = ConnectorError::NeedsUserAction {
            code: "gen.auth".into(),
            message: "Forbidden".into(),
            help_url: None,
        };
        assert_eq!(from_failure(&gen_auth), relink);
    }

    #[test]
    fn a_fatal_con_auth_means_a_bank_needs_a_sign_in_without_naming_it() {
        let con_auth = ConnectorError::NeedsUserAction {
            code: "con.auth".into(),
            message: "Reauthorize".into(),
            help_url: None,
        };
        assert_eq!(
            from_failure(&con_auth),
            Some(vec![AttentionEntry {
                kind: AttentionKind::ReauthInstitution,
                institution: None,
            }])
        );
    }

    #[test]
    fn failures_that_do_not_need_the_user_at_the_provider_leave_the_notice_alone() {
        for err in [
            ConnectorError::RateLimited("slow down".into()),
            ConnectorError::Network("offline".into()),
            ConnectorError::Provider("500".into()),
            ConnectorError::NeedsUserAction {
                code: "lunchflow.account_status".into(),
                message: "inactive".into(),
                help_url: None,
            },
        ] {
            assert_eq!(from_failure(&err), None, "{err:?}");
        }
    }

    #[test]
    fn a_sync_with_no_reauth_entries_is_healthy_and_clears() {
        assert!(from_sync(&[]).is_empty());
    }

    #[test]
    fn a_sync_names_each_bank_once_and_keeps_an_unnamed_one_unnamed() {
        let entries = from_sync(&[
            named("First Bank"),
            ReauthRequired { institution: None },
            named("First Bank"),
            named("Second Bank"),
        ]);
        let names: Vec<_> = entries.iter().map(|e| e.institution.as_deref()).collect();
        assert_eq!(names, [Some("First Bank"), None, Some("Second Bank")]);
        assert!(entries
            .iter()
            .all(|e| e.kind == AttentionKind::ReauthInstitution));
    }

    #[test]
    fn bank_names_are_cleaned_and_bounded_again() {
        let entries = from_sync(&[
            named("Evil\u{202E}Bank\u{7}\u{200B} Name"),
            named("   "),
            named(&"A".repeat(500)),
        ]);
        assert_eq!(entries[0].institution.as_deref(), Some("EvilBank Name"));
        assert_eq!(entries[1].institution, None, "a blank name is no name");
        assert_eq!(
            entries[2].institution.as_deref().map(|n| n.chars().count()),
            Some(MAX_INSTITUTION_CHARS)
        );
    }

    #[test]
    fn what_is_stored_reads_back_and_reads_cleaned() {
        let entries = from_sync(&[named("First Bank"), ReauthRequired { institution: None }]);
        assert_eq!(decode(Some(&encode(&entries))), entries);
        // A value written by something else cannot smuggle control characters.
        let hostile = r#"[{"kind":"reauth_institution","institution":"Bad\u202EBank\u0007"}]"#;
        assert_eq!(
            decode(Some(hostile))[0].institution.as_deref(),
            Some("BadBank")
        );
    }

    #[test]
    fn nothing_stored_or_unreadable_stored_is_no_notice() {
        assert!(decode(None).is_empty());
        assert!(decode(Some("")).is_empty());
        assert!(decode(Some("not json")).is_empty());
        assert!(decode(Some(r#"[{"kind":"something_new"}]"#)).is_empty());
        assert!(decode(Some(r#"{"kind":"reauth_institution"}"#)).is_empty());
    }

    #[test]
    fn the_wire_tokens_are_stable() {
        assert_eq!(
            AttentionKind::ReauthInstitution.token(),
            "reauth_institution"
        );
        assert_eq!(AttentionKind::RelinkConnection.token(), "relink_connection");
        let stored = encode(&[AttentionEntry {
            kind: AttentionKind::RelinkConnection,
            institution: None,
        }]);
        assert_eq!(stored, r#"[{"kind":"relink_connection"}]"#);
    }
}
