//! Choosing the keychain access group from the app's own entitlement (xuu, AC D). Safe code with no
//! platform calls, so it is tested on every host; the entitlement read is in `keychain` (macOS only).

use platform_traits::AccessGroup;

/// The access group to use, from the values of the app's `keychain-access-groups` entitlement.
///
/// The FIRST value must be exactly `<TeamID>.<bundle_id>` (a 10-character uppercase team id, a dot, and
/// this app's bundle id). Anything else (no entitlement, an empty list, a wildcard, another app's group,
/// a malformed team id) is treated as ABSENT: `None`, which the caller turns into the typed
/// `Unavailable(MissingEntitlement)` path. The group is never guessed or defaulted, and no team id lives
/// in the source.
#[must_use]
pub fn access_group_from_entitlement(values: &[String], bundle_id: &str) -> Option<AccessGroup> {
    let first = values.first()?;
    let group = AccessGroup::new(first).ok()?;
    let (_, bundle) = first.split_once('.')?;
    (bundle == bundle_id).then_some(group)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUNDLE: &str = "ai.personalcfo.desktop";

    fn values(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_team_prefixed_group_for_this_bundle_is_accepted() {
        let group =
            access_group_from_entitlement(&values(&["ABCDE12345.ai.personalcfo.desktop"]), BUNDLE)
                .expect("a valid group");
        assert_eq!(group.as_str(), "ABCDE12345.ai.personalcfo.desktop");
    }

    #[test]
    fn only_the_first_value_counts() {
        let list = values(&["ABCDE12345.ai.personalcfo.desktop", "ZZZZZ99999.other.app"]);
        assert!(access_group_from_entitlement(&list, BUNDLE).is_some());
        let reversed = values(&["ZZZZZ99999.other.app", "ABCDE12345.ai.personalcfo.desktop"]);
        assert!(access_group_from_entitlement(&reversed, BUNDLE).is_none());
    }

    #[test]
    fn an_absent_or_malformed_entitlement_is_treated_as_absent() {
        for list in [
            values(&[]),
            values(&[""]),
            values(&["ai.personalcfo.desktop"]), // no team id
            values(&["abcde12345.ai.personalcfo.desktop"]), // lowercase team id
            values(&["ABCDE1234.ai.personalcfo.desktop"]), // 9 characters
            values(&["ABCDE123456.ai.personalcfo.desktop"]), // 11 characters
            values(&["ABCDE12345.*"]),           // a wildcard
            values(&["ABCDE12345."]),            // no bundle id
            values(&["ABCDE12345.ai.personalcfo.desktop.extra"]), // another bundle
            values(&["ABCDE12345.ai.other.app"]), // another app's group
            values(&["ABCDE12345.ai.personalcfo.desktop\n"]), // a control character
        ] {
            assert!(
                access_group_from_entitlement(&list, BUNDLE).is_none(),
                "accepted {list:?}"
            );
        }
    }
}
