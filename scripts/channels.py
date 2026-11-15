#!/usr/bin/env python3
"""Release-channel operations shared by the workflows and their executable proofs."""

import argparse
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import time
import tomllib

sys.path.insert(0, str(Path(__file__).resolve().parent))
import linux_binary  # noqa: E402


CANARY = re.compile(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)-canary-([0-9a-f]{7})")
BUCKETS = ("zega-releases", "zega-wasm")
METADATA = ("manifest.json", "release.json")
# windows-x64 is paused (Sami, 30 Oct): no Windows runner for now. native_artifact keeps its
# .exe rule so putting it back is this one tuple, release.yml's matrix and npm/pack-server.mjs.
NATIVE_PLATFORMS = ("linux-x64", "darwin-arm64", "darwin-x64")


def run(*args):
    return subprocess.check_output(args, text=True).strip()


def git(*args):
    return run("git", *args)


def version():
    with open("Cargo.toml", "rb") as source:
        value = tomllib.load(source)["workspace"]["package"]["version"]
    if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", value):
        raise ValueError("workspace.package.version must be a stable X.Y.Z version")
    return value


def has_tag(tag):
    return subprocess.run(["git", "show-ref", "--verify", "--quiet", f"refs/tags/{tag}"]).returncode == 0


def identity(tag):
    match = CANARY.fullmatch(tag)
    if not match:
        raise ValueError("Expected vX.Y.Z-canary-<7-char-sha>")
    base = ".".join(match.group(1, 2, 3))
    commit = git("rev-parse", f"refs/tags/{tag}^{{commit}}")
    if commit[:7] != match[4]:
        raise ValueError("Canary tag suffix does not match the tagged commit")
    tree = tomllib.loads(git("show", f"{commit}:Cargo.toml"))
    if tree["workspace"]["package"]["version"] != base:
        raise ValueError("Canary tag does not match the workspace version at its commit")
    return base, commit


def output(**values):
    if path := os.environ.get("GITHUB_OUTPUT"):
        with open(path, "a") as target:
            for key, value in values.items():
                target.write(f"{key}={value}\n")
    print(json.dumps(values, sort_keys=True))


def tag_canary():
    base = version()
    tag = f"v{base}-canary-{git('rev-parse', 'HEAD')[:7]}"
    if has_tag(f"v{base}"):
        print(f"REFUSED: v{base} is already promoted; bump the workspace version")
        return
    if has_tag(tag):
        print(f"REFUSED: canary tag {tag} already exists; nothing to do")
        return
    git("tag", "-a", tag, "-m", f"zega canary {tag}")
    git("push", "origin", f"refs/tags/{tag}")
    # GITHUB_TOKEN tag pushes do not start workflows. Explicit dispatch does.
    run("gh", "workflow", "run", "release.yml", "--repo", os.environ["GITHUB_REPOSITORY"], "--ref", tag)
    output(tag=tag)


def resolve(tag):
    base, commit = identity(tag)
    if git("rev-parse", "HEAD") != commit:
        raise ValueError("Release checkout must be the canary commit")
    if has_tag(f"v{base}"):
        raise ValueError(f"v{base} is already promoted")
    output(tag=tag, version=tag[1:], base_version=base, commit=commit,
           npm_version=f"{base}-canary.{commit[:7]}")


def read_json(path):
    return json.loads(Path(path).read_text())


def write_json(path, data):
    Path(path).write_text(json.dumps(data, indent=2) + "\n")


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def repack(source, destination, expected, target):
    """Change only package/package.json's version; preserve every other file byte."""
    with tarfile.open(source, "r:gz") as before, tarfile.open(destination, "w:gz") as after:
        seen = set()
        for member in before.getmembers():
            if not member.isfile() or not member.name.startswith("package/") or ".." in Path(member.name).parts:
                raise ValueError("npm archive contains an unexpected path or non-file entry")
            if member.name in seen:
                raise ValueError("npm archive contains duplicate entries")
            seen.add(member.name)
            payload = before.extractfile(member).read()
            if member.name == "package/package.json":
                data = json.loads(payload)
                if data["name"] != "@zegadb/lib" or data["version"] != expected:
                    raise ValueError("npm archive identity does not match this channel")
                data["version"] = target
                payload = (json.dumps(data, indent=2) + "\n").encode()
            member = copy.copy(member)
            member.size = len(payload)
            after.addfile(member, io.BytesIO(payload))
        if "package/package.json" not in seen:
            raise ValueError("npm archive has no package.json")


