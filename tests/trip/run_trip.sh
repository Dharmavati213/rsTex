#!/usr/bin/env bash
# run_trip.sh -- run Knuth's TRIP and e-TeX's e-TRIP against a given engine binary.
#
#   run_trip.sh REFS_DIR WORK_DIR ENGINE MODE
#     REFS_DIR  directory holding trip/ and etrip/ master files (…/rstex/refs)
#     WORK_DIR  scratch output dir (recreated)
#     ENGINE    absolute path of engine binary (e.g. /usr/bin/tex, /usr/bin/pdftex, target/release/rstex)
#     MODE      trip   -- TeX82 compatibility-mode trip only (tex.web semantics)
#               etrip  -- compat trip + extended-mode trip + etrip.tex (etriptest.test steps)
#
# Environment:
#     TRIP_MEM     main_memory for trip    (default 3000, Knuth's; pdfTeX needs more, see report)
#     TRIP_MEMBOT  mem_bot for trip        (default 1, Knuth's)
#     ETRIP_MEM    main_memory for e-trip  (default 3999, upstream etexdir/etrip/texmf.cnf)
#     TRIP_STRINGS/TRIP_POOL/ETRIP_STRINGS/ETRIP_POOL  string table sizes (pdfTeX's pool does not fit
#                  in Knuth's 3000/40000: "You have to increase POOLSIZE")
#
# Output: WORK_DIR/<step>/… and a summary on stdout. Exit 0 always; read the summary.
# Method mirrors web2c triptest.test / etexdir/etriptest.test from TeX Live; texmf.cnf values come
# from texk/web2c/triptrap/texmf.cnf and etexdir/etrip/texmf.cnf.
set -u
REFS=$1; WORK=$2; ENGINE=$3; MODE=$4
TRIP_MEM=${TRIP_MEM:-3000}; TRIP_MEMBOT=${TRIP_MEMBOT:-1}; ETRIP_MEM=${ETRIP_MEM:-3999}
TRIP_STRINGS=${TRIP_STRINGS:-3000}; TRIP_POOL=${TRIP_POOL:-40000}; ETRIP_STRINGS=${ETRIP_STRINGS:-3300}; ETRIP_POOL=${ETRIP_POOL:-32000}
export LC_ALL=C LANGUAGE=C
rm -rf "$WORK"; mkdir -p "$WORK"; cd "$WORK" || exit 2

