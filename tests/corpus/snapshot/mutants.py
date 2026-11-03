#!/usr/bin/env -S python3 -I
r"""Engines with a wrong snapshot on purpose, so tests/corpus/snapshot/run_snapshot.py can show that it fails when it should.

    mutants.py stale ENGINE-ARGUMENTS...   the first run in a cache directory is kept and replayed for ever, whatever changes
    mutants.py naive ENGINE-ARGUMENTS...   a real preamble snapshot: mylatexformat dumps the preamble into a format the first time, keyed on the
                                           text of the main file's preamble and nothing else, the way a first attempt at this feature would

`naive` gets no credit for the log (a preamble that is skipped leaves no log prefix), so it is judged with --pdf-only.
"""
import hashlib
import os
import re
import shutil
import subprocess
import sys


def parse(args):
    out = next((a.split('=', 1)[1] for a in args if a.startswith('-output-directory=')), '.')
    last = args[-1]
    file_arg = last
    if last.startswith('\\'):
        m = re.search(r'\\input\s*\{?"?([^"}\s]*)', last)
        file_arg = m.group(1) if m else last
    job = next((a.split('=', 1)[1] for a in args if a.startswith('-jobname=')), os.path.splitext(os.path.basename(file_arg))[0])
    return out, file_arg, job


def stale(args):
    cache = os.environ['XDG_CACHE_HOME']
    out, _, job = parse(args)
    key = 'the-only-slot'
    d = os.path.join(cache, key)
    if os.path.isdir(os.path.join(d, 'files')):
        shutil.copytree(os.path.join(d, 'files'), out, dirs_exist_ok=True)
        with open(os.path.join(d, 'rc')) as f:
            return int(f.read())
    p = subprocess.run([shutil.which('pdflatex')] + args)
    os.makedirs(d, exist_ok=True)
    shutil.copytree(out, os.path.join(d, 'files'), dirs_exist_ok=True)
    with open(os.path.join(d, 'rc'), 'w') as f:
        f.write(str(p.returncode))
    return p.returncode


def naive(args):
    cache = os.environ['XDG_CACHE_HOME']
    out, file_arg, job = parse(args)
    engine = shutil.which('pdflatex')
    if not os.path.isfile(file_arg):
        return subprocess.run([engine] + args).returncode
    with open(file_arg, 'rb') as f:
        text = f.read()
    k = text.find(b'\\begin{document}')
    if k < 0:
        return subprocess.run([engine] + args).returncode
    key = hashlib.sha1(text[:k]).hexdigest()[:12]
    fmt = os.path.join(cache, f'pre-{key}.fmt')
    os.makedirs(cache, exist_ok=True)
    if not os.path.isfile(fmt):
        # mylatexformat reads the preamble of the file and dumps a format, which then skips that preamble.  Ours is the part of the
        # command line after the flags; the dump goes to the cache.
        keep = [a for a in args if a.startswith(('-output-directory', '-interaction', '-shell', '-no-shell'))]
        keep = [f'-output-directory={cache}' if a.startswith('-output-directory') else a for a in keep]
        p = subprocess.run([engine, '-ini', f'-jobname=pre-{key}', '&pdflatex', 'mylatexformat.ltx', file_arg] if False else
                           [engine, '-ini', f'-jobname=pre-{key}'] + keep + [f'&pdflatex mylatexformat.ltx {file_arg}'], capture_output=True)
        if not os.path.isfile(fmt):
            _ = p
            return subprocess.run([engine] + args).returncode
    new = [a for a in args if not a.startswith('-fmt=')]
    env = dict(os.environ)
    env['TEXFORMATS'] = cache + ':'
    return subprocess.run([engine, f'-fmt=pre-{key}'] + new, env=env).returncode


def main():
    kind = sys.argv[1]
    args = sys.argv[2:]
    return {'stale': stale, 'naive': naive}[kind](args)


if __name__ == '__main__':
    sys.exit(main())
