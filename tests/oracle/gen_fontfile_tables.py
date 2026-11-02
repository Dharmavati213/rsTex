#!/usr/bin/env python3
"""Expected results for crates/fontfile/tests/fontfile_oracle.rs, taken from the oracle.

Run with python3 -I.  It writes, under crates/fontfile/testdata/:

  fontfile_syn.tsv    synthetic Type 1 fonts, which the test makes again byte for byte at run time, through the
                      oracle: lengths, digest of the stream, name with tag, /CharSet, warnings, fatal errors,
                      and the /FontMatrix and /ItalicAngle lines that show the float arithmetic
  fontfile_float.tsv  every SlantFont and ExtendFont value in pdftex.map, and some awkward ones, on one such font
  fontfile_map.tsv    map lines good and bad: what the oracle makes of them
  fontfile_tex.tsv    fonts from TeX Live, with the digest of the installed font program; the test skips a row
                      if the file is missing or differs
  fontfile_uni.tsv    /ToUnicode streams from TeX Live fonts, with glyphtounicode.tex

No font goes into the tree.  Fields are separated by a tab; the items of a list in a field by |||.
"""
import hashlib
import os
import random
import re
import shutil
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import fontfile_gate as g  # noqa: E402

TESTDATA = os.path.join(g.ROOT, "crates", "fontfile", "testdata")
SEP = "|||"

# What fontfile_oracle.rs knows how to make.
SYN_CASES = [
    "std", "std_pfa", "dupenc", "arrayenc", "dupops", "seac", "seac_std", "hints", "hints_b", "flex", "noreturn",
    "emptysubr", "div", "leniv0", "leniv1", "leniv7", "tokens_dash", "tokens_put", "tokens_dash_put", "tokens_bad",
    "unusual_cs", "subrs0", "synthetic", "nofontname", "type3", "leniv_neg", "undefined_glyph", "notdef_code",
    "subr_range", "subr_store_range", "bad_command", "bad_escape", "escape_range", "more_args", "less_args",
    "stack_pop", "too_many_glyphs", "no_number", "bad_fontmatrix", "brace_fontmatrix", "unclosed_fontmatrix",
    "pred_enc_bad", "array_enc_bad", "noname_font", "uniqueid_forms", "stemv", "truncated",
]

TEX_FONTS = [
    # family, map entries
    ("cm", ["cmr10", "cmmi10", "cmsy10", "cmex10", "cmbx12", "cmtt10"]),
    ("ams", ["msam10", "msbm10", "eufm10", "cmbsy10"]),
    ("lm", ["ec-lmr10", "ec-lmri10", "ec-lmbx12", "rm-lmr10", "ts1-lmr10", "qx-lmr10"]),
    ("cmsuper", ["ecrm1000", "ecti1000", "ecbx1200", "tcrm1000", "ectt1000"]),
    ("urw", ["ptmr8r", "ptmri8r", "ptmro8r", "phvr8r", "phvb8r", "pcrr8r", "pplr8r", "pbkl8r", "pagk8r", "uncr8r"]),
    ("other", ["rsfs10", "wasy10", "stmary10"]),
]

TTF_CASES = [
    ("marvosym.ttf", b"=cmr10 <8r.enc <marvosym.ttf", list(range(33, 127))),
    ("marvosym.ttf", b"=cmr10 <<marvosym.ttf", [65]),
    ("emo-lingchi.ttf", b"=cmr10 <<emo-lingchi.ttf", [65]),
    ("Redacted-Regular.ttf", b"=cmr10 <8r.enc <Redacted-Regular.ttf", [65, 66, 97]),
    ("lmroman10-regular.otf", b"=cmr10 <<lmroman10-regular.otf", [65]),
]


def md5(b):
    return hashlib.md5(b).hexdigest()


def run(job):
    """The manifest records of one job, as lists of fields."""
    text, d = g.run_job(job)
    return [ln.split("\t") for ln in text.split("\n") if ln], d


def unhex(s):
    return b"" if s == "-" else bytes.fromhex(s)


def field(b):
    return b.decode("latin-1")


def desc_of(recs):
    for r in recs:
        if r[0] == "desc":
            return r
    return None


def stream_of(desc):
    if desc is None or desc[14] == "-":
        return b""
    with open(desc[14], "rb") as f:
        return f.read()


def messages(recs, kind):
    return SEP.join(field(unhex(r[1])) for r in recs if r[0] == kind) or "-"


def syn_row(name, font, codes, specials):
    recs, d = run(g.synth_job(name, font, codes, specials))
    desc = desc_of(recs)
    stream = stream_of(desc)
    head = stream[: int(desc[10])] if desc else b""
    fm = re.search(rb"/FontMatrix (.*)\n", head)
    it = re.search(rb"/ItalicAngle (.*)\n", head)
    shutil.rmtree(d, ignore_errors=True)
    return [
        name,
        field(specials) or "-",
        ",".join(map(str, codes)),
        md5(font),
        desc[10] if desc else "-",
        desc[11] if desc else "-",
        desc[12] if desc else "-",
        md5(stream) if desc else "-",
        field(unhex(desc[1])) if desc else "-",
        desc[4] if desc else "-",
        desc[3] if desc else "-",
        desc[8] if desc else "-",
        desc[2] if desc else "-",
        field(unhex(desc[15])) if desc and desc[15] != "-" else "-",
        field(fm.group(1)) if fm else "-",
        field(it.group(1)) if it else "-",
        messages(recs, "warn"),
        messages(recs, "error"),
    ]


