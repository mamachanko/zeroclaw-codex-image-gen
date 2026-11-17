from pathlib import Path
import tempfile
import unittest

from check_release_ready import require_supported_contract


class ReleaseContractTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.contract = self.root / "wit/direct/services.wit"
        self.contract.parent.mkdir(parents=True)

    def test_blocks_the_repositorys_active_proposed_contract(self):
        with self.assertRaisesRegex(ValueError, "Public release blocked"):
            require_supported_contract(Path(__file__).resolve().parent.parent)

    def test_blocks_missing_or_unrecognized_contracts(self):
        with self.assertRaisesRegex(ValueError, "missing"):
            require_supported_contract(self.root)
        self.contract.write_text("// No package declaration\n")
        with self.assertRaisesRegex(ValueError, "no recognized package"):
            require_supported_contract(self.root)

    def test_blocks_any_proposed_package(self):
        self.contract.write_text("package zeroclaw:proposed-another-service@0.1.0;\n")
        with self.assertRaisesRegex(ValueError, "Public release blocked"):
            require_supported_contract(self.root)

    def test_accepts_a_non_proposed_namespace(self):
        # This checks the publication guard, not implementation by a real host.
        self.contract.write_text("package zeroclaw:image-services@1.0.0;\n")
        require_supported_contract(self.root)


if __name__ == "__main__":
    unittest.main()
