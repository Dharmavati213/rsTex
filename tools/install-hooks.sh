#!/bin/sh
# Install the privacy guard as this clone's hooks. Run it once after every clone, since git does not copy hooks, and again
# after changing tools/privacy_guard.py: hooks run the copy made here, so that the tree being committed has no say in what
# checks it. This also records the private directories in .git/config; a hook that cannot find them stops.
#
#     tools/install-hooks.sh                        on the machine that has the private documents
#     tools/install-hooks.sh --no-private-sources   on one that never had them: only rules f, g and h run
#
# Both hooks fail closed. Hooks are still local: `git commit --no-verify` or `git push --no-verify` skips them, which
# tools/install-gate.sh deals with. See docs/DESIGN.md, section 13.
set -eu

marker='rstex-privacy-guard'
root=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
cd "$root"
common=$(cd "$(git rev-parse --git-common-dir)" && pwd)
store=$common/rstex-guard
hooks=$(git rev-parse --git-path hooks)
case "$hooks" in
  /*) ;;
  *) hooks=$root/$hooks ;;
esac

configured=$(git config --get core.hooksPath || true)
if [ -n "$configured" ]; then
  echo "install-hooks: core.hooksPath is set to $configured, so hooks in $hooks would not run." >&2
  echo "install-hooks: unset it or point it here, then run this again." >&2
  exit 1
fi

python=$(command -v python3 || true)
if [ -z "$python" ]; then
  echo "install-hooks: python3 is not available." >&2
  exit 1
fi

mkdir -p "$hooks" "$store"
cp "$root/tools/privacy_guard.py" "$store/privacy_guard.py.tmp"
mv "$store/privacy_guard.py.tmp" "$store/privacy_guard.py"
"$python" -I "$store/privacy_guard.py" --install-config "$@"
if [ ! -e "$store/approved-binaries.txt" ]; then
  cat > "$store/approved-binaries.txt" <<'TEXT'
# Binaries a person has looked at, one per line as printed by sha256sum: SHA256, two spaces, path from the root.
# This file is outside every tree. See the header of tools/privacy-allowed-binaries.txt. Agents do not edit it.
TEXT
fi

write_hook() { # name, with the body on stdin
  target=$hooks/$1
  if [ -e "$target" ] && ! grep -q "$marker" "$target"; then
    echo "install-hooks: $target exists and is not ours; merge it by hand." >&2
    exit 1
  fi
  {
    printf '#!/bin/sh\n# %s: written by tools/install-hooks.sh. Fails closed.\n' "$marker"
    printf "python='%s'\nwhat='%s'\n" "$python" "$2"
    cat <<'HOOK'
stop() { echo "privacy guard: $1; $what stopped." >&2; exit 1; }
common=$(git rev-parse --git-common-dir) || stop "cannot find the git directory"
guard=$common/rstex-guard/privacy_guard.py
[ -f "$guard" ] || stop "the installed guard is missing, run tools/install-hooks.sh"
[ -x "$python" ] || stop "$python is not available"
tree=$(git rev-parse --show-toplevel)/tools/privacy_guard.py
if [ -f "$tree" ] && ! cmp -s "$guard" "$tree"; then
  echo "privacy guard: note: tools/privacy_guard.py differs from the installed copy. Hooks run the installed one; if the tree's is newer, run tools/install-hooks.sh." >&2
fi
check() {
  "$python" -I "$guard" "$@"
  status=$?
  [ "$status" -eq 0 ] && return 0
  [ "$status" -eq 3 ] && stop "private content or a blocked file (listed above)"
  stop "the guard failed with status $status, so nothing was checked"
}
HOOK
    cat
  } > "$target.tmp"
  chmod +x "$target.tmp"
  mv "$target.tmp" "$target"
  echo "install-hooks: installed $target"
}

write_hook pre-commit commit <<'HOOK'
check --hook --staged </dev/null
HOOK

write_hook pre-merge-commit merge <<'HOOK'
check --hook --staged </dev/null
HOOK

write_hook commit-msg commit <<'HOOK'
check --hook --message-file "$1" </dev/null
HOOK

write_hook pre-push push <<'HOOK'
# stdin has one line per ref; the guard asks the remote what it already holds.
check --pre-push "$1" "$2"
HOOK
