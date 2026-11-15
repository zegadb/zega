#!/usr/bin/env python3
"""Linux release executables must run on any ordinary Linux, not only the host that built them.

A Linux executable names its loader (ELF PT_INTERP). A build on a NixOS host
gets /nix/store/<hash>-glibc-<version>/lib/ld-linux-x86-64.so.2 there, and on
every machine without that exact store path (Ubuntu, Debian, Fedora, Docker
images) the kernel cannot find it and exec fails with ENOENT. v0.2.4 and
v0.2.5-canary-5804251 shipped zega-server-linux-x64 that way.

`check` reads the ELF itself (no readelf needed, so it also runs on macOS and
inside channels.py) and refuses an executable whose loader is not the
standard one for its architecture, that has an RPATH/RUNPATH, that names
/nix/store anywhere in its dynamic section, or that needs a glibc newer than
the oldest distribution the release runs on. `run` executes a binary inside a
plain distribution image (no nix), copying it in rather than bind-mounting it,
so it behaves the same under Colima on a Mac as on a Linux runner.
"""

import argparse
from pathlib import Path
import re
import struct
import subprocess
import sys


ELF_MAGIC = b"\x7fELF"
# e_machine -> the loader every glibc distribution has at this path.
LOADERS = {62: ("x86-64", "/lib64/ld-linux-x86-64.so.2"), 183: ("aarch64", "/lib/ld-linux-aarch64.so.1")}
# Ubuntu 22.04 LTS (glibc 2.35), the oldest distribution the release gate runs
# the binaries on (OLDEST_IMAGE). Debian 12 (2.36) builds them.
MAX_GLIBC = (2, 35)
OLDEST_IMAGE = "ubuntu:22.04"
PT_LOAD, PT_DYNAMIC, PT_INTERP = 1, 2, 3
DT_NEEDED, DT_STRTAB, DT_STRSZ, DT_RPATH, DT_RUNPATH = 1, 5, 10, 15, 29
GLIBC = re.compile(rb"GLIBC_([0-9]+)\.([0-9]+)(?:\.([0-9]+))?")


class NotPortable(ValueError):
    pass


def is_elf(path):
    with open(path, "rb") as file:
        return file.read(4) == ELF_MAGIC


def inspect(path):
    """Return {machine, interpreter, needed, rpath, glibc, nix} for a 64-bit little-endian ELF."""
    data = Path(path).read_bytes()
    if data[:4] != ELF_MAGIC:
        raise NotPortable(f"{path}: not an ELF executable")
    if data[4] != 2 or data[5] != 1:
        raise NotPortable(f"{path}: only 64-bit little-endian ELF is shipped")
    (machine,) = struct.unpack_from("<H", data, 0x12)
    if machine not in LOADERS:
        raise NotPortable(f"{path}: unsupported ELF machine {machine}")
    (phoff,) = struct.unpack_from("<Q", data, 0x20)
    phentsize, phnum = struct.unpack_from("<HH", data, 0x36)
    interpreter, dynamic, loads = None, None, []
    for index in range(phnum):
        kind, _flags, offset, vaddr, _paddr, filesz = struct.unpack_from("<IIQQQQ", data, phoff + index * phentsize)
        if kind == PT_INTERP:
            interpreter = data[offset:offset + filesz].split(b"\0", 1)[0].decode()
        elif kind == PT_DYNAMIC:
            dynamic = (offset, filesz)
        elif kind == PT_LOAD:
            loads.append((vaddr, offset, filesz))

    def file_offset(address):
        for vaddr, offset, size in loads:
            if vaddr <= address < vaddr + size:
                return offset + address - vaddr
        raise NotPortable(f"{path}: dynamic string table outside every loaded segment")

    entries = []
    if dynamic:
        offset, size = dynamic
        for at in range(offset, offset + size - 15, 16):
            tag, value = struct.unpack_from("<qQ", data, at)
            if tag == 0:
                break
            entries.append((tag, value))
    values = dict(entries)
    strings = b""
    if DT_STRTAB in values:
        start = file_offset(values[DT_STRTAB])
        strings = data[start:start + values.get(DT_STRSZ, 0)]

    def string(at):
        return strings[at:strings.index(b"\0", at)].decode(errors="replace")

    needed = [string(value) for tag, value in entries if tag == DT_NEEDED]
    rpath = [string(value) for tag, value in entries if tag in (DT_RPATH, DT_RUNPATH)]
    glibc = max((tuple(int(part) for part in match.groups() if part) for match in GLIBC.finditer(strings)), default=None)
    return dict(machine=machine, interpreter=interpreter, needed=needed, rpath=rpath, glibc=glibc, nix=b"/nix/store" in strings)


