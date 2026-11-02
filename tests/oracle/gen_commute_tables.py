#!/usr/bin/env python3
"""Fixtures for crates/commute: box dimensions and expected geometry, from pdflatex and tests/cd/model/model.py.

Run with python3 -I from anywhere.  It writes, under crates/commute/testdata/:

  cd/<test>.tsv     for each diagram of tests/cd/<test>.tex: its source, its font parameters and measured boxes, and,
                    when the model handles it, what the model computes for it
  pdf_forms.tsv     for each short diagram without colours: its arrows' PDF operators as pdflatex wrote them, labels' text
                    taken out

Only files tracked by git under tests/cd go in, and never 21-* or 22-*, whether or not they are present: those are
private documents' diagrams, and this directory is committed.

Each diagram is compiled once with tests/cd/model/instr.tex, which logs every node's box.  Then the model recomputes the
drawing from those boxes alone and is compared with the operators in the PDF, exactly as model.py's check() does.  A
diagram counts as handled by the model if that comparison passes.

Two corrections to check():

  - A diagram inside amsmath's align is typeset twice, a trial and the real one, so from there on the log's pictures
    and the PDF's are out of step; check() then compares the wrong boxes (spec section 1).  Here each diagram gets the
    log entry of its real run.  That makes three more diagrams of test 27 comparable.
  - A diagram in a \\setbox that is never shipped has no PDF operators to compare.  It is still computed, and its picture
    box is compared with the log's.  That adds the two of test 17.

Format: one record per line, fields separated by a tab.  A newline inside a field is written ^^J.

  D  file  index  status  detail      a diagram; status OK, or why the model does not handle it
  F  ex  em  rt  ax                   font parameters
  P  text                             the \\tikzcdset{...} directly before the environment, if any
  S  text                             everything between \\begin{tikzcd} and \\end{tikzcd}, comments included
  C  r  c  w  h  d                    a cell's box, 1-based, only for cells that have a node
  L  w  h  d  lw                      a label's box and the line width in force, in drawing order
  M  W  H  D                          the model's matrix dimensions
  B  minx  miny  maxx  maxy           the model's picture box
  O  minx  miny  maxx  maxy  base     the oracle's picture box and baseline
  X  what  w  cap  join  dash  color  segs     a stroke in the model's order; what = halo line inner tip;
                                      dash = "-" or "a b" in pt; color = the oracle's stroke colour, "-" unless
                                      compared; segs = m x y l x y c x1 y1 x2 y2 x3 y3
  R  x  y  w  h  color                a label's background
  T  cell  r  c  x  y  cos  sin       where a cell's box goes
  T  label  i  x  y  cos  sin         where a label's box goes
"""
import os
import re
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
MODEL = os.path.join(ROOT, "tests", "cd", "model")
OUT = os.path.join(ROOT, "crates", "commute", "testdata")
sys.path.insert(0, MODEL)

import run as runner  # noqa: E402  (creates tests/cd/out on import; removed below)
import model as M  # noqa: E402
import pdfops  # noqa: E402

ENV = re.compile(r"(?:\{\\tikzcdset\{((?:[^{}]|\{[^{}]*\})*)\}\s*)?\\begin\{tikzcd\}(.*?)\\end\{tikzcd\}", re.S)
CELL_OR_MATRIX = re.compile(r"tikz@f@\d+(-\d+-\d+)?")
BP = M.BP


def num(x):
    s = "%.5f" % x
    s = s.rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


def esc(s):
    return s.replace("\n", "^^J").replace("\t", "^^I")


def tracked_tests():
    out = subprocess.run(["git", "ls-files", "tests/cd/*.tex"], cwd=ROOT, capture_output=True, text=True, check=True).stdout
    # The pathspec's * also matches through directories.
    names = [os.path.basename(p) for p in out.split() if os.path.dirname(p) == "tests/cd"]
    return sorted(n for n in names if not re.match(r"2[12]-", n))


