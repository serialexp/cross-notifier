//! APNS HTTP/2 client. Send one push per device token, mint and cache a
//! short-lived ES256 JWT per Apple's provider-token spec.
//!
//! Minimal surface: one `ApnsClient::send(token, payload)` per notification.
//! Errors are categorised into "prune this token", "retryable server
//! problem", and "fatal (bad config)" so the caller can react without
//! having to parse APNS response bodies itself.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::protocol::Notification;

/// Apple's two push hosts. Sandbox is for development builds signed with
/// a development provisioning profile; production is for TestFlight /
/// App Store builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApnsEnvironment {
    Production,
    Sandbox,
}

impl ApnsEnvironment {
    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::Production => "https://api.push.apple.com",
            Self::Sandbox => "https://api.sandbox.push.apple.com",
        }
    }
}

/// Apple-issued authentication key, loaded either directly from disk or
/// from a base64-encoded env var (convenient for k8s secrets / CI).
#[derive(Debug, Clone)]
pub enum ApnsKey {
    /// Raw PEM bytes (the contents of a `.p8` file).
    Pem(Vec<u8>),
}

impl ApnsKey {
    /// Read a `.p8` file off disk.
    pub fn from_file(path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        Ok(Self::Pem(std::fs::read(path)?))
    }

    /// Decode a base64-wrapped `.p8` file (whitespace-tolerant).
    pub fn from_base64(encoded: &str) -> Result<Self, base64::DecodeError> {
        // Strip whitespace first — k8s secrets frequently line-wrap at 76
        // chars, and real-world `kubectl exec printenv` output wraps too.
        let cleaned: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();
        let bytes = base64::engine::general_purpose::STANDARD.decode(cleaned.as_bytes())?;
        Ok(Self::Pem(bytes))
    }

