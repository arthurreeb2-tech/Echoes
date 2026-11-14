//! Annonce et recherche des machines sur le réseau local (Bonjour / mDNS).

use crate::proto::SERVICE;
use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

/// Annonce cette machine ; l'annonce dure tant que la valeur renvoyée vit.
pub fn advertise(name: &str, id: &str, port: u16) -> Result<ServiceDaemon> {
    let daemon = ServiceDaemon::new()?;
    let host = format!("echoes-{id}.local.");
    let info = ServiceInfo::new(SERVICE, &format!("{name}-{id}"), &host, "", port, None)?.enable_addr_auto();
    daemon.register(info)?;
    Ok(daemon)
}

pub fn browse(wait: Duration) -> Vec<SocketAddr> {
    let mut found = Vec::new();
    let Ok(daemon) = ServiceDaemon::new() else { return found };
    let Ok(events) = daemon.browse(SERVICE) else { return found };
    let end = Instant::now() + wait;
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        match events.recv_timeout(left) {
            Ok(ServiceEvent::ServiceResolved(svc)) => {
                for ip in svc.get_addresses_v4() {
                    let addr = SocketAddr::from((ip, svc.port));
                    if !found.contains(&addr) {
                        found.push(addr);
                    }
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let _ = daemon.shutdown();
    found
}
