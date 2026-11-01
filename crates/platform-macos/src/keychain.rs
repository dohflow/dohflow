//! The data-protection keychain, through `SecItemAdd`, `SecItemCopyMatching` and `SecItemDelete`.
//!
//! Every dictionary key is set explicitly here so it can be audited in one place; nothing is left to
//! a wrapper's defaults. **This is the crate's only `unsafe`.** The entitled paths (a store that
//! succeeds) cannot run in an unsigned build: they are exercised by the `#[ignore]`d tests in
//! `tests/keychain.rs`, run from the signed throwaway harness bundle of `personal-cfo-md8h2.18` (the
//! release signing change itself is `personal-cfo-md8h2.17`). The unentitled path (a typed failure) runs in every test.
#![allow(unsafe_code)]

use core_foundation::array::CFArray;
use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::data::CFData;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::CFString;
use core_foundation_sys::base::{kCFAllocatorDefault, CFRelease, CFTypeRef};
use platform_traits::{
    AccessGroup, AccessPolicy, AuthPrompt, PublicTag, Roaming, SecretError, SecretKey, SecretStore,
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
// `SecTask` reads the entitlements of a running process; the app asks about itself.
extern "C" {
    /// The attribute key `kSecAttrGeneric` (a small caller-defined, unprotected data attribute).
    static kSecAttrGeneric: core_foundation_sys::string::CFStringRef;
    fn SecTaskCreateFromSelf(
        allocator: core_foundation_sys::base::CFAllocatorRef,
    ) -> *const std::ffi::c_void;
    fn SecTaskCopyValueForEntitlement(
        task: *const std::ffi::c_void,
        entitlement: core_foundation_sys::string::CFStringRef,
        error: *mut core_foundation_sys::error::CFErrorRef,
    ) -> CFTypeRef;
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
    /// The item's [`PublicTag`] (`kSecAttrGeneric`), if it was stored with one.
    pub public_tag: Option<PublicTag>,
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
        // SAFETY: reading the address of a Security framework CFString constant.
        let public_tag = get(unsafe { kSecAttrGeneric })
            .and_then(|v| v.downcast::<CFData>())
            .and_then(|data| PublicTag::new(data.bytes()).ok());
        Ok(StoredAttributes {
            accessible,
            synchronizable,
            access_group,
            public_tag,
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
    fn store_secret_tagged(
        &self,
        key: &SecretKey,
        value: &SecretValue,
        policy: AccessPolicy,
        tag: Option<&PublicTag>,
    ) -> Result<(), SecretError> {
        // Everything that can be refused is validated and built BEFORE the existing item is touched,
        // so a refused store (an unknown policy, an access control the framework will not create)
        // never destroys the previous secret (review F2 on the first version of this crate).
        //
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

        // A biometric item is refused up front, with the typed reason for the device's CURRENT state,
        // when this Mac cannot create one (measured: with nothing enrolled the OS answers AuthFailed
        // at the add, which a caller cannot tell from a wrong finger). Nothing is replaced.
        if biometric {
            if let Some(refusal) = auth::current_availability().store_refusal() {
                return Err(refusal);
            }
        }

        let mut attrs = self.base_query(key);
        // SAFETY: constants as above; the access control object is created and owned here.
        unsafe {
            attrs.push((
                CFString::wrap_under_get_rule(kSecValueData),
                CFData::from_buffer(value.expose()).as_CFType(),
            ));
            if let Some(tag) = tag {
                // Not protected by the item's access control: readable without a prompt (`public_tag`).
                attrs.push((
                    CFString::wrap_under_get_rule(kSecAttrGeneric),
                    CFData::from_buffer(tag.as_bytes()).as_CFType(),
                ));
            }
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
                    // [U] Mapped to NoPasscode on the documented reading that the framework refuses
                    // this combination when no passcode is set; the error is not inspected, so other
                    // causes would be reported the same way. Unverified until run on a signed build.
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
        // Replace semantics: only now remove any existing item (an access control cannot be changed
        // by an update), then add. A missing item is fine. If the add itself then fails, no item
        // remains: a failed store MAY leave no item (documented on the trait).
        self.delete_secret(key)?;
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

    fn public_tag(&self, key: &SecretKey) -> Result<Option<PublicTag>, SecretError> {
        self.stored_attributes(key).map(|attrs| attrs.public_tag)
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

/// The keychain access group this app is entitled to use, read from its OWN code signature, or `None`
/// when it has none that is valid for `bundle_id` (an unsigned or ad-hoc build, a missing or malformed
/// entitlement, another app's group). `None` is the typed `Unavailable(MissingEntitlement)` path for
/// every keychain call: no team id lives in the source and nothing is guessed (xuu, AC D).
#[must_use]
pub fn own_access_group(bundle_id: &str) -> Option<AccessGroup> {
    let values = own_keychain_access_groups()?;
    crate::group::access_group_from_entitlement(&values, bundle_id)
}

/// The values of this process's `keychain-access-groups` entitlement, if it has the entitlement.
fn own_keychain_access_groups() -> Option<Vec<String>> {
    // SAFETY: `SecTaskCreateFromSelf` returns a +1 SecTask or null; it is released below.
    let task = unsafe { SecTaskCreateFromSelf(kCFAllocatorDefault) };
    if task.is_null() {
        return None;
    }
    let entitlement = CFString::new("keychain-access-groups");
    let mut error: core_foundation_sys::error::CFErrorRef = std::ptr::null_mut();
    // SAFETY: `task` is a live SecTask, the entitlement name is a live CFString, `error` is a valid
    // out-pointer; the result (if any) is a +1 CFType and a non-null error is a +1 CFError.
    let value = unsafe {
        SecTaskCopyValueForEntitlement(task, entitlement.as_concrete_TypeRef(), &mut error)
    };
    // SAFETY: both were returned at +1 (or null) by the calls above.
    unsafe {
        CFRelease(task as CFTypeRef);
        if !error.is_null() {
            CFRelease(error as CFTypeRef);
        }
    }
    if value.is_null() {
        return None;
    }
    // SAFETY: `value` is a +1 CFType we now own.
    let value = unsafe { CFType::wrap_under_create_rule(value) };
    let array = value.downcast_into::<CFArray<*const std::ffi::c_void>>()?;
    Some(
        array
            .iter()
            .filter_map(|item| {
                let pointer = *item;
                // SAFETY: the array's items are CF objects; the type id is checked before the pointer is
                // treated as a CFString, and `wrap_under_get_rule` retains it.
                let is_string = unsafe { core_foundation_sys::base::CFGetTypeID(pointer) }
                    == CFString::type_id();
                is_string.then(|| {
                    unsafe {
                        CFString::wrap_under_get_rule(
                            pointer as core_foundation_sys::string::CFStringRef,
                        )
                    }
                    .to_string()
                })
            })
            .collect(),
    )
}
