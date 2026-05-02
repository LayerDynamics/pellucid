//! `ClerkJwtVerifier` — production implementation of
//! [`pellucid_gateway::ClerkVerifier`].
//!
//! The verifier:
//!
//! 1. Decodes the token header to learn the signing `kid`.
//! 2. Pulls the JWKS document from cache (refreshing once every
//!    [`crate::jwks::DEFAULT_TTL`]) and locates the matching key.
//! 3. Validates signature + `exp` + `iss` (+ optional `aud`) using the
//!    `jsonwebtoken` crate.
//! 4. Returns [`ClerkClaims`] (re-exported from `pellucid-gateway` so
//!    the gateway's stage 5 can consume it directly).

use std::time::Duration;

use async_trait::async_trait;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use pellucid_gateway::traits::{ClerkClaims, ClerkVerifier, ClerkVerifyError};
use serde::{Deserialize, Serialize};

use crate::jwks::{JwksError, JwksFetcher};

/// Default leeway applied to `exp` / `nbf` checks, in seconds.
pub const DEFAULT_LEEWAY_SECS: u64 = 60;

/// Token claim shape Clerk emits. Parsed by `jsonwebtoken::decode`.
#[derive(Debug, Deserialize, Serialize)]
struct RawClaims {
    /// Subject — Clerk user identifier.
    sub: String,
    /// Session identifier. Clerk uses `sid`.
    #[serde(default)]
    sid: Option<String>,
    /// Issuer.
    iss: String,
    /// Expiry (epoch seconds).
    exp: i64,
    /// Issued at (epoch seconds). Optional.
    #[serde(default)]
    iat: Option<i64>,
}

/// Production Clerk JWT verifier.
///
/// ## `aud` (audience) validation policy
///
/// Audience checking is **opt-in**, not on-by-default. Behaviour:
///
/// | Configuration                                      | Verifier behaviour                                              |
/// |----------------------------------------------------|-----------------------------------------------------------------|
/// | `ClerkJwtVerifier::new(...)` (no `with_audience`)  | `aud` is **not** validated — `validation.validate_aud = false`. |
/// | `.with_audience("expected-aud")`                   | Token's `aud` claim must equal `"expected-aud"` exactly.        |
///
/// Rationale: Clerk frontend tokens omit `aud` entirely (the issuer
/// itself uniquely identifies the Clerk instance via `iss`), and the
/// gateway already validates `iss` against `expected_issuer` on every
/// request. Requiring `aud` by default would 401 every legitimate
/// Clerk session — see SPEC-001 §13.1. Production deployments that
/// also issue M2M / backend tokens with an explicit audience SHOULD
/// call [`Self::with_audience`] to harden the verifier; the
/// `wrong_audience_is_rejected_when_audience_required` unit test
/// pins this behaviour.
#[derive(Debug)]
pub struct ClerkJwtVerifier {
    fetcher: JwksFetcher,
    expected_issuer: String,
    expected_audience: Option<String>,
    leeway_secs: u64,
    algorithm: Algorithm,
}

impl ClerkJwtVerifier {
    /// Construct with the default 5-minute JWKS TTL and 60-second
    /// `exp` leeway.
    #[must_use]
    pub fn new(jwks_url: impl Into<String>, expected_issuer: impl Into<String>) -> Self {
        Self {
            fetcher: JwksFetcher::new(jwks_url, default_http_client()),
            expected_issuer: expected_issuer.into(),
            expected_audience: None,
            leeway_secs: DEFAULT_LEEWAY_SECS,
            algorithm: Algorithm::RS256,
        }
    }

