#!/usr/bin/env python3
"""Compute a file-level dependency graph for the portable core, to ground §3."""
import os, re, json, sys

REF = "/home/m-kikuchi/projects/oprs/reference/opus-1.6.1"

def blank_comments(s):
    out = list(s); i, n = 0, len(s)
    while i < n:
        if s.startswith("/*", i):
            j = s.find("*/", i + 2); j = n if j < 0 else j + 2
            for k in range(i, j):
                if out[k] != "\n": out[k] = " "
            i = j
        elif s.startswith("//", i):
            j = s.find("\n", i); j = n if j < 0 else j
            for k in range(i, j): out[k] = " "
            i = j
        else:
            i += 1
    return "".join(out)

FILES = {}
for root in ("celt", "silk", "src"):
    for dirpath, dirnames, filenames in os.walk(os.path.join(REF, root)):
        rel = os.path.relpath(dirpath, REF)
        if re.search(r"/(x86|arm|mips|tests?)(/|$)", rel):
            continue
        for f in sorted(filenames):
            if f.endswith(".c"):
                p = os.path.join(rel, f)
                if p in ("celt/opus_custom_demo.c", "celt/mini_kfft.c",
                         "src/opus_demo.c", "src/opus_compare.c",
                         "src/repacketizer_demo.c", "src/qext_compare.c"):
                    continue
                FILES[p] = blank_comments(open(os.path.join(REF, p), errors="replace").read())

# --- function definitions per file (name -> file), from .c and headers
DEFN = {}
for p, txt in FILES.items():
    for m in re.finditer(r"(?m)^[A-Za-z_][A-Za-z0-9_ \t\*]*?[ \t\*]([A-Za-z_][A-Za-z0-9_]*)"
                         r"[ \t]*\([^;{]*\)[ \t\n]*\{", txt):
        nm = m.group(1)
        if nm in ("if","for","while","switch","return","sizeof","else","do"):
            continue
        DEFN.setdefault(nm, p)

HDR_FOR = {}
for root in ("celt", "silk"):
    for dirpath, dirnames, filenames in os.walk(os.path.join(REF, root)):
        rel = os.path.relpath(dirpath, REF)
        if re.search(r"/(x86|arm|mips|tests?)(/|$)", rel):
            continue
        for f in sorted(filenames):
            if f.endswith(".h"):
                HDR_FOR.setdefault(os.path.join(rel, f), None)

# --- local (static / all-lowercase-celt) functions get no edge; only cross-file
edges = {}
for p, txt in FILES.items():
    deps = {}
    for nm, owner in DEFN.items():
        if owner == p:
            continue
        c = len(re.findall(r"\b" + re.escape(nm) + r"\s*\(", txt))
        if c:
            deps.setdefault(owner, []).append((nm, c))
    edges[p] = deps

# --- includes
inc = {}
for p in FILES:
    raw = open(os.path.join(REF, p), errors="replace").read()
    h = re.findall(r'^\s*#\s*include\s+"([^"]+)"', raw, re.M)
    h = [x for x in h if x not in ("config.h",)]
    inc[p] = h

# --- emit adjacency
order = sorted(FILES)
lines = ["# mechanical call graph (file -> files whose functions it calls)", ""]
for p in order:
    d = edges[p]
    if not d:
        lines.append("%s -> (none)" % p)
        continue
    parts = []
    for owner in sorted(d):
        fs = ", ".join("%s×%d" % (n, c) for n, c in sorted(d[owner], key=lambda x: -x[1])[:8])
        parts.append("%s [%s]" % (owner, fs))
    lines.append("%s -> %s" % (p, "; ".join(parts)))

lines.append("")
lines.append("# include graph")
for p in order:
    lines.append("%s -> %s" % (p, ", ".join(inc[p])))
open("/home/m-kikuchi/projects/oprs/docs/_graph.txt", "w").write("\n".join(lines))
print("\n".join(lines[:0]))
print("wrote _graph.txt", len(lines), "lines")
