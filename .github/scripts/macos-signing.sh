#!/usr/bin/env bash
# Prepares macOS code signing with Skerry's own certificate.
#
# macOS remembers Accessibility and Input Monitoring permissions by the app's
# code signature. An ad-hoc signature (what you get without a certificate) is
# different for every build, so after each update macOS keeps showing Skerry as
# allowed while treating the new version as a stranger. Signing every build
# with the same certificate keeps the permissions across updates. A
# self-signed certificate is enough for that; it doesn't replace Apple
# notarization (Gatekeeper still asks once on first launch).
#
# Env: MACOS_CERTIFICATE (base64 .p12), MACOS_CERTIFICATE_PASSWORD.
# On success, appends APPLE_SIGNING_IDENTITY to $GITHUB_ENV (Tauri signs with
# it). Without the certificate, or if it can't be used, the build stays
# ad-hoc signed and a warning is printed.
set -euo pipefail

if [ -z "${MACOS_CERTIFICATE:-}" ]; then
  echo "::warning::No MACOS_CERTIFICATE secret: this macOS build is ad-hoc signed, so macOS will ask for permissions again after updating."
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
identity="$(security find-identity -p codesigning "$keychain" | awk '/^ *[0-9]+\)/ { print $2; exit }')"
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