    /// Override the JWKS cache TTL.
    #[must_use]
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        let url = self.fetcher.url().to_string();
        let http = default_http_client();
        self.fetcher = JwksFetcher::with_ttl(url, http, ttl);
        self
    }

    /// Inject a pre-built `reqwest::Client` (testing).
    #[must_use]
    pub fn with_http_client(mut self, client: reqwest::Client) -> Self {
        let url = self.fetcher.url().to_string();
        let ttl = self.fetcher.cache().ttl();
        self.fetcher = JwksFetcher::with_ttl(url, client, ttl);
        self
    }

    /// Require the token's `aud` claim equal `audience`.
    ///
    /// Without this call the verifier sets
    /// `validation.validate_aud = false` and the `aud` claim is
    /// ignored (Clerk frontend tokens routinely omit it). Calling
    /// this opts into strict equality: any token whose `aud` differs
    /// — or which omits `aud` entirely — is rejected with
    /// [`ClerkVerifyError::Malformed`]. See the type-level docs on
    /// [`ClerkJwtVerifier`] for the full policy.
    #[must_use]
    pub fn with_audience(mut self, audience: impl Into<String>) -> Self {
        self.expected_audience = Some(audience.into());
        self
    }

    /// Override the `exp`/`nbf` leeway window.
    #[must_use]
    pub fn with_leeway(mut self, secs: u64) -> Self {
        self.leeway_secs = secs;
        self
    }

    /// Override the expected algorithm. Defaults to RS256, which is
    /// what Clerk uses.
    #[must_use]
    pub fn with_algorithm(mut self, alg: Algorithm) -> Self {
        self.algorithm = alg;
        self
    }

    /// Reference to the underlying JWKS fetcher (testing).
    #[must_use]
    pub fn fetcher(&self) -> &JwksFetcher {
        &self.fetcher
    }

    /// Look up `kid` in the cached JWKS, force-refreshing once on
    /// miss to absorb a Clerk key rotation that occurred since the
    /// cached fetch. Returns the cloned [`Jwk`] on success.
    async fn lookup_with_one_refresh(
        &self,
        kid: &str,
    ) -> Result<jsonwebtoken::jwk::Jwk, ClerkVerifyError> {
        let jwks = self
            .fetcher
            .get_or_refresh()
            .await
            .map_err(|e| ClerkVerifyError::Jwks(e.to_string()))?;
        if let Some(jwk) = jwks.find(kid).cloned() {
            return Ok(jwk);
        }
        let refreshed = self
            .fetcher
            .refresh()
            .await
            .map_err(|e| ClerkVerifyError::Jwks(e.to_string()))?;
        refreshed.find(kid).cloned().ok_or_else(|| {
            ClerkVerifyError::Jwks(
                JwksError::KidNotFound {
                    kid: kid.to_string(),
                }
                .to_string(),
            )
        })
    }
}

#[async_trait]
impl ClerkVerifier for ClerkJwtVerifier {
    async fn verify(&self, token: &str) -> Result<ClerkClaims, ClerkVerifyError> {
        let header = decode_header(token).map_err(|err| {
            tracing::debug!(target: "pellucid::auth", "decode_header failed: {err}");
            ClerkVerifyError::Malformed
        })?;
        let kid = header.kid.ok_or(ClerkVerifyError::Malformed)?;

        let jwk = self.lookup_with_one_refresh(&kid).await?;

        let decoding_key = DecodingKey::from_jwk(&jwk).map_err(|err| {
            tracing::debug!(target: "pellucid::auth", "from_jwk: {err}");
            ClerkVerifyError::BadSignature
        })?;

        let mut validation = Validation::new(self.algorithm);
        validation.set_issuer(&[&self.expected_issuer]);
        if let Some(aud) = &self.expected_audience {
            validation.set_audience(&[aud]);
        } else {
            validation.validate_aud = false;
        }
        validation.leeway = self.leeway_secs;

        let token_data = decode::<RawClaims>(token, &decoding_key, &validation).map_err(|err| {
            tracing::debug!(target: "pellucid::auth", "decode: {err}");
            map_jsonwebtoken_error(&err)
        })?;

        Ok(ClerkClaims {
            user_id: token_data.claims.sub,
            session_id: token_data.claims.sid.unwrap_or_default(),
            expires_at: token_data.claims.exp,
            issuer: token_data.claims.iss,
        })
    }
}

fn default_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default()
}

