//! Auto-fuzzy merchant grouping (personal-cfo-5n4.4, ADR 0030 addendum 2026-06-30).
//!
//! Discovers that several normalized merchant keys are the **same multi-location chain**
//! without a curated seed — `RITUAL COFFEE SF` and `RITUAL COFFEE OAKLAND` are one merchant,
//! `AMERICAN AIRLINES` and `AMERICAN EAGLE` are not. The design (an adversarial judge picked
//! it over three denylist variants that all had a single-industry-noun false-merge hole) is
//! **anchored containment**: never fuse two arbitrary keys on string similarity; instead
//! extract a brand by peeling a *lexicon-recognized* location tail, and only treat a brand as
//! a groupable anchor when it is distinctive (multi-token, or a single token ≥6 chars that is
//! neither a generic head nor a generic industry noun). False merges are the cardinal sin
//! (they cross-contaminate categorization), so precision is paramount and recall is bounded
//! by — and grows monotonically with — the city lexicon, at no precision cost.
//!
//! Pure + deterministic: no DB, no clock. The db-worker minting pass groups observed keys by
//! [`brand_anchor`] and links the brand as a `prefix` alias (personal-cfo-5n4.4 backend).

use std::collections::HashSet;
use std::sync::OnceLock;

/// Minimum length for a *single-token* brand to anchor — shorter single tokens are too
/// collision-prone (`SHELL`, `DELTA`) to group on alone.
const MIN_SINGLE_TOKEN_BRAND_LEN: usize = 6;

/// Generic leading words that can never solo-anchor a group (bank/airline/insurer heads):
/// `AMERICAN AIRLINES` vs `AMERICAN EAGLE` must not collapse to `AMERICAN`.
const GENERIC_HEADS: &[&str] = &[
    "AMERICAN",
    "UNITED",
    "DELTA",
    "FIRST",
    "NATIONAL",
    "REPUBLIC",
    "BLUE",
    "GENERAL",
    "NATIONWIDE",
    "LIBERTY",
    "MUTUAL",
    "PACIFIC",
    "ATLANTIC",
    "CONTINENTAL",
    "SOUTHWEST",
    "FRONTIER",
    "STANDARD",
    "PREMIER",
    "SUMMIT",
    "STAR",
    "SUN",
    "CROWN",
    "ROYAL",
    "GOLDEN",
    "SILVER",
    "GREEN",
    "CENTRAL",
    "EASTERN",
    "WESTERN",
    "NORTHERN",
    "SOUTHERN",
    "CITY",
    "STATE",
    "METRO",
    "CAPITAL",
    "COMMUNITY",
    "PEOPLES",
    "SECURITY",
    "TRUST",
    "SAVINGS",
    "SQUARE",
    "NEW",
    "NORTH",
    "SOUTH",
    "EAST",
    "WEST",
    "GRAND",
    "UNION",
];

/// Generic *industry* nouns — the false-merge hole the judge's traps exposed: a lone
/// `PIZZA OAKLAND` / `PIZZA DENVER` are different shops, so an industry noun may never
/// solo-anchor even at ≥6 chars. Extend as real descriptors surface (no precision risk).
const GENERIC_INDUSTRY: &[&str] = &[
    "PIZZA",
    "TACOS",
    "TACO",
    "BURGER",
    "BURGERS",
    "COFFEE",
    "DONUT",
    "DONUTS",
    "BAGEL",
    "BAGELS",
    "SUSHI",
    "NAILS",
    "LAUNDRY",
    "CLEANERS",
    "DELI",
    "WINGS",
    "GRILL",
    "CAFE",
    "KITCHEN",
    "BAKERY",
    "MARKET",
    "LIQUOR",
    "NUTRITION",
    "FITNESS",
    "DENTAL",
    "BARBER",
    "SALON",
    "AUTO",
    "FOODS",
    "MART",
    "EXPRESS",
    "DINER",
    "NOODLE",
    "RAMEN",
    "POKE",
    "JUICE",
    "SEAFOOD",
    "STEAKHOUSE",
    "BISTRO",
    "TAVERN",
    "BREWING",
    "BREWERY",
    "WINERY",
    "FLORIST",
];

/// Directional / venue tokens that are part of a trailing location, not a brand.
const AREA_TOKENS: &[&str] = &[
    "DOWNTOWN",
    "UPTOWN",
    "AIRPORT",
    "INTL",
    "INTERNATIONAL",
    "MALL",
    "CENTER",
    "CENTRE",
    "PLAZA",
    "TERMINAL",
    "OUTLET",
    "OUTLETS",
    "STATION",
    "STN",
    "MIDTOWN",
];