def strip_comments(src):
    return re.sub(r"(?<!\\)%.*", "", src)


def raw_diagrams(src):
    """(preset, text) of each diagram, comments kept in the text, matched like model.scan_diagrams."""
    # Comment lines at the top of a test could hold the words; only the document body counts.
    start = src.index("\\begin{document}")
    return [(m.group(1) or "", m.group(2)) for m in ENV.finditer(src, start)]


def real_run_index(src, diags):
    """For each diagram, the index of its real run among the log's pictures: align typesets its body twice."""
    start = src.index("\\begin{document}")
    aligns = [(m.start(), m.end()) for m in re.finditer(r"\\begin\{align\}.*?\\end\{align\}", src, re.S)]
    spans = [m.span() for m in ENV.finditer(src, start)]
    assert len(spans) == len(diags)
    out = []
    shift = 0
    i = 0
    while i < len(spans):
        block = next((a for a in aligns if a[0] <= spans[i][0] < a[1]), None)
        if block is None:
            out.append(i + shift)
            i += 1
            continue
        members = [j for j in range(len(spans)) if block[0] <= spans[j][0] < block[1]]
        shift += len(members)
        for j in members:
            out.append(j + shift)
        i = members[-1] + 1
    return out


def stroke_ops(ops, nskip):
    """The PDF's stroke operators, without the first nskip, which are cell borders."""
    kept = []
    for o in ops:
        if o["kind"] == "path":
            if nskip > 0:
                nskip -= 1
                continue
            kept.append(o)
    return kept


def color(c):
    return " ".join(num(x) for x in c)


def segs(path):
    out = []
    for s in path:
        out.append(s[0])
        out.extend(num(x) for x in s[1:])
    return " ".join(out)


def text_dev(d, texts, origin):
    ox, oy = origin
    dev = 0.0
    for tp in d.textpos:
        if tp[-2] == 0:
            continue
        x, y = tp[-4], tp[-3]
        best = min((max(abs(x * 0.996264 - (t["at"][0] - ox)), abs(y * 0.996264 - (t["at"][1] - oy))) for t in texts), default=1e9)
        if best <= 0.03:
            dev = max(dev, best)
    return dev


