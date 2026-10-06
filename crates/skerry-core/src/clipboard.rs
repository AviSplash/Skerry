//! Clipboard contents, their wire encoding, and a worker thread that owns the
//! platform clipboard.
//!
//! Sync strategy: the clipboard moves with the cursor. When control passes
//! from one computer to another, the one losing focus sends its clipboard to
//! the one gaining it, but only if the content changed since that peer last
//! saw it (tracked by hash). Nothing is read or sent while the user stays on
//! one computer.

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::io::Cursor;
use std::sync::{mpsc as std_mpsc, Arc, Mutex};
use tokio::sync::oneshot;

use crate::proto::ClipKind;

/// Largest clipboard payload we will send or accept (encoded size).
pub const MAX_CLIPBOARD_BYTES: usize = 64 * 1024 * 1024;
/// Chunk size for clipboard transfer messages.
pub const CLIP_CHUNK: usize = 48 * 1024;

#[derive(Clone, PartialEq, Eq)]
pub struct ImageData {
    pub width: usize,
    pub height: usize,
    /// 8-bit RGBA, row-major, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

#[derive(Clone, PartialEq, Eq)]
pub enum ClipContent {
    Text(String),
    Image(ImageData),
}

impl std::fmt::Debug for ClipContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.describe())
    }
}

pub type ClipHash = [u8; 32];

impl ClipContent {
    pub fn hash(&self) -> ClipHash {
        let mut h = Sha256::new();
        match self {
            ClipContent::Text(t) => {
                h.update(b"text\0");
                h.update(t.as_bytes());
            }
            ClipContent::Image(img) => {
                h.update(b"image\0");
                h.update((img.width as u64).to_le_bytes());
                h.update((img.height as u64).to_le_bytes());
                h.update(&img.rgba);
            }
        }
        h.finalize().into()
    }

    pub fn describe(&self) -> String {
        match self {
            ClipContent::Text(t) => format!("text ({} chars)", t.chars().count()),
            ClipContent::Image(i) => format!("image ({}x{})", i.width, i.height),
        }
    }

    pub fn encode(&self) -> Result<(ClipKind, Vec<u8>)> {
        match self {
            ClipContent::Text(t) => Ok((ClipKind::Text, t.as_bytes().to_vec())),
            ClipContent::Image(img) => {
                if img.rgba.len() != img.width * img.height * 4 {
                    bail!("image buffer size does not match its dimensions");
                }
                let mut out = Vec::new();
                {
                    let mut enc = png::Encoder::new(&mut out, img.width as u32, img.height as u32);
                    enc.set_color(png::ColorType::Rgba);
                    enc.set_depth(png::BitDepth::Eight);
                    enc.set_compression(png::Compression::Fast);
                    let mut w = enc.write_header()?;
                    w.write_image_data(&img.rgba)?;
                }
                Ok((ClipKind::Png, out))
            }
        }
    }

    pub fn decode(kind: ClipKind, data: &[u8]) -> Result<ClipContent> {
        match kind {
            ClipKind::Text => {
                Ok(ClipContent::Text(String::from_utf8(data.to_vec()).context("clipboard text is not UTF-8")?))
            }
            ClipKind::Png => {
                let mut dec =
                    png::Decoder::new_with_limits(Cursor::new(data), png::Limits { bytes: 256 * 1024 * 1024 });
                dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
                let mut reader = dec.read_info()?;
                let size = reader.output_buffer_size().context("image too large")?;
                let mut buf = vec![0u8; size];
                let info = reader.next_frame(&mut buf)?;
                buf.truncate(info.buffer_size());
                let (w, h) = (info.width as usize, info.height as usize);
                let rgba = match info.color_type {
                    png::ColorType::Rgba => buf,
                    png::ColorType::Rgb => {
                        buf.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect()
                    }
                    png::ColorType::GrayscaleAlpha => {
                        buf.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect()
                    }
                    png::ColorType::Grayscale => buf.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
                    other => bail!("unsupported image format {other:?}"),
                };
                Ok(ClipContent::Image(ImageData { width: w, height: h, rgba }))
            }
        }
    }
}

