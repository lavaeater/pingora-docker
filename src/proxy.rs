use crate::auth::{AuthConfig, AuthMode, Authenticator, Claims, IDENTITY_HEADERS, header_value, is_allowed, wants_html};
use async_trait::async_trait;
use log::info;
use pingora::prelude::*;
use pingora::http::{RequestHeader, ResponseHeader};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for a backend service
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BackendConfig {
    /// Hostname or IP of the backend service (can be Docker container name)
    pub host: String,
    /// Port the backend service is listening on
    pub port: u16,
    /// Whether to use TLS when connecting to the backend
    #[serde(default)]
    pub tls: bool,
    /// SNI hostname for TLS connections (defaults to host if not specified)
    pub sni: Option<String>,
    /// Whether a login is needed to reach this backend (default: public)
    #[serde(default)]
    pub auth: AuthMode,
    /// Who may reach this backend: emails or `group:<name>`. Empty = any signed-in user.
    #[serde(default)]
    pub allow: Vec<String>,
}

/// TLS configuration for the proxy listener
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TlsConfig {
    /// Path to the certificate file (PEM format)
    pub cert_path: String,
    /// Path to the private key file (PEM format)
    pub key_path: String,
    /// Optional: Enable HTTP/2 (default: true)
    #[serde(default = "default_true")]
    pub enable_h2: bool,
    /// Optional: DuckDNS token for automatic certificate provisioning
    pub duckdns_token: Option<String>,
    /// Optional: Use Let's Encrypt production (default: false = staging)
    #[serde(default)]
    pub acme_production: bool,
    /// Optional: Seconds to wait for DNS propagation (default: 30)
    #[serde(default = "default_dns_wait")]
    pub dns_wait_seconds: u64,
}

fn default_dns_wait() -> u64 { 30 }

fn default_true() -> bool { true }

/// Main proxy configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProxyConfig {
    /// Address to listen on for HTTP (e.g., "0.0.0.0:8080")
    pub listen_addr: String,
    /// Optional: Address to listen on for HTTPS (e.g., "0.0.0.0:8443")
    pub tls_listen_addr: Option<String>,
    /// Optional: TLS configuration (required if tls_listen_addr is set)
    pub tls: Option<TlsConfig>,
    /// Enable debug mode - logs all requests with details (default: false)
    #[serde(default)]
    pub debug_mode: bool,
    /// Domain to backend mapping
    /// Key: domain name (e.g., "app1.cleverdomain.asuscomm.com")
    /// Value: backend configuration
    pub domains: HashMap<String, BackendConfig>,
    /// Default backend for unmatched domains (optional)
    pub default_backend: Option<BackendConfig>,
    /// Optional: single sign-on settings. Required if any backend uses `auth`.
    pub auth: Option<AuthConfig>,
}

/// Domain-based router that implements ProxyHttp
pub struct DomainRouter {
    config: ProxyConfig,
    auth: Option<Authenticator>,
}

/// Per-request state: the identity verified in `request_filter`, forwarded in
/// `upstream_request_filter`.
#[derive(Default)]
pub struct RequestCtx {
    identity: Option<Claims>,
}

impl DomainRouter {
    pub fn new(config: ProxyConfig) -> anyhow::Result<Self> {
        let auth = config.auth.clone().map(Authenticator::new).transpose()?;
        let protected = config
            .domains
            .iter()
            .filter(|(_, b)| b.auth != AuthMode::Public)
            .map(|(d, _)| d.as_str())
            .collect::<Vec<_>>();
        if auth.is_none() && !protected.is_empty() {
            anyhow::bail!("domains {protected:?} use auth, but config.json has no \"auth\" section");
        }
        Ok(Self { config, auth })
    }

    async fn redirect(session: &mut Session, location: &str) -> Result<()> {
        let mut resp = ResponseHeader::build(302, None)?;
        resp.insert_header("Location", location)?;
        resp.insert_header("Content-Length", "0")?;
        resp.insert_header("Cache-Control", "no-store")?;
        session.write_response_header(Box::new(resp), true).await
    }

    /// Extract the host from the request, handling both Host header and :authority pseudo-header
    fn get_host_from_session(&self, session: &Session) -> Option<String> {
        let req_header = session.req_header();
        
        // Try Host header first (HTTP/1.1)
        if let Some(host) = req_header.headers.get("host") {
            if let Ok(host_str) = host.to_str() {
                // Strip port if present (e.g., "domain.com:8080" -> "domain.com")
                let host_without_port = host_str.split(':').next().unwrap_or(host_str);
                return Some(host_without_port.to_lowercase());
            }
        }
        
        // Try :authority pseudo-header (HTTP/2)
        if let Some(authority) = req_header.headers.get(":authority") {
            if let Ok(auth_str) = authority.to_str() {
                let host_without_port = auth_str.split(':').next().unwrap_or(auth_str);
                return Some(host_without_port.to_lowercase());
            }
        }
        
        // Try URI host as last resort
        if let Some(host) = req_header.uri.host() {
            return Some(host.to_lowercase());
        }
        
        None
    }

