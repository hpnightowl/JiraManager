#!/usr/bin/env bash
# Run EVERYTHING that checks the project: type check, frontend tests, production build, Rust tests.
# Stops at the first failure. Takes about a minute (longer the first time).
set -e
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"
[ -d node_modules ] || npm install

echo "==> Type check (Svelte + TypeScript)"
npm run check
echo "==> Frontend tests"
npm test
echo "==> Frontend production build"
npm run build
echo "==> Rust tests"
(cd src-tauri && cargo test)
echo
echo "All checks passed."
