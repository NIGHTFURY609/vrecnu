/// A capability token gating access to the one recording being published.
///
/// **Single-*session*, not single-*request*.** AGENTS.md §5 says "one-time capability token",
/// which reads as per-request until you notice the recording endpoint is necessarily fetched
/// many times — every Range window of the upload, plus the `<video>` preview. One capability,
/// scoped to one recording and one publish session, invalidated on completion, cancellation
/// or timeout. ADR-006 records this reading explicitly so a later reviewer does not file it
/// as a violation.
#[derive(Clone)]
pub struct CapabilityToken {
    value: String,
}

impl CapabilityToken {
    /// Wraps an already-generated token value.
    ///
    /// Generation lives with the caller so the randomness source stays in one place; 256 bits
    /// from the OS CSPRNG, base64url-encoded, is the intended shape.
    #[must_use]
    pub fn from_value(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }

    /// The token as it appears in a URL.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Constant-time comparison against a candidate.
    ///
    /// A local page cannot read the token, but it *can* probe. Comparing with `==` would leak
    /// the matching prefix length through timing and turn an unguessable 256-bit secret into
    /// a byte-at-a-time search.
    #[must_use]
    pub fn matches(&self, candidate: &str) -> bool {
        let expected = self.value.as_bytes();
        let actual = candidate.as_bytes();
        // Fold the length difference into the result rather than returning early on it.
        let mut diff = (expected.len() ^ actual.len()) as u8;
        for i in 0..expected.len().max(actual.len()) {
            let a = expected.get(i).copied().unwrap_or(0);
            let b = actual.get(i).copied().unwrap_or(0);
            diff |= a ^ b;
        }
        diff == 0
    }
}

impl std::fmt::Debug for CapabilityToken {
    /// Redacted. A token in a log or a panic message is a token that has escaped.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CapabilityToken(<redacted>)")
    }
}

// Note: no `Drop` scrub. Zeroing a `String`'s buffer needs `unsafe`, and this crate forbids
// it; a correct scrub also has to survive reallocation, which hand-rolled code does not. When
// the server lands in M5, use the `zeroize` crate and hold the value in a `Zeroizing<String>`
// rather than approximating it here.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_matches_itself() {
        let token = CapabilityToken::from_value("abc123");
        assert!(token.matches("abc123"));
    }

    #[test]
    fn a_different_token_does_not_match() {
        let token = CapabilityToken::from_value("abc123");
        assert!(!token.matches("abc124"));
    }

    #[test]
    fn a_prefix_does_not_match() {
        // The case a length-check-then-compare would get right by accident and a naive
        // byte loop would get wrong.
        let token = CapabilityToken::from_value("abc123");
        assert!(!token.matches("abc"));
        assert!(!token.matches("abc1234"));
    }

    #[test]
    fn an_empty_candidate_does_not_match() {
        assert!(!CapabilityToken::from_value("abc123").matches(""));
    }

    #[test]
    fn debug_never_prints_the_token() {
        // A token in a log line has escaped, and logs get pasted into issue reports.
        let token = CapabilityToken::from_value("super-secret-value");
        let rendered = format!("{token:?}");
        assert!(!rendered.contains("super-secret-value"), "{rendered}");
        assert_eq!(rendered, "CapabilityToken(<redacted>)");
    }
}
