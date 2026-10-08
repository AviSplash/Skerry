#!/usr/bin/env bash
# Fails unless a built Skerry.app is signed as a whole, as the app macOS
# should recognise.
#
# A bundle whose program only carries the linker's automatic signature passes
# `codesign --verify`, but macOS doesn't recognise it as the app: it shows up
# in Privacy & Security, the switch can be on, and Accessibility and Local
# Network access still never apply. That is what Skerry 1.1.2 and earlier
# shipped, so check the identifier and sealed resources too.
#
# Usage: macos-verify-signature.sh <path to .app> <bundle identifier>
# Env:   APPLE_SIGNING_IDENTITY (optional): when it names a certificate, the
#        designated requirement must not be tied to this one build.
set -euo pipefail

app="$1"
expected="$2"

codesign --verify --deep --strict --verbose=2 "$app"
info="$(codesign --display --verbose=2 "$app" 2>&1)"
printf '%s\n' "$info"
requirement="$(codesign --display --requirements - "$app" 2>&1 | sed -n 's/^\(# \)\{0,1\}designated => //p')"
echo "Designated requirement: $requirement"

identifier="$(printf '%s\n' "$info" | sed -n 's/^Identifier=//p')"
if [ "$identifier" != "$expected" ]; then
  echo "::error::$app is signed as \"$identifier\", not \"$expected\": macOS won't recognise it as Skerry."
  exit 1
fi
if ! printf '%s\n' "$info" | grep -q '^Sealed Resources'; then
  echo "::error::$app has no sealed resources: only the program is signed, not the app."
  exit 1
fi
if printf '%s\n' "$info" | grep -q 'flags=.*linker-signed'; then
  echo "::error::$app only carries the linker's signature."
  exit 1
fi
if [ -n "${APPLE_SIGNING_IDENTITY:-}" ] && [ "$APPLE_SIGNING_IDENTITY" != "-" ]; then
  case "$requirement" in
    *cdhash*)
      echo "::error::$app was meant to be signed with a certificate, but its requirement is tied to this one build."
      exit 1
      ;;
  esac
fi
echo "$app is signed as $expected."
