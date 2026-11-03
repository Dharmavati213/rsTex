#!/usr/bin/env python3
"""Reads a corpus run for tests/gate.sh and says whether its documents agree with the oracle.

    python3 -I tests/gate_verdict.py RESULTS.jsonl [--bytes]

An entry agrees when the runner found it identical: the log, the assignment trace unless the oracle takes too long to make one, every page and the
document structure are equal, and L6 is equal, or equal but for the version string in the banner, or equal once a document's own compression is undone.  An entry on tests/corpus/known_divergences.txt
agrees as well, with its reason.  With --bytes, for a build with the oracle's banner, L6 has to be plainly equal wherever L6 ran, and the count is
printed.  For an entry under user/ or local/ only the id and the level are printed, since a divergence quotes the document.
"""
import json
import sys

PRIVATE = ("user/", "local/", "cd-local/")


def agrees(r, whole_bytes):
    if r["outcome"] in ("known-divergence", "excluded", "oracle-fails"):
        return True
    if r["outcome"] != "identical":
        return False
    return not whole_bytes or (r["levels"].get("L6") or {}).get("status") != "equal-masked"


def main():
    whole_bytes = "--bytes" in sys.argv[2:]
    rows = [json.loads(line) for line in open(sys.argv[1])]
    bad = [r for r in rows if not agrees(r, whole_bytes)]
    for r in bad:
        text = (r.get("first_divergence") or {}).get("text") or ""
        if r["id"].startswith(PRIVATE):
            text = text.split(" ", 1)[0]
        print("  %s: %s: %s" % (r["id"], r["outcome"], text[:200]))
    print("%d of %d entries agree, %d of them on the known list" % (len(rows) - len(bad), len(rows), sum(r["outcome"] == "known-divergence" for r in rows)))
    l6 = [(r["levels"].get("L6") or {}).get("status") for r in rows]
    print("L6: %d equal, %d equal but for the banner, %d equal once decompressed, of %d that ran" % (l6.count("equal"), l6.count("equal-masked"), l6.count("equal-inflated"), sum(s is not None for s in l6)))
    if not rows:
        print("no entries: the run did not start")
    sys.exit(0 if rows and not bad else 1)


if __name__ == "__main__":
    main()