def one_diagram(name, index, opts, body, pic, pdf):
    """Run the model on one diagram.  Returns (status, detail, records)."""
    rec = []
    info = [p for p in pic if p["type"] == "CDINFO"][0]
    f = M.F(M.pt(info["ex"]), M.pt(info["em"]), M.pt(info["rt"]), M.pt(info["ax"]))
    rec.append(["F", num(f.ex), num(f.em), num(f.rt), num(f.ax)])
    nodes = [M.node_rec(p) for p in pic if p["type"] == "CDNODE"]
    picinfo = [p for p in pic if p["type"] == "CDPIC"][0]

    for n in nodes:
        m = re.fullmatch(r"tikz@f@\d+-(\d+)-(\d+)", n["name"])
        if m:
            rec.append(["C", m.group(1), m.group(2), num(n["wd"]), num(n["ht"]), num(n["dp"])])
    labels = [n for n in nodes if not CELL_OR_MATRIX.fullmatch(n["name"]) and n["shape"] != "coordinate"]
    for n in labels:
        rec.append(["L", num(n["wd"]), num(n["ht"]), num(n["dp"]), num(n["lw"])])
    baseline = picinfo["baseline"]
    rec.append(["O", num(M.pt(picinfo["minx"])), num(M.pt(picinfo["miny"])), num(M.pt(picinfo["maxx"])), num(M.pt(picinfo["maxy"])), num(M.pt(baseline))])

    try:
        d = M.Diagram(opts, body, info, f, nodes)
        d.layout()
        if max(abs(d.problems_mat[i] - d.problems_mat[i + 3]) for i in range(3)) > 0.02:
            return "DIFF", "matrix dimensions differ", rec
        d.draw_arrows(iter(labels))
    except M.Unsupported as e:
        return "SKIP", str(e), rec
    except KeyError as e:
        return "EXC", "KeyError %s" % (e,), rec

    oracle_bb = tuple(M.pt(picinfo[k]) for k in ("minx", "miny", "maxx", "maxy"))
    bbd = max(abs(a - b) for a, b in zip(d.bb, oracle_bb))
    maxd = 0.0
    tdev = 0.0
    msgs = []
    pdf_strokes = None
    if pdf is not None:
        nskip = getattr(d, "cell_strokes", 0)
        pops = []
        left = nskip
        for o in pdf["ops"]:
            if o["kind"] == "path" and left > 0:
                left -= 1
                continue
            pops.append(o)
        maxd, msgs = M.compare_ops(d.result(), pops, pdf["origin"])
        pdf_strokes = stroke_ops(pdf["ops"], nskip)
        tdev = text_dev(d, [o for o in pdf["ops"] if o["kind"] == "text"], pdf["origin"])
    if msgs or d.problems or bbd >= 0.15:
        return "DIFF", "; ".join((d.problems + msgs)[:2]) or "bounding box", rec

    w, h, dp = d.problems_mat[:3]
    rec.append(["M", num(w), num(h), num(dp)])
    rec.append(["B"] + [num(x) for x in d.bb])
    for i, o in enumerate(d.result()):
        po = pdf_strokes[i] if pdf_strokes is not None and i < len(pdf_strokes) else None
        cdx, cdy = o.get("canvas", (0.0, 0.0))
        if o["kind"] == "fill":
            node = o["node"]
            x0, y0 = node.to_world((node.SW[0] + node.ox, node.SW[1] + node.oy))
            ww = node.NE[0] - node.ox - node.SW[0] - node.ox
            hh = node.NE[1] - node.oy - node.SW[1] - node.oy
            rec.append(["R", num(x0 + cdx), num(y0 + cdy), num(ww), num(hh), color(po["rg"]) if po else "-"])
            continue
        dash = "-" if not o.get("dash") else " ".join(num(x) for x in o["dash"])
        path = [(s[0],) + tuple(v + (cdx if j % 2 == 0 else cdy) for j, v in enumerate(s[1:])) for s in o["path"]]
        rec.append(["X", o["what"], num(o["w"]), str(o["cap"]), str(o["join"]), dash, color(po["RG"]) if po else "-", segs(path)])
    label_i = 0
    for tp in d.textpos:
        c, s = tp[-1]
        if tp[0] == "cell":
            rec.append(["T", "cell", str(tp[1][0]), str(tp[1][1]), num(tp[2]), num(tp[3]), num(c), num(s)])
        else:
            rec.append(["T", "label", str(label_i), num(tp[2]), num(tp[3]), num(c), num(s)])
            label_i += 1
    return "OK", "maxdev=%.4fbp textdev=%.4fbp bboxdev=%.4fpt" % (maxd, tdev, bbd), rec


def pdf_pictures(pdf):
    all_ops = []
    for c in runner.contents(pdf):
        all_ops += pdfops.parse(c)
    pics = []
    cur = None
    for o in all_ops:
        if o["kind"] == "pic":
            cur = dict(origin=(o["ctm"][4], o["ctm"][5]), ops=[])
            pics.append(cur)
        elif cur is not None:
            cur["ops"].append(o)
    return pics


