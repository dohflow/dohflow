use std::fmt;

use crate::SecretError;

/// Every service name starts with this, so DohFlow items are recognizable and never collide with
/// another app's.
pub const SERVICE_PREFIX: &str = "app.dohflow.";

const MAX_PART_BYTES: usize = 128;

fn valid_part(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_PART_BYTES
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Identifies one secret. **Not secret:** it is routing data (it may carry a vault id) and is safe
/// to print. Validated so it can be passed to a platform API without escaping surprises.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SecretKey {
    service: String,
    account: String,
}

impl SecretKey {
    /// `service` must start with [`SERVICE_PREFIX`] and, like `account`, be 1 to 128 ASCII
    /// letters, digits, `.`, `_` or `-`.
    ///
    /// # Errors
    /// [`SecretError::InvalidKey`] when either part is out of range.
    pub fn new(service: &str, account: &str) -> Result<Self, SecretError> {
        if !service.starts_with(SERVICE_PREFIX) || !valid_part(service) || !valid_part(account) {
            return Err(SecretError::InvalidKey);
        }
        Ok(Self {
            service: service.to_owned(),
            account: account.to_owned(),
        })
    }

    #[must_use]
    pub fn service(&self) -> &str {
        &self.service
    }

    #[must_use]
    pub fn account(&self) -> &str {
        &self.account
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretKey({}/{})", self.service, self.account)
    }
}

impl fmt::Display for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.service, self.account)
    }
}

/// A team-prefixed keychain access group (`<TEAMID>.<bundle id>`), supplied by the app's
/// configuration, never hard-coded in an implementation. Every item and every query names it
/// explicitly, so where an item lives is a decision and not "the first group in the entitlement
/// list" (ADR 0002 addendum 2026-10-24).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessGroup(String);

impl AccessGroup {
    /// A 10-character uppercase team id, a dot, then a bundle id of the allowed characters.
    ///
    /// # Errors
    /// [`SecretError::InvalidKey`] when the shape is wrong.
    pub fn new(group: &str) -> Result<Self, SecretError> {
        let Some((team, bundle)) = group.split_once('.') else {
            return Err(SecretError::InvalidKey);
        };
        let team_ok = team.len() == 10
            && team
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
        if !team_ok || !valid_part(bundle) {
            return Err(SecretError::InvalidKey);
        }
        Ok(Self(group.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
