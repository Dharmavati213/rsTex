#!/usr/bin/env python3
"""The file name routines and start_input against the oracle: crates/engine/testdata/files_oracle.tsv.

Run with python3 -I.  Each case makes a small tree of synthetic files, and runs

    pdftex -ini -etex -interaction=MODE FLAGS FIRST-LINE

in it with SOURCE_DATE_EPOCH set, then keeps the log and what the terminal shows.  crates/engine/tests/files.rs runs the
same tree through the real start_input, with get_x_token for a main loop: \\input is what expand does with it, and
every other token in these documents leaves no trace on the oracle, so the two logs must be the same up to \\end.

Columns, escaped as in errors_oracle.tsv: name, flags, interaction, first line, standard input, the files
(name=content, separated by |), whether the job gets to \\end (1) or dies on the way (0), the log, the terminal,
the name of the log, and the lines of the .fls file that do not depend on the installation.
"""
import os
import re
import shutil
import subprocess
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "engine", "testdata", "files_oracle.tsv")

A = {"a.tex": "% the file a\n"}
LONG = "x" * 60
# What these documents use to define things: the unit test ignores it, and has the macros already.
DEFS = "\\catcode123=1 \\catcode125=2 \\def\\x{a}\\def\\y{.tex}\n"
# name, document, other files, options
CASES = [
    ("plain", r"\input a" "\n" r"\end" "\n", A, {}),
    ("twice", r"\input a \input a" "\n\\end\n", A, {}),
    ("nested", r"\input b" "\n\\end\n", {"a.tex": "%\n", "b.tex": r"\input a" "\n"}, {}),
    ("with_ext", r"\input a.tex" "\n\\end\n", A, {}),
    ("other_ext", r"\input c.dat" "\n\\end\n", {"c.dat": "%\n"}, {}),
    ("no_ext_file", r"\input e" "\n\\end\n", {"e": "%\n"}, {}),
    ("tex_before_bare", r"\input d" "\n\\end\n", {"d": "%\n", "d.tex": "% d.tex\n"}, {}),
    ("dots", r"\input a.b.c" "\n\\end\n", {"a.b.c.tex": "%\n"}, {}),
    ("dots_found_bare", r"\input a.b.c" "\n\\end\n", {"a.b.c": "%\n"}, {}),
    ("stop_at_space", r"\input a b" "\n\\end\n", A, {}),
    ("many_spaces", r"\input    a    " "\n\\end\n", A, {}),
    ("end_of_line", r"\input a" "\n\n\\end\n", A, {}),
    ("trailing_spaces_in_file", r"\input a    " "\n\\end\n", {"a.tex": "%   \n\n   \n"}, {}),
    ("quoted", r'\input "s p"' "\n\\end\n", {"s p.tex": "%\n"}, {}),
    ("quoted_ext", r'\input "s p.tex"' "\n\\end\n", {"s p.tex": "%\n"}, {}),
    ("quoted_part", r'\input s" "p' "\n\\end\n", {"s p.tex": "%\n"}, {}),
    ("quoted_empty_part", r'\input a""' "\n\\end\n", A, {}),
    ("quoted_no_space", r'\input "a"' "\n\\end\n", A, {}),
    ("quoted_area", r'\input "s p/q r.tex"' "\n\\end\n", {"s p/q r.tex": "%\n"}, {}),
    ("quoted_ext_space", r'\input "q.t x"' "\n\\end\n", {"q.t x": "%\n"}, {}),
    ("quoted_unterminated", r'\input "a' "\n\\end\n", A, {}),
    ("area", r"\input sub/c" "\n\\end\n", {"sub/c.tex": "%\n"}, {}),
    ("area_dot", r"\input ./a" "\n\\end\n", A, {}),
    ("area_dotdot", r"\input sub/../a" "\n\\end\n", {"a.tex": "%\n", "sub/k": "x\n"}, {}),
    ("macro", DEFS + r"\input\x" "\n\\end\n", A, {}),
    ("macro_ext", DEFS + r"\input\x.tex" "\n\\end\n", A, {}),
    ("macro_end", DEFS + r"\input\x\y\relax" "\n\\end\n", A, {}),
    ("relax_ends", r"\input a\relax" "\n\\end\n", A, {}),
    # The name is empty, so the file is .tex: a comment in tex.ch says \relax is skipped, and it is not.
    ("relax_first", r"\input\relax a" "\n\\end\n", {".tex": "%\n"}, {}),
    ("input_in_name", r"\input a\input a" "\n\\end\n", A, {}),
    ("input_at_start_of_name", r"\input\input a" "\n\\end\n", {".tex": "%\n", "a.tex": "%\n"}, {}),
    ("caret", "\\catcode94=7 \\input ^^61\n\\end\n", A, {}),
    ("caret_unprintable", "\\catcode94=7 \\input nosuch^^c8\n\\end\n", A, {"fatal": True}),
    ("csname_in_name", r"\input a\csname relax\endcsname" "\n\\end\n", A, {}),
    ("new_csname_in_name", r"\input t8\csname brandnewname\endcsname" "\n\\end\n", {"t8.tex": "%\n"}, {}),
    ("empty_file", r"\input a" "\n\\end\n", {"a.tex": ""}, {}),
    ("no_newline", r"\input a" "\n\\end\n", {"a.tex": "%"}, {}),
    ("crlf", "\\input a\r\n\\end\r\n", {"a.tex": "%\r\n\r\n"}, {}),
    ("cr_only", "\\input a\r\\end\r", {"a.tex": "%\r"}, {}),
    ("endinput", r"\input a" "\n\\end\n", {"a.tex": "\\endinput\nnever read\n"}, {}),
    ("endinput_line", r"\input a" "\n\\end\n", {"a.tex": "\\endinput \\input b\n", "b.tex": "%\n"}, {}),
    ("long_name", "\\input %s\n\\end\n" % LONG, {LONG + ".tex": "%\n"}, {}),
    ("long_names", "\\input %s1\n\\input %s2\n\\end\n" % (LONG, LONG), {LONG + "1.tex": "%\n", LONG + "2.tex": "%\n"}, {}),
    ("nearly_long_names", "\\input %s1\n\\input %s\n\\end\n" % ("y" * 30, "y" * 31),
     {"y" * 30 + "1.tex": "%\n", "y" * 31 + ".tex": "%\n"}, {}),
    ("recursion", r"\input r" "\n\\end\n", {"r.tex": r"\input r" "\n"}, {"fatal": True}),
    # What is not found.
    ("missing_nonstop", r"\input nosuch" "\n\\end\n", A, {"fatal": True}),
    ("missing_ext", r"\input nosuch.xyz" "\n\\end\n", A, {"fatal": True}),
    ("missing_quoted", r'\input "no such"' "\n\\end\n", A, {"fatal": True}),
    ("missing_batch", r"\input nosuch" "\n\\end\n", A, {"fatal": True, "mode": "batchmode"}),
    ("missing_scroll_eof", r"\input nosuch" "\n\\end\n", A, {"fatal": True, "mode": "scrollmode"}),
    ("missing_stop_eof", r"\input nosuch" "\n\\end\n", A, {"fatal": True, "mode": "errorstopmode"}),
    ("missing_first", "", A, {"fatal": True, "first": "nosuch"}),
    # The prompt.
    ("prompt_answer", r"\input nosuch" "\n\\end\n", A, {"mode": "errorstopmode", "stdin": "a\n"}),
    ("prompt_answer_scroll", r"\input nosuch" "\n\\end\n", A, {"mode": "scrollmode", "stdin": "a\n"}),
    ("prompt_blanks", r"\input nosuch" "\n\\end\n", A, {"mode": "scrollmode", "stdin": "   a  \n"}),
    ("prompt_quoted", r"\input nosuch" "\n\\end\n", {"s p.tex": "%\n"}, {"mode": "scrollmode", "stdin": '"s p"\n'}),
    ("prompt_two_words", r"\input nosuch" "\n\\end\n", A, {"mode": "scrollmode", "stdin": "a b\n"}),
    ("prompt_empty_then_answer", r"\input nosuch" "\n\\end\n", A, {"mode": "scrollmode", "stdin": "\na\n"}),
    ("prompt_wrong_then_answer", r"\input nosuch" "\n\\end\n", A, {"mode": "scrollmode", "stdin": "wrong\na\n"}),
    ("prompt_ext", r"\input nosuch.xyz" "\n\\end\n", {"a.xyz": "%\n"}, {"mode": "scrollmode", "stdin": "a.xyz\n"}),
    ("prompt_keeps_area", r"\input sub/nosuch" "\n\\end\n", A, {"mode": "scrollmode", "stdin": "\na\n"}),
    ("prompt_crlf", r"\input nosuch" "\n\\end\n", A, {"mode": "scrollmode", "stdin": "a\r\n"}),
    ("prompt_cr", r"\input nosuch" "\n\\end\n", A, {"mode": "scrollmode", "stdin": "nosuch2\rwrong\ra\r"}),
    ("prompt_eof", r"\input nosuch" "\n\\end\n", A, {"fatal": True, "mode": "scrollmode", "stdin": "wrong\n"}),
    # Other first lines.
    ("first_input", r"\input a" "\n\\end\n", A, {"first": r"\input t"}),
    ("first_two_words", r"\input a" "\n\\end\n", A, {"first": r"t \input a"}),
    ("first_with_ext", r"\input a" "\n\\end\n", A, {"first": "t.tex"}),
    ("job_from_area", r"\input a" "\n\\end\n", {"a.tex": "%\n", "sub/t.tex": r"\input a" "\n\\end\n"}, {"first": "sub/t"}),
    # Flags.
    ("file_line_error", r"\input nosuch" "\n\\end\n", A, {"fatal": True, "flags": ["-file-line-error"]}),
    ("file_line_error_nested", r"\input b" "\n\\end\n", {"b.tex": "%\n" r"\input nosuch" "\n"}, {"fatal": True, "flags": ["-file-line-error"]}),
    ("jobname", r"\input a" "\n\\end\n", A, {"flags": ["-jobname=other"], "log": "other.log"}),
    ("output_directory", r"\input a" "\n\\end\n", {"a.tex": "%\n", "out/x": ""}, {"flags": ["-output-directory=out"], "log": "out/t.log"}),
    # Only the name as it stands is looked for there: \input a finds ./a.tex, and \input a.tex the other.
    ("output_directory_reads", r"\input a" "\n\\end\n", {"a.tex": "%\n", "out/a.tex": "% this one\n"}, {"flags": ["-output-directory=out"], "log": "out/t.log"}),
    ("output_directory_reads_ext", r"\input a.tex" "\n\\end\n", {"a.tex": "%\n", "out/a.tex": "% this one\n"}, {"flags": ["-output-directory=out"], "log": "out/t.log"}),
    ("no_shell", r"\input a" "\n\\end\n", A, {"flags": ["-no-shell-escape"]}),
    ("shell", r"\input a" "\n\\end\n", A, {"flags": ["-shell-escape"]}),
    ("shell_restricted", r"\input a" "\n\\end\n", A, {"flags": ["-shell-restricted"]}),
    ("pipe_off", r'\input"|echo"' "\n\\end\n", A, {"fatal": True, "flags": ["-no-shell-escape"]}),
    ("pipe", r'\input"|echo"' "\n\\end\n", A, {"flags": ["-shell-escape"]}),
    ("pipe_with_space", r'\input"|true a"' "\n\\end\n", A, {"flags": ["-shell-escape"]}),
    ("pipe_restricted", r'\input"|echo"' "\n\\end\n", A, {"fatal": True, "flags": ["-shell-restricted"]}),
    ("log_unwritable", r"\input a" "\n\\end\n", {"a.tex": "%\n"}, {"fatal": True, "flags": ["-output-directory=nosuchdir"], "log": None}),
    ("log_prompt", r"\input a" "\n\\end\n", {"a.tex": "%\n"}, {"mode": "scrollmode", "stdin": "ok\n", "flags": ["-output-directory=nosuchdir"], "log": None, "fatal": True}),
    ("recorder", r"\input a" "\n\\end\n", A, {"flags": ["-recorder"]}),
    ("recorder_nested", r"\input b" "\n\\end\n", {"a.tex": "%\n", "b.tex": r"\input a" "\n"}, {"flags": ["-recorder", "-jobname=other"], "log": "other.log"}),
]


