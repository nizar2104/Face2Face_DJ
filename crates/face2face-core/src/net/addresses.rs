//! This computer's addresses on its networks: what the partner types to link.

use serde::Serialize;

/// Mirrored in `app/src/tauri.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LocalAddress {
    /// An IPv4 address, e.g. "192.168.1.70".
    pub address: String,
    /// The network it is on as the system names it, e.g. "Wi-Fi" or "en0".
    pub interface: String,
}

/// The IPv4 addresses a partner on the same network could reach, the one on
/// the default route first. Loopback, VPN tunnels and interfaces that are
/// down are left out.
pub fn local_addresses() -> Vec<LocalAddress> {
    let mut interfaces: Vec<_> = netdev::get_interfaces()
        .into_iter()
        .filter(|interface| interface.is_up() && interface.is_running() && !interface.is_loopback() && !interface.is_tun())
        .collect();
    interfaces.sort_by_key(|interface| !interface.default);
    interfaces
        .iter()
        .flat_map(|interface| {
            let name = interface.friendly_name.clone().unwrap_or_else(|| interface.name.clone());
            interface.ipv4.iter().map(move |net| LocalAddress { address: net.addr().to_string(), interface: name.clone() })
        })
        .collect()
}