def write(name, rows):
    with open(os.path.join(TESTDATA, name), "w", encoding="latin-1", newline="\n") as f:
        for r in rows:
            f.write("\t".join(r) + "\n")
    print(name, len(rows))


def gen_syn(cases):
    rows = []
    for name in SYN_CASES:
        font, codes, specials = cases[name]
        rows.append(syn_row(name, font, codes, specials))
    write("fontfile_syn.tsv", rows)


def gen_float(cases):
    font = cases["std"][0]
    entries = g.read_map()
    slants, extends = set(), set()
    for raw in entries.values():
        for m in re.finditer(rb"(-?[\d.]+)\s+(Slant|Extend)Font", raw):
            (slants if m.group(2) == b"Slant" else extends).add(m.group(1).decode())
    slants |= {".1", "-.1", ".25", ".3333", "-1", "1", ".0005", "-.0005", "1e-1", "0x1p-3", ".1234567", ".9999", "0"}
    extends |= {".5", ".8", "1.1", "1.0005", ".9995", "2", ".85", "1.2345678", "0"}
    # Where narrowing d * 1000.0 to float changes the rounding: in double these would come out one lower.
    narrowing = {".0025", ".0045", ".0055", ".0145", ".0475", ".0845"}
    slants |= narrowing
    extends |= narrowing
    rows = []
    for v in sorted(slants):
        rows.append(syn_row("s" + v, font, [65], b'"%s SlantFont"' % v.encode()))
    for v in sorted(extends):
        rows.append(syn_row("e" + v, font, [65], b'"%s ExtendFont"' % v.encode()))
    rows.append(syn_row("both", font, [65], b'".167 SlantFont .9 ExtendFont"'))
    rows.append(syn_row("glued", font, [65], b'".8ExtendFont"'))
    rows.append(syn_row("toobig", font, [65], b'"1.5 SlantFont"'))
    write("fontfile_float.tsv", rows)


def gen_map():
    with open(g.kpse("cmr10.tfm"), "rb") as f:
        cmr = f.read()
    lines = [
        (b"zz01 ZZ <cmr10.pfb", 1),
        (b"zz02 ZZ 4 <cmr10.pfb", 1),
        (b"zz03 ZZ 34 <<cmr10.pfb", 1),
        (b"zz04 <cmr10.pfb", 1),
        (b"zz05 ZZ < cmr10.pfb", 1),
        (b"zz06 ZZ << cmr10.pfb", 1),
        (b'zz07 ZZ ".167 SlantFont" <cmr10.pfb', 1),
        (b'zz08 ZZ "1 ExtendFont" <cmr10.pfb', 1),
        (b'zz09 ZZ "3 FooBar" <cmr10.pfb', 1),
        (b"zz10 ZZ <[8r.enc <cmr10.pfb", 1),
        (b"zz11 ZZ 8r.enc <cmr10.pfb", 1),
        (b"zz12 8r.enc <cmr10.pfb", 1),
        (b'zz13 ZZ "TeXBase1Encoding ReEncodeFont" <8r.enc <cmr10.pfb', 1),
        (b"zz14 Times-Roman", 1),
        (b"zz15 Foo", 1),
        (b'zz16 ZZ "1.5 SlantFont" <cmr10.pfb', 0),
        (b'zz17 ZZ "3 ExtendFont" <cmr10.pfb', 0),
        (b'zz18 ZZ "1 SlantFont <cmr10.pfb', 0),
        (b"zz19 ZZ cmr10.pfb", 0),
        (b'zz20 ZZ ".2 SlantFont"', 0),
        (b"zz21 <a.ttf <8r.enc <<a.ttf", 0),
        (b"zz22 ZZ PidEid=3,1 <a.pfb", 0),
        (b"zz23", 0),
        (b"<cmr10.pfb", 0),
    ]
    rows = []
    for i, (line, use) in enumerate(lines):
        tfm = line.split()[0].decode()
        if tfm.startswith("<"):
            tfm = "zz"
        font = g.Font(tfm, [65] if use else [], b"=" + line, "line", [(tfm + ".tfm", cmr)] if use else [])
        recs, d = run(g.Job("tbl_map_%02d" % i, [font]))
        desc = desc_of(recs)
        shutil.rmtree(d, ignore_errors=True)
        name = field(unhex(desc[1])) if desc else "-"
        rows.append([field(line), name, desc[2] if desc else "-", messages(recs, "warn"), messages(recs, "error")])
    write("fontfile_map.tsv", rows)


