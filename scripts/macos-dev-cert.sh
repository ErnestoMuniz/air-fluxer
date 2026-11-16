#!/bin/sh
# One-time: creates a self-signed code-signing identity so TCC (microphone, screen recording) keeps
# the app's permission across rebuilds. An ad-hoc signature changes its cdhash on every build, which
# macOS treats as a different app and asks again. This certificate is free and stays in your login
# keychain.
set -eu
NAME="${CERT_NAME:-Fluxer Dev}"

if security find-certificate -c "$NAME" >/dev/null 2>&1; then
    echo "identity already exists: $NAME"
    exit 0
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
openssl req -x509 -newkey rsa:2048 -keyout "$TMP/key.pem" -out "$TMP/cert.pem" -days 3650 -nodes \
    -subj "/CN=$NAME" \
    -addext "keyUsage=digitalSignature" -addext "extendedKeyUsage=codeSigning" >/dev/null 2>&1
openssl pkcs12 -export -legacy -macalg sha1 -inkey "$TMP/key.pem" -in "$TMP/cert.pem" -out "$TMP/id.p12" -passout pass:fluxer >/dev/null 2>&1
security import "$TMP/id.p12" -k "$HOME/Library/Keychains/login.keychain-db" -P "fluxer" -T /usr/bin/codesign >/dev/null
echo "created the self-signed identity \"$NAME\""
echo "build with: CODESIGN_IDENTITY=\"$NAME\" scripts/macos-bundle.sh"
