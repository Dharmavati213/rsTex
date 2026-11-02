# rstex: verification plan and feature inventory

All results are verified by running on this machine unless marked INFERRED or RECOMMENDATION.

## 0. Headlines

- **Primitive scope:** of 157 pdfTeX-class primitives in the binary, 110 can be jointly undefined. The format still builds with 0 errors and all 3 PDFs stay identical. That leaves a hard core of 47.
- **e-TeX:** implement all 66, because e-TRIP requires them. 30 of the non-Knuth primitives are required by LaTeX or packages.
- **Font expansion is not optional.** microtype's default turns it on in all 3 documents.
  - In the thesis, 1293 of 2441 `Tm` operators have horizontal scale other than 1 (1.02, 0.98, ...).
  - `\pdffontexpand` runs 18–27 times per document and `\lpcode`/`\rpcode` about 2000 times each.
- **File formats:**
  - No VF, PNG, PDF inclusion, PK, TrueType or JBIG2 in the 3 documents.
  - Mandatory: TFM, PFB, ENC, MAP, JPEG (one CMYK image in the thesis).
- **TRIP and e-TRIP are feasible.**
  - Control: `/usr/bin/tex` passes TRIP after the upstream filter, with zero diffs.
  - System pdftex passes with `main_memory` ≥ 3800 and a bigger string pool, leaving 2 residual diff classes in TRIP and 3 in e-TRIP.
- **DVI comparison is useless for real documents.** The thesis in DVI mode switches drivers, loses font expansion, and all 34 pages differ. Keep DVI for TRIP and early rungs only.
- **Byte-level comparison is achievable.** With `\pdfcompresslevel=0 \pdfobjcompresslevel=0 \pdftrailerid{}` plus `SOURCE_DATE_EPOCH=1700000000 FORCE_SOURCE_DATE=1`, two runs in different directories are byte-identical (LMB, 1,129,934 bytes).
- **Refs gap:** `refs/` lacks change files that define primitives LaTeX really uses (section 1.7).

## 1. Inventory

### 1.1 Method

| Method | What it gives | Limit |
|---|---|---|
| Binary probe | 543 primitives with `-ini -etex` (473 without `-etex`), 555 with `-enc -mltex`. Classes: TeX82 316, e-TeX 66, pdfTeX 157, web2c-new 4 | |
| Trace | `\tracingcommands=2 \tracingmacros=2`, log streamed through a FIFO. Log sizes: thesis 766 MB, LMB 743 MB, SGA 146 MB, format build 3.9 GB | Does not see internal-quantity reads such as `\pdflastobj` |
| Static scan | Every loaded file in `.fls`, plus the requested package files | Mention does not imply use |
| Removal experiment (decisive) | For each of the 227 non-Knuth primitives: build a format with it undefined, compile 3 documents, run `compare_pdf.py` | Proves only this format and these 3 documents |

Control run: format rebuilt with nothing removed gave 3/3 identical PDFs.

A trap for the harness: microtype sets `\escapechar=-1`, so traces print `{lpcode}` and `MT@font` without a backslash. My first analyzer missed this and wrongly concluded expansion was unused.

### 1.2 Removal results (227 primitives)

**A. Format build fails or never ends (20):** `currentgrouplevel detokenize dimexpr eTeXversion expanded ifcsname ifdefined numexpr pdfcompresslevel pdfdecimaldigits pdffilesize pdfglyphtounicode pdfhorigin pdfpkresolution pdfstrcmp pdfvorigin protected readline unexpanded unless`
- `unless` hung: rc=124 at a 40 s timeout.
- Gate messages seen: "LaTeX requires e-TeX", "expl3 error: Required primitives not found", "LaTeX requires the e-TeX primitives and additional...".

**B. Errors and/or changed PDF (43):** `currentgrouptype eTeXrevision everyeof fontcharwd glueexpr iffontchar leftmarginkern lpcode marks pdfadjustspacing pdfcatalog pdfcolorstack pdfdest pdfendlink pdfescapestring pdffontexpand pdfinfo pdflastobj pdflastximage pdflinkmargin pdfliteral pdfmatch pdfobj pdfoutline pdfoutput pdfpageheight pdfpageresources pdfpagewidth pdfprotrudechars pdfrefximage pdfrestore pdfsave pdfsetmatrix pdfstartlink pdftexversion pdfunescapehex pdfximage rightmarginkern rpcode scantokens splitbotmarks splitfirstmarks tracingnesting`

