#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

echo "==> Updating nix-index database (this takes ~15 minutes first time)…"
nix-index

echo "==> Regenerating discovered.json…"
cargo run --release -- discover --output discovered.json

echo "==> Summary:"
cargo run --release -- stats --input discovered.json

echo "==> Staging for commit…"
git add discovered.json
if git diff --staged --quiet; then
  echo "No changes to discovered.json."
else
  git commit -m "chore: update discovered.json ($(date -u +%Y-%m-%d))"
  echo "Committed. Push when ready."
fi