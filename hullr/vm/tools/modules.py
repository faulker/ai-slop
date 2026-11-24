#!/usr/bin/env python3
"""List kernel modules with their dependencies in load order.

Usage: modules.py MODDIR NAME [NAME...]

MODDIR is `lib/modules/<version>` from the kernel package. Names use
underscores or dashes interchangeably. Prints module paths relative to MODDIR,
dependencies first. Names that are not modules are assumed to be built into
the kernel and reported on stderr.
"""

import os
import sys


def base(path):
    name = os.path.basename(path)
    for ext in (".gz", ".xz", ".zst"):
        if name.endswith(ext):
            name = name[: -len(ext)]
    return name[:-3].replace("-", "_") if name.endswith(".ko") else None


def main(moddir, names):
    deps, by_name = {}, {}
    with open(os.path.join(moddir, "modules.dep")) as f:
        for line in f:
            if ":" not in line:
                continue
            mod, rest = line.split(":", 1)
            deps[mod] = rest.split()
            by_name[base(mod)] = mod
    order, seen = [], set()

    def visit(mod):
        if mod in seen:
            return
        seen.add(mod)
        for d in deps.get(mod, []):
            visit(d)
        order.append(mod)

    for n in names:
        mod = by_name.get(n.replace("-", "_"))
        if mod is None:
            print(f"modules.py: {n}: not a module (built in?)", file=sys.stderr)
            continue
        visit(mod)
    print("\n".join(order))


if __name__ == "__main__":
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2:])