    /// Find the backend for a given host
    fn find_backend(&self, host: &str) -> Option<&BackendConfig> {
        // Exact match first
        if let Some(backend) = self.config.domains.get(host) {
            return Some(backend);
        }
        
        // Try wildcard match (e.g., "*.example.com" matches "app.example.com")
        for (domain, backend) in &self.config.domains {
            if domain.starts_with("*.") {
                let suffix = &domain[1..]; // ".example.com"
                if host.ends_with(suffix) {
                    return Some(backend);
                }
            }
        }
        
        // Fall back to default backend
        self.config.default_backend.as_ref()
    }
}

#[async_trait]
impl ProxyHttp for DomainRouter {
    type CTX = RequestCtx;

    fn new_ctx(&self) -> Self::CTX {
        RequestCtx::default()
    }

    /// Runs before routing. Returning `Ok(true)` means a response was already sent.
    async fn request_filter(&self, session: &mut Session, ctx: &mut Self::CTX) -> Result<bool> {
        let Some(host) = self.get_host_from_session(session) else {
            return Ok(false);
        };
        let Some(backend) = self.find_backend(&host) else {
            return Ok(false); // upstream_peer reports the missing backend
        };
        let Some(auth) = &self.auth else {
            return Ok(false); // no auth configured, so every backend is public
        };

        match backend.auth {
            AuthMode::Public => Ok(false),
            AuthMode::Optional => {
                ctx.identity = auth.identify(session.req_header());
                Ok(false)
            }
            AuthMode::Required => match auth.identify(session.req_header()) {
                Some(claims) if is_allowed(&claims, &backend.allow) => {
                    ctx.identity = Some(claims);
                    Ok(false)
                }
                Some(claims) => {
                    info!("{} is not allowed on {}", claims.email, host);
                    session.respond_error(403).await?;
                    Ok(true)
                }
                None if wants_html(session.req_header()) => {
                    let location = auth.login_redirect(&host, session.req_header());
                    Self::redirect(session, &location).await?;
                    Ok(true)
                }
                None => {
                    session.respond_error(401).await?;
                    Ok(true)
                }
            },
        }
    }

    async fn upstream_peer(
        &self,
        session: &mut Session,
        _ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>> {
        let req = session.req_header();
        let host = self.get_host_from_session(session)
            .unwrap_or_else(|| "unknown".to_string());
        let path = req.uri.path();
        let method = &req.method;
        
        // Always log incoming requests
        println!("=== INCOMING REQUEST ===");
        println!("  Host: {}", host);
        println!("  Method: {}", method);
        println!("  Path: {}", path);
        if let Some(client) = session.client_addr() {
            println!("  Client IP: {}", client);
        }
        if self.config.debug_mode {
            println!("  Headers:");
            for (name, value) in req.headers.iter() {
                if let Ok(v) = value.to_str() {
                    println!("    {}: {}", name, v);
                }
            }
        }
        println!("========================");
        
        info!("Incoming request for host: {}", host);
        
        let backend = match self.find_backend(&host) {
            Some(b) => b,
            None => {
                println!(">>> NO BACKEND for host: {} - check your config.json domains", host);
                return Err(pingora::Error::new_str("No backend configured for host"));
            }
        };
        
        let upstream_addr = format!("{}:{}", backend.host, backend.port);
        info!("Routing {} -> {}", host, upstream_addr);
        
        // Create the peer with appropriate TLS settings
        let sni = backend.sni.clone().unwrap_or_else(|| backend.host.clone());
        let peer = Box::new(HttpPeer::new(
            upstream_addr.as_str(),
            backend.tls,
            sni,
        ));
        
        Ok(peer)
    }

    async fn upstream_request_filter(
        &self,
        session: &mut Session,
        upstream_request: &mut RequestHeader,
        ctx: &mut Self::CTX,
    ) -> Result<()> {
        // Never let a client supply its own identity headers.
        for name in IDENTITY_HEADERS {
            upstream_request.remove_header(name);
        }
        if let Some(id) = &ctx.identity {
            upstream_request.insert_header("X-Auth-Subject", header_value(&id.sub))?;
            upstream_request.insert_header("X-Auth-Email", header_value(&id.email))?;
            if let Some(name) = &id.name {
                upstream_request.insert_header("X-Auth-Name", header_value(name))?;
            }
            if !id.groups.is_empty() {
                upstream_request.insert_header("X-Auth-Groups", header_value(&id.groups.join(",")))?;
            }
        }

        // Preserve the original Host header for the backend
        // This is important for backends that use virtual hosting
        if let Some(host) = session.req_header().headers.get("host") {
            if let Ok(host_str) = host.to_str() {
                upstream_request.insert_header("Host", host_str)?;
            }
        }
        
        // Add X-Forwarded headers for the backend to know the original request details.
        // X-Forwarded-For carries the bare IP: SocketAddr's Display includes the port,
        // which backends reject as an invalid address.
        if let Some(ip) = session.client_addr().and_then(|a| a.as_inet()).map(|a| a.ip()) {
            upstream_request.insert_header("X-Forwarded-For", ip.to_string())?;
        }
        let is_tls = session
            .digest()
            .is_some_and(|d| d.ssl_digest.is_some());
        upstream_request.insert_header("X-Forwarded-Proto", if is_tls { "https" } else { "http" })?;
        
        Ok(())
    }
}