**C. Silent PDF change (1):** `pdfgentounicode`. It is set in the format, so `/ToUnicode` CMaps are required (every font shows uni=yes).

**D. PDF identical but errors logged (13):** `gluestretchorder lastnodetype muexpr partokencontext pdfcreationdate pdfminorversion pdfobjcompresslevel pdfshellescape pdftexrevision tracingassigns tracinggroups tracingifs tracingscantokens`

**E. Invisible alone (150):** everything else.
- Jointly removing E ∩ pdfTeX (110 primitives) passes: format 0 errors, 3/3 identical.
- Jointly removing all 150 fails: format errors plus 26–43 errors per document ("You can't use `relax' after \the"). Single-removal invisibility does not compose for the e-TeX part.

### 1.3 Required pdfTeX-class core (47)

`expanded leftmarginkern lpcode pdfadjustspacing pdfcatalog pdfcolorstack pdfcompresslevel pdfcreationdate pdfdecimaldigits pdfdest pdfendlink pdfescapestring pdffilesize pdffontexpand pdfgentounicode pdfglyphtounicode pdfhorigin pdfinfo pdflastobj pdflastximage pdflinkmargin pdfliteral pdfmatch pdfminorversion pdfobj pdfobjcompresslevel pdfoutline pdfoutput pdfpageheight pdfpageresources pdfpagewidth pdfpkresolution pdfprotrudechars pdfrefximage pdfrestore pdfsave pdfsetmatrix pdfshellescape pdfstartlink pdfstrcmp pdftexrevision pdftexversion pdfunescapehex pdfvorigin pdfximage rightmarginkern rpcode`

### 1.4 Treatment of the 110 removable primitives (RECOMMENDATION)

Passing this test only says these 3 documents do not need them. Ordinary documents reach many of them.

**Implement properly, low cost:**
- microtype options: `efcode knaccode knbccode knbscode shbscode stbscode tagcode pdfadjustinterwordglue pdfprependkern pdfappendkern pdfnoligatures letterspacefont pdfcopyfont pdffontsize`
- Conditionals and detection: `ifincsname ifpdfabsdim ifpdfabsnum ifpdfprimitive pdfprimitive ignoreprimitiveerror quitvmode pdfdraftmode`
- String and file: `pdfescapehex pdfescapename pdflastmatch pdfmdfivesum pdffiledump pdffilemoddate pdftexbanner pdfmajorversion`
- Time and RNG: `pdfelapsedtime pdfresettimer pdfrandomseed pdfsetrandomseed pdfuniformdeviate pdfnormaldeviate`
- Annotations and page objects: `pdfannot pdflastannot pdflastlink pdfcolorstackinit pdfpageattr pdfpagesattr pdfnames pdfpageref pdfrefobj pdfrunninglinkoff pdfrunninglinkon`
- pgf paths: `pdfxform pdfrefxform pdflastxform pdfxformname pdfsavepos pdflastxpos pdflastypos`
- Images: `pdflastximagepages pdflastximagecolordepth pdfximagebbox pdfpagebox pdfinclusioncopyfonts pdfinclusionerrorlevel pdfimageresolution`
- Fonts and map: `pdfmapfile pdfmapline pdfincludechars pdffontattr pdffontname pdffontobjnum`
- Output control: `pdfinfoomitdate pdftrailerid pdfsuppressptexinfo pdftrailer pdfomitcharset pdfnobuiltintounicode pdfpxdimen`
- Paragraph builder: `pdfignoreddimen pdffirstlineheight pdflastlinedepth pdfeachlineheight pdfeachlinedepth`

