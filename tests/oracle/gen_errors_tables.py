#!/usr/bin/env python3
"""Expected output of errors.rs, Part 6 of the WEB, and of the terminal routines in print.rs, from the oracle.

Run with python3 -I.  It writes crates/engine/testdata/errors_oracle.tsv, read by crates/engine/tests/errors.rs.

One row per case: NAME EXACT STDIN LOG TERM, escaped as in gen_print_tables.py.  The test replays each case by
name in a child process, with that standard input, since §82 reads the terminal and ends the process.

The fields hold what the oracle printed from the line after a marker, or from a given string, up to a second
marker, or else up to what close_files_and_terminate prints.  EXACT is 1 if the case ends at a marker and the
Rust must print those bytes exactly.  It is 0 if the case reaches jump_out, which is main.rs's: the Rust's
output must then begin with those bytes.
"""
import os
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "engine", "testdata", "errors_oracle.tsv")

HEAD = r"\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \catcode`\^=7"
BEGIN = r"\immediate\write16{<<<>>>}"
END = r"\immediate\write16{>>><<<}"
ENDS = (b"No pages of output", b"!  ==> Fatal error", b"Transcript written", b"Output written on")

# \a is undefined; the letters after it are what `1` or `12` at the prompt delete.
UNDEFINED = [BEGIN, r"\a abcdefghijklmnopqrstuvwxyz", END, r"\end"]
LONG_LINE = [BEGIN, "0123456789 " * 8 + r"\a abcdefghijklmnopqrstuvwxyz" + " 0123456789" * 8, END, r"\end"]
GROUPS = [r"\begingroup" * 256]
LITERAL1 = [r"\pdfoutput=1 \font\f=cmr10 \pageheight=10pt \pagewidth=10pt".replace(r"\page", r"\pdfpage"),
            r"\edef\x{\pdfcolorstackinit page direct{\unexpanded{x}}}",
            r"\shipout\hbox{\pdfcolorstack\x push{1 0 0 rg}}", r"\end"]

CASES = [
    # name, command-line flags, environment, stdin, source lines, start string
    ("scroll", ["-interaction=scrollmode"], {}, b"", UNDEFINED, None),
    ("nonstop", ["-interaction=nonstopmode"], {}, b"", UNDEFINED, None),
    ("batch", ["-interaction=batchmode"], {}, b"", UNDEFINED, None),
    ("stop_return", ["-interaction=errorstopmode"], {}, b"\n", UNDEFINED, None),
    ("stop_help", ["-interaction=errorstopmode"], {}, b"H\n\n", UNDEFINED, None),
    ("stop_help_lower", ["-interaction=errorstopmode"], {}, b"h\n\n", UNDEFINED, None),
    ("stop_help_twice", ["-interaction=errorstopmode"], {}, b"H\nH\n\n", UNDEFINED, None),
    ("stop_delete_1", ["-interaction=errorstopmode"], {}, b"1\n\n", UNDEFINED, None),
    ("stop_delete_3", ["-interaction=errorstopmode"], {}, b"3\n\n", UNDEFINED, None),
    ("stop_delete_12", ["-interaction=errorstopmode"], {}, b"12\n\n", UNDEFINED, None),
    ("stop_delete_then_help", ["-interaction=errorstopmode"], {}, b"2\nH\n\n", UNDEFINED, None),
    ("stop_insert_inline", ["-interaction=errorstopmode"], {}, b"Ihello\n", UNDEFINED, None),
    ("stop_insert_prompt", ["-interaction=errorstopmode"], {}, b"I\nhello\n", UNDEFINED, None),
    ("stop_quiet", ["-interaction=errorstopmode"], {}, b"Q\n", UNDEFINED, None),
    ("stop_run", ["-interaction=errorstopmode"], {}, b"R\n", UNDEFINED, None),
    ("stop_scroll", ["-interaction=errorstopmode"], {}, b"S\n", UNDEFINED, None),
    ("stop_menu", ["-interaction=errorstopmode"], {}, b"Z\n\n", UNDEFINED, None),
    ("stop_exit", ["-interaction=errorstopmode"], {}, b"X\n", UNDEFINED, None),
    ("stop_edit", ["-interaction=errorstopmode"], {"TEXEDIT": "true"}, b"E\n", UNDEFINED, None),
    ("stop_eof", ["-interaction=errorstopmode"], {}, b"", UNDEFINED, None),
    ("halt", ["-interaction=scrollmode", "-halt-on-error"], {}, b"", UNDEFINED, None),
    ("halt_batch", ["-interaction=batchmode", "-halt-on-error"], {}, b"", UNDEFINED, None),
    ("narrow", ["-interaction=scrollmode"], {"error_line": "50", "half_error_line": "30", "max_print_line": "60"}, b"", LONG_LINE, None),
    ("wide", ["-interaction=scrollmode"], {"error_line": "254", "half_error_line": "200", "max_print_line": "300"}, b"", LONG_LINE, None),
    ("int_error", ["-interaction=scrollmode"], {}, b"", [BEGIN, r"\count999999=1", END, r"\end"], None),
    ("errmessage", ["-interaction=scrollmode"], {}, b"", [BEGIN, r"\errmessage{one}", r"\errmessage{two}", END, r"\end"], None),
    ("hundred", ["-interaction=scrollmode"], {}, b"", [r"\a"] * 99 + [BEGIN, r"\a abc", r"\end"], None),
    ("fatal", ["-interaction=nonstopmode"], {}, b"", [r"\input surely-no-such-file", r"\end"], b"! Emergency stop"),
    ("fatal_batch", ["-interaction=batchmode"], {}, b"", [r"\input surely-no-such-file", r"\end"], b"! Emergency stop"),
    ("overflow", ["-interaction=scrollmode"], {}, b"", GROUPS, b"! TeX capacity"),
    ("overflow_stop", ["-interaction=errorstopmode"], {}, b"", GROUPS, b"! TeX capacity"),
    ("overflow_batch", ["-interaction=batchmode"], {}, b"", GROUPS, b"! TeX capacity"),
    ("confusion", ["-etex", "-interaction=scrollmode"], {}, b"", LITERAL1, b"! This can't happen"),
    ("confusion_after_error", ["-etex", "-interaction=scrollmode"], {}, b"", [r"\a"] + LITERAL1, b"! I can't go on"),
]


