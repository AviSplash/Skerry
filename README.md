<p align="center">
  <img src="assets/logo.svg" width="112" height="112" alt="Skerry logo">
</p>

<h1 align="center">Skerry</h1>

<p align="center">
  <strong>One mouse and keyboard for all your computers.</strong><br>
  Free, open source, encrypted, and it works between Windows, macOS and Linux.
</p>

<p align="center">
  <a href="https://github.com/AviSplash/Skerry/actions/workflows/ci.yml"><img src="https://github.com/AviSplash/Skerry/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0-teal" alt="License: GPL-3.0"></a>
</p>

---

Skerry is a software KVM. Put your computers side by side, tell Skerry where each one sits, and move the mouse off the edge of one screen: it appears on the next computer, and your keyboard follows it. Copy on one machine, paste on another.

A *skerry* is a small rocky island. Your cursor hops across them.

![The Skerry window](docs/screenshot.png)

## Features

- **Mouse and keyboard sharing.** Move off a screen edge to switch computers, or use hotkeys. Chains of three or more computers work too (A → B → C).
- **Any OS to any OS.** Windows, macOS and Linux can control each other in any combination.
- **Command ↔ Control translation.** Cmd+C on a Mac keyboard copies on Windows and Linux, and Ctrl+C on a PC keyboard copies on a Mac. Turn it off if you prefer.
- **Clipboard sync.** Text and images follow the cursor to the next computer.
- **Encrypted end to end.** Every connection uses the Noise protocol (`Noise_XX_25519_ChaChaPoly_BLAKE2s`). Pairing uses a 6-digit code with SPAKE2, so the code can't be brute-forced offline.
- **Finds your computers automatically** on the local network (mDNS). You can also add them by address.
- **No account, no cloud, no subscription.** Everything stays on your network.
- **Tray app with an arrangement editor**, plus a command-line version for servers and tiling setups.

## Platform support

| | Can control other computers | Can be controlled |
|---|---|---|
| **Windows 10/11** | ✅ | ✅ |
| **macOS 12+** (Apple Silicon and Intel) | ✅ (needs Accessibility permission) | ✅ (needs Accessibility permission) |
| **Linux, X11** | ✅ | ✅ |
| **Linux, GNOME 45+ / KDE Plasma 6.1+ (Wayland)** | ✅ via the InputCapture portal | ✅ via the RemoteDesktop portal |
| **Linux, Sway / Hyprland / other wlroots (Wayland)** | ❌ not yet | ✅ |

## Install

Download the installer for each computer from its release:

| OS | Release | File |
|---|---|---|
| Windows | [v1.0 - Windows](https://github.com/AviSplash/Skerry/releases/tag/v1.0-windows) | `Skerry_1.0.0_x64-setup.exe` or `.msi` |
| macOS | [v1.0 - macOS](https://github.com/AviSplash/Skerry/releases/tag/v1.0-macos) | `Skerry_1.0.0_universal.dmg` (Apple Silicon and Intel) |
| Linux | [v1.0 - Linux](https://github.com/AviSplash/Skerry/releases/tag/v1.0-linux) | `.deb` (Debian, Ubuntu), `.rpm` (Fedora), or `.AppImage` (any distro) |

Install Skerry on **every** computer you want to share between.

### First run, by OS

The current builds are not code-signed (signing certificates cost money; see [Roadmap](#roadmap)), so each OS asks once before running them.

- **Windows:** if SmartScreen says "Windows protected your PC", click **More info → Run anyway**. When Windows Firewall asks, allow Skerry on **private networks**. To control apps running as administrator, Skerry must also run as administrator. Windows never lets any app control UAC prompts or the lock screen.
- **macOS:** open the `.dmg`, drag Skerry to Applications, then **right-click Skerry → Open** the first time. When asked, allow Skerry under **System Settings → Privacy & Security → Accessibility**, then restart Skerry.
- **Linux:** on GNOME or KDE (Wayland), approve the "Remote desktop" and "Input capture" requests the first time. Skerry remembers your answer. On X11 nothing extra is needed.

## Getting started

1. Start Skerry on two computers on the same network. Each one shows the other under **Nearby**.
2. Click **Pair** on one of them. The other shows a 6-digit code: type it on the first one. You only do this once.
3. Drag the paired computer to the side of the screen where it sits on your desk. The other computer updates its own layout to match.
4. Move the mouse off that edge. You're now using the other computer.

Closing the window keeps Skerry running in the tray. Use **Pause sharing** in the tray menu to temporarily keep the mouse on one computer.

### Hotkeys

| Keys | Action |
|---|---|
| Ctrl + Alt + Shift + ← / → / ↑ / ↓ | Jump to the computer on that side |
| Ctrl + Alt + Shift + Esc | Bring the mouse back to this computer |

On a Mac, Alt is the Option key. You can change hotkeys in the settings file (shown under **Settings → Advanced**).

### Command-line version

`skerry-cli` runs the same engine without a window:

```text
$ skerry-cli run
Skerry 1.0.0 on studio (X11)
device id 1c843bbbadba346f  fingerprint 1c84-3bbb-adba-346f-1f47  port 24870
Type `help` for commands.
> devices
> pair 192.168.1.40
> code 2 684858
> layout right 93b98ac43575a291
```

## How it works

- Every Skerry install has its own key pair. A computer's **device id** and **fingerprint** come from its public key.
- Computers find each other with mDNS (`_skerry._tcp`, port 24870 by default) and connect over TCP. Each connection starts with a Noise `XX` handshake, so both sides prove who they are and everything after that is encrypted and authenticated.
- **Pairing:** the first connection between two computers isn't trusted yet. One shows a random 6-digit code, the user types it on the other, and both run SPAKE2 with it. They then confirm the result against this connection's Noise handshake hash, which rules out a man in the middle. After that, each computer remembers the other's public key and never asks again.
- **Switching:** the computer whose mouse you're touching *captures* its input once the cursor crosses an edge. It keeps a virtual cursor on the other computer's screen and sends absolute positions, clicks and keys, which the other computer *replays* as if they came from a local device. Keys travel as physical key positions, so each computer applies its own keyboard layout.
- **Clipboard:** when the cursor leaves a computer, that computer sends its clipboard to the next one, but only if it changed. Large images are sent in chunks behind your typing, so input never lags.

The wire format is described in [docs/protocol.md](docs/protocol.md).

## Troubleshooting

- **The other computer doesn't show up under Nearby.** Both computers must be on the same network. Some office and guest Wi-Fi networks block discovery: use **Pair by address** with the other computer's IP address. To keep reconnecting by address, add it under **Settings → Advanced → Extra addresses**.
- **"Could not listen on 0.0.0.0:24870".** Another program is using the port. Change `port` in the settings file.
- **Firewall:** allow TCP port 24870 and mDNS (UDP 5353) on your local network.
- **Keys stick or the mouse gets stuck on another computer.** Press Ctrl + Alt + Shift + Esc to bring it home. Skerry also releases every held key and button whenever control leaves a computer or a connection drops.

## Build from source

You need [Rust](https://rustup.rs) (stable). On Linux, also install the WebKitGTK build dependencies:

```sh
sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev
```

Then:

```sh
cargo test -p skerry-core -p skerry-platform    # unit and end-to-end tests
cargo run -p skerry-cli -- run                  # command-line version
cargo run -p skerry-app                         # desktop app
npx @tauri-apps/cli@2 build                     # installers (run in apps/skerry-app)
```

The X11 and wlroots backends have integration tests that run against a real (virtual) display server:

```sh
Xvfb :99 &  DISPLAY=:99 cargo test -p skerry-platform --test x11 -- --ignored
WLR_BACKENDS=headless sway -c /dev/null &  WAYLAND_DISPLAY=wayland-1 cargo test -p skerry-platform --test wlr -- --ignored
```

### Project layout

```text
crates/skerry-core       engine, protocol, encryption, pairing, discovery, clipboard sync (OS-independent)
crates/skerry-platform   input capture and replay per OS: Windows, macOS, X11, Wayland
apps/skerry-cli          command-line app
apps/skerry-app          desktop app (Tauri 2; the UI is plain HTML/CSS/JS in ui/)
```

## Roadmap

- **Phase 1 (this release):** mouse and keyboard sharing, hotkeys, Cmd/Ctrl translation, encrypted pairing, discovery, text and image clipboard, arrangement editor, tray app, CI builds for all three OSes.
- **Phase 2:** copy and paste files between computers, then drag-and-drop; Game Mode (relative mouse input for games).
- **Phase 3:** dim inactive displays; lock and sleep computers together.
- Also planned: code-signed builds, controlling others from wlroots compositors, and per-display (rather than per-computer) arrangement.

## Contributing

Bug reports and pull requests are welcome. Please run `cargo fmt`, `cargo clippy --workspace --all-targets` and the tests before opening a PR. If you're adding support for a new desktop environment, the traits to implement are in [`crates/skerry-core/src/input.rs`](crates/skerry-core/src/input.rs).

## License

Skerry is free software, licensed under the [GNU General Public License v3.0 or later](LICENSE). You can use, study, share and change it; if you distribute a changed version, you must share its source under the same license.

Skerry is an independent project. It is not affiliated with any commercial KVM software.
