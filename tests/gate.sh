#!/bin/bash
# What a change to the engine must pass before it is merged: cargo test, the integration cases, TRIP and e-TRIP on a checked
# build, and a fixed set of documents against the oracle: the ladder, the user's own documents, 25 or so arXiv papers and SGA3.
#
#     tests/gate.sh ID [--jobs N] [--only compare|integration|trip|test|native|outcache|native_font|synctex|pages|until_stable]... [--engine PATH] [--oracle-banner] [--filter REGEX]
#
# --engine judges a binary that is already built, such as the one tools/pgo.py leaves, in place of a release build of this tree.  Then trip runs on that
# binary too: it needs only a binary, and a checked build would be a different one.
#
# --oracle-banner builds with the feature oracle-banner, whose version string is the oracle's, and runs the integration cases and the
# compare on that, in target/gate/ID: then a document's L6 has to be plainly equal, with no mask, and tests/gate_verdict.py counts them.  The unit
# tests and TRIP are for the ordinary build.  --filter replaces the compare's selection, so that the claim can be made over another set.
#
# Takes CARGO_TARGET_DIR from the environment, default target/.  Everything it writes goes under target/gate/ID, including a
# wrapper and the engine's own formats, which you can reuse: target/gate/ID/rstex finds them as pdflatex does.  The last line says PASS
# or FAIL, and the exit status agrees.  The corpus runs are read through the oracle's cache in target/corpus, so a worktree should have
#     ln -s /home/site/Projects/rstex/target target
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
ID=${1:?usage: tests/gate.sh ID [--jobs N] [--only STEP]... [--engine PATH] [--oracle-banner] [--filter REGEX]}
shift
JOBS=6
STEPS=""
GIVEN=""
FEATURES=""
VERDICT=""
FILTER='^(ladder/|user/|arxiv/math\.AG/|local/sga3/SGA3-English$)'
while [ $# -gt 0 ]; do
  case "$1" in
    --jobs) JOBS=$2; shift 2 ;;
    --only) STEPS="$STEPS $2"; shift 2 ;;
    --engine) GIVEN=$2; shift 2 ;;
    --oracle-banner) FEATURES="--features rstex-engine/oracle-banner"; VERDICT=--bytes; shift ;;
    --filter) FILTER=$2; shift 2 ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
done
if [ -n "$GIVEN" ] && [ -n "$FEATURES" ]; then echo "--engine and --oracle-banner do not go together" >&2; exit 2; fi
[ -n "$FEATURES" ] && STEPS=${STEPS:-integration compare}
STEPS=${STEPS:-test integration trip compare native outcache native_font synctex pages until_stable}
want() { case " $STEPS " in *" $1 "*) return 0 ;; esac; return 1; }

OUT=$ROOT/target/gate/$ID
TD=${CARGO_TARGET_DIR:-$ROOT/target}
[ -n "$FEATURES" ] && TD=$TD/oracle-banner
mkdir -p "$OUT"
FAILED=""
fail() { FAILED="$FAILED $1"; echo "GATE $1: FAIL"; }
pass() { echo "GATE $1: pass"; }

cd "$ROOT" || exit 2
if [ -n "$GIVEN" ]; then
  ENGINE=$GIVEN
else
  cargo build --release -p rstex $FEATURES --target-dir "$TD" -j"$JOBS" 2>&1 | tail -1
  ENGINE=$TD/release/rstex
fi
[ -x "$ENGINE" ] || { echo "GATE build: FAIL, no $ENGINE"; exit 1; }
cp "$ENGINE" "$OUT/rstex-bin"
# The integration cases see the program's name, in its messages and as the default format, so they run a copy called rstex, whatever this was called.
mkdir -p "$OUT/bin"
cp "$ENGINE" "$OUT/bin/rstex"

if want test; then
  # --no-fail-fast: a test binary that fails must not hide what the others say.
  if cargo test --workspace --no-fail-fast -j"$JOBS" > "$OUT/test.out" 2>&1; then
    pass "test ($(grep -E '^test result' "$OUT/test.out" | awk '{p+=$4; f+=$6} END {print p" passed, "f" failed"}'))"
  else
    fail test
    grep -E '^test .*FAILED|^error|panicked' "$OUT/test.out" | head -20
  fi
