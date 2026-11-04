#!/usr/bin/env -S python3 -I
r"""Engines with a wrong snapshot on purpose, so tests/corpus/snapshot/run_snapshot.py can show that it fails when it should.

    mutants.py stale ENGINE-ARGUMENTS...   the first run in a cache directory is kept and replayed for ever, whatever changes
    mutants.py naive ENGINE-ARGUMENTS...   a real preamble snapshot: mylatexformat dumps the preamble into a format the first time, keyed on the
                                           text of the main file's preamble and nothing else, the way a first attempt at this feature would

`naive` gets no credit for the log (a preamble that is skipped leaves no log prefix), so it is judged with --pdf-only.

And wrong output caches, for tests/corpus/snapshot/warm_equals_cold.py --keep-pdf.  Each runs the real engine, then does to the index what a wrong one would have written, sums and all:

    mutants.py oc-swap --real=ENGINE ARGUMENTS...            two streams have each other's places in the old file: a hit takes the wrong bytes, and their sums say they are right
    mutants.py oc-stream-of-font --real=ENGINE ARGUMENTS...  what stands for a font or a CMap names another stream
    mutants.py oc-font-lengths --real=ENGINE ARGUMENTS...    a font's entry gives another /Length1
    mutants.py oc-lost --real=ENGINE ARGUMENTS...            the index is lost after every run: right, but it never finds anything
    mutants.py oc-sums --real=ENGINE ARGUMENTS...            every sum is wrong: right, but nothing passes
    mutants.py oc-flip --real=ENGINE ARGUMENTS...            a bit of each stream in the old file, which the sums must see: right, but nothing passes
"""
import glob
import hashlib
import os
import re
import shutil
import struct
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))


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


class Index:
    """What `publish` of crates/engine/src/outcache.rs writes."""

    def __init__(self, path):
        self.path = path
        b = open(path, 'rb').read()[:-8]
        at = 8
        def u64():
            nonlocal at
            at += 8
            return struct.unpack_from('<Q', b, at - 8)[0]
        def vec():
            nonlocal at
            n = u64()
            at += n
            return b[at - n:at]
        vec()   # the build id
        u64()   # the CPU
        vec()   # the path
        u64()   # the size of the PDF
        self.head = b[:at]
        self.streams = []
        for _ in range(u64()):
            key = b[at:at + 16]
            at += 16
            self.streams.append([key, u64(), u64(), u64()])
        self.derived = []
        for _ in range(u64()):
            key, stream = b[at:at + 16], b[at + 16:at + 32]
            at += 32
            self.derived.append([key, stream, vec()])
        self.rest = b[at:]

    def write(self):
        o = bytearray(self.head)
        o += struct.pack('<Q', len(self.streams))
        for key, off, length, s in self.streams:
            o += key + struct.pack('<QQQ', off, length, s)
        o += struct.pack('<Q', len(self.derived))
        for key, stream, blob in self.derived:
            o += key + stream + struct.pack('<Q', len(blob)) + blob
        o += self.rest
        from run_snapshot import xxh64
        o += struct.pack('<Q', xxh64(bytes(o), 7))
        with open(self.path, 'wb') as f:
            f.write(o)


def oc_swap(ix, pdf):
    if len(ix.streams) >= 2:
        a, b = ix.streams[0], ix.streams[1]
        a[1:], b[1:] = b[1:], a[1:]


def oc_stream_of_font(ix, pdf):
    if ix.derived:
        others = [s[0] for s in ix.streams if s[0] != ix.derived[0][1]]
        if others:
            ix.derived[0][1] = others[0]


def oc_font_lengths(ix, pdf):
    for d in ix.derived:
        if d[2]:
            d[2] = struct.pack('<Q', struct.unpack_from('<Q', d[2])[0] + 1) + d[2][8:]
            return


def oc_sums(ix, pdf):
    for s in ix.streams:
        s[3] ^= 1


def oc_lost(ix, pdf):
    os.unlink(ix.path)
    return True


def oc_flip(ix, pdf):
    if os.path.exists(pdf):
        b = bytearray(open(pdf, 'rb').read())
        for s in ix.streams:
            b[s[1] + s[2] // 2] ^= 1
        with open(pdf, 'wb') as f:
            f.write(b)
    return True


OUTPUT_CACHE = {'oc-swap': oc_swap, 'oc-stream-of-font': oc_stream_of_font, 'oc-font-lengths': oc_font_lengths, 'oc-sums': oc_sums, 'oc-lost': oc_lost, 'oc-flip': oc_flip}


def output_cache(kind, real, args):
    out, _, job = parse(args)
    pdf = os.path.join(out, job + '.pdf')
    indexes = lambda: glob.glob(os.path.join(os.environ['XDG_CACHE_HOME'], 'rstex', 'outcache', '*.oc'))
    # The damage to the old file, which is what the run is about to read and has to leave right.
    if kind == 'oc-flip':
        for path in indexes():
            oc_flip(Index(path), pdf)
    rc = subprocess.run([real] + args).returncode
    if kind != 'oc-flip':
        for path in indexes():
            ix = Index(path)
            if not OUTPUT_CACHE[kind](ix, pdf):
                ix.write()
    return rc


def main():
    kind = sys.argv[1]
    if kind in OUTPUT_CACHE:
        return output_cache(kind, sys.argv[2].split('=', 1)[1], sys.argv[3:])
    args = sys.argv[2:]
    return {'stale': stale, 'naive': naive}[kind](args)


if __name__ == '__main__':
    sys.exit(main())
