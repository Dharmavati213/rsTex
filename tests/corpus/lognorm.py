#!/usr/bin/env -S python3 -I
r"""The normal form of an unwrapped TeX log, a line at a time, and its fingerprint.

    lognorm.py LOG WORKDIR      prints {"lines": N, "blocks": [...]}

Apart from run.py so that a log of hundreds of MB is hashed in a process of its own.  In a thread of the runner, the Python work on such logs
holds the GIL, and all the workers end up waiting for one core.
"""
import hashlib
import json
import re
import sys

STAT_LINE = re.compile(rb'^(\d+ strings of total length \d+|\d+ memory locations dumped.*|\d+ multiletter control sequences.*'
                       rb'|\d+ words of font info for \d+ preloaded fonts?|\d+ preloaded fonts?|Memory usage before:.*'
                       rb'|\d+ hyphenation exceptions?|Hyphenation trie of length.*)$')
DROP_LINE = {b'entering extended mode', b' restricted \\write18 enabled.', b' \\write18 enabled.',
             b' %&-line parsing enabled.', b'\\write18 disabled.'}
ENGINE_MESSAGE = re.compile(rb'^(!pdfTeX error: |pdfTeX warning: )[^ :]+(?=:| \(file )')
ENGINE_ANYWHERE = re.compile(rb'(?m)^(!pdfTeX error: |pdfTeX warning: )[^ :\n]+(?=:| \(file )')
OUTPUT_LINE = re.compile(rb'\((\d+ pages?), \d+ bytes\)\.')


def normalize_stream(path, workdir):
    """The normal form of run.py's normalize_log for an unwrapped log, as (raw line number, line), without holding the log in memory."""
    mask = workdir.encode('utf-8', 'surrogateescape') if workdir else None
    hide = b'#' * len(mask) if mask else b''
    skipping = False
    with open(path, 'rb') as f:
        for no, line in enumerate(f, 1):
            line = line.rstrip(b'\n')
            if mask and mask in line:
                line = line.replace(mask, hide)
            if skipping:
                if line.startswith(b' '):
                    continue
                skipping = False
            first = line[:1]
            if first in (b'{', b'.', b'\\'):           # nearly every line of a trace: nothing below applies
                yield no, line
                continue
            if (no == 1 and line.startswith(b'This is ')) or line in DROP_LINE or STAT_LINE.match(line):
                continue
            if line.startswith(b"Here is how much of TeX's memory") or line.startswith(b'PDF statistics:'):
                skipping = True
                continue
            if line.startswith(b'Output written on '):
                m = OUTPUT_LINE.search(line)
                yield no, b'Output written on <pdf> (' + (m.group(1) if m else b'?') + b').'
                continue
            if line.startswith((b'!pdfTeX error: ', b'pdfTeX warning: ')):
                line = ENGINE_MESSAGE.sub(rb'\1<engine>', line, count=1)
            yield no, line


def fingerprint(path, workdir):
    """(normal lines, [hash]): enough to tell two logs of any size apart.  This is a fast test, for equal logs, which is the usual case: the lines the normal form
    touches are at the two ends of a log, so only those go through normalize_stream, and the middle is hashed as it is.  Where it says two logs differ,
    the caller compares them with normalize_stream, a line at a time, and that decides."""
    try:
        with open(path, 'rb') as f:
            data = f.read()
    except OSError:
        return 0, ['missing']
    mask = workdir.encode('utf-8', 'surrogateescape') if workdir else None
    if mask:
        data = data.replace(mask, b'#' * len(mask))
    head_end = data.find(b'\n', 4096) + 1 or len(data)
    tail_start = max(head_end, data.rfind(b'\n', head_end, max(head_end, len(data) - (1 << 18))) + 1)
    body = data[head_end:tail_start]
    if b'pdfTeX' in body:
        body = ENGINE_ANYWHERE.sub(rb'\1<engine>', body)
    if b'Memory usage before' in body:
        body = re.sub(rb'(?m)^Memory usage before:.*\n', b'', body)
    h = hashlib.sha1()
    n = body.count(b'\n')
    for part in (data[:head_end], None, data[tail_start:]):
        if part is None:
            h.update(body)
            continue
        for line in _normal_lines(part, first=part is data[:head_end] or tail_start == 0):
            h.update(line + b'\n')
            n += 1
    return n, [h.hexdigest()[:24]]


def _normal_lines(data, first):
    skipping = False
    for no, line in enumerate(data.split(b'\n')):
        if skipping:
            if line.startswith(b' '):
                continue
            skipping = False
        if (first and no == 0 and line.startswith(b'This is ')) or line in DROP_LINE or STAT_LINE.match(line):
            continue
        if line.startswith(b"Here is how much of TeX's memory") or line.startswith(b'PDF statistics:'):
            skipping = True
            continue
        if line.startswith(b'Output written on '):
            m = OUTPUT_LINE.search(line)
            yield b'Output written on <pdf> (' + (m.group(1) if m else b'?') + b').'
            continue
        if line.startswith((b'!pdfTeX error: ', b'pdfTeX warning: ')):
            line = ENGINE_MESSAGE.sub(rb'\1<engine>', line, count=1)
        yield line


if __name__ == '__main__':
    lines, hashes = fingerprint(sys.argv[1], sys.argv[2])
    print(json.dumps({'lines': lines, 'blocks': hashes}))