cat >filter <<'EOF'
s,^\*\*(\./trip\.tex ##,** \&trip  trip \
(trip.tex ##,
s/ (TeX Live 20[^)]*)//
s/ (Web2C 202[3-9][^)]*)//
s/(preloaded format=tex)/(INITEX)/
s/(preloaded format=.*tex)/(INITEX)/
s/format=trip [^)][^)]*)/format=trip)/
s/format=etrip [^)][^)]*)/format=etrip)/
s/)  [0-9A-Z: ]*$/)/
s,^(\./,(,
s/[1-9][0-9]* strings out of [1-9].*/XX strings out of YYY/
s/[1-9][0-9]* string characters out of [1-9].*/XXX string characters out of YYYY/
s/sequences out of [1-9].*/sequences out of YYYY/
s/[1-9] hyphenation exceptions* out of [1-9].*/X hyphenation exceptions out of YYY/
s/[1-9][0-9]* strings of total length [1-9].*/XXXX strings of total length YYYYY/
s/9 ops out of [1-9][0-9]*/9 ops out of YYY/
s/TeX output ....\...\...:..../TeX output YYYY.MM.DD:hhmm/
s/ 16341\.999.*fil/ 16342.0fil/
s/ 16238\.999.*fil/ 16239.0fil/
s/ 16317\.999.*fil/ 16318.0fil/
s/ 16330\.999.*fil/ 16331.0fil/
s/ 16331\.999.*fil/ 16332.0fil/
s/ 16343\.999.*fil/ 16344.0fil/
s/ 9737\.587..fil/ 9737.58789fil/
s/down4 639342.../down4 639342208/
s/y4 2039217../y4 203921760/
s/y0 2039217../y0 203921760/
EOF
# additional normalisation for engines with bigger nodes / more strings (etriptest.test "eTeX" block)
cat >filter_etex <<'EOF'
s/This is .*TeX,/This is *TeX,/
s/ Version 3\.141592653[^(]*(/ Version 3.141592653* (/
s/ before: [1-9][0-9]*&[1-9][0-9]*; / before: XXX\&YYY; /
s/ after: [1-9][0-9]*&[1-9][0-9]*; / after: XXX\&YYY; /
s/ still untouched: [1-9][0-9]*/ still untouched: XXX/
s/ [0-9][0-9]* words of memory out of [0-9]*/ XXX words of memory out of YYY/
s/ memory locations dumped; current usage is .*/ memory locations dumped; current usage is XXX/
s/^[0-9][0-9]* memory locations/NNN memory locations/
s/^[0-9][0-9]* multiletter control sequences/NNN multiletter control sequences/
EOF

mk_cnf() { # dir main_memory mem_bot max_strings pool_size
cat >"$1/texmf.cnf" <<EOF
error_line = 64
half_error_line = 32
max_print_line = 72
mem_bot = $3
main_memory = $2
max_strings = $4
string_vacancies = 8000
pool_size = $5
font_mem_size = 20000
font_max = 75
stack_size = 200
nest_size = 40
buf_size = 500
save_size = 600
dvi_buf_size = 800
MAKETEXTFM = 0
MAKETEXTEX = 0
MAKETEXMF = 0
TEXFONTMAPS = .
TEXINPUTS = .
TFMFONTS = .
TEXFORMATS = .
TEXPOOL = .
EOF
}
DVITYPE_ARGS=(-output-level=2 -dpi=72.27 '-page-start=*.*.*.*.*.*.*.*.*.*')
fdiff() { # master actual label [extra filter]
  local a b extra=${4:-}; a=$(mktemp -p .); b=$(mktemp -p .)
  sed -f filter "$1" >"$a"; sed -f filter "$2" >"$b"
  if [ -n "$extra" ]; then sed -f "$extra" -i "$a"; sed -f "$extra" -i "$b"; fi
  if diff "$a" "$b" >"$3.fdiff"; then echo "  $3: IDENTICAL after filter"; else
    echo "  $3: $(grep -c '^[<>]' "$3.fdiff") changed lines in $(grep -cE '^[0-9]' "$3.fdiff") hunks after filter -> $PWD/$3.fdiff"; fi
  diff "$1" "$2" >"$3.rawdiff"; rm -f "$a" "$b"
}

############################ compatibility mode: tex.web TRIP ###########################
mkdir c; ( cd c
  mk_cnf . "$TRIP_MEM" "$TRIP_MEMBOT" "$TRIP_STRINGS" "$TRIP_POOL"; cp ../filter ../filter_etex .
  export TEXMFCNF=$PWD
  echo "== [compat] step 1: pltotf/tftopl round trip"
  pltotf "$REFS/trip/trip.pl" trip.tfm >pltotf.out 2>&1; tftopl ./trip.tfm tmp.pl >tftopl.out 2>&1
  diff -q "$REFS/trip/trip.pl" tmp.pl >/dev/null && echo "  trip.pl round trip: IDENTICAL" || echo "  trip.pl round trip: DIFFERS"
  cp "$REFS/trip/trip.tex" .
  echo "== [compat] step 2: INITEX pass (trip1.in): builds trip.fmt, tripin.log"
  "$ENGINE" --progname=initex --ini <"$REFS/trip/trip1.in" >tripin.fot 2>&1; echo "  exit=$?"
  [ -s trip.fmt ] && echo "  trip.fmt: $(wc -c <trip.fmt) bytes" || echo "  trip.fmt NOT CREATED"
  mv trip.log tripin.log 2>/dev/null
  fdiff "$REFS/trip/tripin.log" tripin.log tripin
  fdiff "$REFS/trip/tripin.log" tripin.log tripin-mem filter_etex
  echo "== [compat] step 3: trip pass (trip2.in) non-ini: trip.log trip.fot trip.dvi tripos.tex"
  "$ENGINE" --progname=tex <"$REFS/trip/trip2.in" >trip.fot 2>&1; echo "  exit=$?"
  fdiff "$REFS/trip/trip.log" trip.log trip-log
  fdiff "$REFS/trip/trip.log" trip.log trip-log-mem filter_etex
  fdiff "$REFS/trip/trip.fot" trip.fot trip-fot filter_etex
  if [ -s trip.dvi ]; then dvitype "${DVITYPE_ARGS[@]}" trip.dvi >trip.typ 2>&1; fdiff "$REFS/trip/trip.typ" trip.typ trip-typ; else echo "  no trip.dvi"; fi
  ls -la 8terminal.tex tripos.tex 2>&1 | sed 's/^/  /'
)
[ "$MODE" = trip ] && exit 0

############################ extended mode: trip + etrip ###########################
mkdir x; ( cd x
  mk_cnf . "$ETRIP_MEM" 0 "$ETRIP_STRINGS" "$ETRIP_POOL"; cp ../filter ../filter_etex .
  export TEXMFCNF=$PWD
  echo "== [extended] step 4: extended-mode INITEX trip (etrip1.in = '*\\input trip')"
  pltotf "$REFS/trip/trip.pl" trip.tfm >/dev/null 2>&1; cp "$REFS/trip/trip.tex" .
  "$ENGINE" --progname=einitex --ini <"$REFS/etrip/etrip1.in" >xtripin.fot 2>&1; echo "  exit=$?"
  [ -s trip.fmt ] && echo "  trip.fmt: $(wc -c <trip.fmt) bytes" || echo "  trip.fmt NOT CREATED"
  mv trip.log xtripin.log 2>/dev/null
  echo "== [extended] step 5: extended-mode trip pass ('&trip \\toksdef\\tokens=0 \\input trip')"
  printf '&trip \\toksdef\\tokens=0 \\input trip\n' >trip2x.in
  "$ENGINE" --progname=etex <trip2x.in >xtrip.fot 2>&1; echo "  exit=$?"
  mv trip.log xtrip.log 2>/dev/null
  fdiff "$REFS/trip/trip.log" xtrip.log xtrip-vs-master filter_etex
  [ -s trip.dvi ] && { dvitype "${DVITYPE_ARGS[@]}" trip.dvi >xtrip.typ 2>&1; fdiff "$REFS/trip/trip.typ" xtrip.typ xtrip-typ; }
)
mkdir e; ( cd e
  mk_cnf . "$ETRIP_MEM" 0 "$ETRIP_STRINGS" "$ETRIP_POOL"; cp ../filter ../filter_etex .
  export TEXMFCNF=$PWD
  echo "== [e-TRIP] step 6: etrip.pl round trip"
  pltotf "$REFS/etrip/etrip.pl" etrip.tfm >/dev/null 2>&1; tftopl ./etrip.tfm tmp.pl >/dev/null 2>&1
  diff -q "$REFS/etrip/etrip.pl" tmp.pl >/dev/null && echo "  etrip.pl round trip: IDENTICAL" || echo "  etrip.pl round trip: DIFFERS"
  cp "$REFS/etrip/etrip.tex" .
  echo "== [e-TRIP] step 7: INITEX pass (etrip2.in = '*etrip')"
  "$ENGINE" --progname=einitex --ini <"$REFS/etrip/etrip2.in" >etripin.fot 2>&1; echo "  exit=$?"
  [ -s etrip.fmt ] && echo "  etrip.fmt: $(wc -c <etrip.fmt) bytes" || echo "  etrip.fmt NOT CREATED"
  mv etrip.log etripin.log 2>/dev/null
  fdiff "$REFS/etrip/etripin.log" etripin.log etripin
  echo "== [e-TRIP] step 8: run pass ('&etrip etrip')"
  printf '&etrip etrip\n' >etrip3.in
  "$ENGINE" --progname=etex <etrip3.in >etrip.fot 2>&1; echo "  exit=$?"
  fdiff "$REFS/etrip/etrip.log" etrip.log etrip-log
  fdiff "$REFS/etrip/etrip.log" etrip.log etrip-log-mem filter_etex
  fdiff "$REFS/etrip/etrip.fot" etrip.fot etrip-fot filter_etex
  if [ -s etrip.dvi ]; then dvitype "${DVITYPE_ARGS[@]}" etrip.dvi >etrip.typ 2>&1; fdiff "$REFS/etrip/etrip.typ" etrip.typ etrip-typ; else echo "  no etrip.dvi"; fi
)
