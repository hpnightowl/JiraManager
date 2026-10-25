#!/usr/bin/env bash
# Shared by the other scripts: makes Rust (cargo) and Node (nvm) available, and goes to the project folder.
# Not meant to be run by itself.
export PATH="$HOME/.cargo/bin:$PATH"
export NVM_DIR="${NVM_DIR:-$HOME/.nvm}"
[ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh"

for tool in cargo node npm; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "'$tool' not found. Install it first (Rust via rustup.rs, Node 22 via nvm; see README.md)" >&2
    exit 1
  }
done
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
