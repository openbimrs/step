#!/usr/bin/env python3
"""Exercise cloud bootstrap control flow with isolated command doubles.

No package installation, networking, or writes outside a TemporaryDirectory.
The doubles report commands rather than proving remote services are available.
"""

import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

FAKE = r"""#!/usr/bin/python3
import hashlib, io, json, os, pathlib, subprocess, sys, tarfile
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ['SETUP_LOG'], 'a') as log:
    log.write(json.dumps([name, *args]) + '\n')
if name == os.environ.get('FAIL_TOOL'):
    sys.exit(42)
if name == 'id':
    print(os.environ.get('FAKE_UID', '0'))
elif name == 'sudo':
    if os.environ.get('NO_SUDO'):
        sys.exit(1)
    sys.exit(subprocess.call(args[1:]))
elif name == 'python3':
    if args[:2] == ['-m', 'pip']:
        pass
    else:
        sys.exit(subprocess.call([os.environ['REAL_PYTHON'], *args]))
elif name == 'node':
    if args == ['--version']:
        print('v22.23.3')
    elif os.environ.get('OLD_NODE'):
        sys.exit(1)
elif name == 'wasm-bindgen':
    print('wasm-bindgen ' + os.environ['WASM_VERSION'])
elif name == 'pkl':
    if args == ['--version']:
        print('Pkl ' + os.environ.get('PKL_VERSION', '0.32.1') + ' (Linux)')
elif name == 'git' and 'rev-parse' in args:
    if os.environ.get('WRONG_CORPUS'):
        print('mismatched-existing-revision')
        sys.exit(0)
    print('2c552eae435d575f1df7c11ff9ea19e3d2a324ce' if 'foundation-API' in str(args) else '938bd7636e094c2dc3c65ad6dae846ac3c7b6a9b')
elif name == 'mkdir':
    # Only the system bin directory is intercepted; scratch dirs are real.
    if '/usr/local/bin' not in args:
        sys.exit(subprocess.call(['/bin/mkdir', *args]))
elif name == 'curl':
    output = pathlib.Path(args[args.index('--output') + 1])
    if not os.environ.get('ALLOW_DOWNLOAD'):
        raise SystemExit('unexpected download in the preinstalled-tools scenario')
    archive = output.parent / 'node-v22.23.3-linux-x64.tar.xz'
    if output.name.endswith('.tar.xz'):
        with tarfile.open(output, 'w:xz') as tar:
            for tool in ['node', 'npm', 'npx']:
                data = pathlib.Path(os.environ['FAKE_BIN'], tool).read_bytes()
                info = tarfile.TarInfo('node/bin/' + tool)
                info.mode = 0o755
                info.size = len(data)
                tar.addfile(info, io.BytesIO(data))
    elif output.name == 'SHASUMS256.txt':
        digest = '0' * 64 if os.environ.get('BAD_CHECKSUM') else hashlib.sha256(archive.read_bytes()).hexdigest()
        output.write_text(digest + '  ' + archive.name + '\n')
    elif output.name == 'rustup-init.sh':
        output.write_text('#!/usr/bin/env bash\ncp "$FAKE_BIN/cargo" "$CARGO_HOME/bin/rustup"\nchmod +x "$CARGO_HOME/bin/rustup"\n')
    else:
        output.write_text('deliberately invalid download')
"""


class CloudSetupTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="cloud-setup-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "repository with spaces"
        self.bin = self.root / "bin"
        self.bin.mkdir()
        source = Path(__file__).resolve().parent.parent
        self.repo.mkdir()
        # Only the metadata needed by setup, never the whole source/build tree.
        for name in ["Cargo.lock", "rust-toolchain.toml", "package-lock.json"]:
            if (source / name).exists():
                shutil.copy(source / name, self.repo / name)
        (self.repo / "scripts").mkdir()
        shutil.copy(source / "scripts/cloud-setup.sh", self.repo / "scripts")
        for script in ["fetch-ifc-schemas.sh", "fetch-official-references.py"]:
            path = self.repo / "scripts" / script
            path.write_text('#!/usr/bin/env bash\nprintf "reference fetch\\n"\n')
            path.chmod(0o755)
        (self.repo / "packages/example").mkdir(parents=True)
        (self.repo / "packages/example/PklProject").touch()
        (self.repo / "examples").mkdir()
        self.log = self.root / "commands.jsonl"
        home = self.root / "home"
        cargo_bin = home / ".cargo/bin"
        cargo_bin.mkdir(parents=True)
        tools = [
            "id",
            "sudo",
            "apt-get",
            "python3",
            "cargo",
            "rustup",
            "node",
            "npm",
            "npx",
            "wasm-bindgen",
            "uv",
            "maturin",
            "mkdocs",
            "pkl",
            "git",
            "ln",
            "mkdir",
            "install",
            "chromium",
            "curl",
        ]
        for tool in tools:
            path = self.bin / tool
            path.write_text(FAKE)
            path.chmod(0o755)
        for tool in [
            "rustup",
            "cargo",
            "rustc",
            "rustfmt",
            "cargo-fmt",
            "cargo-clippy",
            "clippy-driver",
            "wasm-bindgen",
            "cargo-deny",
        ]:
            (cargo_bin / tool).symlink_to(self.bin / (tool if tool in tools else "cargo"))
        import re

        lock = (source / "Cargo.lock").read_text() if (source / "Cargo.lock").exists() else ""
        match = re.search(r'name = "wasm-bindgen"\nversion = "([^"]+)"', lock)
        self.env = {
            **os.environ,
            "PATH": str(self.bin) + ":/usr/bin:/bin",
            "HOME": str(home),
            "CARGO_HOME": str(home / ".cargo"),
            "SETUP_LOG": str(self.log),
            "REAL_PYTHON": shutil.which("python3"),
            "FAKE_BIN": str(self.bin),
            "WASM_VERSION": match[1] if match else "unused",
            "OPENCDE_REFERENCE_ROOT": str(self.root / "corpus"),
        }
        for name in [
            "FAIL_TOOL",
            "NO_SUDO",
            "FAKE_UID",
            "CHROME_BIN",
            "OLD_NODE",
            "ALLOW_DOWNLOAD",
            "BAD_CHECKSUM",
            "PKL_VERSION",
            "WRONG_CORPUS",
        ]:
            self.env.pop(name, None)
        (self.root / "corpus").mkdir()
        for name in ["foundation-API", "documents-API"]:
            (self.root / "corpus" / name).mkdir()

    def run_setup(self, *args, **env):
        return subprocess.run(
            ["/bin/bash", str(self.repo / "scripts/cloud-setup.sh"), *args],
            cwd=self.root,
            env={**self.env, **env},
            text=True,
            capture_output=True,
            check=False,
        )

    def commands(self):
        return (
            [json.loads(line) for line in self.log.read_text().splitlines()]
            if self.log.exists()
            else []
        )

    def test_help_has_no_side_effects(self):
        result = self.run_setup("--help")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.commands(), [])

    def test_unknown_option_has_no_side_effects(self):
        result = self.run_setup("--unknown")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(self.commands(), [])

    def test_root_setup_and_rerun_from_another_directory(self):
        for _ in range(2):
            result = self.run_setup()
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("Cloud setup complete", result.stdout)
        commands = self.commands()
        self.assertTrue(any(c[:2] == ["apt-get", "install"] for c in commands))
        if (self.repo / "Cargo.lock").exists():
            self.assertIn(["cargo", "fetch", "--locked"], commands)
        self.assertFalse(any("install" in c and c[0] == "cargo" for c in commands))

    def test_nonroot_uses_noninteractive_sudo(self):
        result = self.run_setup(FAKE_UID="1000")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(any(c[:2] == ["sudo", "-n"] for c in self.commands()))

    def test_missing_privileges_fail_before_install(self):
        result = self.run_setup(FAKE_UID="1000", NO_SUDO="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("passwordless sudo", result.stderr)
        self.assertFalse(any(c[0] == "apt-get" for c in self.commands()))

    def test_package_failure_stops_setup(self):
        result = self.run_setup(FAIL_TOOL="apt-get")
        self.assertEqual(result.returncode, 42)
        self.assertNotIn("Cloud setup complete", result.stdout)
        self.assertFalse(any(c[0] == "cargo" for c in self.commands()))

    def test_node_download_checks_integrity(self):
        if "node_version=" not in (self.repo / "scripts/cloud-setup.sh").read_text():
            self.skipTest("Workspace does not need Node")
        result = self.run_setup(OLD_NODE="1", ALLOW_DOWNLOAD="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(any(c[0] == "curl" for c in self.commands()))

    def test_bad_node_checksum_stops_before_exposure(self):
        if "node_version=" not in (self.repo / "scripts/cloud-setup.sh").read_text():
            self.skipTest("Workspace does not need Node")
        result = self.run_setup(OLD_NODE="1", ALLOW_DOWNLOAD="1", BAD_CHECKSUM="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("Cloud setup complete", result.stdout)
        self.assertFalse(
            any(c[0] == "ln" and c[-1] == "/usr/local/bin/node" for c in self.commands())
        )

    def test_wasm_mismatch_installs_lockfile_version(self):
        if "wasm_version=" not in (self.repo / "scripts/cloud-setup.sh").read_text():
            self.skipTest("Workspace does not need wasm-bindgen")
        expected = self.env["WASM_VERSION"]
        result = self.run_setup(WASM_VERSION="0.0.0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(
            [
                "cargo",
                "+stable",
                "install",
                "wasm-bindgen-cli",
                "--version",
                expected,
                "--locked",
                "--force",
            ],
            self.commands(),
        )

    def test_missing_browser_installs_user_cache(self):
        if "browser_found=" not in (self.repo / "scripts/cloud-setup.sh").read_text():
            self.skipTest("Workspace does not need Chromium")
        (self.bin / "chromium").unlink()
        result = self.run_setup()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(
            ["npx", "--yes", "playwright@1.63.0", "install-deps", "chromium"], self.commands()
        )
        self.assertIn(["npx", "--yes", "playwright@1.63.0", "install", "chromium"], self.commands())

    def test_bad_pkl_checksum_stops_setup(self):
        if "pkl-linux-amd64" not in (self.repo / "scripts/cloud-setup.sh").read_text():
            self.skipTest("Workspace does not need Pkl")
        result = self.run_setup(PKL_VERSION="0.0.0", ALLOW_DOWNLOAD="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("Cloud setup complete", result.stdout)
        self.assertFalse(any(c[0] == "install" for c in self.commands()))

    def test_rustup_bootstrap_when_missing(self):
        if not (self.repo / "Cargo.lock").exists():
            self.skipTest("Pkl does not use Rust")
        (self.bin / "rustup").unlink()
        (Path(self.env["CARGO_HOME"]) / "bin/rustup").unlink()
        result = self.run_setup(ALLOW_DOWNLOAD="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(
            any(c[0] == "curl" and "https://sh.rustup.rs" in c for c in self.commands())
        )
        self.assertTrue(any(c[:3] == ["rustup", "toolchain", "install"] for c in self.commands()))

    def test_existing_cde_corpus_is_preserved_on_mismatch(self):
        if "OPENCDE_REFERENCE_ROOT" not in (self.repo / "scripts/cloud-setup.sh").read_text():
            self.skipTest("Workspace does not use the OpenCDE corpus")
        result = self.run_setup(WRONG_CORPUS="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("refusing to overwrite", result.stderr)
        self.assertFalse(any(c[0] == "git" and "checkout" in c for c in self.commands()))

    def test_dependency_failure_stops_setup(self):
        if not (self.repo / "Cargo.lock").exists():
            self.skipTest("Pkl has no Cargo dependencies")
        result = self.run_setup(FAIL_TOOL="cargo")
        self.assertEqual(result.returncode, 42)
        self.assertNotIn("Cloud setup complete", result.stdout)


if __name__ == "__main__":
    unittest.main()
