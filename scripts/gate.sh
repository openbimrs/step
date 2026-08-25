#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/mnt/backup/build-cache/openbim-step-target}"

cargo fmt --all -- --check
cargo build --workspace --all-targets --locked
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
cargo package -p openbim-step --allow-dirty
package_root="${CARGO_TARGET_DIR:-target}/package/openbim-step-0.1.0"
test -f "$package_root/README.md"
test -f "$package_root/LICENSE"
cargo test --manifest-path "$package_root/Cargo.toml" --locked
