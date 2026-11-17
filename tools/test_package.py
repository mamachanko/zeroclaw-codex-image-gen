import hashlib
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile

from package import build_package


class PackageTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.target = self.root / "build"
        self.environment = patch.dict(os.environ, {"CARGO_TARGET_DIR": str(self.target)})
        self.environment.start()
        self.addCleanup(self.environment.stop)
        (self.root / "Cargo.toml").write_text('[package]\nversion = "0.2.0-dev.0"\n')
        (self.root / "manifest.toml").write_text(
            'name = "codex-image-gen"\nversion = "0.2.0-dev.0"\n'
            'wasm_path = "codex_image_gen.wasm"\n'
        )
        self.component = self.target / "wasm32-wasip2/release/codex_image_gen.wasm"
        self.component.parent.mkdir(parents=True)
        self.component.write_bytes(b"\x00asm\x0d\x00\x01\x00")
        for name in ["README.md", "LICENSE", "NOTICE", "wit/LICENSE-MIT", "wit/LICENSE-APACHE"]:
            path = self.root / name
            path.parent.mkdir(exist_ok=True)
            path.write_text(f"Fixture for {name}\n")
        # Legacy files must not slip back into the runtime package.
        (self.root / "bridge").mkdir()
        (self.root / "bridge/codex_image_bridge.py").write_text("legacy bridge fixture\n")

    def test_contains_only_component_manifest_docs_and_license_notices(self):
        archive = build_package(self.root)
        with zipfile.ZipFile(archive) as package:
            self.assertIsNone(package.testzip())
            self.assertEqual(set(package.namelist()), {
                "codex-image-gen/" + name for name in [
                    "codex_image_gen.wasm", "manifest.toml", "README.md", "LICENSE", "NOTICE",
                    "wit/LICENSE-MIT", "wit/LICENSE-APACHE",
                ]
            })
            self.assertEqual(package.read("codex-image-gen/codex_image_gen.wasm"), self.component.read_bytes())
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        self.assertEqual(archive.with_suffix(".zip.sha256").read_text(), f"{digest}  {archive.name}\n")

    def test_identical_inputs_produce_identical_archives(self):
        first = build_package(self.root).read_bytes()
        self.assertEqual(first, build_package(self.root).read_bytes())

    def test_rejects_mismatched_versions(self):
        (self.root / "Cargo.toml").write_text('[package]\nversion = "0.1.0"\n')
        with self.assertRaisesRegex(SystemExit, "versions must agree"):
            build_package(self.root)

    def test_requires_a_built_component(self):
        self.component.unlink()
        with self.assertRaisesRegex(SystemExit, "Build first"):
            build_package(self.root)
        self.component.write_bytes(b"\x00asm\x01\x00\x00\x00")
        with self.assertRaisesRegex(SystemExit, "not a core WASM module"):
            build_package(self.root)


if __name__ == "__main__":
    unittest.main()
