#!/usr/bin/env python3
"""Expected output of print.rs, Part 5 of the WEB, taken from the oracle.

Run with python3 -I.  It writes, under crates/engine/testdata/, for crates/engine/tests/print.rs:

  print_oracle.tsv       what each case puts on the terminal, in the log and in a \\write file
  print_init_oracle.tsv  what `pdftex -ini -etex` holds before anything has happened: §4 initialize

print_oracle.tsv: one row `printable CONFIG LIST` for the table of a config that is not the default; `two H M
TEXT` for the time at the head of the log, §1537, the only place print_two shows; `wide TEXT` for a \\number that
needs the oracle's 64 bits; and for each case `case CONFIG NAME OPS LOG TERM FILE`.  A config is one run of the
oracle:

  default  plain ASCII is printable             mpl60, mpl200  max_print_line=60, 200
  eight    -8bit                                cp227          -translate-file=cp227.tcx
  batch    -interaction=batchmode               first          the case is on the first line, before the log opens

OPS, separated by spaces, each what the test replays through the Rust print routines:

  nl:N esc:N          \\newlinechar=N  \\escapechar=N
  w:S:BYTES           \\immediate\\write S{...}, the bytes as tokens of category 12: token_show, §58-§59, §1623
  msg:BYTES           \\message{...}: §1457, the path of message_printing
  cs:S:BYTES          the control sequence of that name in a \\write: print_cs, §284
  str:S:BYTES         \\string of it in a \\write: sprint_cs, §285, and str_toks; so selector=new_string
  num:S:N             \\number N in a \\write: print_int          roman:S:N  \\romannumeral N: print_roman_int
  scaled:S:N          \\the of a \\dimen of N sp: print_scaled
  char:S:N mchar:S:N  \\meaning of a \\chardef, \\mathchardef: print_hex
  mode:M              \\batchmode or \\scrollmode: §1443
  show:S:N            \\showstream=S \\showthe of a \\count of N: §1475, §1932

BYTES is ALL for the 256 characters in order, or hex with runs, like 61*300,01.  LOG, TERM and FILE are escaped:
\\n for a newline, \\t for a tab, \\\\ for a backslash, \\xHH for what is not plain ASCII.

Each case starts with both offsets 0, after a line of the oracle's own, and ends with the print_nl("") of the
next \\write.  So this holds the bytes between two lines of the oracle's output, exactly.
"""
import os
import re
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
TESTDATA = os.path.join(ROOT, "crates", "engine", "testdata")

PREAMBLE = r"""\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \catcode`\^=7 \catcode0=12 \newlinechar=-1
\def\B#1{\begingroup\lccode`\?=#1 \lowercase{\endgroup\xdef\out{\out ?}}}
\def\Bz{\xdef\out{\out ^^@}}
\immediate\openout3=o3.out
"""
# The same on one line, for the first line of a run.
FIRST_PREAMBLE = PREAMBLE.replace("\n", " ").replace(r"\immediate\openout3=o3.out", "")

ALL = bytes(range(256))
ALL_CONFIGS = ("default", "mpl60", "mpl200", "eight", "cp227", "batch")
EDGE = [0, 1, -1, 9, 10, -9, 99, 100, 12345, 99999999, 100000000, 100000001, -99999999, -100000000, -100000001,
        -100000009, -100000010, 2147483647, -2147483647, 1073741824, -1073741824]


def h(b):
    """The bytes of an op: ALL, or runs like 61*300,01 in hex."""
    if b == ALL:
        return "ALL"
    out = []
    i = 0
    while i < len(b):
        j = i
        while j < len(b) and b[j] == b[i]:
            j += 1
        out.append("%02x*%d" % (b[i], j - i) if j - i > 1 else "%02x" % b[i])
        i = j
    return ",".join(out)


def payload(b):
    """Makes \\out hold these bytes as tokens of category 12."""
    return r"\gdef\out{}" + "".join(r"\Bz " if c == 0 else r"\B{%d}" % c for c in b)