def arrow_forms(pdf):
    """For each picture of a PDF, the operators after the matrix, without the text of labels, which is TeX's."""
    st = " ".join(runner.contents(pdf))
    st = re.sub(r"\s+", " ", st)
    out = []
    for part in re.split(r"(?=q (?:0 G 0 g|[01. ]+ RG [01. ]+ rg) 0\.3985 w)", st)[1:]:
        body = part[: part.find(" Q n Q")] + " "
        # The matrix's scope closes with a translation and two Q, and an arrow or the end follows.  A cell's own close
        # has a third Q; so has a label's.
        m = re.search(r"1 0 0 1 -?[\d.]+ -?[\d.]+ cm Q Q (?=q |$)", body)
        ops = body[m.end():].strip() if m else ""
        ops = re.sub(r"q 0 G 0 g 1 0 0 1 -?[\d.]+ -?[\d.]+ cm BT .*? ET 1 0 0 1 -?[\d.]+ -?[\d.]+ cm Q", "q 0 G 0 g Q", ops)
        out.append(ops)
    return out


def plain_form(ops):
    """No colours but the default, and short enough to keep: colour operators are compared in the cd tables."""
    if not ops or len(ops) > 1500 or " BT " in ops:
        return False
    rest = ops.replace("q 0 G 0 g", "q").replace(" 1 G ", " ").replace("q 1 g q 1 g", "q")
    return not re.search(r"(^| )(g|G|rg|RG|k|K|gs)( |$)", rest)


def main():
    runner.OUT = tempfile.mkdtemp(prefix="commute-oracle-")
    stray = os.path.join(ROOT, "tests", "cd", "out")
    if os.path.isdir(stray) and not os.listdir(stray):
        os.rmdir(stray)
    os.makedirs(os.path.join(OUT, "cd"), exist_ok=True)

    total = {}
    forms = []
    worst = worst_text = worst_bbox = 0.0
    for name in tracked_tests():
        path = os.path.join(ROOT, "tests", "cd", name)
        pdf, log, _ = runner.compile(path)
        logtxt = open(pdf.replace(".pdf", ".log"), errors="replace").read()
        src = open(path).read()
        model_diags = M.scan_diagrams(src)
        raw = raw_diagrams(src)
        assert len(raw) == len(model_diags), name
        pics = M.group_pictures(M.parse_log(logtxt))
        run_of = real_run_index(src, raw)
        pdfpics = pdf_pictures(pdf)

        lines = []
        statuses = []
        for i, ((opts, body), (preset, text)) in enumerate(zip(model_diags, raw)):
            pic_i = run_of[i]
            pdf_pic = pdfpics[i] if i < len(pdfpics) else None
            if pic_i >= len(pics):
                status, detail, rec = "NOLOG", "no log entry", []
            else:
                status, detail, rec = one_diagram(name, i + 1, opts, body, pics[pic_i], pdf_pic)
            total[status] = total.get(status, 0) + 1
            statuses.append(status)
            print("%s #%d: %s %s" % (name, i + 1, status, detail))
            if status == "OK":
                m = re.fullmatch(r"maxdev=([\d.]+)bp textdev=([\d.]+)bp bboxdev=([\d.]+)pt", detail)
                worst = max(worst, float(m.group(1)))
                worst_text = max(worst_text, float(m.group(2)))
                worst_bbox = max(worst_bbox, float(m.group(3)))
            lines.append(["D", name[:-4], str(i + 1), status, esc(detail)])
            if preset:
                lines.append(["P", esc(preset)])
            lines.append(["S", esc(text)])
            lines.extend(rec)

        with open(os.path.join(OUT, "cd", name[:-4] + ".tsv"), "w") as f:
            for r in lines:
                f.write("\t".join(r) + "\n")

        for i, ops in enumerate(arrow_forms(pdf)):
            if statuses[i] == "OK" and plain_form(ops):
                forms.append([name[:-4], str(i + 1), ops])

    with open(os.path.join(OUT, "pdf_forms.tsv"), "w") as f:
        for r in forms:
            f.write("\t".join(r) + "\n")

    print("status counts:", total)
    print("worst model vs pdflatex: stroke %.4f bp, text %.4f bp, bbox %.4f pt" % (worst, worst_text, worst_bbox))


if __name__ == "__main__":
    main()
