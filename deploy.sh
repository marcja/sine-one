#!/bin/bash
set -e
PLUGIN_NAME="sine_one"
BUNDLE="target/bundled/SineOne.clap"
CLAP_DIR="$HOME/Library/Audio/Plug-Ins/CLAP"

# Build
cargo xtask bundle "$PLUGIN_NAME" --release

# Validate CLAP compliance (fail fast)
clap-validator validate "$BUNDLE" --only-failed

# Install
cp -r "$BUNDLE" "$CLAP_DIR/"
echo "Installed to $CLAP_DIR/SineOne.clap"
echo "→ Rescan plugins in Bitwig: Preferences > Plug-ins > Rescan"
