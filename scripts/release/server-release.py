"""Publish and promote the server image and its native installer bundle in GHCR."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

IMAGE = os.environ.get("IMAGE", "ghcr.io/arkazzae/stellarr")
KIND = "application/vnd.stellarr.server-installers.v1"
VERSION = "org.opencontainers.image.version"
TITLE = "org.opencontainers.image.title"
TARGETS = [f"stellarr-installer-{arch}-{os_name}" for arch in ("aarch64", "x86_64")
           for os_name in ("apple-darwin", "unknown-linux-musl")]


def oras(*args, cwd=None):
    return subprocess.check_output(["oras", *args], cwd=cwd, text=True).strip()


def installer_tag(version):
    tag = "installer-" + version
    return tag if len(tag) <= 128 else "installer-" + hashlib.sha256(version.encode()).hexdigest()


def published(version):
    return version in oras("repo", "tags", IMAGE).splitlines()


def manifest(tag):
    return json.loads(oras("manifest", "fetch", f"{IMAGE}:{tag}"))


def publish(version, directory):
    assets = [*TARGETS, "stellarr-installer.sha256"]
    tag = installer_tag(version)
    if not published(tag):
        files = [name + ":application/octet-stream" for name in TARGETS]
        oras("push", f"{IMAGE}:{tag}", "--artifact-type", KIND, "--annotation", f"{VERSION}={version}",
             *files, "stellarr-installer.sha256:text/plain", cwd=directory)
    bundle = manifest(tag)
    expected = {}
    for name in assets:
        data = (directory / name).read_bytes()
        expected[name] = ("sha256:" + hashlib.sha256(data).hexdigest(), len(data))
    layers = bundle["layers"]
    actual = {layer["annotations"][TITLE]: (layer["digest"], layer["size"]) for layer in layers}
    if (bundle.get("artifactType") != KIND or bundle["annotations"].get(VERSION) != version
            or len(layers) != len(assets) or actual != expected):
        raise ValueError(f"Published installer manifest {tag} differs from the built files")
    print(f"Verified {len(assets)} installer files in {IMAGE}:{tag}")


def stable_version(text):
    if not re.fullmatch(r"(?:0|[1-9][0-9]*)(?:\.(?:0|[1-9][0-9]*)){2}", text):
        raise ValueError(f"Invalid stable version {text}")
    return tuple(map(int, text.split(".")))


def promote(version):
    incoming = stable_version(version)
    tags = set(oras("repo", "tags", IMAGE).splitlines())
    for alias in (version.rsplit(".", 1)[0], "latest"):
        channel = installer_tag(alias)
        if channel in tags:
            current = manifest(channel)
            if current.get("artifactType") != KIND:
                raise ValueError(f"Unexpected manifest for {channel}")
            if stable_version(current["annotations"][VERSION]) > incoming:
                continue
        # Publish the version selector first. An older queued job then sees the
        # newer version even if a retry is needed to finish the image alias.
        for source, target in ((installer_tag(version), channel), (version, alias)):
            oras("tag", f"{IMAGE}:{source}", target)
            if oras("resolve", f"{IMAGE}:{source}") != oras("resolve", f"{IMAGE}:{target}"):
                raise ValueError(f"Digest mismatch after promoting {target}")
        print(f"Promoted server {version} to {alias}")
        if alias == "latest" and "GITHUB_OUTPUT" in os.environ:
            with open(os.environ["GITHUB_OUTPUT"], "a") as out:
                print("latest=true", file=out)


if __name__ == "__main__":
    action, version, *args = sys.argv[1:]
    if action == "publish":
        publish(version, Path(args[0]).resolve())
    elif action == "promote":
        promote(version)
    else:
        sys.exit("Expected publish or promote")
