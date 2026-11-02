#!/usr/bin/env python3
"""Expected results for show.rs and copy.rs, taken from the oracle.

Run with python3 -I.  It writes, under crates/engine/testdata/:

  show_oracle.tsv   a case's name, then what show_box prints for it, but for the last newline
  short_oracle.tsv  a case's name, then what short_display prints for the list of its box
  copy_oracle.tsv   a case's name, then the memory that \\copy and flushing take and give back:
                    copy_var copy_dyn flush_var flush_dyn
  copy_show_oracle.tsv  the cases whose \\copy does not show as the original does: a case's name, then
                    what show_box prints for the copy

Each case is a list of nodes, made by the real engine from TeX source.  The tests in show.rs build the same
nodes by hand, with the names of the cases here, and compare what show_box prints with what the
oracle wrote for `\\showbox` or `\\showlists`.

A case is (name, source, options):

  source   for a box, what follows \\setbox1=.  For `pick`, what makes the mode whose list is shown.
  setup    TeX run before the case, whose nodes stay allocated in every measurement
  pick     the case is shown by \\showlists: the source says \\showlists where it should run, and
           this is which `### ` section of the display to take, from 0
  dims     \\ht, \\dp, \\wd of the box are set to these, 0pt for each if not given, so the tests need no TFM
  tokens   how many one-word cells the oracle's token lists in the box occupy, header included.
           These lists are not in `mem` in the port, so its dyn_used does not count them.

Format: fields separated by a tab.  A newline inside a field is byte 0x0C: privacy_guard.py takes 0x0B as a sign of a binary file.

    gen_show_tables.py NAME...    prints those cases, and writes nothing
"""
import os
import re
import struct
import subprocess
import sys
import tempfile
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import gen_mem_tables as gm  # noqa: E402

OUT = gm.OUT

PREAMBLE = r"""\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \catcode`\^=7 \catcode`\_=8
\catcode`\&=4 \catcode`\$=3 \catcode`\~=13
\scrollmode \tracingonline=0 \newlinechar=-1 \errorcontextlines=0
\showboxbreadth=1000 \showboxdepth=1000 \hbadness=10000 \vbadness=10000 \hfuzz=16383pt \vfuzz=16383pt
\pdfoutput=1 \baselineskip=12pt \delcode`(="028300 \delcode`)="029301
\font\xx=cmr10 \font\yy=cmmi10 \font\zz=cmsy10 \font\ww=cmex10
\textfont0=\xx \scriptfont0=\xx \scriptscriptfont0=\xx
\textfont1=\yy \scriptfont1=\yy \scriptscriptfont1=\yy
\textfont2=\zz \scriptfont2=\zz \scriptscriptfont2=\zz
\textfont3=\ww \scriptfont3=\ww \scriptscriptfont3=\ww
\pdfpagewidth=1pt \pdfpageheight=1pt \xx \def\space{ }
"""


def box(name, src, **kw):
    return (name, src, kw)