def check(path):
    """Raise NotPortable unless `path` runs on any glibc Linux from OLDEST_IMAGE on; return a summary line."""
    found = inspect(path)
    arch, loader = LOADERS[found["machine"]]
    problems = []
    if found["interpreter"] is not None and found["interpreter"] != loader:
        problems.append(f"loader is {found['interpreter']}, not {loader}")
    if found["rpath"]:
        problems.append(f"has RPATH/RUNPATH {':'.join(found['rpath'])}")
    if found["nix"]:
        problems.append("names /nix/store in its dynamic strings")
    if found["glibc"] and found["glibc"][:2] > MAX_GLIBC:
        problems.append(f"needs GLIBC_{'.'.join(map(str, found['glibc']))}, newer than {OLDEST_IMAGE}'s {'.'.join(map(str, MAX_GLIBC))}")
    if problems:
        raise NotPortable(f"{path}: not portable: " + "; ".join(problems))
    glibc = "GLIBC_" + ".".join(map(str, found["glibc"])) if found["glibc"] else "no glibc symbols"
    return f"{Path(path).name}: {arch}, loader {found['interpreter'] or '(static)'}, no RPATH/RUNPATH, newest {glibc}"


def check_release(directory):
    """Every Linux executable in a release directory: each *-linux-* file must be a portable ELF."""
    lines = []
    for path in sorted(Path(directory).rglob("*")):
        if path.is_file() and ("-linux-" in path.name or is_elf(path)):
            lines.append(check(path))
    return lines


def run(image, binary, *args, ports=(), files=(), detach=False):
    """Run `binary` (copied to /usr/local/bin/<name>) with `args` in a fresh `image` container.

    `files` are (local path, container path) pairs copied in beside it. Returns
    the container id and, unless `detach`, waits and returns the exit code.
    """
    name = Path(binary).name
    publish = [flag for port in ports for flag in ("-p", f"127.0.0.1:{port}:{port}")]
    container = subprocess.check_output(["docker", "create", "--init", *publish, image, f"/usr/local/bin/{name}", *args], text=True).strip()
    try:
        # docker cp keeps the file mode: the binary must already be executable.
        for local, inside in ((binary, f"/usr/local/bin/{name}"), *files):
            subprocess.check_call(["docker", "cp", str(local), f"{container}:{inside}"], stdout=subprocess.DEVNULL)
    except BaseException:
        remove(container)
        raise
    if detach:
        return container, subprocess.Popen(["docker", "start", "-a", container])
    try:
        return container, subprocess.call(["docker", "start", "-a", container])
    finally:
        remove(container)


def remove(container):
    subprocess.run(["docker", "rm", "-f", container], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    checking = commands.add_parser("check", help="refuse any non-portable Linux executable (files or release directories)")
    checking.add_argument("paths", nargs="+")
    running = commands.add_parser("run", help=f"run a binary in a plain distribution image (default {OLDEST_IMAGE})")
    running.add_argument("--image", default=OLDEST_IMAGE)
    running.add_argument("binary")
    running.add_argument("args", nargs=argparse.REMAINDER)
    options = parser.parse_args()
    if options.command == "check":
        try:
            for path in options.paths:
                for line in (check_release(path) if Path(path).is_dir() else [check(path)]):
                    print(line)
        except NotPortable as error:
            print(f"::error::{error}", file=sys.stderr)
            raise SystemExit(1)
    else:
        _, code = run(options.image, options.binary, *options.args)
        print(f"{Path(options.binary).name} {' '.join(options.args)} in {options.image}: exit {code}")
        raise SystemExit(code)


if __name__ == "__main__":
    main()
