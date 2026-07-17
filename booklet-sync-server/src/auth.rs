//! Password hashing and device-token helpers.
//!
//! Passwords are argon2id (low-entropy, so a slow hash); device tokens are
//! high-entropy random strings, so a fast SHA-256 of the token is all the server
//! stores — it never keeps the token itself.

use argon2::password_hash::rand_core::{OsRng, RngCore};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use sha2::{Digest, Sha256};

/// Hashes a password for storage (argon2id with a random salt).
pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);

    Ok(Argon2::default().hash_password(password.as_bytes(), &salt)?.to_string())
}

/// Whether a password matches a stored hash. A malformed stored hash verifies as
/// false rather than erroring — an unusable credential is simply a failed login.
pub fn verify_password(password: &str, stored_hash: &str) -> bool {
    match PasswordHash::new(stored_hash) {
        Ok(parsed) => Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok(),
        Err(_) => false,
    }
}

/// A fresh device token: 32 random bytes, hex-encoded. Returned to the device
/// once; only its [`token_hash`] is stored.
pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);

    hex(&bytes)
}

/// The SHA-256 of a token, hex-encoded — what the server keeps and compares.
pub fn token_hash(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());

    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_verifies_against_its_own_hash_and_nothing_else() {
        let hash = hash_password("correct horse").unwrap();

        assert!(verify_password("correct horse", &hash));
        assert!(!verify_password("wrong", &hash));
        assert!(!verify_password("correct horse", "not a real hash"));
    }

    #[test]
    fn tokens_are_distinct_and_hash_stably() {
        let one = new_token();
        let two = new_token();

        assert_ne!(one, two);
        assert_eq!(one.len(), 64);
        assert_eq!(token_hash(&one), token_hash(&one));
        assert_ne!(token_hash(&one), token_hash(&two));
    }
}
