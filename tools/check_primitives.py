#!/usr/bin/env python3
"""Check tests/primitives.json, the oracle's primitives, against the merged WEB.

Run with python3 -I.  Every primitive of the oracle must appear as primitive("name",...) in
refs/merged/pdftex-merged.web, except the encTeX ones, whose change files are omitted.  Prints the
gap, and the MLTeX primitives, which are in the WEB but which the port drops.  Exit status 1 on an
unexpected gap.
"""
import json
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ENC = {"endmubyte", "mubyte", "mubytein", "mubytelog", "mubyteout", "noconvert", "specialout",
       "xchrcode", "xordcode", "xprncode"}
MLTEX = {"charsubdef", "charsubdefmin", "charsubdefmax", "tracingcharsubdef"}


def web_primitives():
    src = open(os.path.join(ROOT, "refs", "merged", "pdftex-merged.web"), encoding="latin-1").read()
    return set(s.replace('""', '"') for s in re.findall(r'primitive\("((?:[^"]|"")*)"', src))


def main():
    prims = json.load(open(os.path.join(ROOT, "tests", "primitives.json")))
    names = {p["name"] for p in prims}
    web = web_primitives()
    missing = sorted(names - web)
    extra = sorted(web - names)
    print("oracle primitives: %d; in the merged WEB: %d" % (len(names), len(web)))
    print("missing from the WEB, encTeX (expected): %s" % " ".join(sorted(set(missing) & ENC)))
    gap = sorted(set(missing) - ENC)
    print("missing from the WEB, other (a gap): %s" % (" ".join(gap) or "none"))
    print("in the WEB, not in the inventory: %s" % (" ".join(extra) or "none"))
    print("in the WEB, dropped by the port (MLTeX): %s" % " ".join(sorted(MLTEX & web)))
    sys.exit(1 if gap or extra else 0)


if __name__ == "__main__":
    main()
