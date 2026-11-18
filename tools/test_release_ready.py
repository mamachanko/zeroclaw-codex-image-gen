from pathlib import Path
import tempfile
import unittest

from check_release_ready import require_release_version


class ReleaseVersionTests(unittest.TestCase):
    def test_blocks_the_repositorys_development_version(self):
        with self.assertRaisesRegex(ValueError, "Public release blocked"):
            require_release_version(Path(__file__).resolve().parent.parent)

    def test_requires_a_manifest_and_valid_stable_version(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaisesRegex(ValueError, "missing"):
                require_release_version(root)
            manifest = root / "manifest.toml"
            for version in ('""', '"0.2.0-dev.1"', '42'):
                with self.subTest(version=version):
                    manifest.write_text(f"version = {version}\n")
                    with self.assertRaisesRegex(ValueError, "Public release blocked"):
                        require_release_version(root)
            manifest.write_text('version = "0.2.0"\n')
            require_release_version(root)