def native_artifact(platform):
    if platform not in NATIVE_PLATFORMS:
        raise ValueError(f"Unsupported native platform: {platform}")
    return f"zega-server-{platform}" + (".exe" if platform == "windows-x64" else "")


def cli_artifact(platform):
    """The `zega` CLI executable (cli/, Bun-compiled) the release ships beside zega-server."""
    if platform not in NATIVE_PLATFORMS:
        raise ValueError(f"Unsupported native platform: {platform}")
    return f"zega-{platform}" + (".exe" if platform == "windows-x64" else "")


def artifact_name(platform):
    print(native_artifact(platform))


def cli_artifact_name(platform):
    print(cli_artifact(platform))


def prepare(tag, artifacts, destination):
    base, commit = identity(tag)
    source, dest = Path(artifacts), Path(destination)
    dest.mkdir(parents=True, exist_ok=False)
    for platform in NATIVE_PLATFORMS:
        name = native_artifact(platform)
        shutil.copyfile(source / f"native-{platform}" / name, dest / name)
        cli = cli_artifact(platform)
        shutil.copyfile(source / f"cli-{platform}" / cli, dest / cli)
    shutil.copytree(source / "npm-package" / "wasm", dest / "wasm")
    shutil.copyfile(source / "npm-package" / "package.tgz", dest / "package.tgz")
    # Check the package identity before making this release promotable.
    with tarfile.open(dest / "package.tgz") as archive:
        package = json.load(archive.extractfile("package/package.json"))
        for path in (dest / "wasm").iterdir():
            if archive.extractfile(f"package/wasm/{path.name}").read() != path.read_bytes():
                raise ValueError("R2 WASM must match the tested npm package bytes")
    if package["name"] != "@zegadb/lib" or package["version"] != f"{base}-canary.{commit[:7]}":
        raise ValueError("Release package must be the tested @zegadb/lib canary")
    # A Linux executable that only starts on the host that built it (a NixOS
    # /nix/store loader: v0.2.4) must never get a manifest.
    for line in linux_binary.check_release(dest):
        print(line)
    data = dict(version=tag[1:], tag=tag, base_version=base, commit=commit,
                channel="canary", promoted_from=None,
                cargo_lock_sha256=digest("Cargo.lock"),
                artifacts={p.relative_to(dest).as_posix(): digest(p) for p in sorted(dest.rglob("*")) if p.is_file()})
    for name in METADATA:
        write_json(dest / name, data)


# Whole-command retries on top of the CLI's own per-request ones. Every call here
# is idempotent (cp/sync of the same bytes, rm of a key), so repeating it is safe.
AWS_ATTEMPTS = 3
AWS_RETRY_DELAYS = (2, 8)
# Only these shapes are ever echoed from aws CLI stderr. Raw stderr can carry the
# endpoint (account id) and request details, so it is never printed.
S3_URI = re.compile(r"s3://[a-z0-9][a-z0-9.-]{1,62}/[A-Za-z0-9._/+-]*")
AWS_ERROR = re.compile(r"An error occurred \(([A-Za-z0-9]{1,64})\) when calling the ([A-Za-z0-9]{1,64}) operation")
# Client-side failures have no S3 error code; their messages embed the endpoint URL.
AWS_CLIENT_ERRORS = {"Could not connect to the endpoint URL": "EndpointConnectionError",
                     "Connect timeout on endpoint URL": "ConnectTimeoutError",
                     "Read timeout on endpoint URL": "ReadTimeoutError",
                     "SSL validation failed": "SSLError"}


