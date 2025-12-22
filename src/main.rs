mod acme;
mod proxy;

use crate::acme::{cert_covers_domains, provision_certificates, AcmeConfig};
use crate::proxy::{DomainRouter, ProxyConfig};
use log::info;
use pingora::listeners::tls::TlsSettings;
use pingora::prelude::*;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

fn main() {
    // Install the ring crypto provider for rustls before any TLS operations
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    env_logger::init();

    // Load configuration from JSON file
    let file = File::open("config.json").expect("Failed to open config file");
    let reader = BufReader::new(file);
    let config: ProxyConfig = serde_json::from_reader(reader)
        .expect("Failed to parse config file");

    // Check if we need to provision certificates
    if let Some(tls_config) = &config.tls {
        if let Some(duckdns_token) = &tls_config.duckdns_token {
            let domains: Vec<String> = config.domains.keys().cloned().collect();
            let cert_path = PathBuf::from(&tls_config.cert_path);
            
            if !cert_covers_domains(&cert_path, &domains) {
                info!("Certificate needs to be provisioned for domains: {:?}", domains);
                
                let acme_config = AcmeConfig {
                    domains,
                    duckdns_token: duckdns_token.clone(),
                    cert_path: cert_path.clone(),
                    key_path: PathBuf::from(&tls_config.key_path),
                    production: tls_config.acme_production,
                    dns_wait_seconds: tls_config.dns_wait_seconds,
                    account_path: Some(cert_path.parent().unwrap_or(&PathBuf::from(".")).join("account.json")),
                };
                
                // Run the async provisioning in a blocking context
                let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
                rt.block_on(async {
                    if let Err(e) = provision_certificates(&acme_config).await {
                        eprintln!("Failed to provision certificates: {}", e);
                        eprintln!("Continuing with existing certificates if available...");
                    }
                });
            }
        }
    }

    let mut my_server = Server::new(None).unwrap();
    my_server.bootstrap();

    // Create the domain router with our configuration
    let router = DomainRouter::new(config.clone());
    
    let mut proxy_service = http_proxy_service(&my_server.configuration, router);
    
    // Add HTTP listener
    proxy_service.add_tcp(&config.listen_addr);
    println!("HTTP listener on {}", config.listen_addr);

    // Add HTTPS listener if TLS is configured
    if let (Some(tls_addr), Some(tls_config)) = (&config.tls_listen_addr, &config.tls) {
        let mut tls_settings = TlsSettings::intermediate(&tls_config.cert_path, &tls_config.key_path)
            .expect("Failed to load TLS certificates");
        
        if tls_config.enable_h2 {
            tls_settings.enable_h2();
        }
        
        proxy_service.add_tls_with_settings(tls_addr, None, tls_settings);
        println!("HTTPS listener on {}", tls_addr);
    }

    println!("Configured domains:");
    for (domain, backend) in &config.domains {
        println!("  {} -> {}:{} (tls to backend: {})", domain, backend.host, backend.port, backend.tls);
    }

    my_server.add_service(proxy_service);
    my_server.run_forever();
}
