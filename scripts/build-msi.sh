#!/bin/bash
set -e
cd "$(dirname "$0")/.."
echo "=== Building Paper Search MSI ==="
npm install
npm run tauri build
echo "=== Build complete ==="
echo "MSI at: src-tauri/target/release/bundle/msi/"
ls -la src-tauri/target/release/bundle/msi/ 2>/dev/null || echo "Check src-tauri/target/release/bundle/ for output"