/// Access to the system clipboard. Implementations run on a dedicated thread.
pub trait ClipboardProvider: Send + 'static {
    fn get(&mut self) -> Result<Option<ClipContent>>;
    fn set(&mut self, content: &ClipContent) -> Result<()>;
}

/// A clipboard that holds nothing (used when sync is unavailable).
pub struct NullClipboard;

impl ClipboardProvider for NullClipboard {
    fn get(&mut self) -> Result<Option<ClipContent>> {
        Ok(None)
    }
    fn set(&mut self, _: &ClipContent) -> Result<()> {
        Ok(())
    }
}

/// An in-memory clipboard, shared between clones. Used in tests.
#[derive(Clone, Default)]
pub struct MemoryClipboard(pub Arc<Mutex<Option<ClipContent>>>);

impl ClipboardProvider for MemoryClipboard {
    fn get(&mut self) -> Result<Option<ClipContent>> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn set(&mut self, content: &ClipContent) -> Result<()> {
        *self.0.lock().unwrap() = Some(content.clone());
        Ok(())
    }
}

enum Cmd {
    Read(oneshot::Sender<Option<(ClipContent, ClipHash)>>),
    Write(ClipContent, oneshot::Sender<Result<()>>),
}

/// Async front-end for a clipboard provider living on its own thread.
#[derive(Clone)]
pub struct ClipboardHandle {
    tx: std_mpsc::Sender<Cmd>,
}

impl ClipboardHandle {
    pub fn spawn(mut provider: Box<dyn ClipboardProvider>) -> ClipboardHandle {
        let (tx, rx) = std_mpsc::channel::<Cmd>();
        std::thread::Builder::new()
            .name("skerry-clipboard".into())
            .spawn(move || {
                while let Ok(cmd) = rx.recv() {
                    match cmd {
                        Cmd::Read(reply) => {
                            let got = match provider.get() {
                                Ok(Some(c)) => {
                                    let h = c.hash();
                                    Some((c, h))
                                }
                                Ok(None) => None,
                                Err(e) => {
                                    tracing::debug!("clipboard read failed: {e:#}");
                                    None
                                }
                            };
                            let _ = reply.send(got);
                        }
                        Cmd::Write(c, reply) => {
                            let _ = reply.send(provider.set(&c));
                        }
                    }
                }
            })
            .expect("spawning clipboard thread");
        ClipboardHandle { tx }
    }

    pub async fn read(&self) -> Option<(ClipContent, ClipHash)> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(Cmd::Read(tx)).ok()?;
        rx.await.ok().flatten()
    }

    pub async fn write(&self, content: ClipContent) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(Cmd::Write(content, tx)).map_err(|_| anyhow::anyhow!("clipboard thread stopped"))?;
        rx.await.map_err(|_| anyhow::anyhow!("clipboard thread stopped"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_png_roundtrip() {
        let img = ImageData { width: 3, height: 2, rgba: (0..24).collect() };
        let c = ClipContent::Image(img);
        let (kind, bytes) = c.encode().unwrap();
        assert_eq!(kind, ClipKind::Png);
        let back = ClipContent::decode(kind, &bytes).unwrap();
        assert_eq!(back, c);
        assert_eq!(back.hash(), c.hash());
    }

    #[test]
    fn text_roundtrip_and_hash_differs() {
        let a = ClipContent::Text("héllo".into());
        let (k, b) = a.encode().unwrap();
        assert_eq!(ClipContent::decode(k, &b).unwrap(), a);
        assert_ne!(a.hash(), ClipContent::Text("hello".into()).hash());
    }

    #[tokio::test]
    async fn worker_reads_and_writes() {
        let mem = MemoryClipboard::default();
        let h = ClipboardHandle::spawn(Box::new(mem.clone()));
        assert!(h.read().await.is_none());
        h.write(ClipContent::Text("x".into())).await.unwrap();
        let (c, hash) = h.read().await.unwrap();
        assert_eq!(c, ClipContent::Text("x".into()));
        assert_eq!(hash, c.hash());
    }
}
