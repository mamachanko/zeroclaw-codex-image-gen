#!/usr/bin/env python3
"""Package the WASM component, manifest, and notices. Development tooling only."""
from pathlib import Path
import hashlib
import os
import tomllib
import zipfile


def build_package(root: Path) -> Path:
    manifest = tomllib.loads((root / "manifest.toml").read_text())
    cargo = tomllib.loads((root / "Cargo.toml").read_text())
    if manifest["version"] != cargo["package"]["version"]:
        raise SystemExit("Cargo.toml and manifest.toml versions must agree")
    name, version = manifest["name"], manifest["version"]
    target = Path(os.environ.get("CARGO_TARGET_DIR", root / "target"))
    if not target.is_absolute():
        target = root / target
    component = target / "wasm32-wasip2/release" / manifest["wasm_path"]
    if not component.is_file():
        raise SystemExit("Build first: cargo build --locked --target wasm32-wasip2 --release")
    data = component.read_bytes()
    if data[:8] != b"\x00asm\x0d\x00\x01\x00":
        raise SystemExit("Expected a WebAssembly component, not a core WASM module")
    files = {
        manifest["wasm_path"]: data,
        **{path: (root / path).read_bytes() for path in (
            "manifest.toml", "README.md", "LICENSE", "NOTICE",
            "wit/LICENSE-MIT", "wit/LICENSE-APACHE",
        )},
    }
    destination = root / "dist"
    destination.mkdir(exist_ok=True)
    archive = destination / f"{name}-{version}.zip"
    # Fixed timestamps/modes make packaging reproducible for identical inputs.
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as package:
        for path, content in sorted(files.items()):
            info = zipfile.ZipInfo(f"{name}/{path}", date_time=(2026, 1, 1, 0, 0, 0))
            info.create_system = 3
            info.external_attr = 0o644 << 16
            package.writestr(info, content, compress_type=zipfile.ZIP_DEFLATED)
    with zipfile.ZipFile(archive) as package:
        if package.testzip() is not None:
            raise SystemExit("Package integrity check failed")
    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_suffix(".zip.sha256").write_text(f"{checksum}  {archive.name}\n")
    return archive


def main():
    print(build_package(Path(__file__).resolve().parent.parent))


if __name__ == "__main__":
    main()
