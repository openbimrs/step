# Cloud setup

Use this setup command in Codex or Claude cloud environments, from the repository root:

```bash
#!/usr/bin/env bash
set -euo pipefail
exec ./scripts/cloud-setup.sh
```

The script supports Ubuntu/Debian Linux images with root or passwordless sudo.
Run it during the network-enabled setup phase. It installs image packages and
repository tools, warms dependency caches, and can be rerun. Errors stop setup
with the failing command. `--help` prints usage without installing anything.

Tools installed by setup are exposed through `/usr/local/bin`, so subsequent
agent shells can use them after the setup process exits. Existing proxy and CA
settings are inherited. Rust workspaces use their CI toolchain; setup does not
run builds or tests or claim that the gate passed.

Run the verification command printed at the end. Setup caches dependencies,
but the gate may still need network access for package verification, external
references, browser tools, or optional features. Restricted standards material
is never committed by setup. Optional fuzzing and upstream-parity toolchains
are outside this bootstrap.

Test the bootstrap's command flow without downloads or system changes:

```bash
bash -n scripts/cloud-setup.sh
python3 scripts/test-cloud-setup.py
```
