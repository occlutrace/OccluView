#!/usr/bin/env python3
"""Write ``dist/latest.json``, the signed update manifest.

The manifest shape is a contract with installed copies: the reader in
``crates/occluview-update`` deserialises these fields and matches the platform
keys. Adding a field is safe -- older clients ignore what they do not know --
but changing an existing field's meaning must bump ``SCHEMA`` on both sides.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

# Keep in step with MANIFEST_SCHEMA in crates/occluview-update/src/lib.rs.
SCHEMA = 1

WINDOWS_PLATFORM = "windows-x86_64"
LINUX_PLATFORM = "linux-x86_64"
MACOS_PLATFORM = "macos-aarch64"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version", help="release version, without the leading v")
    parser.add_argument("tag", help="release tag, for example v1.3.0")
    parser.add_argument("windows_name", help="Windows MSI file name")
    parser.add_argument("windows_sha256", help="lowercase hex SHA-256 of the MSI")
    parser.add_argument("windows_signature", help="path to the MSI .minisig")
    parser.add_argument("linux_name", help="Linux .deb file name")
    parser.add_argument("linux_sha256", help="lowercase hex SHA-256 of the .deb")
    parser.add_argument("linux_signature", help="path to the .deb .minisig")
    parser.add_argument(
        "macos_name", nargs="?", default="", help="macOS .pkg file name, when notarized"
    )
    parser.add_argument(
        "macos_sha256", nargs="?", default="", help="lowercase hex SHA-256 of the .pkg"
    )
    parser.add_argument(
        "macos_signature", nargs="?", default="", help="path to the .pkg .minisig"
    )
    parser.add_argument(
        "--output", default="dist/latest.json", help="where to write the manifest"
    )
    return parser.parse_args()


def platform_entry(
    release_url: str, name: str, sha256: str, signature_path: str
) -> dict[str, str]:
    return {
        "url": f"{release_url}/{name}",
        "signature": Path(signature_path).read_text(encoding="utf-8"),
        "sha256": sha256,
    }


def main() -> int:
    args = parse_args()
    release_url = f"https://github.com/occlutrace/OccluView/releases/download/{args.tag}"
    platforms = {
        WINDOWS_PLATFORM: platform_entry(
            release_url,
            args.windows_name,
            args.windows_sha256,
            args.windows_signature,
        ),
        LINUX_PLATFORM: platform_entry(
            release_url,
            args.linux_name,
            args.linux_sha256,
            args.linux_signature,
        ),
    }
    if args.macos_name:
        platforms[MACOS_PLATFORM] = platform_entry(
            release_url,
            args.macos_name,
            args.macos_sha256,
            args.macos_signature,
        )
    manifest = {
        "schema": SCHEMA,
        "version": args.version,
        "notes": f"OccluView {args.tag} \u2014 see the GitHub release for details.",
        "platforms": platforms,
    }
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
