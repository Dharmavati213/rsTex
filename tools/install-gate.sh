#!/bin/sh
# Route pushes to a remote through a local bare repository whose pre-receive hook runs the privacy guard and then forwards
# to the real remote. Hooks in a clone can be skipped (--no-verify, core.hooksPath, git send-pack, a guard edited in place);
# a server-side hook cannot be skipped by the client.
#
#     tools/install-gate.sh [REMOTE]        default origin; contacts the remote once, to learn what it holds
#     git remote set-url --delete --push REMOTE "$(git remote get-url --push REMOTE)"       to go back to pushing directly
#
# What this does not stop: a push that names the real URL, a clone without this, or anything else that talks to the host,
# such as `gh` or an API call. All of them run as the same user. It turns accidents and habits into failures, not attacks
# into failures. Keeping the host's credentials where only the gate finds them would go further; see docs/DESIGN.md.
set -eu

remote=${1:-origin}
root=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
cd "$root"
common=$(cd "$(git rev-parse --git-common-dir)" && pwd)
gate=$common/rstex-guard/gate.git
python=$(command -v python3 || true)
if [ -z "$python" ]; then
  echo "install-gate: python3 is not available." >&2
  exit 1
fi
upstream=$(git config --get "rstex.gateupstream.$remote" || git remote get-url "$remote")
if [ "$upstream" = "$gate" ]; then
  echo "install-gate: $remote already points at the gate and the upstream is not recorded." >&2
  exit 1
fi

[ -d "$gate" ] || git init -q --bare "$gate"
# The gate starts from what the remote holds: everything else has to come through it.
git -C "$gate" fetch -q "$upstream" '+refs/heads/*:refs/heads/*' '+refs/tags/*:refs/tags/*'
mkdir -p "$gate/rstex-guard"
cp "$root/tools/privacy_guard.py" "$gate/rstex-guard/privacy_guard.py"
git -C "$gate" config rstex.upstream "$upstream"
(cd "$gate" && unset GIT_DIR && "$python" -I rstex-guard/privacy_guard.py --install-config)
ln -sf ../../approved-binaries.txt "$gate/rstex-guard/approved-binaries.txt"

{
  printf "#!/bin/sh\n# rstex-privacy-guard gate: written by tools/install-gate.sh. Fails closed.\npython='%s'\n" "$python"
  cat <<'HOOK'
stop() { echo "privacy gate: $1; push refused." >&2; exit 1; }
upstream=$(git config --local --get rstex.upstream) || stop "no upstream is recorded"
[ -x "$python" ] || stop "$python is not available"
input=$(cat)
printf '%s\n' "$input" | "$python" -I rstex-guard/privacy_guard.py --pre-receive
status=$?
[ "$status" -eq 3 ] && stop "private content or a blocked file (listed above)"
[ "$status" -eq 0 ] || stop "the guard failed with status $status, so nothing was checked"
# Forward before accepting, so that the gate never holds what the remote refused. Each ref must still be where the
# gate last saw it, or someone has pushed past the gate.
leases=
specs=
while read -r old new ref; do
  case "$old" in *[!0]*) lease=$old ;; *) lease= ;; esac
  case "$new" in *[!0]*) spec=+$new:$ref ;; *) spec=:$ref ;; esac
  leases="$leases --force-with-lease=$ref:$lease"
  specs="$specs $spec"
done <<REFS
$input
REFS
# shellcheck disable=SC2086 # ref names have no spaces
git push -q --atomic $leases "$upstream" $specs || stop "the remote refused it"
HOOK
} > "$gate/hooks/pre-receive"
chmod +x "$gate/hooks/pre-receive"

current=$(git remote get-url --push "$remote")
if [ "$current" != "$gate" ]; then
  git config "rstex.gateupstream.$remote" "$upstream"
  git remote set-url --push "$remote" "$gate"
fi
echo "install-gate: pushes to $remote now go through $gate, then to the upstream recorded there."
