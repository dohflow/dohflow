//! Canonical merchant-identity entity layer (ADR 0030 addendum; bead personal-cfo-zrpg).
//!
//! `merchant_identities` are real-world merchants; `merchant_aliases` map each
//! `categorization::normalize_merchant` key (7yh0) onto exactly one identity, so different
//! raw strings for the same merchant (`"AMZN MKTP US"`, `"AMAZON COM"`) resolve together. A
//! category learned for one alias then applies to all, and `is_ambiguous` flags merchants
//! that span categories so auto-categorization defers to disambiguation.
//!
//! This is the schema seam only — seeding, fuzzy alias matching, and the read-model
//! population are downstream beads (7yh0-v2, the ambiguous-merchant beads), each designed
//! against its real consumer.
//!
//! Writes: the only writer here is the genesis seed (the vault-creating transaction). Earned brands
//! and learned categories reach the tables through `MintMerchantAnchors` and `ApplyMerchantMemory`
//! (`apply/merchant_cmds.rs`, personal-cfo-5ymg8.5); the rest of this module is read-only.

use categorization::normalize_merchant;
use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::DbError;

/// A canonical merchant seeded at vault init (ADR 0030 addendum, personal-cfo-6p6b / -2a6r /
/// -9rtv / -x75h). Each ambiguous-merchant bead extends [`SEED_MERCHANTS`].
struct SeedMerchant {
    display_name: &'static str,
    ambiguous: bool,
    /// Descriptors that resolve by exact normalized key — online merchants whose key is
    /// stable (`AMAZON.COM`, `APPLE.COM/BILL`).
    exact_aliases: &'static [&'static str],
    /// Brand prefixes that resolve any key beginning with them on a word boundary — physical
    /// chains whose normalized key keeps the city (`COSTCO` matches `COSTCO WHSE SEATTLE`).
    /// Must be unambiguous brand tokens; the word boundary keeps `APPLE` off `APPLEBEE'S`.
    prefix_aliases: &'static [&'static str],
}

/// Canonical merchants seeded into every vault — genuinely category-spanning merchants
/// flagged ambiguous so the merchant-memory guard (7yh0-v2) defers them to disambiguation.
/// Online merchants use exact aliases; physical chains use brand-prefix aliases (their key
/// keeps the store city). Consistently-categorizable sub-brands (Amazon Prime, Whole Foods,
/// Apple Card payments) are deliberately left out so they categorize normally.
const SEED_MERCHANTS: &[SeedMerchant] = &[
    SeedMerchant {
        display_name: "Amazon",
        ambiguous: true,
        exact_aliases: &[
            "AMZN MKTP US",
            "AMAZON.COM",
            "AMAZON MKTPL",
            "AMAZON MARKETPLACE",
            "AMZN MKTP",
        ],
        prefix_aliases: &[],
    },
    SeedMerchant {
        display_name: "Apple",
        ambiguous: true,
        exact_aliases: &[
            "APPLE.COM/BILL",
            "APPLE.COM",
            "ITUNES.COM/BILL",
            "ITUNES.COM",
        ],
        prefix_aliases: &["APPLE"],
    },
    SeedMerchant {
        display_name: "Costco",
        ambiguous: true,
        exact_aliases: &["COSTCO.COM"],
        prefix_aliases: &["COSTCO"],
    },
    SeedMerchant {
        display_name: "Target",
        ambiguous: true,
        exact_aliases: &["TARGET.COM"],
        prefix_aliases: &["TARGET"],
    },
    SeedMerchant {
        display_name: "Walmart",
        ambiguous: true,
        exact_aliases: &["WALMART.COM"],
        prefix_aliases: &["WALMART", "WAL-MART", "WM SUPERCENTER"],
    },
    // P2P transfers: ambiguous when the descriptor carries no counterparty (a bare "VENMO").
    // When it does, `normalize_merchant` strips the processor prefix and the counterparty
    // survives as the key, which groups + categorizes on its own — so only the bare forms
    // are seeded. The richer amount/recurrence signals are a follow-on (personal-cfo-x75h).
    SeedMerchant {
        display_name: "Venmo",
        ambiguous: true,
        exact_aliases: &["VENMO"],
        prefix_aliases: &[],
    },
    SeedMerchant {
        display_name: "Zelle",
        ambiguous: true,
        exact_aliases: &["ZELLE"],
        prefix_aliases: &[],
    },
    SeedMerchant {
        display_name: "Cash App",
        ambiguous: true,
        exact_aliases: &["CASH APP", "CASHAPP"],
        prefix_aliases: &[],
    },
];

