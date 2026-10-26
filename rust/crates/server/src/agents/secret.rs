//! The secrets of the agents' address (`docs/plans/agents-from-anywhere.md`, §5):
//! each token and key is 32 random bytes, kept only as its SHA-256 and compared
//! on the hashes in constant time; a request's id is 128 random bits; the code
//! shown on both pages is six letters with no lookalikes.

/// The letters a code is made of: no two a person could read as each other
/// (no B/8, I/1/L, O/0, S/5, Z/2).
const LETTERS: &[u8] = b"ACDEFGHJKMNPQRTUVWXY";

fn random(n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    openssl::rand::rand_bytes(&mut b).expect("the system's random source answers");
    b
}

/// Bytes in the URL-safe base64 alphabet, unpadded (RFC 4648 §5).
pub fn b64url(bytes: &[u8]) -> String {
    openssl::base64::encode_block(bytes).trim_end_matches('=').replace('+', "-").replace('/', "_")
}

/// A new secret, as it is handed out once, beginning with `prefix` so a person
/// who finds one knows whose it is.
pub fn secret(prefix: &str) -> String {
    format!("{prefix}{}", b64url(&random(32)))
}

/// The SHA-256 of a secret as it was handed out: all that is kept of it.
pub fn hash(secret: &str) -> Vec<u8> {
    openssl::sha::sha256(secret.as_bytes()).to_vec()
}

/// Whether two hashes are equal, taking the same time whatever they hold.
pub fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && openssl::memcmp::eq(a, b)
}

/// A request's id: 128 random bits, in hex.
pub fn request_id() -> String {
    random(16).iter().map(|b| format!("{b:02x}")).collect()
}

/// The six letters shown on both pages.
pub fn code() -> String {
    // each byte taken only below the largest multiple of the alphabet's size, so
    // every letter is as likely as any other
    let limit = 256 - 256 % LETTERS.len();
    let mut out = String::new();
    while out.len() < 6 {
        for b in random(12) {
            if (b as usize) < limit && out.len() < 6 {
                out.push(LETTERS[b as usize % LETTERS.len()] as char);
            }
        }
    }
    out
}

/// A new id for a grant, a key or a registered client.
pub fn id(prefix: &str) -> String {
    format!("{prefix}{}", b64url(&random(12)))
}

/// Whether `verifier` answers `challenge` by S256 (RFC 7636 §4.6): the only
/// method taken.
pub fn pkce_ok(verifier: &str, challenge: &str) -> bool {
    // a verifier is 43 to 128 unreserved characters (§4.1)
    let ok = (43..=128).contains(&verifier.len()) && verifier.bytes().all(|c| c.is_ascii_alphanumeric() || b"-._~".contains(&c));
    ok && same(b64url(&openssl::sha::sha256(verifier.as_bytes())).as_bytes(), challenge.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s256_answers_its_verifier_alone() {
        // the challenge worked out apart from this code (Python's hashlib and base64)
        let challenge = "W15NQRG56e508qdyi6FrpLycmzoIRN-fBvEBGJkYnvE";
        assert!(pkce_ok("dBjftJeZ4CVP-mJ0z4ExTCJrd2Pvg5ZjMk86qGm-eJ8", challenge));
        assert!(!pkce_ok("dBjftJeZ4CVP-mJ0z4ExTCJrd2Pvg5ZjMk86qGm-eJ9", challenge));
        assert!(!pkce_ok("short", challenge));
    }

    #[test]
    fn a_code_is_six_letters_with_no_lookalikes() {
        for _ in 0..200 {
            let c = code();
            assert_eq!(c.len(), 6);
            assert!(c.bytes().all(|b| LETTERS.contains(&b)), "{c}");
        }
        assert_eq!(request_id().len(), 32);
        assert_ne!(secret("x_"), secret("x_"));
    }
}
