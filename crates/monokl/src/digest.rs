//! Algorithm-prefixed content digests, written `algorithm:hex` (the OCI descriptor digest grammar).

use std::fmt;
use std::str::FromStr;

/// The hash algorithm a [`Digest`] was computed with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DigestAlgorithm {
    Sha256,
    Blake3,
}

/// A 32-byte digest that names its algorithm, so a change of algorithm never collides with old values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Digest {
    pub algorithm: DigestAlgorithm,
    pub bytes: [u8; 32],
}

/// Why a string is not a valid [`Digest`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DigestParseError {
    #[error("digest has no `:` between algorithm and hex")]
    MissingSeparator,
    #[error("unknown digest algorithm {algorithm:?}")]
    UnknownAlgorithm { algorithm: String },
    #[error("digest hex must be 64 lowercase hex characters")]
    InvalidEncoding,
}

impl DigestAlgorithm {
    /// Returns the name used before the `:` in a digest's text form.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sha256 => "sha256",
            Self::Blake3 => "blake3",
        }
    }
}

impl FromStr for DigestAlgorithm {
    type Err = DigestParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "sha256" => Ok(Self::Sha256),
            "blake3" => Ok(Self::Blake3),
            other => Err(DigestParseError::UnknownAlgorithm { algorithm: other.to_string() }),
        }
    }
}

impl Digest {
    /// Hashes `content` with blake3.
    ///
    /// # Examples
    ///
    /// ```
    /// use monokl::digest::Digest;
    ///
    /// let d = Digest::blake3(b"");
    /// assert_eq!(d.to_string(), "blake3:af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262");
    /// ```
    #[must_use]
    pub fn blake3(content: &[u8]) -> Self {
        Self { algorithm: DigestAlgorithm::Blake3, bytes: *blake3::hash(content).as_bytes() }
    }
}

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use fmt::Write as _;
        f.write_str(self.algorithm.as_str())?;
        f.write_char(':')?;
        for byte in self.bytes {
            f.write_char(char::from(HEX_DIGITS[usize::from(byte >> 4)]))?;
            f.write_char(char::from(HEX_DIGITS[usize::from(byte & 0x0f)]))?;
        }
        Ok(())
    }
}

impl FromStr for Digest {
    type Err = DigestParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let Some((algorithm, encoded)) = s.split_once(':') else {
            return Err(DigestParseError::MissingSeparator);
        };
        let algorithm = algorithm.parse()?;
        let (pairs, []) = encoded.as_bytes().as_chunks::<2>() else {
            return Err(DigestParseError::InvalidEncoding);
        };
        if pairs.len() != 32 {
            return Err(DigestParseError::InvalidEncoding);
        }
        let mut bytes = [0u8; 32];
        for (byte, [high, low]) in bytes.iter_mut().zip(pairs) {
            *byte = (lowercase_hex_value(*high)? << 4) | lowercase_hex_value(*low)?;
        }
        Ok(Self { algorithm, bytes })
    }
}

/// Uppercase is rejected: OCI forbids it, and a digest whose case can vary breaks equality and hashing.
fn lowercase_hex_value(c: u8) -> Result<u8, DigestParseError> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        _ => Err(DigestParseError::InvalidEncoding),
    }
}

impl serde::Serialize for Digest {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for Digest {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use rstest::rstest;

    use super::*;

    const EMPTY_BLAKE3: &str = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
    const HEX64: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn blake3_of_empty_input_matches_the_published_digest() {
        let d = Digest::blake3(b"");
        assert_eq!(d, Digest { algorithm: DigestAlgorithm::Blake3, bytes: *blake3::hash(b"").as_bytes() });
        assert_eq!(d.to_string(), format!("blake3:{EMPTY_BLAKE3}"));
    }

    #[test]
    fn blake3_is_deterministic_and_input_sensitive() {
        assert_eq!(Digest::blake3(b"a"), Digest::blake3(b"a"));
        assert_ne!(Digest::blake3(b"a"), Digest::blake3(b"b"));
    }

    #[rstest]
    #[case(Digest { algorithm: DigestAlgorithm::Blake3, bytes: [0; 32] })]
    #[case(Digest { algorithm: DigestAlgorithm::Sha256, bytes: [0xff; 32] })]
    #[case(Digest::blake3(b"monokl"))]
    fn display_and_parse_round_trip(#[case] d: Digest) {
        assert_eq!(d.to_string().parse::<Digest>(), Ok(d));
    }

    #[rstest]
    #[case(Digest { algorithm: DigestAlgorithm::Blake3, bytes: [0xab; 32] }, "blake3")]
    #[case(Digest { algorithm: DigestAlgorithm::Sha256, bytes: [0xab; 32] }, "sha256")]
    fn display_is_algorithm_colon_lowercase_hex(#[case] d: Digest, #[case] algorithm: &str) {
        assert_eq!(d.to_string(), format!("{algorithm}:{}", "ab".repeat(32)));
    }

    #[test]
    fn bare_hex_without_separator_is_rejected() {
        assert_eq!(EMPTY_BLAKE3.parse::<Digest>(), Err(DigestParseError::MissingSeparator));
    }

    #[rstest]
    #[case("md5")]
    #[case("BLAKE3")]
    #[case("")]
    fn unknown_or_miscased_algorithm_is_rejected(#[case] algorithm: &str) {
        assert_eq!(
            format!("{algorithm}:{HEX64}").parse::<Digest>(),
            Err(DigestParseError::UnknownAlgorithm { algorithm: algorithm.to_string() })
        );
    }

    #[rstest]
    #[case(&HEX64[..63])]
    #[case(&format!("{HEX64}0"))]
    #[case("")]
    #[case(&format!("A{}", &HEX64[1..]))]
    #[case(&format!("g{}", &HEX64[1..]))]
    #[case(&format!("ab:{}", &HEX64[3..]))]
    fn malformed_hex_is_rejected(#[case] hex: &str) {
        assert_eq!(format!("blake3:{hex}").parse::<Digest>(), Err(DigestParseError::InvalidEncoding));
    }

    #[test]
    fn serde_uses_the_display_string() {
        let d = Digest::blake3(b"");
        let json = serde_json::to_string(&d).unwrap();
        assert_eq!(json, format!("\"blake3:{EMPTY_BLAKE3}\""));
        assert_eq!(serde_json::from_str::<Digest>(&json).unwrap(), d);
    }

    #[rstest]
    #[case(&format!("\"{EMPTY_BLAKE3}\""))]
    #[case(&format!("\"md5:{HEX64}\""))]
    #[case(&format!("\"blake3:{}\"", HEX64.to_uppercase()))]
    #[case("null")]
    #[case("42")]
    #[case("true")]
    #[case("[]")]
    #[case("{}")]
    fn deserialize_rejects_invalid_digests_and_non_strings(#[case] json: &str) {
        assert!(serde_json::from_str::<Digest>(json).is_err(), "accepted {json}");
    }

    #[test]
    fn equal_digests_dedupe_as_hash_keys() {
        let set: HashSet<Digest> = [Digest::blake3(b"x"), Digest::blake3(b"x"), Digest::blake3(b"y")].into();
        assert_eq!(set.len(), 2);
    }
}
