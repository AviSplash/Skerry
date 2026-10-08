#!/usr/bin/env bash
# Prepares macOS code signing with Skerry's own certificate.
#
# macOS identifies an app by its code signature: Accessibility and Input
# Monitoring approvals, and Local Network access, all belong to the signature
# they were given to. Without a certificate the app is ad-hoc signed
# (tauri.conf.json: "signingIdentity": "-"): the whole bundle is signed as
# org.skerry.app, so macOS can recognise it, but every build is a different
# identity, so after each update macOS keeps showing Skerry as allowed while
# treating the new version as a stranger. Signing every build with the same
# certificate keeps the approvals across updates:
#
# - a self-signed certificate (free, see make-macos-certificate.sh) keeps
#   Accessibility and Input Monitoring; Gatekeeper still asks once on first
#   launch;
# - an Apple "Developer ID Application" certificate (Apple Developer Program)
#   also makes Local Network access reliable and, with the notarization
#   secrets below, lets Skerry open without any Gatekeeper warning.
#
# Env: MACOS_CERTIFICATE (base64 .p12), MACOS_CERTIFICATE_PASSWORD.
#      Optional, for notarization with a Developer ID certificate:
#      APPLE_ID, APPLE_PASSWORD (an app-specific password), APPLE_TEAM_ID.
# On success, appends APPLE_SIGNING_IDENTITY to $GITHUB_ENV (Tauri signs with
# it, and notarizes when the APPLE_* variables are set too). Without the
# certificate, or if it can't be used, the build stays ad-hoc signed and a
# warning is printed.
set -euo pipefail

if [ -z "${MACOS_CERTIFICATE:-}" ]; then
  echo "::warning::No MACOS_CERTIFICATE secret: this macOS build is ad-hoc signed, so macOS will ask for permissions again after updating. Run .github/scripts/make-macos-certificate.sh once to fix that."
  exit 0
fi

tmp="${RUNNER_TEMP:-$(mktemp -d)}"
keychain="$tmp/skerry-signing.keychain-db"
keychain_password="$(openssl rand -hex 16)"

printf '%s' "$MACOS_CERTIFICATE" | base64 --decode > "$tmp/skerry-signing.p12"
security create-keychain -p "$keychain_password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"
security import "$tmp/skerry-signing.p12" -k "$keychain" -P "${MACOS_CERTIFICATE_PASSWORD:-}" -T /usr/bin/codesign
rm -f "$tmp/skerry-signing.p12"
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" "$keychain" > /dev/null
# codesign looks identities up in the user search list.
existing=()
while IFS= read -r k; do
  k="$(printf '%s' "$k" | sed -e 's/^[[:space:]]*"//' -e 's/"[[:space:]]*$//')"
  if [ -n "$k" ]; then existing+=("$k"); fi
done < <(security list-keychains -d user)
security list-keychains -d user -s "$keychain" ${existing[@]+"${existing[@]}"}

# A self-signed certificate isn't "valid" to `find-identity -v`, but
# codesign uses it. Sign by its SHA-1 hash so the name can't be ambiguous.
identities="$(security find-identity -p codesigning "$keychain")"
identity="$(printf '%s\n' "$identities" | awk '/^ *[0-9]+\)/ { print $2; exit }')"
if [ -z "$identity" ]; then
  echo "::warning::MACOS_CERTIFICATE holds no code-signing identity: this macOS build is ad-hoc signed."
  exit 0
fi

# Prove it works before the real build relies on it.
probe="$tmp/skerry-signing-probe"
cp /usr/bin/true "$probe"
if ! codesign --force --sign "$identity" --options runtime --identifier org.skerry.signing-probe "$probe" \
  || ! codesign --verify "$probe" || ! "$probe"; then
  echo "::warning::Couldn't sign with MACOS_CERTIFICATE: this macOS build is ad-hoc signed."
  exit 0
fi
echo "Designated requirement of a probe signed with this certificate:"
codesign --display --requirements - "$probe" 2>&1 | grep designated || true

echo "APPLE_SIGNING_IDENTITY=$identity" >> "${GITHUB_ENV:-/dev/null}"
echo "macOS builds will be signed with certificate $identity."

# Notarize only with a Developer ID certificate and complete credentials:
# Tauri tries to notarize as soon as APPLE_ID is set, even to an empty value.
if printf '%s\n' "$identities" | grep -q '"Developer ID Application:'; then
  if [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
    {
      echo "APPLE_ID=$APPLE_ID"
      echo "APPLE_PASSWORD=$APPLE_PASSWORD"
      echo "APPLE_TEAM_ID=$APPLE_TEAM_ID"
    } >> "${GITHUB_ENV:-/dev/null}"
    echo "The app will be notarized by Apple."
  else
    echo "::warning::Developer ID certificate without the APPLE_ID, APPLE_PASSWORD and APPLE_TEAM_ID secrets: the app is signed but not notarized, so Gatekeeper still asks on first launch."
  fi
fi
