#!/bin/bash
# What a change to the engine must pass before it is merged: cargo test, the integration cases, TRIP and e-TRIP on a checked
# build, and a fixed set of documents against the oracle: the ladder, the user's own documents, 25 or so arXiv papers and SGA3.
#
#     tests/gate.sh ID [--jobs N] [--only compare|integration|trip|test|native|pages]... [--engine PATH] [--oracle-banner] [--filter REGEX]
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
STEPS=${STEPS:-test integration trip compare native pages}
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

if want compare || want native || want pages; then
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
  python3 -I tests/corpus/cd/compare_native.py geometry --engine "$OUT/rstex" --engine-arg=--commute-report -j "$JOBS" --show 40 > "$OUT/native.out" 2>&1
  GEOMETRY=$?
  python3 -I tests/corpus/cd/compare_native.py streams --engine "$OUT/rstex" -j "$JOBS" > "$OUT/native-streams.out" 2>&1
  STREAMS=$?
  python3 -I tests/cd/native/run.py --engine "$OUT/rstex" -j "$JOBS" --work "$OUT/native-cases" > "$OUT/native-cases.out" 2>&1
  CASES=$?
  python3 -I tests/corpus/cd/compare_native.py documents --engine "$OUT/rstex" -j "$JOBS" > "$OUT/native-documents.out" 2>&1
  DOCUMENTS=$?
  python3 -I tests/corpus/snapshot/warm_equals_cold.py --engine "$OUT/rstex" --path-on --filter '^(ladder/|user/)' -j "$JOBS" --run-id "gate-$ID-native-$$" > "$OUT/native-warm.out" 2>&1
  WARM=$?
  if [ $GEOMETRY = 0 ] && [ $STREAMS = 0 ] && [ $CASES = 0 ] && [ $DOCUMENTS = 0 ] && [ $WARM = 0 ]; then
    pass "native ($(head -1 "$OUT/native.out" | cut -c1-120); $(head -1 "$OUT/native-streams.out"); $(tail -1 "$OUT/native-cases.out"); $(tail -1 "$OUT/native-documents.out"); $(tail -1 "$OUT/native-warm.out" | cut -d';' -f1))"
  else
    fail native
    head -60 "$OUT/native.out" | cut -c1-160
    head -30 "$OUT/native-streams.out" | cut -c1-160
    tail -30 "$OUT/native-cases.out" | cut -c1-160
    tail -30 "$OUT/native-documents.out" | cut -c1-160
    tail -30 "$OUT/native-warm.out" | cut -c1-160
  fi
fi

if want pages; then
  # Page records: crates/engine/src/checkpoint/pages.rs.  What is made here, every way it can be got wrong; then real documents, each resumed and going on, against the oracle.  The native step does the last with the path on.
  python3 -I tests/corpus/snapshot/pages.py gate --engine "$OUT/rstex-bin" -j "$JOBS" > "$OUT/pages.out" 2>&1
  CHAINS=$?
  python3 -I tests/corpus/snapshot/warm_equals_cold.py --engine "$OUT/rstex" --filter '^(ladder/|user/|arxiv/math\.AG/)' -j "$JOBS" --run-id "gate-$ID-pages-$$" > "$OUT/pages-warm.out" 2>&1
  WARM=$?
  if [ $CHAINS = 0 ] && [ $WARM = 0 ]; then
    pass "pages ($(tail -1 "$OUT/pages.out"); $(tail -1 "$OUT/pages-warm.out" | cut -d';' -f1))"
  else
    fail pages
    tail -40 "$OUT/pages.out" | cut -c1-160
    head -30 "$OUT/pages-warm.out" | cut -c1-160
    tail -6 "$OUT/pages-warm.out" | cut -c1-160
  fi
fi

if [ -n "$FAILED" ]; then
  echo "GATE $ID: FAIL:$FAILED"
  exit 1
fi
echo "GATE $ID: PASS"
