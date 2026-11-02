#!/bin/sh
# Install the privacy guard as this clone's pre-commit and pre-push hooks. Run it once after every clone: git does
# not copy hooks. Hooks are local and `git commit --no-verify` or `git push --no-verify` skips them; see docs/DESIGN.md.
# Both hooks fail closed: if the guard is missing, crashes or cannot run, the commit or push stops.
set -eu

marker='rstex-privacy-guard'
root=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
hooks=$(git -C "$root" rev-parse --git-path hooks)
case "$hooks" in
  /*) ;;
  *) hooks=$root/$hooks ;;
esac

configured=$(git -C "$root" config --get core.hooksPath || true)
if [ -n "$configured" ]; then
  echo "install-hooks: core.hooksPath is set to $configured, so hooks in $hooks would not run." >&2
  echo "install-hooks: unset it or point it here, then run this again." >&2
  exit 1
fi

mkdir -p "$hooks"

write_hook() { # name, with the body on stdin
  target=$hooks/$1
  if [ -e "$target" ] && ! grep -q "$marker" "$target"; then
    echo "install-hooks: $target exists and is not ours; merge it by hand." >&2
    exit 1
  fi
  cat > "$target.tmp"
  chmod +x "$target.tmp"
  mv "$target.tmp" "$target"
  echo "install-hooks: installed $target"
}

write_hook pre-commit <<'HOOK'
#!/bin/sh
# rstex-privacy-guard: written by tools/install-hooks.sh. Fails closed.
stop() { echo "privacy guard: $1; commit stopped." >&2; exit 1; }
root=$(git rev-parse --show-toplevel) || stop "cannot find the repository root"
guard=$root/tools/privacy_guard.py
[ -f "$guard" ] || stop "tools/privacy_guard.py is missing"
command -v python3 >/dev/null 2>&1 || stop "python3 is not available"
python3 -I "$guard" --staged </dev/null
status=$?
[ "$status" -eq 0 ] && exit 0
[ "$status" -eq 1 ] && stop "private content or a blocked file is staged (listed above)"
stop "the guard failed with status $status, so nothing was checked"
HOOK

write_hook pre-push <<'HOOK'
#!/bin/sh
# rstex-privacy-guard: written by tools/install-hooks.sh. Fails closed.
# stdin has one line per ref: local-ref local-sha remote-ref remote-sha, all zeros meaning "no such object".
stop() { echo "privacy guard: $1; push stopped." >&2; exit 1; }
remote=${1:-}
root=$(git rev-parse --show-toplevel) || stop "cannot find the repository root"
guard=$root/tools/privacy_guard.py
[ -f "$guard" ] || stop "tools/privacy_guard.py is missing"
command -v python3 >/dev/null 2>&1 || stop "python3 is not available"
is_zero() { case "$1" in *[!0]*) return 1 ;; *) return 0 ;; esac; }
while read -r local_ref local_sha remote_ref remote_sha; do
  # Deleting a remote ref sends no objects.
  is_zero "$local_sha" && continue
  if ! is_zero "$remote_sha" && git cat-file -e "$remote_sha" 2>/dev/null; then
    spec="$remote_sha..$local_sha"
  elif git config --get "remote.$remote.url" >/dev/null 2>&1; then
    # New branch or tag, or a force-push over commits we do not have: all that the remote is not known to hold.
    spec="$local_sha --not --remotes=$remote"
  else
    # A bare URL: nothing is known about what the other side holds, so everything reachable is scanned.
    spec="$local_sha"
  fi
  python3 -I "$guard" --range "$spec" --ref-name "$local_ref" --ref-name "$remote_ref" </dev/null
  status=$?
  [ "$status" -eq 0 ] && continue
  [ "$status" -eq 1 ] && stop "private content or a blocked file is in what $local_ref would send (listed above)"
  stop "the guard failed with status $status, so nothing was checked"
done
exit 0
HOOK
