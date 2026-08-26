//! PKCE (RFC 7636) and state nonce generation. NEROA_OAUTH_LOOPBACK_V11.

use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};

/// A high-entropy code verifier: 32 random bytes, base64url without padding.
pub fn verifier() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// S256 challenge: base64url(sha256(verifier)), no padding.
pub fn challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

/// Opaque state nonce, checked on the redirect to reject injected codes.
pub fn nonce() -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_is_deterministic_for_a_verifier() {
        // The RFC 7636 appendix B test vector.
        let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(challenge(v), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn verifier_and_nonce_are_unique() {
        assert_ne!(verifier(), verifier());
        assert_ne!(nonce(), nonce());
    }
}
