#!/usr/bin/env bash
# Creates the key pair that signs Skerry's in-app updates. Run it once, on any
# computer with Node.js (it runs the Tauri CLI through npx), then add the three
# values it prints as repository secrets:
#
#   GitHub → Settings → Secrets and variables → Actions → New repository secret
#     TAURI_SIGNING_PRIVATE_KEY           contents of TAURI_SIGNING_PRIVATE_KEY.txt
#     TAURI_SIGNING_PRIVATE_KEY_PASSWORD  contents of TAURI_SIGNING_PRIVATE_KEY_PASSWORD.txt
#     TAURI_UPDATER_PUBLIC_KEY            contents of TAURI_UPDATER_PUBLIC_KEY.txt
#
# (or with the GitHub CLI: the commands are printed at the end).
#
# Every release is then signed with the private key, and every build trusts
# the matching public key, so installed copies can download and install the
# next version. Keep the folder somewhere safe: with a new key pair, installed
# copies can't update to releases signed with it and must be reinstalled by
# hand once.
#
# Usage: make-updater-key.sh [output folder]   (default: ./skerry-updater-key)
set -euo pipefail

out="${1:-skerry-updater-key}"

if ! command -v npx > /dev/null 2>&1; then
  echo "This needs Node.js (for npx). Install the LTS version from https://nodejs.org" >&2
  echo "(or on a Mac with Homebrew: brew install node), then run this again." >&2
  exit 1
fi
if [ -e "$out" ]; then
  echo "$out already exists. Move it away or pass another folder." >&2
  exit 1
fi
umask 077
mkdir -p "$out"

password="$(openssl rand -hex 24)"
npx --yes @tauri-apps/cli@2 signer generate --ci -p "$password" -w "$out/skerry-updater.key" > /dev/null
printf '%s' "$password" > "$out/TAURI_SIGNING_PRIVATE_KEY_PASSWORD.txt"
cp "$out/skerry-updater.key" "$out/TAURI_SIGNING_PRIVATE_KEY.txt"
cp "$out/skerry-updater.key.pub" "$out/TAURI_UPDATER_PUBLIC_KEY.txt"
key_id="$(openssl base64 -d -A -in "$out/skerry-updater.key.pub" | sed -n 's/^.*public key: //p')"

cat << DONE
Created Skerry's update signing key $key_id in $out/

Add these three repository secrets (GitHub → Settings → Secrets and variables →
Actions → New repository secret), pasting each file's contents:
  TAURI_SIGNING_PRIVATE_KEY           $out/TAURI_SIGNING_PRIVATE_KEY.txt
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD  $out/TAURI_SIGNING_PRIVATE_KEY_PASSWORD.txt
  TAURI_UPDATER_PUBLIC_KEY            $out/TAURI_UPDATER_PUBLIC_KEY.txt

Or with the GitHub CLI, from inside your clone of the repository:
  gh secret set TAURI_SIGNING_PRIVATE_KEY < "$out/TAURI_SIGNING_PRIVATE_KEY.txt"
  gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD < "$out/TAURI_SIGNING_PRIVATE_KEY_PASSWORD.txt"
  gh secret set TAURI_UPDATER_PUBLIC_KEY < "$out/TAURI_UPDATER_PUBLIC_KEY.txt"

Then run the Release workflow. Keep $out/ private and backed up; don't
commit it.
DONE