def aws_diagnostics(stderr):
    """The safe part of a failed aws CLI run: the object URIs that failed and the S3 error codes."""
    text = stderr.decode(errors="replace") if isinstance(stderr, bytes) else stderr
    lines = []
    for line in text.splitlines():
        client = next((name for phrase, name in AWS_CLIENT_ERRORS.items() if phrase in line), None)
        if " failed: " not in line and "An error occurred" not in line and not client:
            continue
        # "upload failed: <local> to s3://b/k", "download failed: s3://b/k to <local>",
        # "delete failed: s3://b/k": the s3:// side names the object.
        uris = [uri.removeprefix("s3://") for uri in S3_URI.findall(line)]
        error = AWS_ERROR.search(line)
        detail = f"{error[1]} on {error[2]}" if error else client or "unrecognised error"
        entry = f"{uris[0]}: {detail}" if uris else detail
        if entry not in lines:
            lines.append(entry)
    return lines


def run_aws(command, failure):
    """Run an aws CLI command with retries; print and raise only the safe diagnostics."""
    for attempt in range(1, AWS_ATTEMPTS + 1):
        result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if not result.returncode:
            return result.stdout.decode()
        details = aws_diagnostics(result.stderr) or ["no recognisable error line"]
        summary = f"{failure} (exit {result.returncode}, attempt {attempt}/{AWS_ATTEMPTS}; raw output withheld)"
        print(summary + "".join(f"\n  {line}" for line in details), file=sys.stderr, flush=True)
        if attempt < AWS_ATTEMPTS:
            time.sleep(AWS_RETRY_DELAYS[attempt - 1])
    raise RuntimeError(f"{summary}: " + "; ".join(details))


def aws(*args):
    paths = [arg for arg in args[1:] if arg.startswith("s3://")]
    key_prefix = paths[-1].removeprefix("s3://") if paths else "unknown prefix"
    run_aws(["aws", "s3", *args, "--endpoint-url", os.environ["R2_ENDPOINT"], "--only-show-errors"],
            f"R2 {args[0]} failed for {key_prefix}")


def aws_list_keys(bucket, prefix):
    """Every key under prefix/. Pages are followed here, not left to CLI auto-pagination."""
    keys, token, seen = [], None, set()
    while True:
        command = ["aws", "s3api", "list-objects-v2", "--bucket", bucket, "--prefix", f"{prefix}/",
                   "--no-paginate", "--output", "json", "--endpoint-url", os.environ["R2_ENDPOINT"]]
        if token is not None:
            command += ["--continuation-token", token]
        page = json.loads(run_aws(command, f"R2 list-objects-v2 failed for {bucket}/{prefix}/"))
        contents = page.get("Contents", []) if isinstance(page, dict) else None
        if not isinstance(contents, list) or any(not isinstance(item, dict) or not isinstance(item.get("Key"), str)
                                                 for item in contents):
            raise ValueError(f"R2 list-objects-v2 returned invalid keys for {bucket}/{prefix}/")
        keys.extend(item["Key"] for item in contents)
        if not page.get("IsTruncated"):
            return keys
        token = page.get("NextContinuationToken")
        if not isinstance(token, str) or not token or token in seen:
            raise ValueError(f"R2 list-objects-v2 returned an invalid continuation token for {bucket}/{prefix}/")
        seen.add(token)


def publish_pointer(directory, bucket, prefix):
    """Replace a mutable pointer without exposing an empty prefix, then return its readback.

    The payload goes first and release.json/manifest.json last, so a failed upload never
    leaves the pointer's metadata naming the new release over old payload bytes.
    """
    directory = Path(directory)
    local_keys = {f"{prefix}/{path.relative_to(directory).as_posix()}"
                  for path in directory.rglob("*") if path.is_file()}
    excludes = [arg for name in METADATA for arg in ("--exclude", name)]
    aws("cp", "--recursive", f"{directory}/", f"s3://{bucket}/{prefix}/", *excludes)
    for name in METADATA:
        aws("cp", f"{directory}/{name}", f"s3://{bucket}/{prefix}/{name}")
    for key in aws_list_keys(bucket, prefix):
        if key.startswith(f"{prefix}/") and key not in local_keys:
            aws("rm", f"s3://{bucket}/{key}")
    readback = Path(".tmp") / f"readback-{bucket}-{prefix}"
    readback.mkdir(parents=True, exist_ok=False)
    aws("cp", "--recursive", f"s3://{bucket}/{prefix}/", f"{readback}/")
    return readback


