#!/usr/bin/env python3
"""Write a reproducible newc cpio archive (the initramfs format).

Usage: mkcpio.py ROOT OUT [--node PATH:MODE:MAJOR:MINOR ...]

Entries are sorted, owned by root, timestamped with $SOURCE_DATE_EPOCH (or
0), and inode numbers are assigned sequentially, so the same tree always
gives the same bytes. Hard links are stored as independent files. `--node`
adds character device nodes (e.g. `dev/console:0600:5:1`) that an
unprivileged build cannot mknod; the kernel needs /dev/console before /init
runs. No root and no cpio binary required.
"""

import os
import stat
import sys

MTIME = int(os.environ.get("SOURCE_DATE_EPOCH", "0"))


def pad(n):
    return b"\0" * ((4 - n % 4) % 4)


def entry(out, ino, name, mode, data=b"", rdev=(0, 0), nlink=1):
    nb = name.encode() + b"\0"
    fields = [ino, mode, 0, 0, nlink, MTIME, len(data), 0, 0, rdev[0], rdev[1], len(nb), 0]
    hdr = b"070701" + b"".join(b"%08X" % f for f in fields)
    out.write(hdr + nb + pad(len(hdr) + len(nb)))
    out.write(data + pad(len(data)))


def main(root, out_path, nodes):
    paths = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames.sort()
        for n in dirnames + filenames:
            paths.append(os.path.join(dirpath, n))
    rel = sorted(os.path.relpath(p, root) for p in paths)
    node_names = {n[0] for n in nodes}
    ino = 1
    with open(out_path, "wb") as out:
        for r in rel:
            if r in node_names:
                continue
            p = os.path.join(root, r)
            st = os.lstat(p)
            perm = stat.S_IMODE(st.st_mode)
            if stat.S_ISLNK(st.st_mode):
                entry(out, ino, r, stat.S_IFLNK | 0o777, os.readlink(p).encode())
            elif stat.S_ISDIR(st.st_mode):
                entry(out, ino, r, stat.S_IFDIR | perm, nlink=2)
            elif stat.S_ISREG(st.st_mode):
                with open(p, "rb") as f:
                    entry(out, ino, r, stat.S_IFREG | perm, f.read())
            else:
                print(f"mkcpio: skipping special file {r}", file=sys.stderr)
                continue
            ino += 1
        for name, mode, major, minor in nodes:
            entry(out, ino, name, stat.S_IFCHR | mode, rdev=(major, minor))
            ino += 1
        entry(out, 0, "TRAILER!!!", 0, nlink=1)


def parse_node(spec):
    name, mode, major, minor = spec.split(":")
    return (name.lstrip("/"), int(mode, 8), int(major), int(minor))


if __name__ == "__main__":
    a = sys.argv[1:]
    if len(a) < 2:
        sys.exit(__doc__)
    root, out, rest = a[0], a[1], a[2:]
    nodes = []
    while rest:
        if rest[0] != "--node" or len(rest) < 2:
            sys.exit(__doc__)
        nodes.append(parse_node(rest[1]))
        rest = rest[2:]
    main(root, out, nodes)
