#!/usr/bin/env python3
"""Expected results for the token machine's unit tests, taken from the oracle.

Run with python3 -I.  It writes, under crates/engine/testdata/:

  tokenize_oracle.tsv   source text, then the tokens it becomes, as \\detokenize prints them
  macrocall_oracle.tsv  a parameter text and an input, then the arguments as \\tracingmacros prints
                        them, and the tokens left over
  primitives.tsv        every primitive("name",cmd,chr) of the merged WEB, as numbers: print_cmd_chr
                        must print \\name for it.  This one comes from the WEB, not from the oracle.

Each case is typeset by `pdftex -ini -etex`, in a temporary directory.  The tests in getnext.rs and
macrocall.rs read the same source text through the Rust get_next and macro_call.

Format: one case per line, fields separated by a tab.  A newline inside a field is byte 0x0B.
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
\immediate\write16{ready}
"""

# Source text for get_next.  Multi-letter control sequences are marked: they need id_lookup.
TOKENIZE = [
    ("a b", 0), ("a   b", 0), ("  a", 0), ("a  ", 0), ("a%comment", 0), ("%x", 0),
    (r"\a b", 0), (r"\a   b", 0), (r"\. b", 0), (r"\.   b", 0), (r"\  b", 0), (r"\   b", 0),
    ("^^41", 0), ("^^41b", 0), ("^^4a", 0), ("^^4A", 0), ("^^61", 0), ("^^ff", 0),
    ("a^^", 0), ("x^^Mignored", 0), ("^^5cx", 0), ("^^5c x", 0), (r"\^^41", 0), (r"\^^41 x", 0), (r"\^^5c", 0),
    (r"\^^M", 0), (r"\^^Mx", 0), ("{a}", 0), ("{{a}b}", 0), ("$&^_~", 0), ("a#b", 0),
    ("a^^@b", 0), ("^^^41", 0), ("^^z", 0), ("^^ x", 0), ("^^41^^42", 0), ("^x", 0), ("^^", 0),
    ("a\n\nb", 0), ("a\n  b", 0), (r"\a" "\n b", 0), ("%\n%\nb", 0), ("a\n\n\n\nb", 0), ("\n\n", 0),
    (r"\.", 0), (r"\ ", 0), ("\\", 0), ("\\\nb", 0), (r"\a%" "\nb", 0), (r"a\%b", 0),
    (r"\foo bar", 1), (r"\foo", 1), (r"\foo.", 1), (r"\foo^^62 a", 1), (r"\fo^^6f", 1), (r"\foo^^Mx", 1),
    (r"\foo^^62^^63 a", 1), (r"\a^^62", 1), (r"\^^61b", 1), (r"\foo^^", 1), (r"\foo^^4a", 1), (r"\foo^^4", 1),
    (r"\foo\bar", 1), (r"\foo%" "\nb", 1), (r"\foo" "\n\nb", 1),
]

# (parameter text, input)
MACROS = [
    ("#1", "abc"), ("#1", " a"), ("#1", "{abc}d"), ("#1", "{}d"), ("#1", "{{a}}d"), ("#1", r"\x y"),
    ("#1#2", "{ab}{c}d"), ("#1#2", "ab"), ("#1#2", "a b"), ("#1#2", "a {b}c"),
    ("#1#2#3#4#5#6#7#8#9", "abcdefghij"),
    ("#1.", "a b{c}d.e"), ("#1.", "{abc}."), ("#1.", "{a}{b}."), ("#1.", "{a}b."), ("#1.", "a{b}."),
    ("#1.", "."), ("#1.", "{}."), ("#1.", "{{a}}."), ("#1.", "{a.b}."), ("#1.", " a."), ("#1.", "a ."),
    ("#1.", " {a} ."), ("#1.", " {a}."), ("#1.", "{a} ."),
    ("#1 ", "a b"), ("#1 ", " a b"), ("#1 ", "{a} b"), ("#1 #2 ", "a b c"), ("#1 #2.", "a b."),
    ("#1ab", "xaay abz"), ("#1aab", "aaabz"), ("#1aab", "xaaaab"), ("#1abab", "ababcabababz"),
    ("#1aba", "ababaz"), ("#1abcabd", "abcabcabdz"), ("#1aaa", "aaaaz"), ("#1aaa", "aabaaaz"),
    ("#1abc", "ababcz"), ("#1abab", "abaabab"), ("#1aabaac", "aabaabaacz"), ("#1abac", "ababacz"),
    ("#1ab", "{a}abz"), ("#1ab", "a{b}abz"), ("#1ab", "aab"), ("#1a b", "aa bz"), ("#1 a", "x a "),
    ("x#1", "xy"), ("xy#1.", "xy z.w"), ("xy", "xyz"), ("x", "xy"), ("x y", "x yz"), ("x#1y#2z", "xayb{c}zw"),
    (r"#1\x", r"ab\x"), (r"#1\x y", r"a\x yz"), (r"#1\x", r"\y\x"), (r"#1\x\y", r"a\x\z\x\y"), (r"\x#1", r"\x a"), (r"\.#1", r"\.a"),
    ("#1.#2", ".{x}"), ("#1.#2;", "a.b;rest"), ("#1.#2;", "a.;"), ("#1.#2;", ";.;"), ("#1..", "a.b.."), ("#1..", "..."),
    ("#1#2.", "a."), ("#1#2.", "{a}."), ("#1#2.", "{a}{b}."), ("#1#2.", "ab c."), ("#1.#2", "a.b"), ("#1.#2", "a. b"),
    ("#1", "$"), ("#1", "&"), ("#1", "^"), ("#1", "_"), ("#1", "~"), ("#1.", "a~b."), ("#1.", "$&^_~."),
    ("#1", r"\."), ("#1.", r"\.."), ("#1", r"\  a"), ("#1.", r"\x  y."),
]


