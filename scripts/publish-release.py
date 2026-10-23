#!/usr/bin/env python3
"""Upload, verify, and publish a signed release using its stable numeric ID."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile
from urllib.parse import urlencode


def api(repo, path, method="GET", payload=None):
    command = ["gh", "api", f"repos/{repo}/{path}", "--method", method]
    if payload is not None:
        command += ["--input", "-"]
    result = subprocess.run(
        command,
        input=json.dumps(payload) if payload is not None else None,
        text=True,
        capture_output=True,
        check=True,
    )
    return json.loads(result.stdout) if result.stdout.strip() else None


def find_release(repo, tag):
    page = 1
    while releases := api(repo, f"releases?per_page=100&page={page}"):
        matches = [release for release in releases if release["tag_name"] == tag]
        if len(matches) > 1:
            raise SystemExit("Multiple releases use the requested tag.")
        if matches:
            return matches[0]
        page += 1
    return None


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def require(condition, message):
    if not condition:
        raise SystemExit(message)


def validate_source(repo, release, tag, source_sha):
    remote_sha = api(repo, f"commits/{tag}")["sha"]
    require(remote_sha == source_sha, "The remote tag does not match the build source.")
    require(release["tag_name"] == tag, "The release tag changed.")
    require(release["draft"] is True, "Only an unpublished draft can be finalized.")
    source_link = f"https://github.com/{repo}/commit/{source_sha})"
    require(source_link in (release.get("body") or ""), "The draft names a different source.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--assets-dir", type=Path)
    args = parser.parse_args()
    require(re.fullmatch(r"v\d+\.\d+\.\d+", args.tag), "Expected a versioned release tag.")
    require(re.fullmatch(r"[0-9a-f]{40}", args.source_sha), "Expected a complete source SHA.")

    primaries = [
        "OccluView-Windows-Setup.msi",
        "OccluView-Windows-Portable.zip",
        "OccluView-Linux.deb",
        f"OccluView-{args.tag[1:]}-verification.zip",
    ]
    release = find_release(args.repo, args.tag)
    if args.assets_dir:
        notes = (args.assets_dir / "release-notes.md").read_text()
        for name in ["OccluView-macOS-AppleSilicon.dmg", "OccluView-macOS-AppleSilicon.pkg"]:
            if (args.assets_dir / name).is_file():
                primaries.append(name)
        names = ["latest.json", "latest.json.minisig"]
        for name in primaries:
            names += [name, name + ".sha256", name + ".minisig"]
        require(all((args.assets_dir / name).is_file() for name in names), "Release files are incomplete.")
        if release:
            validate_source(args.repo, release, args.tag, args.source_sha)
            release = api(args.repo, f"releases/{release['id']}", "PATCH", {"name": f"OccluView {args.tag}", "body": notes})
            for asset in release["assets"]:
                api(args.repo, f"releases/assets/{asset['id']}", "DELETE")
        else:
            release = api(args.repo, "releases", "POST", {
                "tag_name": args.tag, "target_commitish": args.source_sha,
                "name": f"OccluView {args.tag}", "body": notes, "draft": True,
            })
        validate_source(args.repo, release, args.tag, args.source_sha)
        upload_url = release["upload_url"].split("{", 1)[0]
        for name in names:
            subprocess.run([
                "gh", "api", upload_url + "?" + urlencode({"name": name}),
                "--method", "POST", "--header", "Content-Type: application/octet-stream",
                "--input", str(args.assets_dir / name),
            ], check=True, stdout=subprocess.DEVNULL)

    require(release is not None, "No draft exists for this release; hand off its source first.")
    release_id = release["id"]
    release = api(args.repo, f"releases/{release_id}")
    validate_source(args.repo, release, args.tag, args.source_sha)
    assets = {asset["name"]: asset for asset in release["assets"]}
    require(len(assets) == len(release["assets"]), "Release asset names are duplicated.")
    if not args.assets_dir:
        primaries += [name for name in ["OccluView-macOS-AppleSilicon.dmg", "OccluView-macOS-AppleSilicon.pkg"] if name in assets]
    names = {"latest.json", "latest.json.minisig"}
    for name in primaries:
        names.update([name, name + ".sha256", name + ".minisig"])
    require(set(assets) == names, "Release assets do not match the expected signed packages.")

    source = subprocess.check_output([
        "git", "show", f"{args.source_sha}:crates/occluview-update/src/lib.rs"
    ], text=True)
    key_match = re.search(r'^pub const UPDATE_PUBKEY: &str = "([^"]+)";', source, re.M)
    require(key_match is not None, "Cannot read the public key compiled into this release.")
    with tempfile.TemporaryDirectory(prefix="occluview-release-assets-") as directory:
        folder = Path(directory)
        for name, asset in assets.items():
            path = folder / name
            with path.open("wb") as stream:
                subprocess.run([
                    "gh", "api", f"repos/{args.repo}/releases/assets/{asset['id']}",
                    "--header", "Accept: application/octet-stream",
                ], check=True, stdout=stream)
            digest = sha256(path)
            require(asset.get("digest") == "sha256:" + digest, f"GitHub asset digest mismatch: {name}")
            if args.assets_dir:
                require(sha256(args.assets_dir / name) == digest, f"Uploaded asset differs from the build: {name}")
        for name in ["latest.json", *primaries]:
            subprocess.run([
                "minisign", "-V", "-P", key_match[1], "-m", str(folder / name),
                "-x", str(folder / (name + ".minisig")),
            ], check=True, stdout=subprocess.DEVNULL)
            if name != "latest.json":
                recorded = (folder / (name + ".sha256")).read_text().split()
                require(recorded == [sha256(folder / name), name], f"Checksum sidecar mismatch: {name}")
        manifest = json.loads((folder / "latest.json").read_text())
        require(manifest["version"] == args.tag[1:], "The update manifest names a different version.")
        for platform in manifest["platforms"].values():
            prefix = f"https://github.com/{args.repo}/releases/download/{args.tag}/"
            require(platform["url"].startswith(prefix), "An update URL points outside this release.")
            name = platform["url"][len(prefix):]
            require(name in primaries, "An update URL does not name a release package.")
            require(platform["sha256"] == sha256(folder / name), "An update hash differs from its package.")
            require(platform["signature"] == (folder / (name + ".minisig")).read_text(), "An update signature differs from its sidecar.")

    current = api(args.repo, f"releases/{release_id}")
    validate_source(args.repo, current, args.tag, args.source_sha)
    snapshot = lambda item: {(asset["id"], asset["name"], asset.get("digest")) for asset in item["assets"]}
    require(snapshot(current) == snapshot(release), "The draft assets changed during verification.")
    published = api(args.repo, f"releases/{release_id}", "PATCH", {"draft": False, "make_latest": "true"})
    require(published["draft"] is False, "GitHub did not publish the release.")
    print(published["html_url"])


if __name__ == "__main__":
    main()
