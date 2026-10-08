//! Network helpers: local interfaces, choosing which of a computer's
//! addresses to try, racing connection attempts, and the host list for a
//! LAN sweep.
//!
//! Computers often advertise addresses that are unreachable from elsewhere
//! (Hyper-V/WSL/Docker/VirtualBox virtual adapters, VPN tunnels, link-local
//! addresses). Trying those one after another, each with a connect timeout,
//! made connecting slow or appear to fail, so addresses on the same subnet as
//! one of ours are tried first and all candidates are raced.

use anyhow::{anyhow, Result};
use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::task::JoinSet;

use crate::transport;

/// Delay between starting connection attempts to successive addresses.
const STAGGER: Duration = Duration::from_millis(250);
/// Largest number of hosts a sweep will probe.
const MAX_SWEEP_HOSTS: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalNet {
    pub ip: Ipv4Addr,
    pub prefix: u8,
}

impl LocalNet {
    fn mask(&self) -> u32 {
        if self.prefix == 0 {
            0
        } else {
            u32::MAX << (32 - self.prefix.min(32) as u32)
        }
    }

    pub fn contains(&self, other: Ipv4Addr) -> bool {
        let m = self.mask();
        u32::from(self.ip) & m == u32::from(other) & m
    }
}

/// IPv4 addresses of this computer's active, non-loopback interfaces.
pub fn local_ipv4() -> Vec<LocalNet> {
    let Ok(ifaces) = if_addrs::get_if_addrs() else { return Vec::new() };
    let mut out = Vec::new();
    for i in ifaces {
        if i.is_loopback() || i.is_link_local() || !i.is_oper_up() {
            continue;
        }
        if let if_addrs::IfAddr::V4(v4) = &i.addr {
            let net = LocalNet { ip: v4.ip, prefix: v4.prefixlen };
            if !out.contains(&net) {
                out.push(net);
            }
        }
    }
    out
}

fn usable(a: &SocketAddr) -> bool {
    match a.ip() {
        IpAddr::V4(v4) => !v4.is_unspecified() && !v4.is_link_local() && !v4.is_broadcast() && !v4.is_multicast(),
        // Link-local IPv6 needs an interface scope that discovery doesn't give us.
        IpAddr::V6(v6) => !v6.is_unspecified() && !v6.is_multicast() && (v6.segments()[0] & 0xffc0) != 0xfe80,
    }
}

/// Order candidate addresses for connecting: same-subnet IPv4 first, then
/// other IPv4, then IPv6. Unusable and duplicate addresses are dropped.
pub fn order_candidates(addrs: Vec<SocketAddr>, local: &[LocalNet]) -> Vec<SocketAddr> {
    let mut seen = HashSet::new();
    let mut v: Vec<SocketAddr> = addrs.into_iter().filter(|a| usable(a) && seen.insert(*a)).collect();
    v.sort_by_key(|a| match a.ip() {
        IpAddr::V4(ip) if ip.is_loopback() || local.iter().any(|n| n.contains(ip)) => 0,
        IpAddr::V4(_) => 1,
        IpAddr::V6(_) => 2,
    });
    v
}

/// Connect to whichever address answers first. Attempts start a little
/// apart (best candidates first). If none succeeds, the error of the best
/// candidate is returned, since it is the most meaningful one.
pub async fn connect_any(addrs: Vec<SocketAddr>) -> Result<(TcpStream, SocketAddr)> {
    if addrs.is_empty() {
        return Err(anyhow!("no usable network address for that computer"));
    }
    let mut set = JoinSet::new();
    for (i, addr) in addrs.iter().copied().enumerate() {
        set.spawn(async move {
            tokio::time::sleep(STAGGER * i as u32).await;
            (i, addr, transport::connect(addr).await)
        });
    }
    let mut errors: Vec<(usize, anyhow::Error)> = Vec::new();
    while let Some(res) = set.join_next().await {
        match res {
            Ok((_, addr, Ok(stream))) => {
                set.abort_all();
                return Ok((stream, addr));
            }
            Ok((i, _, Err(e))) => errors.push((i, e)),
            Err(_) => {}
        }
    }
    errors.sort_by_key(|(i, _)| *i);
    Err(errors.into_iter().next().map(|(_, e)| e).unwrap_or_else(|| anyhow!("no address answered")))
}

/// Hosts to probe when sweeping the local network: every address in the
/// subnet of each local interface, limited to the /24 around our own address
/// for larger networks.
pub fn sweep_hosts(local: &[LocalNet]) -> Vec<Ipv4Addr> {
    let own: HashSet<Ipv4Addr> = local.iter().map(|n| n.ip).collect();
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for n in local {
        let prefix = n.prefix.max(24);
        let net = LocalNet { ip: n.ip, prefix };
        let base = u32::from(n.ip) & net.mask();
        let size = 1u32 << (32 - prefix as u32);
        for off in 1..size.saturating_sub(1) {
            let ip = Ipv4Addr::from(base + off);
            if !own.contains(&ip) && seen.insert(ip) {
                out.push(ip);
            }
            if out.len() >= MAX_SWEEP_HOSTS {
                return out;
            }
        }
    }
    out
}

/// This Mac's macOS version, e.g. "15.3.1". `None` on other systems.
pub fn macos_version() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        static VERSION: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        VERSION
            .get_or_init(|| {
                let out = std::process::Command::new("/usr/bin/sw_vers").arg("-productVersion").output().ok()?;
                let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
                (out.status.success() && !v.is_empty()).then_some(v)
            })
            .clone()
    }
    #[cfg(not(target_os = "macos"))]
    None
}

