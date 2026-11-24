#!/usr/bin/env python3
"""Resolve Alpine packages and their dependencies from APKINDEX files.

Usage:
    apkindex.py [--installed DB] [--no-deps] APKINDEX [APKINDEX...] -- PKG [PKG...]

Prints one line per package to fetch, `<repo-index-number> <name> <version>`,
in dependency order. Packages listed in the minirootfs' installed database
(`lib/apk/db/installed`, same record format) are treated as present and not
printed. Dependency tokens understood: `name`, `name>=ver` (any constraint,
the newest version is used), `so:libfoo.so.1`, `cmd:foo`, `pc:foo`, and
`!conflict` (ignored).
"""

import re
import sys


def records(path):
    """Yield dicts of APKINDEX/installed-db records (single-letter keys)."""
    rec = {}
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            line = line.rstrip("\n")
            if not line:
                if rec:
                    yield rec
                rec = {}
                continue
            if len(line) > 2 and line[1] == ":":
                rec.setdefault(line[0], line[2:])
    if rec:
        yield rec


def strip_constraint(tok):
    return re.split(r"[<>=~]", tok, maxsplit=1)[0]


def main(argv):
    installed_db = None
    no_deps = False
    args = list(argv)
    while args and args[0].startswith("--"):
        opt = args.pop(0)
        if opt == "--installed":
            installed_db = args.pop(0)
        elif opt == "--no-deps":
            no_deps = True
        elif opt == "--":
            break
        else:
            sys.exit(f"unknown option {opt}")
    if "--" not in args:
        sys.exit(__doc__)
    split = args.index("--")
    indexes, wanted = args[:split], args[split + 1:]

    pkgs = {}  # name -> (repo, version, deps)
    provides = {}  # provided token -> name
    for repo, path in enumerate(indexes):
        for r in records(path):
            name = r.get("P")
            if not name or name in pkgs:
                continue  # earlier index wins (main before community)
            deps = r.get("D", "").split()
            pkgs[name] = (repo, r.get("V", ""), deps)
            for p in r.get("p", "").split():
                provides.setdefault(strip_constraint(p), name)

    present = set()
    if installed_db:
        for r in records(installed_db):
            if r.get("P"):
                present.add(r["P"])
                for p in r.get("p", "").split():
                    present.add(strip_constraint(p))

    order, seen = [], set()

    def resolve_token(tok):
        tok = strip_constraint(tok)
        if tok in present:
            return None
        if tok in pkgs:
            return tok
        name = provides.get(tok)
        if name is None:
            sys.exit(f"apkindex: nothing provides '{tok}'")
        return None if name in present else name

    def visit(name, chain):
        if name in seen or name in present:
            return
        if name not in pkgs:
            sys.exit(f"apkindex: unknown package '{name}'")
        seen.add(name)
        if not no_deps:
            for dep in pkgs[name][2]:
                if dep.startswith("!"):
                    continue
                target = resolve_token(dep)
                if target and target not in chain:
                    visit(target, chain | {name})
        order.append(name)

    for w in wanted:
        target = resolve_token(w) if not no_deps else w
        if target:
            visit(target, frozenset())

    for name in order:
        repo, version, _ = pkgs[name]
        print(repo, name, version)


if __name__ == "__main__":
    main(sys.argv[1:])
