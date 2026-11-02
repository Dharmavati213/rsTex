#!/bin/sh
# Rebuild refs/merged/pdftex-merged.web from refs/pinned/.
#
# Provenance of refs/pinned/ (verified by git blob hash, see docs/PORTING.md section 1):
#   TeX-Live/texlive-source tag texlive-2026.0, commit f26cc5ed05a1f784d1e694fe5b9cfc3ce992c03d,
#   plus texk/web2c/tex.ch as patched by upstream commit b50e042b46015e58b787df57e400f8ee288f3b62,
#   which is exactly what Arch's texlive-bin 2026.0-4 builds (the oracle).
#
# Order is pdftexdir/am/pdftex.am's pdftex_ch_srcs with synctexdir/am/synctex.am's
# pdftex_ch_synctex, omitting enctexdir/enctex{1,-pdftex,2}.ch, and adding
# tools/oracle-compat.ch last.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root/refs/pinned"
mkdir -p ../merged
tie -m ../merged/pdftex-merged.web \
    pdftexdir/pdftex.web \
    pdftexdir/tex.ch0 \
    tex.ch \
    tracingstacklevels.ch \
    partoken-102.ch \
    partoken.ch \
    locnull-optimize.ch \
    showstream.ch \
    zlib-fmt.ch \
    unbalanced-braces.ch \
    synctexdir/synctex-def.ch0 \
    synctexdir/synctex-mem.ch0 \
    synctexdir/synctex-e-mem.ch0 \
    synctexdir/synctex-e-mem.ch1 \
    synctexdir/synctex-rec.ch0 \
    synctexdir/synctex-rec.ch1 \
    synctexdir/synctex-e-rec.ch0 \
    synctexdir/synctex-pdf-rec.ch2 \
    pdftexdir/pdftex.ch \
    pdftexdir/char-warning-pdftex.ch \
    tex-binpool.ch \
    "$root/tools/oracle-compat.ch"
