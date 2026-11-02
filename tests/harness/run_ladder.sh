#!/usr/bin/env bash
# run_ladder.sh [CANDIDATE] : compile each rung with the pdfTeX oracle and a candidate engine and compare.
# Default candidate = pdflatex itself (self-check that the ladder and comparator are stable).
# CANDIDATE is invoked exactly like pdflatex/pdftex: CANDIDATE [--plain] -interaction=... -output-directory=DIR file
# Private documents are never named here. RSTEX_LADDER_PRIVATE names a list file outside the repo, one document per line:
#   /path/to/doc.tex [/path/to/precomputed.bbl]
# Blank lines and lines starting with # are skipped, and ~/ is expanded. Without it only the public rungs run.
set -u
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
LD=$ROOT/tests/ladder; CMP=$ROOT/tests/harness/compare_pdf.py
OUT=${LADDER_OUT:-$ROOT/target/ladder-out}; CAND=${1:-}
export SOURCE_DATE_EPOCH=1700000000 FORCE_SOURCE_DATE=1
oracle() { case "$1" in 0[0-3]-*) echo pdftex;; *) echo pdflatex;; esac; }
compile() { # prog dir srcdir file passes
  local t0 t1; for ((i=0;i<$5;i++)); do t0=$(date +%s%N); (cd "$3" && "$1" -interaction=batchmode -output-directory="$2" "$4" >/dev/null 2>&1); t1=$(date +%s%N); done; echo $(( (t1-t0)/1000000 ))
}
printf '%-30s %8s %6s %10s %5s %s\n' rung ms pages bytes errs compare
RUNGS=(); BBLS=()
for f in "$LD"/[0-9]*.tex; do RUNGS+=("$f"); BBLS+=(""); done
if [ -n "${RSTEX_LADDER_PRIVATE:-}" ]; then
  [ -r "$RSTEX_LADDER_PRIVATE" ] || { echo "run_ladder: cannot read the file named by RSTEX_LADDER_PRIVATE" >&2; exit 2; }
  while read -r tex bbl _; do
    case "$tex" in ''|'#'*) continue;; esac
    RUNGS+=("${tex/#\~\//$HOME/}"); BBLS+=("${bbl/#\~\//$HOME/}")
  done < "$RSTEX_LADDER_PRIVATE"
else
  echo "run_ladder: RSTEX_LADDER_PRIVATE is not set, running the public rungs only" >&2
fi
for i in "${!RUNGS[@]}"; do
  f=${RUNGS[$i]}; bbl=${BBLS[$i]}
  b=$(basename "$f" .tex); src=$(dirname "$f"); prog=$(oracle "$b")
  A=$OUT/a/$b; B=$OUT/b/$b; rm -rf "$A" "$B"; mkdir -p "$A" "$B"
  # user docs with a precomputed bibliography
  [ -n "$bbl" ] && cp "$bbl" "$A/"
  ms=$(compile "$prog" "$A" "$src" "$(basename "$f")" 2)
  cp "$A"/*.aux "$A"/*.toc "$A"/*.out "$A"/*.bbl "$B"/ 2>/dev/null
  if [ -n "$CAND" ]; then compile "$CAND" "$B" "$src" "$(basename "$f")" 1 >/dev/null
  else compile "$prog" "$B" "$src" "$(basename "$f")" 1 >/dev/null; fi
  pages=$(pdfinfo "$A/$b.pdf" 2>/dev/null | awk '/^Pages/{print $2}'); size=$(stat -c %s "$A/$b.pdf" 2>/dev/null)
  errs=$(grep -c '^!' "$A/$b.log" 2>/dev/null)
  res=$(python3 "$CMP" "$A/$b.pdf" "$B/$b.pdf" 2>&1 | tail -1 | sed 's/RESULT: //')
  printf '%-30s %8s %6s %10s %5s %s\n' "$b" "$ms" "${pages:--}" "${size:--}" "$errs" "$res"
done
