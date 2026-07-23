use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, OsRng, rand_core::RngCore},
};
use anyhow::{Context, bail};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use axum::http::{HeaderMap, header};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{Duration, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Settings;

pub const SESSION_COOKIE: &str = "ai_image_studio_session";

pub fn session_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    cookie::Cookie::split_parse(raw)
        .filter_map(Result::ok)
        .find(|cookie| cookie.name() == SESSION_COOKIE)
        .map(|cookie| cookie.value().to_owned())
}

pub async fn hash_password(password: SecretString) -> anyhow::Result<String> {
    tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut rand::thread_rng());
        Argon2::default()
            .hash_password(password.expose_secret().as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|error| anyhow::anyhow!("failed to hash password: {error}"))
    })
    .await
    .context("password hashing task failed")?
}

pub async fn verify_password(password: SecretString, encoded_hash: String) -> anyhow::Result<bool> {
    tokio::task::spawn_blocking(move || {
        let parsed = PasswordHash::new(&encoded_hash)
            .map_err(|error| anyhow::anyhow!("stored password hash is invalid: {error}"))?;
        Ok(Argon2::default()
            .verify_password(password.expose_secret().as_bytes(), &parsed)
            .is_ok())
    })
    .await
    .context("password verification task failed")?
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionClaims {
    pub sub: Uuid,
    pub session_version: i64,
    pub iat: usize,
    pub exp: usize,
}

#[derive(Clone)]
pub struct SessionManager {
    encoding: EncodingKey,
    decoding: DecodingKey,
    ttl_seconds: i64,
    secure_cookie: bool,
}

impl SessionManager {
    pub fn new(settings: &Settings) -> Self {
        let secret = settings.session_secret.expose_secret().as_bytes();
        Self {
            encoding: EncodingKey::from_secret(secret),
            decoding: DecodingKey::from_secret(secret),
            ttl_seconds: settings.session_ttl_seconds,
            secure_cookie: settings.session_cookie_secure,
        }
    }

    pub fn issue(&self, user_id: Uuid, session_version: i64) -> anyhow::Result<String> {
        let issued_at = Utc::now();
        let claims = SessionClaims {
            sub: user_id,
            session_version,
            iat: issued_at.timestamp() as usize,
            exp: (issued_at + Duration::seconds(self.ttl_seconds)).timestamp() as usize,
        };
        encode(&Header::default(), &claims, &self.encoding).context("failed to issue session")
    }

    pub fn decode(&self, token: &str) -> anyhow::Result<SessionClaims> {
        decode::<SessionClaims>(token, &self.decoding, &Validation::default())
            .map(|data| data.claims)
            .context("invalid session")
    }

    pub fn set_cookie_header(&self, token: String) -> String {
        cookie::Cookie::build((SESSION_COOKIE, token))
            .http_only(true)
            .secure(self.secure_cookie)
            .same_site(cookie::SameSite::Lax)
            .path("/")
            .max_age(cookie::time::Duration::seconds(self.ttl_seconds))
            .build()
            .to_string()
    }

    pub fn clear_cookie_header(&self) -> String {
        cookie::Cookie::build((SESSION_COOKIE, ""))
            .http_only(true)
            .secure(self.secure_cookie)
            .same_site(cookie::SameSite::Lax)
            .path("/")
            .max_age(cookie::time::Duration::ZERO)
            .build()
            .to_string()
    }
}

pub struct EncryptedCredential {
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
    pub key_version: i32,
}

pub struct CredentialCipher {
    cipher: Aes256Gcm,
}

impl CredentialCipher {
    pub fn new(encoded_key: &SecretString) -> anyhow::Result<Self> {
        let key = STANDARD
            .decode(encoded_key.expose_secret())
            .context("CREDENTIAL_MASTER_KEY must be base64")?;
        if key.len() != 32 {
            bail!("CREDENTIAL_MASTER_KEY must decode to exactly 32 bytes");
        }
        Ok(Self {
            cipher: Aes256Gcm::new_from_slice(&key).expect("validated AES-256 key length"),
        })
    }

    pub fn encrypt(&self, credential: &SecretString) -> anyhow::Result<EncryptedCredential> {
        let mut nonce = [0_u8; 12];
        OsRng.fill_bytes(&mut nonce);
        let ciphertext = self
            .cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                credential.expose_secret().as_bytes(),
            )
            .map_err(|_| anyhow::anyhow!("credential encryption failed"))?;
        Ok(EncryptedCredential {
            ciphertext,
            nonce: nonce.to_vec(),
            key_version: 1,
        })
    }

    pub fn decrypt(&self, ciphertext: &[u8], nonce: &[u8]) -> anyhow::Result<SecretString> {
        if nonce.len() != 12 {
            bail!("stored credential nonce is invalid");
        }
        let plaintext = self
            .cipher
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .map_err(|_| anyhow::anyhow!("credential decryption failed"))?;
        let value = String::from_utf8(plaintext).context("stored credential is not UTF-8")?;
        Ok(SecretString::from(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_manager(secure_cookie: bool) -> SessionManager {
        let secret = b"test-session-secret-at-least-32-characters";
        SessionManager {
            encoding: EncodingKey::from_secret(secret),
            decoding: DecodingKey::from_secret(secret),
            ttl_seconds: 3600,
            secure_cookie,
        }
    }

    #[test]
    fn credential_round_trip() {
        let key = SecretString::from(STANDARD.encode([7_u8; 32]));
        let cipher = CredentialCipher::new(&key).unwrap();
        let original = SecretString::from("test-secret".to_owned());
        let encrypted = cipher.encrypt(&original).unwrap();
        let decrypted = cipher
            .decrypt(&encrypted.ciphertext, &encrypted.nonce)
            .unwrap();
        assert_eq!(decrypted.expose_secret(), original.expose_secret());
    }

    #[test]
    fn session_cookie_attributes_follow_transport_security() {
        let development = session_manager(false).set_cookie_header("token".to_owned());
        assert!(development.contains("HttpOnly"));
        assert!(development.contains("SameSite=Lax"));
        assert!(development.contains("Path=/"));
        assert!(development.contains("Max-Age=3600"));
        assert!(!development.contains("; Secure"));

        let production = session_manager(true);
        let issued = production.set_cookie_header("token".to_owned());
        assert!(issued.contains("; Secure"));
        let cleared = production.clear_cookie_header();
        assert!(cleared.contains("HttpOnly"));
        assert!(cleared.contains("SameSite=Lax"));
        assert!(cleared.contains("Path=/"));
        assert!(cleared.contains("Max-Age=0"));
        assert!(cleared.contains("; Secure"));
    }
}