def rows():
    return [
        # §202: boxes, §204: their glue
        box("hbox_empty", r"\hbox{}"),
        box("vbox_empty", r"\vbox{}"),
        box("hbox_dims", r"\hbox{}", dims=(r"1pt", r"2pt", r"3pt")),
        box("vbox_neg_dims", r"\vbox{}", dims=(r"-1.5pt", r"0.25pt", r"-16383.99998pt")),
        box("glue_set_normal", r"\hbox to 100pt{\hskip 0pt plus 3pt}"),
        box("glue_set_fil", r"\hbox to 100pt{\hskip 0pt plus 3fil}"),
        box("glue_set_fill", r"\hbox to 100pt{\hskip 0pt plus 1fill}"),
        box("glue_set_filll", r"\hbox to 100pt{\hskip 0pt plus 8filll}"),
        box("glue_set_shrink", r"\hbox to 10pt{\kern20pt\hskip 0pt minus 20pt}"),
        box("glue_set_shrink_fil", r"\hbox to 10pt{\kern20pt\hskip 0pt minus 4fil}"),
        box("glue_set_huge", r"\hbox to 16000pt{\hskip 0pt plus 0.5pt}"),
        box("glue_set_huge_fil", r"\hbox to 16000pt{\hskip 0pt plus 0.5fil}"),
        box("glue_set_huge_neg", r"\hbox to 16000pt{\hskip 0pt plus -0.5pt}"),
        box("glue_set_20000", r"\hbox to 10000pt{\hskip 0pt plus 0.5pt}"),
        box("shifted", r"\vbox{\moveright3pt\hbox{}\moveleft1.5pt\vbox{}}"),
        box("glue_set_and_shifted", r"\vbox{\moveright3pt\hbox to 10pt{\hskip 0pt plus 1pt}}"),
        box("display", r"\vbox{\TeXXeTstate=1 \hsize=100pt \parindent=0pt \abovedisplayskip=0pt \belowdisplayskip=0pt \abovedisplayshortskip=0pt \belowdisplayshortskip=0pt \leftskip=0pt \rightskip=0pt \parfillskip=0pt \predisplaypenalty=0 \postdisplaypenalty=0 \noindent$$\hbox{}$$\par}"),
        # §201: characters; §192 print_font_identifier
        box("chars", r"\hbox{a\yy b\xx c}"),
        box("chars_unprintable", r"\hbox{\char0 \char31 \char127 \char32 \char`\\ \ee\char128 \char255 }", setup=r"\font\ee=ecrm1000"),
        box("tracing_fonts", r"\hbox{a\xy b}", setup=r"\pdftracingfonts=1 \font\xy=cmr10 at 12pt"),
        box("font_expand", r"\vbox{\hsize=11pt \parindent=0pt \leftskip=0pt \rightskip=0pt \parfillskip=0pt \pdfadjustspacing=2 \noindent ab\par}", setup=r"\pdffontexpand\xx 20 20 5 autoexpand"),
        box("font_expand_shrink", r"\vbox{\hsize=10.4pt \parindent=0pt \leftskip=0pt \rightskip=0pt \parfillskip=0pt \pdfadjustspacing=2 \noindent ab\par}", setup=r"\pdffontexpand\xx 20 20 5 autoexpand"),
        # §205 rules
        box("rule_all", r"\hbox{\vrule width 1pt height 2pt depth 3pt}"),
        box("rule_running_v", r"\hbox{\vrule}"),
        box("rule_running_h", r"\vbox{\hrule}"),
        box("rule_mixed", r"\vbox{\hrule height 1pt depth 2pt width 3pt\hrule depth 0pt}"),
        # §207-§208 glue, leaders
        box("glue_natural", r"\hbox{\hskip 1pt plus 2pt minus 3pt}"),
        box("glue_orders", r"\hbox{\hskip 1pt plus 2fil minus 3fill\hskip -1pt plus 2filll minus 1pt}"),
        box("glue_zero_parts", r"\hbox{\hskip 0pt\hskip 0pt plus 1pt\hskip 0pt minus 1pt}"),
        box("glue_hfil", r"\hbox{\hfil\hfill\hss\hfilneg}"),
        box("glue_vfil", r"\vbox{\vfil\vfill\vss\vfilneg}"),
        box("glue_baselineskip", r"\vbox{\hbox{}\hbox{}}"),
        box("glue_lineskip", r"\vbox{\hbox{}\hbox{}}", setup=r"\lineskip=3pt \lineskiplimit=20pt"),
        box("leaders_rule", r"\hbox{\leaders\hrule\hskip 1pt}"),
        box("cleaders_box", r"\hbox{\cleaders\hbox{}\hskip 1pt plus 1fil}"),
        box("xleaders_box", r"\vbox{\xleaders\vbox{\kern1pt}\vskip 2pt minus 1pt}"),
        # §209 kerns, margin kerns
        box("kern_explicit", r"\hbox{\kern3pt\kern-3pt\kern0pt}"),
        box("kern_font", r"\hbox{AV}"),
        box("kern_accent", r"\hbox{\accent127 a}"),
        box("kern_auto", r"\hbox{\pdfappendkern=1 \pdfprependkern=1 \knbccode\xx`a=1000 \knaccode\xx`b=2000 ab}"),
        box("margin_kern", r"\vbox{\hsize=100pt \parindent=0pt \leftskip=0pt \rightskip=0pt \parfillskip=0pt \pdfprotrudechars=2 \rpcode\xx`b=500 \lpcode\xx`a=500 \noindent ab\par}"),
        # §210 math
        box("math_plain", r"\hbox{$$}"),
        box("math_surround", r"\hbox{\mathsurround=3pt $$}"),
        box("math_lr", r"\hbox{\TeXXeTstate=1 \beginL\endL\beginR\endR}"),
        box("math_m", r"\vbox{\TeXXeTstate=1 \hsize=100pt \parindent=0pt \leftskip=0pt \rightskip=0pt \parfillskip=0pt \abovedisplayskip=0pt \belowdisplayskip=0pt \abovedisplayshortskip=0pt \belowdisplayshortskip=0pt \predisplaypenalty=0 \postdisplaypenalty=0 \noindent\beginR$$\hbox{}$$\endR\par}"),
        # §211 ligatures
        box("ligature", r"\hbox{fi}"),
        box("ligature_three", r"\hbox{ffi}"),
        # §212 penalties
        box("penalties", r"\hbox{\penalty5 \penalty-100 \penalty0 }"),
        # §213 discretionaries
        box("disc_empty", r"\hbox{\-}"),
        box("disc_full", r"\hbox{\discretionary{a}{\kern1pt b}{c\kern2pt}}"),
        # §214 marks
        box("mark", r"\hbox{\mark{abc}}", tokens=4),
        box("mark_empty", r"\hbox{\mark{}}", tokens=1),
        box("marks_class", r"\hbox{\marks3{x\y z}}", setup=r"\let\y=\relax", tokens=4),
        box("mark_long", r"\hbox{\mark{" + "0123456789" * 8 + r"}}", tokens=81),
        # §215 adjustments, §206 insertions
        box("vadjust", r"\hbox{\vadjust{\kern1pt}}"),
        box("vadjust_pre", r"\hbox{\vadjust pre{\kern1pt\penalty3 }}"),
        box("insert", r"\hbox{\insert100{\kern1pt}}", setup=r"\skip100=0pt \count100=1000 \dimen100=16383.99999pt"),
        box("insert_spec", r"\hbox{\floatingpenalty=7 \splittopskip=1pt plus 2pt minus 3pt \splitmaxdepth=4pt \insert5{\penalty1 \kern-1pt}}", setup=r"\skip5=0pt"),
        # §1603 whatsits
        box("write_streams", r"\hbox{\write3{abc}\write16{d}\write-1{e}}", tokens=8),
        box("openout", r"\hbox{\openout3=foo.bar \openout4=bar }"),
        box("openout_quoted", r'\hbox{\openout3="a b.c" }'),
        box("closeout", r"\hbox{\closeout3 \closeout16 \closeout-1 }"),
        box("special", r"\hbox{\special{abc}\special shipout{d}}", tokens=6),
        box("setlanguage", r"\hbox{\setlanguage3 \lefthyphenmin=5 \righthyphenmin=6 \setlanguage4 }"),
        box("pdfliteral", r"\hbox{\pdfliteral{a}\pdfliteral page{b}\pdfliteral direct{c}\pdfliteral shipout{d}\pdfliteral shipout page{e}\pdfliteral shipout direct{f}}", tokens=12),
        box("pdfcolorstack", r"\hbox{\pdfcolorstack0 set{1 g}\pdfcolorstack0 push{2 g}\pdfcolorstack0 pop\pdfcolorstack0 current}", setup=r"\edef\s{\pdfcolorstackinit page direct{0 g}}", tokens=8),
        box("pdfsetmatrix", r"\hbox{\pdfsetmatrix{1 0}\pdfsave\pdfrestore}", tokens=4),
        box("pdfrefobj", r"\hbox{\pdfrefobj\a\pdfrefobj\b\pdfrefobj\c\pdfrefobj\d}",
            setup=r"\pdfobj{ab}\edef\a{\the\pdflastobj\space}\pdfobj stream{c}\edef\b{\the\pdflastobj\space}\pdfobj stream attr{/A 1}{de}\edef\c{\the\pdflastobj\space}\pdfobj file{f}\edef\d{\the\pdflastobj\space}"),
        box("pdfrefobj_reserved", r"\hbox{\pdfrefobj\a}", setup=r"\pdfobj reserveobjnum \edef\a{\the\pdflastobj\space}"),
        box("pdfrefxform", r"\hbox{\pdfrefxform\a}", setup=r"\setbox0=\hbox to 5pt{}\ht0=6pt \dp0=7pt \pdfxform0 \edef\a{\the\pdflastxform\space}"),
        box("pdfrefximage", r"\hbox{\pdfrefximage\a}", setup=r"\pdfximage width 5pt height 6pt depth 7pt{img.png}\edef\a{\the\pdflastximage\space}"),
        box("pdfannot", r"\hbox{\pdfannot width 1pt height 2pt depth 3pt{a}\pdfannot{b}}", tokens=4),
        box("pdfstartlink_user", r"\hbox{\pdfstartlink width 1pt height 2pt depth 3pt attr{a} user{b}\pdfendlink}", tokens=4),
        box("pdfstartlink_goto_name", r"\hbox{\pdfstartlink goto name{ab}\pdfendlink}", tokens=3),
        box("pdfstartlink_goto_num", r"\hbox{\pdfstartlink goto num 7\pdfendlink}"),
        box("pdfstartlink_goto_file", r"\hbox{\pdfstartlink goto file{f} name{ab}\pdfendlink}", tokens=5),
        box("pdfstartlink_page", r"\hbox{\pdfstartlink goto page 3{xy}\pdfendlink}", tokens=3),
        box("pdfstartlink_page_file", r"\hbox{\pdfstartlink goto file{f} page 3{xy}\pdfendlink}", tokens=5),
        box("pdfstartlink_thread_name", r"\hbox{\pdfstartlink thread name{ab}\pdfendlink}", tokens=3),
        box("pdfstartlink_thread_num", r"\hbox{\pdfstartlink thread num 7\pdfendlink}"),
        box("pdfstartlink_thread_file", r"\hbox{\pdfstartlink thread file{f} num 7\pdfendlink}", tokens=2),
        box("pdfdest_num_xyz", r"\vbox{\pdfdest num 5 xyz}"),
        box("pdfdest_xyz_zoom", r"\vbox{\pdfdest num 5 xyz zoom 1000}"),
        box("pdfdest_xyz_zoom0", r"\vbox{\pdfdest num 5 xyz zoom 0}"),
        box("pdfdest_name", r"\vbox{\pdfdest name{ab} fit}", tokens=3),
        box("pdfdest_struct", r"\vbox{\pdfdest struct 9 num 5 fit}", setup=r"\pdfobj{}"),
        box("pdfdest_types", r"\vbox{\pdfdest num 1 fit\pdfdest num 2 fith\pdfdest num 3 fitv\pdfdest num 4 fitb\pdfdest num 5 fitbh\pdfdest num 6 fitbv}"),
        box("pdfdest_fitr", r"\vbox{\pdfdest num 1 fitr width 1pt height 2pt depth 3pt\pdfdest num 2 fitr}"),
        box("pdfthread", r"\vbox{\pdfthread num 5 \pdfthread name{ab}\pdfthread width 1pt height 2pt depth 3pt attr{c} num 6 }", tokens=5),
        box("pdfstartthread", r"\vbox{\pdfstartthread num 5 \pdfendthread}"),
        box("pdfsavepos", r"\hbox{\pdfsavepos\pdfsnaprefpoint}"),
        box("pdfsnapy", r"\hbox{\pdfsnapy 1pt plus 2pt minus 3pt}"),
        box("pdfsnapycomp", r"\hbox{\pdfsnapycomp 5 }"),
        box("pdfinterword", r"\hbox{\pdfinterwordspaceon\pdfinterwordspaceoff\pdffakespace}"),
        box("pdfrunninglink", r"\hbox{\pdfrunninglinkoff\pdfrunninglinkon}"),
        # §203: unset nodes, only while an alignment is open
        box("unset_cells", r"\vbox{\tabskip=1pt plus 2fil minus 3fill \halign{#&#\cr a&b\cr\noalign{\showlists}}}", pick=0),
        box("unset_span", r"\vbox{\halign{#&#\cr a\span b\cr\noalign{\showlists}}}", pick=0),
        box("unset_stretch", r"\vbox{\halign{#\tabskip=0pt plus 2fil minus 3fill&#\cr\hskip 0pt plus 1pt minus 2fil&\hskip 0pt plus 1fill minus 2filll\showlists\cr}}", pick=1),
        # §866-§873: mlists, only while a formula is open
        box("noads_kinds", r"\hbox{$a\mathop b\mathbin c\mathrel d\mathopen e\mathclose f\mathpunct g\mathinner h\mathord i\showlists$}", pick=0),
        box("noads_limits", r"\hbox{$\mathop a\limits\mathop b\nolimits\mathop c\displaylimits\showlists$}", pick=0),
        box("noad_scripts", r"\hbox{$a^b_c{}^d\showlists$}", pick=0),
        box("noad_box", r"\hbox{$\hbox{}\mathord{\hbox to 1pt{}}\showlists$}", pick=0),
        box("noad_mlist", r"\hbox{${a}{a^b}{}{\mathord{}}\showlists$}", pick=0),
        box("noad_over", r"\hbox{$\overline a\underline b\vcenter{}\showlists$}", pick=0),
        box("noad_radical", r'\hbox{$\radical"161270 a\showlists$}', pick=0),
        box("noad_accent", r'\hbox{$\mathaccent"7016 a\showlists$}', pick=0),
        box("noad_left_right", r"\hbox{$\left(a\right)\showlists$}", pick=0),
        box("noad_middle", r"\hbox{$\left(a\middle)b\right.\showlists$}", pick=0),
        box("noad_left_small_fam", r'\hbox{$\left\delimiter"100200 a\right.\showlists$}', pick=0),
        box("fraction_default", r"\hbox{${a\over b}\showlists$}", pick=0),
        box("fraction_thickness", r"\hbox{${a\above 1pt b}\showlists$}", pick=0),
        box("fraction_atop", r"\hbox{${a\atop b}\showlists$}", pick=0),
        box("fraction_delims", r"\hbox{${a\overwithdelims() b}\showlists$}", pick=0),
        box("fraction_right_only", r"\hbox{${a\abovewithdelims.) 2pt b}\showlists$}", pick=0),
        box("styles", r"\hbox{$\displaystyle\textstyle\scriptstyle\scriptscriptstyle\showlists$}", pick=0),
        box("choice", r"\hbox{$\mathchoice{a}{}{b}{c^d}\showlists$}", pick=0),
        box("math_glue", r"\hbox{$\nonscript\mskip 3mu\mskip 1mu plus 2mu minus 1fil\hskip 1pt\mkern 3mu\kern 1pt\showlists$}", pick=0),
        box("math_nonscript_alone", r"\hbox{$\nonscript\showlists$}", pick=0),
        box("math_disc_penalty", r"\hbox{$\discretionary{a}{}{}\penalty 3 \showlists$}", pick=0),
        box("math_vcenter_body", r"\hbox{$\vcenter{\kern1pt}\showlists$}", pick=0),
        # §200 and §216: the limits of the display.  The tests set \showboxdepth and \showboxbreadth as these do.
        box("limit_depth0", r"\hbox{\vadjust{\kern1pt}\kern1pt}", setup=r"\showboxdepth=0"),
        box("limit_depth1", r"\hbox{\vadjust{\kern1pt}\kern1pt}", setup=r"\showboxdepth=1"),
        box("limit_depth_negative", r"\hbox{\kern1pt}", setup=r"\showboxdepth=-1"),
        box("limit_depth_max", r"\hbox{\vadjust{\kern1pt}\kern1pt}", setup=r"\showboxdepth=2147483647"),
        box("limit_breadth1", r"\hbox{\penalty1 \penalty2 \penalty3 }", setup=r"\showboxbreadth=1"),
        box("limit_breadth2", r"\hbox{\penalty1 \penalty2 \penalty3 }", setup=r"\showboxbreadth=2"),
        box("limit_breadth3", r"\hbox{\penalty1 \penalty2 \penalty3 }", setup=r"\showboxbreadth=3"),
        box("limit_breadth0", r"\hbox{\penalty1 \penalty2 \penalty3 \penalty4 \penalty5 \penalty6 }", setup=r"\showboxbreadth=0"),
        box("limit_breadth_negative", r"\hbox{\penalty1 \penalty2 \penalty3 \penalty4 \penalty5 \penalty6 }", setup=r"\showboxbreadth=-3"),
        box("limit_breadth_max", r"\hbox{\penalty1 \penalty2 }", setup=r"\showboxbreadth=2147483647"),
        box("limit_depth_noad", r"\hbox{$a^b\showlists$}", setup=r"\showboxdepth=0", pick=0),
        box("limit_depth_noad1", r"\hbox{$\mathord{\hbox{\kern1pt}}^b\showlists$}", setup=r"\showboxdepth=1", pick=0),
    ]


