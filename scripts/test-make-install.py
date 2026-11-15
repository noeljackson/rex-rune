#!/usr/bin/env python3
"""Exercise Make and the real installer in private paths, with no Rex server.

Requires GNU Make, Python 3, Bash, coreutils and desktop-file-utils. A tiny Cargo
fixture generates an executable to check build/install ordering and failure
propagation; use REX_INSTALL_TEST_BIN for an additional real-binary copy check.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
MAKE = shutil.which("make")
RESET = {"PREFIX", "BINDIR", "DATADIR", "DESTDIR", "CARGO", "RUSTC", "ZIG",
         "CARGO_TARGET_DIR", "CARGO_BUILD_TARGET", "REX_BIN", "MAKEFLAGS", "MFLAGS",
         "MAKELEVEL", "MAKEOVERRIDES"}


def executable(path, content):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content)
    path.chmod(0o755)


class InstallTests(unittest.TestCase):
    def setUp(self):
        # Some workstations mount /tmp noexec; these checks execute installed files.
        cache = Path(os.environ.get("REX_TEST_TMPDIR", os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")))
        cache.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix="rex-make-install-", dir=cache)
        self.addCleanup(self.temp.cleanup)
        self.work = Path(self.temp.name)
        self.home = self.work / 'home with "quotes" and \\slashes'
        self.home.mkdir()
        self.bin = self.home / "custom bin"
        self.data = self.home / "custom data"
        self.target = self.work / "cargo output"
        self.tools = self.work / "tools"
        self.tools.mkdir()
        self.log = self.work / "cargo.jsonl"
        self.cache_log = self.work / "cache-calls"
        self.env = {k: v for k, v in os.environ.items() if k not in RESET and not k.startswith("XDG_")}
        self.env.update(HOME=str(self.home), XDG_BIN_HOME=str(self.bin),
                        XDG_DATA_HOME=str(self.data), CARGO_TARGET_DIR=str(self.target),
                        PATH=str(self.tools) + os.pathsep + os.environ["PATH"],
                        TEST_CARGO_LOG=str(self.log), TEST_CACHE_LOG=str(self.cache_log))
        self.cargo = self.tools / "cargo fixture"
        executable(self.cargo, '''#!/usr/bin/env python3
import json, os, pathlib, sys
# Like Cargo itself, refuse an empty target override rather than ignoring it.
if 'CARGO_BUILD_TARGET' in os.environ and not os.environ['CARGO_BUILD_TARGET']:
    sys.exit('target was empty')
with open(os.environ['TEST_CARGO_LOG'], 'a') as log:
    log.write(json.dumps(sys.argv[1:]) + '\\n')
if os.environ.get('TEST_BUILD_FAIL'):
    sys.exit(66)
if sys.argv[1] == 'build':
    target = pathlib.Path(os.environ['CARGO_TARGET_DIR'])
    if os.environ.get('CARGO_BUILD_TARGET'):
        target /= os.environ['CARGO_BUILD_TARGET']
    binary = target / 'release/rex-linux'
    binary.parent.mkdir(parents=True, exist_ok=True)
    binary.write_text('#!/bin/sh\\nprintf "rex-linux fixture\\\\n"\\n')
    binary.chmod(0o755)
''')
        executable(self.tools / "rustc", "#!/bin/sh\necho 'rustc 1.96.1 (fixture)'\n")
        executable(self.tools / "zig", "#!/bin/sh\necho 0.17.1\n")
        # Never rebuild desktop caches on the real desktop during these tests.
        for tool in ("update-desktop-database", "gtk-update-icon-cache"):
            executable(self.tools / tool, '#!/bin/sh\nprintf "called\\n" >> "$TEST_CACHE_LOG"\n')
        self.env.update(CARGO=str(self.cargo), RUSTC=str(self.tools / "rustc"), ZIG=str(self.tools / "zig"))
        self.prefs = self.home / ".config/rex-linux/workspace.json"
        self.prefs.parent.mkdir(parents=True)
        self.prefs.write_text("keep preferences")

    def make(self, *args, ok=True, env=None):
        result = subprocess.run([MAKE, "--no-print-directory", *args], cwd=ROOT,
                                env=env or self.env, text=True, capture_output=True, timeout=30)
        self.assertEqual(result.returncode == 0, ok, result.stdout + result.stderr)
        return result

    def check_install(self, binary, data):
        self.assertEqual(subprocess.check_output([str(binary)], text=True).strip(), "rex-linux fixture")
        desktop = data / "applications/dev.shuv.rex-linux.desktop"
        subprocess.run(["desktop-file-validate", str(desktop)], check=True, capture_output=True)
        for size in (32, 128, 256):
            self.assertTrue((data / f"icons/hicolor/{size}x{size}/apps/dev.shuv.rex-linux.png").is_file())
        return desktop

    def test_build_install_is_independent_and_uninstall_preserves_other_files(self):
        self.make("install")
        self.check_install(self.bin / "rex-linux", self.data)
        self.assertTrue(self.log.exists(), "install must build the previously absent binary")
        shutil.rmtree(self.target)
        self.check_install(self.bin / "rex-linux", self.data)
        self.make("install")
        sentinel = self.data / "applications/other.desktop"
        sentinel.write_text("unrelated")
        for _ in range(2):
            self.make("uninstall")
        self.assertFalse((self.bin / "rex-linux").exists())
        self.assertFalse((self.data / "applications/dev.shuv.rex-linux.desktop").exists())
        self.assertEqual(self.prefs.read_text(), "keep preferences")
        self.assertEqual(sentinel.read_text(), "unrelated")

    def test_default_install_uses_local_bin_and_share(self):
        env = {k: v for k, v in self.env.items() if not k.startswith("XDG_")}
        self.make("install", env=env)
        binary = self.home / ".local/bin/rex-linux"
        self.check_install(binary, self.home / ".local/share")
        self.make("uninstall", env=env)
        self.assertFalse(binary.exists())

    def test_prefix_staging_uses_final_paths_and_skips_caches(self):
        prefix = self.work / 'final prefix with "quotes"'
        stage = self.work / "stage"
        self.make("install", f"PREFIX={prefix}", f"DESTDIR={stage}")
        staged = stage / str(prefix).lstrip("/")
        desktop = self.check_install(staged / "bin/rex-linux", staged / "share")
        contents = desktop.read_text()
        self.assertNotIn(str(stage), contents)
        self.assertIn('TryExec=' + str(prefix / "bin/rex-linux"), contents)
        self.assertFalse(prefix.exists(), "nothing may escape the staging tree")
        self.assertFalse(self.bin.exists(), "explicit PREFIX must override ambient XDG paths")
        self.assertFalse(self.cache_log.exists(), "staging must not refresh any desktop cache")
        sentinel = staged / "share/keep"
        sentinel.write_text("unrelated")
        for _ in range(2):
            self.make("uninstall", f"PREFIX={prefix}", f"DESTDIR={stage}")
        self.assertFalse((staged / "bin/rex-linux").exists())
        self.assertEqual(sentinel.read_text(), "unrelated")
        self.assertFalse(self.cache_log.exists())

    def test_install_only_needs_no_compiler_and_respects_directory_overrides(self):
        self.make("build")
        binary = self.target / "release/rex-linux"
        bindir, datadir = self.work / "bin override", self.work / "data override"
        args = [f"BINDIR={bindir}", f"DATADIR={datadir}", f"REX_BIN={binary}",
                "CARGO=/missing/cargo", "RUSTC=/missing/rustc", "ZIG=/missing/zig"]
        self.make("install-only", *args)
        self.check_install(bindir / "rex-linux", datadir)
        # A different prefix must remain untouched by uninstall.
        other = self.work / "other prefix/bin/rex-linux"
        other.parent.mkdir(parents=True)
        other.write_text("keep other installation")
        self.make("uninstall", *args)
        self.assertFalse((bindir / "rex-linux").exists())
        self.assertEqual(other.read_text(), "keep other installation")

    def test_failed_build_and_unsupported_tools_never_install(self):
        for updates, message in (({"TEST_BUILD_FAIL": "1"}, "Error"),
                                 ({"ZIG": "/missing/zig"}, "Build requirement:")):
            result = self.make("install", env={**self.env, **updates}, ok=False)
            self.assertIn(message, result.stderr)
            self.assertFalse(self.bin.exists())
        self.log.unlink()
        executable(self.tools / "zig", "#!/bin/sh\necho 0.15.2\n")
        result = self.make("install", ok=False)
        self.assertIn("Zig 0.17.x required; found 0.15.2", result.stderr)
        self.assertFalse(self.log.exists(), "an unsupported compiler must fail before building")

    def test_missing_desktop_tools_preserves_receipt(self):
        self.make("build")
        only = self.work / "minimal PATH"
        only.mkdir()
        for tool in ("bash", "realpath", "dirname", "install"):
            (only / tool).symlink_to(shutil.which(tool))
        receipt = self.data / "rex-linux/install.json"
        receipt.parent.mkdir(parents=True)
        receipt.write_text("keep existing receipt")
        result = self.make("install-only", env={**self.env, "PATH": str(only)}, ok=False)
        self.assertIn("Install requirement: missing desktop-file-install", result.stderr)
        self.assertEqual(receipt.read_text(), "keep existing receipt")
        self.assertFalse(self.bin.exists())

    def test_invalid_install_paths_fail_before_writing(self):
        self.make("build")
        for arg in ("PREFIX=relative", "DESTDIR=relative", "DATADIR=/tmp/line\nbreak"):
            self.make("install-only", arg, ok=False)
            self.assertFalse(self.bin.exists())

    def test_target_triple_uses_the_matching_artifact(self):
        self.make("install", "CARGO_BUILD_TARGET=fixture-target")
        self.check_install(self.bin / "rex-linux", self.data)
        self.assertTrue((self.target / "fixture-target/release/rex-linux").is_file())

    @unittest.skipUnless(os.environ.get("REX_INSTALL_TEST_BIN"), "set REX_INSTALL_TEST_BIN for a real release artifact")
    def test_real_binary_survives_removal_of_its_source_copy(self):
        source = self.work / "source/rex-linux"
        source.parent.mkdir()
        shutil.copy2(os.environ["REX_INSTALL_TEST_BIN"], source)
        self.make("install-only", f"REX_BIN={source}")
        source.unlink()
        installed = self.bin / "rex-linux"
        version = subprocess.check_output([str(installed), "--version"], text=True).strip()
        self.assertRegex(version, r"^rex-linux \d+\.\d+\.\d+$")
        self.make("uninstall")
        self.assertFalse(installed.exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)