fn map_jsonwebtoken_error(err: &jsonwebtoken::errors::Error) -> ClerkVerifyError {
    use jsonwebtoken::errors::ErrorKind;
    match err.kind() {
        ErrorKind::ExpiredSignature => ClerkVerifyError::Expired,
        ErrorKind::InvalidSignature
        | ErrorKind::InvalidAlgorithm
        | ErrorKind::InvalidAlgorithmName
        | ErrorKind::InvalidEcdsaKey
        | ErrorKind::InvalidRsaKey(_)
        | ErrorKind::RsaFailedSigning
        | ErrorKind::InvalidKeyFormat
        | ErrorKind::MissingAlgorithm => ClerkVerifyError::BadSignature,
        ErrorKind::InvalidToken
        | ErrorKind::Base64(_)
        | ErrorKind::Json(_)
        | ErrorKind::Utf8(_) => ClerkVerifyError::Malformed,
        ErrorKind::InvalidIssuer
        | ErrorKind::InvalidAudience
        | ErrorKind::InvalidSubject
        | ErrorKind::ImmatureSignature
        | ErrorKind::MissingRequiredClaim(_) => ClerkVerifyError::BadSignature,
        _ => ClerkVerifyError::BadSignature,
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::test_keys::{fixture_jwk, fixture_signing_key, FIXTURE_KID};
    use jsonwebtoken::{encode, EncodingKey, Header};
    use std::sync::Arc;
    use tokio::sync::OnceCell;

    fn now_secs() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    static CACHED_JWKS_FOR_TESTS: OnceCell<jsonwebtoken::jwk::JwkSet> = OnceCell::const_new();

    async fn primed_verifier(issuer: &str) -> ClerkJwtVerifier {
        let v = ClerkJwtVerifier::new("http://unused-test/.well-known/jwks.json", issuer)
            .with_leeway(0)
            .with_ttl(Duration::from_secs(3600));
        // Prime the cache directly so the verifier never makes an
        // HTTP call. The fetcher's URL stays unused.
        let jwks = CACHED_JWKS_FOR_TESTS
            .get_or_init(|| async { jsonwebtoken::jwk::JwkSet { keys: vec![fixture_jwk()] } })
            .await
            .clone();
        v.fetcher().cache().set(jwks).await;
        v
    }

    fn sign(claims: serde_json::Value, alg: Algorithm) -> String {
        let mut header = Header::new(alg);
        header.kid = Some(FIXTURE_KID.to_string());
        let key = EncodingKey::from_rsa_pem(fixture_signing_key().as_bytes()).unwrap();
        encode(&header, &claims, &key).unwrap()
    }

    #[tokio::test]
    async fn valid_token_verifies_with_expected_claims() {
        let v = primed_verifier("https://clerk.test").await;
        let token = sign(
            serde_json::json!({
                "sub": "user_abc",
                "sid": "sess_xyz",
                "iss": "https://clerk.test",
                "exp": now_secs() + 600,
                "iat": now_secs() - 5,
            }),
            Algorithm::RS256,
        );
        let claims = v.verify(&token).await.unwrap();
        assert_eq!(claims.user_id, "user_abc");
        assert_eq!(claims.session_id, "sess_xyz");
        assert_eq!(claims.issuer, "https://clerk.test");
        assert!(claims.expires_at > now_secs());
    }

    #[tokio::test]
    async fn expired_token_is_rejected() {
        let v = primed_verifier("https://clerk.test").await;
        let token = sign(
            serde_json::json!({
                "sub": "user_abc",
                "sid": "sess",
                "iss": "https://clerk.test",
                "exp": now_secs() - 60,
                "iat": now_secs() - 600,
            }),
            Algorithm::RS256,
        );
        let err = v.verify(&token).await.unwrap_err();
        assert!(matches!(err, ClerkVerifyError::Expired), "got {err:?}");
    }

    #[tokio::test]
    async fn wrong_issuer_is_rejected() {
        let v = primed_verifier("https://clerk.test").await;
        let token = sign(
            serde_json::json!({
                "sub": "user_abc",
                "sid": "sess",
                "iss": "https://attacker.example",
                "exp": now_secs() + 600,
            }),
            Algorithm::RS256,
        );
        let err = v.verify(&token).await.unwrap_err();
        assert!(matches!(err, ClerkVerifyError::BadSignature), "got {err:?}");
    }

    #[tokio::test]
    async fn wrong_audience_is_rejected_when_audience_required() {
        let v = primed_verifier("https://clerk.test")
            .await
            .with_audience("expected-aud");
        let token = sign(
            serde_json::json!({
                "sub": "user_abc",
                "sid": "sess",
                "iss": "https://clerk.test",
                "aud": "wrong-aud",
                "exp": now_secs() + 600,
            }),
            Algorithm::RS256,
        );
        let err = v.verify(&token).await.unwrap_err();
        assert!(matches!(err, ClerkVerifyError::BadSignature), "got {err:?}");
    }

    #[tokio::test]
    async fn correct_audience_accepted() {
        let v = primed_verifier("https://clerk.test")
            .await
            .with_audience("expected-aud");
        let token = sign(
            serde_json::json!({
                "sub": "user_abc",
                "sid": "sess",
                "iss": "https://clerk.test",
                "aud": "expected-aud",
                "exp": now_secs() + 600,
            }),
            Algorithm::RS256,
        );
        let claims = v.verify(&token).await.unwrap();
        assert_eq!(claims.user_id, "user_abc");
    }

    #[tokio::test]
    async fn malformed_token_is_rejected() {
        let v = primed_verifier("https://clerk.test").await;
        let err = v.verify("not.a.jwt").await.unwrap_err();
        assert!(matches!(err, ClerkVerifyError::Malformed), "got {err:?}");
    }

    #[tokio::test]
    async fn token_missing_sub_is_rejected() {
        let v = primed_verifier("https://clerk.test").await;
        let token = sign(
            serde_json::json!({
                "iss": "https://clerk.test",
                "exp": now_secs() + 600,
            }),
            Algorithm::RS256,
        );
        let err = v.verify(&token).await.unwrap_err();
        // jsonwebtoken treats `sub` as a non-required claim by
        // default, but our `RawClaims` deserializer requires it. The
        // resulting kind is `Json` → `Malformed`.
        assert!(matches!(err, ClerkVerifyError::Malformed), "got {err:?}");
    }

    #[tokio::test]
    async fn header_without_kid_is_rejected_as_malformed() {
        let v = primed_verifier("https://clerk.test").await;
        // Sign without a kid by going below the helper.
        let mut header = Header::new(Algorithm::RS256);
        // No kid set.
        let key = EncodingKey::from_rsa_pem(fixture_signing_key().as_bytes()).unwrap();
        let token = encode(
            &header,
            &serde_json::json!({
                "sub": "user_abc",
                "sid": "sess",
                "iss": "https://clerk.test",
                "exp": now_secs() + 600,
            }),
            &key,
        )
        .unwrap();
        let _ = &mut header;
        let err = v.verify(&token).await.unwrap_err();
        assert!(matches!(err, ClerkVerifyError::Malformed), "got {err:?}");
    }

    #[tokio::test]
    async fn unknown_kid_returns_jwks_error_after_refresh_attempt() {
        // We force the cache to a JWKS that does NOT contain the fixture
        // kid; the verifier will try to refresh from the unused URL,
        // which fails on the network. Result: Jwks error.
        let v = ClerkJwtVerifier::new("http://127.0.0.1:1/jwks", "https://clerk.test")
            .with_leeway(0)
            .with_ttl(Duration::from_secs(3600));
        let bogus = jsonwebtoken::jwk::JwkSet { keys: vec![] };
        v.fetcher().cache().set(bogus).await;
        let token = sign(
            serde_json::json!({
                "sub": "user_abc",
                "sid": "sess",
                "iss": "https://clerk.test",
                "exp": now_secs() + 600,
            }),
            Algorithm::RS256,
        );
        let err = v.verify(&token).await.unwrap_err();
        assert!(matches!(err, ClerkVerifyError::Jwks(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn ttl_setter_keeps_url_intact() {
        let v = ClerkJwtVerifier::new("https://clerk.test/jwks", "https://clerk.test")
            .with_ttl(Duration::from_secs(60));
        assert_eq!(v.fetcher().url(), "https://clerk.test/jwks");
        assert_eq!(v.fetcher().cache().ttl(), Duration::from_secs(60));
    }

    #[tokio::test]
    async fn http_client_can_be_swapped() {
        let v = ClerkJwtVerifier::new("https://clerk.test/jwks", "https://clerk.test")
            .with_http_client(reqwest::Client::new());
        assert_eq!(v.fetcher().url(), "https://clerk.test/jwks");
    }

    #[tokio::test]
    async fn verify_uses_cached_jwks_without_refetch() {
        // Two sequential verifies should both hit the cache and not
        // touch the network; the cache's `read_fresh` returning Some
        // is the proof.
        let v = primed_verifier("https://clerk.test").await;
        let token = sign(
            serde_json::json!({
                "sub": "user_abc",
                "sid": "sess",
                "iss": "https://clerk.test",
                "exp": now_secs() + 600,
            }),
            Algorithm::RS256,
        );
        v.verify(&token).await.unwrap();
        // Cache must still be present.
        assert!(v.fetcher().cache().read_fresh().await.is_some());
        v.verify(&token).await.unwrap();
        assert!(v.fetcher().cache().read_fresh().await.is_some());
    }

    #[tokio::test]
    async fn verifier_implements_clerk_verifier_trait_object() {
        // Exercises the dynamic-dispatch path the gateway uses.
        let v: Arc<dyn ClerkVerifier> = Arc::new(primed_verifier("https://clerk.test").await);
        let token = sign(
            serde_json::json!({
                "sub": "user_abc",
                "sid": "sess",
                "iss": "https://clerk.test",
                "exp": now_secs() + 600,
            }),
            Algorithm::RS256,
        );
        let claims = v.verify(&token).await.unwrap();
        assert_eq!(claims.user_id, "user_abc");
    }
}
