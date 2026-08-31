#!/usr/bin/env python3

import argparse
import hashlib
import json
from pathlib import Path


CHUNK_SIZE = 1024 * 1024


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate VCLogg2 update metadata.")
    parser.add_argument("--archive", required=True, type=Path)
    parser.add_argument("--platform", required=True, choices=("windows", "macos", "linux"))
    parser.add_argument("--architecture", required=True, choices=("x86_64", "aarch64"))
    parser.add_argument("--version", required=True)
    parser.add_argument("--blockmap-name", required=True)
    args = parser.parse_args()

    archive = args.archive.resolve(strict=True)
    output_directory = archive.parent
    chunks: list[str] = []
    whole_hash = hashlib.sha256()
    with archive.open("rb") as stream:
        while chunk := stream.read(CHUNK_SIZE):
            whole_hash.update(chunk)
            chunks.append(hashlib.sha256(chunk).hexdigest())

    blockmap = {
        "schemaVersion": 1,
        "algorithm": "sha256",
        "chunkSize": CHUNK_SIZE,
        "file": archive.name,
        "chunks": chunks,
    }
    manifest = {
        "schemaVersion": 1,
        "product": "VCLogg2",
        "version": args.version,
        "platform": args.platform,
        "architecture": args.architecture,
        "artifact": archive.name,
        "sha256": whole_hash.hexdigest(),
        "size": archive.stat().st_size,
        "blockmap": args.blockmap_name,
    }
    (output_directory / args.blockmap_name).write_text(
        json.dumps(blockmap, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    (output_directory / "latest.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