/// Metro abbreviations that appear as a trailing location token.
const METRO_ABBREVS: &[&str] = &["SF", "LA", "NYC", "DC", "ATL", "PHL", "PDX", "SLC", "DFW"];

/// Full US state names (single + multi-word) that can trail a location.
const US_STATES: &[&str] = &[
    "ALABAMA",
    "ALASKA",
    "ARIZONA",
    "ARKANSAS",
    "CALIFORNIA",
    "COLORADO",
    "CONNECTICUT",
    "DELAWARE",
    "FLORIDA",
    "GEORGIA",
    "HAWAII",
    "IDAHO",
    "ILLINOIS",
    "INDIANA",
    "IOWA",
    "KANSAS",
    "KENTUCKY",
    "LOUISIANA",
    "MAINE",
    "MARYLAND",
    "MASSACHUSETTS",
    "MICHIGAN",
    "MINNESOTA",
    "MISSISSIPPI",
    "MISSOURI",
    "MONTANA",
    "NEBRASKA",
    "NEVADA",
    "OHIO",
    "OKLAHOMA",
    "OREGON",
    "PENNSYLVANIA",
    "TENNESSEE",
    "TEXAS",
    "UTAH",
    "VERMONT",
    "VIRGINIA",
    "WASHINGTON",
    "WISCONSIN",
    "WYOMING",
];

/// A starter set of populous US cities (single-token), uppercased. Deliberately **pruned** of
/// tokens that collide with common brand words (`MOBILE`, `ORANGE`, `READING`, `HOLLYWOOD`,
/// `PARIS`, `JACKSON`, `SALEM`, `AURORA`) — those drop to safe misses, never false merges.
/// Growable at no precision cost (every added token is a genuine place).
const US_CITIES_SINGLE: &[&str] = &[
    "SEATTLE",
    "PORTLAND",
    "OAKLAND",
    "BERKELEY",
    "FREMONT",
    "DENVER",
    "BOULDER",
    "HOUSTON",
    "DALLAS",
    "AUSTIN",
    "CHICAGO",
    "MIAMI",
    "ATLANTA",
    "PHOENIX",
    "TUCSON",
    "SACRAMENTO",
    "FRESNO",
    "MODESTO",
    "STOCKTON",
    "OAKDALE",
    "PASADENA",
    "GLENDALE",
    "BURBANK",
    "IRVINE",
    "ANAHEIM",
    "OXNARD",
    "VENTURA",
    "TEMPE",
    "MESA",
    "CHANDLER",
    "SCOTTSDALE",
    "BOSTON",
    "CAMBRIDGE",
    "BROOKLYN",
    "QUEENS",
    "BRONX",
    "MANHATTAN",
    "PHILADELPHIA",
    "PITTSBURGH",
    "BALTIMORE",
    "RICHMOND",
    "CHARLOTTE",
    "RALEIGH",
    "DURHAM",
    "NASHVILLE",
    "MEMPHIS",
    "ORLANDO",
    "TAMPA",
    "JACKSONVILLE",
    "DETROIT",
    "CLEVELAND",
    "COLUMBUS",
    "CINCINNATI",
    "INDIANAPOLIS",
    "MILWAUKEE",
    "MINNEAPOLIS",
    "STPAUL",
    "KANSAS",
    "OMAHA",
    "TULSA",
    "ALBUQUERQUE",
    "ELPASO",
    "ARLINGTON",
    "PLANO",
    "FRISCO",
    "MCKINNEY",
    "DENTON",
    "WACO",
    "BELLEVUE",
    "REDMOND",
    "TACOMA",
    "SPOKANE",
    "EUGENE",
    "BEAVERTON",
    "HILLSBORO",
    "RENO",
    "HENDERSON",
    "BOISE",
    "OGDEN",
    "PROVO",
    "HONOLULU",
    "ANCHORAGE",
    "BUFFALO",
    "ROCHESTER",
    "SYRACUSE",
    "ALBANY",
    "HARTFORD",
    "STAMFORD",
    "NEWARK",
    "TRENTON",
    "PROVIDENCE",
    "WORCESTER",
    "SPRINGFIELD",
    "LOUISVILLE",
    "LEXINGTON",
    "BIRMINGHAM",
    "MONTGOMERY",
    "SAVANNAH",
    "AUGUSTA",
    "COLUMBIA",
    "GREENVILLE",
    "ASHEVILLE",
    "WICHITA",
    "TOPEKA",
    "LINCOLN",
    "MADISON",
    "GREENBAY",
    "DULUTH",
    "FARGO",
    "BILLINGS",
    "CHEYENNE",
];