fi

if want integration; then
  # A long timeout: pdfobj/objects_overflow takes 4 s on a quiet machine and 19 s at load 42.
  if RSTEX_INTEGRATION_OUT="$OUT/integration" python3 -I tests/integration/run.py --engine "$OUT/bin/rstex" -j"$JOBS" --timeout 120 > "$OUT/integration.out" 2>&1; then
    pass "integration ($(grep -E '^total' "$OUT/integration.out" | tail -1))"
  else
    fail integration
    grep -E '^FAIL|^total' "$OUT/integration.out" | head -30
  fi
fi

if want trip; then
  if [ -n "$GIVEN" ]; then
    TRIP_ENGINE=$OUT/rstex-bin
  else
    cargo build --release -p rstex --features rstex-engine/checked --target-dir "$TD/checked" -j"$JOBS" 2>&1 | tail -1
    TRIP_ENGINE=$TD/checked/release/rstex
  fi
  if python3 -I tests/trip/compare_trip.py "$TRIP_ENGINE" --work "$OUT/trip" > "$OUT/trip.out" 2>&1; then
    pass "trip ($(tail -1 "$OUT/trip.out"))"
  else
    fail trip
    tail -30 "$OUT/trip.out" | cut -c1-160
  fi
fi

if want compare || want native || want outcache || want native_font || want synctex || want pages || want until_stable; then
  # *.local.json are not in git, so a worktree has none, and user/ and local/ would drop out of the selection with no word said.
  MAIN=$(cd -P "$ROOT/target/.." 2>/dev/null && pwd)
  for m in "$MAIN"/tests/corpus/manifests/*.local.json; do
    if [ -e "$m" ] && [ ! -e "$ROOT/tests/corpus/manifests/$(basename "$m")" ]; then
      ln -s "$m" "$ROOT/tests/corpus/manifests/"
    fi
  done
  mkdir -p "$OUT/fmt" "$OUT/fmt-pdftex" "$OUT/fmt-latex"
  (cd "$OUT/fmt" && "$OUT/rstex-bin" -ini -jobname=pdflatex -progname=pdflatex -translate-file=cp227.tcx '*pdflatex.ini' > build.out 2>&1)
  (cd "$OUT/fmt-pdftex" && "$OUT/rstex-bin" -ini -jobname=pdftex -progname=pdftex -translate-file=cp227.tcx '*pdfetex.ini' > build.out 2>&1)
  (cd "$OUT/fmt-latex" && "$OUT/rstex-bin" -ini -jobname=latex -progname=latex -translate-file=cp227.tcx '*latex.ini' > build.out 2>&1)
  cat > "$OUT/rstex" <<EOF
#!/bin/sh
# Finds this engine's own formats: the installed ones are the oracle's gzip files.
for x in "\$@"; do
  case "\$x" in
    -fmt=pdflatex) set -- "\$@" -fmt=$OUT/fmt/pdflatex -progname=pdflatex ;;
    -fmt=pdftex) set -- "\$@" -fmt=$OUT/fmt-pdftex/pdftex -progname=pdftex ;;
    -fmt=latex) set -- "\$@" -fmt=$OUT/fmt-latex/latex -progname=latex ;;
    *) set -- "\$@" "\$x" ;;
  esac
  shift
done
exec $OUT/rstex-bin "\$@"
EOF
  chmod +x "$OUT/rstex"
fi

if want compare; then
  RUN=gate-$ID-$$
  python3 -I tests/corpus/run.py compare --engine "$OUT/rstex" --raster none --mode both -j "$JOBS" --run-id "$RUN" --box-log always \
    --timeout-scale 5 --filter "$FILTER" > "$OUT/compare.out" 2>&1
  if python3 -I tests/gate_verdict.py "${RSTEX_CORPUS_TARGET:-$ROOT/target/corpus}/runs/$RUN/results.jsonl" $VERDICT; then
    pass compare
  else
    fail compare
  fi
fi

if want native; then
  # DESIGN section 10: every extracted diagram with the path on, the content streams of those that take it, its cases one requirement at a time, the documents that hold diagrams,
  # and those runs from a checkpoint, equal to cold ones, since the compare and snapshot/warm_equals_cold.py judge the literal engine.
  # Every comparison passes with every diagram declined, so what takes the path has a floor, and that the floor can fail is shown first.
  python3 -I tests/corpus/cd/compare_native.py floor-check --engine "$OUT/rstex" -j "$JOBS" > "$OUT/native-floor.out" 2>&1
  FLOOR=$?
  python3 -I tests/corpus/cd/compare_native.py geometry --engine "$OUT/rstex" --engine-arg=--commute-report -j "$JOBS" --show 40 --floor tests/corpus/cd/native_floor.json > "$OUT/native.out" 2>&1
  GEOMETRY=$?
  python3 -I tests/corpus/cd/compare_native.py streams --engine "$OUT/rstex" -j "$JOBS" > "$OUT/native-streams.out" 2>&1
  STREAMS=$?
  python3 -I tests/cd/native/run.py --engine "$OUT/rstex" -j "$JOBS" --work "$OUT/native-cases" > "$OUT/native-cases.out" 2>&1
  CASES=$?
  python3 -I tests/corpus/cd/compare_native.py documents --engine "$OUT/rstex" -j "$JOBS" --floor tests/corpus/cd/native_floor.json > "$OUT/native-documents.out" 2>&1
  DOCUMENTS=$?
  python3 -I tests/corpus/snapshot/warm_equals_cold.py --engine "$OUT/rstex" --path-on --filter '^(ladder/|user/)' -j "$JOBS" --run-id "gate-$ID-native-$$" > "$OUT/native-warm.out" 2>&1
  WARM=$?
  # The documents written to break it, and to make its check slow.
  python3 -I tests/cd/native/adversarial/run.py --engine "$OUT/rstex" -j "$JOBS" --work "$OUT/native-adversarial" > "$OUT/native-adversarial.out" 2>&1
  ADVERSARIAL=$?
  python3 -I tests/cd/native/adversarial/cost.py --engine "$OUT/rstex" --work "$OUT/native-cost" > "$OUT/native-cost.out" 2>&1
  COST=$?
  if [ $FLOOR = 0 ] && [ $GEOMETRY = 0 ] && [ $STREAMS = 0 ] && [ $CASES = 0 ] && [ $DOCUMENTS = 0 ] && [ $WARM = 0 ] && [ $ADVERSARIAL = 0 ] && [ $COST = 0 ]; then
    pass "native ($(head -1 "$OUT/native.out" | cut -c1-120); $(head -1 "$OUT/native-streams.out"); $(tail -1 "$OUT/native-cases.out"); $(tail -1 "$OUT/native-documents.out"); $(tail -1 "$OUT/native-warm.out" | cut -d';' -f1); $(tail -1 "$OUT/native-adversarial.out"); $(tail -1 "$OUT/native-cost.out"))"
  else
    fail native
    tail -3 "$OUT/native-floor.out" | cut -c1-160
    head -60 "$OUT/native.out" | cut -c1-160
    head -30 "$OUT/native-streams.out" | cut -c1-160
    tail -30 "$OUT/native-cases.out" | cut -c1-160
    tail -30 "$OUT/native-documents.out" | cut -c1-160
    tail -30 "$OUT/native-warm.out" | cut -c1-160
    tail -40 "$OUT/native-adversarial.out" | cut -c1-160
    tail -16 "$OUT/native-cost.out" | cut -c1-160
  fi
fi

if want outcache; then
  # DESIGN section 9: what a change to the world may do, and the documents again with the last PDF in its place, by the default filter and as a user has them, with a .fls and the diagrams' path.  Each stream of
  # those must come from the old file, and the outputs be those of a cold run; the first of them is judged against the oracle too.  Then that a wrong cache is caught.
  python3 -I tests/corpus/snapshot/outcache.py --engine "$OUT/rstex-bin" > "$OUT/outcache.out" 2>&1
  CASES=$?
  RSTEX_CRITIC_ENGINE="$OUT/rstex-bin" python3 -I tools/prof/critic/oc_adv.py > "$OUT/outcache-adv.out" 2>&1
  ADV=$?
  python3 -I tests/corpus/snapshot/warm_equals_cold.py --engine "$OUT/rstex" --keep-pdf -j "$JOBS" --run-id "gate-$ID-outcache-$$" > "$OUT/outcache-warm.out" 2>&1
  WARM=$?
  python3 -I tests/corpus/snapshot/warm_equals_cold.py --engine "$OUT/rstex" --keep-pdf --path-on --fls --filter '^(ladder/|user/)' -j "$JOBS" --run-id "gate-$ID-outcache-on-$$" > "$OUT/outcache-on.out" 2>&1
  ON=$?
  python3 -I tests/corpus/snapshot/warm_equals_cold.py --engine "$OUT/rstex" --check-mutants -j "$JOBS" --run-id "gate-$ID-outcache-mutants-$$" > "$OUT/outcache-mutants.out" 2>&1
  MUTANTS=$?
  if [ $CASES = 0 ] && [ $ADV = 0 ] && [ $WARM = 0 ] && [ $ON = 0 ] && [ $MUTANTS = 0 ]; then
    pass "outcache ($(tail -1 "$OUT/outcache.out"); $(tail -1 "$OUT/outcache-adv.out"); $(grep -E '^output cache' "$OUT/outcache-warm.out"); $(grep -E '^output cache' "$OUT/outcache-on.out"); $(tail -1 "$OUT/outcache-mutants.out"))"
  else
    fail outcache
    grep -E '^FAIL|failed$' "$OUT/outcache.out" | cut -c1-160
    tail -1 "$OUT/outcache-adv.out" | cut -c1-160
    tail -30 "$OUT/outcache-warm.out" | cut -c1-160
    tail -30 "$OUT/outcache-on.out" | cut -c1-160
    tail -30 "$OUT/outcache-mutants.out" | cut -c1-160
  fi
fi

if want native_font; then
  # The native \\pickup@font: the compare and the integration cases hold it against the oracle, and this against the real macros on the same program, with what stays between runs, its margins and its switches.
  python3 -I tests/native_font/run.py --engine "$OUT/rstex" -j "$JOBS" --work "$OUT/native-font-cases" > "$OUT/native-font-cases.out" 2>&1
  CASES=$?
  python3 -I tests/native_font/run.py --engine "$OUT/rstex" -j "$JOBS" --corpus '^(ladder/|user/|manuals/microtype/|local/sga3/SGA3-English$|arxiv/math\.AG/)' > "$OUT/native-font-corpus.out" 2>&1
  CORPUS=$?
  if [ $CASES = 0 ] && [ $CORPUS = 0 ]; then
    pass "native_font ($(tail -1 "$OUT/native-font-cases.out"); $(tail -1 "$OUT/native-font-corpus.out"))"
  else
    fail native_font
    tail -30 "$OUT/native-font-cases.out" | cut -c1-160
    tail -30 "$OUT/native-font-corpus.out" | cut -c1-160
  fi
fi

if want synctex; then
  # DESIGN section 9: its cases against the oracle, the text and the answers of the synctex tool on the ladder and the user's documents, and all of that from a checkpoint.
  python3 -I tests/synctex/run.py --engine "$OUT/rstex" -j "$JOBS" --work "$OUT/synctex" > "$OUT/synctex.out" 2>&1
  CASES=$?
  python3 -I tests/synctex/documents.py --engine "$OUT/rstex-bin" --docs 'ladder/*,user/*' -j "$JOBS" > "$OUT/synctex-documents.out" 2>&1
  DOCUMENTS=$?
  python3 -I tests/corpus/snapshot/warm_equals_cold.py --engine "$OUT/rstex" --synctex --filter '^(ladder/|user/)' -j "$JOBS" --run-id "gate-$ID-synctex-$$" > "$OUT/synctex-warm.out" 2>&1
  WARM=$?
  # As an editor runs it: the native path on, the recorder on, the last PDF in its place.
  python3 -I tests/corpus/snapshot/warm_equals_cold.py --engine "$OUT/rstex" --synctex --path-on --fls --keep-pdf --filter '^(ladder/|user/)' -j "$JOBS" --run-id "gate-$ID-synctex-on-$$" > "$OUT/synctex-warm-on.out" 2>&1
  WARM_ON=$?
  if [ $CASES = 0 ] && [ $DOCUMENTS = 0 ] && [ $WARM = 0 ] && [ $WARM_ON = 0 ]; then
    pass "synctex ($(grep "cases agree" "$OUT/synctex.out" | cut -d';' -f1); $(grep "mutants caught" "$OUT/synctex.out"); $(tail -2 "$OUT/synctex-documents.out" | head -1); $(tail -1 "$OUT/synctex-warm.out" | cut -d';' -f1); with the path on $(tail -1 "$OUT/synctex-warm-on.out" | cut -d';' -f1))"
  else
    fail synctex
    tail -30 "$OUT/synctex.out" | cut -c1-160
    grep -E 'FAIL|PASS' "$OUT/synctex-documents.out" | tail -30 | cut -c1-160
    tail -30 "$OUT/synctex-warm.out" | cut -c1-160
    tail -30 "$OUT/synctex-warm-on.out" | cut -c1-160
  fi
fi

if want pages; then
  # Page records: crates/engine/src/checkpoint/pages.rs.  What is made here, every way it can be got wrong, with no PDF to find and with the last one in its place; then real documents, each resumed and going on, against the oracle.
  # The native step does the last with the path on, and the outcache step with the PDF in place.
  python3 -I tests/corpus/snapshot/pages.py gate --engine "$OUT/rstex-bin" -j "$JOBS" > "$OUT/pages.out" 2>&1
  CHAINS=$?
  python3 -I tests/corpus/snapshot/pages.py gate --keep-pdf --engine "$OUT/rstex-bin" -j "$JOBS" > "$OUT/pages-kept.out" 2>&1
  KEPT=$?
  python3 -I tests/corpus/snapshot/warm_equals_cold.py --engine "$OUT/rstex" --filter '^(ladder/|user/|arxiv/math\.AG/)' -j "$JOBS" --run-id "gate-$ID-pages-$$" > "$OUT/pages-warm.out" 2>&1
  WARM=$?
  if [ $CHAINS = 0 ] && [ $KEPT = 0 ] && [ $WARM = 0 ]; then
    pass "pages ($(tail -1 "$OUT/pages.out"); with the PDF in place $(tail -1 "$OUT/pages-kept.out"); $(tail -1 "$OUT/pages-warm.out" | cut -d';' -f1))"
  else
    fail pages
    tail -40 "$OUT/pages.out" | cut -c1-160
    tail -40 "$OUT/pages-kept.out" | cut -c1-160
    head -30 "$OUT/pages-warm.out" | cut -c1-160
    tail -6 "$OUT/pages-warm.out" | cut -c1-160
  fi
fi

if want until_stable; then
  # What crates/rstex/tests/until_stable.rs cannot have: LaTeX, the real bibtex, biber and makeindex, a pass from a checkpoint and from a page record, and latexmk beside it.
  if python3 -I tests/until_stable/run.py --engine "$OUT/rstex-bin" > "$OUT/until-stable.out" 2>&1; then
    pass "until_stable ($(tail -2 "$OUT/until-stable.out" | head -1))"
  else
    fail until_stable
    grep -E '^FAILED' "$OUT/until-stable.out" | cut -c1-160
    tail -1 "$OUT/until-stable.out" | cut -c1-160
  fi
fi

if [ -n "$FAILED" ]; then
  echo "GATE $ID: FAIL:$FAILED"
  exit 1
fi
echo "GATE $ID: PASS"
