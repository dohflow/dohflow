//! The data-protection keychain, through `SecItemAdd`, `SecItemCopyMatching` and `SecItemDelete`.
//!
//! Every dictionary key is set explicitly here so it can be audited in one place; nothing is left to
//! a wrapper's defaults. **This is the crate's only `unsafe`.** The entitled paths (a store that
//! succeeds) cannot run in an unsigned build: they are exercised by the `#[ignore]`d tests in
//! `tests/keychain.rs`, run from the signed throwaway harness bundle of `personal-cfo-md8h2.18` (the
//! release signing change itself is `personal-cfo-md8h2.17`). The unentitled path (a typed failure) runs in every test.
#![allow(unsafe_code)]

use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::data::CFData;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::CFString;
use core_foundation_sys::base::{kCFAllocatorDefault, CFRelease, CFTypeRef};
use platform_traits::{
    AccessGroup, AccessPolicy, AuthPrompt, Roaming, SecretError, SecretKey, SecretStore,
    SecretValue, StoreCapabilities,
};
use security_framework_sys::access_control::{
    kSecAccessControlBiometryCurrentSet, kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly,
    SecAccessControlCreateWithFlags,
};
use security_framework_sys::item::{
    kSecAttrAccessControl, kSecAttrAccessGroup, kSecAttrAccount, kSecAttrService,
    kSecAttrSynchronizable, kSecClass, kSecClassGenericPassword, kSecMatchLimit,
    kSecReturnAttributes, kSecReturnData, kSecUseAuthenticationContext,
    kSecUseDataProtectionKeychain, kSecValueData,
};
use security_framework_sys::keychain_item::{SecItemAdd, SecItemCopyMatching, SecItemDelete};
use zeroize::Zeroizing;

use crate::auth;
use crate::status::{map_os_status, ERR_SEC_ITEM_NOT_FOUND};

// Two constants the sys crate does not export; both are CFString constants of the Security framework.
extern "C" {
    /// The attribute key `kSecAttrAccessible` (the accessibility class).
    static kSecAttrAccessible: core_foundation_sys::string::CFStringRef;
    /// The `kSecMatchLimit` value "return one item".
    static kSecMatchLimitOne: core_foundation_sys::string::CFStringRef;
}

/// The attributes the platform reports for a stored item: what the release gate and the
/// entitled test assert (device-only class, not synchronizable, and where the item lives).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAttributes {
    /// `kSecAttrAccessible` as reported (the class name string), if the platform reports one.
    pub accessible: Option<String>,
    /// `kSecAttrSynchronizable` as reported, if present.
    pub synchronizable: Option<bool>,
    /// `kSecAttrAccessGroup` as reported.
    pub access_group: Option<String>,
}

/// A [`SecretStore`] on the data-protection keychain, with every item in `group`.
pub struct MacSecretStore {
    group: AccessGroup,
}

impl MacSecretStore {
    #[must_use]
    pub fn new(group: AccessGroup) -> Self {
        Self { group }
    }

    fn base_query(&self, key: &SecretKey) -> Vec<(CFString, CFType)> {
        // SAFETY: each `k*` is a process-lifetime CFString constant owned by the Security framework;
        // `wrap_under_get_rule` retains it.
        unsafe {
            vec![
                (
                    CFString::wrap_under_get_rule(kSecClass),
                    CFString::wrap_under_get_rule(kSecClassGenericPassword).as_CFType(),
                ),
                (
                    CFString::wrap_under_get_rule(kSecAttrService),
                    CFString::new(key.service()).as_CFType(),
                ),
                (
                    CFString::wrap_under_get_rule(kSecAttrAccount),
                    CFString::new(key.account()).as_CFType(),
                ),
                (
                    CFString::wrap_under_get_rule(kSecAttrAccessGroup),
                    CFString::new(self.group.as_str()).as_CFType(),
                ),
                (
                    CFString::wrap_under_get_rule(kSecUseDataProtectionKeychain),
                    CFBoolean::true_value().as_CFType(),
                ),
                (
                    CFString::wrap_under_get_rule(kSecAttrSynchronizable),
                    CFBoolean::false_value().as_CFType(),
                ),
            ]
        }
    }