/// Seed the canonical merchants + aliases of a NEW vault (ADR 0030 addendum), in the transaction
/// that creates it (personal-cfo-32fmp.3). Deterministic v5 ids and `created_at` = the vault's
/// creation time make the rows a function of the vault; `INSERT OR IGNORE` keeps a user's own alias
/// mapping for the same normalized key from being clobbered.
///
/// # Errors
/// Returns [`DbError`] if a SQLite operation fails.
pub(crate) fn seed_merchants(conn: &Connection, created_at: &str) -> Result<(), DbError> {
    let now = created_at;
    for m in SEED_MERCHANTS {
        let id = Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("merchant-seed:{}", m.display_name).as_bytes(),
        );
        conn.execute(
            "INSERT OR IGNORE INTO merchant_identities
                (id, display_name, default_category_id, is_ambiguous, source, created_at)
             VALUES (?1, ?2, NULL, ?3, 'seed', ?4)",
            params![id, m.display_name, i64::from(m.ambiguous), now],
        )?;
        for raw in m.exact_aliases {
            seed_alias(conn, raw, id, "exact", now)?;
        }
        for raw in m.prefix_aliases {
            seed_alias(conn, raw, id, "prefix", now)?;
        }
    }
    Ok(())
}

/// Insert one seed alias (normalized), `INSERT OR IGNORE` so a user mapping is never clobbered.
fn seed_alias(
    conn: &Connection,
    raw: &str,
    identity_id: Uuid,
    match_type: &str,
    now: &str,
) -> Result<(), DbError> {
    let key = normalize_merchant(raw);
    // A seed is static data, never over-wide; an over-wide key would only be skipped.
    if key.is_empty() || !alias_key_fits(&key) {
        return Ok(());
    }
    conn.execute(
        "INSERT OR IGNORE INTO merchant_aliases
            (normalized_key, merchant_identity_id, source, confidence_bps, match_type, created_at)
         VALUES (?1, ?2, 'seed', 10000, ?3, ?4)",
        params![key, identity_id, match_type, now],
    )?;
    Ok(())
}

/// A canonical merchant entity resolved from a normalized merchant key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerchantIdentity {
    /// Stable entity id.
    pub id: Uuid,
    /// Display name, e.g. `"Amazon"`.
    pub display_name: String,
    /// The entity's usual category when unambiguous — the auto-categorization seam. `None`
    /// for ambiguous merchants.
    pub default_category_id: Option<Uuid>,
    /// True for merchants that span categories (Amazon, Costco, Venmo): auto-categorization
    /// must defer to disambiguation / a split template rather than apply one category.
    pub is_ambiguous: bool,
    /// Provenance: `seed` | `user` | `auto`.
    pub source: String,
}

/// The longest `merchant_aliases.normalized_key`, in UTF-8 bytes. The key is the table's primary
/// key, and the canonical row encoding caps a primary key at 256 bytes (vault-format.md
/// C.1.5): a text key is a tag byte, a `u32` length and the bytes, so 256 - 5 = 251. A
/// `column_kinds` test pins this against the encoder.
pub const MAX_ALIAS_KEY_BYTES: usize = 251;

/// Whether `key` fits the replicated key width.
pub(crate) fn alias_key_fits(key: &str) -> bool {
    key.len() <= MAX_ALIAS_KEY_BYTES
}

/// How a normalized merchant key resolves to its canonical identity.
pub(crate) struct Resolved {
    /// The identity the key resolves to.
    pub(crate) identity: Uuid,
    /// Whether that identity spans categories (never auto-categorized).
    pub(crate) ambiguous: bool,
}

/// The alias index, loaded once: exact-key lookups plus prefix aliases for multi-location chains,
/// longest-first so a more specific brand token wins. The ONE resolver both planners and
/// [`resolve`] use, so the grouping pass and the memory pass cannot disagree about a key.
pub(crate) struct AliasIndex {
    exact: HashMap<String, Resolved>,
    prefixes: Vec<(String, Resolved)>,
}

