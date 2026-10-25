#!/usr/bin/env bash
# Run the app in DEVELOPMENT mode: it rebuilds and reloads when you change code.
# The first start compiles for a few minutes; after that it is fast.
set -e
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"
[ -d node_modules ] || npm install
exec npm run tauri dev
