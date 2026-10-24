//! Algorithm-prefixed content digests, written `algorithm:hex` (the OCI descriptor digest grammar).

use std::fmt;
use std::str::FromStr;

/// The hash algorithm a [`Digest`] was computed with.
///
/// The set is closed, so a `match` needs no wildcard arm. Adding an algorithm is a breaking change.
///
/// # Examples
///
/// ```
/// use monokl::digest::DigestAlgorithm;
///
/// fn hex_len(algorithm: DigestAlgorithm) -> usize {
///     match algorithm {
///         DigestAlgorithm::Sha256 => 64,
///         DigestAlgorithm::Blake3 => 64,
///     }
/// }
/// assert_eq!(hex_len(DigestAlgorithm::Blake3), 64);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    ///
    /// # Examples
    ///
    /// ```
    /// use monokl::digest::DigestAlgorithm;
    ///
    /// assert_eq!(DigestAlgorithm::Blake3.as_str(), "blake3");
    /// assert_eq!(DigestAlgorithm::Sha256.as_str(), "sha256");
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sha256 => "sha256",
            Self::Blake3 => "blake3",
        }
    }
}

/// Parses exactly `sha256` or `blake3`; any other spelling, including a different case, is
/// [`DigestParseError::UnknownAlgorithm`].
///
/// # Examples
///
/// ```
/// use monokl::digest::{DigestAlgorithm, DigestParseError};
///
/// assert_eq!("blake3".parse(), Ok(DigestAlgorithm::Blake3));
/// assert_eq!(
///     "Blake3".parse::<DigestAlgorithm>(),
///     Err(DigestParseError::UnknownAlgorithm { algorithm: "Blake3".to_string() })
/// );
/// ```
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