impl AliasIndex {
    /// Read every alias (one row per alias, so small).
    pub(crate) fn load(conn: &Connection) -> Result<Self, DbError> {
        let mut exact: HashMap<String, Resolved> = HashMap::new();
        let mut prefixes: Vec<(String, Resolved)> = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT ma.normalized_key, ma.merchant_identity_id, mi.is_ambiguous, ma.match_type
             FROM merchant_aliases ma
             JOIN merchant_identities mi ON mi.id = ma.merchant_identity_id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Uuid>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        for row in rows {
            let (key, identity, ambiguous, match_type) = row?;
            let resolved = Resolved {
                identity,
                ambiguous: ambiguous != 0,
            };
            if match_type == "prefix" {
                prefixes.push((key, resolved));
            } else {
                exact.insert(key, resolved);
            }
        }
        // Longest prefix first, ties by key, so the most specific brand token wins and the order
        // never depends on row order.
        prefixes.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        Ok(Self { exact, prefixes })
    }

    /// Resolve a normalized key: an exact alias first, then a prefix alias the key begins with on a
    /// word boundary (`COSTCO` matches `COSTCO WHSE SEATTLE`, never `COSTCOX`).
    pub(crate) fn get(&self, normalized_key: &str) -> Option<&Resolved> {
        if let Some(found) = self.exact.get(normalized_key) {
            return Some(found);
        }
        self.prefixes
            .iter()
            .find(|(prefix, _)| {
                normalized_key.len() > prefix.len()
                    && normalized_key.starts_with(prefix.as_str())
                    && normalized_key.as_bytes()[prefix.len()] == b' '
            })
            .map(|(_, resolved)| resolved)
    }

    /// Resolve a key the way a reader of one alias asks: an alias whose own key is exactly this
    /// one resolves it whatever its match type (a prefix alias resolves its bare token), then the
    /// prefix rule of [`AliasIndex::get`]. [`resolve`] and the grouping planner use this; merchant
    /// memory groups with [`AliasIndex::get`], which does not treat a bare prefix token as a hit.
    pub(crate) fn lookup(&self, normalized_key: &str) -> Option<&Resolved> {
        if let Some(found) = self.exact.get(normalized_key) {
            return Some(found);
        }
        if let Some((_, found)) = self
            .prefixes
            .iter()
            .find(|(prefix, _)| prefix == normalized_key)
        {
            return Some(found);
        }
        self.get(normalized_key)
    }

    /// Whether any alias, exact or prefix, holds exactly this key.
    pub(crate) fn holds_key(&self, key: &str) -> bool {
        self.exact.contains_key(key) || self.prefixes.iter().any(|(prefix, _)| prefix == key)
    }
}

/// Resolve a normalized merchant key (the `normalize_merchant` output) to its canonical
/// identity: an exact alias first, then a **prefix** alias whose token the key begins with on
/// a word boundary (`COSTCO` resolves `COSTCO WHSE SEATTLE`). The longest matching prefix
/// wins, so a more specific brand token is preferred.
///
/// # Errors
/// Returns [`DbError`] if the read fails.
pub(crate) fn resolve(
    conn: &Connection,
    normalized_key: &str,
) -> Result<Option<MerchantIdentity>, DbError> {
    let index = AliasIndex::load(conn)?;
    let Some(found) = index.lookup(normalized_key) else {
        return Ok(None);
    };
    Ok(conn
        .query_row(
            "SELECT id, display_name, default_category_id, is_ambiguous, source
             FROM merchant_identities WHERE id = ?1",
            params![found.identity],
            |r| {
                Ok(MerchantIdentity {
                    id: r.get(0)?,
                    display_name: r.get(1)?,
                    default_category_id: r.get(2)?,
                    is_ambiguous: r.get::<_, i64>(3)? != 0,
                    source: r.get(4)?,
                })
            },
        )
        .optional()?)
}

/// Count of canonical merchant identities — the observability / test seam.
///
/// # Errors
/// Returns [`DbError`] if the read fails.
pub(crate) fn identity_count(conn: &Connection) -> Result<u64, DbError> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM merchant_identities", [], |r| r.get(0))?;
    Ok(u64::try_from(n).unwrap_or(0))
}
