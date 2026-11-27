//! Web Push (RFC 8030/8291/8292) to the browsers that turned notifications
//! on, the same way SemmelTV's TV app sends its own.

use crate::db::{Db, Subscription};
use base64::Engine;
use base64::alphabet::URL_SAFE;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{self, EcdsaKeyPair, KeyPair};
use ring::{aead, agreement, hkdf};
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OnceCell;

const KEY_SETTING: &str = "push_key";
const RECORD_SIZE: u32 = 4096;
const KEPT_BY_THE_PUSH_SERVICE: u64 = 6 * 60 * 60;

const LENIENT: GeneralPurpose = GeneralPurpose::new(
    &URL_SAFE,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Notice {
    pub title: String,
    pub body: String,
    pub url: String,
    pub tag: String,
}

struct Vapid {
    pair: EcdsaKeyPair,
    subject: String,
}

impl Vapid {
    fn new(pkcs8: &[u8], subject: &str) -> Result<Self, String> {
        let pair = EcdsaKeyPair::from_pkcs8(&signature::ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8, &SystemRandom::new())
            .map_err(|error| format!("push key: {error}"))?;
        Ok(Self {
            pair,
            subject: subject.to_string(),
        })
    }

    fn generate() -> Result<Vec<u8>, String> {
        EcdsaKeyPair::generate_pkcs8(&signature::ECDSA_P256_SHA256_FIXED_SIGNING, &SystemRandom::new())
            .map(|document| document.as_ref().to_vec())
            .map_err(|_| "push key: could not make one".to_string())
    }

    fn public_key(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.pair.public_key().as_ref())
    }

    fn authorization(&self, endpoint: &str, now: u64) -> Result<String, String> {
        let url = url::Url::parse(endpoint).map_err(|error| format!("{endpoint}: {error}"))?;
        let header = URL_SAFE_NO_PAD.encode(r#"{"typ":"JWT","alg":"ES256"}"#);
        let claims = serde_json::json!({
            "aud": url.origin().ascii_serialization(),
            "exp": now + KEPT_BY_THE_PUSH_SERVICE,
            "sub": self.subject,
        });
        let claims = URL_SAFE_NO_PAD.encode(claims.to_string());
        let signed = format!("{header}.{claims}");
        let signature = self
            .pair
            .sign(&SystemRandom::new(), signed.as_bytes())
            .map_err(|_| "push key: could not sign".to_string())?;
        Ok(format!(
            "vapid t={signed}.{}, k={}",
            URL_SAFE_NO_PAD.encode(signature.as_ref()),
            self.public_key()
        ))
    }
}

struct Length(usize);

impl hkdf::KeyType for Length {
    fn len(&self) -> usize {
        self.0
    }
}

fn expand(prk: &hkdf::Prk, info: &[u8], length: usize) -> Vec<u8> {
    let mut out = vec![0; length];
    let info = [info];
    prk.expand(&info, Length(length))
        .and_then(|okm| okm.fill(&mut out))
        .expect("HKDF-SHA256 gives up to 8160 bytes");
    out
}

/// The `aes128gcm` body RFC 8291 sends to one browser: a header with the salt
/// and a one-off public key, then the message in a single record.
fn encrypt(plain: &[u8], to: &Subscription, ephemeral: &dyn SecureRandom, salt: [u8; 16]) -> Result<Vec<u8>, String> {
    let theirs = LENIENT.decode(&to.p256dh).map_err(|error| format!("p256dh: {error}"))?;
    let auth = LENIENT.decode(&to.auth).map_err(|error| format!("auth: {error}"))?;
    let private = agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, ephemeral)
        .map_err(|_| "push: no key for this message".to_string())?;
    let ours = private
        .compute_public_key()
        .map_err(|_| "push: no key for this message".to_string())?
        .as_ref()
        .to_vec();
    let shared = agreement::agree_ephemeral(
        private,
        &agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, &theirs),
        <[u8]>::to_vec,
    )
    .map_err(|_| "push: the browser's key is not a P-256 point".to_string())?;
    let mut info = b"WebPush: info\0".to_vec();
    info.extend_from_slice(&theirs);
    info.extend_from_slice(&ours);
    let ikm = expand(&hkdf::Salt::new(hkdf::HKDF_SHA256, &auth).extract(&shared), &info, 32);
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, &salt).extract(&ikm);
    let cek = expand(&prk, b"Content-Encoding: aes128gcm\0", 16);
    let nonce = expand(&prk, b"Content-Encoding: nonce\0", 12);
    let key = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_128_GCM, &cek).expect("16 bytes"));
    let nonce = aead::Nonce::try_assume_unique_for_key(&nonce).expect("12 bytes");
    let mut record = plain.to_vec();
    record.push(2);
    if record.len() + aead::AES_128_GCM.tag_len() > RECORD_SIZE as usize {
        return Err("push: the message is too long".to_string());
    }
    key.seal_in_place_append_tag(nonce, aead::Aad::empty(), &mut record)
        .map_err(|_| "push: could not encrypt".to_string())?;
    let mut body = salt.to_vec();
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(ours.len() as u8);
    body.extend_from_slice(&ours);
    body.extend_from_slice(&record);
    Ok(body)
}

enum Delivered {
    Sent,
    Gone,
    Failed(String),
}

