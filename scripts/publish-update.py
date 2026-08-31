#!/usr/bin/env python3

import argparse
import hashlib
import json
import os
import shutil
from pathlib import Path


def safe_file_name(value: object) -> str:
    if not isinstance(value, str) or not value or Path(value).name != value:
        raise ValueError(f"Invalid update file name: {value!r}")
    return value


def file_hash(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description="Publish one VCLogg2 platform update feed.")
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--target", required=True, type=Path)
    args = parser.parse_args()

    source = args.source.resolve(strict=True)
    target = args.target.resolve()
    manifest_path = source / "latest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if (
        manifest.get("schemaVersion") != 1
        or manifest.get("product") != "VCLogg2"
        or manifest.get("platform") not in {"windows", "macos", "linux"}
        or manifest.get("architecture") not in {"x86_64", "aarch64"}
    ):
        raise ValueError("latest.json is not a compatible VCLogg2 update manifest")

    artifact_name = safe_file_name(manifest.get("artifact"))
    blockmap_name = safe_file_name(manifest.get("blockmap"))
    artifact_path = source / artifact_name
    blockmap_path = source / blockmap_name
    if not artifact_path.is_file() or not blockmap_path.is_file():
        raise FileNotFoundError("The update artifact or blockmap is missing")
    if artifact_path.stat().st_size != manifest.get("size"):
        raise ValueError("The update artifact size does not match latest.json")
    if file_hash(artifact_path).lower() != str(manifest.get("sha256", "")).lower():
        raise ValueError("The update artifact SHA-256 does not match latest.json")

    target.mkdir(parents=True, exist_ok=True)
    shutil.copy2(artifact_path, target / artifact_name)
    shutil.copy2(blockmap_path, target / blockmap_name)
    pending_manifest = target / f"latest.json.pending-{os.getpid()}"
    shutil.copy2(manifest_path, pending_manifest)
    pending_manifest.replace(target / "latest.json")
    print(f"Published VCLogg2 {manifest['version']} to: {target}")


if __name__ == "__main__":
    main()
