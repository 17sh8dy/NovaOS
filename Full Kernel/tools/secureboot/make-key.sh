#!/usr/bin/env bash
# Makes Nova's Machine Owner Key (MOK): the key NovaBoot is signed with, which
# you enrol once per PC through shim's MokManager.
#
# The PRIVATE key stays in ~/.nova-keys (outside the repository). Never copy
# nova-mok.key to a USB stick, a chat or git. Only nova-mok.der (the public
# certificate) goes on the stick.
set -euo pipefail
dir="${NOVA_KEY_DIR:-$HOME/.nova-keys}"
mkdir -p "$dir"
chmod 700 "$dir"
if [ -f "$dir/nova-mok.key" ]; then
    echo "A key already exists in $dir (delete it first to make a new one)."
    exit 0
fi
openssl req -new -x509 -newkey rsa:2048 -sha256 -nodes -days 3650 \
    -subj "/CN=Nova OS Machine Owner Key/" \
    -keyout "$dir/nova-mok.key" -out "$dir/nova-mok.crt"
openssl x509 -in "$dir/nova-mok.crt" -outform DER -out "$dir/nova-mok.der"
chmod 600 "$dir/nova-mok.key"
echo "Key made in $dir:"
echo "  nova-mok.key  private: keep it here, never share it"
echo "  nova-mok.crt  public certificate (PEM), for sbsign"
echo "  nova-mok.der  public certificate (DER), for enrolment in MokManager"