    fn pem_bytes(&self) -> &[u8] {
        match self {
            Self::Pem(b) => b,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ApnsConfig {
    /// `https://api.push.apple.com` in production; overridden by tests
    /// to point at a local mock.
    pub base_url: String,
    /// Apple developer team ID (10 chars).
    pub team_id: String,
    /// Auth key ID (10 chars) — the `kid` header of every JWT.
    pub key_id: String,
    /// iOS app bundle identifier — sent as the `apns-topic` header.
    pub bundle_id: String,
    /// Signing key material.
    pub key: ApnsKey,
}

impl ApnsConfig {
    /// Sensible default base URL for the given environment. Tests use
    /// [`ApnsConfig::new`] directly and override `base_url`.
    pub fn for_environment(
        env: ApnsEnvironment,
        team_id: impl Into<String>,
        key_id: impl Into<String>,
        bundle_id: impl Into<String>,
        key: ApnsKey,
    ) -> Self {
        Self {
            base_url: env.default_base_url().to_string(),
            team_id: team_id.into(),
            key_id: key_id.into(),
            bundle_id: bundle_id.into(),
            key,
        }
    }
}

/// Categorised result of a push attempt.
#[derive(Debug)]
pub enum PushOutcome {
    /// APNS accepted the notification for delivery.
    Delivered,
    /// APNS says this token is dead — caller should remove it from the
    /// registry. Carries the APNS `reason` string for logging.
    PruneToken(String),
    /// Transient server failure (429, 5xx, network error). Caller should
    /// try again on the next notification.
    Retryable(String),
}

/// Unrecoverable errors — config is wrong, key can't be parsed, etc.
/// Push is effectively disabled until the operator fixes it.
#[derive(Debug, Error)]
pub enum PushError {
    #[error("failed to sign APNS JWT: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),
    #[error("APNS rejected credentials: {0}")]
    AuthRejected(String),
    #[error("system clock before UNIX_EPOCH — refuse to sign tokens")]
    ClockSkew,
}

/// Cached JWT. APNS rejects tokens older than 1 hour and warns against
/// minting more than once per 20 minutes, so we refresh at the 40-minute
/// mark — comfortably inside the window either way.
struct CachedJwt {
    token: String,
    issued_at: u64,
}

/// Async-safe APNS push client. Cheap to clone.
#[derive(Clone)]
pub struct ApnsClient {
    http: reqwest::Client,
    config: Arc<ApnsConfig>,
    jwt: Arc<Mutex<Option<CachedJwt>>>,
}

const JWT_MAX_AGE_SECS: u64 = 40 * 60;

impl ApnsClient {
    pub fn new(config: ApnsConfig) -> Self {
        Self::with_http_client(config, reqwest::Client::new())
    }

    /// Construct with a pre-built reqwest client — useful if the caller
    /// wants to share a connection pool or override timeouts.
    pub fn with_http_client(config: ApnsConfig, http: reqwest::Client) -> Self {
        Self {
            http,
            config: Arc::new(config),
            jwt: Arc::new(Mutex::new(None)),
        }
    }

    /// Build the APNS payload body from a cross-notifier `Notification`.
    /// Returns a JSON object ready to be POSTed.
    ///
    /// v1 is deliberately minimal: title + body + a custom-data envelope
    /// so the iOS app can recover source / id / icon URL without parsing
    /// the alert text. Action buttons and exclusive resolution are not
    /// mapped yet; those need a client-side `UNNotificationCategory`
    /// negotiation we haven't built.
    pub fn build_payload(n: &Notification) -> Value {
        let mut alert = serde_json::Map::new();
        if !n.title.is_empty() {
            alert.insert("title".into(), Value::String(n.title.clone()));
        }
        if !n.message.is_empty() {
            alert.insert("body".into(), Value::String(n.message.clone()));
        }

        let mut payload = json!({
            "aps": {
                "alert": Value::Object(alert),
                "sound": "default",
                // Lets a Notification Service Extension on-device mutate
                // the payload (e.g. download icon_href) before display.
                "mutable-content": 1,
            },
        });

        let obj = payload.as_object_mut().expect("payload is an object");
        if !n.source.is_empty() {
            obj.insert("source".into(), Value::String(n.source.clone()));
        }
        if !n.id.is_empty() {
            obj.insert("id".into(), Value::String(n.id.clone()));
        }
        if !n.icon_href.is_empty() {
            obj.insert("iconHref".into(), Value::String(n.icon_href.clone()));
        }
        payload
    }

    /// Send `payload` to a single device token. Caller handles registry
    /// prune on `PushOutcome::PruneToken`.
    pub async fn send(
        &self,
        device_token: &str,
        payload: &Value,
    ) -> Result<PushOutcome, PushError> {
        let jwt = self.current_jwt().await?;

        let url = format!("{}/3/device/{}", self.config.base_url, device_token);
        let req = self
            .http
            .post(&url)
            .bearer_auth(&jwt)
            .header("apns-topic", &self.config.bundle_id)
            .header("apns-push-type", "alert")
            .header("apns-priority", "10")
            .json(payload);

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                // Network-level failure: DNS, TCP, TLS, etc. Transient.
                return Ok(PushOutcome::Retryable(e.to_string()));
            }
        };

        let status = resp.status();
        if status.is_success() {
            debug!(token = short(device_token), "APNS accepted");
            return Ok(PushOutcome::Delivered);
        }

        // Apple returns JSON like `{"reason":"Unregistered"}`. Timestamp
        // (on 410) is optional and we ignore it.
        let body_text = resp.text().await.unwrap_or_default();
        let reason = serde_json::from_str::<ApnsErrorBody>(&body_text)
            .map(|b| b.reason)
            .unwrap_or_else(|_| body_text.clone());

        Ok(classify(status, reason)?)
    }

    /// Fetch (minting if necessary) the current provider JWT.
    async fn current_jwt(&self) -> Result<String, PushError> {
        let mut guard = self.jwt.lock().await;
        let now = now_secs()?;
        if let Some(cached) = guard.as_ref() {
            if now.saturating_sub(cached.issued_at) < JWT_MAX_AGE_SECS {
                return Ok(cached.token.clone());
            }
        }
        let token = sign_provider_token(&self.config, now)?;
        *guard = Some(CachedJwt {
            token: token.clone(),
            issued_at: now,
        });
        Ok(token)
    }

    /// For tests: force the cached JWT to be treated as expired on the
    /// next `send()`. Not exposed publicly outside tests.
    #[cfg(any(test, feature = "test-util"))]
    pub async fn force_jwt_refresh(&self) {
        *self.jwt.lock().await = None;
    }
}

fn classify(status: StatusCode, reason: String) -> Result<PushOutcome, PushError> {
    // Apple's documented terminal "this token is dead" statuses. 400 is a
    // grab-bag so we narrow on the `reason` payload.
    match status {
        StatusCode::GONE => Ok(PushOutcome::PruneToken(reason)), // 410 Unregistered
        StatusCode::BAD_REQUEST => {
            if reason == "BadDeviceToken" || reason == "DeviceTokenNotForTopic" {
                Ok(PushOutcome::PruneToken(reason))
            } else {
                // Other 400s (BadExpirationDate, PayloadTooLarge, etc.)
                // are bugs on our end, not dead tokens.
                Err(PushError::AuthRejected(format!("400 {reason}")))
            }
        }
        StatusCode::FORBIDDEN => Err(PushError::AuthRejected(reason)),
        StatusCode::TOO_MANY_REQUESTS => Ok(PushOutcome::Retryable(reason)),
        s if s.is_server_error() => Ok(PushOutcome::Retryable(format!("{s} {reason}"))),
        s => {
            warn!("APNS returned unexpected status {s}: {reason}");
            Ok(PushOutcome::Retryable(format!("{s} {reason}")))
        }
    }
}

fn short(token: &str) -> String {
    if token.len() <= 8 {
        token.to_string()
    } else {
        format!("{}…", &token[..8])
    }
}

#[derive(Serialize)]
struct ProviderClaims<'a> {
    iss: &'a str, // team id
    iat: u64,     // issued-at, seconds since epoch
}

