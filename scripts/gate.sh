#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
if [[ -z "${CARGO_TARGET_DIR:-}" && -d /mnt/backup/build-cache ]]; then
  export CARGO_TARGET_DIR="/mnt/backup/build-cache/openbim-step-target"
fi

cargo fmt --all -- --check
cargo build --workspace --all-targets --locked
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
cargo package -p openbim-step --allow-dirty
# Read the version rather than hardcoding it: a hardcoded path silently breaks
# the packaged-crate test on every version bump.
version="$(cargo metadata --format-version 1 --no-deps \
  | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="openbim-step"))')"
package_root="${CARGO_TARGET_DIR:-target}/package/openbim-step-${version}"
test -d "$package_root" || {
  echo "packaged crate not found at $package_root" >&2
  exit 1
}
test -f "$package_root/README.md"
test -f "$package_root/LICENSE"
cargo test --manifest-path "$package_root/Cargo.toml" --locked
