//! Persistent settings, stored as TOML in the platform's config directory.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::geometry::Edge;
use crate::keys::{default_hotkeys, Hotkey, HotkeyAction, OsKind};

pub const DEFAULT_PORT: u16 = 24870;

#[derive(Debug, Clone)]
pub struct Paths {
    pub dir: PathBuf,
    pub config: PathBuf,
    pub identity: PathBuf,
}

impl Paths {
    pub fn in_dir(dir: impl Into<PathBuf>) -> Paths {
        let dir = dir.into();
        Paths { config: dir.join("config.toml"), identity: dir.join("identity.key"), dir }
    }

    /// `$SKERRY_CONFIG_DIR`, or the platform default
    /// (`~/.config/skerry`, `%APPDATA%\Skerry\config`, `~/Library/Application Support/org.skerry.Skerry`).
    pub fn default_location() -> Result<Paths> {
        if let Some(dir) = std::env::var_os("SKERRY_CONFIG_DIR") {
            return Ok(Paths::in_dir(PathBuf::from(dir)));
        }
        let dirs = directories::ProjectDirs::from("org", "Skerry", "Skerry")
            .context("could not determine a configuration directory")?;
        Ok(Paths::in_dir(dirs.config_dir()))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub left: Option<String>,
    pub right: Option<String>,
    pub top: Option<String>,
    pub bottom: Option<String>,
}

impl Layout {
    pub fn get(&self, edge: Edge) -> Option<&str> {
        match edge {
            Edge::Left => self.left.as_deref(),
            Edge::Right => self.right.as_deref(),
            Edge::Top => self.top.as_deref(),
            Edge::Bottom => self.bottom.as_deref(),
        }
    }

    fn slot(&mut self, edge: Edge) -> &mut Option<String> {
        match edge {
            Edge::Left => &mut self.left,
            Edge::Right => &mut self.right,
            Edge::Top => &mut self.top,
            Edge::Bottom => &mut self.bottom,
        }
    }

    /// Place `id` on `edge`, removing it from any other edge. Returns the id
    /// that previously occupied the edge, if it was a different device.
    pub fn place(&mut self, edge: Edge, id: &str) -> Option<String> {
        self.remove(id);
        let prev = self.slot(edge).replace(id.to_string());
        prev.filter(|p| p != id)
    }

    /// Clear an edge, returning who was there.
    pub fn clear(&mut self, edge: Edge) -> Option<String> {
        self.slot(edge).take()
    }

    pub fn remove(&mut self, id: &str) -> Option<Edge> {
        let mut found = None;
        for e in Edge::ALL {
            let slot = self.slot(e);
            if slot.as_deref() == Some(id) {
                *slot = None;
                found = Some(e);
            }
        }
        found
    }

    pub fn edge_of(&self, id: &str) -> Option<Edge> {
        Edge::ALL.into_iter().find(|e| self.get(*e) == Some(id))
    }

    pub fn as_array(&self) -> [Option<String>; 4] {
        [self.left.clone(), self.right.clone(), self.top.clone(), self.bottom.clone()]
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeerConfig {
    pub id: String,
    pub name: String,
    /// Hex-encoded X25519 public key learned during pairing.
    pub public_key: String,
    pub os: OsKind,
    /// Last address the peer was reached at (host:port).
    #[serde(default)]
    pub last_addr: Option<String>,
    /// Pointer speed multiplier while controlling this peer.
    #[serde(default = "one")]
    pub speed: f64,
}

fn one() -> f64 {
    1.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HotkeyBinding {
    pub keys: String,
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Name shown to other computers.
    pub name: String,
    pub port: u16,
    /// Master switch: when false this computer neither controls nor is controlled.
    pub enabled: bool,
    pub clipboard_sync: bool,
    /// Swap Command and Control when moving between a Mac and a PC.
    pub swap_cmd_ctrl: bool,
    /// Switch computers by pushing the cursor against a screen edge.
    pub edge_switching: bool,
    /// Do not switch while a mouse button is held (protects window drags).
    pub block_switch_while_dragging: bool,
    /// Let the desktop app look for new versions once in a while.
    pub check_updates: bool,
    pub hotkeys: Vec<HotkeyBinding>,
    pub layout: Layout,
    pub peers: Vec<PeerConfig>,
    /// Extra addresses to connect to when multicast discovery is blocked.
    pub manual_peers: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            name: default_name(),
            port: DEFAULT_PORT,
            enabled: true,
            clipboard_sync: true,
            swap_cmd_ctrl: true,
            edge_switching: true,
            block_switch_while_dragging: true,
            check_updates: true,
            hotkeys: default_hotkeys().into_iter().map(|(keys, action)| HotkeyBinding { keys, action }).collect(),
            layout: Layout::default(),
            peers: Vec::new(),
            manual_peers: Vec::new(),
        }
    }
}

pub fn default_name() -> String {
    let raw = gethostname::gethostname().to_string_lossy().to_string();
    let name = raw.split('.').next().unwrap_or("").trim().to_string();
    if name.is_empty() {
        "My Computer".to_string()
    } else {
        name
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        if !path.exists() {
            return Ok(Config::default());
        }
        let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let cfg: Config = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self)?;
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn peer(&self, id: &str) -> Option<&PeerConfig> {
        self.peers.iter().find(|p| p.id == id)
    }

    pub fn peer_mut(&mut self, id: &str) -> Option<&mut PeerConfig> {
        self.peers.iter_mut().find(|p| p.id == id)
    }

    /// Parsed hotkeys; invalid entries are skipped with a warning.
    pub fn parsed_hotkeys(&self) -> Vec<(Hotkey, HotkeyAction)> {
        self.hotkeys
            .iter()
            .filter_map(|b| match (Hotkey::parse(&b.keys), HotkeyAction::parse(&b.action)) {
                (Some(k), Some(a)) => Some((k, a)),
                _ => {
                    tracing::warn!("ignoring invalid hotkey {:?} -> {:?}", b.keys, b.action);
                    None
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_place_moves_device() {
        let mut l = Layout::default();
        assert_eq!(l.place(Edge::Right, "a"), None);
        assert_eq!(l.place(Edge::Left, "a"), None);
        assert_eq!(l.right, None);
        assert_eq!(l.left.as_deref(), Some("a"));
        assert_eq!(l.place(Edge::Left, "b"), Some("a".to_string()));
        assert_eq!(l.edge_of("b"), Some(Edge::Left));
        assert_eq!(l.edge_of("a"), None);
    }

    #[test]
    fn save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.toml");
        let mut c = Config { name: "Test".into(), ..Default::default() };
        c.layout.place(Edge::Top, "abcd");
        c.peers.push(PeerConfig {
            id: "abcd".into(),
            name: "Laptop".into(),
            public_key: "00".repeat(32),
            os: OsKind::Macos,
            last_addr: Some("10.0.0.2:24870".into()),
            speed: 1.5,
        });
        c.save(&p).unwrap();
        let back = Config::load(&p).unwrap();
        assert_eq!(back, c);
        assert_eq!(back.parsed_hotkeys().len(), 5);
    }

    #[test]
    fn partial_config_uses_defaults() {
        let c: Config = toml::from_str("name = \"X\"\n").unwrap();
        assert_eq!(c.port, DEFAULT_PORT);
        assert!(c.clipboard_sync);
    }
}