    /// Reads back the attributes of a stored item (no secret data, no prompt).
    ///
    /// # Errors
    /// [`SecretError::NotFound`], [`SecretError::Unavailable`] (unentitled) or
    /// [`SecretError::Platform`].
    pub fn stored_attributes(&self, key: &SecretKey) -> Result<StoredAttributes, SecretError> {
        let mut query = self.base_query(key);
        // SAFETY: constants as above.
        unsafe {
            query.push((
                CFString::wrap_under_get_rule(kSecReturnAttributes),
                CFBoolean::true_value().as_CFType(),
            ));
            query.push((
                CFString::wrap_under_get_rule(kSecMatchLimit),
                CFString::wrap_under_get_rule(kSecMatchLimitOne).as_CFType(),
            ));
        }
        let result = copy_matching(&query)?;
        // SAFETY: with kSecReturnAttributes and limit one, the result is a CFDictionary.
        let dict: CFDictionary<CFString, CFType> =
            unsafe { CFDictionary::wrap_under_create_rule(result as CFDictionaryRef) };
        let get = |key_const: core_foundation_sys::string::CFStringRef| -> Option<CFType> {
            // SAFETY: constant as above.
            let name = unsafe { CFString::wrap_under_get_rule(key_const) };
            dict.find(&name).map(|v| (*v).clone())
        };
        let text = |value: Option<CFType>| {
            value
                .and_then(|v| v.downcast::<CFString>())
                .map(|s| s.to_string())
        };
        // SAFETY: reading the address of Security framework CFString constants.
        let (accessible_key, sync_key, group_key) = unsafe {
            (
                kSecAttrAccessible,
                kSecAttrSynchronizable,
                kSecAttrAccessGroup,
            )
        };
        let accessible = text(get(accessible_key));
        let synchronizable = get(sync_key).and_then(|v| {
            v.downcast::<CFBoolean>().map(bool::from).or_else(|| {
                v.downcast::<core_foundation::number::CFNumber>()
                    .and_then(|n| n.to_i32())
                    .map(|n| n != 0)
            })
        });
        let access_group = text(get(group_key));
        Ok(StoredAttributes {
            accessible,
            synchronizable,
            access_group,
        })
    }
}

fn copy_matching(query: &[(CFString, CFType)]) -> Result<CFTypeRef, SecretError> {
    let dict = CFDictionary::from_CFType_pairs(query);
    let mut result: CFTypeRef = std::ptr::null();
    // SAFETY: `dict` is a valid CFDictionary for the call; `result` is a valid out-pointer that the
    // framework fills (a +1 reference we take ownership of) only on success.
    let status = unsafe { SecItemCopyMatching(dict.as_concrete_TypeRef(), &mut result) };
    if status == 0 && !result.is_null() {
        Ok(result)
    } else {
        Err(map_os_status(if status == 0 {
            ERR_SEC_ITEM_NOT_FOUND
        } else {
            status
        }))
    }
}

