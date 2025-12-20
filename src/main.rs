mod proxy;

use crate::proxy::{DomainRouter, ProxyConfig};
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
    proxy_service.add_tcp(&config.listen_addr);

    println!("Starting reverse proxy on {}", config.listen_addr);
    println!("Configured domains:");
    for (domain, backend) in &config.domains {
        println!("  {} -> {}:{} (tls: {})", domain, backend.host, backend.port, backend.tls);
    }

    my_server.add_service(proxy_service);
    my_server.run_forever();
}
