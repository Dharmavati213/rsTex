What the numbers on the output cache came from.  Each says what it does at its top.  They name documents by id and print counters, nothing of a document.
    oc.py BINARY DOCS [--edit]      no cache, first, hits, and a word, a new symbol and the preamble edited in a copy under target/; compares PDF, log and .aux
    breakdown.py BINARY DOCS        phases and scopes of a prof build, with the last PDF kept
    turns.py BINARY DOCS N [counted]   CPU, wall and counters in turns, with and without
    paired.py BINARY DOCS [-n N] [--base PATH]   the same in rounds, ratios to no cache round by round, with a second no cache for the noise
    mainthread.py BINARY DOCS       the engine's thread alone against all threads
    passes.py BINARY DOCS           three passes from nothing
    firstrun.py BINARY DOCS N       what it adds to a run with no earlier one
    ckdiff.py BINARY                what in a checkpoint depends on the main file's name, and what the class line and the rest cost
    census.py, prefix.py            first lines and shared preamble prefixes of the public corpus
tools/prof/critic has what the critic of the plan judged it with: oc_adv.py the changes a key might not see, bigpdf.py the file that was the losing class, oc_census.py documents nobody picked.