/// Multi-token US cities (longest match wins so these peel before their constituent tokens).
const US_CITIES_MULTI: &[&[&str]] = &[
    &["SAN", "FRANCISCO"],
    &["SAN", "JOSE"],
    &["SAN", "DIEGO"],
    &["SAN", "ANTONIO"],
    &["LOS", "ANGELES"],
    &["LAS", "VEGAS"],
    &["SANTA", "CLARA"],
    &["SANTA", "MONICA"],
    &["SANTA", "ANA"],
    &["SANTA", "BARBARA"],
    &["PALO", "ALTO"],
    &["MOUNTAIN", "VIEW"],
    &["SALT", "LAKE", "CITY"],
    &["NEW", "YORK"],
    &["NEW", "ORLEANS"],
    &["SAN", "MATEO"],
    &["SAN", "RAFAEL"],
    &["FORT", "WORTH"],
    &["FORT", "LAUDERDALE"],
    &["COLORADO", "SPRINGS"],
    &["LONG", "BEACH"],
    &["VIRGINIA", "BEACH"],
    &["KANSAS", "CITY"],
    &["OKLAHOMA", "CITY"],
    &["DALY", "CITY"],
    &["REDWOOD", "CITY"],
    &["UNION", "CITY"],
    &["SOUTH", "SAN", "FRANCISCO"],
];

macro_rules! lazy_set {
    ($name:ident, $src:expr) => {
        fn $name() -> &'static HashSet<&'static str> {
            static S: OnceLock<HashSet<&'static str>> = OnceLock::new();
            S.get_or_init(|| $src.iter().copied().collect())
        }
    };
}

lazy_set!(generic_heads, GENERIC_HEADS);
lazy_set!(generic_industry, GENERIC_INDUSTRY);
lazy_set!(location_tokens_raw, AREA_TOKENS);
lazy_set!(metro_abbrevs, METRO_ABBREVS);
lazy_set!(us_states, US_STATES);
lazy_set!(us_cities_single, US_CITIES_SINGLE);

/// Is `token` a single-token location (city, state, metro abbrev, or area/venue word)?
fn is_location_token(token: &str) -> bool {
    us_cities_single().contains(token)
        || us_states().contains(token)
        || metro_abbrevs().contains(token)
        || location_tokens_raw().contains(token)
}

/// A bare 1–3 digit residue (store/lane number `normalize_merchant` did not strip — it only
/// removes runs of ≥4 digits). Never a brand token.
fn is_store_residue(token: &str) -> bool {
    !token.is_empty() && token.len() <= 3 && token.bytes().all(|b| b.is_ascii_digit())
}

/// Peel the brand spine from a normalized key: drop a trailing multi-token city, then any
/// trailing single-token location or 1–3 digit store residue, never peeling the last token.
/// Returns `(brand_tokens, peeled_anything)`.
fn peel(tokens: &[&str]) -> (Vec<String>, bool) {
    let mut t: Vec<&str> = tokens.to_vec();
    let mut peeled = false;

    // Longest multi-token city first (e.g. SAN FRANCISCO before a lone FRANCISCO).
    'outer: loop {
        for city in US_CITIES_MULTI {
            if t.len() > city.len() && t.ends_with(city) {
                t.truncate(t.len() - city.len());
                peeled = true;
                continue 'outer;
            }
        }
        break;
    }
    // Then trailing single-token locations / store residues, never the last token.
    while t.len() > 1 {
        let last = t[t.len() - 1];
        if is_location_token(last) || is_store_residue(last) {
            t.pop();
            peeled = true;
        } else {
            break;
        }
    }
    (t.iter().map(|s| (*s).to_owned()).collect(), peeled)
}

/// The brand-anchor of a normalized key: its location-peeled spine, but only when that spine
/// is **distinctive enough to group on** (multi-token, or a single token ≥6 chars that is not
/// a generic head or industry noun). `None` for un-anchorable keys (a bare industry noun, a
/// generic head, a too-short single token) — those never auto-group.
#[must_use]
pub fn brand_anchor(normalized_key: &str) -> Option<String> {
    let tokens: Vec<&str> = normalized_key.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }
    let (brand, _peeled) = peel(&tokens);
    if brand.is_empty() {
        return None;
    }
    if brand.len() == 1 {
        let only = &brand[0];
        let distinctive = only.len() >= MIN_SINGLE_TOKEN_BRAND_LEN
            && !generic_heads().contains(only.as_str())
            && !generic_industry().contains(only.as_str());
        if !distinctive {
            return None;
        }
    } else {
        // Multi-token: reject a fully-generic head phrase (e.g. a bare "BANK OF").
        let all_generic = brand
            .iter()
            .all(|tok| generic_heads().contains(tok.as_str()));
        if all_generic {
            return None;
        }
    }
    Some(brand.join(" "))
}

