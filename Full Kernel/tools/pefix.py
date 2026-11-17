#!/usr/bin/env python3
"""Make NovaBoot loadable by shim (Secure Boot).

lld merges NovaBoot's all-zero statics into `.data` and drops their bytes, leaving a section that is
flagged "initialised data" but has no data in the file (raw size 0, raw pointer 0). Firmware loaders
accept that; shim's stricter loader refuses the image ("Section N is inside image headers. Malformed
section header." then "Verification failed: (0x3) Unsupported").

Such a section is really BSS, so flag it that way (UNINITIALIZED_DATA instead of INITIALIZED_DATA).
Run on the .EFI before it is signed (the signature covers the headers). Safe to run twice.

usage: pefix.py FILE.EFI
"""
import struct
import sys

INITIALIZED_DATA = 0x00000040
UNINITIALIZED_DATA = 0x00000080


def main(path):
    d = bytearray(open(path, "rb").read())
    pe = struct.unpack_from("<I", d, 0x3C)[0]
    if d[pe:pe + 4] != b"PE\0\0":
        sys.exit("pefix: %s is not a PE file" % path)
    nsec, = struct.unpack_from("<H", d, pe + 6)
    optsize, = struct.unpack_from("<H", d, pe + 20)
    first = pe + 24 + optsize
    fixed = 0
    for i in range(nsec):
        off = first + 40 * i
        name = bytes(d[off:off + 8]).rstrip(b"\0").decode("ascii", "replace")
        vsize, _va, rawsize, rawptr = struct.unpack_from("<IIII", d, off + 8)
        flags, = struct.unpack_from("<I", d, off + 36)
        if vsize and rawsize == 0 and rawptr == 0 and flags & INITIALIZED_DATA:
            flags = (flags & ~INITIALIZED_DATA) | UNINITIALIZED_DATA
            struct.pack_into("<I", d, off + 36, flags)
            fixed += 1
            print("pefix: %s: section %d (%s) has no file data: marked as BSS" % (path, i, name))
    if fixed:
        open(path, "wb").write(d)


if len(sys.argv) != 2:
    sys.exit(__doc__)
main(sys.argv[1])