**Accept the assignment, then ignore or raise an error when used:**
`pdfmovechars pdfsnaprefpoint pdfsnapy pdfsnapycomp pdfthread pdfstartthread pdfendthread pdfthreadmargin pdfgamma pdfimagegamma pdfimagehicolor pdfimageapplygamma pdfforcepagebox pdfoptionalwaysusepdfpagebox pdfoptionpdfinclusionerrorlevel pdffakespace pdfinterwordspaceon pdfinterwordspaceoff pdfspacefont pdfpkmode pdftracingfonts pdfuniqueresname pdfsuppresswarningdupdest pdfsuppresswarningdupmap pdfsuppresswarningpagegroup pdfptexuseunderscore pdfomitinfodict pdfomitprocset pdfinsertht pdfdestmargin pdfretval`

### 1.5 Per-package static use (non-Knuth primitives)

| Package | Count | Primitives |
|---|---|---|
| latex.ltx | 38 | `currentgrouplevel currentgrouptype detokenize dimexpr eTeXversion everyeof expanded gluestretchorder ifcsname ifdefined ifincsname ignoreprimitiveerror interactionmode lastnodetype marks numexpr partokencontext pdfannot pdffilesize pdfgentounicode pdfhorigin pdftexrevision pdftexversion pdfvorigin protected showtokens splitbotmarks splitfirstmarks tracingassigns tracinggroups tracingifs tracingnesting tracingscantokens tracingstacklevels unexpanded unless`, plus `charsubdef mubyte` mentioned |
| expl3-code | 217 | Aliases all, but 110 pdfTeX-class are mapped and only 6 are used in the kernel: `expanded:22 pdfstrcmp:6 pdftexversion:5 pdftexrevision:1 pdfmapfile:1 pdfmapline:1` |
| l3backend-pdftex.def | 24 | `pdfannot pdfcatalog pdfcolorstack pdfcolorstackinit pdfcompresslevel pdfdest pdfendlink pdfinfo pdflastannot pdflastlink pdflastobj pdflinkmargin pdfliteral pdfmajorversion pdfminorversion pdfobj pdfobjcompresslevel pdfpageref pdfrestore pdfrunninglinkoff pdfrunninglinkon pdfsave pdfsetmatrix pdfstartlink` |
| hyperref (hpdftex.def, hyperref.sty) | 39 | `dimexpr ifcsname lastnodetype numexpr pdfcatalog pdfdest pdfendlink pdfescapestring pdfhorigin pdfinfo pdflastannot pdflastlink pdflastmatch pdflastobj pdflastxform pdflastximage pdflinkmargin pdfliteral pdfmajorversion pdfmatch pdfminorversion pdfobj pdfobjcompresslevel pdfoptionpdfminorversion pdfoutline pdfpageattr pdfpageheight pdfpageref pdfpageresources pdfpagesattr pdfpagewidth pdfrefobj pdfstartlink pdfstrcmp pdftexversion pdfvorigin pdfxform protected unexpanded` |
| microtype (.sty, -pdftex.def) | 39 | `currentgrouplevel currentgrouptype currentiflevel detokenize dimexpr eTeXversion efcode fontcharwd ifcsname ifdefined iffontchar knaccode knbccode knbscode lastnodetype leftmarginkern letterspacefont lpcode numexpr partokencontext pdfadjustinterwordglue pdfadjustspacing pdfappendkern pdfcopyfont pdffontexpand pdffontsize pdfmatch pdfnoligatures pdfoutput pdfprependkern pdfprotrudechars pdfstrcmp pdftexrevision pdftexversion rightmarginkern rpcode shbscode stbscode tagcode` |
| geometry | 4 | `pdfhorigin pdfpageheight pdfpagewidth pdfvorigin` |
| graphicx, graphics, pdftex.def | 17 | `currentgrouplevel detokenize pdfcolorstack pdffilesize pdflastxform pdflastximage pdfliteral pdfpageheight pdfpagewidth pdfrefxform pdfrefximage pdfrestore pdfsave pdfsetmatrix pdfxform pdfximage protected` |
| xcolor | 2 | `pdfliteral protected` |
| babel | 13 | `TeXXeTstate beginL beginR currentgrouplevel currentgrouptype detokenize endL endR ifcsname ifincsname numexpr scantokens unexpanded` |
| xy (including xypdf.tex) | 13 | `detokenize dimexpr ifpdfabsdim numexpr pdfcolorstack pdfdecimaldigits pdfliteral pdfrestore pdfsave pdfsetmatrix protected unexpanded unless` |
| pgfsys-pdftex.def | 16 | `pdflastobj pdflastxform pdflastximage pdflastxpos pdflastypos pdfliteral pdfobj pdfpageheight pdfpageresources pdfpagewidth pdfrefobj pdfrefxform pdfrefximage pdfsavepos pdfxform pdfximage` |
| pgf (rest) | 18 | `detokenize dimexpr eTeXrevision eTeXversion expanded glueexpr ifcsname ifdefined muexpr numexpr pdfelapsedtime pdfmdfivesum pdfshellescape pdfstrcmp pdftexversion protected scantokens unexpanded` |

