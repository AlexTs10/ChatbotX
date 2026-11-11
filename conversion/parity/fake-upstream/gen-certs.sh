#!/bin/sh
# Makes the test CA and the fake upstream leaf certificate in $TLS_DIR.
# Runs at container start, so no private key is ever committed. The CA key is
# deleted after it signs the leaf. Certificates are made again when hosts.txt
# changes or when the leaf expires within 5 days.
set -eu

TLS_DIR="${TLS_DIR:-/parity-tls}"
HOSTS_FILE="${HOSTS_FILE:-/fake-upstream/hosts.txt}"
[ -f "$HOSTS_FILE" ] || { echo "gen-certs: missing $HOSTS_FILE" >&2; exit 1; }
mkdir -p "$TLS_DIR"
chmod 755 "$TLS_DIR"

hosts_hash=$(grep -v '^\s*#' "$HOSTS_FILE" | sed 's/#.*//; s/[[:space:]]//g' | grep -v '^$' | sha256sum | cut -c1-64)

if [ -f "$TLS_DIR/leaf.crt" ] && [ -f "$TLS_DIR/leaf.key" ] && [ -f "$TLS_DIR/ca.crt" ] \
  && [ "$(cat "$TLS_DIR/hosts.sha256" 2>/dev/null)" = "$hosts_hash" ] \
  && openssl x509 -checkend 432000 -noout -in "$TLS_DIR/leaf.crt" >/dev/null 2>&1; then
  echo "gen-certs: reuse existing certificates in $TLS_DIR"
  exit 0
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

san=""
for entry in $(grep -v '^\s*#' "$HOSTS_FILE" | sed 's/#.*//; s/[[:space:]]//g' | grep -v '^$'); do
  case "$entry" in
    *[!0-9.]*) item="DNS:$entry" ;;
    *) item="IP:$entry" ;;
  esac
  san="${san:+$san,}$item"
done

openssl req -x509 -newkey rsa:2048 -nodes -sha256 -days 30 \
  -subj "/CN=ChatbotX Parity Test CA (test only)" \
  -addext "basicConstraints=critical,CA:TRUE" \
  -addext "keyUsage=critical,keyCertSign,cRLSign" \
  -keyout "$work/ca.key" -out "$work/ca.crt" 2>/dev/null

openssl req -newkey rsa:2048 -nodes -sha256 \
  -subj "/CN=parity-fake-upstream" \
  -keyout "$work/leaf.key" -out "$work/leaf.csr" 2>/dev/null

cat >"$work/leaf.ext" <<EOF
basicConstraints=CA:FALSE
keyUsage=critical,digitalSignature,keyEncipherment
extendedKeyUsage=serverAuth
subjectAltName=$san
EOF

openssl x509 -req -sha256 -days 30 -in "$work/leaf.csr" \
  -CA "$work/ca.crt" -CAkey "$work/ca.key" -set_serial "0x$(openssl rand -hex 16)" \
  -extfile "$work/leaf.ext" -out "$work/leaf.crt" 2>/dev/null

install -m 644 "$work/ca.crt" "$TLS_DIR/ca.crt"
install -m 644 "$work/leaf.crt" "$TLS_DIR/leaf.crt"
install -m 600 "$work/leaf.key" "$TLS_DIR/leaf.key"
echo "$hosts_hash" >"$TLS_DIR/hosts.sha256"
echo "gen-certs: new test CA and leaf certificate in $TLS_DIR"
