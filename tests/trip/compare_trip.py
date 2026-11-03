#!/usr/bin/env python3
"""TRIP and e-TRIP on a candidate engine against the oracle's own run, not against Knuth's master logs.

The master logs are tex.web's: they differ from any pdfTeX in primitives, pool and memory layout, which is why
run_trip.sh's filters exist and why its counts never reach zero.  The oracle takes the same input through the same
texmf.cnf, so what remains here is what the port did differently.

    python3 -I tests/trip/compare_trip.py ENGINE [--work DIR] [--refs DIR]

Exit status 0 when nothing but MASKED differs.  Output goes under the work directory, by default target/trip.
"""
import argparse
import difflib
import os
import re
import shutil
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ORACLE = "/usr/bin/pdftex"

# The oracle's pool does not fit Knuth's sizes, so both engines get the same larger ones.
SIZES = {"TRIP_MEM": "3999", "TRIP_STRINGS": "6000", "TRIP_POOL": "90000", "ETRIP_STRINGS": "6000", "ETRIP_POOL": "90000"}

# A hunk made only of lines like these is not a difference.  Each is by design:
MASKED = re.compile(
    "|".join(
        (
            r"^This is pdfTeX",                     # (TeX Live ...) against (rstex)
            r"^Memory usage before",                # statistics of mem: token lists are not in mem, and the sizes differ
            r"^ *\d+ strings out of",               # no pool file, so fewer strings
            r"^ *\d+ string characters out of",     # likewise
            r"^ *\d+ words of memory out of",       # likewise for mem
            r"^\d+ strings of total length",        # likewise, in the format's statistics
            r"^\d+ memory locations dumped",        # likewise
            r"^\s*$",                               # "still untouched: 33552348" wraps at max_print_line, 72 in TRIP
        )
    )
)
COMPARED = re.compile(r"\.(log|fot|dvi|typ|out)$|^(8terminal|tripos)\.tex$")


def run(engine, work, refs):
    # One clock for both: the DVI's preamble says " TeX output YYYY.MM.DD:HHMM", and two runs either side of a minute's end would differ there, in trip.dvi and trip.typ.
    env = dict(os.environ, SOURCE_DATE_EPOCH="1700000000", FORCE_SOURCE_DATE="1", **SIZES)
    p = subprocess.run(
        ["bash", os.path.join(ROOT, "tests", "trip", "run_trip.sh"), refs, work, engine, "etrip"],
        env=env, capture_output=True, text=True, timeout=600,
    )
    if p.returncode != 0:
        sys.exit("run_trip.sh failed for %s:\n%s%s" % (engine, p.stdout, p.stderr))


def compare(a, b):
    """The hunks of two logs that are not all MASKED, as (line, lines of a, lines of b)."""
    al = a.decode("latin-1").split("\n")
    bl = b.decode("latin-1").split("\n")
    found = []
    for tag, i1, i2, j1, j2 in difflib.SequenceMatcher(None, al, bl, autojunk=False).get_opcodes():
        if tag != "equal" and not all(MASKED.search(t) for t in al[i1:i2] + bl[j1:j2]):
            found.append((i1 + 1, al[i1:i2], bl[j1:j2]))
    return found


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("engine")
    ap.add_argument("--work", default=os.path.join(ROOT, "target", "trip"))
    ap.add_argument("--refs", default=os.path.join(ROOT, "refs"))
    args = ap.parse_args()
    oracle, candidate = os.path.join(args.work, "oracle"), os.path.join(args.work, "candidate")
    shutil.rmtree(args.work, ignore_errors=True)
    os.makedirs(args.work)
    run(ORACLE, oracle, args.refs)
    run(os.path.abspath(args.engine), candidate, args.refs)

    bad = 0
    for sub in ("c", "x", "e"):  # compatibility mode, extended mode, e-TRIP
        names = sorted(n for n in os.listdir(os.path.join(oracle, sub)) if COMPARED.search(n))
        if not names:
            sys.exit("the oracle wrote nothing in %s: the run itself failed" % sub)
        for n in names:
            pa, pb = os.path.join(oracle, sub, n), os.path.join(candidate, sub, n)
            if not os.path.exists(pb):
                print("%s/%s: MISSING on the candidate" % (sub, n))
                bad += 1
                continue
            a, b = open(pa, "rb").read(), open(pb, "rb").read()
            if a == b:
                print("%s/%s: identical" % (sub, n))
            elif n.endswith(".dvi"):
                print("%s/%s: DIFFERS, %d against %d bytes" % (sub, n, len(a), len(b)))
                bad += 1
            else:
                hunks = compare(a, b)
                print("%s/%s: %s" % (sub, n, "same but for the masked lines" if not hunks else "%d hunks DIFFER" % len(hunks)))
                bad += len(hunks)
                for line, x, y in hunks[:12]:
                    print("  line %d" % line)
                    for t in x[:6]:
                        print("   < " + t[:110])
                    for t in y[:6]:
                        print("   > " + t[:110])
    print("TRIP and e-TRIP: %s" % ("PASS" if not bad else "%d differences" % bad))
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