def tex_op(op, first=False):
    k = op[0]
    if k == "nl":
        return r"\newlinechar=%d " % op[1]
    if k == "esc":
        return r"\escapechar=%d " % op[1]
    if k == "w":
        return payload(op[2]) + r"\immediate\write%d{\out}" % op[1]
    if k == "msg":
        return payload(op[1]) + r"\message{\out}"
    if k == "cs":
        return payload(op[2]) + r"\immediate\write%d{\csname\out\endcsname}" % op[1]
    if k == "str":
        return payload(op[2]) + r"\immediate\write%d{\expandafter\string\csname\out\endcsname}" % op[1]
    if k == "num":
        return r"\immediate\write%d{\number%d}" % (op[1], op[2])
    if k == "roman":
        return r"\immediate\write%d{\romannumeral%d}" % (op[1], op[2])
    if k == "scaled":
        return r"\dimen1=%dsp \immediate\write%d{\the\dimen1}" % (op[2], op[1])
    if k == "char":
        return r"\chardef\c=%d \immediate\write%d{\meaning\c}" % (op[2], op[1])
    if k == "mchar":
        return r"\mathchardef\c=%d \immediate\write%d{\meaning\c}" % (op[2], op[1])
    if k == "mode":
        return "\\" + op[1] + "mode "
    if k == "show":
        return r"\showstream=%d \count1=%d \showthe\count1 \showstream=-1 " % (op[1], op[2])
    raise ValueError(k)


def show_op(op):
    k = op[0]
    if k in ("nl", "esc"):
        return "%s:%d" % (k, op[1])
    if k in ("w", "cs", "str"):
        return "%s:%d:%s" % (k, op[1], h(op[2]))
    if k == "msg":
        return "msg:" + h(op[1])
    if k in ("num", "roman", "scaled", "char", "mchar", "show"):
        return "%s:%d:%d" % (k, op[1], op[2])
    if k == "mode":
        return "mode:" + op[1]
    raise ValueError(k)