Execution counts for the design of hot paths, from `\tracingcommands`:

| Scope | Count |
|---|---|
| Total commands traced, format build | 19.39 M |
| Total, thesis | 7.32 M |
| Total, LMB | 6.99 M |
| Total, SGA | 1.79 M |

Thesis top: `expandafter 1.74M, ifx 814k, fi 791k, let 699k, def 370k, else 286k, csname 280k, futurelet 189k, edef 183k, relax 172k, string 141k, global 123k, ifnum 111k`.

Non-Knuth in thesis: `ifcsname 98k, detokenize 60k, expanded 18k, pdfstrcmp 7.6k, ifdefined 6.2k, protected 4.4k, pdfliteral 2.6k, rpcode 2.0k, lpcode 1.8k, unless 1.7k, pdfcolorstack 599, pdffilesize 194, pdfescapestring 212, pdfdest 129, pdfmatch 61`.

`\pdfmatch` is POSIX ERE through `regcomp(REG_EXTENDED)` (`utils.c:817`), with captures through `\pdflastmatch`. INFERRED: Rust's `regex` crate has leftmost-first semantics, not POSIX leftmost-longest, so check this if it is used.

### 1.6 What can be dropped

| Feature | Evidence | Decision (RECOMMENDATION) |
|---|---|---|
| encTeX | `\mubyte` family exists only with `-enc`; `fmtutil.cnf` line 60 passes no `-enc` | Drop |
| MLTeX | `\charsubdef` family exists only with `-mltex` | Drop |
| IPC, src-specials | CLI flags only | Drop |
| TCX reader | No `.tcx` in document-run `.fls`; `cp227.tcx` (161 lines) used at format build | Hardcode the table; TRIP needs the default table instead |
| mktex* | No generated files in this corpus | Drop; report missing files |
| PK, Type 3 | All fonts Type 1 per pdffonts; `pkin.c` 425 + `writet3.c` 429 lines. `ecrm1000` maps to cm-super pfb, so T1 without lmodern stays Type 1. Map has 64 no-embed entries | Error out; `\pdfpkresolution` must exist as a register because `pdftexconfig.tex` sets it |
| TrueType, OTF, subfont | `pdftex.map`: ttf 0, otf 0, sfd 0, pfb 4360; `writettf.c` 1463 + `subfont.c` 228 lines | Drop |
| JBIG2 | `writejbig2.c` 834 lines, no use | Error out |

These drops remove 3379 of the 12364 C/C++ lines in `refs/pdftex`.

| Deferred | Lines | Notes |
|---|---|---|
| PNG | `writepng.c` 677 | Not in the 3 documents |
| PDF inclusion | `pdftoepdf.cc` 1142 + `epdf.c` 111 | Only in a Lemma fixture (`syntax.tex: img/plot.pdf`) |
| VF | `vfpacket.c` 107 + `pdftex.web` §32e around lines 17708–18000 | `\letterspacefont` builds an internal VF there, so `\textls` needs it |

### 1.7 File formats actually read

| Document | Unique files | Bytes | tfm | pfb | enc | map | fd | Other |
|---|---|---|---|---|---|---|---|---|
| thesis | 215 | 8.47 MB | 32 | 19 | 5 | 2 | 9 | 1 jpg, aux, toc, bbl |
| LMB | 235 | 9.05 MB | 38 | 24 | 5 | 2 | 9 | aux, toc |
| SGA1-VI | 148 | 6.14 MB | 46 | 17 | 5 | 2 | 7 | aux, toc |

