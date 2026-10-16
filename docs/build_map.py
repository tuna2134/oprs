#!/usr/bin/env python3
"""Concatenate the section files into docs/libopus-map.md."""
import os, subprocess

D = os.path.dirname(os.path.abspath(__file__))
ORDER = ["_s0_1.md", "_s2.md", "_s3.md", "_s4.md", "_s5.md", "_s6.md",
         "_s7.md", "_s8_appendix.md"]
OUT = os.path.join(D, "libopus-map.md")

TOC = """
## Table of contents

1. [Build configuration matrix](#1-build-configuration-matrix)
2. [File inventory](#2-file-inventory)
3. [Dependency order (bottom-up porting order)](#3-dependency-order-bottom-up-porting-order)
4. [Public / internal API surface needed by the opus layer](#4-public--internal-api-surface-needed-by-the-opus-layer)
5. [State structs](#5-state-structs)
6. [Data tables](#6-data-tables)
7. [Bit-exactness hazards](#7-bit-exactness-hazards)

Appendices:
[A — file-level call graph](#appendix-a--mechanically-extracted-file-level-call-graph) ·
[B — reproducing the reference build](#appendix-b--reproducing-the-reference-build) ·
[C — reference `config.h` macro set](#appendix-c--the-reference-configh-macro-set) ·
[D — how to use this document](#appendix-d--how-to-use-this-document)

---

"""

parts = []
for f in ORDER:
    p = os.path.join(D, f)
    if not os.path.exists(p):
        raise SystemExit("missing " + p)
    parts.append(open(p).read().rstrip("\n"))

head = parts[0]
# insert the TOC right after the document title block (before "## 1.")
i = head.index("## 1. Build configuration matrix")
body = head[:i] + TOC.lstrip("\n") + head[i:]
open(OUT, "w").write(body.rstrip("\n") + "\n\n" + "\n\n".join(parts[1:]) + "\n")
print("wrote", OUT)
print("lines:", sum(1 for _ in open(OUT)),
      "bytes:", os.path.getsize(OUT))
