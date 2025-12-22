use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use log::{info, warn};

use instant_acme::{
    Account, AccountCredentials, AuthorizationStatus, ChallengeType, Identifier, LetsEncrypt,
    NewAccount, NewOrder, OrderStatus, RetryPolicy,
};

/// Check if the certificate at the given path covers all the required domains
pub fn cert_covers_domains(cert_path: &Path, required_domains: &[String]) -> bool {
    let cert_data = match std::fs::read(cert_path) {
        Ok(data) => data,
        Err(_) => {
            info!("No existing certificate found at {}", cert_path.display());
            return false;
        }
    };

    let cert_pem = match std::str::from_utf8(&cert_data) {
        Ok(s) => s,
        Err(_) => return false,
    };

    // Parse the certificate to extract SANs
    let cert_sans = match extract_sans_from_pem(cert_pem) {
        Some(sans) => sans,
        None => {
            warn!("Could not parse certificate SANs");
            return false;
        }
    };

    let required_set: HashSet<&str> = required_domains.iter().map(|s| s.as_str()).collect();
    let cert_set: HashSet<&str> = cert_sans.iter().map(|s| s.as_str()).collect();

    if required_set == cert_set {
        info!("Certificate covers all required domains");
        true
    } else {
        info!(
            "Certificate domain mismatch. Required: {:?}, Have: {:?}",
            required_set, cert_set
        );
        false
    }
}

/// Extract Subject Alternative Names from a PEM certificate
fn extract_sans_from_pem(pem_data: &str) -> Option<Vec<String>> {
    use rustls_pemfile::certs;
    use std::io::BufReader;

    let mut reader = BufReader::new(pem_data.as_bytes());
    let certs: Vec<_> = certs(&mut reader).filter_map(|r| r.ok()).collect();

    if certs.is_empty() {
        return None;
    }

    // Parse the first certificate using x509-parser
    let (_, cert) = x509_parser::parse_x509_certificate(&certs[0]).ok()?;

    let mut sans = Vec::new();

    // Get Subject Alternative Names extension
    for ext in cert.extensions() {
        if let x509_parser::extensions::ParsedExtension::SubjectAlternativeName(san) =
            ext.parsed_extension()
        {
            for name in &san.general_names {
                if let x509_parser::extensions::GeneralName::DNSName(dns) = name {
                    sans.push(dns.to_string());
                }
            }
        }
    }

    Some(sans)
}

/// Configuration for ACME certificate provisioning
#[derive(Debug, Clone)]
pub struct AcmeConfig {
    pub domains: Vec<String>,
    pub duckdns_token: String,
    pub cert_path: std::path::PathBuf,
    pub key_path: std::path::PathBuf,
    pub production: bool,
    pub dns_wait_seconds: u64,
    pub account_path: Option<std::path::PathBuf>,
}