def tiny_png(path):
    """A one-pixel grey PNG, for \\pdfximage."""
    def chunk(t, b):
        c = t + b
        return struct.pack(">I", len(b)) + c + struct.pack(">I", zlib.crc32(c) & 0xFFFFFFFF)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 0, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(b"\x00\x80")) + chunk(b"IEND", b""))


def source(src, kw, extra=""):
    text = kw.get("setup", "") + "\n\\setbox1=" + src + "\n"
    if "pick" not in kw:
        text += "\\ht1=%s \\dp1=%s \\wd1=%s\n" % kw.get("dims", ("0pt", "0pt", "0pt"))
    return text + extra


def run(body, tmp, job="t"):
    path = os.path.join(tmp, job + ".tex")
    with open(path, "wb") as f:
        f.write((PREAMBLE + body + "\n\\csname bye\\endcsname\\end\n").encode("latin-1"))
    subprocess.run(["pdftex", "-ini", "-etex", "-interaction=nonstopmode", job + ".tex"], cwd=tmp,
                   stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
    with open(os.path.join(tmp, job + ".log"), "rb") as f:
        return f.read().decode("latin-1")


def errors_in(log):
    return re.findall(r"^! (?!OK\.).*$", log, re.M)


def shown_box(log, reg=1):
    m = re.search(r"^> \\box%d=(.*?)\n\n! OK\." % reg, log, re.M | re.S)
    return None if m is None else m.group(1)


def shown_list(log, k):
    parts = re.split(r"^### .*\n", log, flags=re.M)
    if len(parts) < k + 2:
        return None
    text = parts[k + 1]
    lines = []
    for line in text.split("\n"):
        if line.startswith(("prevdepth", "this will", "spacefactor", "! OK", "prevgraf", "goal", "total")):
            break
        if line == "":
            break
        lines.append(line)
    return "\n" + "\n".join(lines)


def show_case(name, src, kw, tmp):
    body = source(src, kw, "\\showbox1\n" if "pick" not in kw else "")
    log = run(body, tmp)
    err = errors_in(log)
    if err:
        sys.exit("case %s: %s\n%s" % (name, err, log[-2000:]))
    text = shown_list(log, kw["pick"]) if "pick" in kw else shown_box(log)
    if text is None:
        sys.exit("case %s: nothing shown\n%s" % (name, log[-2000:]))
    return text


def copy_show_case(name, src, kw, tmp):
    """What \\showbox says of a copy, made while the original is alive, as in the test in show.rs."""
    log = run(source(src, kw, "\\setbox2=\\copy1 \\showbox2\n"), tmp)
    err = errors_in(log)
    if err:
        sys.exit("case %s: %s\n%s" % (name, err, log[-2000:]))
    text = shown_box(log, 2)
    if text is None:
        sys.exit("case %s: nothing shown\n%s" % (name, log[-2000:]))
    return text


def short_case(name, src, kw, tmp):
    """§839: Underfull \\hbox prints short_display of the list, and then show_box.  Only for an hbox made by us."""
    log = run(kw.get("setup", "") + "\n\\hbadness=-1 \\setbox1=\\hbox to 16000pt" + src[len(r"\hbox"):] + "\n", tmp)
    m = re.search(r"^Underfull \\hbox \(badness \d+\) detected at line \d+\n(.*?)\n\n?\\hbox\(", log, re.M | re.S)
    if m is None:
        sys.exit("case %s: no Underfull\n%s" % (name, log[-2000:]))
    return m.group(1)


def mem_case(name, src, kw, tmp):
    """§1489's statistics, as the format has them, in the four stages of the test in copy.rs."""
    base = source(src, kw)

    def stats(extra):
        s, _ = gm.run_dump(base + extra, tmp, preamble=PREAMBLE)
        return s["var_used"], s["dyn_used"]

    va, da = stats("")
    vb, db = stats("\\setbox2=\\copy1\n")
    vc, dc = stats("\\setbox2=\\copy1\n\\setbox2=\\box255\n")
    vd, dd = stats("\\setbox1=\\box255\n")
    if (vc, dc) != (va, da):
        sys.exit("case %s: flushing the copy does not give back what it took: %r %r" % (name, (va, da), (vc, dc)))
    toks = kw.get("tokens", 0)
    return vb - va, db - da, va - vd, da - dd - toks


def field(s):
    return s.replace("\n", "\x0c")


def main():
    os.makedirs(OUT, exist_ok=True)
    only = set(sys.argv[1:])
    with tempfile.TemporaryDirectory() as tmp:
        tiny_png(os.path.join(tmp, "img.png"))
        show_rows, copy_rows, short_rows, copy_show_rows = [], [], [], []
        for name, src, kw in rows():
            if only and name not in only:
                continue
            text = show_case(name, src, kw, tmp)
            show_rows.append("%s\t%s\n" % (name, field(text)))
            if only:
                print("=== %s\n%s" % (name, text))
            if "pick" not in kw:
                copy_rows.append("%s\t%d\t%d\t%d\t%d\n" % ((name,) + mem_case(name, src, kw, tmp)))
                shown = copy_show_case(name, src, kw, tmp)
                if shown != text:
                    copy_show_rows.append("%s\t%s\n" % (name, field(shown)))
                if only:
                    print("    mem", copy_rows[-1].strip())
            if src.startswith(r"\hbox{") and "pick" not in kw and name in SHORT:
                short_rows.append("%s\t%s\n" % (name, field(short_case(name, src, kw, tmp))))
                if only:
                    print("    short", short_rows[-1].strip())
    if only:
        return
    for fn, data in (("show_oracle.tsv", show_rows), ("copy_oracle.tsv", copy_rows), ("short_oracle.tsv", short_rows),
                     ("copy_show_oracle.tsv", copy_show_rows)):
        with open(os.path.join(OUT, fn), "wb") as f:
            f.write("".join(data).encode("latin-1"))
    print("show:", len(show_rows), "cases; copy:", len(copy_rows), "cases; short:", len(short_rows), "cases; copies that differ:",
          [r.split("\t")[0] for r in copy_show_rows])


SHORT = {"chars", "chars_unprintable", "rule_all", "glue_natural", "glue_zero_parts", "kern_explicit", "kern_font", "math_plain", "math_lr",
         "ligature", "ligature_three", "penalties", "disc_empty", "disc_full", "mark", "vadjust", "insert", "write_streams"}


if __name__ == "__main__":
    main()