def cases():
    out = []

    def add(name, ops, only=("default", "batch"), file=False):
        out.append((name, ops, set(only), file))

    bytes_ = ("default", "eight", "cp227")  # where the table of printable characters matters
    lines = ("default", "mpl60", "mpl200")  # where max_print_line matters
    wide = lines + ("batch",)

    # Every character, on each target, with each kind of new-line character.
    add("all_log", [("w", -1, ALL)], only=bytes_ + ("batch",))
    add("all_both", [("w", 16, ALL)], only=bytes_ + ("mpl60",))
    add("all_file", [("w", 3, ALL)], only=bytes_, file=True)
    add("all_msg", [("msg", ALL)], only=bytes_ + ("mpl60",))
    for nl in (0, 10, 13, 65, 94, 97, 128, 255, 256, -2):
        add("all_nl%d" % nl, [("nl", nl), ("w", 16, ALL)], only=("default",))
    add("all_nl10_both", [("nl", 10), ("w", 16, ALL)], only=("eight", "cp227"))
    add("all_nl10_log", [("nl", 10), ("w", -1, ALL)], only=("default",))
    add("all_nl10_file", [("nl", 10), ("w", 3, ALL)], only=("default",), file=True)
    add("all_nl10_msg", [("nl", 10), ("msg", ALL)], only=bytes_)
    # Line breaking at max_print_line, around each config's limit.
    for n in (59, 60, 61, 78, 79, 80, 199, 200, 201, 300):
        add("long_both_%d" % n, [("w", 16, b"x" * n)], only=lines)
        add("long_log_%d" % n, [("w", -1, b"y" * n)], only=lines)
    add("long_batch", [("w", 16, b"x" * 200), ("w", -1, b"y" * 100)], only=("batch",))
    add("long_file", [("w", 3, b"z" * 300)], only=lines, file=True)
    add("long_msg", [("msg", b"w" * 300)], only=lines)
    # A ^^ notation across the end of a line.
    for k in (57, 58, 59, 76, 77, 78, 79):
        add("wrap_caret_%d" % k, [("w", 16, b"x" * k + b"\x01" + b"y" * 90)], only=lines)
    add("wrap_hex", [("w", 16, b"x" * 77 + b"\xab" + b"y" * 90)], only=lines + ("eight",))
    # The offsets of the terminal and the log part company.
    add("mix", [("msg", b"abc"), ("w", -1, b"def"), ("msg", b"ghi"), ("w", 16, b"jkl"), ("msg", b"mno"), ("w", 3, b"pqr"), ("msg", b"stu")], only=wide, file=True)
    add("desync", [("msg", b"x" * 50), ("w", -1, b"y" * 70), ("msg", b"z" * 25), ("w", -1, b"y" * 70), ("msg", b"w" * 27), ("msg", b"v" * 3)], only=wide)
    # §1458's test.
    add("msg_fill", [("msg", b"0123456789")] * 30, only=wide)
    for n in (5, 6, 7, 8):
        add("msg_edge_%d" % n, [("msg", b"m" * 70), ("msg", b"n" * n), ("msg", b"o")], only=lines)
    add("msg_edge_60", [("msg", b"m" * 50), ("msg", b"n" * 7), ("msg", b"n" * 8), ("msg", b"o")], only=lines)
    add("msg_nl", [("nl", 10), ("msg", b"ab\ncd"), ("msg", b"e\nf"), ("msg", b"\n")], only=wide)
    add("msg_nonprint", [("msg", b"\x01\x80x"), ("msg", b"\xff"), ("msg", b"\x7f" * 30), ("msg", b"\x00" * 30)], only=bytes_ + ("mpl60",))
    add("msg_nl_nonprint", [("nl", 1), ("msg", b"a\x01b"), ("nl", 128), ("msg", b"c\x80d")], only=bytes_)
    add("msg_empty", [("msg", b""), ("msg", b"a"), ("msg", b"")])
    # The escape character, the three kinds of control sequence, and their names.
    for e in (92, -1, 255, 256, 65, 1, 128, 0, 32):
        add("esc_%d" % e, [("esc", e), ("cs", 16, b"abc"), ("cs", 16, b"a"), ("cs", 16, b""), ("cs", 16, b"."), ("str", 16, b"abc"), ("str", 16, b"a"), ("str", 16, b""), ("str", 16, b".")], only=bytes_)
    add("esc_is_nl", [("nl", 10), ("esc", 10), ("cs", 16, b"abc"), ("str", 16, b"abc"), ("cs", -1, b".")], only=bytes_)
    add("cs_bytes", [("cs", 16, b"\x80a\x01"), ("cs", -1, b"\x80"), ("cs", 3, b"\xff\x01x"), ("str", 16, b"\x80a\x01"), ("str", 3, b"\x01"), ("str", -1, b"a b")], only=bytes_ + ("batch",), file=True)
    add("cs_nl", [("nl", 97), ("cs", 16, b"banana"), ("str", 16, b"banana"), ("cs", -1, b"a"), ("cs", 3, b"cab")], only=bytes_, file=True)
    # The number printers.
    add("num", [("num", -1, n) for n in EDGE])
    add("roman", [("roman", -1, n) for n in (0, -5, 1, 2, 3, 4, 5, 6, 8, 9, 10, 14, 19, 40, 49, 90, 99, 400, 444, 900, 999, 1994, 3999, 4000, 4999, 5000, 12345)])
    add("roman_long", [("roman", 16, 100000), ("roman", -1, 300999)], only=wide)
    add("scaled", [("scaled", -1, n) for n in (0, 1, -1, 2, 65535, 65536, 65537, -65536, 32768, 1073741823, -1073741823, 186467)])
    add("hex", [("char", -1, n) for n in (0, 1, 9, 10, 15, 16, 160, 171, 255)] + [("mchar", 16, n) for n in (0, 255, 256, 4095, 4096, 32767, 0x7ABC)])
    # Numbers, with a digit as the new-line character.
    add("num_nl", [("nl", 53), ("num", 16, 1525354), ("roman", 16, 16), ("scaled", 16, 98304)])
    # selector, through the mode.
    add("batch", [("w", 16, b"before"), ("mode", "batch"), ("w", 16, b"in batch"), ("w", -1, b"log"), ("msg", b"m1"), ("msg", b"m2"), ("cs", 16, b"abc"), ("mode", "scroll"), ("w", 16, b"after")], only=lines)
    add("mode_mid_line", [("msg", b"on the line"), ("mode", "scroll"), ("msg", b"next"), ("w", -1, b"only here")], only=lines)
    # \showstream: print_nl on a file always begins a line.
    add("show", [("show", 3, 5), ("show", 3, -12345), ("w", 3, b"x"), ("show", 3, 0), ("w", 16, b"y")], file=True)
    add("show_nl", [("nl", 53), ("show", 3, 1525)], file=True)
    return out


FIRST_CASES = [
    ("first_short", [("w", 16, b"short"), ("msg", b"m"), ("msg", b"n")]),
    ("first_long", [("w", 16, b"x" * 200), ("w", -1, b"y" * 200), ("msg", b"z" * 80)]),
    ("first_bytes", [("w", 16, b"\x01\x80\xff"), ("nl", 10), ("w", 16, b"x\ny")]),
]

CONFIGS = {
    "default": ({}, []),
    "mpl60": ({"max_print_line": "60"}, []),
    "mpl200": ({"max_print_line": "200"}, []),
    "eight": ({}, ["-8bit"]),
    "cp227": ({}, ["-translate-file=cp227.tcx"]),
    "batch": ({}, ["-interaction=batchmode"]),
}


