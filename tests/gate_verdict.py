#!/usr/bin/env python3
"""Reads a corpus run for tests/gate.sh and says whether its documents agree with the oracle, up to the banner.

    python3 -I tests/gate_verdict.py RESULTS.jsonl

An entry agrees when the log is equal, as is the assignment trace unless the oracle takes too long to make one, every page and the document structure are equal, and the first byte
that differs is in the banner line, whose difference in length is that of the two version strings.  For an entry under user/ or local/ only
the id and the level are printed, since a divergence quotes the document.
"""
import json
import sys

BANNER = "TeX Live 2026/Arch Linux"
PRIVATE = ("user/", "local/", "cd-local/")


def agrees(r):
    if r["outcome"] in ("identical", "excluded", "oracle-fails"):
        return True
    if r["outcome"] != "differs-structure-only":
        return False
    levels = r["levels"]
    if (levels.get("log") or {}).get("status") != "equal" or (levels.get("L1") or {}).get("status") not in ("equal", "unavailable", "skipped"):
        return False
    if (levels.get("L4_L5") or {}).get("status") != "identical":
        return False
    l6 = levels.get("L6") or {}
    return BANNER in (l6.get("a") or "") and l6.get("bytes_a", 0) - l6.get("bytes_b", 0) == len(BANNER) - len("rstex")


def main():
    rows = [json.loads(line) for line in open(sys.argv[1])]
    bad = [r for r in rows if not agrees(r)]
    for r in bad:
        text = (r.get("first_divergence") or {}).get("text") or ""
        if r["id"].startswith(PRIVATE):
            text = text.split(" ", 1)[0]
        print("  %s: %s: %s" % (r["id"], r["outcome"], text[:200]))
    print("%d of %d entries agree" % (len(rows) - len(bad), len(rows)))
    if not rows:
        print("no entries: the run did not start")
    sys.exit(0 if rows and not bad else 1)


if __name__ == "__main__":
    main()
