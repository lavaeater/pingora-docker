//! Forward-auth: Pingora checks a signed session token on every request to a
//! protected backend, and passes the verified identity on as `X-Auth-*` headers.
//!
//! The token is an EdDSA (Ed25519) JWT issued by the login service
//! (`login.<domain>`). Browsers send it as a cookie scoped to the parent domain;
//! mobile apps and other API clients send it as `Authorization: Bearer <token>`.
//! Pingora only needs the public key, so a check costs no network round trip.

use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use percent_encoding::{AsciiSet, CONTROLS, NON_ALPHANUMERIC, utf8_percent_encode};
use pingora::http::RequestHeader;
use serde::{Deserialize, Serialize};

/// Headers Pingora sets for the backend. Always stripped from client requests,
/// so a backend can trust them whenever they are present.
pub const IDENTITY_HEADERS: [&str; 4] =
    ["X-Auth-Subject", "X-Auth-Email", "X-Auth-Name", "X-Auth-Groups"];

/// Header values must be visible ASCII, so identity values are percent-encoded
/// UTF-8: "Åsa" becomes "%C3%85sa". `%` itself is encoded so decoding is exact.
/// Apps decode with `percent_encoding::percent_decode_str`.
const HEADER_UNSAFE: &AsciiSet = &CONTROLS.add(b'%');

pub fn header_value(value: &str) -> String {
    utf8_percent_encode(value, HEADER_UNSAFE).to_string()
}

/// Global auth settings (`"auth"` in config.json).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthConfig {
    /// Where unauthenticated browsers are sent, e.g. "https://login.kidvhs.com/login".
    /// Pingora appends `?rd=<original url>`.
    pub login_url: String,
    /// Name of the session cookie the login service sets with `Domain=.kidvhs.com`.
    #[serde(default = "default_cookie_name")]
    pub cookie_name: String,
    /// Expected `iss` claim, e.g. "https://login.kidvhs.com".
    pub issuer: String,
    /// Expected `aud` claim.
    pub audience: String,
    /// PEM file with the login service's Ed25519 public key.
    pub public_key_path: String,
}

fn default_cookie_name() -> String {
    "session".to_string()
}

/// Per-backend auth mode (`"auth"` on a domain entry). Defaults to `public`
/// so existing entries keep working.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    /// No check, and identity headers are never set (Jellyfin, Medusa, the login service).
    #[default]
    Public,
    /// Identity headers are set when a valid token is present; nobody is turned away.
    /// For apps with both public pages and signed-in features.
    Optional,
    /// A valid token is needed to reach the backend.
    Required,
}

/// Claims Pingora reads from the token. `exp`, `iss` and `aud` are checked by
/// `jsonwebtoken` itself.
#[derive(Debug, Clone, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub email: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub groups: Vec<String>,
}

pub struct Authenticator {
    config: AuthConfig,
    key: DecodingKey,
    validation: Validation,
}

impl Authenticator {
    pub fn new(config: AuthConfig) -> anyhow::Result<Self> {
        let pem = std::fs::read(&config.public_key_path).map_err(|e| {
            anyhow::anyhow!("reading auth public key {}: {e}", config.public_key_path)
        })?;
        let key = DecodingKey::from_ed_pem(&pem)?;
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_issuer(&[&config.issuer]);
        validation.set_audience(&[&config.audience]);
        Ok(Self { config, key, validation })
    }

    /// Returns the verified identity, or `None` if there is no token or it is
    /// invalid or expired. A bearer token takes precedence over the cookie.
    pub fn identify(&self, req: &RequestHeader) -> Option<Claims> {
        let token = bearer_token(req).or_else(|| cookie(req, &self.config.cookie_name))?;
        match decode::<Claims>(token, &self.key, &self.validation) {
            Ok(data) => Some(data.claims),
            Err(e) => {
                log::debug!("rejected auth token: {e}");
                None
            }
        }
    }

    /// Login URL with the page the user was trying to reach. Always `https`,
    /// because the session cookie is `Secure` and never reaches a plain-HTTP request.
    pub fn login_redirect(&self, host: &str, req: &RequestHeader) -> String {
        let path = req.uri.path_and_query().map_or("/", |pq| pq.as_str());
        let original = format!("https://{host}{path}");
        let sep = if self.config.login_url.contains('?') { '&' } else { '?' };
        format!(
            "{}{sep}rd={}",
            self.config.login_url,
            utf8_percent_encode(&original, NON_ALPHANUMERIC)
        )
    }
}

/// True if `claims` match the backend's allow list. An empty list allows any
/// signed-in user. Entries are emails, or `group:<name>`.
pub fn is_allowed(claims: &Claims, allow: &[String]) -> bool {
    allow.is_empty()
        || allow.iter().any(|entry| match entry.strip_prefix("group:") {
            Some(group) => claims.groups.iter().any(|g| g == group),
            None => entry.eq_ignore_ascii_case(&claims.email),
        })
}