/// Do two normalized keys belong to the same merchant? True iff they share the *same* brand
/// anchor and at least one carried a peeled location tail (an exact-equal pair resolves
/// upstream, not here). This is the pure decision the precision fixture asserts.
#[must_use]
pub fn same_merchant(a: &str, b: &str) -> bool {
    match (brand_anchor(a), brand_anchor(b)) {
        (Some(ba), Some(bb)) => ba == bb && (ba != a || bb != b),
        _ => false,
    }
}

/// Minimum length of the SHORTER side before the truncation rule may fire — short prefixes
/// are far too collision-prone to treat as the same merchant.
const MIN_TRUNCATION_PREFIX_LEN: usize = 10;

/// Bank descriptors truncate mid-word (`SEVEN SEAS ROASTING C` for `SEVEN SEAS ROASTING
/// CO…`): do `a` and `b` differ only by such a truncation? Conservative by design (ADR 0047
/// §2.3): the shorter side must be **multi-token** and ≥ [`MIN_TRUNCATION_PREFIX_LEN`]
/// chars, and the longer side must extend it at a word boundary. Single tokens and short
/// stems never match (`NEW YORK LIFE` vs `NEW YORK PIZZA` differ at the boundary word, so
/// they never conflict here).
#[must_use]
pub fn truncation_variant(a: &str, b: &str) -> bool {
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    if short.len() < MIN_TRUNCATION_PREFIX_LEN
        || short == long
        || !short.contains(' ')
        || !long.starts_with(short)
    {
        return false;
    }
    // The remainder must continue at a word boundary: either the long side adds new
    // token(s) (" …") or finishes a truncated final token. The mid-token carve-out is
    // tightly bounded (adversarial review of 4d8.25.9): the short side's last token must
    // be a single LETTER (a hard bank truncation — digits are store/unit residues, and a
    // legit section/lot letter key like "PARKING LOT A" must not become a wildcard) and
    // the remainder may only COMPLETE that token briefly (≤2 chars, no new tokens):
    // "…ROASTING C" ↔ "…ROASTING CO" bridges; "PARKING LOT A" ↔ "PARKING LOT AIRPORT"
    // never does.
    let remainder = &long[short.len()..];
    let hard_truncation_completion = remainder.len() <= 2
        && !remainder.contains(' ')
        && short
            .rsplit(' ')
            .next()
            .is_some_and(|last| last.len() == 1 && last.chars().all(|c| c.is_ascii_alphabetic()));
    remainder.starts_with(' ') || hard_truncation_completion
}