def run(tex_source, envadd, args, tmp, first_line=None):
    env = dict(os.environ, **envadd)
    if first_line is None:
        with open(os.path.join(tmp, "t.tex"), "wb") as f:
            f.write(tex_source.encode("latin-1"))
        target = "t.tex"
    else:
        target = first_line
    cmd = ["pdftex", "-ini", "-etex"] + args
    if not any(a.startswith("-interaction") for a in args):
        cmd.append("-interaction=nonstopmode")
    cmd.append(target)
    p = subprocess.run(cmd, cwd=tmp, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, check=False)
    return p.stdout


def read(tmp, name):
    p = os.path.join(tmp, name)
    if not os.path.exists(p):
        return b""
    with open(p, "rb") as f:
        return f.read()


def slice_of(data, name):
    begin = b"<<<" + name + b">>>\n"
    end = b">>>" + name + b"<<<\n"
    a = data.find(begin)
    if a < 0:
        return None
    a += len(begin)
    b = data.find(end, a)
    if b < 0:
        sys.exit("no end marker for %r" % name)
    return data[a:b]


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


def run_config(config, selected):
    envadd, args = CONFIGS[config]
    src = [PREAMBLE]
    for name, ops, _, file in selected:
        src.append(r"\immediate\write16{<<<%s>>>}" % name)
        if file:
            src.append(r"\immediate\write3{<<<%s>>>}" % name)
        src.append(r"\begingroup")
        src.extend(tex_op(op) for op in ops)
        src.append(r"\endgroup")
        src.append(r"\immediate\write16{>>>%s<<<}" % name)
        if file:
            src.append(r"\immediate\write3{>>>%s<<<}" % name)
    src.append(r"\immediate\closeout3 \end")
    with tempfile.TemporaryDirectory() as tmp:
        term = run("\n".join(src) + "\n", envadd, args, tmp)
        log = read(tmp, "t.log")
        o3 = read(tmp, "o3.out")
    rows = []
    for name, ops, _, file in selected:
        nm = name.encode("latin-1")
        lg = slice_of(log, nm)
        if lg is None:
            sys.exit("%s: no %s in the log\n%s" % (config, name, log[-1500:].decode("latin-1")))
        tm = slice_of(term, nm)
        if tm is None:
            tm = b""
        fl = slice_of(o3, nm) if file else b""
        if lg.startswith(b"!") or b"\n! " in lg:
            sys.exit("%s: an error in case %s\n%s" % (config, name, lg.decode("latin-1")))
        rows.append(["case", config, name, " ".join(show_op(o) for o in ops), esc(lg), esc(tm), esc(fl)])
    return rows


def run_first():
    """The case is on the first line: selector=term_only, and no log yet."""
    rows = []
    for name, ops in FIRST_CASES:
        nm = name.encode("latin-1")
        line = (FIRST_PREAMBLE + r"\immediate\write16{<<<%s>>>}\begingroup " % name + "".join(tex_op(op) for op in ops)
                + r"\endgroup\immediate\write16{>>>%s<<<}\end" % name)
        with tempfile.TemporaryDirectory() as tmp:
            term = run("", {}, [], tmp, first_line=line)
        tm = slice_of(term, nm)
        if tm is None:
            sys.exit("first line: no %s on the terminal\n%s" % (name, term.decode("latin-1")))
        rows.append(["case", "first", name, " ".join(show_op(o) for o in ops), "", esc(tm), ""])
    return rows


def cp227_printable():
    p = subprocess.run(["kpsewhich", "--format=web2c files", "cp227.tcx"], stdout=subprocess.PIPE, check=True).stdout.decode().strip()
    out = set(range(32, 127))
    with open(p, encoding="latin-1") as f:
        for line in f:
            line = line.split("%")[0].split()
            if not line:
                continue
            src = int(line[0], 0)
            printable = int(line[2], 0) if len(line) > 2 else 1
            if printable:
                out.add(src)
            else:
                out.discard(src)
    return sorted(out)


def run_two():
    """print_two, through the date in the first line of the log, with the clock pinned."""
    rows = []
    for hh, mm in [(0, 0), (0, 5), (9, 59), (10, 0), (12, 34), (23, 59)]:
        epoch = 1700000000 - 1700000000 % 86400 + hh * 3600 + mm * 60
        with tempfile.TemporaryDirectory() as tmp:
            run("\\end\n", {"SOURCE_DATE_EPOCH": str(epoch), "FORCE_SOURCE_DATE": "1"}, [], tmp)
            first = read(tmp, "t.log").split(b"\n")[0].decode("latin-1")
        rows.append(["two", str(hh), str(mm), first.split()[-1]])
    return rows


