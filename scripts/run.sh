#!/usr/bin/env bash
# Start the app you already built (no compiling). The app starts the local AI itself and stops it when you close the window.
BIN="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/src-tauri/target/release/project-jira"
[ -x "$BIN" ] || { echo "Not built yet. Run:  scripts/build.sh" >&2; exit 1; }
exec "$BIN"
