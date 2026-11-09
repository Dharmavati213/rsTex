#!/usr/bin/env python3
"""Expected results for the unit tests of etex.rs, taken from the oracle.

Run with python3 -I.  It writes, under crates/engine/testdata/:

  etex_expr_oracle.tsv     kind, x, y, z, then what \\number gives for that expression and 1 if the oracle said
                           "Arithmetic overflow".  The kinds are add_or_sub, quotient and fract, for \\numexpr
                           (limit infinity) and \\dimexpr (limit max_dimen).
  etex_sa_oracle.tsv       operations on sparse registers, then the \\tracingassigns and \\tracingrestores lines
  etex_pseudo_oracle.tsv   the text of a \\scantokens, \\newlinechar, then the lines the pseudo file gives,
                           each followed by a | that \\endlinechar put there

Each case runs in `pdftex -ini -etex`, in a temporary directory.  The tests read the same input and call the Rust
routines directly.

Format: one case per line, fields separated by a tab.  In etex_sa_oracle.tsv operations are separated by `;`,
their arguments by a space, and a newline in the expected output is `|`.
"""
import os
import random
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

INF = 2147483647
MAXD = 1073741823
CHUNK = 60  # TeX stops after 100 errors in one paragraph


def run_oracle(body, tmp):
    path = os.path.join(tmp, "t.tex")
    with open(path, "wb") as f:
        f.write((PREAMBLE + body + "\n\\end\n").encode("latin-1"))
    env = dict(os.environ, max_print_line="100000", error_line="254", half_error_line="238")
    cmd = ["pdftex", "-ini", "-etex", "-interaction=nonstopmode", "t.tex"]
    subprocess.run(cmd, cwd=tmp, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
    with open(os.path.join(tmp, "t.log"), "rb") as f:
        return f.read().decode("latin-1")


def expr_rows():
    rng = random.Random(1791)
    rows = []
    add = [0, 1, -1, 1073741823, 1073741824, 2147483646, INF, -INF]
    for kind in ("add", "sub"):
        rows += [(kind, x, y, 0) for x in add for y in add]
    dim = [0, 1, -1, 536870912, MAXD - 1, MAXD, -MAXD]
    for kind in ("dadd", "dsub"):
        rows += [(kind, x, y, 0) for x in dim for y in dim]
    num = [0, 1, -1, 3, 7, 65536, 1073741824, 2147483646, INF, -INF]
    den = [0, 1, -1, 2, -2, 3, 7, 1073741824, INF, -INF]
    rows += [("div", x, y, 0) for x in num for y in den]
    rows += [("ddiv", x, y, 0) for x in [0, 1, -1, 3, 65536, MAXD, -MAXD] for y in [0, 1, -1, 2, 3, 7, INF]]
    edge = [0, 1, -1, 2, 3, 7, 10, 100, 46340, 46341, 65536, 715827882, 1073741823, 1073741824, 2147483646, INF, -INF]
    for _ in range(170):
        rows.append(("scale", rng.choice(edge), rng.choice(edge), rng.choice(edge)))
    for _ in range(90):
        bits = [rng.randint(0, 31) for _ in range(3)]
        x, n, d = [rng.randint(-(1 << b), (1 << b) - 1) for b in bits]
        rows.append(("scale", x, n, d))
    dedge = [0, 1, -1, 3, 65536, 32768, 536870912, MAXD]
    for _ in range(60):
        rows.append(("dscale", rng.choice(dedge), rng.choice(edge), rng.choice(edge)))
    for _ in range(40):
        x = rng.randint(-MAXD, MAXD)
        n, d = rng.randint(-(1 << 20), 1 << 20), rng.randint(-(1 << 20), 1 << 20)
        rows.append(("dscale", x, n, d))
    return rows


def expr_tex(kind, x, y, z):
    return {
        "add": r"\numexpr %d+%d\relax" % (x, y),
        "sub": r"\numexpr %d-%d\relax" % (x, y),
        "dadd": r"\dimexpr %dsp+%dsp\relax" % (x, y),
        "dsub": r"\dimexpr %dsp-%dsp\relax" % (x, y),
        "div": r"\numexpr %d/%d\relax" % (x, y),
        "ddiv": r"\dimexpr %dsp/%d\relax" % (x, y),
        "scale": r"\numexpr %d*%d/%d\relax" % (x, y, z),
        "dscale": r"\dimexpr %dsp*%d/%d\relax" % (x, y, z),
    }[kind]


def gen_expr(tmp):
    rows = expr_rows()
    with open(os.path.join(OUT, "etex_expr_oracle.tsv"), "w") as f:
        for start in range(0, len(rows), CHUNK):
            part = rows[start:start + CHUNK]
            body = ["\\immediate\\write16{R:%d:\\number%s}" % (k, expr_tex(*r)) for k, r in enumerate(part)]
            log = run_oracle("\n".join(body), tmp)
            pieces = re.split(r"^R:(\d+):(-?\d+)$", log, flags=re.M)
            if len(pieces) != 3 * len(part) + 1:
                sys.exit("expr: %d results for %d cases; see %s" % ((len(pieces) - 1) // 3, len(part), tmp))
            for k, r in enumerate(part):
                assert int(pieces[3 * k + 1]) == k
                err = pieces[3 * k].count("! Arithmetic overflow.")
                if err > 1 or pieces[3 * k].count("\n! ") > err:
                    sys.exit("expr: case %r is another error; see %s" % (r, tmp))
                f.write("%s\t%d\t%d\t%d\t%s\t%d\n" % (r + (pieces[3 * k + 2], err)))
    return len(rows)


# Operations on sparse registers.  Lower case is local, upper case is \global.
#   beg, end          \begingroup, \endgroup
#   c N V, d N V      \countN=V, \dimenN=Vsp
#   s N W P, m N W P  \skipN=Wsp plus Psp; \muskipN, whose W and P are whole multiples of 65536
#   t N [TEXT]        \toksN={TEXT}
#   b N [W]           \setboxN=\hbox to Wsp{}, or \setboxN=\box0 when there is no W
SA_CASES = [
    "c 300 5",
    "beg;c 300 5;end",
    "c 300 5;beg;c 300 5;c 300 6;c 300 6;end",
    "beg;C 300 7;end",
    "beg;c 300 1;beg;c 300 2;beg;c 300 3;end;end;end",
    "beg;c 300 1;C 300 9;end",
    "beg;c 300 5;c 300 0;end",
    "c 300 5;beg;c 300 0;c 300 0;end",
    "c 300 0",
    "c 300 5;beg;c 301 6;beg;c 302 7;end;c 300 1;end",
    "beg;c 300 1;beg;C 300 2;end;end",
    "d 256 3;d 257 4;d 4095 5;d 4096 6;d 4097 7;d 32767 8",
    "c 256 3;c 257 4;c 4095 5;c 4096 6;c 4097 7;c 32767 8",
    "beg;d 300 3;D 301 4;end",
    "s 256 3 0;s 257 0 4;s 4095 5 6;s 4096 7 8;s 4097 9 0;s 32767 1 1",
    "s 300 0 0;s 300 0 0;s 300 5 0;s 300 0 0",
    "beg;s 300 65536 131072;end",
    "s 300 5 0;beg;s 300 6 0;s 300 7 0;end",
    "beg;s 300 65536 0;S 300 131072 0;end",
    "S 300 3 0;S 300 0 0",
    "m 256 196608 0;m 257 0 262144;m 4096 327680 393216;m 32767 458752 524288",
    "beg;m 300 65536 0;end;M 301 65536 0",
    "t 256 a;t 257 b;t 4095 c;t 4096 d;t 4097 e;t 32767 f",
    "t 300 abc;t 300;t 300 x",
    "beg;t 300 abc;end",
    "beg;t 300 abc;T 300 d;end",
    "t 300 abc;beg;t 300;end",
    "t 300 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "b 256 3;b 257 4;b 4095 5;b 4096 6;b 4097 7;b 32767 8",
    "b 300 5;b 300 7;beg;b 300 9;end;B 300 1",
    "beg;b 300 5;B 300 6;end",
    "b 300 5;b 300;b 300",
    "beg;b 300;end",
]


def sa_tex(op):
    f = op.split(" ")
    k = f[0].lower()
    g = "\\global" if f[0][0].isupper() and k not in ("beg", "end") else ""
    if k == "beg":
        return "\\begingroup"
    if k == "end":
        return "\\endgroup"
    if k == "c":
        return "%s\\count%s=%s " % (g, f[1], f[2])
    if k == "d":
        return "%s\\dimen%s=%ssp " % (g, f[1], f[2])
    if k == "s":
        return "%s\\skip%s=%ssp plus %ssp " % (g, f[1], f[2], f[3])
    if k == "m":
        return "%s\\muskip%s=%dmu plus %dmu " % (g, f[1], int(f[2]) // 65536, int(f[3]) // 65536)
    if k == "t":
        return "%s\\toks%s={%s}" % (g, f[1], f[2] if len(f) > 2 else "")
    if k == "b":
        return "%s\\setbox%s=%s" % (g, f[1], "\\hbox to%ssp{}" % f[2] if len(f) > 2 else "\\box0 ")
    sys.exit("unknown operation " + op)


def gen_sa(tmp):
    with open(os.path.join(OUT, "etex_sa_oracle.tsv"), "w") as f:
        for ops in SA_CASES:
            body = ["\\hbadness=10000 \\tracingassigns=1 \\tracingrestores=1"]
            body += [sa_tex(op) for op in ops.split(";")]
            body.append("\\tracingassigns=0 \\tracingrestores=0")
            log = run_oracle("\n".join(body), tmp)
            if "\n! " in log:
                sys.exit("sa: %r is an error; see %s" % (ops, tmp))
            lines = []
            open_record = False  # \\box shows its contents on the next line, before the closing brace
            for l in log.split("\n"):
                if open_record or (l.startswith("{") and "\\tracing" not in l and "\\hbadness" not in l):
                    lines.append(l)
                    open_record = not l.endswith("}")
            f.write("%s\t%s\n" % (ops, "|".join(lines)))
    return len(SA_CASES)


LONG = "abcdefghijklmnopqrstuvwxyz"
PSEUDO_CASES = [
    ("", 33), ("a", 33), ("ab", 33), ("abc", 33), ("abcd", 33), ("abcde", 33), ("abcdefgh", 33), ("abcdefghi", 33),
    ("a!b", 33), ("a!!b", 33), ("!a", 33), ("a!", 33), ("!", 33), ("!!", 33), ("a  !b  ", 33), ("   ", 33),
    ("a   ", 33), ("    a", 33), ("abcd    ", 33), ("abcd    !    ", 33), ("a!b", -1), ("a!b", 256), ("a!b", 0),
    ("a b c", 32), ("a  b ", 32), (" ", 32), ("  ", 32), (LONG + "!" + LONG + "!!", 33), (LONG * 4, 33),
    ("abc!abcd!abcde!abcdef!abcdefg!abcdefgh", 33),
]


def gen_pseudo(tmp):
    body = ["\\endlinechar=-1 \\catcode32=12 \\catcode`\\|=12 \\catcode`\\!=12"]
    for k, (text, nl) in enumerate(PSEUDO_CASES):
        body.append("\\begingroup\\newlinechar=%d \\endlinechar=124 \\everyeof{\\noexpand}\\xdef\\x{\\scantokens{%s}}\\endgroup" % (nl, text))
        body.append("\\immediate\\write16{B:%d:\\detokenize\\expandafter{\\x}:E}" % k)
    log = run_oracle("\n".join(body), tmp)
    got = re.findall(r"^B:(\d+):(.*):E$", log, re.M)
    if len(got) != len(PSEUDO_CASES):
        sys.exit("pseudo: %d results for %d cases; see %s" % (len(got), len(PSEUDO_CASES), tmp))
    with open(os.path.join(OUT, "etex_pseudo_oracle.tsv"), "w") as f:
        for (text, nl), (_, res) in zip(PSEUDO_CASES, got):
            f.write("%s\t%d\t%s\n" % (text, nl, res))
    return len(PSEUDO_CASES)


def main():
    os.makedirs(OUT, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        a = gen_expr(tmp)
        b = gen_sa(tmp)
        c = gen_pseudo(tmp)
    print("expr:", a, "cases; sparse arrays:", b, "cases; pseudo files:", c, "cases")


if __name__ == "__main__":
    main()
