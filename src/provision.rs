use std::time::Duration;

use clap::Parser;
use tracing::{info, warn};

use instant_acme::{
    Account, AuthorizationStatus, ChallengeType, Identifier, LetsEncrypt, NewAccount, NewOrder,
    OrderStatus, RetryPolicy,
};

/// Set TXT record via DuckDNS API
async fn set_duckdns_txt(domain: &str, token: &str, txt_value: &str) -> anyhow::Result<()> {
    // Extract the subdomain part (e.g., "jelly-tea" from "jelly-tea.duckdns.org")
    let subdomain = domain
        .strip_suffix(".duckdns.org")
        .unwrap_or(domain);
    
    let url = format!(
        "https://www.duckdns.org/update?domains={}&token={}&txt={}&verbose=true",
        subdomain, token, txt_value
    );
    
    info!("Setting DuckDNS TXT record for {}", subdomain);
    
    let response = reqwest::get(&url).await?.text().await?;
    
    if response.starts_with("OK") {
        info!("DuckDNS TXT record set successfully: {}", response.replace('\n', " "));
        Ok(())
    } else {
        Err(anyhow::anyhow!("DuckDNS API error: {}", response))
    }
}

/// Clear TXT record via DuckDNS API
async fn clear_duckdns_txt(domain: &str, token: &str) -> anyhow::Result<()> {
    let subdomain = domain
        .strip_suffix(".duckdns.org")
        .unwrap_or(domain);
    
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
        Ok(()) // Don't fail on cleanup
    }
}

/// Wait for DNS propagation
async fn wait_for_dns_propagation(seconds: u64) {
    info!("Waiting {}s for DNS propagation...", seconds);
    tokio::time::sleep(Duration::from_secs(seconds)).await;
}

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    // Install the ring crypto provider for rustls
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");
    
    tracing_subscriber::fmt::init();
    let opts = Options::parse();

    info!("Starting certificate provisioning for: {:?}", opts.domains);

    // Create a new ACME account
    let (account, credentials) = Account::builder()?
        .create(
            &NewAccount {
                contact: &[],
                terms_of_service_agreed: true,
                only_return_existing: false,
            },
            if opts.production {
                LetsEncrypt::Production.url().to_owned()
            } else {
                info!("Using Let's Encrypt STAGING environment (use --production for real certs)");
                LetsEncrypt::Staging.url().to_owned()
            },
            None,
        )
        .await?;
    
    info!("ACME account created");

    // Create the ACME order for all domains
    let identifiers = opts
        .domains
        .iter()
        .map(|d| Identifier::Dns(d.clone()))
        .collect::<Vec<_>>();
    
    let mut order = account
        .new_order(&NewOrder::new(identifiers.as_slice()))
        .await?;

    info!("Order created, status: {:?}", order.state().status);

    // Process each authorization (one per domain)
    let mut authorizations = order.authorizations();
    while let Some(result) = authorizations.next().await {
        let mut authz = result?;
        
        match authz.status {
            AuthorizationStatus::Pending => {}
            AuthorizationStatus::Valid => {
                info!("Authorization already valid, skipping");
                continue;
            }
            status => {
                return Err(anyhow::anyhow!("Unexpected authorization status: {:?}", status));
            }
        }

        // Get the DNS-01 challenge
        let mut challenge = authz
            .challenge(ChallengeType::Dns01)
            .ok_or_else(|| anyhow::anyhow!("No DNS-01 challenge found"))?;

        let domain = challenge.identifier().to_string();
        let txt_value = challenge.key_authorization().dns_value();
        
        info!("Processing challenge for: {}", domain);
        info!("TXT record value: {}", txt_value);

        // Set the TXT record via DuckDNS API
        set_duckdns_txt(&domain, &opts.duckdns_token, &txt_value).await?;
        
        // Wait for DNS propagation (DuckDNS is usually fast, but Let's Encrypt needs time)
        wait_for_dns_propagation(opts.dns_wait).await;

        // Tell ACME server we're ready
        challenge.set_ready().await?;
        info!("Challenge marked ready for {}", domain);
    }

    // Wait for order to become ready
    info!("Waiting for order to become ready...");
    let status = order.poll_ready(&RetryPolicy::default()).await?;
    
    if status != OrderStatus::Ready {
        return Err(anyhow::anyhow!("Order failed with status: {:?}", status));
    }

    // Finalize the order and get the certificate
    info!("Finalizing order...");
    let private_key_pem = order.finalize().await?;
    let cert_chain_pem = order.poll_certificate(&RetryPolicy::default()).await?;

    // Save certificate and key to files
    let cert_path = opts.output_dir.join("cert.pem");
    let key_path = opts.output_dir.join("key.pem");
    
    std::fs::create_dir_all(&opts.output_dir)?;
    std::fs::write(&cert_path, &cert_chain_pem)?;
    std::fs::write(&key_path, &private_key_pem)?;
    
    info!("Certificate saved to: {}", cert_path.display());
    info!("Private key saved to: {}", key_path.display());

    // Save account credentials for renewal
    let creds_path = opts.output_dir.join("account.json");
    std::fs::write(&creds_path, serde_json::to_string_pretty(&credentials)?)?;
    info!("Account credentials saved to: {}", creds_path.display());

    // Clean up TXT records
    for domain in &opts.domains {
        clear_duckdns_txt(domain, &opts.duckdns_token).await?;
    }

    println!("\n✅ Certificate provisioning complete!");
    println!("   Certificate: {}", cert_path.display());
    println!("   Private key: {}", key_path.display());
    
    Ok(())
}

#[derive(Parser)]
#[clap(name = "provision", about = "Provision Let's Encrypt certificates using DuckDNS")]
pub struct Options {
    /// Domain names to provision (e.g., jelly-tea.duckdns.org)
    #[clap(long, required = true)]
    domains: Vec<String>,
    
    /// DuckDNS API token
    #[clap(long)]
    duckdns_token: String,
    
    /// Output directory for certificates
    #[clap(long, default_value = "./certs")]
    output_dir: std::path::PathBuf,
    
    /// Use production Let's Encrypt (default is staging)
    #[clap(long)]
    production: bool,
    
    /// Seconds to wait for DNS propagation
    #[clap(long, default_value = "30")]
    dns_wait: u64,
}