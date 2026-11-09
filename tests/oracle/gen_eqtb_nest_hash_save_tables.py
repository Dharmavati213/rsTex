#!/usr/bin/env python3
"""Expected results for the unit tests of eqtb.rs, nest.rs, hash.rs and save.rs, taken from the oracle.

Run with python3 -I.  It writes, under crates/engine/testdata/:

  save_trace_oracle.tsv     mode, operations, then the \\tracingrestores and \\tracingassigns lines
  save_groups_oracle.tsv    the first operation's line, \\tracinggroups, operations, then the lines
                            of \\showgroups and \\tracinggroups
  save_limits_oracle.tsv    what overflows, its limit, the steps that succeeded, the oracle's message
  save_mag_oracle.tsv       \\mag values, one per call of prepare_mag, then its messages and the results
  nest_oracle.tsv           the first operation's line, operations, then the lines of \\showlists
  nest_page_oracle.tsv      the same for the status of the current page; the test builds that by hand
  nest_in_mode_oracle.tsv   a mode, then the tail of "You can't use"
  eqtb_names_oracle.tsv     every primitive of assign_int, assign_dimen, assign_glue, assign_mu_glue,
                            with its offset in the region and how the oracle prints it
  hash_print_cs_oracle.tsv  a kind of control sequence, then how the oracle prints it in a list

Every case runs in `pdftex -ini -etex` (or without -etex for compatibility mode) in a directory of its
own, with each operation on a line of its own, so that its line number is known and the machine starts
afresh.  The tests read the same operations and carry them out on the Rust routines directly, doing
what the commands of main_control would do.

Format: one case per line, fields separated by a tab.  Operations are separated by `;` and their
arguments by a space.  A newline inside a field is `|`, which no output here contains; a control byte
would make the privacy guard take the table for a binary file.
"""
import os
import re
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "engine", "testdata")

PREAMBLE = r"""\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \catcode`\^=7 \catcode`\_=8
\catcode`\&=4 \catcode`\$=3 \catcode`\~=13
\scrollmode \tracingonline=0 \newlinechar=-1
"""
PREAMBLE_LINES = PREAMBLE.count("\n")


