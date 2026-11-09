#!/usr/bin/env python3
"""Expected results for the unit tests of control.rs and build.rs, taken from the oracle.

Run with python3 -I.  It writes, under crates/engine/testdata/:

  build_messages_oracle.tsv  KEY, MESSAGE, HELP: the first error after a snippet starts, from the log.  The test reaches
                             the same procedure by setting the state KEY names.
  build_space_oracle.tsv     space factor, \\spaceskip, \\xspaceskip, then \\lastskip after a space, for cmr10; first row is
                             cmr10's parameters in sp (space, stretch, shrink, extra_space).
  build_prevgraf_oracle.tsv  \\lefthyphenmin, \\righthyphenmin, \\language, then what \\showlists prints for a new paragraph.

Format: one case per line, fields separated by a tab; a newline in a field is `|`, which none of these contain.
"""
import os
import re
import subprocess
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "engine", "testdata")

HEAD = r"""\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \catcode`\^=7 \catcode`\$=3 \catcode`\&=4 \catcode`\_=8
\errorcontextlines=0 \tracingonline=1 \showboxbreadth=100 \showboxdepth=100
\output={\setbox0\box255 \deadcycles=0 }
\font\tenrm=cmr10 \tenrm \delcode`(="028300
"""
MARK = r"\immediate\write16{<<<>>>}"

MESSAGES = [
    ("illegal_mac_param", "#"),
    ("illegal_raise", r"\raise1pt\hbox{}"),
    ("illegal_vadjust", r"\vadjust{}"),
    ("illegal_ital_corr", r"\/"),
    ("illegal_end_in_vbox", r"\setbox0\vbox{\end}"),
    ("dollar", "^"),
    ("extra_endgroup", r"\endgroup"),
    ("missing_right_brace", r"{\endgroup"),
    ("missing_endgroup", r"$\begingroup$"),
    ("missing_dollar", r"$\endgroup"),
    ("missing_right_dot", r"$\left(\endgroup"),
    ("too_many", "}"),
    ("extra_endgroup_brace", r"\begingroup}"),
    ("extra_dollar_brace", "$}"),
    ("extra_right_brace", r"$\left(}"),
    ("missing_cr", r"\halign{#\cr x}"),
    ("leaders", r"\setbox0\hbox{\leaders\hrule x}"),
    ("no_box", r"\setbox0 x"),
    ("hrule_in_hbox", r"\setbox0\hbox{\hrule"),
    ("lastbox_math", r"$\lastbox"),
    ("lastbox_page", r"\lastbox"),
    ("unskip", r"\hrule\vskip1pt\par\unskip"),
    ("unkern", r"\unkern"),
    ("unpenalty", r"\unpenalty"),
    ("improper_disc", r"\setbox0\hbox{\discretionary{\penalty0}"),
    ("illegal_math_disc", r"$\discretionary{}{}{a}"),
    ("misplaced_tab", "&"),
    ("misplaced_span", r"\span"),
    ("misplaced_cr", r"\cr"),
    ("misplaced_crcr", r"\crcr"),
    ("missing_right_in_cell", r"\halign{#&#\cr {a&"),
    ("missing_left_in_cell", r"\halign{#&#\cr \relax\iffalse}\fi a&"),
    ("noalign", r"\noalign"),
    ("omit", r"\omit"),
    ("endcsname", r"\endcsname"),
]

# (\spacefactor, \spaceskip, \xspaceskip)
SPACES = [
    (s, sk, xs)
    for sk, xs in [("0pt", "0pt"), ("3pt plus 2pt minus 1pt", "0pt"), ("0pt", "7pt plus 5pt minus 3pt"),
                   ("3pt plus 2pt minus 1pt", "7pt plus 5pt minus 3pt"), ("1pt plus 2fil minus 3filll", "0pt"),
                   ("0pt plus 1pt", "0pt plus 1pt")]
    for s in [1, 999, 1001, 1250, 1999, 2000, 3000, 32767]
]

PREVGRAF = [(-1, 0, 0), (0, 1, 1), (1, 2, 2), (2, 3, 255), (62, 63, 256), (63, 64, -1), (64, 65, 7), (1000000, -1000000, 3)]


def run(body):
    with tempfile.TemporaryDirectory() as d:
        with open(os.path.join(d, "t.tex"), "w") as f:
            f.write(HEAD + body + "\n\\end\n")
        subprocess.run(["pdftex", "-ini", "-etex", "-interaction=nonstopmode", "t.tex"], cwd=d, stdin=subprocess.DEVNULL,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        return open(os.path.join(d, "t.log"), encoding="latin1").read()


def first_error(log):
    lines = log[log.index("<<<>>>"):].split("\n")
    k = next(i for i, x in enumerate(lines) if x.startswith("! "))
    message = lines[k]
    k += 1
    while not re.match(r"l\.\d+ ", lines[k]):
        k += 1
    k += 2  # the context's two lines
    help_lines = []
    while lines[k]:
        help_lines.append(lines[k])
        k += 1
    return message, help_lines


def messages():
    rows = []
    for key, src in MESSAGES:
        m, h = first_error(run(MARK + "\n" + src))
        rows.append("\t".join([key, m, "|".join(h)]))
    return rows


def spaces():
    log = run(r"\immediate\write16{<\number\fontdimen2\tenrm,\number\fontdimen3\tenrm,\number\fontdimen4\tenrm,"
              r"\number\fontdimen7\tenrm>}")
    a = re.search(r"<(-?\d+),(-?\d+),(-?\d+),(-?\d+)>", log).groups()
    rows = ["\t".join(a)]
    body = []
    for s, sk, xs in SPACES:
        body.append(r"\spaceskip=%s \xspaceskip=%s \setbox0\hbox{\spacefactor=%d { }\edef\x{\the\lastskip}\immediate\write16{=\x=}}"
                    % (sk, xs, s))
    log = run("\n".join(body))
    values = re.findall(r"=([^=\n]*)=\n", log)
    assert len(values) == len(SPACES), (len(values), len(SPACES))
    for (s, sk, xs), v in zip(SPACES, values):
        rows.append("\t".join([str(s), sk, xs, v]))
    return rows


def prevgraf():
    rows = []
    for lhm, rhm, lang in PREVGRAF:
        log = run(r"\lefthyphenmin=%d \righthyphenmin=%d \language=%d \noindent\showlists" % (lhm, rhm, lang))
        m = re.search(r"### horizontal mode entered at line \d+( \(language.*\))?", log)
        rows.append("\t".join([str(lhm), str(rhm), str(lang), (m.group(1) or "").strip()]))
    return rows


def write(name, rows):
    with open(os.path.join(OUT, name), "w") as f:
        f.write("\n".join(rows) + "\n")
    print(name, len(rows))


write("build_messages_oracle.tsv", messages())
write("build_space_oracle.tsv", spaces())
write("build_prevgraf_oracle.tsv", prevgraf())
