#!/usr/bin/env bash
# Creates Skerry's own (self-signed) code-signing certificate for the Mac
# builds. Run it once, on any computer with openssl (macOS, Linux, or Git
# Bash on Windows), then add the two values it prints as repository secrets:
#
#   GitHub → Settings → Secrets and variables → Actions → New repository secret
#     MACOS_CERTIFICATE           contents of MACOS_CERTIFICATE.txt
#     MACOS_CERTIFICATE_PASSWORD  contents of MACOS_CERTIFICATE_PASSWORD.txt
#
# (or with the GitHub CLI: the commands are printed at the end).
#
# Every Mac release is then signed with this certificate, so macOS keeps
# Skerry's Accessibility and Input Monitoring approvals across updates. Keep
# the folder somewhere safe: a new certificate means everyone allows Skerry
# once more. It doesn't replace an Apple Developer ID: Gatekeeper still asks
# once when Skerry is first opened.
#
# Usage: make-macos-certificate.sh [output folder]   (default: ./skerry-signing-certificate)
# Env:   OPENSSL (optional): the openssl program to use.
set -euo pipefail

out="${1:-skerry-signing-certificate}"
openssl="${OPENSSL:-openssl}"
name="Skerry Code Signing"

if [ -e "$out" ]; then
  echo "$out already exists. Move it away or pass another folder." >&2
  exit 1
fi
umask 077
mkdir -p "$out"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

cat > "$work/req.cnf" <<CNF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $name
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
subjectKeyIdentifier = hash
CNF

# Valid for 20 years: macOS keeps the approvals for as long as releases are
# signed with it.
"$openssl" req -x509 -newkey rsa:2048 -sha256 -days 7300 -nodes -config "$work/req.cnf" \
  -keyout "$work/key.pem" -out "$work/cert.pem" 2> "$work/req.log" || {
  cat "$work/req.log" >&2
  exit 1
}

password="$("$openssl" rand -hex 16)"
# macOS's `security import` only reads the older PKCS#12 encryption. OpenSSL
# 3 needs to be told to use it; LibreSSL (macOS's openssl) uses it already.
p12="$out/skerry-signing.p12"
export_p12() {
  "$openssl" pkcs12 -export -inkey "$work/key.pem" -in "$work/cert.pem" -name "$name" \
    -out "$p12" -passout "pass:$password" "$@" 2>> "$work/p12.log"
}
export_p12 -keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES -macalg sha1 \
  || export_p12 -keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES \
  || {
    cat "$work/p12.log" >&2
    exit 1
  }

"$openssl" base64 -A -in "$p12" > "$out/MACOS_CERTIFICATE.txt"
printf '%s' "$password" > "$out/MACOS_CERTIFICATE_PASSWORD.txt"
cp "$work/cert.pem" "$out/skerry-signing.crt"

fingerprint="$("$openssl" x509 -in "$work/cert.pem" -noout -fingerprint -sha1 | sed 's/^.*=//')"
cat <<DONE
Created "$name" (SHA-1 $fingerprint) in $out/

Add these two repository secrets (GitHub → Settings → Secrets and variables →
Actions → New repository secret), pasting each file's contents:
  MACOS_CERTIFICATE           $out/MACOS_CERTIFICATE.txt
  MACOS_CERTIFICATE_PASSWORD  $out/MACOS_CERTIFICATE_PASSWORD.txt

Or with the GitHub CLI, from inside your clone of the repository:
  gh secret set MACOS_CERTIFICATE < "$out/MACOS_CERTIFICATE.txt"
  gh secret set MACOS_CERTIFICATE_PASSWORD < "$out/MACOS_CERTIFICATE_PASSWORD.txt"

Then run the Release workflow. Keep $out/ private and backed up; don't
commit it.
DONE
