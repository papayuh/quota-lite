"""Package a native binary and SHA-256 sidecar using only Python's stdlib."""
import hashlib
import os
from pathlib import Path
import re
import sys
import tarfile
import zipfile


def package(target, tag):
    manifest = Path("Cargo.toml").read_text(encoding="utf-8")
    version = re.search(r'^version = "([^"]+)"$', manifest, re.MULTILINE).group(1)
    if tag != f"v{version}":
        raise ValueError("release tag must match Cargo.toml version")
    windows = target.endswith("windows-msvc")
    binary = Path("target") / target / "release" / ("quota-lite.exe" if windows else "quota-lite")
    stem = f"quota-lite-{tag}-{target}"
    dest = Path("dist")
    dest.mkdir(exist_ok=True)
    archive = dest / (stem + (".zip" if windows else ".tar.gz"))
    files = [binary, Path("LICENSE"), Path("README.md")]
    if windows:
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as output:
            for source in files:
                output.write(source, f"{stem}/{source.name}")
    else:
        with tarfile.open(archive, "w:gz") as output:
            for source in files:
                member = output.gettarinfo(source, arcname=f"{stem}/{source.name}")
                member.mode = 0o755 if source == binary else 0o644
                with source.open("rb") as contents:
                    output.addfile(member, contents)
    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_name(archive.name + ".sha256").write_text(
        f"{checksum}  {archive.name}\n", encoding="ascii"
    )


if __name__ == "__main__":
    package(sys.argv[1], os.environ["RELEASE_TAG"])