- **Virtual fonts: not needed.** Zero `.vf`, `.pk`, `.png`, `.pdf`, `.ttf`, `.otf`, `.mps`, `.eps` in any `.fls`. `fonts/vf/public/lm` does not exist here, and 665 `.vf` exist elsewhere in the tree.
- **Map:** `/var/lib/texmf/fonts/map/pdftex/updmap/pdftex.map` has 4424 lines and 325,717 bytes; `texfonts.map` is 3.5 KB. 146 entries use Slant or Extend.
- **Encodings seen:** `lm-ec lm-rm lm-mathit lm-mathsy lm-mathex`.
- **Fonts:** Type 1 only, plus `rsfs10/5` in LMB and `xycmat10`, `xycmbt10` in SGA.
- **Image:** one JPEG, CMYK, 8 bpc, 829×379, 656 KB.

**PDF writer requirements** (thesis): PDF 1.7, 496 objects, 413 in 5 object streams, xref stream, 129 named dests, `\pdfcompresslevel=9`, `\pdfobjcompresslevel=2`, `\pdfdecimaldigits=3`, ToUnicode on all fonts, Type 1 subsetting. Format-time: `\pdfglyphtounicode` 5505 calls, `\readline` 4942 calls, TCX, `language.dat`.

**SyncTeX:** the Lemma build uses it (`.build/main.synctex.gz`). It is not needed for PDF equality; add a rung with paths normalized.

**Refs gap** (from upstream `texk/web2c/pdftexdir/am/pdftex.am`). The real binary is built from `pdftex.web` plus these change files. Present in refs: `tex.ch0 tex.ch pdftex.ch`. Missing:

| Missing file | Why it matters |
|---|---|
| `tracingstacklevels.ch` | Defines `\tracingstacklevels` |
| `partoken-102.ch`, `partoken.ch` | Define `\partokenname`, `\partokencontext`. `latex.ltx:22441` sets `\partokencontext=2`; removal gives 177–324 errors per document |
| `showstream.ch` | Defines `\showstream` (`expl3-code.tex:12062`) |
| `locnull-optimize.ch` | Optimization, `loc=null` implies `state=token_list` |
| `unbalanced-braces.ch` | `\write`/`\output` overrun defense |
| `pdftexdir/char-warning-pdftex.ch` | `\tracinglostchars>=3` becomes an error |
| `zlib-fmt.ch`, `tex-binpool.ch`, `enctexdir/*`, the synctex change files | Not typesetting semantics, or for droppable features |

Also missing for trip: `tripman.tex`, `tripos.tex`, both `texmf.cnf`, `etrip3.in`, `etrip.out`, `triptest.test`, `etriptest.test`. I fetched most of these (not the synctex change files) to `/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/verification/upstream/`, and they should be moved into `refs/`.

## 2. TRIP and e-TRIP

**Script:** `/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/verification/trip/run_trip.sh REFS WORK ENGINE trip|etrip`

It embeds `texmf.cnf` and the upstream sed filters.

```
TRIP_MEM=3999 TRIP_STRINGS=6000 TRIP_POOL=90000 ETRIP_STRINGS=6000 ETRIP_POOL=90000 \
  run_trip.sh /home/site/Projects/rstex/refs WORK /usr/bin/pdftex etrip
```

Golden outputs from pdftex are in `/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/verification/trip/final-pdftex/`.

**Required configuration:**
- tripman Appendix A: `mem_min=mem_bot=1`, `mem_top=mem_max=3000`, `error_line=64`, `half_error_line=32`, `max_print_line=72`, default unprintable table (§49).
- TeX Live cnf: `max_strings=3000 string_vacancies=8000 pool_size=40000 font_mem_size=20000 font_max=75 stack_size=200 nest_size=40 buf_size=500 save_size=600 dvi_buf_size=800`.
- e-TRIP: `main_memory=3999`, `mem_bot=0`, `max_strings=3300`, `pool_size=32000`.
- Input: `trip1.in` (blank line, `\input trip`), `trip2.in` (` &trip  trip `), `etrip1.in` (`*\input trip`), `&trip \toksdef\tokens=0 \input trip`, `*etrip`, `&etrip etrip`.
- `dvitype -output-level=2 -dpi=72.27 '-page-start=*.*.*.*.*.*.*.*.*.*'`.
- Exit status 1 is normal on every engine step because the test contains errors.