def esc(b):
    out = []
    for c in b:
        if c == 10:
            out.append("\\n")
        elif c == 9:
            out.append("\\t")
        elif c == 92:
            out.append("\\\\")
        elif 32 <= c < 127:
            out.append(chr(c))
        else:
            out.append("\\x%02x" % c)
    return "".join(out)


def cut(data, start):
    """Returns (slice, exact)."""
    if start is None:
        a = data.find(b"<<<>>>\n")
        if a < 0:
            return b"", False
        a += 7
    else:
        a = data.find(start)
        if a < 0:
            return b"", False
    b = data.find(b">>><<<\n", a)
    if b >= 0:
        return data[a:b], True
    e = [data.find(s, a) for s in ENDS]
    e = [x for x in e if x >= 0]
    if not e:
        return data[a:], False
    # A line ending just before is the print_nl of what comes next, if the line was not empty: not ours.
    return data[a:min(e)].removesuffix(b"\n"), False


def run_case(flags, envadd, stdin, lines, start):
    with tempfile.TemporaryDirectory() as tmp:
        with open(os.path.join(tmp, "t.tex"), "w", encoding="latin-1") as f:
            f.write("\n".join([HEAD] + lines) + "\n")
        env = dict(os.environ, **envadd)
        p = subprocess.run(["pdftex", "-ini"] + flags + ["t.tex"], cwd=tmp, env=env, input=stdin,
                           stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, check=False)
        with open(os.path.join(tmp, "t.log"), "rb") as f:
            log = f.read()
    return log, p.stdout


def first_line_cases():
    """§37: the ** prompt, on a run with nothing on its command line."""
    rows = []
    for name, stdin in [("terminal_blank_lines", b"\n   \n\\end\n"), ("terminal_eof", b""), ("terminal_blank_then_eof", b"\n")]:
        with tempfile.TemporaryDirectory() as tmp:
            p = subprocess.run(["pdftex", "-ini"], cwd=tmp, input=stdin, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, check=False)
        term = p.stdout
        a = term.find(b"**")
        b = term.find(b"\\end")
        tail = term[a:]
        # What follows the last prompt, when \end is taken, is not this routine's.
        k = tail.rfind(b"**") + 2
        if b"End of file on the terminal" in tail:
            k = len(tail)
        rows.append([name, "1", esc(stdin), "", esc(tail[:k])])
    return rows


def main():
    rows = []
    for name, flags, envadd, stdin, lines, start in CASES:
        log, term = run_case(flags, envadd, stdin, lines, start)
        lg, exact = cut(log, start)
        tm, _ = cut(term, start)
        if not lg:
            sys.exit("%s: nothing found in the log\n%s" % (name, log.decode("latin-1")[-1500:]))
        rows.append([name, "1" if exact else "0", esc(stdin), esc(lg), esc(tm)])
    rows += first_line_cases()
    with open(OUT, "w", encoding="ascii", newline="\n") as f:
        for r in rows:
            f.write("\t".join(r) + "\n")
    print("%s: %d rows, %d bytes" % (OUT, len(rows), os.path.getsize(OUT)))


if __name__ == "__main__":
    main()
