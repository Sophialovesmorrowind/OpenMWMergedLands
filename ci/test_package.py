import hashlib
from pathlib import Path
import stat
import tempfile
import unittest
import zipfile

from package import TARGETS, package


class PackagingTests(unittest.TestCase):
    def test_platform_packages_contain_executable_docs_license_and_valid_checksums(self):
        for platform, target in TARGETS.items():
            with self.subTest(platform=platform), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                name = "merged_lands.exe" if platform.startswith("windows-") else "merged_lands"
                executable = root / "target" / target / "release" / name
                executable.parent.mkdir(parents=True)
                executable.write_bytes(b"test executable")
                (root / "README.md").write_text("test docs", encoding="utf-8")
                (root / "LICENSE").write_text("test license", encoding="utf-8")
                (root / "merged_lands.toml").write_text("private settings", encoding="utf-8")
                result = package(root, platform, target, "test")
                with zipfile.ZipFile(result) as archive:
                    self.assertEqual(set(archive.namelist()), {
                        f"merged_lands/{name}", "merged_lands/README.md",
                        "merged_lands/LICENSE", "merged_lands/BUILD.txt", "merged_lands/Conflicts/",
                    })
                    self.assertEqual(archive.read(f"merged_lands/{name}"), executable.read_bytes())
                    self.assertTrue(archive.getinfo(f"merged_lands/{name}").external_attr >> 16 & stat.S_IXUSR)
                for algorithm in ("sha256", "sha512"):
                    checksum = result.with_name(f"{result.name}.{algorithm}sum.txt").read_text().split()
                    self.assertEqual(checksum, [hashlib.new(algorithm, result.read_bytes()).hexdigest(), result.name])

    def test_missing_or_empty_inputs_fail_before_creating_an_archive(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = TARGETS["linux-amd64"]
            with self.assertRaises(FileNotFoundError):
                package(root, "linux-amd64", target, "test")
            executable = root / "target" / target / "release" / "merged_lands"
            executable.parent.mkdir(parents=True)
            executable.touch()
            with self.assertRaises(ValueError):
                package(root, "linux-amd64", target, "test")
            self.assertFalse((root / "dist").exists())

    def test_mismatched_target_is_rejected(self):
        with self.assertRaises(ValueError):
            package(Path("."), "windows-amd64", TARGETS["linux-amd64"], "test")


if __name__ == "__main__":
    unittest.main()
