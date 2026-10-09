# Releasing Skerry

A release builds Skerry for Windows, macOS and Linux, publishes one GitHub release per platform, and tells every installed copy that a new version is ready. This page covers the one-time setup and what the Release workflow checks.

- [One-time setup](#one-time-setup)
  - [1. The update signing key (required)](#1-the-update-signing-key-required)
  - [2. The macOS signing certificate (recommended)](#2-the-macos-signing-certificate-recommended)
- [Making a release](#making-a-release)
- [How in-app updates work](#how-in-app-updates-work)
- [Troubleshooting](#troubleshooting)

## One-time setup

All of these are **repository secrets**: GitHub → the repository → **Settings → Secrets and variables → Actions → New repository secret** ([direct link](https://github.com/AviSplash/Skerry/settings/secrets/actions)).

| Secret | What it is | Needed for |
|---|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | Private key that signs every update | **Required.** Without it the Release workflow stops |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Its password | Required |
| `TAURI_UPDATER_PUBLIC_KEY` | The matching public key, built into Skerry | Required |
| `MACOS_CERTIFICATE`, `MACOS_CERTIFICATE_PASSWORD` | Mac code-signing certificate | Recommended: keeps Mac permissions across updates |
| `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | Apple notarization | Optional, only with an Apple Developer ID certificate |

### 1. The update signing key (required)

Installed copies of Skerry only install an update if it's signed with the private key whose public half is built into them. You need [Node.js](https://nodejs.org) (the LTS version) on the computer where you make the key.

**On a Mac (or Linux):**

1. Install Node.js if you don't have it: download the LTS installer from [nodejs.org](https://nodejs.org), or `brew install node`.
2. Open **Terminal** and run:

   ```sh
   curl -fsSLO https://raw.githubusercontent.com/AviSplash/Skerry/main/.github/scripts/make-updater-key.sh
   bash make-updater-key.sh
   ```

   It creates the folder `skerry-updater-key` and prints `Created Skerry's update signing key …`.
3. Add the three secrets. For each one, copy the file to the clipboard, click **New repository secret**, type the name, paste with ⌘V and click **Add secret**:

   | Copy with | Secret name |
   |---|---|
   | `pbcopy < skerry-updater-key/TAURI_SIGNING_PRIVATE_KEY.txt` | `TAURI_SIGNING_PRIVATE_KEY` |
   | `pbcopy < skerry-updater-key/TAURI_SIGNING_PRIVATE_KEY_PASSWORD.txt` | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` |
   | `pbcopy < skerry-updater-key/TAURI_UPDATER_PUBLIC_KEY.txt` | `TAURI_UPDATER_PUBLIC_KEY` |

4. Back up the `skerry-updater-key` folder somewhere private (a password manager or an encrypted drive). Never commit it.

**On Windows (PowerShell):** with Node.js installed:

```powershell
mkdir skerry-updater-key; cd skerry-updater-key
$pw = -join ((48..57) + (97..102) | Get-Random -Count 32 | ForEach-Object { [char]$_ })
npx --yes @tauri-apps/cli@2 signer generate --ci -p $pw -w skerry-updater.key
Get-Content skerry-updater.key -Raw | Set-Clipboard        # → TAURI_SIGNING_PRIVATE_KEY
$pw | Set-Clipboard                                         # → TAURI_SIGNING_PRIVATE_KEY_PASSWORD
Get-Content skerry-updater.key.pub -Raw | Set-Clipboard    # → TAURI_UPDATER_PUBLIC_KEY
```

Run the three `Set-Clipboard` lines one at a time, adding each secret in between, and save the password somewhere safe (it isn't written to a file).

**Keep the key.** If it's lost, make a new one the same way. Installed copies trust the old key, though, so they can't install updates signed with the new one: everyone installs the next version once by hand, and updates work again from then on.

### 2. The macOS signing certificate (recommended)

Without it, macOS treats every Mac update as a new app and asks for Accessibility again. Make the free self-signed certificate once with `.github/scripts/make-macos-certificate.sh` and add `MACOS_CERTIFICATE` and `MACOS_CERTIFICATE_PASSWORD`. Step by step: [Skerry on macOS → For maintainers](macos.md#creating-the-free-signing-certificate-step-by-step).

## Making a release

1. Open **Actions → Release → Run workflow**.
2. Leave the branch on `main`, type the version (for example `v1.2` or `v1.1.4`) and click **Run workflow**.
3. Wait about ten minutes. When it's green, the releases **vX - Windows**, **vX - macOS** and **vX - Linux** are published, and installed copies see the update within six hours (or right away with **Check for updates**).

The version in the source (`Cargo.toml`, `tauri.conf.json`) is replaced by the one you type; keep them roughly in step anyway.

## How in-app updates work

1. Every installed copy reads `latest.json` on the `updater` branch ([link](https://raw.githubusercontent.com/AviSplash/Skerry/updater/latest.json)) when it starts, every six hours, and whenever you click **Check for updates**.
2. If that file names a newer version, Skerry shows **Skerry x.y is available → Update and restart**. If it doesn't, or no update has been published for that system, **Check for updates** says you have the latest version.
3. **Update and restart** downloads the file for that system, checks its signature against the public key built into Skerry, installs it in place and restarts. Pairings and settings are kept. On Windows the installer asks for administrator approval (for the firewall rule). On Linux, `.deb` and `.rpm` installs ask for your password.

What the Release workflow checks, so a release can always be installed by the updater:

| When | Check | If it fails |
|---|---|---|
| Before building, on every platform | `TAURI_SIGNING_PRIVATE_KEY` exists, opens with its password, and signs a test file that verifies with the public key Skerry is being built with (`TAURI_UPDATER_PUBLIC_KEY`) | Stops before compiling, saying which secret is wrong |
| After building the Mac app | The whole app is signed as `org.skerry.app` | Stops before publishing |
| After every platform is published | Every file in the new `latest.json` is downloaded and its signature verified, exactly as the app does; the version matches | `latest.json` isn't replaced, so no installed copy is offered a broken update |

## Troubleshooting

- **"No TAURI_SIGNING_PRIVATE_KEY secret"**: add the update signing key (above).
- **"Couldn't sign with TAURI_SIGNING_PRIVATE_KEY … Check TAURI_SIGNING_PRIVATE_KEY_PASSWORD"**: the password secret doesn't match the key. Paste it again from `TAURI_SIGNING_PRIVATE_KEY_PASSWORD.txt`.
- **"TAURI_SIGNING_PRIVATE_KEY doesn't belong to the public key …"**: `TAURI_UPDATER_PUBLIC_KEY` is missing or comes from a different key pair. Paste it again from `TAURI_UPDATER_PUBLIC_KEY.txt` in the same folder as the private key.
- **Skerry says "This update isn't signed with the key this copy of Skerry trusts"**: that copy was built with a different public key (for example, before the key was replaced). Download the latest version from the releases page and install it over the old one once.
- **Check for updates says "Couldn't reach the update server"**: that computer can't reach raw.githubusercontent.com (no internet, a proxy or a firewall).
