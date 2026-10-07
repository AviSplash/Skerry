//! Finding other Skerry computers on the local network with mDNS / DNS-SD.
//!
//! Each install advertises `_skerry._tcp.local.` with its device id, name and
//! OS in the TXT record. Discovery only finds candidates: trust comes from
//! pairing, never from what a TXT record claims.

use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use tokio::sync::mpsc::UnboundedSender;

use crate::keys::OsKind;

pub const SERVICE_TYPE: &str = "_skerry._tcp.local.";

#[derive(Debug, Clone, PartialEq)]
pub struct Discovered {
    pub id: String,
    pub name: String,
    pub os: OsKind,
    pub version: String,
    pub addrs: Vec<SocketAddr>,
}

#[derive(Debug, Clone)]
pub enum DiscoveryEvent {
    Found(Discovered),
    Lost(String),
}

pub fn parse_os(s: &str) -> OsKind {
    match s {
        "windows" => OsKind::Windows,
        "macos" => OsKind::Macos,
        "linux" => OsKind::Linux,
        _ => OsKind::Other,
    }
}

pub fn os_str(os: OsKind) -> &'static str {
    match os {
        OsKind::Windows => "windows",
        OsKind::Macos => "macos",
        OsKind::Linux => "linux",
        OsKind::Other => "other",
    }
}

pub struct Discovery {
    daemon: ServiceDaemon,
    id: String,
    os: OsKind,
    port: u16,
    fullname: Mutex<Option<String>>,
    tx: UnboundedSender<DiscoveryEvent>,
}

impl Discovery {
    pub fn start(
        id: &str,
        name: &str,
        os: OsKind,
        port: u16,
        tx: UnboundedSender<DiscoveryEvent>,
    ) -> Result<Discovery> {
        let daemon = ServiceDaemon::new()?;
        let me = Discovery { daemon, id: id.to_string(), os, port, fullname: Mutex::new(None), tx };
        me.advertise(name)?;
        me.browse()?;
        Ok(me)
    }

    /// Start browsing; results are sent to the engine from a background thread.
    fn browse(&self) -> Result<()> {
        let rx = self.daemon.browse(SERVICE_TYPE)?;
        let my_id = self.id.clone();
        let tx = self.tx.clone();
        std::thread::Builder::new().name("skerry-discovery".into()).spawn(move || {
            while let Ok(ev) = rx.recv() {
                let out = match ev {
                    ServiceEvent::ServiceResolved(svc) => {
                        let props = &svc.txt_properties;
                        let Some(pid) = props.get_property_val_str("id").map(str::to_string) else { continue };
                        if pid == my_id {
                            continue;
                        }
                        let mut addrs: Vec<SocketAddr> =
                            svc.addresses.iter().map(|a| SocketAddr::new(a.to_ip_addr(), svc.port)).collect();
                        // Prefer IPv4, then stable order.
                        addrs.sort_by_key(|a| (!a.is_ipv4(), *a));
                        DiscoveryEvent::Found(Discovered {
                            id: pid,
                            name: props.get_property_val_str("name").unwrap_or("Unknown").to_string(),
                            os: parse_os(props.get_property_val_str("os").unwrap_or("")),
                            version: props.get_property_val_str("v").unwrap_or("").to_string(),
                            addrs,
                        })
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        let instance = fullname.strip_suffix(&format!(".{SERVICE_TYPE}")).unwrap_or(&fullname);
                        DiscoveryEvent::Lost(instance.to_string())
                    }
                    _ => continue,
                };
                if tx.send(out).is_err() {
                    break;
                }
            }
        })?;
        Ok(())
    }

    /// Ask the network again: announce ourselves and restart browsing, which
    /// sends fresh queries so every running Skerry answers.
    pub fn rescan(&self, name: &str) -> Result<()> {
        let _ = self.daemon.stop_browse(SERVICE_TYPE);
        self.advertise(name)?;
        self.browse()
    }

    /// (Re-)publish this computer under `name`.
    pub fn advertise(&self, name: &str) -> Result<()> {
        if let Some(old) = self.fullname.lock().unwrap().take() {
            let _ = self.daemon.unregister(&old);
        }
        let mut props = HashMap::new();
        props.insert("id".to_string(), self.id.clone());
        props.insert("name".to_string(), name.to_string());
        props.insert("os".to_string(), os_str(self.os).to_string());
        props.insert("v".to_string(), crate::APP_VERSION.to_string());
        let host = format!("skerry-{}.local.", self.id);
        let info = ServiceInfo::new(SERVICE_TYPE, &self.id, &host, "", self.port, props)?.enable_addr_auto();
        *self.fullname.lock().unwrap() = Some(info.get_fullname().to_string());
        self.daemon.register(info)?;
        Ok(())
    }

    pub fn shutdown(&self) {
        if let Some(old) = self.fullname.lock().unwrap().take() {
            let _ = self.daemon.unregister(&old);
        }
        let _ = self.daemon.shutdown();
    }
}