fn sign_provider_token(cfg: &ApnsConfig, issued_at: u64) -> Result<String, PushError> {
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(cfg.key_id.clone());
    let claims = ProviderClaims {
        iss: &cfg.team_id,
        iat: issued_at,
    };
    let key = EncodingKey::from_ec_pem(cfg.key.pem_bytes())?;
    Ok(encode(&header, &claims, &key)?)
}

fn now_secs() -> Result<u64, PushError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| PushError::ClockSkew)
}

#[derive(Deserialize)]
struct ApnsErrorBody {
    reason: String,
}

// Max retry delay — exported so callers can align their own backoff.
#[allow(dead_code)]
pub const MAX_RETRY_BACKOFF: Duration = Duration::from_secs(60);

#[cfg(test)]
mod tests {
    use super::*;

    /// An ES256 private key in `.p8` PEM format generated for tests only.
    /// Produced with:
    ///   openssl ecparam -name prime256v1 -genkey -noout -out /tmp/k.pem
    ///   openssl pkcs8 -topk8 -nocrypt -in /tmp/k.pem -out /tmp/k.p8
    const TEST_P8: &str = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgevZzL1gdAFr88hb2\nOF/2NxApJCzGCEDdfSp6VQO30hyhRANCAAQRWz+jn65BtOMvdyHKcvjBeBSDZH2r\n1RTwjmYSi9R/zpBnuQ4EiMnCqfMPWiZqB4QdbAd0E7oH50VpuZ1P087G\n-----END PRIVATE KEY-----\n";

    fn test_config() -> ApnsConfig {
        ApnsConfig {
            base_url: "http://127.0.0.1:1".into(), // overridden in mock tests
            team_id: "TEAM123456".into(),
            key_id: "KEYID67890".into(),
            bundle_id: "com.example.test".into(),
            key: ApnsKey::Pem(TEST_P8.as_bytes().to_vec()),
        }
    }

    #[test]
    fn build_payload_minimal() {
        let mut n = Notification::default();
        n.source = "svc".into();
        n.title = "Hello".into();
        n.message = "World".into();
        let p = ApnsClient::build_payload(&n);
        let aps = &p["aps"];
        assert_eq!(aps["alert"]["title"], "Hello");
        assert_eq!(aps["alert"]["body"], "World");
        assert_eq!(aps["sound"], "default");
        assert_eq!(aps["mutable-content"], 1);
        assert_eq!(p["source"], "svc");
    }

    #[test]
    fn build_payload_omits_empty_fields() {
        let mut n = Notification::default();
        n.source = "svc".into();
        let p = ApnsClient::build_payload(&n);
        assert!(p.get("id").is_none());
        assert!(p.get("iconHref").is_none());
        assert!(p["aps"]["alert"].as_object().unwrap().is_empty());
    }

    #[test]
    fn signs_jwt_with_es256_and_kid() {
        let cfg = test_config();
        let now = now_secs().unwrap();
        let token = sign_provider_token(&cfg, now).unwrap();
        // JWT = header.payload.signature, base64url
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3, "JWT has three segments");

