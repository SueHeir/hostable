//! Web Push: message encryption (RFC 8291, `aes128gcm` from RFC 8188) and
//! VAPID authentication (RFC 8292), with no OpenSSL.
//!
//! VAPID keys come from the environment (`HOSTABLE_VAPID_PRIVATE_KEY`, a
//! base64url P-256 private scalar, and `HOSTABLE_VAPID_SUBJECT`, e.g.
//! `mailto:admin@example.com`). They are never committed.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes128Gcm, Nonce};
use base64::Engine as _;
use futures_util::future::BoxFuture;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use p256::ecdsa::signature::Signer;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::{PublicKey, SecretKey};
use serde_json::{json, Value};
use sha2_push::Sha256;

const B64URL: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;
const RECORD_SIZE: u32 = 4096;

/// Something that delivers push messages. The real one speaks HTTP to the
/// browser's push service; tests use [`Recorder`].
pub trait PushSender: Send + Sync {
    fn public_key(&self) -> String;
    fn send<'a>(
        &'a self,
        subscription: &'a Value,
        payload: Value,
    ) -> BoxFuture<'a, Result<(), String>>;
}

/// Push is off: nothing is sent, and the client hides the opt-in.
pub struct Disabled;

impl PushSender for Disabled {
    fn public_key(&self) -> String {
        String::new()
    }
    fn send<'a>(&'a self, _: &'a Value, _: Value) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
}

/// A test double that keeps every message instead of sending it.
#[derive(Default, Clone)]
pub struct Recorder {
    pub sent: Arc<Mutex<Vec<(String, Value)>>>,
}

impl PushSender for Recorder {
    fn public_key(&self) -> String {
        "test-key".into()
    }
    fn send<'a>(&'a self, sub: &'a Value, payload: Value) -> BoxFuture<'a, Result<(), String>> {
        let endpoint = sub["endpoint"].as_str().unwrap_or_default().to_string();
        self.sent.lock().unwrap().push((endpoint, payload));
        Box::pin(async { Ok(()) })
    }
}

/// Real Web Push with VAPID.
pub struct WebPush {
    key: SecretKey,
    subject: String,
    http: reqwest::Client,
}

impl WebPush {
    pub fn new(private_key_b64url: &str, subject: &str) -> Result<Self, String> {
        let raw = B64URL
            .decode(private_key_b64url.trim().trim_end_matches('='))
            .map_err(|e| format!("VAPID key: {e}"))?;
        let key = SecretKey::from_slice(&raw).map_err(|e| format!("VAPID key: {e}"))?;
        if !subject.starts_with("mailto:") && !subject.starts_with("https://") {
            return Err("VAPID subject must be a mailto: or https: URL".into());
        }
        Ok(Self {
            key,
            subject: subject.to_string(),
            http: reqwest::Client::new(),
        })
    }

    /// From `HOSTABLE_VAPID_PRIVATE_KEY` and `HOSTABLE_VAPID_SUBJECT`.
    pub fn from_env() -> Option<Result<Self, String>> {
        let key = std::env::var("HOSTABLE_VAPID_PRIVATE_KEY").ok()?;
        let subject = std::env::var("HOSTABLE_VAPID_SUBJECT")
            .unwrap_or_else(|_| "mailto:admin@localhost".into());
        Some(Self::new(&key, &subject))
    }

    /// `Authorization: vapid t=<jwt>, k=<public key>` for an endpoint.
    pub fn authorization(&self, endpoint: &str, now_s: u64) -> Result<String, String> {
        let url = reqwest::Url::parse(endpoint).map_err(|e| e.to_string())?;
        let aud = url.origin().ascii_serialization();
        let header = B64URL.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims = B64URL.encode(
            json!({"aud": aud, "exp": now_s + 12 * 3600, "sub": self.subject})
                .to_string()
                .as_bytes(),
        );
        let signing_input = format!("{header}.{claims}");
        let signer = p256::ecdsa::SigningKey::from(&self.key);
        let sig: p256::ecdsa::Signature = signer.sign(signing_input.as_bytes());
        let jwt = format!("{signing_input}.{}", B64URL.encode(sig.to_bytes()));
        Ok(format!("vapid t={jwt}, k={}", self.public_key()))
    }
}

impl PushSender for WebPush {
    fn public_key(&self) -> String {
        B64URL.encode(self.key.public_key().to_encoded_point(false).as_bytes())
    }

    fn send<'a>(&'a self, sub: &'a Value, payload: Value) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let endpoint = sub["endpoint"].as_str().ok_or("no endpoint")?;
            let p256dh = sub["keys"]["p256dh"].as_str().ok_or("no p256dh key")?;
            let auth = sub["keys"]["auth"].as_str().ok_or("no auth secret")?;
            let ua_public = B64URL
                .decode(p256dh.trim_end_matches('='))
                .map_err(|e| e.to_string())?;
            let auth = B64URL
                .decode(auth.trim_end_matches('='))
                .map_err(|e| e.to_string())?;
            let ephemeral = SecretKey::random(&mut p256::elliptic_curve::rand_core::OsRng);
            let mut salt = [0u8; 16];
            rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut salt);
            let body = encrypt(
                payload.to_string().as_bytes(),
                &ua_public,
                &auth,
                &ephemeral,
                salt,
            )?;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let res = self
                .http
                .post(endpoint)
                .header("Content-Encoding", "aes128gcm")
                .header("Content-Type", "application/octet-stream")
                .header("TTL", "86400")
                .header("Urgency", "high")
                .header("Authorization", self.authorization(endpoint, now)?)
                .body(body)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if res.status().is_success() {
                Ok(())
            } else {
                Err(format!("push service answered {}", res.status()))
            }
        })
    }
}