/// Whether macOS's Local Network privacy can block this computer's
/// outgoing connections: macOS 15 and later. Assumed when the version is
/// unknown.
pub fn local_network_privacy(local_os: crate::keys::OsKind) -> bool {
    let major = || macos_version()?.split('.').next()?.parse::<u32>().ok();
    local_os == crate::keys::OsKind::Macos && major().is_none_or(|v| v >= 15)
}

/// Whether a connection error is how macOS reports that Local Network
/// privacy blocked it ("No route to host").
pub fn blocked_by_local_network(raw: &str, local_os: crate::keys::OsKind) -> bool {
    let e = raw.to_ascii_lowercase();
    local_network_privacy(local_os) && (e.contains("no route to host") || e.contains("os error 65"))
}

/// Plain-language explanation of a connection error, with what to check.
pub fn friendly_error(raw: &str, local_os: crate::keys::OsKind) -> String {
    let e = raw.to_ascii_lowercase();
    if blocked_by_local_network(raw, local_os) {
        return "macOS blocked the connection. Switch Skerry on in System Settings → Privacy & Security → Local \
                Network (if it's already on, switch it off and on again). Until then, connect from the other \
                computer instead: it can still reach this Mac (to pair, click Scan network there, then Pair). If \
                that's not it, check that the other computer is on and still has this address."
            .into();
    }
    if e.contains("timed out") {
        "Couldn't reach it (no answer). Make sure Skerry is running on that computer and that its firewall allows \
         Skerry (Windows: Settings → Windows Security → Firewall → Allow an app)."
            .into()
    } else if e.contains("refused") {
        "Connection refused: Skerry isn't running on that computer, or it uses a different port.".into()
    } else if e.contains("no route to host") || e.contains("os error 65") {
        "No route to that computer. Check that both computers are on the same network and that it's still on.".into()
    } else if e.contains("network is unreachable") || e.contains("host is unreachable") {
        "That computer isn't reachable from this network. Check that both are on the same network.".into()
    } else if e.contains("no usable network address") || e.contains("cannot resolve") {
        "Couldn't find an address for that computer. Try Scan network, or pair by its IP address.".into()
    } else {
        raw.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subnet_ordering_prefers_local_network() {
        let local = vec![LocalNet { ip: "192.168.1.20".parse().unwrap(), prefix: 24 }];
        let addrs: Vec<SocketAddr> = vec![
            "172.20.48.1:24870".parse().unwrap(),   // WSL / Hyper-V adapter
            "[fe80::1]:24870".parse().unwrap(),     // link-local: dropped
            "192.168.56.1:24870".parse().unwrap(),  // VirtualBox host-only
            "192.168.1.40:24870".parse().unwrap(),  // the real LAN address
            "[2001:db8::5]:24870".parse().unwrap(), // IPv6
            "169.254.3.4:24870".parse().unwrap(),   // APIPA: dropped
            "192.168.1.40:24870".parse().unwrap(),  // duplicate
        ];
        let o = order_candidates(addrs, &local);
        assert_eq!(o[0], "192.168.1.40:24870".parse().unwrap());
        assert_eq!(o.len(), 4);
        assert!(o[3].is_ipv6());
    }

    #[test]
    fn sweep_covers_slash_24_and_skips_self() {
        let local = vec![LocalNet { ip: "10.1.2.3".parse().unwrap(), prefix: 16 }];
        let h = sweep_hosts(&local);
        assert_eq!(h.len(), 253);
        assert!(h.contains(&"10.1.2.1".parse().unwrap()));
        assert!(h.contains(&"10.1.2.254".parse().unwrap()));
        assert!(!h.contains(&"10.1.2.3".parse().unwrap()));
        assert!(!h.contains(&"10.1.3.1".parse().unwrap()));
        let small = sweep_hosts(&[LocalNet { ip: "192.168.5.9".parse().unwrap(), prefix: 28 }]);
        assert_eq!(small.len(), 13);
    }

    #[tokio::test]
    async fn connect_any_uses_the_address_that_answers() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let good = listener.local_addr().unwrap();
        // A closed port first, then the listening one.
        let closed = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap()
        };
        let (_, used) = connect_any(vec![closed, good]).await.unwrap();
        assert_eq!(used, good);
        assert!(connect_any(vec![closed]).await.is_err());
    }

    #[test]
    fn friendly_errors() {
        use crate::keys::OsKind;
        assert!(friendly_error("timed out connecting to 1.2.3.4:24870", OsKind::Linux).contains("firewall"));
        assert!(friendly_error("No route to host (os error 65)", OsKind::Macos).contains("Local Network"));
        assert!(!friendly_error("No route to host (os error 113)", OsKind::Linux).contains("Local Network"));
        assert!(blocked_by_local_network(
            "connecting to 10.0.0.2:24870: No route to host (os error 65)",
            OsKind::Macos
        ));
        assert!(!blocked_by_local_network("timed out connecting to 10.0.0.2:24870", OsKind::Macos));
        assert!(!blocked_by_local_network("No route to host (os error 113)", OsKind::Linux));
        assert!(friendly_error("Connection refused (os error 111)", OsKind::Linux).contains("isn't running"));
    }
}