/// Parses `algorithm:hex`, checking the separator, then the algorithm, then the hex. Nothing is trimmed.
///
/// # Examples
///
/// ```
/// use monokl::digest::{Digest, DigestParseError};
///
/// let text = "blake3:af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
/// assert_eq!(text.parse(), Ok(Digest::blake3(b"")));
/// assert_eq!("md5:zz".parse::<Digest>(), Err(DigestParseError::UnknownAlgorithm { algorithm: "md5".to_string() }));
/// ```
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
    use std::collections::{HashMap, HashSet};

    use proptest::prelude::*;
    use rstest::rstest;

    use super::*;

    const EMPTY_BLAKE3: &str = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
    const HEX64: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn ascending_bytes() -> [u8; 32] {
        std::array::from_fn(|i| u8::try_from(i).unwrap())
    }

    #[test]
    fn blake3_of_empty_input_matches_the_published_digest() {
        let d = Digest::blake3(b"");
        assert_eq!(d, Digest { algorithm: DigestAlgorithm::Blake3, bytes: *blake3::hash(b"").as_bytes() });
        assert_eq!(d.to_string(), format!("blake3:{EMPTY_BLAKE3}"));
    }

    /// The official BLAKE3 test vectors hash the bytes `i % 251` for `i` in `0..len`.
    #[rstest]
    #[case(0, "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262")]
    #[case(1, "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213")]
    #[case(1025, "d00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444")]
    fn blake3_matches_the_official_test_vectors(#[case] len: usize, #[case] hex: &str) {
        let input: Vec<u8> = (0..len).map(|i| u8::try_from(i % 251).unwrap()).collect();
        assert_eq!(Digest::blake3(&input).to_string(), format!("blake3:{hex}"));
    }

    #[test]
    fn blake3_is_deterministic_and_input_sensitive() {
        assert_eq!(Digest::blake3(b"a"), Digest::blake3(b"a"));
        assert_ne!(Digest::blake3(b"a"), Digest::blake3(b"b"));
    }

    #[rstest]
    fn display_and_parse_round_trip(
        #[values(DigestAlgorithm::Blake3, DigestAlgorithm::Sha256)] algorithm: DigestAlgorithm,
        #[values([0x00; 32], [0xff; 32], Digest::blake3(b"monokl").bytes, ascending_bytes())] bytes: [u8; 32],
    ) {
        let d = Digest { algorithm, bytes };
        assert_eq!(d.to_string().parse::<Digest>(), Ok(d));
    }

    proptest! {
        #[test]
        fn any_digest_round_trips_through_its_text_form(
            bytes in any::<[u8; 32]>(),
            algorithm in prop_oneof![Just(DigestAlgorithm::Blake3), Just(DigestAlgorithm::Sha256)],
        ) {
            let d = Digest { algorithm, bytes };
            prop_assert_eq!(d.to_string().parse::<Digest>(), Ok(d));
        }
    }

    #[rstest]
    #[case(Digest { algorithm: DigestAlgorithm::Blake3, bytes: [0xab; 32] }, "blake3")]
    #[case(Digest { algorithm: DigestAlgorithm::Sha256, bytes: [0xab; 32] }, "sha256")]
    fn display_is_algorithm_colon_lowercase_hex(#[case] d: Digest, #[case] algorithm: &str) {
        assert_eq!(d.to_string(), format!("{algorithm}:{}", "ab".repeat(32)));
    }

    #[rstest]
    #[case(DigestAlgorithm::Sha256, "sha256")]
    #[case(DigestAlgorithm::Blake3, "blake3")]
    fn algorithm_names_round_trip(#[case] algorithm: DigestAlgorithm, #[case] name: &str) {
        assert_eq!(algorithm.as_str(), name);
        assert_eq!(algorithm.as_str().parse::<DigestAlgorithm>(), Ok(algorithm));
    }

    #[test]
    fn algorithm_names_are_case_sensitive() {
        assert_eq!(
            "Blake3".parse::<DigestAlgorithm>(),
            Err(DigestParseError::UnknownAlgorithm { algorithm: "Blake3".to_string() })
        );
    }

    #[test]
    fn bare_hex_without_separator_is_rejected() {
        assert_eq!(EMPTY_BLAKE3.parse::<Digest>(), Err(DigestParseError::MissingSeparator));
    }

    #[rstest]
    #[case(String::new(), DigestParseError::MissingSeparator)]
    #[case(HEX64.to_string(), DigestParseError::MissingSeparator)]
    #[case("md5:zz".to_string(), DigestParseError::UnknownAlgorithm { algorithm: "md5".to_string() })]
    #[case(format!(":{HEX64}"), DigestParseError::UnknownAlgorithm { algorithm: String::new() })]
    #[case(format!(" blake3:{HEX64}"), DigestParseError::UnknownAlgorithm { algorithm: " blake3".to_string() })]
    #[case(format!("blake3:{HEX64}\n"), DigestParseError::InvalidEncoding)]
    fn parse_checks_separator_then_algorithm_then_encoding_without_trimming(
        #[case] input: String,
        #[case] expected: DigestParseError,
    ) {
        assert_eq!(input.parse::<Digest>(), Err(expected));
    }

    #[rstest]
    #[case("md5")]
    #[case("sha512")]
    #[case("BLAKE3")]
    #[case("Sha256")]
    fn unknown_or_miscased_algorithm_is_rejected(#[case] algorithm: &str) {
        assert_eq!(
            format!("{algorithm}:{HEX64}").parse::<Digest>(),
            Err(DigestParseError::UnknownAlgorithm { algorithm: algorithm.to_string() })
        );
    }

    #[rstest]
    fn malformed_hex_is_rejected(
        #[values("blake3", "sha256")] algorithm: &str,
        #[values(
            HEX64[..63].to_string(),
            format!("{HEX64}0"),
            String::new(),
            format!("A{}", &HEX64[1..]),
            format!("g{}", &HEX64[1..]),
            "\u{e9}".repeat(32),
            format!("ab:{}", &HEX64[3..]),
        )]
        hex: String,
    ) {
        assert_eq!(format!("{algorithm}:{hex}").parse::<Digest>(), Err(DigestParseError::InvalidEncoding));
    }

    #[test]
    fn serde_uses_the_display_string() {
        let d = Digest::blake3(b"");
        let literal = "\"blake3:af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262\"";
        assert_eq!(serde_json::to_string(&d).unwrap(), literal);
        assert_eq!(serde_json::from_str::<Digest>(literal).unwrap(), d);
        assert_eq!(serde_json::from_slice::<Digest>(literal.as_bytes()).unwrap(), d);
        assert_eq!(serde_json::from_reader::<_, Digest>(literal.as_bytes()).unwrap(), d);
        assert_eq!(serde_json::from_value::<Digest>(serde_json::to_value(d).unwrap()).unwrap(), d);
    }

    #[rstest]
    #[case(String::new(), DigestParseError::MissingSeparator)]
    #[case(EMPTY_BLAKE3.to_string(), DigestParseError::MissingSeparator)]
    #[case(format!("md5:{HEX64}"), DigestParseError::UnknownAlgorithm { algorithm: "md5".to_string() })]
    #[case(format!("BLAKE3:{HEX64}"), DigestParseError::UnknownAlgorithm { algorithm: "BLAKE3".to_string() })]
    #[case(format!("blake3:{}", &HEX64[..63]), DigestParseError::InvalidEncoding)]
    #[case(format!("blake3:{}", HEX64.to_uppercase()), DigestParseError::InvalidEncoding)]
    fn deserialize_reports_the_parse_error(#[case] text: String, #[case] expected: DigestParseError) {
        let json = serde_json::to_string(&text).unwrap();
        let err = serde_json::from_str::<Digest>(&json).unwrap_err();
        assert!(err.to_string().contains(&expected.to_string()), "{err} does not contain {expected}");
    }

    #[rstest]
    #[case("null")]
    #[case("1")]
    #[case("true")]
    #[case("[]")]
    #[case("{}")]
    fn deserialize_rejects_non_strings(#[case] json: &str) {
        assert!(serde_json::from_str::<Digest>(json).is_err(), "accepted {json}");
    }

    #[test]
    fn digest_keyed_map_round_trips_through_json() {
        let (a, b) = (Digest::blake3(b"a"), Digest::blake3(b"b"));
        let map = HashMap::from([(a, 1u32), (b, 2u32)]);
        let value = serde_json::to_value(&map).unwrap();
        let keys: HashSet<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, HashSet::from([a.to_string().as_str(), b.to_string().as_str()]));
        assert_eq!(serde_json::from_value::<HashMap<Digest, u32>>(value).unwrap(), map);
    }

    #[test]
    fn digest_keyed_map_rejects_an_invalid_key() {
        assert!(serde_json::from_str::<HashMap<Digest, u32>>(r#"{"deadbeef":1}"#).is_err());
    }

    #[test]
    fn same_bytes_under_different_algorithms_are_different_digests() {
        let blake3 = Digest { algorithm: DigestAlgorithm::Blake3, bytes: [0xab; 32] };
        let sha256 = Digest { algorithm: DigestAlgorithm::Sha256, bytes: [0xab; 32] };
        assert_ne!(blake3, sha256);
        assert_ne!(blake3.to_string(), sha256.to_string());
        assert_eq!(HashSet::from([blake3, sha256]).len(), 2);
    }

    #[test]
    fn equal_digests_dedupe_as_hash_keys() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<Digest>();
        assert_copy::<DigestAlgorithm>();
        let set: HashSet<Digest> = [Digest::blake3(b"x"), Digest::blake3(b"x"), Digest::blake3(b"y")].into();
        assert_eq!(set.len(), 2);
        let mut map = HashMap::new();
        map.insert(Digest::blake3(b"x"), 1);
        map.insert(Digest::blake3(b"x"), 2);
        assert_eq!(map.len(), 1);
        assert_eq!(map[&Digest::blake3(b"x")], 2);
    }
}