def run_oracle(body, tmp, etex=True, cnf=()):
    path = os.path.join(tmp, "t.tex")
    with open(path, "wb") as f:
        f.write((PREAMBLE + body + "\n\\end\n").encode("latin-1"))
    cmd = ["pdftex", "-ini"] + (["-etex"] if etex else [])
    cmd += ["-interaction=nonstopmode", "-translate-file=cp227.tcx"]
    cmd += ["--cnf-line=%s" % c for c in cnf]
    subprocess.run(cmd + ["t.tex"], cwd=tmp, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
    with open(os.path.join(tmp, "t.log"), "rb") as f:
        return f.read().decode("latin-1")


def field(s):
    if "|" in s:
        sys.exit("a | in the expected output: %r" % s)
    return s.replace("\n", "|")


def fp(sp, unit="pt"):
    """A scaled value as TeX source; every value used is an exact multiple of 2^-1 pt."""
    return "%g%s" % (sp / 65536, unit)


NAMED_INT = {"tolerance", "TeXXeTstate", "pdfoutput", "synctex", "language", "lefthyphenmin", "righthyphenmin"}
NAMED_DIMEN = {"hsize", "pdfpagewidth"}


def word_tex(kind, n, v):
    if kind == "count":
        return "\\count%s=%s " % (n, v)
    if kind == "dimen":
        return "\\dimen%s=%ssp " % (n, v)
    if kind == "delcode":
        return "\\delcode%s=%s " % (n, v)
    if kind in NAMED_INT:
        return "\\%s=%s " % (kind, v)
    if kind in NAMED_DIMEN:
        return "\\%s=%ssp " % (kind, v)
    sys.exit("word: " + kind)


def glue_tex(kind, n, w, s, so, h, ho):
    unit = "mu" if kind in ("muskip", "thinmuskip") else "pt"
    reg = "\\%s%s" % (kind, n if kind in ("skip", "muskip") else "")
    orders = [unit, "fil", "fill", "filll"]
    return "%s=%s plus %s minus %s " % (reg, fp(int(w), unit), fp(int(s), orders[int(so)]), fp(int(h), orders[int(ho)]))


def name_tex(name):
    if name == "~":
        return "~"
    if name == "NULLCS":
        return "\\csname\\endcsname"
    return "\\" + name


def trace_op_tex(op):
    f = op.split(" ")
    k = f[0]
    if k == "beg":
        return "\\begingroup"
    if k == "end":
        return "\\endgroup"
    g = "\\global" if k[0].isupper() else ""
    k = k.lower()
    if k == "w":
        return g + word_tex(f[1], f[2], f[3])
    if k == "h":
        return "%s\\%s%s=%s " % (g, f[1], f[2], f[3])
    if k == "f":
        if f[1] == "curfont":
            return g + "\\nullfont "
        return "%s\\%s%s=\\nullfont " % (g, f[1], f[2])
    if k == "g":
        return g + glue_tex(*f[1:])
    if k == "t":
        reg = "\\toks%s" % f[2] if f[1] == "toks" else "\\" + f[1]
        return "%s%s={%s}" % (g, reg, f[3] if len(f) > 3 else "")
    if k == "q":
        return "%s\\toks%s=\\toks%s " % (g, f[2], f[4])
    if k == "d":
        n = name_tex(f[1])
        return "%s\\def%s{%s}" % (g, n, f[2] if len(f) > 2 else "") if f[1] != "NULLCS" else "%s\\expandafter\\def%s{%s}" % (g, n, f[2] if len(f) > 2 else "")
    if k == "l":
        return "%s\\let%s=%s " % (g, name_tex(f[1]), name_tex(f[2]))
    if k == "s":
        n = int(f[1])
        return "%s\\parshape=%d %s" % (g, n, "1pt 2pt " * n)
    if k == "p":
        return "%s\\%s=%d %s " % (g, f[1], len(f) - 2, " ".join(f[2:]))
    if k == "b":
        return "%s\\setbox%s=\\box255 " % (g, f[1])
    sys.exit("op: " + op)


LONG = "A" * 40

# (mode, operations); e is e-TeX's extended mode, c is compatibility mode.
TRACE = [
    ("e", "beg;w count 1 5;end"),
    ("e", "beg;w dimen 1 3;end"),
    ("e", "w count 1 5;beg;w count 1 5;w count 1 6;w count 1 6;end"),
    ("e", "beg;W count 2 7;end"),
    ("e", "beg;w count 1 1;beg;w count 1 2;beg;w count 1 3;end;end;end"),
    ("e", "beg;w count 1 1;beg;W count 1 9;end;end"),
    ("e", "beg;W count 1 9;w count 1 3;end"),
    ("e", "w count 1 4;beg;w count 1 5;beg;W count 1 6;w count 1 7;end;w count 1 8;end"),
    ("e", "beg;w dimen 3 65536;w dimen 3 -65536;W dimen 4 1;end"),
    ("e", "beg;g skip 1 65536 131072 1 196608 0;g muskip 1 65536 131072 2 0 0;end"),
    ("e", "beg;g baselineskip 0 262144 0 0 0 0;g thinmuskip 0 131072 0 0 0 0;end"),
    ("e", "beg;g skip 2 0 0 0 0 0;end"),
    ("e", "g skip 1 65536 0 0 0 0;beg;g skip 1 131072 0 3 0 1;end"),
    ("e", "beg;G skip 3 65536 0 0 0 0;end"),
    ("e", "beg;d foo ab;d foo cd;l y foo;end"),
    ("e", "d foo ab;beg;d foo cd;end"),
    ("e", "beg;d x a;D x b;end"),
    ("e", "beg;D x b;d x a;end"),
    ("e", "d foo ab;beg;l foo foo;end"),
    ("e", "beg;d ~ ab;d a b;d . c;d NULLCS e;end"),
    ("e", "beg;l y relax;l y relax;end"),
    ("e", "beg;d x;l z x;d x;end"),
    ("e", "beg;h catcode 65 12;h lccode 65 5;h uccode 65 6;h sfcode 65 7;h mathcode 65 8;w delcode 65 9;end"),
    ("e", "beg;h catcode 97 11;h lccode 97 97;end"),
    ("e", "beg;H catcode 65 12;H sfcode 65 7;end"),
    ("e", "beg;t toks 1 abc;t toks 1 ;t toks 2 ;t everypar 0 x;q toks 3 toks 2;end"),
    ("e", "t toks 2 abc;beg;q toks 3 toks 2;t toks 2 de;T toks 4 fg;end"),
    ("e", "beg;s 1;s 2;s 0;s 0;end"),
    ("e", "beg;p interlinepenalties 100 200;p clubpenalties 100;p widowpenalties 0;p displaywidowpenalties 1 2 3;end"),
    ("e", "beg;f textfont 1;f scriptfont 2;f scriptscriptfont 3;f curfont 0;end"),
    ("e", "beg;F textfont 15;F scriptfont 15;F scriptscriptfont 15;end"),
    ("e", "beg;w tolerance 0 200;w hsize 0 3;w TeXXeTstate 0 1;w pdfpagewidth 0 65536;w pdfoutput 0 1;w synctex 0 1;end"),
    ("e", "beg;b 1;B 2;end"),
    ("e", "beg;d x " + LONG + ";end"),
    ("c", "beg;w count 1 5;end"),
    ("c", "w count 1 5;beg;w count 1 5;w count 1 5;end"),
    ("c", "beg;w count 1 1;beg;W count 1 9;end;end"),
    ("c", "d foo ab;beg;d foo ab;l y foo;end"),
    ("c", "beg;d x a;D x b;end"),
    ("c", "beg;s 1;t toks 1 abc;g skip 1 65536 131072 1 196608 0;end"),
]


def gen_trace(tmp):
    with open(os.path.join(OUT, "save_trace_oracle.tsv"), "wb") as f:
        for mode, ops in TRACE:
            lines = [trace_op_tex(o) for o in ops.split(";")]
            head = "\\tracingrestores=1 \\tracingassigns=1 " if mode == "e" else "\\tracingrestores=1 "
            body = head + "\n\\immediate\\write-1{BEGIN}\n" + "\n".join(lines) + "\n\\immediate\\write-1{END}"
            log = run_oracle(body, tmp, etex=mode == "e")
            m = re.search(r"^BEGIN$\n(.*?)^END$", log, re.M | re.S)
            if not m:
                sys.exit("trace %r: no result; see %s" % (ops, log))
            if re.search(r"^! ", m.group(1), re.M):
                sys.exit("trace %r: an error\n%s" % (ops, m.group(1)))
            f.write(("%s\t%s\t%s\n" % (mode, ops, field(m.group(1)))).encode("latin-1"))
    return len(TRACE)


def box_tex(f):
    # box KIND SPEC AMT CTX [ARG]
    kind, spec, amt, ctx = f[1], f[2], f[3], f[4]
    arg = f[5] if len(f) > 5 else ""
    prefix = {
        "none": "",
        "setbox": "\\setbox%s" % arg,
        "gsetbox": "\\global\\setbox%s" % arg,
        "moveleft": "\\moveleft %spt" % arg,
        "moveright": "\\moveright %spt" % arg,
        "raise": "\\raise %spt" % arg,
        "lower": "\\lower %spt" % arg,
        "shipout": "\\shipout",
        "leaders": "\\leaders",
        "cleaders": "\\cleaders",
        "xleaders": "\\xleaders",
    }[ctx]
    sp = {"none": "", "to": " to %spt" % amt, "spread": " spread %spt" % amt}[spec]
    return "%s\\%s%s{" % (prefix, kind, sp)


def group_op_tex(op):
    f = op.split(" ")
    k = f[0]
    return {
        "{": lambda: "{",
        "}": lambda: "}",
        "beg": lambda: "\\begingroup",
        "end": lambda: "\\endgroup",
        "S": lambda: "\\showgroups",
        "box": lambda: box_tex(f),
        "H": lambda: "\\noindent",
        "$": lambda: "$",
        "$$": lambda: "$$",
        "insert": lambda: "\\insert%s{" % (f[1] if len(f) > 1 else ""),
        "vadjust": lambda: "\\vadjust{",
        "disc": lambda: "\\discretionary{",
        "disc2": lambda: "}{",
        "choice": lambda: "\\mathchoice{",
        "choice2": lambda: "}{",
        "sup": lambda: "^{",
        "vcenter": lambda: "\\vcenter%s{" % ({"none": "", "to": " to %spt" % (f[2] if len(f) > 2 else ""), "spread": " spread %spt" % (f[2] if len(f) > 2 else "")}[f[1]]),
    }[k]()


# (\tracinggroups, operations)
GROUPS = [
    (0, "S"),
    (0, "beg;S;end;S"),
    (0, "{;S;};S"),
    (0, "beg;{;beg;S;end;S;};S;end;S"),
    (1, "beg;{;};end"),
    (1, "{;beg;end;}"),
    (1, "beg;beg;beg;end;end;end"),
    (0, "box hbox none 0 none;S"),
    (0, "box hbox to 5 setbox 1;S"),
    (0, "box hbox spread 3 gsetbox 2;S"),
    (0, "box vbox none 0 moveleft 2;S"),
    (0, "box vtop to 7 moveright 2;S"),
    (0, "box vbox none 0 shipout;S"),
    (0, "box vbox none 0 leaders;S"),
    (0, "box vbox none 0 cleaders;S"),
    (0, "box vbox none 0 xleaders;S"),
    (0, "H;box hbox none 0 raise 2;S"),
    (0, "H;box vbox spread 1 lower 2;S"),
    (0, "H;box hbox none 0 none;S"),
    (0, "box hbox to 5 setbox 1;box vbox spread 3 none;box vtop none 0 none;{;S"),
    (0, "box vbox none 0 none;box hbox none 0 none;box vbox none 0 none;S"),
    (0, "insert 100;S"),
    (0, "box vbox none 0 none;insert 100;S"),
    (0, "H;vadjust;S"),
    (0, "H;disc;S"),
    (0, "H;disc;disc2;S"),
    (0, "H;disc;disc2;disc2;S"),
    (0, "H;$;S"),
    (0, "$$;S"),
    (0, "H;$;sup;S"),
    (0, "H;$;choice;S"),
    (0, "H;$;choice;choice2;S"),
    (0, "H;$;choice;choice2;choice2;choice2;S"),
    (0, "H;$;vcenter none;S"),
    (0, "H;$;vcenter to 5;S"),
    (0, "H;$;vcenter spread 5;S"),
    (1, "H;$;box hbox none 0 none;S"),
    (1, "box hbox to 5 setbox 1;{;S"),
]


def gen_groups(tmp):
    with open(os.path.join(OUT, "save_groups_oracle.tsv"), "wb") as f:
        for tg, ops in GROUPS:
            head = "\\tracinggroups=1 " if tg else ""
            lines = [group_op_tex(o) for o in ops.split(";")]
            body = head + "\n" + "\n".join(lines) + "\n\\immediate\\write-1{ENDOFCASE}"
            first = PREAMBLE_LINES + 2
            log = run_oracle(body, tmp)
            # What \\end prints about groups left open is not part of the case.
            got = re.findall(r"^(### .*|\{(?:entering|leaving) .*\})$", log[: log.index("\nENDOFCASE\n")], re.M)
            f.write(("%d\t%d\t%s\t%s\n" % (first, tg, ops, field("\n".join(got)))).encode("latin-1"))
    return len(GROUPS)


def gen_limits(tmp):
    rows = []
    # Each level of \begingroup; the oracle says how many it allowed.
    body = (r"\count1=0 \def\r{\begingroup\global\advance\count1 by 1 \immediate\write-1{step \the\count1}\r}" "\n\\r")
    log = run_oracle(body, tmp)
    rows.append(("grouping levels", 0, log))
    body = (r"\count1=0 \def\r{\hbox\bgroup\global\advance\count1 by 1 \immediate\write-1{step \the\count1}\r}" "\n\\let\\bgroup={ \\r")
    log = run_oracle(body, tmp, cnf=["nest_size=40"])
    rows.append(("semantic nest size", 40, log))
    # Two words a level for the group, two for the saved value; the group limit is out of the way.
    body = (r"\count1=0 \def\r{\begingroup\global\advance\count2 by 1 \advance\count1 by 1 \immediate\write-1{step \the\count2}\r}" "\n\\r")
    log = run_oracle(body, tmp, cnf=["save_size=600"])
    rows.append(("save size", 600, log))
    with open(os.path.join(OUT, "save_limits_oracle.tsv"), "wb") as f:
        for what, limit, log in rows:
            steps = re.findall(r"^step (\d+)$", log, re.M)
            msg = re.findall(r"^! (TeX capacity exceeded.*)$", log, re.M)
            if len(msg) != 1 or what not in msg[0]:
                sys.exit("limits: %s: %r; see %s" % (what, msg, log[-2000:]))
            f.write(("%s\t%d\t%s\t%s\n" % (what, limit, steps[-1], msg[0])).encode("latin-1"))
    return len(rows)


def gen_mag(tmp):
    """prepare_mag: `true` units call it.  Each case is the \\mag values set, one per call."""
    cases = ["1000", "2000;2000", "2000;3000", "0", "-5", "32768", "32769", "2000;40000", "2000;3000;2000"]
    with open(os.path.join(OUT, "save_mag_oracle.tsv"), "wb") as f:
        for case in cases:
            lines = ["\\mag=%s \\dimen0=1truept \\immediate\\write-1{MAG=\\the\\mag}" % m for m in case.split(";")]
            log = run_oracle("\n".join(lines), tmp)
            got = re.findall(r"^(! .*| the previous value will be retained.*|MAG=.*)$", log, re.M)
            f.write(("%s\t%s\n" % (case, field("\n".join(got)))).encode("latin-1"))
    return len(cases)


def nest_op_tex(op):
    f = op.split(" ")
    k = f[0]
    if k == "S":
        return "\\showlists"
    if k == "w":
        return word_tex(f[1], f[2], f[3])
    if k == "prevdepth":
        return "\\prevdepth=%ssp " % f[1]
    if k == "prevgraf":
        return "\\prevgraf=%s " % f[1]
    if k == "spacefactor":
        return "\\spacefactor=%s " % f[1]
    if k == "over":
        return "\\over "
    return group_op_tex(op)


# Operations, with one \showlists at the end of each: what follows it would only obscure the case.
NEST = [
    "S",
    "prevdepth 65536;S",
    "prevgraf 1;S",
    "prevgraf 3;prevdepth 65536;S",
    "H;S",
    "w language 0 3;H;S",
    "w lefthyphenmin 0 2;w righthyphenmin 0 3;w language 0 1;H;S",
    # plain's values, with language 0: show_activities prints nothing about them.
    "w lefthyphenmin 0 2;w righthyphenmin 0 3;H;S",
    "w lefthyphenmin 0 0;w righthyphenmin 0 100;H;S",
    "w lefthyphenmin 0 1;w righthyphenmin 0 63;w language 0 0;H;S",
    "H;spacefactor 2000;S",
    "box hbox none 0 none;S",
    "box hbox none 0 none;spacefactor 2000;S",
    "box vbox none 0 none;S",
    "box vbox none 0 none;prevdepth 65536;prevgraf 2;S",
    "box hbox none 0 none;box vbox none 0 none;S",
    "H;$;S",
    "$$;S",
    "H;$;over;S",
    "H;$;box vbox none 0 none;S",
]


def gen_nest(tmp):
    keep = re.compile(r"^(?:### .*|prevdepth .*|spacefactor .*|this will begin denominator of:|total height .*| goal height .*|\\insert\d+ adds .*)$", re.M)
    with open(os.path.join(OUT, "nest_oracle.tsv"), "wb") as f:
        for ops in NEST:
            lines = [nest_op_tex(o) for o in ops.split(";")]
            log = run_oracle("\n".join(lines) + "\n\\immediate\\write-1{ENDOFCASE}", tmp)
            got = [m.group(0) for m in keep.finditer(log[: log.index("\nENDOFCASE\n")])]
            f.write(("%d\t%s\t%s\n" % (PREAMBLE_LINES + 1, ops, field("\n".join(got)))).encode("latin-1"))
    # The status of the current page, §1163: the second \\showlists is in the output routine.
    page = r"""\vsize=100pt \maxdepth=0pt \topskip=0pt \count100=500 \dimen100=1000pt \skip100=0pt
\count101=1000 \dimen101=1000pt \skip101=0pt
\count102=1000 \dimen102=1pt \skip102=0pt
\insert100{\hrule height 5pt}
\insert101{\hrule height 6pt}
\insert102{\hrule height 1pt\penalty0 \hrule height 4pt}
\hrule height 1pt
\vskip 1pt plus 1pt minus 5pt
\hrule height 1pt
\showlists
\output={\showlists\immediate\write-1{ENDOFCASE}\end}
\penalty-10000
\hrule height 2pt
"""
    log = run_oracle(page, tmp)
    log = log[: log.index("\nENDOFCASE\n")]
    a, b = log.split("\n! OK.", 1)
    with open(os.path.join(OUT, "nest_page_oracle.tsv"), "wb") as f:
        for part in (a, b):
            f.write((field("\n".join(m.group(0) for m in keep.finditer(part))) + "\n").encode("latin-1"))
    # print_in_mode: the tail of the message for a command not allowed there, with the Rust mode it is in.
    in_mode = [
        ("vmode", "\\vadjust"),
        ("hmode", "\\noindent\\moveleft"),
        ("-vmode", "\\vbox{\\raise"),
        ("-hmode", "\\hbox{\\moveleft"),
        ("-mmode", "\\noindent$\\moveleft"),
        ("mmode", "$$\\moveleft"),
    ]
    with open(os.path.join(OUT, "nest_in_mode_oracle.tsv"), "wb") as f:
        for mode, src in in_mode:
            log = run_oracle(src + " 1pt", tmp)
            m = re.search(r"^! You can't use `[^']*('.*)\.$", log, re.M)
            if not m:
                sys.exit("in_mode %r: no message: %s" % (src, log[-800:]))
            f.write(("%s\t%s\n" % (mode, m.group(1))).encode("latin-1"))
    return len(NEST) + len(in_mode)


# The MLTeX primitives are not in the oracle without -mltex; print_param keeps their names, from the WEB.
MLTEX = {"charsubdefmin", "charsubdefmax", "tracingcharsubdef"}


def gen_names(tmp):
    sys.path.insert(0, os.path.join(ROOT, "tools"))
    import web as weblib
    import webgen
    w = weblib.Web()
    ev = webgen.Evaluator(w)
    src = open(os.path.join(ROOT, "refs", "merged", "pdftex-merged.web"), encoding="latin-1").read()
    found = re.findall(r'primitive\("([A-Za-z]+)"\s*,\s*(assign_int|assign_dimen|assign_glue|assign_mu_glue)\s*,\s*([^;)]*)\)', src)
    base = {"assign_int": "int_base", "assign_dimen": "dimen_base", "assign_glue": "glue_base", "assign_mu_glue": "glue_base"}
    body = []
    for name, _, _ in found:
        if name not in MLTEX:
            body.append("\\show\\" + name)
    log = run_oracle("\n".join(body), tmp)
    shown = dict(re.findall(r"^> \\([A-Za-z]+)=(\\[A-Za-z]+)\.$", log, re.M))
    with open(os.path.join(OUT, "eqtb_names_oracle.tsv"), "w") as f:
        for name, cmd, chr_ in found:
            if name in MLTEX:
                printed = "\\" + name
            elif name in shown:
                printed = shown[name]
            else:
                sys.exit("names: %s is not shown; see %s" % (name, log[-800:]))
            f.write("%s\t%s\t%d\t%s\n" % (name, cmd, ev.eval(chr_) - ev.eval(base[cmd]), printed))
    return len(found)


# A kind of control sequence, and how to get a token of it into \y.
PRINT_CS = [
    ("active", "\\def\\y{~}"),
    ("letter", "\\def\\y{\\a}"),
    ("nonletter", "\\def\\y{\\.}"),
    ("unprintable", "\\def\\y{\\^^M}"),
    ("nullcs", "\\expandafter\\def\\expandafter\\y\\expandafter{\\csname\\endcsname}"),
    ("multi", "\\def\\y{\\foo}"),
    # \pdfprimitive turns the token into its location in the prim_eqtb region.
    ("prim", "\\edef\\y{\\pdfprimitive\\relax}"),
]


def gen_print_cs(tmp):
    with open(os.path.join(OUT, "hash_print_cs_oracle.tsv"), "wb") as f:
        for kind, src in PRINT_CS:
            log = run_oracle(src + "\n\\show\\y", tmp)
            m = re.search(r"^> \\y=macro:\n->(.*)\.$", log, re.M)
            if not m:
                sys.exit("print_cs %s: %s" % (kind, log[-800:]))
            f.write(("%s\t%s\n" % (kind, m.group(1))).encode("latin-1"))
    return len(PRINT_CS)


def main():
    os.makedirs(OUT, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        print("trace:", gen_trace(tmp))
        print("groups:", gen_groups(tmp))
        print("limits:", gen_limits(tmp))
        print("mag:", gen_mag(tmp))
        print("nest:", gen_nest(tmp))
        print("names:", gen_names(tmp))
        print("print_cs:", gen_print_cs(tmp))


if __name__ == "__main__":
    main()