def run_oracle(body, tmp):
    path = os.path.join(tmp, "t.tex")
    with open(path, "wb") as f:
        f.write((PREAMBLE + body + "\n\\csname bye\\endcsname\\end\n").encode("latin-1"))
    subprocess.run(["pdftex", "-ini", "-etex", "-interaction=nonstopmode", "-translate-file=cp227.tcx", "t.tex"],
                   cwd=tmp, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
    with open(os.path.join(tmp, "t.log"), "rb") as f:
        return f.read().decode("latin-1")


def field(s):
    return s.replace("\n", "\x0b")


def gen_tokenize(tmp):
    body = [r"\long\def\grab#1\endtest{\immediate\write16{BEGIN:\detokenize{#1}:END}}"]
    for text, _ in TOKENIZE:
        body.append("\\grab " + text + "\n\\endtest")
    log = run_oracle("\n".join(body), tmp)
    got = re.findall(r"^BEGIN:(.*?):END$", log, re.M | re.S)
    if len(got) != len(TOKENIZE):
        sys.exit("tokenize: %d results for %d cases; see the log in %s" % (len(got), len(TOKENIZE), tmp))
    with open(os.path.join(OUT, "tokenize_oracle.tsv"), "wb") as f:
        for (text, multi), res in zip(TOKENIZE, got):
            res = res.replace("\n", "")  # the line breaks at max_print_line are not tokens
            f.write(("%s\t%d\t%s\n" % (field(text), multi, res)).encode("latin-1"))


def gen_macros(tmp):
    body = [r"\long\def\grab#1\endtest{\immediate\write16{REST:\detokenize{#1}:END}}", r"\let\x=\relax \let\y=\relax \let\z=\relax"]
    for k, (param, inp) in enumerate(MACROS):
        body.append("\\def\\m %s{\\grab}\\immediate\\write16{CASE:%d:}\\tracingmacros=1 \\m %s\\endtest\\tracingmacros=0" % (param, k, inp))
    log = run_oracle("\n".join(body), tmp)
    parts = re.split(r"^CASE:(\d+):$", log, flags=re.M)
    if len(parts) != 2 * len(MACROS) + 1:
        sys.exit("macros: %d results for %d cases; see the log in %s" % ((len(parts) - 1) // 2, len(MACROS), tmp))
    with open(os.path.join(OUT, "macrocall_oracle.tsv"), "wb") as f:
        for k, (param, inp) in enumerate(MACROS):
            assert int(parts[2 * k + 1]) == k
            text = parts[2 * k + 2]
            if text.count("\n! ") or text.startswith("! "):
                sys.exit("macros: case %d (%r, %r) is an error; see %s" % (k, param, inp, tmp))
            # Only the lines of \m itself: \grab's expansion follows them.
            head = text.split("\n\\grab #1", 1)[0]
            args = re.findall(r"^#\d<-(.*)$", head, re.M)
            rest = re.findall(r"^REST:(.*?):END$", text, re.M | re.S)
            if len(rest) != 1:
                sys.exit("macros: case %d (%r, %r): no REST; see %s" % (k, param, inp, tmp))
            f.write(("\t".join([field(param), field(inp), str(len(args))] + args + [rest[0].replace("\n", "")]) + "\n").encode("latin-1"))


# Commands whose chr_code is printed by a routine of another module, or that has no name of its own.
OTHER_MODULES = {"assign_glue", "assign_mu_glue", "assign_int", "assign_dimen", "math_style", "def_family", "set_font"}
# MLTeX's primitives are dropped.
DROPPED = {"charsubdef", "charsubdefmin", "charsubdefmax", "tracingcharsubdef"}


def gen_primitives():
    sys.path.insert(0, os.path.join(ROOT, "tools"))
    import web as weblib
    import webgen
    w = weblib.Web()
    ev = webgen.Evaluator(w)
    src = open(os.path.join(ROOT, "refs", "merged", "pdftex-merged.web"), encoding="latin-1").read()
    # primitive("name",cmd,chr) may span lines.
    found = re.findall(r'primitive\("((?:[^"]|"")*)"\s*,\s*([A-Za-z_]+)\s*,\s*([^;)]*)\)', src)
    n = 0
    with open(os.path.join(OUT, "primitives.tsv"), "w") as f:
        for name, cmd, chr_ in found:
            name = name.replace('""', '"')
            if cmd in OTHER_MODULES or name in DROPPED:
                continue
            f.write("%s\t%d\t%d\n" % (name, ev.value(cmd), ev.eval(chr_)))
            n += 1
    return n


def main():
    os.makedirs(OUT, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        gen_tokenize(tmp)
        gen_macros(tmp)
    n = gen_primitives()
    print("tokenize:", len(TOKENIZE), "cases; macros:", len(MACROS), "cases; primitives:", n)


if __name__ == "__main__":
    main()