**pdftex needs two changes from Knuth's values:**
- Pool: 40000/3000 fails with "You have to increase POOLSIZE". 90000/6000 works; the minimum was not searched.
- Main memory: 3000–3700 overflows ("TeX capacity exceeded, sorry [main memory size=3000]" at `trip.tex` l.285). 3800, 3900, 3999, 5000 all give identical diff counts. `mem_bot=0` and `1` give a byte-identical `trip.log`.

**Results after the upstream filter (`filter_etex` also masks memory figures):**

| Step | `/usr/bin/tex` control | `/usr/bin/pdftex` |
|---|---|---|
| `trip.pl` round trip | identical | identical |
| `tripin.log` | identical | 8 lines in 3 hunks: banner; 480 vs 471 locations, usage 104&279 vs 96&278; 500 vs 341 multiletter; 2 extra lines "0 words of pdfTeX memory", "0 indirect objects" |
| `trip.log` | identical | 40 lines in 19 hunks: 16 memory lines, banner, words-of-memory; **2 remain after masking** |
| `trip.fot` | identical | identical (with memory mask) |
| `trip.typ` | identical | identical |
| `tripos.tex`, `8terminal.tex` | as expected | identical to upstream (59 bytes), empty |
| extended `trip.log` vs master | not run | 44 lines in 24 hunks, listed below |
| extended `trip.typ` | not run | identical |
| `etrip.pl` round trip | not run | identical |
| `etripin.log` | not run | 8 lines in 3 hunks: banner; 283 vs 282 locations, usage 36&169 vs 36&168; 570 vs 408 multiletter; the 2 extra stat lines |
| `etrip.log` | not run | 14 lines in 7 hunks; **3 hunks remain after masking** |
| `etrip.fot`, `etrip.typ`, `etrip.out` | not run | identical |

**The real residuals in pdftex:**
- `trip.log:2652`: `cm, mm, dd, cc, nd, nc, bp, or sp` (extra units `nd`, `nc`).
- `trip.log:7301`: 531 vs 372 multiletter control sequences.
- `etrip.log:589`: `\insert0{` vs `\vadjust{` in a group display.
- `etrip.log:4422`: 571 vs 409 multiletter control sequences.
- `etrip.log:4425`: `80s` vs `79s` save stack.

**Extended mode** shows the e-TeX differences listed in `refs/etrip/etrip.diffs`: `, display` flags, removed `{restoring ...}` lines, `or \protected`, the `### semi simple group` warning, and `136b,40s`. It adds the `nd, nc` line and a count of 601 vs 372.

**Glue set:** pdftex prints `16341.99998fil` and `9737.58742fil` where Knuth's master has `16342.0` and `9737.58789`, and the upstream filter masks these. `GLUERATIO_TYPE` defaults to `double` (`texmfmp.h:186`), so use f64 to match the oracle. `trip.typ` differs the same way (`down4 639342177`).

**Acceptance for rstex (RECOMMENDATION):**
1. Golden file is system pdftex under the same cnf: zero diffs after masking banner, `Memory usage`, `memory locations dumped`, `words of memory`, and string counts.
2. Against Knuth's master: only the whitelist above.

This requires a DVI back end, INITEX mode, compatibility mode (70 primitives hidden unless `*` or `-etex`), and exact terminal output. A growable memory removes the overflow issue, and nothing else in trip varied with `main_memory` from 3800 to 5000 or between `mem_bot` 0 and 1.

## 3. Differential harness

### 3.1 Levels

Run L4 first because it is cheap, then drill down on divergence.

