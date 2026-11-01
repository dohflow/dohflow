//! The markers stored beside the Touch ID item (`personal-cfo-xuu` AC K4): non-secret, in the item's
//! public tag, readable and updatable without a prompt.
//!
//! ```text
//! offset  len  field
//!      0    1  version               = 1
//!      1   32  fingerprint           of the wrapped DEK in the vault's envelope when Touch ID was enabled
//!     33   16  boot id              of the boot in which the vault password was last entered
//!     49    8  password stamp       monotonic nanoseconds (little endian) of that password unlock, same boot
//!     57    1  failed prompts       cancelled or failed Touch ID prompts since the last password unlock
//! ```
//!
//! 58 bytes of the 64 a tag may hold. Anything that does not decode exactly is "unreadable", and an
//! unreadable marker set is never a reason to offer Touch ID: the caller asks for the vault password.

use platform_traits::{BootId, PublicTag};

pub(super) const VERSION: u8 = 1;
const LEN: usize = 1 + 32 + 16 + 8 + 1;

/// The decoded markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Markers {
    /// Fingerprint of the wrapped DEK this item was enabled against.
    pub(super) fingerprint: [u8; 32],
    /// The boot in which the vault password was last entered (the markers are re-stamped by it).
    pub(super) boot: BootId,
    /// `monotonic_ns` at that password unlock; comparable only within `boot`.
    pub(super) password_stamp_ns: u64,
    /// Cancelled or failed prompts since that password unlock.
    pub(super) failed_prompts: u8,
}

impl Markers {
    pub(super) fn encode(&self) -> PublicTag {
        let mut bytes = [0_u8; LEN];
        bytes[0] = VERSION;
        bytes[1..33].copy_from_slice(&self.fingerprint);
        bytes[33..49].copy_from_slice(self.boot.as_bytes());
        bytes[49..57].copy_from_slice(&self.password_stamp_ns.to_le_bytes());
        bytes[57] = self.failed_prompts;
        PublicTag::new(&bytes).expect("58 bytes is a valid tag")
    }

    /// `None` for anything that is not exactly a version-1 marker set.
    pub(super) fn decode(tag: &PublicTag) -> Option<Self> {
        let bytes = tag.as_bytes();
        if bytes.len() != LEN || bytes[0] != VERSION {
            return None;
        }
        let mut fingerprint = [0_u8; 32];
        fingerprint.copy_from_slice(&bytes[1..33]);
        let mut boot = [0_u8; 16];
        boot.copy_from_slice(&bytes[33..49]);
        let mut stamp = [0_u8; 8];
        stamp.copy_from_slice(&bytes[49..57]);
        Some(Self {
            fingerprint,
            boot: BootId::new(boot),
            password_stamp_ns: u64::from_le_bytes(stamp),
            failed_prompts: bytes[57],
        })
    }

    #[must_use]
    pub(super) fn with_failed_prompts(mut self, failed_prompts: u8) -> Self {
        self.failed_prompts = failed_prompts;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn markers() -> Markers {
        Markers {
            fingerprint: [0xAB; 32],
            boot: BootId::new([0x11; 16]),
            password_stamp_ns: 0x0102_0304_0506_0708,
            failed_prompts: 2,
        }
    }

    #[test]
    fn markers_round_trip_in_58_bytes() {
        let tag = markers().encode();
        assert_eq!(tag.as_bytes().len(), 58);
        assert_eq!(Markers::decode(&tag), Some(markers()));
    }

    #[test]
    fn the_layout_is_fixed() {
        let bytes = markers().encode();
        let bytes = bytes.as_bytes();
        assert_eq!(bytes[0], 1);
        assert_eq!(&bytes[1..33], &[0xAB; 32]);
        assert_eq!(&bytes[33..49], &[0x11; 16]);
        assert_eq!(&bytes[49..57], &0x0102_0304_0506_0708_u64.to_le_bytes());
        assert_eq!(bytes[57], 2);
    }

    #[test]
    fn anything_else_is_unreadable() {
        let good = markers().encode();
        let mut wrong_version = good.as_bytes().to_vec();
        wrong_version[0] = 2;
        for bad in [
            wrong_version,
            good.as_bytes()[..57].to_vec(),
            [good.as_bytes(), &[0_u8]].concat(),
            vec![1_u8],
        ] {
            let tag = PublicTag::new(&bad).unwrap();
            assert_eq!(Markers::decode(&tag), None, "{} bytes", bad.len());
        }
    }
}
