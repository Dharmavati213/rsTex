#!/usr/bin/env python3
"""Measure tests/oracle/commute_extra.tex with pdflatex and write what the PDF holds, for crates/commute/tests/extra.rs.

    python3 -I tests/oracle/check_commute_extra.py
    COMMUTE_EXTRA=target/commute-extra/extra.tsv cargo test -p commute --test extra -- --nocapture

The table goes under target/, not into the repository: these diagrams exist to check keys beyond the model, and the
committed fixtures hold only what derives from committed tests/cd files.

Records as in gen_commute_tables.py, and for what the PDF draws, in bp relative to the picture's origin:

  Y  op  w  cap  join  dash  RG  rg  segs      op = S or f; dash = values or "-"; segs as there, plus re x y w h
  G  x  y                                       origin of a text run, in bp
"""
import os
import re
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(ROOT, "tests", "cd", "model"))

import run as runner  # noqa: E402
import model as M  # noqa: E402
import gen_commute_tables as G  # noqa: E402

OUT = os.path.join(ROOT, "target", "commute-extra")


def main():
    runner.OUT = tempfile.mkdtemp(prefix="commute-extra-")
    stray = os.path.join(ROOT, "tests", "cd", "out")
    if os.path.isdir(stray) and not os.listdir(stray):
        os.rmdir(stray)
    os.makedirs(OUT, exist_ok=True)
    path = os.path.join(HERE, "commute_extra.tex")
    pdf, log, rc = runner.compile(path)
    logtxt = open(pdf.replace(".pdf", ".log"), errors="replace").read()
    errors = re.findall(r"^!.*", logtxt, re.M)
    if errors:
        print("pdflatex errors:", errors[:10])
    src = open(path).read()
    raw = G.raw_diagrams(src)
    pics = M.group_pictures(M.parse_log(logtxt))
    pdfpics = G.pdf_pictures(pdf)
    assert len(pics) == len(raw) == len(pdfpics), (len(pics), len(raw), len(pdfpics))
    lines = []
    for i, ((preset, text), pic, pp) in enumerate(zip(raw, pics, pdfpics)):
        info = [p for p in pic if p["type"] == "CDINFO"][0]
        nodes = [M.node_rec(p) for p in pic if p["type"] == "CDNODE"]
        picinfo = [p for p in pic if p["type"] == "CDPIC"][0]
        lines.append(["D", "extra", str(i + 1), "PDF", "-"])
        if preset:
            lines.append(["P", G.esc(preset)])
        lines.append(["S", G.esc(text)])
        lines.append(["F"] + [G.num(M.pt(info[k])) for k in ("ex", "em", "rt", "ax")])
        for n in nodes:
            m = re.fullmatch(r"tikz@f@\d+-(\d+)-(\d+)", n["name"])
            if m:
                lines.append(["C", m.group(1), m.group(2), G.num(n["wd"]), G.num(n["ht"]), G.num(n["dp"])])
        # A label with only an alias gets an automatic name, like the matrix, which comes first.
        matrix_seen = False
        for n in nodes:
            if re.fullmatch(r"tikz@f@\d+-\d+-\d+", n["name"]) or n["shape"] == "coordinate":
                continue
            if not matrix_seen and re.fullmatch(r"tikz@f@\d+", n["name"]):
                matrix_seen = True
                continue
            lines.append(["L", G.num(n["wd"]), G.num(n["ht"]), G.num(n["dp"]), G.num(n["lw"])])
        lines.append(["O"] + [G.num(M.pt(picinfo[k])) for k in ("minx", "miny", "maxx", "maxy", "baseline")])
        ox, oy = pp["origin"]
        for o in pp["ops"]:
            if o["kind"] == "text":
                lines.append(["G", G.num(o["at"][0] - ox), G.num(o["at"][1] - oy)])
            if o["kind"] != "path":
                continue
            segs = []
            for s in o["path"]:
                if s[0] == "h":
                    segs.append("h")
                elif s[0] == "re":
                    segs += ["re", G.num(s[1] - ox), G.num(s[2] - oy), G.num(s[3]), G.num(s[4])]
                else:
                    segs.append(s[0])
                    for j, v in enumerate(s[1:]):
                        segs.append(G.num(v - (ox if j % 2 == 0 else oy)))
            dash = " ".join(G.num(x) for x in o["dash"][0]) or "-"
            lines.append(["Y", o["op"], G.num(o["w"]), str(o["cap"]), str(o["join"]), dash, G.color(o["RG"]), G.color(o["rg"]), " ".join(segs)])
    with open(os.path.join(OUT, "extra.tsv"), "w") as f:
        for r in lines:
            f.write("\t".join(r) + "\n")
    print("wrote %d diagrams to %s" % (len(raw), os.path.join(OUT, "extra.tsv")))


if __name__ == "__main__":
    main()
