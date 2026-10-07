//! Skerry core: everything that is the same on every operating system.
//!
//! - [`transport`]: Noise-encrypted TCP connections
//! - [`pairing`]: SPAKE2 pairing with a 6-digit code
//! - [`discovery`]: mDNS discovery of other Skerry computers
//! - [`engine`]: the state machine that decides which computer the mouse
//!   and keyboard control, and moves input and clipboard between them
//!
//! Operating-system specific capture and emulation live in `skerry-platform`
//! and plug in through the traits in [`input`] and [`clipboard`].

pub mod clipboard;
pub mod config;
pub mod discovery;
pub mod engine;
pub mod geometry;
pub mod identity;
pub mod input;
pub mod keys;
pub mod net;
pub mod pairing;
pub mod proto;
pub mod transport;

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
