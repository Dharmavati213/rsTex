#!/usr/bin/env python3
"""Expected results for pdffont.rs's map trees, taken from the oracle.

Run with python3 -I.  It writes crates/engine/testdata/pdffont_map_oracle.tsv: one row per operation of one run,

    op  argument  what the log got for it, with | for a newline

where op is `line` (\\pdfmapline), `suppress` (\\pdfsuppresswarningdupmap) or `use` (a font asks for its entry).  The
test in pdffont.rs does the same operations in the same order, on the same machine, and compares the text.
"""
import os
import re
import subprocess
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "engine", "testdata", "pdffont_map_oracle.tsv")

ROWS = [
    ("line", "x"),
    ("line", "+"), ("line", "="), ("line", "-"), ("line", "+# comment"), ("line", "+* comment"), ("line", "+; comment"),
    ("line", "+aa AA aa.pfb"),
    ("line", "+ <aa.pfb"),
    ("line", "+ab AB <8r.enc <<ab.ttf"),
    ("line", '+ac AC ".2 SlantFont"'),
    ("line", '+ad AD "1.5 ExtendFont"'),
    ("line", '+ae AE "1.5 SlantFont" <ae.pfb'),
    ("line", '+ae AE "-1.5 SlantFont" <ae.pfb'),
    ("line", '+af AF "2.5 ExtendFont" <af.pfb'),
    ("line", '+ag AG "1.5 SlantFont 2.5 ExtendFont" <ag.pfb'),
    ("line", '+ah AH "1 ExtendFont" <ah.pfb'),
    ("line", '+ai AI ".2 SlantFont" <ai.ttf'),
    ("line", "+aj AJ PidEid=3,1 <aj.pfb"),
    ("line", "+ak AK PidEid=3,1 <<ak.ttf"),
    ("line", "+al AL PidEid=3,1 <8r.enc <al.ttf"),
    ("line", "+am AM PidEid=3,1 <am.ttf"),
    ("line", "+an <a"),
    ("line", "+ao AO"),
    ("line", '+ap AP "3 FooBar" <ap.pfb'),
    ("line", '+aq AQ "3 FooBar <aq.pfb'),
    ("line", '+ar AR ".1 SlantFont <ar.pfb'),
    ("line", '+as AS "foo bar" <as.pfb'),
    ("line", '+at AT "0.5ExtendFont" <at.pfb'),
    ("line", "+au AU 4 <au.pfb"),
    ("line", "+av 4 <av.pfb"),
    ("line", "+aw AW 4x <aw.pfb"),
    ("line", "+ax 8r.enc <ax.pfb"),
    ("line", "+ay AY < ay.pfb"),
    ("line", "+az AZ << az.pfb"),
    ("line", "+ba BA <[ ba.pfb"),
    ("line", "+bb BB <[8r.enc <bb.pfb"),
    ("line", "+bc BC <bc.TTF"),
    ("line", "+bd BD <<bd.OTF"),
    ("line", "+be BE <be.TTC"),
    ("line", "+bf BF <8R.ENC <bf.pfb"),
    ("line", " + bg BG <bg.pfb"),
    ("line", " = bh BH <bh.pfb"),
    ("line", "+bg BG <bg.pfb"),
    ("suppress", "1"),
    ("line", "+bg BG <bg.pfb"),
    ("suppress", "0"),
    # A ps name in the tree stops the ps half quietly; the tfm half has gone in.
    ("line", "+cmr5 Foo <cmr5.pfb"),
    ("line", "+cmr6 Foo <cmr6.pfb"),
    ("line", "+cmr5 Foo <cmr5.pfb"),
    ("line", '+cmr7 Foo ".1 SlantFont" <cmr7.pfb'),
    # Replace and delete, of something there and of something not.
    ("line", "=cmr8 Bar <cmr8.pfb"),
    ("line", "=cmr8 Baz <cmr8.pfb"),
    ("line", "-cmr8"),
    ("line", "-cmr8"),
    ("line", "-cmr8 Baz <cmr8.pfb"),
    ("line", "-cmr9 Baz cmr9.pfb"),
    # An entry in use stays.
    ("use", "cmr5"),
    ("line", "=cmr5 Foo2 <cmr5.pfb"),
    ("line", "-cmr5"),
    ("line", "+cmr5 Foo <cmr5.pfb"),
    ("line", "=cmr10 Foo <cmr10.pfb"),
    ("use", "cmr10"),
    ("line", "=cmr12 Foo <cmr12.pfb"),
    ("line", "=cmr10 Quux <cmr10.pfb"),
    ("line", "-cmr12 Foo"),
    # The second half of a replace, with a ps name in use: quiet, and the tfm half stays done.
    ("line", "=cmr17 Foo <cmr17.pfb"),
    ("line", "+cmr17 Foo <cmr17.pfb"),
]


def main():
    doc = [r"\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \catcode`\^=7 \catcode`\%=12 \scrollmode",
           r"\pdfoutput=1 \showboxbreadth=100 \showboxdepth=100"]
    fonts = 0
    for n, (op, arg) in enumerate(ROWS):
        doc.append(r"\immediate\write-1{ROW %d}" % n)
        if op == "line":
            doc.append(r"\pdfmapline{%s}" % arg)
        elif op == "suppress":
            doc.append(r"\pdfsuppresswarningdupmap=%s" % arg)
        else:
            fonts += 1
            doc.append(r"\font\f%s=%s\relax" % ("abcdefgh"[fonts - 1], arg))
            doc.append(r"\setbox0\hbox{\pdffontobjnum\f%s}" % "abcdefgh"[fonts - 1])
    doc.append(r"\immediate\write-1{ROW END}")
    doc.append(r"\end")
    with tempfile.TemporaryDirectory() as d:
        with open(os.path.join(d, "t.tex"), "w") as f:
            f.write("\n".join(doc) + "\n")
        subprocess.run(["pdftex", "-ini", "-interaction=nonstopmode", "t.tex"], cwd=d, capture_output=True)
        log = open(os.path.join(d, "t.log"), encoding="latin-1").read()
    parts = re.split(r"^ROW (\d+|END)\n", log, flags=re.M)
    out = []
    for i, (op, arg) in enumerate(ROWS):
        assert parts[2 * i + 1] == str(i), (i, parts[2 * i + 1])
        text = parts[2 * i + 2]
        assert "|" not in text and "\t" not in text and "|" not in arg and "\t" not in arg
        out.append("%s\t%s\t%s" % (op, arg, text.replace("\n", "|")))
    with open(OUT, "w") as f:
        f.write("\n".join(out) + "\n")
    print(len(out), "rows")


main()