        let header_json = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[0])
            .unwrap();
        let header: serde_json::Value = serde_json::from_slice(&header_json).unwrap();
        assert_eq!(header["alg"], "ES256");
        assert_eq!(header["kid"], "KEYID67890");

        let claims_json = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1])
            .unwrap();
        let claims: serde_json::Value = serde_json::from_slice(&claims_json).unwrap();
        assert_eq!(claims["iss"], "TEAM123456");
        assert!(claims["iat"].as_u64().unwrap() > 0);
    }

    #[test]
    fn jwt_cache_reuses_within_window() {
        let client = ApnsClient::new(test_config());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (a, b) = rt.block_on(async {
            let a = client.current_jwt().await.unwrap();
            let b = client.current_jwt().await.unwrap();
            (a, b)
        });
        // ES256 signatures are non-deterministic, so identical tokens
        // prove the cache hit rather than coincidence.
        assert_eq!(a, b);
    }

    #[test]
    fn key_from_base64_roundtrip() {
        let b64 = base64::engine::general_purpose::STANDARD.encode(TEST_P8);
        let key = ApnsKey::from_base64(&b64).unwrap();
        assert_eq!(key.pem_bytes(), TEST_P8.as_bytes());
    }

    #[tokio::test]
    async fn roundtrip_through_mock_success() {
        let mock = crate::push::mock::MockApns::spawn().await;
        let mut cfg = test_config();
        cfg.base_url = mock.base_url.clone();
        let client = ApnsClient::new(cfg);

        let mut n = Notification::default();
        n.source = "svc".into();
        n.title = "hi".into();
        n.message = "yo".into();
        let payload = ApnsClient::build_payload(&n);

        let outcome = client.send("tok-live", &payload).await.unwrap();
        assert!(matches!(outcome, PushOutcome::Delivered));

        let reqs = mock.requests();
        assert_eq!(reqs.len(), 1);
        let r = &reqs[0];
        assert_eq!(r.token, "tok-live");
        assert_eq!(r.apns_topic.as_deref(), Some("com.example.test"));
        assert_eq!(r.apns_push_type.as_deref(), Some("alert"));
        let auth = r.authorization.as_deref().unwrap();
        assert!(auth.starts_with("Bearer "), "got {auth:?}");
        assert_eq!(r.body["aps"]["alert"]["title"], "hi");
    }

    #[tokio::test]
    async fn unregistered_token_prunes() {
        let mock = crate::push::mock::MockApns::spawn().await;
        mock.set_response("dead-token", StatusCode::GONE, "Unregistered");
        let mut cfg = test_config();
        cfg.base_url = mock.base_url.clone();
        let client = ApnsClient::new(cfg);
        let payload = ApnsClient::build_payload(&Notification::default());
        match client.send("dead-token", &payload).await.unwrap() {
            PushOutcome::PruneToken(r) => assert_eq!(r, "Unregistered"),
            other => panic!("expected PruneToken, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn bad_device_token_prunes() {
        let mock = crate::push::mock::MockApns::spawn().await;
        mock.set_response("junk", StatusCode::BAD_REQUEST, "BadDeviceToken");
        let mut cfg = test_config();
        cfg.base_url = mock.base_url.clone();
        let client = ApnsClient::new(cfg);
        let payload = ApnsClient::build_payload(&Notification::default());
        match client.send("junk", &payload).await.unwrap() {
            PushOutcome::PruneToken(r) => assert_eq!(r, "BadDeviceToken"),
            other => panic!("expected PruneToken, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn server_error_is_retryable() {
        let mock = crate::push::mock::MockApns::spawn().await;
        mock.set_response("any", StatusCode::INTERNAL_SERVER_ERROR, "InternalServerError");
        let mut cfg = test_config();
        cfg.base_url = mock.base_url.clone();
        let client = ApnsClient::new(cfg);
        let payload = ApnsClient::build_payload(&Notification::default());
        assert!(matches!(
            client.send("any", &payload).await.unwrap(),
            PushOutcome::Retryable(_)
        ));
    }

    #[tokio::test]
    async fn forbidden_is_fatal() {
        let mock = crate::push::mock::MockApns::spawn().await;
        mock.set_default(StatusCode::FORBIDDEN, "InvalidProviderToken");
        let mut cfg = test_config();
        cfg.base_url = mock.base_url.clone();
        let client = ApnsClient::new(cfg);
        let payload = ApnsClient::build_payload(&Notification::default());
        match client.send("any", &payload).await {
            Err(PushError::AuthRejected(r)) => assert_eq!(r, "InvalidProviderToken"),
            other => panic!("expected AuthRejected, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn jwt_refreshed_after_force() {
        let mock = crate::push::mock::MockApns::spawn().await;
        let mut cfg = test_config();
        cfg.base_url = mock.base_url.clone();
        let client = ApnsClient::new(cfg);
        let payload = ApnsClient::build_payload(&Notification::default());

        client.send("tok", &payload).await.unwrap();
        // Bust the cache; next send must mint a new JWT — and ES256
        // signatures are non-deterministic, so the Bearer header differs.
        client.force_jwt_refresh().await;
        // Sleep 1s so iat differs too (belt + suspenders).
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        client.send("tok", &payload).await.unwrap();

        let reqs = mock.requests();
        assert_eq!(reqs.len(), 2);
        assert_ne!(reqs[0].authorization, reqs[1].authorization);
    }

    #[test]
    fn key_from_base64_tolerates_whitespace() {
        // Simulates a line-wrapped k8s secret.
        let raw = base64::engine::general_purpose::STANDARD.encode(TEST_P8);
        let wrapped = raw
            .as_bytes()
            .chunks(64)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let key = ApnsKey::from_base64(&wrapped).unwrap();
        assert_eq!(key.pem_bytes(), TEST_P8.as_bytes());
    }
}
