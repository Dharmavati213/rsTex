#!/usr/bin/env -S python3 -I
"""test_token_fuzz.py: token_fuzz.py judges as its docstring says, on documents whose verdict is known, run on the oracle and on an engine.

    python3 -I tests/test_token_fuzz.py [ENGINE] [--fuzz PATH]

ENGINE is target/release/rstex by default.  --fuzz judges another copy of token_fuzz.py, which has to lie in tests/ to find tests/integration/run.py.  Writes only under
target/token-fuzz/test-PID, and takes that away.  A line per check, ok or FAILED; the last line says PASS or FAIL, and the exit status agrees.
"""
import argparse
import importlib.util
import os
import shutil
import sys

sys.dont_write_bytecode = True
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# (what, f.tex, stdin, interaction, options, the verdict).  Each was judged differs before the change that made it so.
DOCUMENTS = [
    ("kpathsea names the program on stderr, pdftex there and rstex here, and the masks take that out",
     "\\immediate\\openout\\show\n", b"", "batchmode", [], "same"),
    ("a warning that names the program wraps at --small's max_print_line, and the masks join it again there",
     "\\catcode`\\}2{\\pdfmatch{}\n", b"", "scrollmode", ["-cnf-line=error_line=119", "-cnf-line=half_error_line=76", "-cnf-line=max_print_line=69"], "same"),
    ("an e at the prompt runs TEXEDIT, the same on both",
     "\\catcode`\\{1\\catcode`\\}2 \\undefinedone\n\\end\n", b"e\n", "errorstopmode", [], "same"),
]


def load(path):
    spec = importlib.util.spec_from_file_location("token_fuzz_under_test", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("engine", nargs="?", default=os.path.join(ROOT, "target", "release", "rstex"))
    ap.add_argument("--fuzz", default=os.path.join(ROOT, "tests", "token_fuzz.py"))
    args = ap.parse_args()
    fuzz = load(os.path.abspath(args.fuzz))
    engine = os.path.abspath(args.engine)
    base = os.path.join(ROOT, "target", "token-fuzz", "test-%d" % os.getpid())
    work = os.path.join(base, "w")
    os.makedirs(base)
    failed = 0

    def check(what, thunk, want):
        nonlocal failed
        try:
            got = thunk()
        except Exception as e:  # a copy of the fuzzer without what is checked
            got = "%s: %s" % (type(e).__name__, e)
        ok = got == want
        print(("ok " if ok else "FAILED ") + what + ("" if ok else ": %r, not %r" % (got, want)))
        failed += not ok

    try:
        for what, text, stdin, interaction, options, want in DOCUMENTS:
            case = fuzz.Case({"f.tex": text}, stdin, interaction, options)
            check(what, lambda: fuzz.judge(engine, case, work, 40)[0], want)
        check("the environment of a run has TEXEDIT=true", lambda: fuzz.environment().get("TEXEDIT"), "true")

    finally:
        shutil.rmtree(base, ignore_errors=True)
    print("PASS" if not failed else "FAIL: %d" % failed)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
