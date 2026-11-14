use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};

pub fn hash(password: &str) -> Result<String, String> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|hashed| hashed.to_string())
        .map_err(|error| error.to_string())
}

pub fn verify(password: &str, stored: &str) -> bool {
    match PasswordHash::new(stored) {
        Ok(parsed) => Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok(),
        Err(error) => {
            eprintln!("auth: a stored password hash is unreadable: {error}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hash_verifies_its_password_and_no_other() {
        let stored = hash("correct horse").unwrap();
        assert!(verify("correct horse", &stored));
        assert!(!verify("correct horsf", &stored));
        assert!(!verify("correct horse", "not a hash"));
        assert_ne!(hash("correct horse").unwrap(), stored, "every hash has its own salt");
    }
}
