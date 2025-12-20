mod proxy;

use crate::proxy::{DomainRouter, ProxyConfig};
use pingora::listeners::tls::TlsSettings;
use pingora::prelude::*;
use std::fs::File;
use std::io::BufReader;

fn main() {
    env_logger::init();

    // Load configuration from JSON file
    let file = File::open("config.json").expect("Failed to open config file");
    let reader = BufReader::new(file);
    let config: ProxyConfig = serde_json::from_reader(reader)
        .expect("Failed to parse config file");

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