/// Provision certificates for the given domains using ACME DNS-01 challenge
/// Note: Due to DuckDNS limitation (one TXT record per subdomain), we provision
/// each domain separately. Each domain gets its own cert file (domain_cert.pem, domain_key.pem).
/// The first domain's cert is also saved as the default cert.pem/key.pem.
pub async fn provision_certificates(config: &AcmeConfig) -> anyhow::Result<()> {
    info!("Starting certificate provisioning for: {:?}", config.domains);

    // Try to load existing account credentials, or create new account
    let account = match &config.account_path {
        Some(path) if path.exists() => {
            let creds_json = std::fs::read_to_string(path)?;
            let creds: AccountCredentials = serde_json::from_str(&creds_json)?;
            info!("Loaded existing ACME account");
            Account::builder()?.from_credentials(creds).await?
        }
        _ => {
            let (account, credentials) = Account::builder()?
                .create(
                    &NewAccount {
                        contact: &[],
                        terms_of_service_agreed: true,
                        only_return_existing: false,
                    },
                    if config.production {
                        LetsEncrypt::Production.url().to_owned()
                    } else {
                        info!("Using Let's Encrypt STAGING environment");
                        LetsEncrypt::Staging.url().to_owned()
                    },
                    None,
                )
                .await?;

            // Save account credentials for future use
            if let Some(path) = &config.account_path {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, serde_json::to_string_pretty(&credentials)?)?;
                info!("Saved ACME account credentials to {}", path.display());
            }

            info!("Created new ACME account");
            account
        }
    };

    // Process each domain separately due to DuckDNS TXT record limitation
    let cert_dir = config.cert_path.parent().unwrap_or(Path::new("."));
    let mut first_cert_saved = false;

    for (i, domain) in config.domains.iter().enumerate() {
        info!("Processing domain {}/{}: {}", i + 1, config.domains.len(), domain);

        // Create order for single domain
        let identifier = Identifier::Dns(domain.clone());
        let mut order = account
            .new_order(&NewOrder::new(&[identifier]))
            .await?;

        info!("Order created for {}, status: {:?}", domain, order.state().status);

        // Process authorization
        let mut authorizations = order.authorizations();
        if let Some(result) = authorizations.next().await {
            let mut authz = result?;

            if authz.status == AuthorizationStatus::Pending {
                let mut challenge = authz
                    .challenge(ChallengeType::Dns01)
                    .ok_or_else(|| anyhow::anyhow!("No DNS-01 challenge found"))?;

                let txt_value = challenge.key_authorization().dns_value();

                info!("Setting TXT record for {}", domain);
                set_duckdns_txt(domain, &config.duckdns_token, &txt_value).await?;

                wait_for_dns_propagation(config.dns_wait_seconds).await;

                challenge.set_ready().await?;
                info!("Challenge marked ready for {}", domain);
            }
        }
        drop(authorizations);

        // Wait for order to become ready with longer timeout (2 minutes)
        info!("Waiting for order to become ready for {}...", domain);
        let retry_policy = RetryPolicy::new()
            .initial_delay(Duration::from_secs(3))
            .backoff(1.5)
            .timeout(Duration::from_secs(120));
        let status = order.poll_ready(&retry_policy).await?;

        if status != OrderStatus::Ready {
            warn!("Order failed for {} with status: {:?}, skipping", domain, status);
            clear_duckdns_txt(domain, &config.duckdns_token).await?;
            continue;
        }

        // Finalize and get certificate
        info!("Finalizing order for {}...", domain);
        let key_pem = order.finalize().await?;
        let cert_pem = order.poll_certificate(&retry_policy).await?;

        // Save domain-specific cert files
        let subdomain = domain.strip_suffix(".duckdns.org").unwrap_or(domain);
        let domain_cert_path = cert_dir.join(format!("{}_cert.pem", subdomain));
        let domain_key_path = cert_dir.join(format!("{}_key.pem", subdomain));
        
        std::fs::create_dir_all(cert_dir)?;
        std::fs::write(&domain_cert_path, &cert_pem)?;
        std::fs::write(&domain_key_path, &key_pem)?;
        info!("Certificate for {} saved to {}", domain, domain_cert_path.display());

        // Save first successful cert as the default
        if !first_cert_saved {
            std::fs::write(&config.cert_path, &cert_pem)?;
            std::fs::write(&config.key_path, &key_pem)?;
            info!("Default certificate saved to: {}", config.cert_path.display());
            first_cert_saved = true;
        }

        info!("Certificate obtained for {}", domain);
        clear_duckdns_txt(domain, &config.duckdns_token).await?;
    }

    if !first_cert_saved {
        return Err(anyhow::anyhow!("Failed to provision any certificates"));
    }

    info!("Certificate provisioning complete!");
    Ok(())
}

/// Set TXT record via DuckDNS API
async fn set_duckdns_txt(domain: &str, token: &str, txt_value: &str) -> anyhow::Result<()> {
    let subdomain = domain.strip_suffix(".duckdns.org").unwrap_or(domain);

    let url = format!(
        "https://www.duckdns.org/update?domains={}&token={}&txt={}&verbose=true",
        subdomain, token, txt_value
    );

    info!("Setting DuckDNS TXT record for {}", subdomain);

    let response = reqwest::get(&url).await?.text().await?;

    if response.starts_with("OK") {
        info!(
            "DuckDNS TXT record set successfully: {}",
            response.replace('\n', " ")
        );
        Ok(())
    } else {
        Err(anyhow::anyhow!("DuckDNS API error: {}", response))
    }
}

/// Clear TXT record via DuckDNS API
async fn clear_duckdns_txt(domain: &str, token: &str) -> anyhow::Result<()> {
    let subdomain = domain.strip_suffix(".duckdns.org").unwrap_or(domain);

    let url = format!(
        "https://www.duckdns.org/update?domains={}&token={}&txt=&clear=true",
        subdomain, token
    );

    let response = reqwest::get(&url).await?.text().await?;

    if response.starts_with("OK") {
        info!("DuckDNS TXT record cleared for {}", subdomain);
        Ok(())
    } else {
        warn!("Failed to clear TXT record: {}", response);
        Ok(())
    }
}

/// Wait for DNS propagation
async fn wait_for_dns_propagation(seconds: u64) {
    info!("Waiting {}s for DNS propagation...", seconds);
    tokio::time::sleep(Duration::from_secs(seconds)).await;
}
