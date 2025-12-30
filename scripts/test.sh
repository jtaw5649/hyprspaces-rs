#!/usr/bin/env bash
set -euo pipefail

cargo test
if [[ -d scripts/tests ]]; then
  python -m unittest discover -s scripts/tests
fi