def pointer_tag(bucket, key, work):
    """The tag a published pointer object names, or why it could not be read."""
    target = work / key.replace("/", "-")
    try:
        aws("cp", f"s3://{bucket}/{key}", str(target))
        return read_json(target)["tag"]
    except Exception as error:  # Reporting only: the original failure is re-raised.
        return f"unreadable ({type(error).__name__})"


def report_channel_state(buckets, channel, tag):
    """After a failed advance, say plainly where every bucket's two pointers stand."""
    work = Path(".tmp") / f"state-{channel}"
    work.mkdir(parents=True, exist_ok=True)
    print(f"::error title=Channel {channel} not advanced::Publishing {tag} to {channel} failed; "
          "pointer state follows", file=sys.stderr)
    for bucket in buckets:
        prefix = pointer_tag(bucket, f"{channel}/release.json", work)
        root = pointer_tag(bucket, f"{channel}.json", work)
        state = "consistent" if prefix == root else "MISMATCH"
        print(f"::error title={bucket} {channel} {state}::{bucket}: {channel}/release.json = {prefix}, "
              f"{channel}.json = {root}", file=sys.stderr)
        if prefix != tag:
            print(f"  {bucket}/{channel}/ payload objects may mix {tag} with the previous release; "
                  f"its release.json still names {prefix}. The next successful publish replaces them.",
                  file=sys.stderr)
    sys.stderr.flush()


def advance_channel(sources, channel, tag, check):
    """Move <channel>/ and then <channel>.json to `tag` in every bucket in `sources`.

    No root <channel>.json is written until every bucket's <channel>/ prefix has been read back
    and checked, so the root JSON never runs ahead of a prefix. On any failure the run prints
    each bucket's two pointer values before failing.
    """
    try:
        for bucket, source in sources.items():
            check(bucket, publish_pointer(source, bucket, channel))
        for bucket, source in sources.items():
            aws("cp", f"{source}/release.json", f"s3://{bucket}/{channel}.json")
    except Exception:
        report_channel_state(sources, channel, tag)
        raise


def compare_directories(expected, actual, message):
    expected = Path(expected)
    actual = Path(actual)
    expected_files = {path.relative_to(expected).as_posix(): path.read_bytes()
                      for path in expected.rglob("*") if path.is_file()}
    actual_files = {path.relative_to(actual).as_posix(): path.read_bytes()
                    for path in actual.rglob("*") if path.is_file()}
    if actual_files != expected_files:
        raise ValueError(message)


def check_commit(directory, tag, commit):
    release = read_json(Path(directory) / "release.json")
    if release["commit"] != commit:
        raise ValueError(f"release.json commit mismatch: tag {tag} points at {commit}, artifacts record {release['commit']}")
    return release


def verify(directory, tag, commit):
    directory = Path(directory)
    # This check MUST precede any object copy, tag creation or pointer mutation.
    release = check_commit(directory, tag, commit)
    for name in METADATA:
        data = read_json(directory / name)
        if data["tag"] != tag or data["commit"] != commit or data["channel"] != "canary":
            raise ValueError(f"{name} is not the selected canary")
        if data["artifacts"] != release["artifacts"]:
            raise ValueError("Metadata checksum inventories disagree")
    files = {p.relative_to(directory).as_posix() for p in directory.rglob("*") if p.is_file()} - set(METADATA)
    if not files or files != set(release["artifacts"]):
        raise ValueError("Canary payload inventory is incomplete or has unexpected objects")
    for name, expected in release["artifacts"].items():
        if digest(directory / name) != expected:
            raise ValueError(f"Canary checksum mismatch: {name}")


