#!/usr/bin/env python3
"""Turn a distribution kernel into something VZLinuxBootLoader can boot.

Usage: kernel.py ARCH VMLINUZ OUT

On Apple Silicon, Virtualization.framework only boots an uncompressed arm64
`Image` (it checks the arm64 image header magic "ARM\\x64" at offset 56 and
does not run a decompressor). Alpine's arm64 `vmlinuz-virt` is one of:

* a plain gzip of `Image` (older kernels): gunzip it;
* an EFI zboot image (kernels built with CONFIG_EFI_ZBOOT): a small PE/COFF
  stub whose header holds "zimg" at offset 4, the payload offset and size as
  little-endian u32 at offsets 8 and 12, and the compression name at offset
  24; the payload is extracted and decompressed (gzip here, zstd via the
  `zstd` command);
* already a raw `Image`: copied.

x86_64 (Intel Macs) boots the bzImage as is, so it is copied unchanged.
"""

import gzip
import shutil
import struct
import subprocess
import sys

ARM64_MAGIC_OFFSET = 56
ARM64_MAGIC = b"ARM\x64"


def decompress(kind, data):
    if kind == "gzip":
        return gzip.decompress(data)
    if kind == "zstd":
        return subprocess.run(["zstd", "-dc"], input=data, check=True, stdout=subprocess.PIPE).stdout
    sys.exit(f"kernel.py: unsupported zboot compression '{kind}'")


def is_arm64_image(data):
    return data[ARM64_MAGIC_OFFSET:ARM64_MAGIC_OFFSET + 4] == ARM64_MAGIC


def extract_arm64(data):
    if is_arm64_image(data):
        return data
    if data[:2] == b"\x1f\x8b":
        return gzip.decompress(data)
    if data[:2] == b"MZ" and data[4:8] == b"zimg":
        offset, size = struct.unpack_from("<II", data, 8)
        kind = data[24:56].split(b"\0", 1)[0].decode()
        return decompress(kind, data[offset:offset + size])
    sys.exit("kernel.py: unrecognized arm64 kernel format")


def main(arch, src, out):
    if arch == "x86_64":
        shutil.copyfile(src, out)
        return
    with open(src, "rb") as f:
        data = f.read()
    image = extract_arm64(data)
    if not is_arm64_image(image):
        sys.exit("kernel.py: result is not an arm64 Image (missing ARM\\x64 magic)")
    with open(out, "wb") as f:
        f.write(image)


if __name__ == "__main__":
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    main(*sys.argv[1:])
