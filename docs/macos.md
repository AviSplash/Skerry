# Skerry on macOS: permissions, Local Network and code signing

macOS asks before an app may see or send keyboard and mouse input, and (from macOS 15) before it may connect to other computers on your network. This page explains what Skerry needs, how to fix the common problems, and how releases are signed so that macOS remembers Skerry's permissions.

- [What Skerry needs](#what-skerry-needs)
- [Installing and first launch](#installing-and-first-launch)
- [Fixing permissions](#fixing-permissions)
- [Fixing Local Network](#fixing-local-network)
- [If the Mac's mouse or keyboard stops responding](#if-the-macs-mouse-or-keyboard-stops-responding)
- [Checking how your copy is signed](#checking-how-your-copy-is-signed)
- [For maintainers: signing releases](#for-maintainers-signing-releases)
- [Background: why this went wrong before 1.1.3](#background-why-this-went-wrong-before-113)

## What Skerry needs

| Permission | Where | macOS versions | Needed for | Without it |
|---|---|---|---|---|
| **Accessibility** | System Settings → Privacy & Security → Accessibility | 12 and later | Controlling other computers from this Mac, and being controlled by them | The banner "Permission needed to share its mouse and keyboard" stays, and the mouse never crosses in either direction |
| **Input Monitoring** | System Settings → Privacy & Security → Input Monitoring | 12 and later | Seeing keys on some macOS versions | Keys may not reach the other computer |
| **Local Network** | System Settings → Privacy & Security → Local Network | 15 and later | Connecting *from* this Mac to other computers (pairing, reconnecting, Scan network) | Skerry's log shows `No route to host (os error 65)`; other computers can still connect *to* the Mac |

macOS ties each of these to the exact app it was given to, identified by the app's **code signature**. That is why signing matters (see below).

## Installing and first launch

1. Download `Skerry_<version>_universal.dmg` from the [latest macOS release](https://github.com/AviSplash/Skerry/releases) (one file for Apple Silicon and Intel).
2. Open it and drag **Skerry** to **Applications**. Eject the disk image, then open Skerry **from Applications**. macOS doesn't keep the permissions of a copy that runs from the disk image or the Downloads folder, and Skerry warns you if it is running from one.
3. The first time, macOS asks before opening an app that isn't notarized by Apple:
   - **macOS 15 or later:** macOS says it can't verify Skerry. Click **Done**, open **System Settings → Privacy & Security**, scroll down and click **Open Anyway**, then confirm.
   - **macOS 12 to 14:** **right-click Skerry → Open**, then **Open**.
4. When macOS asks, switch Skerry on under **Accessibility** (and **Input Monitoring** if it's listed).
5. On macOS 15 or later, when macOS asks whether Skerry may find devices on your local network, click **Allow**.

If macOS says the app "is damaged", run `xattr -dr com.apple.quarantine /Applications/Skerry.app` in Terminal and open it again.

## Fixing permissions

**Symptom:** Skerry says it needs permission, or macOS keeps asking, although Skerry is switched on in System Settings.

**Coming from Skerry 1.1.2 or earlier (do this once):**

1. Quit Skerry (menu bar icon → **Quit Skerry**).
2. Open **System Settings → Privacy & Security → Accessibility**. Select every entry named Skerry (or `skerry`) and click **−**. Do the same under **Input Monitoring**.
3. Make sure there is only one Skerry, in **Applications** (delete copies in Downloads and eject old disk images).
4. Open Skerry from Applications and allow it when macOS asks.

**After updating Skerry:** if releases are signed with Skerry's own certificate (see below), approvals carry over and nothing is needed. If a release is ad-hoc signed, macOS treats it as a new app: Skerry notices that macOS doesn't allow it, clears the old entries itself (once per installed copy) and macOS asks again. Allow it.

**Still stuck?** Click **Reset permissions** in Skerry's banner and allow Skerry when macOS asks. The same from Terminal:

```sh
tccutil reset Accessibility org.skerry.app
tccutil reset ListenEvent org.skerry.app
```

Then quit and reopen Skerry.

## Fixing Local Network

Only macOS 15 and later have this setting. On macOS 12 to 14, "No route to host" means a real network problem (the other computer is off, asleep or on another network).

**Symptom:** pairing or connecting works from the other computer to the Mac, but not from the Mac to the other computer, and the log shows `No route to host (os error 65)`.

1. Open **System Settings → Privacy & Security → Local Network** and switch Skerry on. If it's already on, switch it **off and on again**.
2. In Skerry, click **Scan network**.
3. Until it works, connect from the other computer: it can always reach the Mac. To pair, click **Scan network** on the other computer, then **Pair** next to the Mac. Once paired, the other computer reconnects to the Mac by itself.

To tell macOS's block apart from a network problem, run this in Terminal (Terminal is exempt from the Local Network check):

```sh
nc -vz <other computer's IP address> 24870
```

If that connects but Skerry can't, macOS is blocking Skerry. If it doesn't connect either, the other computer is off, has a different address, or its firewall blocks Skerry.

**Last resort (macOS 15.5 or later):** let every app reach your home network directly. Replace the network with yours, then restart the Mac:

```sh
sudo defaults write com.apple.network.local-network AllowedWiFiLocalNetworkAddresses -array "192.168.1.0/24"
# for a wired connection as well:
sudo defaults write com.apple.network.local-network AllowedEthernetLocalNetworkAddresses -array "192.168.1.0/24"
```

macOS only lets you remove an app from the Local Network list from macOS 27.2 (the **−** button there). On earlier versions, entries for old copies of Skerry stay in the list; switching the current one off and on is the way to refresh it.

## If the Mac's mouse or keyboard stops responding

Skerry 1.1.3 and earlier could leave the Mac's mouse and keyboard dead after control came back from another computer: they detached the mouse from the cursor while controlling the other computer, and macOS doesn't always let a background app re-attach it. Since 1.1.4 Skerry keeps the hidden cursor parked by moving it back after every movement instead (as Input Leap and Barrier do), so there's nothing to re-attach.

If it still happens:

1. Press **Ctrl + Option + Shift + Esc**: it brings the mouse home and releases everything.
2. If that doesn't help, quit Skerry from another computer's keyboard if you can, or press **⌘ + Option + Esc** and force-quit Skerry. Input comes back as soon as Skerry is gone.
3. Send the **Diagnostics** report: since 1.1.4 its log shows every switch ("controlling …", "back on this computer").

## Checking how your copy is signed

**Settings → Advanced → Help → Diagnostics** in Skerry shows the macOS version, how the app is signed, where it runs from, and whether macOS currently allows Accessibility and Input Monitoring. For example:

```text
App: /Applications/Skerry.app (signed as org.skerry.app by Skerry Code Signing)
Accessibility: allowed, Input Monitoring: allowed
```

| Diagnostics says | Meaning |
|---|---|
| `only the program is signed, as skerry-…` | A build from 1.1.2 or earlier. macOS can never apply its permissions. Install the latest version. |
| `signed as org.skerry.app, ad-hoc` | Signed correctly, but without a certificate. Permissions work; after each update macOS asks again. |
| `signed as org.skerry.app by Skerry Code Signing` | Signed with Skerry's own certificate. Permissions carry over to later updates. |
| `signed as org.skerry.app by Developer ID Application: …` | Signed with an Apple Developer ID. Everything carries over, and Gatekeeper doesn't warn when notarized. |

In Terminal: `codesign --display --verbose=2 /Applications/Skerry.app` (look at `Identifier=`, `Authority=` and `Sealed Resources`).

## For maintainers: signing releases

The Release workflow signs the Mac app in one of three ways, depending on the repository secrets. It fails the build unless the whole app is signed as `org.skerry.app` with sealed resources (`.github/scripts/macos-verify-signature.sh`).

| Secrets | Signature | Accessibility / Input Monitoring after an update | Local Network | Gatekeeper on first launch |
|---|---|---|---|---|
| none | ad-hoc (`signingIdentity: "-"` in `tauri.conf.json`) | asked again | usually works after switching it off and on | warns ("Open Anyway") |
| `MACOS_CERTIFICATE` + `MACOS_CERTIFICATE_PASSWORD` with a self-signed certificate (free) | Skerry Code Signing | **kept** | works, but Apple only promises reliable tracking for Apple-issued certificates | warns ("Open Anyway") |
| the same with an Apple **Developer ID Application** certificate, plus `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | Developer ID, notarized | **kept** | **reliable** | no warning |

### Creating the free signing certificate (step by step)

Do this once. You need a Mac (or Linux, or Git Bash on Windows) and access to the repository's settings.

1. Open **Terminal** on your Mac.
2. Download and run the script. It creates a folder `skerry-signing-certificate` in the current folder (your home folder in a new Terminal window):

   ```sh
   curl -fsSLO https://raw.githubusercontent.com/AviSplash/Skerry/main/.github/scripts/make-macos-certificate.sh
   bash make-macos-certificate.sh
   ```

   It prints `Created "Skerry Code Signing" …` and the next steps.
3. Copy the certificate to the clipboard:

   ```sh
   pbcopy < skerry-signing-certificate/MACOS_CERTIFICATE.txt
   ```

4. In your browser, open the repository's **Settings → Secrets and variables → Actions** ([direct link](https://github.com/AviSplash/Skerry/settings/secrets/actions)) and click **New repository secret**.
   - **Name:** `MACOS_CERTIFICATE`
   - **Secret:** paste (⌘V)
   - Click **Add secret**.
5. Copy the password:

   ```sh
   pbcopy < skerry-signing-certificate/MACOS_CERTIFICATE_PASSWORD.txt
   ```

6. Click **New repository secret** again.
   - **Name:** `MACOS_CERTIFICATE_PASSWORD`
   - **Secret:** paste (⌘V)
   - Click **Add secret**.
7. Keep the `skerry-signing-certificate` folder somewhere safe and private (a password manager or an encrypted backup), and never commit it. If it's lost, make a new certificate the same way: everyone then allows Skerry once more.
8. Run the Release workflow. In its **macOS** job, the step **Set up macOS code signing** should say `macOS builds will be signed with certificate …`, and **Check the macOS app signature** should say `Authority=Skerry Code Signing`.

The first release signed with the certificate is new to macOS once more (allow Skerry once); from then on, approvals carry over.

With the GitHub CLI, steps 3 to 6 are:

```sh
gh secret set MACOS_CERTIFICATE --repo AviSplash/Skerry < skerry-signing-certificate/MACOS_CERTIFICATE.txt
gh secret set MACOS_CERTIFICATE_PASSWORD --repo AviSplash/Skerry < skerry-signing-certificate/MACOS_CERTIFICATE_PASSWORD.txt
```

### Using an Apple Developer ID instead

With a paid [Apple Developer Program](https://developer.apple.com/programs/) membership:

1. In Xcode or on developer.apple.com, create a **Developer ID Application** certificate. In **Keychain Access**, export it with its private key as a `.p12` file and choose a password.
2. Base64-encode it: `base64 -i DeveloperID.p12 | pbcopy`, and save it as the secret `MACOS_CERTIFICATE`; the password goes in `MACOS_CERTIFICATE_PASSWORD`.
3. For notarization, add `APPLE_ID` (your Apple Account email), `APPLE_PASSWORD` (an [app-specific password](https://support.apple.com/102654), not your normal password) and `APPLE_TEAM_ID` (from developer.apple.com → Membership).

The workflow only notarizes when all three are present and the certificate is a Developer ID.

## Background: why this went wrong before 1.1.3

Up to 1.1.2, the Release workflow ran without a signing certificate, and in that case the Tauri bundler doesn't sign the app at all. The only signature was the one the linker adds to the program file inside `Skerry.app`: identifier `skerry-<hash>` instead of `org.skerry.app`, `Info.plist` not bound, no sealed resources. `codesign --verify` accepts that, but macOS doesn't recognise such a copy as the app listed in Privacy & Security. So:

- Accessibility stayed refused however often Skerry was switched on, even on a fresh install, and macOS kept asking. Without it, Skerry can neither capture input (control another computer) nor replay it (be controlled).
- Local Network access didn't apply, so the Mac couldn't open connections to other computers ("No route to host"), while connections *to* the Mac worked. That's why pairing only worked when started on Windows.

Since 1.1.3 the whole app is always signed as `org.skerry.app`, CI and the Release workflow check that on a real Mac, and the certificate options above make approvals survive updates.

References: Apple, [TN3179: Understanding local network privacy](https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy).
