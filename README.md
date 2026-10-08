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
- **Finds your computers automatically** on the local network (mDNS), with a **Scan network** button that sweeps your subnet when automatic discovery is blocked. You can also add computers by address.
- **Updates itself.** Skerry checks for new versions and installs them in place; your pairings and settings stay.
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
| Windows | [v1.1 - Windows](https://github.com/AviSplash/Skerry/releases/tag/v1.1-windows) | `Skerry_1.1.0_x64-setup.exe` or `.msi` |
| macOS | [v1.1 - macOS](https://github.com/AviSplash/Skerry/releases/tag/v1.1-macos) | `Skerry_1.1.0_universal.dmg` (Apple Silicon and Intel) |
| Linux | [v1.1 - Linux](https://github.com/AviSplash/Skerry/releases/tag/v1.1-linux) | `.deb` (Debian, Ubuntu), `.rpm` (Fedora), or `.AppImage` (any distro) |

Install Skerry on **every** computer you want to share between. Use the same version everywhere.

### Updates

Skerry 1.1 and later update themselves. When a new version is out, the window shows **Skerry x.y is available → Update and restart**; you can also use **Check for updates** in the tray menu or under **Settings → Advanced → Help**. Updates are downloaded from this repository's releases and verified against a signing key built into Skerry before they're installed. Pairings and settings are kept. To stop automatic checks, turn off **Settings → Check for updates**.

Coming from 1.0, install 1.1 once by hand over the old version (no need to uninstall first).

### First run, by OS

The current builds are not code-signed (signing certificates cost money; see [Roadmap](#roadmap)), so each OS asks once before running them.

- **Windows:** if SmartScreen says "Windows protected your PC", click **More info → Run anyway**. At the end of the installation, approve the administrator prompt: it adds the Windows Firewall rule that lets your other computers connect to this one. To control apps running as administrator, Skerry must also run as administrator. Windows never lets any app control UAC prompts or the lock screen.
- **macOS:** open the `.dmg`, drag Skerry to Applications, eject the disk image, and open Skerry from Applications (macOS doesn't keep the permissions of a copy running from the disk image). The first time, macOS asks before opening it: on macOS 15 or later, click **Done**, then **System Settings → Privacy & Security → Open Anyway**; on macOS 12 to 14, **right-click Skerry → Open**. When asked, switch Skerry on under **System Settings → Privacy & Security → Accessibility** (and **Input Monitoring** if macOS lists it there too). On macOS 15 or later, when macOS asks whether Skerry may find devices on your local network, click **Allow**.
- **Linux:** on GNOME or KDE (Wayland), approve the "Remote desktop" and "Input capture" requests the first time. Skerry remembers your answer. On X11 nothing extra is needed.

### Uninstalling

Open **Settings → Advanced → Uninstall Skerry…**. Skerry lists exactly what it will remove and asks you to confirm, then removes itself and everything it stored on that computer and quits:

- **All systems:** your settings, pairings and the computer's Skerry key, logs, saved window data and caches, and the "Start at login" entry.
- **Windows:** the program, its shortcuts and its Apps & features entry (through its own uninstaller), its registry entries, and its Windows Firewall rules (Windows asks for administrator approval).
- **macOS:** the app, its preferences and saved state, and its Accessibility and Input Monitoring entries. macOS doesn't let apps remove their **Local Network** entry; it stays in that list but does nothing once Skerry is gone.
- **Linux:** the `.deb` or `.rpm` package (Linux asks for your password) or the AppImage file.

If removing the program itself fails or you cancel the password prompt, nothing else is removed. Your other computers keep this one in their list until you click **Forget** on them. You can also uninstall the usual way (Apps & features, dragging the app to the Trash, `apt remove skerry`); that leaves your settings behind.

## Getting started

1. Start Skerry on two computers on the same network. Each one shows the other under **Nearby**. If it doesn't appear within a few seconds, click **Scan network**.
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
Skerry 1.1.0 on studio (X11)
device id 1c843bbbadba346f  fingerprint 1c84-3bbb-adba-346f-1f47  port 24870
Type `help` for commands.
> devices
> scan
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

Start with **Settings → Advanced → Help → Diagnostics**. It shows what Skerry sees on this computer (permissions, network addresses, each computer's connection state and the last connection error) plus the recent log. **Copy** it into a bug report. Log files are kept for a week; **Open logs folder** shows them.

- **Pairing or connecting works from one computer but not from the other** (for example Windows → Mac works, Mac → Windows doesn't). The computer that can't be reached is blocking incoming connections:
  - **Windows:** Windows Firewall blocks Skerry if its prompt was dismissed. Skerry shows a **Windows Firewall is blocking Skerry** banner: click **Allow Skerry** and approve the prompt. Or allow it yourself under **Windows Security → Firewall & network protection → Allow an app through firewall**, for both private and public networks.
  - **macOS 15 or later:** turn on Skerry under **System Settings → Privacy & Security → Local Network** (if it's already on, switch it off and on again). Without it, macOS stops Skerry from connecting to other computers, and Skerry's log shows "No route to host (os error 65)". Other computers can still connect to the Mac, which is why one direction works: until it's fixed, pair and connect from the other computer (click **Scan network** there, then **Pair** next to the Mac).
    - Skerry 1.1.2 and earlier weren't signed as a whole app, so macOS couldn't reliably tell which app the Local Network switch belonged to. Install the latest version, then switch Skerry off and on under Local Network.
    - To check, run `nc -vz <other computer's IP> 24870` in Terminal. Terminal is exempt from this macOS check: if that connects but Skerry can't, macOS is blocking Skerry. If it doesn't connect either, the other computer is off, has a different address now, or its firewall blocks Skerry.
    - Keep only one copy of Skerry on the Mac (in Applications). Old copies in Downloads or a mounted disk image confuse this macOS setting.
    - Last resort (macOS 15.5 or later): let every app reach your home network directly with `sudo defaults write com.apple.network.local-network AllowedWiFiLocalNetworkAddresses -array "192.168.1.0/24"` (use your network; for wired, also set `AllowedEthernetLocalNetworkAddresses`), then restart the Mac.
  - Third-party firewalls and antivirus suites need the same: allow Skerry, or TCP port 24870 and mDNS (UDP 5353).
- **The other computer doesn't show up under Nearby.** Both computers must be on the same network. Click **Scan network** to sweep your subnet (this finds computers even when the network blocks automatic discovery). Otherwise use **Pair by address** with the other computer's IP address; to keep reconnecting by address, add it under **Settings → Advanced → Extra addresses**.
- **The mouse won't cross to the other computer.** Check that:
  1. the other computer shows **Online** under **Computers** on both sides (if it says Offline, the line under its name says why);
  2. it's placed on the side where it really is, in the **Arrangement** on either computer;
  3. you're pushing against the *outer* edge: with several monitors, that's the edge of the outermost monitor on that side;
  4. no mouse button is held (switching waits while you drag, unless you turn that off);
  5. on a Mac, Skerry is allowed under **Accessibility**.
- **On a Mac, Skerry says it needs permission although it's already switched on.** macOS ties the approval to the exact copy of Skerry you approved.
  - **Skerry 1.1.2 and earlier** weren't signed as a whole app (only the program inside was), so macOS never applied the approval, however often it was switched on. Install the latest version, quit Skerry, remove every Skerry entry under **Accessibility** and **Input Monitoring** with the **−** button, then open Skerry and allow it again.
  - **After an update**, a Mac build without Skerry's own certificate is a new app to macOS. Skerry notices and clears the old entry itself, so macOS asks once and you allow Skerry again. If the switch still shows on but Skerry says it needs permission, click **Reset permissions** in Skerry's banner (or in Terminal: `tccutil reset Accessibility org.skerry.app`, then reopen Skerry). Releases signed with Skerry's own certificate (see [Releasing](#releasing)) keep the approval across updates.
  - Run Skerry from **Applications**, not from the disk image or the Downloads folder: macOS can't keep the permissions of those copies.
  - **Settings → Advanced → Help → Diagnostics** shows how this copy is signed and whether macOS currently allows it.
- **Hotkeys do nothing.** Hotkeys only switch to computers that are online and placed in the arrangement. On a Mac, check Accessibility (and Input Monitoring, if Skerry is listed there). On Windows, keys pressed while an app running as administrator has focus only reach Skerry if Skerry also runs as administrator.
- **"Could not listen on 0.0.0.0:24870".** Another program is using the port. Change `port` in the settings file.
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

- **Phase 1 (done in 1.0 and 1.1):** mouse and keyboard sharing, hotkeys, Cmd/Ctrl translation, encrypted pairing, discovery, text and image clipboard, arrangement editor, tray app, CI builds for all three OSes.
- **Phase 2:** copy and paste files between computers, then drag-and-drop; Game Mode (relative mouse input for games).
- **Phase 3:** dim inactive displays; lock and sleep computers together.
- Also planned: code-signed builds, controlling others from wlroots compositors, and per-display (rather than per-computer) arrangement.

## Releasing

Run **Actions → Release → Run workflow** with a version like `v1.2`. It builds every platform, publishes the three releases, and, when the repository has the secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, signs the updates and publishes `latest.json` on the `updater` branch, which installed copies check. The public half of that key is `plugins.updater.pubkey` in `apps/skerry-app/tauri.conf.json`. Keep the private key safe: a lost key means existing installs can't verify later updates.

macOS remembers Accessibility, Input Monitoring and Local Network approval per signing identity. Without a certificate, Mac builds are ad-hoc signed: the whole app is signed as `org.skerry.app`, but each build is a new identity, so users allow Skerry again after every update. The Release workflow fails if the app isn't signed as a whole (`.github/scripts/macos-verify-signature.sh`). To keep approvals across updates, give the workflow a certificate, and always use the same one:

- **Free, self-signed:** run `.github/scripts/make-macos-certificate.sh` once (macOS, Linux or Git Bash; it needs `openssl`) and add the two values it prints as the repository secrets `MACOS_CERTIFICATE` and `MACOS_CERTIFICATE_PASSWORD`. Accessibility and Input Monitoring then survive updates. Gatekeeper still asks once on first launch, and Apple only promises reliable Local Network tracking for Apple-issued certificates.
- **Apple Developer ID** (Apple Developer Program): export your *Developer ID Application* certificate as a `.p12` and use it for the same two secrets. Add `APPLE_ID`, `APPLE_PASSWORD` (an app-specific password) and `APPLE_TEAM_ID` to have Apple notarize each build, so Skerry opens without any Gatekeeper warning.

## Contributing

Bug reports and pull requests are welcome. Please run `cargo fmt`, `cargo clippy --workspace --all-targets` and the tests before opening a PR. If you're adding support for a new desktop environment, the traits to implement are in [`crates/skerry-core/src/input.rs`](crates/skerry-core/src/input.rs).

## License

Skerry is free software, licensed under the [GNU General Public License v3.0 or later](LICENSE). You can use, study, share and change it; if you distribute a changed version, you must share its source under the same license.

Skerry is an independent project. It is not affiliated with any commercial KVM software.
