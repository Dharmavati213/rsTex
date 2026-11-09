#!/usr/bin/env -S python3 -I
"""test_token_fuzz.py: token_fuzz.py judges as its docstring says, on documents whose verdict is known, run on the oracle and on an engine, and on runs made up here.

    python3 -I tests/test_token_fuzz.py [ENGINE] [--fuzz PATH]

ENGINE is target/release/rstex by default.  --fuzz judges another copy of token_fuzz.py, which has to lie in tests/ to find tests/integration/run.py.  Writes only under
target/token-fuzz/test-PID, and takes that away.  A line per check, ok or FAILED; the last line says PASS or FAIL, and the exit status agrees.
"""
import argparse
import importlib.util
import os
import shutil
import signal
import sys

sys.dont_write_bytecode = True
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# (what, f.tex, stdin, interaction, options, the verdict).  Each was judged differs before the change that made it so.
DOCUMENTS = [
    ("kpathsea names the program on stderr, pdftex there and rstex here, and the masks take that out",
     "\\immediate\\openout\\show\n", b"", "batchmode", [], "same"),
    ("a warning that names the program wraps at --small's max_print_line, and the masks join it again there",
     "\\catcode`\\}2{\\pdfmatch{}\n", b"", "scrollmode", ["-cnf-line=error_line=119", "-cnf-line=half_error_line=76", "-cnf-line=max_print_line=69"], "same"),
    ("a page counter past 2^31 is a wrap: [0.0.0.2147483648] there, [0.0.0.-2147483648] here",
     "\\catcode`\\{1\\catcode`\\}2\\count3=1 \\advance\\count3 2147483647 \\shipout\\hbox{}\\end\n", b"", "batchmode", [], "wrap"),
    ("a box width that wraps makes a Huge page there and none here, and the exit status comes first",
     "\\catcode`\\{1\\catcode`\\}2 \\shipout\\hbox{\\hskip16383.99998pt\\vrule width16383.99998pt\\hskip3.33333pt}\\end\n", b"", "batchmode", [], "wrap"),
    ("the same Huge page with another error first, so that the log comes first, at [0",
     "\\catcode`\\{1\\catcode`\\}2 \\undefined \\shipout\\hbox{\\hskip16383.99998pt\\vrule width16383.99998pt\\hskip3.33333pt}\\end\n", b"", "batchmode", [], "wrap"),
    ("a line whose width wraps is Overfull there and not here, a line after the first difference",
     "\\catcode`\\{1 \\catcode`\\}2 \\fontdimen2\\nullfont=16383.99998pt \\hsize0pt \\parindent0pt \\hbadness10000\n\\setbox0\\hbox{\\ \\ \\ }\\noindent\\copy0\\par\\end\n",
     b"", "batchmode", [], "wrap"),
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

        # Runs made up here, for what takes too long to make or cannot be made with a correct engine.
        def run(rc=1, log=(), pdf_size=None, width=79):
            return fuzz.Run(rc, list(log), [], [], None, {}, 0.0, pdf_size, width)
        memory = "! TeX capacity exceeded, sorry [main memory size=5000000]."
        pool = "! TeX capacity exceeded, sorry [pool size=6231990]."
        check("main memory on the oracle's side only differs", lambda: fuzz.verdict_of(run(log=["a", memory]), run(log=["a", "b"]))[0], "differs")
        check("main memory on the engine's side only differs", lambda: fuzz.verdict_of(run(log=["a", "b"]), run(log=["a", memory]))[0], "differs")
        check("main memory at the same token on both is compared to the end", lambda: fuzz.verdict_of(run(log=["a", memory, "c"]), run(log=["a", memory, "c"]))[0], "overflow")
        check("main memory on both, and then another line", lambda: fuzz.verdict_of(run(log=["a", memory, "c"]), run(log=["a", memory, "d"]))[0], "differs")
        check("the pool on one side is passed over", lambda: fuzz.verdict_of(run(log=["a", pool]), run(log=["a", "b"]))[0], "capacity:pool size")
        check("the pool on both, with numbers that differ by design, is passed over",
              lambda: fuzz.verdict_of(run(log=["a", pool]), run(log=["a", pool.replace("6231990", "6201739")]))[0], "capacity:pool size")
        wrapped = ["! TeX capacity exceeded, sorry [indirect objects table size=83", "88607]."]
        check("a capacity whose message wraps at max_print_line is read whole",
              lambda: fuzz.capacity_of(run(log=["a"] + wrapped, width=len(wrapped[0]))), ("indirect objects table size", "8388607"))
        check("and on one side only it differs", lambda: fuzz.verdict_of(run(log=["a"] + wrapped, width=len(wrapped[0])), run(log=["a", "b"], width=len(wrapped[0])))[0], "differs")
        wrapped_pool = ["! TeX capacity exceeded, sorry [pool size=623199", "0]."]
        check("the pool's message wrapped is passed over too", lambda: fuzz.verdict_of(run(log=["a"] + wrapped_pool, width=len(wrapped_pool[0])), run(log=["a", "b"]))[0], "capacity:pool size")
        xfsz = -signal.SIGXFSZ
        limit = fuzz.OUTPUT_LIMIT
        check("the oracle's PDF past the limit, and the engine ended with none, is passed over",
              lambda: fuzz.verdict_of(run(rc=xfsz, log=["a"], pdf_size=limit), run(rc=1, log=["a", "b"]))[0], "timeout")
        check("the oracle's log past the limit, with its PDF short of it, differs",
              lambda: fuzz.verdict_of(run(rc=xfsz, log=["a"], pdf_size=1000), run(rc=1, log=["a", "b"]))[0], "differs")
        check("the oracle's PDF past the limit, and the engine crashed, differs",
              lambda: fuzz.verdict_of(run(rc=xfsz, log=["a"], pdf_size=limit), run(rc=-signal.SIGSEGV, log=["a"]))[0], "differs")
        check("the oracle's PDF past the limit, and the engine wrote one, differs",
              lambda: fuzz.verdict_of(run(rc=xfsz, log=["a"], pdf_size=limit), run(rc=0, log=["a"], pdf_size=5000))[0], "differs")

        def kept_format():
            fuzz.FORMAT_KIND[0] = "outer"
            fuzz.save_case(fuzz.Case({"f.tex": "x"}, b"", "batchmode", ["-cnf-line=max_print_line=70"]), os.path.join(base, "kept"))
            fuzz.FORMAT_KIND[0] = None
            case, kind = fuzz.load_case(os.path.join(base, "kept"))
            return kind, case.options
        check("a kept case says what format it ran on, and its options", kept_format, ("outer", ["-cnf-line=max_print_line=70"]))

        # --memory: the filler leaves the cells it says, on both programs, and a \def of nothing takes two, its head and its end_match.
        def filled(left, rest, kind=None, m=3000, kerns=0):
            fuzz.FORMAT_KIND[0] = kind
            try:
                text = fuzz.PREAMBLE + fuzz.memory_filler(m - fuzz.MEMORY_USED[kind] - left, kerns) + rest + "\\end\n"
                return fuzz.judge(engine, fuzz.Case({"f.tex": text}, b"", "batchmode", ["-cnf-line=main_memory=%d" % m]), work, 40)[0]
            finally:
                fuzz.FORMAT_KIND[0] = None
                fuzz.drop_formats_for([work])
        check("--memory's filler with one cell left: a \\def of nothing fills main memory at the same token on both", lambda: filled(1, "\\def\\a{}"), "overflow")
        check("and with two left it fits", lambda: filled(2, "\\def\\a{}"), "same")
        check("under --format, whose formats are made with the document's main_memory, above webmem.rs's MARGIN",
              lambda: (filled(1, "\\def\\a{}", "clean", 70000), filled(2, "\\def\\a{}", "outer", 70000)), ("overflow", "same"))
        check("so many kerns before the filler leave its room as it was, on -ini and on a format",
              lambda: tuple(filled(left, "\\def\\a{}", kind, 5000, fuzz.MEMORY_KERNS) for kind in (None, "clean") for left in (1, 2)), ("overflow", "same") * 2)

        def work_kept():
            fuzz.write_case(fuzz.Case({"f.tex": "a"}), work)
            open(os.path.join(work, "f.log"), "w").close()
            fd = os.open(work, os.O_RDONLY)
            try:
                fuzz.write_case(fuzz.Case({"f.tex": "b"}), work)
                other = os.path.join(base, "w9-fmt-oracle")
                os.makedirs(other, exist_ok=True)
                fuzz.drop_formats_for([work])
                return os.fstat(fd).st_nlink > 0 and os.path.samestat(os.fstat(fd), os.stat(work)), sorted(os.listdir(work)), os.path.isdir(other)
            finally:
                os.close(fd)
        check("a work directory stays from one document to the next, emptied, as kpathsea's xgetcwd stops at an entry that goes, and a run takes away only its own formats",
              work_kept, (True, ["f.tex"], True))
        check("no line of the filler is longer than --small's least buf_size", lambda: max(len(line) for line in fuzz.memory_filler(300000, fuzz.MEMORY_KERNS).split("\n")) < 500, True)

        def seeded():
            a, b, small = fuzz.case_for("scan", 7, True, True), fuzz.case_for("scan", 7, True, True), fuzz.case_for("scan", 7, True, False)
            plain = fuzz.case_of("scan", 7).files["f.tex"]
            m = fuzz.main_memory_of(a.options)
            return (a.files == b.files and a.options == b.options, a.options[:len(small.options)] == small.options, 3000 <= m <= 300000,
                    a.files["f.tex"].startswith(fuzz.PREAMBLE) and "\\def\\Mfill{" in a.files["f.tex"] and a.files["f.tex"].endswith(plain[len(fuzz.PREAMBLE):]))
        check("the seed alone makes a --memory document: with --small's capacities and main_memory among its options, and the generator's document after the filler",
              seeded, (True, True, True, True))
    finally:
        shutil.rmtree(base, ignore_errors=True)
    print("PASS" if not failed else "FAIL: %d" % failed)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