| Level | Check | Verified facts |
|---|---|---|
| L0 side files | `.aux .toc .out .bbl` bytes, `.fls` order | thesis `.aux` identical across runs |
| L1 macros | `\tracingcommands=2 \tracingmacros=2`, `max_print_line=1000000`, stream through a FIFO, stop at first difference | 766 MB and about 25 s for the thesis. On demand only |
| L2 boxes | `\tracingoutput=1 \showboxbreadth=2147483647 \showboxdepth=2147483647` | LMB: 3.5 MB, 115k lines, 2.7–3.3 s vs 1.4 s. Two runs differ only in 4 output-dir path lines |
| L3 DVI | dvitype | sample2e byte-identical under `SOURCE_DATE_EPOCH`. Thesis in DVI mode unusable, see below |
| L4 PDF content | `compare_pdf.py` | See 3.4 |
| L5 PDF structure | `compare_pdf.py` | Info, catalog, outlines, dests, fonts, ToUnicode, FontFile hash |
| L6 bytes | Uncompressed `cmp` | LMB byte-identical across directories. Requires identical object numbering. INFERRED: default compression would also need a zlib-identical deflate, so compare uncompressed |
| L7 raster | `pdftoppm -r 100` | One-letter change: 0.2032% on page 17. Safety net only |

Thesis under DVI mode:
- It loads `dvips.def` and `hdvips.def`.
- microtype reports "No font expansion".
- It raises "Cannot determine size of graphic" on its JPEG.
- 34/34 box logs differ from PDF mode.

**First divergence flow (RECOMMENDATION):**
1. L4 gives page and operator.
2. L2 gives the first differing box line on that page, with depth from the dots.
3. If L2 is equal, the bug is in the back end (`pdf_hlist_out`, font writer).
4. If L2 differs, run L1 streamed, narrowing with `\tracingmacros` around that page, and report macro name and `l.NNN`.

Verified example, the one-letter mutation in LMB:
- L4 reported page 17, operator #1154, `(isomorphisms.)` vs `(isomorphisns.)`.
- L2's first real difference was log line 56770, `glue set 133.31723fill` vs `136.3586fill`.

### 3.2 Normalization

- **PDF, plain runs:** differ in exactly 70 bytes (`/CreationDate`, `/ModDate`, `/ID`).
- **PDF, handled by the script:** object numbers, compression, `/F48`-style font names (BaseFont minus tag plus encoding hash), XObject names (content hash), ExtGState and ColorSpace names, page refs (become page numbers), subset tags, dates, producer, banner.
- **Log:** banner line and date, `restricted \write18 enabled`, `entering extended mode`, output-dir paths, line wrap, `Here is how much of TeX's memory`, `PDF statistics`. `normalize_log` in `l3build-check.lua:111` has further rules worth adopting.
- **Determinism:** `SOURCE_DATE_EPOCH=1700000000 FORCE_SOURCE_DATE=1` gives byte-identical PDF and log in the same directory. `\pdftrailerid{}` removes the path-dependent `/ID`.
- INFERRED: RNG primitives are time-seeded, so documents using them need `\pdfsetrandomseed`.

### 3.3 Ladder (oracle timings, load about 3)

Driver: `/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/verification/harness/run_ladder.sh [CANDIDATE]`

Documents: `/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/verification/harness/ladder/`

Oracle against oracle: all 24 rungs IDENTICAL, 0 errors. Rung order is the suggested gate order, with TRIP and e-TRIP as rung -1. Bibliography for `summary` and `main` comes from a copied `.bbl`.