/// The ADR 0047 §2 series-consumption test: do two normalized merchant keys refer to the
/// same tracked series? Exact match, anchored same-merchant grouping, or a conservative
/// truncation variant. Used to consume candidates against tracked bills — never for
/// dismissal suppression (ADR 0046 stays exact-keyed).
#[must_use]
pub fn keys_conflict(a: &str, b: &str) -> bool {
    a == b || same_merchant(a, b) || truncation_variant(a, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The judge's precision fixture (personal-cfo-5n4.4): every POSITIVE pair groups, and —
    /// the cardinal rule — every NEGATIVE pair must NOT group, including the single-industry
    /// noun + city traps that disqualified the denylist approaches.
    #[test]
    fn grouping_precision_fixture() {
        // POSITIVES — same merchant, different locations.
        for (a, b) in [
            ("RITUAL COFFEE SF", "RITUAL COFFEE OAKLAND"),
            ("TRADER JOE'S PORTLAND", "TRADER JOE'S SEATTLE"),
            ("SHELL OIL HOUSTON", "SHELL OIL DALLAS"),
            ("7-ELEVEN SAN JOSE", "7-ELEVEN OAKLAND"),
            ("STARBUCKS SAN FRANCISCO", "STARBUCKS SEATTLE"),
            ("CHIPOTLE DENVER", "CHIPOTLE BOULDER"),
            ("CHIPOTLE 12 DENVER", "CHIPOTLE 7 BOULDER"),
        ] {
            assert!(same_merchant(a, b), "should group: {a:?} :: {b:?}");
        }

        // NEGATIVES — different merchants. A single false merge here is a release blocker.
        for (a, b) in [
            ("AMERICAN AIRLINES", "AMERICAN EAGLE"),
            ("BANK OF AMERICA", "BANK OF THE WEST"),
            ("FIRST REPUBLIC BANK", "FIRST NATIONAL BANK"),
            ("BLUE BOTTLE COFFEE", "BLUE STAR DONUTS"),
            ("UNITED AIRLINES", "UNITED HEALTHCARE"),
            ("DELTA AIR LINES", "DELTA DENTAL"),
            ("APPLE STORE PALO ALTO", "APPLEBEE'S DENVER"),
            ("SQUARE SPACE", "SQUARE ENIX"),
            // Added traps — the discriminators.
            ("PIZZA OAKLAND", "PIZZA DENVER"),
            ("TACOS AUSTIN", "TACOS DALLAS"),
            ("BURGER FRESNO", "BURGER MODESTO"),
            ("NAILS FREMONT", "NAILS BERKELEY"),
            ("JACKSON HEWITT MEMPHIS", "JACKSON NATIONAL DENVER"),
            ("NORTH FACE BERKELEY", "NORTH ITALIA BERKELEY"),
            ("TARGET FIELD MINNEAPOLIS", "TARGET OPTICAL MINNEAPOLIS"),
            ("NEW YORK LIFE", "NEW YORK PIZZA"),
            ("CHASE BANK MIAMI", "CHASE FIELD MIAMI"),
        ] {
            assert!(!same_merchant(a, b), "must NOT group: {a:?} :: {b:?}");
        }
    }

    /// ADR 0047 §2.3: the truncation rule bridges bank-truncated descriptors without
    /// opening the false-merge hole the 5n4.4 judge fixture guards.
    #[test]
    fn truncation_variants_conflict_but_distinct_merchants_never_do() {
        // POSITIVES — the same series across truncation depths.
        for (a, b) in [
            ("SEVEN SEAS ROASTING", "SEVEN SEAS ROASTING C"),
            ("SEVEN SEAS ROASTING C", "SEVEN SEAS ROASTING CO"),
            ("BLUE BOTTLE COFFEE", "BLUE BOTTLE COFFEE COMPANY"),
            ("CRUNCHYROLL MEMBERSHIP", "CRUNCHYROLL MEMBERSHIP RENEWAL"),
        ] {
            assert!(keys_conflict(a, b), "should conflict: {a:?} :: {b:?}");
            assert!(keys_conflict(b, a), "symmetry: {b:?} :: {a:?}");
        }
        // NEGATIVES — prefixes that are NOT the same merchant.
        for (a, b) in [
            ("NEW YORK LIFE", "NEW YORK PIZZA"), // differs at the boundary word
            ("STARBUCKS", "STARBUCKS RESERVE ROASTERY"), // single-token short side
            ("NETFLIX", "NETFLIX GAMES"),        // single-token short side
            ("TACO BELL", "TACO BELL CANTINA"),  // short side < 10 chars
            ("AMERICAN AIR", "AMERICAN AIRLINES CARGO"), // mid-token growth without a truncated tail
            // A legit single-LETTER tail must not become a prefix wildcard: the
            // carve-out only accepts a brief completion (<=2 chars, no new tokens).
            ("PARKING LOT A", "PARKING LOT AIRPORT"),
            // Digit tails are store/unit residues, never hard truncations.
            ("TERMINAL LOT 2", "TERMINAL LOT 2B"),
        ] {
            assert!(!keys_conflict(a, b), "must NOT conflict: {a:?} :: {b:?}");
        }
        // Location variants still conflict via the anchored-grouping arm.
        assert!(keys_conflict("RITUAL COFFEE SF", "RITUAL COFFEE OAKLAND"));
        // Exact equality conflicts trivially.
        assert!(keys_conflict("SPOTIFY", "SPOTIFY"));
    }

    #[test]
    fn brand_anchor_extracts_spine_and_rejects_ungroupable() {
        // Distinctive brands keep their location-peeled spine.
        assert_eq!(
            brand_anchor("STARBUCKS SEATTLE").as_deref(),
            Some("STARBUCKS")
        );
        assert_eq!(
            brand_anchor("RITUAL COFFEE SAN FRANCISCO").as_deref(),
            Some("RITUAL COFFEE")
        );
        // Un-anchorable: a bare industry noun, and a generic head + brand-ish tail (no peel,
        // but AIRLINES is not generic so the multi-token spine is kept — it simply never
        // matches AMERICAN EAGLE because the remainders differ).
        assert!(
            brand_anchor("PIZZA OAKLAND").is_none(),
            "industry-noun spine"
        );
        assert_ne!(
            brand_anchor("AMERICAN AIRLINES"),
            brand_anchor("AMERICAN EAGLE"),
            "distinct American-* brands never share an anchor"
        );
        // A bare brand with no location tail is not itself a grouping signal here.
        assert!(!same_merchant("STARBUCKS", "STARBUCKS"));
    }
}
