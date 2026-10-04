//! Protocol version parsing and the major check.
use crate::types::ErrorCode;
use std::fmt;

/// The version this implementation writes.
pub const PROTOCOL_VERSION: &str = "0.1";
/// The one MAJOR this implementation accepts.
pub const SUPPORTED_MAJOR: u32 = 0;

/// `MAJOR.MINOR`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolVersion {
    pub major: u32,
    pub minor: u32,
}

impl ProtocolVersion {
    /// Parse `^[0-9]+\.[0-9]+$`. Anything else is `malformed_envelope`.
    pub fn parse(s: &str) -> Result<Self, ErrorCode> {
        let (a, b) = s.split_once('.').ok_or(ErrorCode::MalformedEnvelope)?;
        let digits = |x: &str| !x.is_empty() && x.bytes().all(|c| c.is_ascii_digit());
        if !digits(a) || !digits(b) {
            return Err(ErrorCode::MalformedEnvelope);
        }
        Ok(Self {
            major: a.parse().map_err(|_| ErrorCode::MalformedEnvelope)?,
            minor: b.parse().map_err(|_| ErrorCode::MalformedEnvelope)?,
        })
    }

    /// A receiver implements exactly one MAJOR; any other is `unsupported_version`.
    pub fn check_major(&self) -> Result<(), ErrorCode> {
        if self.major == SUPPORTED_MAJOR {
            Ok(())
        } else {
            Err(ErrorCode::UnsupportedVersion)
        }
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_ok_and_bad() {
        assert_eq!(
            ProtocolVersion::parse("0.1").unwrap(),
            ProtocolVersion { major: 0, minor: 1 }
        );
        for bad in ["", "1", "a.b", "1.", ".1", "1.2.3", "-1.0", "0.1 "] {
            assert_eq!(
                ProtocolVersion::parse(bad),
                Err(ErrorCode::MalformedEnvelope),
                "{bad}"
            );
        }
    }
    #[test]
    fn major_mismatch_rejected() {
        assert_eq!(
            ProtocolVersion::parse("2.0").unwrap().check_major(),
            Err(ErrorCode::UnsupportedVersion)
        );
        assert!(ProtocolVersion::parse("0.9").unwrap().check_major().is_ok());
    }
}