def gen_tex():
    rng = random.Random(2026)
    entries = g.read_map()
    rows = []
    for family, names in TEX_FONTS:
        for tfm in names:
            raw = entries.get(tfm)
            if raw is None:
                print("not in the map:", tfm)
                continue
            p = g.kpse(tfm + ".tfm")
            if p:
                codes, files = g.tfm_codes(p), ()
            else:
                sur = g.kpse("ec-lmr10.tfm")
                with open(sur, "rb") as f:
                    files = [(tfm + ".tfm", f.read())]
                codes = g.tfm_codes(sur)
            for kind, chosen in (("all", codes), ("few", sorted(rng.sample(codes, min(10, len(codes)))))):
                recs, d = run(g.Job("tbl_%s_%s" % (tfm, kind), [g.Font(tfm, chosen, raw.strip(), "file", files)]))
                desc = desc_of(recs)
                font_rec = next(r for r in recs if r[0] == "font")
                if desc is None or font_rec[4] == "-":
                    shutil.rmtree(d, ignore_errors=True)
                    print("no result:", tfm, kind, messages(recs, "error"))
                    continue
                with open(font_rec[4], "rb") as f:
                    program = md5(f.read())
                enc = ""
                if font_rec[3] != "-":
                    with open(font_rec[3], "rb") as f:
                        enc = md5(f.read())
                stream = stream_of(desc)
                rows.append([
                    family, tfm, kind, field(raw.strip()), ",".join(map(str, chosen)),
                    os.path.basename(font_rec[4]), program,
                    os.path.basename(font_rec[3]) if font_rec[3] != "-" else "-", enc or "-",
                    desc[10], desc[11], desc[12], md5(stream), field(unhex(desc[1])),
                    desc[3], desc[4], desc[2], md5(unhex(desc[15])),
                ])
                shutil.rmtree(d, ignore_errors=True)
    for name, line, codes in TTF_CASES:
        path = g.kpse(name)
        if not path:
            continue
        recs, d = run(g.Job("tbl_tt_" + re.sub(r"\W", "_", name), [g.Font("cmr10", codes, line, "line")]))
        desc = desc_of(recs)
        font_rec = next(r for r in recs if r[0] == "font")
        with open(path, "rb") as f:
            program = md5(f.read())
        enc_name = os.path.basename(font_rec[3]) if font_rec[3] != "-" else "-"
        enc = "-"
        if font_rec[3] != "-":
            with open(font_rec[3], "rb") as f:
                enc = md5(f.read())
        stream = stream_of(desc)
        rows.append([
            "tt", "cmr10", messages(recs, "warn") and "ttf", field(line), ",".join(map(str, codes)),
            name, program, enc_name, enc,
            desc[10], desc[11], desc[12], md5(stream), field(unhex(desc[1])),
            desc[3], desc[4], desc[2], "-",
        ])
        shutil.rmtree(d, ignore_errors=True)
    write("fontfile_tex.tsv", rows)


def gen_uni():
    entries = g.read_map()
    rows = []
    for tfm in ("cmr10", "cmmi10", "ec-lmr10", "ptmr8r", "ecrm1000", "msam10", "rm-lmr10"):
        raw = entries.get(tfm)
        if raw is None:
            continue
        p = g.kpse(tfm + ".tfm")
        files = ()
        if not p:
            sur = g.kpse("ec-lmr10.tfm")
            with open(sur, "rb") as f:
                files = [(tfm + ".tfm", f.read())]
            codes = g.tfm_codes(sur)
        else:
            codes = g.tfm_codes(p)
        recs, d = run(g.Job("tbl_uni_" + tfm, [g.Font(tfm, codes, raw.strip(), "file", files)], tounicode=True))
        uni = next((r for r in recs if r[0] == "unicode"), None)
        font_rec = next(r for r in recs if r[0] == "font")
        if uni is None:
            shutil.rmtree(d, ignore_errors=True)
            continue
        with open(uni[3], "rb") as f:
            stream = f.read()
        with open(font_rec[4], "rb") as f:
            program = md5(f.read())
        enc = "-"
        if font_rec[3] != "-":
            with open(font_rec[3], "rb") as f:
                enc = md5(f.read())
        with open(g.kpse("glyphtounicode.tex"), "rb") as f:
            gtu = md5(f.read())
        rows.append([
            tfm, field(raw.strip()), os.path.basename(font_rec[4]), program,
            os.path.basename(font_rec[3]) if font_rec[3] != "-" else "-", enc, gtu,
            str(len(stream)), md5(stream),
        ])
        shutil.rmtree(d, ignore_errors=True)
    write("fontfile_uni.tsv", rows)


def main():
    os.makedirs(g.WORK, exist_ok=True)
    os.makedirs(TESTDATA, exist_ok=True)
    g.write_gtu()
    # fontfile_uni.tsv is made with glyphtounicode.tex alone, which is what the test reads.
    shutil.copy(g.kpse("glyphtounicode.tex"), os.path.join(g.OUT, "gtu.tex"))
    cases = {name: (font, codes, specials) for name, font, codes, specials in g.synth_cases()}
    gen_syn(cases)
    gen_float(cases)
    gen_map()
    gen_tex()
    gen_uni()
    g.write_gtu()


if __name__ == "__main__":
    main()
