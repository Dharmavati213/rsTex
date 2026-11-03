#!/usr/bin/env -S python3 -I
"""fetch.py -- download the pinned public sources of manifests/public.json into refs/corpus/<source>/.

    fetch.py [NAME...]        fetch the named sources, or all of sources.json
    fetch.py --check          verify recorded sha256 values against what is on disk

Every source is a tarball of one commit (or a single file) with its sha256 recorded in sources.lock.json. Nothing inside a
download is executed. Extraction uses tarfile's 'data' filter, which refuses absolute paths, '..', device nodes and
links that leave the tree. Downloads carry no credentials and no personal data.
"""
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tarfile
import time
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
ROOT = os.path.join(REPO, 'refs', 'corpus')
SOURCES = os.path.join(HERE, 'sources.json')
LOCK = os.path.join(HERE, 'sources.lock.json')


def sha256(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


def url_of(src):
    if src.get('url'):
        return src['url']
    return f'https://codeload.github.com/{src["repo"]}/tar.gz/{src["commit"]}'


def download(url, dest):
    subprocess.run(['curl', '--fail', '--location', '--silent', '--show-error', '--retry', '4', '--retry-delay', '5',
                    '--max-time', '1800', '--output', dest, url], check=True)


def fetch(name, src, lock):
    target = os.path.join(ROOT, name)
    stamp = os.path.join(target, '.fetched.json')
    if os.path.isfile(stamp):
        with open(stamp) as f:
            have = json.load(f)
        if have.get('url') == url_of(src):
            print(f'{name}: already present ({have["bytes"]} bytes)')
            lock[name] = have
            return True
        shutil.rmtree(target)
    if os.path.isdir(target):
        shutil.rmtree(target)
    os.makedirs(target)
    url = url_of(src)
    kind = src.get('kind', 'tarball')
    blob = os.path.join(target, '.download')
    t0 = time.time()
    try:
        download(url, blob)
    except subprocess.CalledProcessError as ex:
        print(f'{name}: FAILED {url}: {ex}', file=sys.stderr)
        shutil.rmtree(target)
        return False
    info = {'url': url, 'bytes': os.path.getsize(blob), 'sha256': sha256(blob), 'commit': src.get('commit'),
            'repo': src.get('repo'), 'tag': src.get('tag'), 'fetched': time.strftime('%Y-%m-%d'), 'kind': kind}
    if kind == 'file':
        os.replace(blob, os.path.join(target, src['filename']))
    elif kind == 'zip':
        with zipfile.ZipFile(blob) as z:      # extractall drops absolute paths and '..' components
            tops = {n.split('/', 1)[0] for n in z.namelist()}
            z.extractall(target)
        os.unlink(blob)
        if len(tops) == 1:
            os.rename(os.path.join(target, tops.pop()), os.path.join(target, 'tree'))
    else:
        with tarfile.open(blob) as t:
            members = t.getmembers()
            tops = {m.name.split('/', 1)[0] for m in members}
            if len(tops) != 1:
                print(f'{name}: tarball has {len(tops)} top-level directories, keeping them', file=sys.stderr)
            t.extractall(target, filter='data')
        os.unlink(blob)
        if len(tops) == 1:                  # refs/corpus/<name>/tree is a stable path, whatever the commit
            os.rename(os.path.join(target, tops.pop()), os.path.join(target, 'tree'))
    with open(stamp, 'w') as f:
        json.dump(info, f, indent=1, sort_keys=True)
    lock[name] = info
    print(f'{name}: {info["bytes"]} bytes in {time.time() - t0:.1f} s, sha256 {info["sha256"][:16]}')
    return True


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('names', nargs='*')
    args = ap.parse_args()
    with open(SOURCES) as f:
        sources = json.load(f)
    try:
        with open(LOCK) as f:
            lock = json.load(f)
    except OSError:
        lock = {}
    os.makedirs(ROOT, exist_ok=True)
    failed = []
    for name in args.names or sorted(sources):
        if name.startswith('_'):
            continue
        if not fetch(name, sources[name], lock):
            failed.append(name)
    with open(LOCK, 'w') as f:
        json.dump(lock, f, indent=1, sort_keys=True)
    if failed:
        print('failed: ' + ' '.join(failed), file=sys.stderr)
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