fn hmac(key: &[u8], data: &[&[u8]]) -> [u8; 32] {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("any key length");
    for d in data {
        mac.update(d);
    }
    mac.finalize().into_bytes().into()
}

/// Encrypt one push message (RFC 8291 section 3, one record).
pub fn encrypt(
    plaintext: &[u8],
    ua_public: &[u8],
    auth_secret: &[u8],
    as_private: &SecretKey,
    salt: [u8; 16],
) -> Result<Vec<u8>, String> {
    if plaintext.len() + 17 + 86 > RECORD_SIZE as usize {
        return Err("push payload too large".into());
    }
    let ua = PublicKey::from_sec1_bytes(ua_public).map_err(|e| format!("p256dh: {e}"))?;
    let as_public = as_private.public_key().to_encoded_point(false);
    let shared = p256::ecdh::diffie_hellman(as_private.to_nonzero_scalar(), ua.as_affine());
    let ecdh_secret = shared.raw_secret_bytes();
    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(ua_public);
    key_info.extend_from_slice(as_public.as_bytes());
    // IKM = HKDF(auth_secret, ecdh_secret, key_info, 32)
    let prk_key = hmac(auth_secret, &[ecdh_secret.as_slice()]);
    let ikm = hmac(&prk_key, &[&key_info, &[1]]);
    let hk = Hkdf::<Sha256>::new(Some(&salt), &ikm);
    let mut cek = [0u8; 16];
    let mut nonce = [0u8; 12];
    hk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
        .map_err(|e| e.to_string())?;
    hk.expand(b"Content-Encoding: nonce\0", &mut nonce)
        .map_err(|e| e.to_string())?;
    let mut padded = plaintext.to_vec();
    padded.push(2); // last-record delimiter
    let cipher = Aes128Gcm::new_from_slice(&cek).map_err(|e| e.to_string())?;
    let sealed = cipher
        .encrypt(Nonce::from_slice(&nonce), padded.as_slice())
        .map_err(|e| e.to_string())?;
    let mut body = Vec::with_capacity(86 + sealed.len());
    body.extend_from_slice(&salt);
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(as_public.as_bytes().len() as u8);
    body.extend_from_slice(as_public.as_bytes());
    body.extend_from_slice(&sealed);
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Vec<u8> {
        B64URL.decode(s).unwrap()
    }

    /// RFC 8291 Appendix A.
    #[test]
    fn matches_the_rfc_8291_example() {
        let as_private =
            SecretKey::from_slice(&d("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw")).unwrap();
        assert_eq!(
            B64URL.encode(as_private.public_key().to_encoded_point(false).as_bytes()),
            "BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8"
        );
        let ua_private =
            SecretKey::from_slice(&d("q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94")).unwrap();
        let ua_public = ua_private
            .public_key()
            .to_encoded_point(false)
            .as_bytes()
            .to_vec();
        let salt: [u8; 16] = d("DGv6ra1nlYgDCS1FRnbzlw").try_into().unwrap();
        let auth = d("BTBZMqHH6r4Tts7J_aSIgg");
        let plaintext = d("V2hlbiBJIGdyb3cgdXAsIEkgd2FudCB0byBiZSBhIHdhdGVybWVsb24");
        let body = encrypt(&plaintext, &ua_public, &auth, &as_private, salt).unwrap();
        let header = d("DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8");
        let ciphertext =
            d("8pfeW0KbunFT06SuDKoJH9Ql87S1QUrdirN6GcG7sFz1y1sqLgVi1VhjVkHsUoEsbI_0LpXMuGvnzQ");
        assert_eq!(&body[..86], header.as_slice());
        assert_eq!(&body[86..], ciphertext.as_slice());
    }

    #[test]
    fn vapid_headers_carry_a_verifiable_es256_token() {
        use p256::ecdsa::signature::Verifier;
        let key = SecretKey::random(&mut p256::elliptic_curve::rand_core::OsRng);
        let raw = B64URL.encode(key.to_bytes());
        let push = WebPush::new(&raw, "mailto:ops@example.com").unwrap();
        let auth = push
            .authorization("https://push.example.net/send/abc", 1_000)
            .unwrap();
        let (t, k) = auth
            .strip_prefix("vapid t=")
            .unwrap()
            .split_once(", k=")
            .unwrap();
        assert_eq!(k, push.public_key());
        let parts: Vec<&str> = t.split('.').collect();
        let claims: Value = serde_json::from_slice(&d(parts[1])).unwrap();
        assert_eq!(claims["aud"], "https://push.example.net");
        assert_eq!(claims["exp"], 1_000 + 12 * 3600);
        let sig = p256::ecdsa::Signature::from_slice(&d(parts[2])).unwrap();
        let verifier = p256::ecdsa::VerifyingKey::from(key.public_key());
        verifier
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig)
            .unwrap();
        assert!(WebPush::new(&raw, "nobody").is_err());
    }
}
