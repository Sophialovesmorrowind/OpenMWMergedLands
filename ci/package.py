"""Package one built target without reading any user configuration or uploading files."""

import argparse
import hashlib
from pathlib import Path
import stat
import zipfile


TARGETS = {
    "linux-amd64": "x86_64-unknown-linux-gnu",
    "linux-arm64": "aarch64-unknown-linux-gnu",
    "macos-amd64": "x86_64-apple-darwin",
    "macos-arm64": "aarch64-apple-darwin",
    "windows-amd64": "x86_64-pc-windows-msvc",
    "android-arm64": "aarch64-linux-android",
}


def package(root: Path, platform: str, target: str, version: str) -> Path:
    if TARGETS.get(platform) != target:
        raise ValueError(f"Unsupported platform/target pair: {platform}/{target}")
    executable = "merged_lands.exe" if platform.startswith("windows-") else "merged_lands"
    inputs = {
        executable: root / "target" / target / "release" / executable,
        "README.md": root / "README.md",
        "LICENSE": root / "LICENSE",
    }
    contents = {}
    for name, source in inputs.items():
        data = source.read_bytes()
        if not data:
            raise ValueError(f"Empty package input: {source}")
        contents[name] = data
    contents["BUILD.txt"] = f"Version: {version}\nTarget: {target}\n".encode()
    contents["Conflicts/"] = b""

    output = root / "dist" / f"merged_lands-{platform}.zip"
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.with_suffix(".zip.tmp")
    try:
        with zipfile.ZipFile(temporary, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            for name, data in contents.items():
                entry = zipfile.ZipInfo(f"merged_lands/{name}")
                entry.create_system = 3
                entry.compress_type = zipfile.ZIP_DEFLATED
                mode = stat.S_IFREG | (0o755 if name == executable else 0o644)
                if name.endswith("/"):
                    mode = stat.S_IFDIR | 0o755
                entry.external_attr = mode << 16
                archive.writestr(entry, data)
        temporary.replace(output)
    finally:
        temporary.unlink(missing_ok=True)
    data = output.read_bytes()
    for algorithm in ("sha256", "sha512"):
        digest = hashlib.new(algorithm, data).hexdigest()
        output.with_name(f"{output.name}.{algorithm}sum.txt").write_text(
            f"{digest}  {output.name}\n", encoding="utf-8"
        )
    return output


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("platform", choices=TARGETS)
    parser.add_argument("target")
    parser.add_argument("--version", default="development")
    args = parser.parse_args()
    print(package(Path(__file__).resolve().parent.parent, args.platform, args.target, args.version))
