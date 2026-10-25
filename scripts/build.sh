#!/usr/bin/env bash
# Build the optimized app and the installable Debian package. Takes about 4 minutes the first time.
#   scripts/build.sh              build and show where the files are
#   scripts/build.sh --install    ...and install the package with apt (asks for sudo)
set -e
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"
[ -d node_modules ] || npm install

npm run tauri build -- --bundles deb

BIN="$PWD/src-tauri/target/release/project-jira"
DEB="$(ls "$PWD"/src-tauri/target/release/bundle/deb/*.deb | head -1)"
echo
echo "Built:"
echo "  app      $BIN"
echo "  package  $DEB"
echo
echo "Run the app:      $BIN"
echo "Install package:  sudo apt install '$DEB'"

if [ "${1:-}" = "--install" ]; then
  sudo apt-get install -y "$DEB"
  echo "Installed. Start it with:  project-jira"
fi