# After \end: the paragraph the words of a document have made, the file ends, the output line.
END = re.compile(rb"( \[\d+\])*( \))*\s*(?:No pages of output\.|Output written on [^\n]*\.)?\s*(?:Transcript written on [^\n]*\n?)?$")
# After a fatal error, from close_files_and_terminate on.
TAIL = re.compile(rb"\n?(?:No pages of output\.|Output written on [^\n]*\.)?\s*(?:Transcript written on [^\n]*\n?)?$")


def recorded(fls):
    """What the recorder says about this job: not where the format and texmf.cnf are, or the directory."""
    lines = fls.split(b"\n")
    keep = [l for l in lines if l and not l.startswith(b"PWD ") and not re.search(rb"texmf\.cnf$|\.fmt$|\.map$", l)]
    return b"\n".join(keep)


def esc(b):
    out = []
    for c in b:
        if c == 0x5c:
            out.append("\\\\")
        elif c == 10:
            out.append("\\n")
        elif c == 9:
            out.append("\\t")
        elif c == 13:
            out.append("\\r")
        elif c == 0x7c:
            out.append("\\x7c")
        elif 32 <= c < 127:
            out.append(chr(c))
        else:
            out.append("\\x%02x" % c)
    return "".join(out)


def run_case(name, doc, files, opt):
    tmp = tempfile.mkdtemp(prefix="rstex-files-")
    try:
        allf = dict(files)
        if doc:
            allf[opt.get("doc", "t.tex")] = doc
        for path, text in allf.items():
            p = os.path.join(tmp, path)
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "wb") as f:
                f.write(text.encode("latin-1"))
        first = opt.get("first", "t")
        mode = opt.get("mode", "nonstopmode")
        flags = opt.get("flags", [])
        args = ["pdftex", "-ini", "-etex", "-interaction=" + mode] + flags + first.split(" ")
        env = dict(os.environ, SOURCE_DATE_EPOCH="1700000000", FORCE_SOURCE_DATE="1")
        env.pop("TEXMF_OUTPUT_DIRECTORY", None)
        r = subprocess.run(args, cwd=tmp, env=env, input=opt.get("stdin", "").encode("latin-1"),
                           stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, check=False, timeout=20)
        term = r.stdout
        log_name = opt.get("log", "t.log")
        if log_name is None:
            log = b""
            log_name = ""
        else:
            lp = os.path.join(tmp, log_name)
            log = open(lp, "rb").read() if os.path.exists(lp) else b""
            if not log:
                # Before the log opens, a failure leaves texput.log only sometimes.
                log_name = ""
        # What main_body prints before the first thing here runs.
        m = re.search(rb"entering extended mode\n", term)
        term = term[m.end():] if m else term
        fatal = opt.get("fatal", False)
        if fatal:
            # jump_out's part: the unit test asks that its output begin with this, whatever close_files_and_terminate
            # prints after it.
            log = TAIL.sub(b"", log)
            term = TAIL.sub(b"", term)
        else:
            # What final_cleanup prints: not in the unit test, which stops at the \end.
            log = END.sub(b"", log)
            term = END.sub(b"", term)
        # The two differ here, and the date is the same by SOURCE_DATE_EPOCH.
        log = re.sub(rb"^(This is pdfTeX, Version \S+) \(.*?\) \(INITEX\)", rb"\1 (rstex) (INITEX)", log)
        fls = b""
        if "-recorder" in flags:
            with open(os.path.join(tmp, log_name[:-4] + ".fls"), "rb") as f:
                fls = recorded(f.read())
        fields = [name, " ".join(flags), mode, first, esc(opt.get("stdin", "").encode("latin-1")),
                  "|".join("%s=%s" % (k, esc(v.encode("latin-1"))) for k, v in allf.items()),
                  "0" if fatal else "1", esc(log), esc(term), log_name, esc(fls)]
        return "\t".join(fields)
    finally:
        shutil.rmtree(tmp)


def main():
    rows = [run_case(*c) for c in CASES]
    with open(OUT, "w", encoding="latin-1", newline="\n") as f:
        f.write("\n".join(rows) + "\n")
    print(len(rows), "cases in", OUT)


if __name__ == "__main__":
    main()
