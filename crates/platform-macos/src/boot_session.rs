//! Parsing `kern.bootsessionuuid` (safe code with no platform calls, so it is tested on every host; the
//! `sysctl` read is in `clock`).
//!
//! The boot identity is the per-boot random UUID the kernel generates once at start-up. Unlike
//! `kern.boottime`, which the kernel shifts whenever the wall clock is stepped (`clock_set_calendar_microtime`),
//! nothing a user or NTP does moves it, so it changes on a restart and only on a restart.

use platform_traits::{BootClockError, BootId};

/// Parses the textual UUID (`8-4-4-4-12` hexadecimal digits, as `sysctl` prints it) into 16 bytes.
/// Anything else is an error, so a caller fails closed: wrong length, a missing or misplaced hyphen, a
/// non-hex character, an all-zero value, or text after the UUID.
///
/// # Errors
/// [`BootClockError`] for any value that is not exactly one non-zero UUID.
pub fn parse_boot_session_uuid(text: &str) -> Result<BootId, BootClockError> {
    let text = text.strip_suffix('\0').unwrap_or(text);
    let bytes = text.as_bytes();
    if bytes.len() != 36 {
        return Err(BootClockError);
    }
    let mut out = [0_u8; 16];
    let mut index = 0;
    let mut position = 0;
    while position < 36 {
        if matches!(position, 8 | 13 | 18 | 23) {
            if bytes[position] != b'-' {
                return Err(BootClockError);
            }
            position += 1;
            continue;
        }
        let high = hex(bytes[position])?;
        let low = hex(bytes[position + 1])?;
        // The hyphen positions are 8, 13, 18 and 23: a pair never straddles one.
        if matches!(position + 1, 8 | 13 | 18 | 23) {
            return Err(BootClockError);
        }
        out[index] = (high << 4) | low;
        index += 1;
        position += 2;
    }
    if out == [0_u8; 16] {
        return Err(BootClockError);
    }
    Ok(BootId::new(out))
}

fn hex(byte: u8) -> Result<u8, BootClockError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(BootClockError),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_uuid_parses_to_its_sixteen_bytes_in_either_case() {
        let expected = [
            0x7F, 0xC3, 0x98, 0xBE, 0x46, 0xAF, 0x4B, 0xE6, 0x9E, 0xCA, 0xE3, 0xEC, 0x6C, 0xD9,
            0x8A, 0x2E,
        ];
        let upper = parse_boot_session_uuid("7FC398BE-46AF-4BE6-9ECA-E3EC6CD98A2E").unwrap();
        let lower = parse_boot_session_uuid("7fc398be-46af-4be6-9eca-e3ec6cd98a2e").unwrap();
        assert_eq!(upper.as_bytes(), &expected);
        assert_eq!(lower, upper);
    }

    #[test]
    fn a_trailing_nul_from_the_sysctl_buffer_is_accepted() {
        assert!(parse_boot_session_uuid("7FC398BE-46AF-4BE6-9ECA-E3EC6CD98A2E\0").is_ok());
    }

    #[test]
    fn two_different_boots_are_two_different_ids() {
        let a = parse_boot_session_uuid("7FC398BE-46AF-4BE6-9ECA-E3EC6CD98A2E").unwrap();
        let b = parse_boot_session_uuid("7FC398BE-46AF-4BE6-9ECA-E3EC6CD98A2F").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn a_malformed_value_fails_closed() {
        for bad in [
            "",
            "7FC398BE-46AF-4BE6-9ECA-E3EC6CD98A2", // one digit short
            "7FC398BE-46AF-4BE6-9ECA-E3EC6CD98A2E0", // one digit long
            "7FC398BE46AF4BE69ECAE3EC6CD98A2E",    // no hyphens
            "7FC398BE-46AF-4BE6-9ECA-E3EC6CD98A2G", // a non-hex digit
            "7FC398BE-46AF-4BE6-9ECA_E3EC6CD98A2E", // a wrong separator
            "7FC398B-E46AF-4BE6-9ECA-E3EC6CD98A2E", // a misplaced hyphen
            "{7FC398BE-46AF-4BE6-9ECA-E3EC6CD98A2E}",
            "7FC398BE-46AF-4BE6-9ECA-E3EC6CD98A2E\n",
            " 7FC398BE-46AF-4BE6-9ECA-E3EC6CD98A2E",
            "00000000-0000-0000-0000-000000000000", // an all-zero id is "unset", not a boot
            "ＦＣ398BE-46AF-4BE6-9ECA-E3EC6CD98A2E", // wide characters
        ] {
            assert_eq!(parse_boot_session_uuid(bad), Err(BootClockError), "{bad:?}");
        }
    }
}