/// A browser navigation gets a redirect to the login page; anything else
/// (fetch, API, mobile app) gets a plain 401 it can act on.
pub fn wants_html(req: &RequestHeader) -> bool {
    req.method == "GET"
        && req
            .headers
            .get("accept")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|a| a.contains("text/html"))
}

fn bearer_token(req: &RequestHeader) -> Option<&str> {
    let value = req.headers.get("authorization")?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
}

/// HTTP/2 clients may split cookies across several `cookie` headers.
fn cookie<'a>(req: &'a RequestHeader, name: &str) -> Option<&'a str> {
    req.headers
        .get_all("cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(k, v)| (k == name).then_some(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(email: &str, groups: &[&str]) -> Claims {
        Claims {
            sub: "1".into(),
            email: email.into(),
            name: None,
            groups: groups.iter().map(|g| g.to_string()).collect(),
        }
    }

    #[test]
    fn allow_list() {
        let c = claims("Me@Example.com", &["family"]);
        assert!(is_allowed(&c, &[]));
        assert!(is_allowed(&c, &["me@example.com".into()]));
        assert!(is_allowed(&c, &["group:family".into()]));
        assert!(!is_allowed(&c, &["other@example.com".into(), "group:admins".into()]));
    }

    #[test]
    fn identity_header_values_are_ascii() {
        assert_eq!(header_value("Tommie Nygren"), "Tommie Nygren");
        assert_eq!(header_value("Åsa 100%"), "%C3%85sa 100%25");
        let mut req = RequestHeader::build("GET", b"/", None).unwrap();
        req.insert_header("X-Auth-Name", header_value("Åsa")).unwrap();
    }

    #[test]
    fn finds_cookie_and_bearer() {
        let mut req = RequestHeader::build("GET", b"/", None).unwrap();
        req.append_header("cookie", "a=1; session=tok").unwrap();
        assert_eq!(cookie(&req, "session"), Some("tok"));
        assert_eq!(cookie(&req, "missing"), None);
        req.insert_header("authorization", "Bearer abc").unwrap();
        assert_eq!(bearer_token(&req), Some("abc"));
    }

    // Throwaway keypair, used only by this test.
    const TEST_PRIVATE: &str = "-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIN8Ii+ViYBLzUjoPYWA1Gx5obfCHeCToeORTswGSbdPG
-----END PRIVATE KEY-----";
    const TEST_PUBLIC: &str = "-----BEGIN PUBLIC KEY-----
MCowBQYDK2VwAyEASH6A69DDBgU+nfTRx7m2vCUSNrlgevsMzy4HbTUjL/M=
-----END PUBLIC KEY-----";

    #[test]
    fn verifies_token_from_login_service() {
        use jsonwebtoken::{EncodingKey, Header, encode};
        let dir = std::env::temp_dir().join(format!("pingora-auth-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let key_path = dir.join("pub.pem");
        std::fs::write(&key_path, TEST_PUBLIC).unwrap();
        let auth = Authenticator::new(AuthConfig {
            login_url: "https://login.example.com/login".into(),
            cookie_name: "session".into(),
            issuer: "https://login.example.com".into(),
            audience: "home".into(),
            public_key_path: key_path.to_string_lossy().into(),
        })
        .unwrap();

        let sign = |iss: &str, exp: u64| {
            let claims = serde_json::json!({
                "sub": "42", "email": "me@example.com", "groups": ["family"],
                "iss": iss, "aud": "home", "exp": exp,
            });
            let key = EncodingKey::from_ed_pem(TEST_PRIVATE.as_bytes()).unwrap();
            encode(&Header::new(Algorithm::EdDSA), &claims, &key).unwrap()
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let request = |token: &str| {
            let mut req = RequestHeader::build("GET", b"/books?id=1", None).unwrap();
            req.append_header("cookie", format!("session={token}")).unwrap();
            req
        };

        let ok = auth.identify(&request(&sign("https://login.example.com", now + 600))).unwrap();
        assert_eq!(ok.email, "me@example.com");
        assert_eq!(ok.groups, ["family"]);
        assert!(auth.identify(&request(&sign("https://evil.example.com", now + 600))).is_none());
        assert!(auth.identify(&request(&sign("https://login.example.com", now - 600))).is_none());
        assert_eq!(
            auth.login_redirect("books.example.com", &request("x")),
            "https://login.example.com/login?rd=https%3A%2F%2Fbooks%2Eexample%2Ecom%2Fbooks%3Fid%3D1"
        );
        std::fs::remove_dir_all(dir).ok();
    }
}
