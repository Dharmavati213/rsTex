#!/usr/bin/env python3
"""Runs what one line of `% mode:` cannot say: a format dumped by one run and loaded by the next, and the
command line itself.  Run with python3 -I.

    run_pairs.py                   on the oracle alone: says how each run ended
    run_pairs.py --show CASE       on the oracle alone: prints that case's log
    run_pairs.py ENGINE            on the oracle and on ENGINE, comparing logs after masking

A pair is NAME.tex, an ordinary case that ends in \\dump, and NAME.load, run in that format.  All of it happens in
a temporary copy of tests/integration.  ENGINE and the oracle each dump their own format.  A case with no log, such
as a format that cannot be found, is compared on its terminal output.
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
INTEGRATION = os.path.dirname(HERE)
ORACLE = "pdftex"
PAIRS = [
    ("pair_basic", "image/pair_basic.load"),
    ("pair_etex", "image/pair_etex.load"),
    ("pair_hyph_next", "image/pair_hyph_next.load"),
    ("pair_pdf", "image/pair_pdf.load"),
    ("pair_unicode", "image/pair_unicode.load"),
    ("pair_image", "image/pair_image.load"),
    # \endlinechar=-1 in the format: a file name on the first line would have nothing to end it.
    ("pair_endline", "\\input{image/pair_endline.load}"),
]
# No mktexfmt and the like on the oracle: the port drops them, docs/PORTING.md section 19.
ENV = {"SOURCE_DATE_EPOCH": "1700000000", "FORCE_SOURCE_DATE": "1", "MKTEXFMT": "0", "MKTEXTFM": "0", "MKTEXTEX": "0", "MKTEXPK": "0"}
# What an INITEX first line needs to say anything.
B = "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\^=7 "

# Each is (name, arguments after the program, standard input).  File names are in the temporary copy.  What each
# one is aimed at, in maininit, §1517 or §1648, is in its name; the log's first lines and what follows them count.
COMMAND_LINES = [
    ("star_no_spaces", ["-ini", "*\\relax\\end"], ""),
    # A first argument that is no \ or & is a file name, and one with spaces gets quotes: the star is not first.
    ("first_argument_with_spaces", ["-ini", "*" + B + "\\message{[\\the\\eTeXversion]}\\end"], ""),
    ("etex_option", ["-ini", "-etex", B + "\\message{[\\the\\eTeXversion]}\\end"], ""),
    ("no_etex_without_star", ["-ini", B + "\\message{\\ifx\\middle\\undefined undefined\\fi}\\end"], ""),
    ("first_line_at_prompt", ["-ini", "-interaction=nonstopmode"], B + "\\message{typed at the prompt}\\end\n"),
    ("star_at_prompt", ["-ini", "-interaction=nonstopmode"], "*" + B + "\\message{[\\the\\eTeXversion]}\\end\n"),
    ("if_on_line_zero", ["-ini", "-interaction=nonstopmode", "\\iftrue\\end"], ""),
    ("group_on_line_zero", ["-ini", "-interaction=nonstopmode", "\\begingroup\\end"], ""),
    ("blank_first_lines", ["-ini", "-interaction=nonstopmode"], "\n   \n\\end\n"),
    ("eof_at_prompt", ["-ini"], ""),
    ("jobname", ["-ini", "-jobname=otherjob", B + "\\message{\\jobname}\\end"], ""),
    ("jobname_quoted", ["-ini", "-jobname=\"other job\"", B + "\\message{\\jobname}\\end"], ""),
    ("interaction_batch", ["-ini", "-interaction=batchmode", "\\undefinedone\\end"], ""),
    ("interaction_unknown", ["-ini", "-interaction=loud", "-interaction=nonstopmode", "\\end"], ""),
    ("file_line_error", ["-ini", "-file-line-error", "-interaction=nonstopmode", "image/run_pairs_error.inc"], ""),
    ("no_file_line_error", ["-ini", "-no-file-line-error", "-interaction=nonstopmode", "image/run_pairs_error.inc"], ""),
    ("halt_on_error", ["-ini", "-halt-on-error", "image/run_pairs_error.inc"], ""),
    ("eight_bit", ["-ini", "-8bit", B + "\\message{^^80^^ff^^01}\\end"], ""),
    ("not_eight_bit", ["-ini", B + "\\message{^^80^^ff^^01}\\end"], ""),
    ("translate_file", ["-ini", "-translate-file=cp227.tcx", B + "\\message{^^80^^ff^^01^^09}\\end"], ""),
    ("output_format_dvi", ["-ini", "-output-format=dvi", B + "\\pdfoutput=1 \\message{[\\the\\pdfoutput]}\\end"], ""),
    ("output_format_pdf", ["-ini", "-output-format=pdf", B + "\\message{[\\the\\pdfoutput]}\\end"], ""),
    ("draftmode", ["-ini", "-draftmode", B + "\\message{[\\the\\pdfdraftmode]}\\end"], ""),
    ("shell_escape", ["-ini", "-shell-escape", B + "\\message{[\\the\\pdfshellescape]}\\end"], ""),
    ("shell_restricted", ["-ini", "-shell-restricted", B + "\\message{[\\the\\pdfshellescape]}\\end"], ""),
    ("no_shell_escape", ["-ini", "-no-shell-escape", B + "\\message{[\\the\\pdfshellescape]}\\end"], ""),
    ("output_directory", ["-ini", "-output-directory=outdir", B + "\\immediate\\openout3=written \\immediate\\closeout3 \\end"], ""),
    ("recorder", ["-ini", "-recorder", "-jobname=rec", "\\end"], ""),
    ("kpathsea_debug", ["-ini", "-kpathsea-debug=0", "\\end"], ""),
    ("double_dash", ["--ini", "--interaction", "nonstopmode", "\\end"], ""),
    ("dash_dash_ends", ["-ini", "--", "\\end"], ""),
    ("prefix", ["-ini", "-inter=nonstopmode", "\\end"], ""),
    ("unknown_option", ["-ini", "-nosuchoption", "\\end"], ""),
    ("progname_initex", ["--progname=initex", B + "\\message{initex}\\end"], ""),
    ("format_missing", ["-fmt=nosuchformat", "\\end"], ""),
    ("ampersand_missing", ["-fmt=pair_basic", "&nosuchformat", "\\end"], ""),
    ("format_not_a_format", ["-ini", "&notaformat", "\\end"], ""),
    ("format_named_on_first_line", ["-interaction=nonstopmode", "&pair_basic", "\\message{[\\the\\count10]}\\end"], ""),
    ("format_erases_initex", ["-ini", "-interaction=nonstopmode", "&pair_basic", "\\message{[\\the\\count10]}\\end"], ""),
    ("first_line_format", ["-parse-first-line", "image/run_pairs_percent.inc"], ""),
    ("first_line_off", ["-ini", "-no-parse-first-line", "image/run_pairs_percent.inc"], ""),
]


def run(cmd, cwd, stdin=""):
    env = dict(os.environ)
    env.update(ENV)
    try:
        r = subprocess.run(cmd, cwd=cwd, env=env, input=stdin.encode(), stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30)
    except subprocess.TimeoutExpired:
        return -1, "TIMEOUT", ""
    return r.returncode, r.stdout.decode("latin-1"), r.stderr.decode("latin-1")


def mask(text):
    out = []
    for line in text.split("\n"):
        line = re.sub(r"^This is .*$", "BANNER", line)
        line = re.sub(r"\b\d+ strings out of \d+", "N strings out of M", line)
        line = re.sub(r"\b\d+ string characters out of \d+", "N string characters out of M", line)
        line = re.sub(r"\b\d+ words of memory out of \d+", "N words of memory out of M", line)
        line = re.sub(r"\b\d+ strings of total length \d+", "N strings of total length M", line)
        line = re.sub(r"\b\d+ memory locations dumped; current usage is .*", "N memory locations dumped", line)
        line = re.sub(r"(Hyphenation trie of length) \d+ has (\d+ ops? out of) \d+", r"\1 N has \2 M", line)
        line = re.sub(r"Memory usage before: .*", "Memory usage", line)
        line = re.sub(r"stack positions out of .*", "stack positions", line)
        out.append(line)
    return "\n".join(out)


def take_log(work, name):
    for where in (work, os.path.join(work, "outdir")):
        p = os.path.join(where, name + ".log")
        if os.path.exists(p):
            log = open(p, encoding="latin-1").read()
            os.remove(p)
            return log
    return ""


def one_engine(engine):
    """All of it, on one engine: {case: (exit status, terminal, log)}."""
    res = {}
    with tempfile.TemporaryDirectory() as tmp:
        work = os.path.join(tmp, "w")
        shutil.copytree(INTEGRATION, work)
        os.makedirs(os.path.join(work, "outdir"))
        shutil.copy(os.path.join(HERE, "run_pairs_error.inc"), os.path.join(work, "notaformat.fmt"))
        # An image for pair_image: always the oracle's, so both engines read the same file.
        src = "\\pdfoutput=1 \\pdfcompresslevel=0 \\pdfobjcompresslevel=0 \\pdfinfoomitdate=1 \\pdftrailerid{} "
        src += "\\pdfpagewidth=30pt \\pdfpageheight=40pt \\shipout\\hbox{\\pdfliteral{0 0 m 10 10 l S}}\\end"
        run([ORACLE, "-ini", "-jobname=pair_image_src", "-interaction=batchmode", B + src], work)
        for p, line in PAIRS:
            first = open(os.path.join(HERE, p + ".tex"), encoding="latin-1").readline()
            etex = "ini-etex" in first
            name = ("*" if etex else "") + "image/" + p + ".tex"
            r = run([engine, "-ini"] + (["-etex"] if etex else []) + ["-interaction=nonstopmode", name], work)
            res[p + ":dump"] = (r[0], r[1], take_log(work, p))
            if not os.path.exists(os.path.join(work, p + ".fmt")):
                res[p + ":dump"] = (r[0], r[1] + "\nNO FORMAT FILE", res[p + ":dump"][2])
                continue
            job = p + "-run"
            r = run([engine, "-jobname=" + job, "-interaction=nonstopmode", "&" + p, line], work)
            res[p + ":load"] = (r[0], r[1], take_log(work, job))
        # §1507: -interaction wins over the format's; the format named by -fmt, not on the line.
        job = "pair_endline-run2"
        r = run([engine, "-jobname=" + job, "-interaction=scrollmode", "-fmt=pair_endline", PAIRS[-1][1]], work)
        res["fmt_option:load"] = (r[0], r[1], take_log(work, job))
        for name, args, stdin in COMMAND_LINES:
            r = run([engine] + args, work, stdin)
            log = ""
            for cand in ("otherjob", "other job", "rec", "texput", "run_pairs_error", "run_pairs_percent"):
                log = take_log(work, cand) or log
            res["cli:" + name] = (r[0], r[1] + r[2], log)
    return res


def main():
    args = sys.argv[1:]
    if args and args[0] == "--show":
        s = one_engine(ORACLE)[args[1]]
        print("exit", s[0])
        print(s[2] or s[1])
        return
    oracle = one_engine(ORACLE)
    if not args:
        for k, (status, term, log) in oracle.items():
            print("=== %s: exit %d, log %d lines" % (k, status, log.count("\n")))
            print("\n".join(l[:120] for l in term.split("\n")[-5:]))
        return
    ours = one_engine(args[0])
    failed = 0
    for k in oracle:
        a, b = oracle[k], ours.get(k, (None, "", ""))
        # With no log the terminal is all there is.  Its options' messages name the program, so only there.
        x, y = (a[2], b[2]) if a[2] else (a[1], b[1])
        same = a[0] == b[0] and mask(x) == mask(y)
        print("%s %s" % ("same   " if same else "DIFFERS", k))
        if not same:
            failed += 1
            if a[0] != b[0]:
                print("  exit %s, oracle %s" % (b[0], a[0]))
            for i, (p, q) in enumerate(zip(mask(x).split("\n"), mask(y).split("\n"))):
                if p != q:
                    print("  line %d\n    oracle: %s\n    ours:   %s" % (i + 1, p[:150], q[:150]))
                    break
    print("%d differ, of %d" % (failed, len(oracle)))
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
