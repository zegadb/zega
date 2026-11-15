"""The portability gate on Linux release executables (scripts/linux_binary.py).

The fixtures are real ELF headers, program headers and dynamic sections, built
here so the proof needs no Linux toolchain; the nix loader is the exact one
v0.2.4's zega-server-linux-x64 shipped with.
"""

from pathlib import Path
import struct
import tempfile
import unittest

from scripts import linux_binary

NIX_LOADER = "/nix/store/fsd4vbp9zjd9iaicq2fb9lpc1h5j973y-glibc-2.42-100/lib/ld-linux-x86-64.so.2"


def elf(interpreter="/lib64/ld-linux-x86-64.so.2", machine=62, needed=("libc.so.6", "libgcc_s.so.1"),
        runpath=None, versions=("GLIBC_2.2.5", "GLIBC_2.34")):
    """A minimal 64-bit little-endian ELF executable; every address equals its file offset."""
    strings = bytearray(b"\0")

    def add(text):
        at = len(strings)
        strings.extend(text.encode() + b"\0")
        return at

    dynamic = [(linux_binary.DT_NEEDED, add(name)) for name in needed]
    if runpath:
        dynamic.append((linux_binary.DT_RUNPATH, add(runpath)))
    for version in versions:
        add(version)
    headers = 64 + 56 * 3
    interp = (interpreter.encode() + b"\0") if interpreter else b""
    strtab = headers + len(interp)
    dynamic += [(linux_binary.DT_STRTAB, strtab), (linux_binary.DT_STRSZ, len(strings)), (0, 0)]
    table = b"".join(struct.pack("<qQ", tag, value) for tag, value in dynamic)
    dyn = strtab + len(strings)
    size = dyn + len(table)
    segments = [(linux_binary.PT_LOAD, 0, size)]
    if interpreter:
        segments.append((linux_binary.PT_INTERP, headers, len(interp)))
    segments.append((linux_binary.PT_DYNAMIC, dyn, len(table)))
    ident = b"\x7fELF" + bytes([2, 1, 1]) + bytes(9)
    header = ident + struct.pack("<HHIQQQIHHHHHH", 3, machine, 1, 0, 64, 0, 0, 64, 56, len(segments), 64, 0, 0)
    program = b"".join(struct.pack("<IIQQQQQQ", kind, 4, offset, offset, offset, length, length, 8)
                       for kind, offset, length in segments)
    program += bytes(56 * 3 - len(program))
    return header + program + interp + bytes(strings) + table


class Check(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.dir = Path(self.temp.name)

    def write(self, name, payload):
        path = self.dir / name
        path.write_bytes(payload)
        return path

    def refused(self, payload, reason):
        with self.assertRaisesRegex(linux_binary.NotPortable, reason):
            linux_binary.check(self.write("zega-server-linux-x64", payload))

    def test_standard_loader_passes_and_reports_what_it_read(self):
        line = linux_binary.check(self.write("zega-server-linux-x64", elf()))
        self.assertEqual(line, "zega-server-linux-x64: x86-64, loader /lib64/ld-linux-x86-64.so.2, no RPATH/RUNPATH, newest GLIBC_2.34")

    def test_nix_loader_is_refused(self):
        self.refused(elf(interpreter=NIX_LOADER), f"loader is {NIX_LOADER}, not /lib64/ld-linux-x86-64.so.2")

    def test_runpath_and_nix_libraries_are_refused(self):
        self.refused(elf(runpath="/nix/store/abc-gcc-14/lib"), "has RPATH/RUNPATH /nix/store/abc-gcc-14/lib")
        self.refused(elf(needed=("/nix/store/abc-glibc/lib/libc.so.6",)), "names /nix/store")

    def test_glibc_newer_than_the_oldest_supported_distribution_is_refused(self):
        linux_binary.check(self.write("ok", elf(versions=("GLIBC_2.35",))))
        self.refused(elf(versions=("GLIBC_2.2.5", "GLIBC_2.38")), r"needs GLIBC_2\.38, newer than ubuntu:22\.04's 2\.35")

    def test_arm64_has_its_own_standard_loader(self):
        linux_binary.check(self.write("zega-server-linux-arm64", elf(interpreter="/lib/ld-linux-aarch64.so.1", machine=183)))
        self.refused(elf(machine=183), "not /lib/ld-linux-aarch64.so.1")

    def test_static_executable_passes(self):
        self.assertIn("loader (static)", linux_binary.check(self.write("static", elf(interpreter=None, needed=(), versions=()))))

    def test_release_directory_requires_every_linux_artifact_to_be_portable_elf(self):
        self.write("zega-server-darwin-x64", b"\xcf\xfa\xed\xfe mach-o")
        self.write("zega-linux-x64", elf())
        self.write("zega-server-linux-x64", elf())
        self.assertEqual(len(linux_binary.check_release(self.dir)), 2)
        self.write("zega-server-linux-x64", b"not an executable")
        with self.assertRaisesRegex(linux_binary.NotPortable, "not an ELF"):
            linux_binary.check_release(self.dir)
        self.write("zega-server-linux-x64", elf(interpreter=NIX_LOADER))
        with self.assertRaisesRegex(linux_binary.NotPortable, "zega-server-linux-x64: not portable"):
            linux_binary.check_release(self.dir)


if __name__ == "__main__":
    unittest.main()