| Rung | ms | pp | Adds |
|---|---|---|---|
| 00-plain-hello | 33 | 1 | Startup, Type 1 embed |
| 01-plain-story | 34 | 1 | Paragraphs |
| 02-plain-math | 40 | 2 | Math, `\halign`, insert, marks |
| 03-plain-texweb (woven `tex.web`) | 504 | 536 | Scale without LaTeX; also a benchmark |
| 04-latex-hello | 69 | 1 | Format load |
| 05-latex-sample2e | 85 | 3 | LaTeX basics |
| 06-latex-t1-lmodern | 95 | 1 | T1, enc, lm fonts |
| 07-amsmath-suite | 123 | 1 | amsmath, amsthm, mathtools, msam, msbm, rsfs |
| 08-layout-babel-etc | 129 | 1 | babel, enumitem, xcolor, geometry, url |
| 09-expl3-xparse | 78 | 1 | l3fp, l3regex, `\pdfmdfivesum`, `\pdfuniformdeviate` |
| 10-microtype-protrusion | 126 | 2 | Protrusion |
| 11-microtype-full | 132 | 2 | Expansion |
| 12-graphics | 101 | 1 | JPEG, PNG, PDF inclusion, rotate |
| 13-hyperref | 184 | 1 | Links, outlines, dests, ObjStm |
| 14-tikzcd-one | 216 | 1 | pgf |
| 15-tikzcd-many | 1559 | 5 | 40 diagrams |
| 16-xy-one | 93 | 1 | xy, xypdf, xycm fonts |
| 17-class-amsart / scrartcl / scrbook | 101 / 446 / 443 | 1 / 1 / 2 | Classes; KOMA is expensive |
| summary | 592 | 2 | User document |
| SGA1-VI | 448 | 25 | User document |
| LMB | 1465 | 34 | User document |
| thesis | 1458 | 34 | User document |

**Gates (RECOMMENDATION):**
- Rungs 00–05: L3 and L2.
- From rung 06 on: L2, L4, L5, and L6 if object order is preserved.

**Later extras (RECOMMENDATION):**
- Per-primitive unit documents compared at log level.
- The official LaTeX regression suite. `latex3/latex2e` has at least 1000 entries in `base/testfiles` (the API listing caps at 1000) and 106 in `required/amsmath/testfiles`; existence is verified, running them is not.
- A SyncTeX rung.
- Before rung 13, a "format state" check comparing `\meaning` for names scanned from `latex.ltx` and `expl3-code.tex`.

### 3.4 `compare_pdf.py`

`/home/site/Projects/rstex/tests/harness/compare_pdf.py` is 292 lines. It needs qpdf ≥ 11 (tested with 12.4.2) and pdftoppm; `magick` only for `--diff-dir`. Exit status: 0 identical, 1 different, 2 could not run.

| Test | Expected | Result |
|---|---|---|
| Thesis, two runs (raw `cmp` differs) | Identical | 34/34 content, 34/34 meta, 0.0000%, 22 fonts; about 1.5 s |
| LMB, two runs | Identical | 34/34, 0.0000%, 28 fonts |
| SGA, two runs | Identical | 25/25, 0.0000%, 20 fonts |
| LMB, 2 extra fonts shifting `/F` numbers, compression off (543 KB vs 1130 KB) | Identical | Identical |
| LMB, one-letter change | Different | Page 17 only, operator #1154, 0.2032%, exit 1 |
| `--diff-dir` | Image written | `page17.png` written |

## 4. Open items

- **Removal limits:** it proves the 3 documents only. Undefined primitives also trigger package fallbacks, so a defined stub could behave differently; test stubs through the same harness.
- **Untouched scope:** PNG and PDF inclusion are exercised only as oracle self-checks in rung 12. Subset-tag normalization has not been stressed, because pdftex's tags are stable across runs.
- **Bug-compatibility decision:** whether rstex copies pdfTeX's `\insert0{` group display, or Knuth's and e-TeX's `\vadjust{`. Copying is the simpler rule while pdftex is the oracle.
- **Leftover temp data:** a cleanup `rm` of `/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/verification/inv/dropwork/` was blocked by a safety check, so I did not retry it. It holds 1 entry, 64 KB, from the killed `\unless` run, and you can delete it.
- **Project writes:** in `/home/site/Projects/rstex` I wrote only `tests/harness/compare_pdf.py`, not committed. Everything else is in scratch; user documents were not modified. `bench/` and `docs/` in the project tree are not mine.

Scratch working files, all under `/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/verification/`:
- `/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/verification/rec/` (`.fls` and logs)
- `/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/verification/inv/drop_results.jsonl` (per-primitive results, 226 rows; `unless` was classified separately)
- `/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/verification/inv/drop_summary.json`
- `/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/verification/inv/inventory.json`
- `/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/verification/inv/tiers.json`
- `/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/verification/inv/static_report.txt`
- `/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/verification/inv/drop_one.sh`
- `/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/verification/inv/trace_doc.sh`
- `/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/verification/inv/analyze_trace.py`