def publish_r2(tag, directory):
    base, commit = identity(tag)
    if has_tag(f"v{base}"):
        raise ValueError(f"v{base} is already promoted")
    verify(directory, tag, commit)
    # Serialized with promotion. Never overwrite an existing immutable prefix.
    for bucket in BUCKETS:
        existing = run_aws(["aws", "s3api", "list-objects-v2", "--bucket", bucket, "--prefix", f"{tag}/",
                            "--max-keys", "1", "--query", "KeyCount", "--output", "text", "--endpoint-url", os.environ["R2_ENDPOINT"]],
                           f"R2 list-objects-v2 failed for {bucket}/{tag}/")
        if existing.strip() != "0":
            raise ValueError(f"Canary prefix already exists in {bucket}; refusing overwrite")
    for bucket in BUCKETS:
        aws("cp", "--recursive", f"{directory}/", f"s3://{bucket}/{tag}/")
    # Read back before advancing canary. latest is only touched by promote.
    for bucket in BUCKETS:
        check = Path(".tmp/readback") / bucket
        check.mkdir(parents=True, exist_ok=False)
        aws("cp", "--recursive", f"s3://{bucket}/{tag}/", f"{check}/")
        verify(check, tag, commit)
    advance_channel({bucket: Path(directory) for bucket in BUCKETS}, "canary", tag,
                    lambda bucket, readback: verify(readback, tag, commit))


def promote(tag, directory):
    base, commit = identity(tag)
    if has_tag(f"v{base}"):
        raise ValueError(f"v{base} is already promoted")
    work = Path(directory)
    work.mkdir(parents=True, exist_ok=False)
    for bucket in BUCKETS:
        source = work / bucket
        source.mkdir()
        # Fetch and compare release.json FIRST, before downloading any payload.
        aws("cp", f"s3://{bucket}/{tag}/release.json", str(source / "release.json"))
        check_commit(source, tag, commit)
        aws("cp", "--recursive", f"s3://{bucket}/{tag}/", f"{source}/")
        verify(source, tag, commit)
        # Canaries published before prepare checked this (v0.2.4-canary-9c02df2,
        # v0.2.5-canary-5804251) must not become stable.
        linux_binary.check_release(source)
    repack(work / BUCKETS[0] / "package.tgz", work / "package.tgz", f"{base}-canary.{commit[:7]}", base)
    for bucket in BUCKETS:
        source = work / bucket
        for name in METADATA:
            data = read_json(source / name)
            data.update(version=base, tag=f"v{base}", channel="stable", promoted_from=tag)
            write_json(source / name, data)
        aws("sync", f"{source}/", f"s3://{bucket}/v{base}/", "--delete")
        readback = work / f"{bucket}-readback"
        readback.mkdir()
        aws("cp", "--recursive", f"s3://{bucket}/v{base}/", f"{readback}/")
        expected = {p.relative_to(source): p.read_bytes() for p in source.rglob("*") if p.is_file()}
        actual = {p.relative_to(readback): p.read_bytes() for p in readback.rglob("*") if p.is_file()}
        if actual != expected:
            raise ValueError("Stable readback differs from the canary payload and rewritten metadata")
    git("tag", "-a", f"v{base}", commit, "-m", f"zega v{base} (promoted from {tag})")
    git("push", "origin", f"refs/tags/v{base}")
    advance_channel({bucket: work / bucket for bucket in BUCKETS}, "latest", f"v{base}",
                    lambda bucket, readback: compare_directories(
                        work / bucket, readback, "Latest readback differs from the promoted release payload"))
    output(tag=f"v{base}", version=base, commit=commit)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["tag-canary", "resolve", "prepare", "publish-r2", "promote", "repack", "artifact-name", "cli-artifact-name"])
    parser.add_argument("args", nargs="*")
    args = parser.parse_args()
    commands = {"tag-canary": tag_canary, "resolve": resolve, "prepare": prepare,
                "publish-r2": publish_r2, "promote": promote, "repack": repack, "artifact-name": artifact_name,
                "cli-artifact-name": cli_artifact_name}
    try:
        commands[args.command](*args.args)
    except (ValueError, RuntimeError, KeyError, OSError, subprocess.CalledProcessError) as error:
        # CalledProcessError may include commands with endpoint values. Hide it.
        message = "External command failed" if isinstance(error, subprocess.CalledProcessError) else str(error)
        parser.exit(1, f"ERROR: {message}\n")


if __name__ == "__main__":
    main()
