import importlib.util
from pathlib import Path
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "system/runtime_inventory.py"
spec = importlib.util.spec_from_file_location("runtime_inventory", SCRIPT)
inventory = importlib.util.module_from_spec(spec)
spec.loader.exec_module(inventory)


class RuntimeInventoryTest(unittest.TestCase):
    def test_missing_files_are_explicit_and_binary_changes_change_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            runtime = root / "runtime"
            runtime.mkdir()
            library = runtime / "libhalsim_vmx.so"
            library.write_bytes(b"first binary")
            first = inventory.collect(root, runtime, "test-image", None, "test-release")
            self.assertIsNone(first["image"]["sha256"])
            self.assertIn(str(runtime / "libwpiHal.so"), first["missingRequired"])
            library.write_bytes(b"second binary")
            second = inventory.collect(root, runtime, "test-image", None, "test-release")
            self.assertNotEqual(first["files"][str(library)]["sha256"],
                                second["files"][str(library)]["sha256"])


if __name__ == "__main__":
    unittest.main()