impl SecretStore for MacSecretStore {
    fn store_secret(
        &self,
        key: &SecretKey,
        value: &SecretValue,
        policy: AccessPolicy,
    ) -> Result<(), SecretError> {
        // Replace semantics: remove any existing item first (an access control cannot be changed by
        // an update), then add. A missing item is fine.
        self.delete_secret(key)?;

        // A policy this build does not know (the enum is non-exhaustive) is refused, never stored with
        // some other protection.
        let biometric = match policy {
            AccessPolicy::DeviceOnly => false,
            AccessPolicy::DeviceOnlyBiometryCurrentSet => true,
            _ => {
                return Err(SecretError::Unavailable(
                    platform_traits::UnavailableReason::Unsupported,
                ))
            }
        };

        let mut attrs = self.base_query(key);
        // SAFETY: constants as above; the access control object is created and owned here.
        unsafe {
            attrs.push((
                CFString::wrap_under_get_rule(kSecValueData),
                CFData::from_buffer(value.expose()).as_CFType(),
            ));
            if biometric {
                // The access control carries both the device-only class and the biometric constraint
                // (`kSecAttrAccessible` and `kSecAttrAccessControl` cannot be given together).
                let mut error: core_foundation_sys::error::CFErrorRef = std::ptr::null_mut();
                let control = SecAccessControlCreateWithFlags(
                    kCFAllocatorDefault,
                    kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly as CFTypeRef,
                    kSecAccessControlBiometryCurrentSet,
                    &mut error,
                );
                if control.is_null() {
                    if !error.is_null() {
                        CFRelease(error as CFTypeRef);
                    }
                    // The framework refuses this combination when no passcode is set.
                    return Err(SecretError::Unavailable(
                        platform_traits::UnavailableReason::NoPasscode,
                    ));
                }
                // `control` is a +1 reference: `wrap_under_create_rule` takes ownership.
                attrs.push((
                    CFString::wrap_under_get_rule(kSecAttrAccessControl),
                    CFType::wrap_under_create_rule(control as CFTypeRef),
                ));
            } else {
                attrs.push((
                    CFString::wrap_under_get_rule(kSecAttrAccessible),
                    CFString::wrap_under_get_rule(kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly)
                        .as_CFType(),
                ));
            }
        }
        let dict = CFDictionary::from_CFType_pairs(&attrs);
        // SAFETY: `dict` is valid for the call; a null result pointer means "return nothing".
        let status = unsafe { SecItemAdd(dict.as_concrete_TypeRef(), std::ptr::null_mut()) };
        if status == 0 {
            Ok(())
        } else {
            Err(map_os_status(status))
        }
    }

    fn load_secret(
        &self,
        key: &SecretKey,
        prompt: &AuthPrompt,
    ) -> Result<SecretValue, SecretError> {
        let mut query = self.base_query(key);
        let context = auth::authentication_context(prompt);
        // SAFETY: constants as above; `context` is a retained LAContext used as a CFType.
        unsafe {
            query.push((
                CFString::wrap_under_get_rule(kSecReturnData),
                CFBoolean::true_value().as_CFType(),
            ));
            query.push((
                CFString::wrap_under_get_rule(kSecMatchLimit),
                CFString::wrap_under_get_rule(kSecMatchLimitOne).as_CFType(),
            ));
            query.push((
                CFString::wrap_under_get_rule(kSecUseAuthenticationContext),
                context,
            ));
        }
        let result = copy_matching(&query)?;
        // SAFETY: with kSecReturnData and limit one, the result is a CFData (+1, we own it).
        let data = unsafe {
            CFData::wrap_under_create_rule(result as core_foundation_sys::data::CFDataRef)
        };
        // Copy into an exactly sized zeroizing buffer immediately; the framework's own copy is not
        // ours to wipe (ADR 0002 addendum, decision 7).
        let copy = Zeroizing::new(data.bytes().to_vec());
        SecretValue::new(&copy)
    }

    fn delete_secret(&self, key: &SecretKey) -> Result<(), SecretError> {
        let query = self.base_query(key);
        let dict = CFDictionary::from_CFType_pairs(&query);
        // SAFETY: `dict` is valid for the call.
        let status = unsafe { SecItemDelete(dict.as_concrete_TypeRef()) };
        match status {
            0 | ERR_SEC_ITEM_NOT_FOUND => Ok(()),
            other => Err(map_os_status(other)),
        }
    }

    fn capabilities(&self) -> StoreCapabilities {
        StoreCapabilities {
            device_only_class: true,
            biometry_current_set: true,
            // Not synchronizable and device-only: it does not move to another device. Whether the
            // SAME device can be rolled back is a different fact, and a keychain is never the witness.
            roams_to_other_device: Roaming::Never,
            rollback_resistant: false,
        }
    }
}
