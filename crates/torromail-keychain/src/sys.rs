//! The only `unsafe` in the workspace. Three things no safe binding covers:
//! building the team-scoped access list (the legacy `SecAccess`/`SecACL`
//! API), adding an item with that list attached, and reading this process's
//! own Team ID. Each wraps what it gets back under the matching Core
//! Foundation ownership rule before anything else touches it, so nothing
//! leaks and nothing is released twice.

use std::ffi::c_void;
use std::ptr;

use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFType, OSStatus, TCFType};
use core_foundation::data::CFData;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::propertylist::{create_data, kCFPropertyListXMLFormat_v1_0};
use core_foundation::string::{CFString, CFStringRef};
use security_framework::os::macos::access::SecAccess;
use security_framework_sys::base::SecAccessRef;
use security_framework_sys::code_signing::{SecCSFlags, SecCodeCopySelf, SecCodeRef, SecStaticCodeRef};
use security_framework_sys::item::{kSecAttrAccount, kSecAttrService, kSecClass, kSecClassGenericPassword, kSecValueData};
use security_framework_sys::keychain_item::SecItemAdd;

type SecACLRef = *const c_void;
/// `SecKeychainPromptSelector`, a `uint16_t`.
type PromptSelector = u16;

/// `kSecCSSigningInformation`: ask `SecCodeCopySigningInformation` for the
/// signature's details, the Team ID among them.
const SIGNING_INFORMATION: SecCSFlags = 1 << 1;

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecAttrAccess: CFStringRef;
    static kSecCodeInfoTeamIdentifier: CFStringRef;

    fn SecAccessCreate(descriptor: CFStringRef, trusted_list: CFArrayRef, access: *mut SecAccessRef) -> OSStatus;
    fn SecAccessCopyACLList(access: SecAccessRef, acl_list: *mut CFArrayRef) -> OSStatus;
    fn SecACLCopyAuthorizations(acl: SecACLRef) -> CFArrayRef;
    fn SecACLCopyContents(
        acl: SecACLRef,
        application_list: *mut CFArrayRef,
        description: *mut CFStringRef,
        prompt_selector: *mut PromptSelector,
    ) -> OSStatus;
    fn SecACLSetContents(
        acl: SecACLRef,
        application_list: CFArrayRef,
        description: CFStringRef,
        prompt_selector: PromptSelector,
    ) -> OSStatus;
    fn SecCodeCopyStaticCode(code: SecCodeRef, flags: SecCSFlags, static_code: *mut SecStaticCodeRef) -> OSStatus;
    fn SecCodeCopySigningInformation(code: SecStaticCodeRef, flags: SecCSFlags, information: *mut CFDictionaryRef)
    -> OSStatus;
}

/// Releases a Core Foundation object this module was handed under the create
/// rule and has no typed wrapper for.
fn release(object: *const c_void) {
    if !object.is_null() {
        // SAFETY: `object` came from a Copy/Create call, so this module owns
        // exactly one reference, and it is not used afterwards.
        drop(unsafe { CFType::wrap_under_create_rule(object) });
    }
}

/// Adds a generic password, with the team-scoped access list when there is a
/// team. Answers the raw `OSStatus`; the caller knows what a duplicate means.
pub(crate) fn add_generic_password(service: &str, account: &str, secret: &[u8], team: Option<&str>) -> OSStatus {
    // SAFETY: the `kSec…` keys are immutable constants the framework exports.
    let key = |constant: CFStringRef| unsafe { CFString::wrap_under_get_rule(constant) };
    let mut pairs: Vec<(CFString, CFType)> = vec![
        (key(unsafe { kSecClass }), key(unsafe { kSecClassGenericPassword }).into_CFType()),
        (key(unsafe { kSecAttrService }), CFString::new(service).into_CFType()),
        (key(unsafe { kSecAttrAccount }), CFString::new(account).into_CFType()),
        (key(unsafe { kSecValueData }), CFData::from_buffer(secret).into_CFType()),
    ];
    if let Some(access) = team.and_then(|team| team_scoped_access(service, team)) {
        pairs.push((key(unsafe { kSecAttrAccess }), access.into_CFType()));
    }
    let attributes = CFDictionary::from_CFType_pairs(&pairs);
    // SAFETY: `attributes` is a valid dictionary for the duration of the
    // call; a null result pointer asks for nothing back.
    unsafe { SecItemAdd(attributes.as_concrete_TypeRef(), ptr::null_mut()) }
}

