"""Release packaging smoke tests; never publish or touch real release binaries."""
import hashlib
import importlib.util
from pathlib import Path
import os
import tarfile
import tempfile
import unittest
import zipfile

SPEC = importlib.util.spec_from_file_location(
    "package_release", Path(__file__).resolve().parents[1] / ".github/scripts/package-release.py"
)
PACKAGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACKAGE)


class PackagingTests(unittest.TestCase):
    def test_archives_checksums_and_tag_guard(self):
        old_cwd = Path.cwd()
        with tempfile.TemporaryDirectory() as temp:
            try:
                os.chdir(temp)
                Path("Cargo.toml").write_text('[package]\nversion = "0.1.0"\n', encoding="utf-8")
                Path("LICENSE").write_text("MIT", encoding="utf-8")
                Path("README.md").write_text("Usage", encoding="utf-8")
                for target in ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-pc-windows-msvc"]:
                    windows = target.endswith("windows-msvc")
                    binary_name = "quota-lite.exe" if windows else "quota-lite"
                    binary = Path("target") / target / "release" / binary_name
                    binary.parent.mkdir(parents=True)
                    binary.write_bytes(b"synthetic binary")
                    binary.chmod(0o755)
                    PACKAGE.package(target, "v0.1.0")
                    stem = f"quota-lite-v0.1.0-{target}"
                    archive = Path("dist") / (stem + (".zip" if windows else ".tar.gz"))
                    expected = f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n"
                    self.assertEqual(archive.with_name(archive.name + ".sha256").read_text(), expected)
                    if windows:
                        with zipfile.ZipFile(archive) as packed:
                            self.assertEqual(packed.read(f"{stem}/{binary_name}"), b"synthetic binary")
                            self.assertEqual(len(packed.namelist()), 3)
                    else:
                        with tarfile.open(archive) as packed:
                            member = packed.getmember(f"{stem}/{binary_name}")
                            self.assertTrue(member.mode & 0o111)
                            self.assertEqual(packed.extractfile(member).read(), b"synthetic binary")
                            self.assertEqual(len(packed.getmembers()), 3)
                with self.assertRaises(ValueError):
                    PACKAGE.package("x86_64-unknown-linux-gnu", "v9.9.9")
            finally:
                os.chdir(old_cwd)


if __name__ == "__main__":
    unittest.main()