def run_wide():
    """What \\number gives where the oracle's integer has left 32 bits; the port cannot make these, but print_int can print them."""
    src = [r"\catcode`\{=1 \catcode`\}=2 \count1=2147483647 \advance\count1 by 1 \immediate\write-1{=\number\count1}",
           r"\count1=-2147483647 \advance\count1 by -1 \immediate\write-1{=\number\count1}",
           r"\count1=2147483647 \advance\count1 by 2147483647 \advance\count1 by 2147483647 \immediate\write-1{=\number\count1}",
           r"\count1=-2147483647 \advance\count1 by -2147483647 \advance\count1 by -2147483647 \immediate\write-1{=\number\count1}",
           r"\end"]
    with tempfile.TemporaryDirectory() as tmp:
        run("\n".join(src) + "\n", {}, [], tmp)
        log = read(tmp, "t.log").decode("latin-1")
    return [["wide", line[1:]] for line in log.split("\n") if line.startswith("=")]


def gen_print():
    rows = [["printable", "cp227", ",".join(str(c) for c in cp227_printable())]]
    rows += run_two()
    rows += run_wide()
    allcases = cases()
    for config in CONFIGS:
        rows += run_config(config, [c for c in allcases if config in c[2]])
    rows += run_first()
    path = os.path.join(TESTDATA, "print_oracle.tsv")
    with open(path, "w", encoding="ascii", newline="\n") as f:
        for r in rows:
            f.write("\t".join(r) + "\n")
    print("%s: %d rows, %d bytes" % (path, len(rows), os.path.getsize(path)))


# ---- §4 ----


def const_values():
    out = {}
    with open(os.path.join(ROOT, "crates", "engine", "src", "consts.rs"), encoding="utf-8") as f:
        for m in re.finditer(r"pub const ([A-Z_0-9a-z]+): i32 = (-?\d+);", f.read()):
            out[m.group(1)] = int(m.group(2))
    return out


def gen_init():
    c = const_values()
    kinds = {"assign_int": "int", "assign_dimen": "dimen", "assign_glue": "glue", "assign_mu_glue": "muglue", "assign_toks": "toks"}
    prims = []
    with open(os.path.join(ROOT, "refs", "merged", "pdftex-merged.web"), encoding="latin-1") as f:
        web = f.read()
    for m in re.finditer(r'primitive\("(\w+)",(assign_int|assign_dimen|assign_glue|assign_mu_glue|assign_toks),(\w+)(?:\+(\w+))?\)', web):
        name, cmd, base, code = m.groups()
        # \\time and the like are set by §241 from the clock, not by initialize.  MLTeX is dropped.
        if name in ("time", "day", "month", "year", "charsubdefmin", "charsubdefmax", "tracingcharsubdef"):
            continue
        loc = c[base.upper()] + (c[code.upper()] if code else 0)
        prims.append((name, kinds[cmd], loc))
    tables = ["catcode", "lccode", "uccode", "sfcode", "mathcode", "delcode"]
    src = [r"\catcode`\{=1 \catcode`\}=2"]
    src2 = []
    for t in tables:
        for k in range(256):
            src2.append(r"\immediate\write-1{=%s %d \number\%s%d}" % (t, k, t, k))
    for name, kind, loc in prims:
        if kind in ("int", "dimen"):
            src2.append(r"\immediate\write-1{=%s \number\%s}" % (name, name))
        elif kind == "toks":
            src2.append(r"\immediate\write-1{=%s [\the\%s]}" % (name, name))
        else:
            src2.append(r"\immediate\write-1{=%s \the\%s}" % (name, name))
    src2.append(r"\end")
    with tempfile.TemporaryDirectory() as tmp:
        # The braces are the first thing the source changes, and neither has a table entry that matters here
        # but \catcode: the test allows for those two.
        run("\n".join(src + src2) + "\n", {"max_print_line": "1000"}, [], tmp)
        log = read(tmp, "t.log").decode("latin-1")
    seen = {}
    for line in log.split("\n"):
        if line.startswith("="):
            k, _, v = line[1:].partition(" ")
            seen.setdefault(k, []).append(v)
        elif line.startswith("! "):
            sys.exit("an error from the oracle: " + line)
    rows = []
    for t in tables:
        vals = []
        for s in seen[t]:
            vals.append(s.split()[1])
        rows.append(["table", t, ",".join(vals)])
    for name, kind, loc in prims:
        rows.append(["par", name, kind, str(loc), seen[name][0]])
    path = os.path.join(TESTDATA, "print_init_oracle.tsv")
    with open(path, "w", encoding="ascii", newline="\n") as f:
        for r in rows:
            f.write("\t".join(r) + "\n")
    print("%s: %d rows, %d bytes" % (path, len(rows), os.path.getsize(path)))


if __name__ == "__main__":
    gen_print()
    gen_init()