/// The partition list as `SecACLSetContents` wants it for the partition ACL:
/// an XML property list `{"Partitions": ["teamid:<team>"]}`, hex-encoded into
/// the description string.
pub(crate) fn partition_description(team: &str) -> String {
    let partitions = CFArray::from_CFTypes(&[CFString::new(&format!("teamid:{team}"))]);
    let list = CFDictionary::from_CFType_pairs(&[(CFString::from_static_string("Partitions"), partitions.into_CFType())]);
    let Ok(xml) = create_data(list.as_CFTypeRef(), kCFPropertyListXMLFormat_v1_0) else {
        return String::new();
    };
    xml.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// An access list that grants read and write to any binary signed with
/// `team` and nothing else: the application list of the decrypt and encrypt
/// ACLs is emptied ("any application"), and the partition ACL is pinned to
/// the team. `None` when the framework refuses to build one; the item is then
/// stored with the default list.
fn team_scoped_access(service: &str, team: &str) -> Option<SecAccess> {
    let descriptor = CFString::new(service);
    let nobody: CFArray<CFType> = CFArray::from_CFTypes(&[]);
    let mut raw_access: SecAccessRef = ptr::null_mut();
    // SAFETY: valid string and array in, an out pointer this frame owns.
    let status = unsafe { SecAccessCreate(descriptor.as_concrete_TypeRef(), nobody.as_concrete_TypeRef(), &mut raw_access) };
    if status != 0 || raw_access.is_null() {
        return None;
    }
    // SAFETY: SecAccessCreate hands over one reference.
    let access = unsafe { SecAccess::wrap_under_create_rule(raw_access) };

    let mut raw_list: CFArrayRef = ptr::null();
    // SAFETY: `access` is alive; the out pointer is this frame's.
    if unsafe { SecAccessCopyACLList(access.as_concrete_TypeRef(), &mut raw_list) } != 0 || raw_list.is_null() {
        return None;
    }
    // SAFETY: a Copy call hands over one reference.
    let acls: CFArray<CFType> = unsafe { CFArray::wrap_under_create_rule(raw_list) };
    let partitions = CFString::new(&partition_description(team));

    for acl in acls.iter() {
        let acl: SecACLRef = acl.as_CFTypeRef();
        let authorizations = authorizations(acl);
        let mut applications: CFArrayRef = ptr::null();
        let mut description: CFStringRef = ptr::null();
        let mut prompt: PromptSelector = 0;
        // SAFETY: `acl` lives as long as `acls`; the out pointers are ours
        // and whatever they receive is released below.
        let copied = unsafe { SecACLCopyContents(acl, &mut applications, &mut description, &mut prompt) };
        if copied != 0 {
            continue;
        }
        let label = if description.is_null() { descriptor.as_concrete_TypeRef() } else { description };
        if authorizations.iter().any(|name| name == "ACLAuthorizationDecrypt" || name == "ACLAuthorizationEncrypt") {
            // SAFETY: a null application list means "any application".
            unsafe { SecACLSetContents(acl, ptr::null(), label, 0) };
        }
        if authorizations.iter().any(|name| name == "ACLAuthorizationPartitionID") {
            // SAFETY: all arguments are alive for the call.
            unsafe { SecACLSetContents(acl, applications, partitions.as_concrete_TypeRef(), prompt) };
        }
        release(applications.cast());
        release(description.cast());
    }
    Some(access)
}

/// The authorization tags of one ACL, as strings.
fn authorizations(acl: SecACLRef) -> Vec<String> {
    // SAFETY: `acl` is alive; the call returns an owned array or null.
    let raw = unsafe { SecACLCopyAuthorizations(acl) };
    if raw.is_null() {
        return Vec::new();
    }
    // SAFETY: a Copy call hands over one reference.
    let names: CFArray<CFType> = unsafe { CFArray::wrap_under_create_rule(raw) };
    names.iter().filter_map(|name| name.downcast::<CFString>()).map(|name| name.to_string()).collect()
}

pub(crate) fn own_team_identifier() -> Option<String> {
    let mut code: SecCodeRef = ptr::null_mut();
    // SAFETY: an out pointer this frame owns.
    if unsafe { SecCodeCopySelf(0, &mut code) } != 0 || code.is_null() {
        return None;
    }
    let mut static_code: SecStaticCodeRef = ptr::null_mut();
    // SAFETY: `code` is the reference just handed over.
    let status = unsafe { SecCodeCopyStaticCode(code, 0, &mut static_code) };
    release(code.cast_const().cast());
    if status != 0 || static_code.is_null() {
        return None;
    }
    let mut raw_information: CFDictionaryRef = ptr::null();
    // SAFETY: `static_code` is the reference just handed over.
    let status = unsafe { SecCodeCopySigningInformation(static_code, SIGNING_INFORMATION, &mut raw_information) };
    release(static_code.cast_const().cast());
    if status != 0 || raw_information.is_null() {
        return None;
    }
    // SAFETY: a Copy call hands over one reference.
    let information: CFDictionary<CFString, CFType> = unsafe { CFDictionary::wrap_under_create_rule(raw_information) };
    // SAFETY: an immutable constant the framework exports.
    let key = unsafe { CFString::wrap_under_get_rule(kSecCodeInfoTeamIdentifier) };
    information.find(&key).and_then(|team| team.downcast::<CFString>()).map(|team| team.to_string())
}
