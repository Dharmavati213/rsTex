#!/bin/bash
# What a change to the engine must pass before it is merged: cargo test, the integration cases, TRIP and e-TRIP on a checked
# build, and a fixed set of documents against the oracle: the ladder, the user's own documents, 25 or so arXiv papers and SGA3.
#
#     tests/gate.sh ID [--jobs N] [--only compare|integration|trip|test]... [--oracle-banner] [--filter REGEX]
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
ID=${1:?usage: tests/gate.sh ID [--jobs N] [--only STEP]... [--oracle-banner] [--filter REGEX]}
shift
JOBS=6
STEPS=""
FEATURES=""
VERDICT=""
FILTER='^(ladder/|user/|arxiv/math\.AG/|local/sga3/SGA3-English$)'
while [ $# -gt 0 ]; do
  case "$1" in
    --jobs) JOBS=$2; shift 2 ;;
    --only) STEPS="$STEPS $2"; shift 2 ;;
    --oracle-banner) FEATURES="--features rstex-engine/oracle-banner"; VERDICT=--bytes; shift ;;
    --filter) FILTER=$2; shift 2 ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
done
[ -n "$FEATURES" ] && STEPS=${STEPS:-integration compare}
STEPS=${STEPS:-test integration trip compare}
want() { case " $STEPS " in *" $1 "*) return 0 ;; esac; return 1; }

OUT=$ROOT/target/gate/$ID
TD=${CARGO_TARGET_DIR:-$ROOT/target}
[ -n "$FEATURES" ] && TD=$TD/oracle-banner
mkdir -p "$OUT"
FAILED=""
fail() { FAILED="$FAILED $1"; echo "GATE $1: FAIL"; }
pass() { echo "GATE $1: pass"; }

cd "$ROOT" || exit 2
cargo build --release -p rstex $FEATURES --target-dir "$TD" -j"$JOBS" 2>&1 | tail -1
ENGINE=$TD/release/rstex
[ -x "$ENGINE" ] || { echo "GATE build: FAIL, no $ENGINE"; exit 1; }
cp "$ENGINE" "$OUT/rstex-bin"

if want test; then
  if cargo test --workspace -j"$JOBS" > "$OUT/test.out" 2>&1; then
    pass "test ($(grep -E '^test result' "$OUT/test.out" | awk '{p+=$4; f+=$6} END {print p" passed, "f" failed"}'))"
  else
    fail test
    grep -E '^test .*FAILED|^error|panicked' "$OUT/test.out" | head -20
  fi
fi

if want integration; then
  if python3 -I tests/integration/run.py --engine "$OUT/rstex-bin" -j"$JOBS" > "$OUT/integration.out" 2>&1; then
    pass "integration ($(grep -E '^total' "$OUT/integration.out" | tail -1))"
  else
    fail integration
    grep -E '^FAIL|^total' "$OUT/integration.out" | head -30
  fi
fi

if want trip; then
  cargo build --release -p rstex --features rstex-engine/checked --target-dir "$TD/checked" -j"$JOBS" 2>&1 | tail -1
  if python3 -I tests/trip/compare_trip.py "$TD/checked/release/rstex" --work "$OUT/trip" > "$OUT/trip.out" 2>&1; then
    pass "trip ($(tail -1 "$OUT/trip.out"))"
  else
    fail trip
    tail -30 "$OUT/trip.out" | cut -c1-160
  fi
fi

if want compare; then
  # *.local.json are not in git, so a worktree has none, and user/ and local/ would drop out of the selection with no word said.
  MAIN=$(cd -P "$ROOT/target/.." 2>/dev/null && pwd)
  for m in "$MAIN"/tests/corpus/manifests/*.local.json; do
    if [ -e "$m" ] && [ ! -e "$ROOT/tests/corpus/manifests/$(basename "$m")" ]; then
      ln -s "$m" "$ROOT/tests/corpus/manifests/"
    fi
  done
  mkdir -p "$OUT/fmt" "$OUT/fmt-pdftex"
  (cd "$OUT/fmt" && "$OUT/rstex-bin" -ini -jobname=pdflatex -progname=pdflatex -translate-file=cp227.tcx '*pdflatex.ini' > build.out 2>&1)
  (cd "$OUT/fmt-pdftex" && "$OUT/rstex-bin" -ini -jobname=pdftex -progname=pdftex -translate-file=cp227.tcx '*pdfetex.ini' > build.out 2>&1)
  cat > "$OUT/rstex" <<EOF
#!/bin/sh
# Finds this engine's own formats: the installed ones are the oracle's gzip files.
for x in "\$@"; do
  case "\$x" in
    -fmt=pdflatex) set -- "\$@" -fmt=$OUT/fmt/pdflatex -progname=pdflatex ;;
    -fmt=pdftex) set -- "\$@" -fmt=$OUT/fmt-pdftex/pdftex -progname=pdftex ;;
    *) set -- "\$@" "\$x" ;;
  esac
  shift
done
exec $OUT/rstex-bin "\$@"
EOF
  chmod +x "$OUT/rstex"
  RUN=gate-$ID-$$
  python3 -I tests/corpus/run.py compare --engine "$OUT/rstex" --raster none --mode last-pass -j "$JOBS" --run-id "$RUN" --box-log never \
    --timeout-scale 5 --filter "$FILTER" > "$OUT/compare.out" 2>&1
  if python3 -I tests/gate_verdict.py "$ROOT/target/corpus/runs/$RUN/results.jsonl" $VERDICT; then
    pass compare
  else
    fail compare
  fi
fi

if [ -n "$FAILED" ]; then
  echo "GATE $ID: FAIL:$FAILED"
  exit 1
fi
echo "GATE $ID: PASS"