fn deliver(agent: &ureq::Agent, vapid: &Vapid, to: &Subscription, notice: &Notice, now: u64) -> Delivered {
    let sent = (|| {
        let mut salt = [0; 16];
        let random = SystemRandom::new();
        random.fill(&mut salt).map_err(|_| "push: no randomness".to_string())?;
        let body = encrypt(
            serde_json::to_string(notice).unwrap_or_default().as_bytes(),
            to,
            &random,
            salt,
        )?;
        let authorization = vapid.authorization(&to.endpoint, now)?;
        agent
            .post(&to.endpoint)
            .header("TTL", KEPT_BY_THE_PUSH_SERVICE.to_string())
            .header("Urgency", "high")
            .header("Topic", topic(&notice.tag))
            .header("Content-Encoding", "aes128gcm")
            .header("Content-Type", "application/octet-stream")
            .header("Authorization", authorization)
            .send(&body[..])
            .map_err(|error| match error {
                ureq::Error::StatusCode(404 | 410) => "gone".to_string(),
                error => format!("{}: {error}", host(&to.endpoint)),
            })
    })();
    match sent {
        Ok(_) => Delivered::Sent,
        Err(error) if error == "gone" => Delivered::Gone,
        Err(error) => Delivered::Failed(error),
    }
}

/// A push service replaces a waiting message with a newer one of the same
/// topic: at most 32 URL-safe characters.
fn topic(tag: &str) -> String {
    tag.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .take(32)
        .collect()
}

fn host(endpoint: &str) -> String {
    url::Url::parse(endpoint)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_else(|| "push".to_string())
}

#[derive(Clone)]
pub struct Push {
    db: Db,
    subject: String,
    vapid: Arc<OnceCell<Arc<Vapid>>>,
    agent: ureq::Agent,
}

impl Push {
    /// `subject` is how a push service reaches whoever runs this: its address on the web.
    pub fn new(db: Db, subject: String) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(true)
            .build()
            .new_agent();
        Self {
            db,
            subject,
            vapid: Arc::default(),
            agent,
        }
    }

    async fn vapid(&self) -> Result<Arc<Vapid>, String> {
        self.vapid
            .get_or_try_init(|| async {
                let kept = self
                    .db
                    .setting(KEY_SETTING)
                    .await
                    .map_err(|error| format!("push key: {error}"))?;
                let pkcs8 = match kept.and_then(|kept| LENIENT.decode(kept).ok()) {
                    Some(pkcs8) => pkcs8,
                    None => {
                        let made = Vapid::generate()?;
                        self.db
                            .set_setting(KEY_SETTING, &URL_SAFE_NO_PAD.encode(&made))
                            .await
                            .map_err(|error| format!("push key: {error}"))?;
                        made
                    }
                };
                Vapid::new(&pkcs8, &self.subject).map(Arc::new)
            })
            .await
            .cloned()
    }

    /// The key a browser subscribes with, so it takes pushes only from here.
    pub async fn key(&self) -> Result<String, String> {
        Ok(self.vapid().await?.public_key())
    }

    /// Sends `notice` to every browser that turned notifications on, and
    /// forgets the ones their push service no longer knows.
    pub async fn notify(&self, notice: &Notice) -> usize {
        let vapid = match self.vapid().await {
            Ok(vapid) => vapid,
            Err(error) => {
                eprintln!("{error}");
                return 0;
            }
        };
        let subscriptions = self.db.subscriptions().await.unwrap_or_else(|error| {
            eprintln!("push: subscriptions: {error}");
            Vec::new()
        });
        let mut sent = 0;
        for to in subscriptions {
            let (agent, vapid, notice, endpoint) =
                (self.agent.clone(), vapid.clone(), notice.clone(), to.endpoint.clone());
            let now = crate::now() as u64;
            let delivered = tokio::task::spawn_blocking(move || deliver(&agent, &vapid, &to, &notice, now))
                .await
                .unwrap_or_else(|error| Delivered::Failed(error.to_string()));
            match delivered {
                Delivered::Sent => sent += 1,
                Delivered::Gone => {
                    eprintln!("push: {} forgot a browser; dropping it", host(&endpoint));
                    if let Err(error) = self.db.unsubscribe(&endpoint).await {
                        eprintln!("push: {error}");
                    }
                }
                Delivered::Failed(error) => eprintln!("push: {error}"),
            }
        }
        sent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vapid_header_carries_a_token_signed_with_our_key() {
        let vapid = Vapid::new(&Vapid::generate().unwrap(), "https://courts.example").unwrap();
        let header = vapid.authorization("https://push.example/abc", 1000).unwrap();
        let (token, key) = header.strip_prefix("vapid t=").unwrap().split_once(", k=").unwrap();
        assert_eq!(key, vapid.public_key());
        let claims = token.split('.').nth(1).unwrap();
        let claims: serde_json::Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(claims).unwrap()).unwrap();
        assert_eq!(claims["aud"], "https://push.example");
        assert_eq!(claims["sub"], "https://courts.example");
    }

    #[test]
    fn a_message_is_sealed_for_the_browsers_key() {
        let browser = agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &SystemRandom::new()).unwrap();
        let public = browser.compute_public_key().unwrap();
        let to = Subscription {
            endpoint: "https://push.example/abc".into(),
            p256dh: URL_SAFE_NO_PAD.encode(public.as_ref()),
            auth: URL_SAFE_NO_PAD.encode([7u8; 16]),
            created_at: 0,
        };
        let body = encrypt(b"hello", &to, &SystemRandom::new(), [1; 16]).unwrap();
        assert_eq!(&body[..16], &[1; 16]);
        assert_eq!(body[20], 65);
        assert_eq!(body.len(), 16 + 4 + 1 + 65 + 5 + 1 + 16);
    }
}
