#!/usr/bin/env bash
set -euo pipefail

features=()
if [[ -n "${CARGO_TEST_FEATURES:-}" ]]; then
  features=(--features "${CARGO_TEST_FEATURES}")
fi
cargo test "${features[@]}"
if [[ -d scripts/tests ]]; then
  python -m unittest discover -s scripts/tests
fi
