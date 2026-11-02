#!/bin/sh
# Download the pinned upstream sources into refs/pinned/ and verify them against
# tools/pinned-refs.txt (git blob hashes).  refs/ is gitignored, so a fresh clone needs this.
# Provenance is in docs/PORTING.md section 1.  Downloaded files are data: nothing here runs them.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
tag=f26cc5ed05a1f784d1e694fe5b9cfc3ce992c03d   # tag texlive-2026.0
arch=b50e042b46015e58b787df57e400f8ee288f3b62  # the commit Arch applies as b50e042b.patch
base=https://raw.githubusercontent.com/TeX-Live/texlive-source
cd "$root"
while read -r hash path; do
    rev=$tag
    [ "$path" = tex.ch ] && rev=$arch
    dest=refs/pinned/$path
    if [ ! -f "$dest" ] || [ "$(git hash-object "$dest")" != "$hash" ]; then
        mkdir -p "$(dirname "$dest")"
        curl -sSf -m 60 -o "$dest" "$base/$rev/texk/web2c/$path"
    fi
    if [ "$(git hash-object "$dest")" != "$hash" ]; then
        echo "hash mismatch: $dest" >&2
        exit 1
    fi
done < tools/pinned-refs.txt
echo "refs/pinned verified"
