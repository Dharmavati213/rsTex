#!/usr/bin/env -S python3 -I
r"""What a difference on known_divergences.txt is: a token per level, which two machines agree on.

    difftoken.py inst ORACLE_LOG ORACLE_OUT CANDIDATE_LOG CANDIDATE_OUT MASKS.json [SHOW_DIR]     prints {"L1": token or null, "L2": token or null}

A token is -N+M:HASH, as in tests/integration/run.py: N lines of the oracle's and M of the candidate's, from the first line that differs to the last, and a hash of those lines.  Anything else that
changes moves those limits or changes that text.  What a machine puts in those lines comes out first:

- A path that depends on where the tree is, the home directory, the output directory or the engine: Masks, each by a word of its own, so its length does not count either.
- TeX's wrapping, which those lengths and the length of the program's name move: logical_lines puts back what TeX cut at max_print_line, and the new line that section 537 starts for a file name
  too long for the rest of this one.  The instrumented pass has max_print_line=1000000, and neither happens there.
- A macro's text is shown to 32 tokens in a trace, so a path there shows as much of itself as there is room for, and what follows it shows only if the path is short: Masks.trace.  So a line that differs only
  in such a path, as the format's in a document that reads its own .fls, leaves nothing here, and its level has the token -0+0:0000000000000000.

A log of hundreds of MB is compared here in a process of its own, as in lognorm.py.
"""
import array
import hashlib
import hmac
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lognorm  # noqa: E402

# The levels in the order a list entry gives them.  The first three come from the instrumented pass, which is one per entry and starts from the last pass's state in either mode.
INST_LEVELS = ('L1', 'L2', 'L6')
LEVELS = ('L0', 'L1', 'L2', 'L4', 'L5', 'L6', 'log', 'log_strict', 'exit')
SYMBOLS = (b'<OUT>', b'<CWD>', b'<TREE>', b'<HOME>', b'<ENGINE>')
_SYMBOL = re.compile(b'|'.join(re.escape(s) for s in SYMBOLS))
_FILE_OPENS = tuple(b'(' + s for s in SYMBOLS)
_TRUNCATED_PATH = re.compile(rb'/[^ {}\\]*(?=\\?ETC\.\})')       # with no backslash under \escapechar=-1


class Masks:
    """[(path, symbol)], longest first, so that the output directory goes before the tree that holds it.  A path masked by a run of # of its own length, as normalize_log leaves it, is one of these too."""

    def __init__(self, pairs):
        self.pairs = sorted(((os.fsencode(p) if isinstance(p, str) else p, s) for p, s in pairs if p), key=lambda x: -len(x[0]))

    def apply(self, line):
        for path, symbol in self.pairs:
            if path in line:
                line = line.replace(path, symbol)
        return line

    def trace(self, line):
        r"""A line of \tracingassigns or \tracingrestores.  It shows a macro's text to 32 tokens, so a path there may be cut off, and the cut comes elsewhere in another tree, and what comes after a path
        shows or does not by the length of the path.  So from the first path on, the line says only that there was one.  A word that begins with a slash and is cut off may be any path: "/m" is the start of one
        under /mnt, and of no path here, so that cannot be told by looking at this machine's paths, and every such word goes."""
        text = self.apply(line)
        if text[:1] != b'{':
            return text
        starts = [x.start() for x in (_SYMBOL.search(text), _TRUNCATED_PATH.search(text)) if x]
        return text[:min(starts)] + b'<PATH>' if starts else text


def logical_lines(lines, max_line, masks):
    """The lines of a log with TeX's wrapping undone and the machine's paths taken out.  A line of exactly max_line columns was cut there, and goes on on the next one.  Section 537 starts a new line for
    a file name that would not fit, under batchmode for one that is itself near max_line long, which a short path is not: so such a name goes back where a short one would have stayed, and the empty
    line it left at column 0 goes too.  lines: bytes."""
    out, held = [], b''
    for line in list(lines) + [b'']:
        held += line
        if len(line) == max_line:
            continue
        text, held = masks.trace(held), b''
        if text.startswith(_FILE_OPENS) and out:
            if not out[-1] and len(out) > 1:
                out.pop()
            out[-1] += b' ' + text
        else:
            out.append(text)
    return out


def between(a, b):
    """(the oracle's lines, the candidate's), from the first that differs to the last, or None.  a, b: sequences of comparable lines."""
    i, n = 0, min(len(a), len(b))
    while i < n and a[i] == b[i]:
        i += 1
    if i == len(a) == len(b):
        return None
    j = 0
    while j < len(a) - i and j < len(b) - i and a[-1 - j] == b[-1 - j]:
        j += 1
    return a[i:len(a) - j], b[i:len(b) - j]


class Token:
    """-N+M:HASH over what is added.  key: for a private document, an HMAC under a key made from that document, so that the hash of a short line cannot be tried against guesses by someone who does not
    have the document, and someone who has it has no need to."""

    def __init__(self, key=None):
        self.h = hmac.new(bytes.fromhex(key), digestmod='sha256') if key else hashlib.sha256()
        self.n = self.m = 0

    def add(self, where, a, b):
        self.h.update(where.encode() + b'\0')
        for line in a:
            self.h.update(b'-' + line + b'\n')
        for line in b:
            self.h.update(b'+' + line + b'\n')
        self.n += len(a)
        self.m += len(b)

    def text(self):
        return f'-{self.n}+{self.m}:{self.h.hexdigest()[:16]}'


def token_of(parts, key=None, show=None):
    """The token of [(where, the oracle's lines, the candidate's)], or None if there is nothing there."""
    parts = [p for p in parts if p]
    if not parts:
        return None
    t = Token(key)
    for where, a, b in parts:
        t.add(where, a, b)
        if show is not None:
            show.extend(_shown(where, a, b))
    return t.text()


def _shown(where, a, b):
    yield f'@@ {where}: -{len(a)} +{len(b)}'.encode()
    for line in a:
        yield b'- ' + line
    for line in b:
        yield b'+ ' + line


def lines_token(where, a, b, key=None, show=None):
    """The token of two sequences of lines.  What lies between the first and the last difference counts, so nothing can change there unseen."""
    d = between(a, b)
    return token_of([(where, d[0], d[1])] if d else [], key, show)


def write_shown(path, lines):
    """A difference to read, of a size that can be read: 40000 lines at most, 2000 columns each."""
    lines = list(lines)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    if len(lines) > 40000:
        lines = lines[:20000] + [b'... %d lines not shown ...' % (len(lines) - 40000)] + lines[-20000:]
    with open(path, 'wb') as f:
        f.write(b'\n'.join(x[:2000] for x in lines) + b'\n')


def normal_text(path, workdir):
    """The log as normal lines, as bytes, in the way of lognorm.fingerprint: the normal form touches the two ends of a log and a line or so in between, so that is a matter of seconds where a line at a time is
    minutes on a log of 500 MB."""
    with open(path, 'rb') as f:
        data = f.read()
    mask = os.fsencode(workdir)
    data = data.replace(mask, b'#' * len(mask))
    head_end = data.find(b'\n', 4096) + 1 or len(data)
    tail_start = max(head_end, data.rfind(b'\n', head_end, max(head_end, len(data) - (1 << 18))) + 1)
    body = data[head_end:tail_start]
    if b'pdfTeX' in body:
        body = lognorm.ENGINE_ANYWHERE.sub(rb'\1<engine>', body)
    if b'Memory usage before' in body:
        body = re.sub(rb'(?m)^Memory usage before:.*\n', b'', body)

    def end_part(part, first):
        lines = list(lognorm._normal_lines(part, first=first))
        if part.endswith(b'\n') and lines and not lines[-1]:
            lines.pop()
        return b''.join(x + b'\n' for x in lines)
    return end_part(data[:head_end], True) + body + end_part(data[tail_start:], False)


def common_prefix(a, b, step=1 << 20):
    n, i = min(len(a), len(b)), 0
    while i < n and a[i:i + step] == b[i:i + step]:
        i += step
    if i >= n:
        return n
    x, y = a[i:i + step], b[i:i + step]
    k = next((k for k in range(min(len(x), len(y))) if x[k] != y[k]), min(len(x), len(y)))
    return i + k


def common_suffix(a, b, room, step=1 << 20):
    """How many bytes the two end in alike, at most room."""
    i = 0
    while i < room:
        w = min(step, room - i)
        x, y = a[len(a) - i - w:len(a) - i], b[len(b) - i - w:len(b) - i]
        if x != y:
            return i + next(k for k in range(w) if x[w - 1 - k] != y[w - 1 - k])
        i += w
    return room


def parts_of(text, masks, keep):
    """A trace as the two parts the levels judge, each line by 8 bytes of its hash, and whether the last line was in a box dump.  keep: the lines as well, for reading them."""
    parts = {True: array.array('Q'), False: array.array('Q')}
    kept = {True: [], False: []}
    inside = False
    for inside, (_, line) in lognorm.split_box_lines(enumerate(text.split(b'\n'))):
        line = masks.apply(line) if inside else masks.trace(line)
        parts[inside].append(int.from_bytes(hashlib.blake2b(line, digest_size=8).digest(), 'little'))
        if keep:
            kept[inside].append(line)
    return parts, kept, inside


def inst_tokens(oracle_log, oracle_out, cand_log, cand_out, masks_json, show_dir=None):
    """The tokens of L1 and L2 for one instrumented pass.  What the two logs begin and end in alike is taken off first, as such lines cannot be the first or the last that differ, as long as each side is in the
    same part of the log after them: from the start of the box dump the first difference is in, to one line into what they end in.  If they do not end up in the same part, all of it is read."""
    spec = json.loads(masks_json) if isinstance(masks_json, str) else masks_json
    key = spec.get('key')
    pairs = [(p, s.encode()) for p, s in spec['paths']]
    mo = Masks(pairs + [(b'#' * len(os.fsencode(oracle_out)), b'<OUT>')] + [(p, s.encode()) for p, s in spec.get('oracle', [])])
    mc = Masks(pairs + [(b'#' * len(os.fsencode(cand_out)), b'<OUT>')] + [(p, s.encode()) for p, s in spec.get('candidate', [])])
    ta, tb = normal_text(oracle_log, oracle_out), normal_text(cand_log, cand_out)
    i = common_prefix(ta, tb)
    i = ta.rfind(b'\n', 0, i) + 1
    dump = ta.rfind(b'Completed box being shipped out [', 0, i)
    start = ta.rfind(b'\n', 0, dump) + 1 if dump >= 0 else i
    s = common_suffix(ta, tb, min(len(ta), len(tb)) - i)
    stop_a, stop_b = ta.find(b'\n', len(ta) - s), tb.find(b'\n', len(tb) - s)
    if s and stop_a >= 0 and stop_b >= 0:          # the first whole line they end in alike, and one more
        more_a, more_b = ta.find(b'\n', stop_a + 1), tb.find(b'\n', stop_b + 1)
        stop_a, stop_b = (more_a, more_b) if more_a >= 0 and more_b >= 0 else (len(ta), len(tb))
    else:
        stop_a, stop_b = len(ta), len(tb)
    a, ka, ia = parts_of(ta[start:stop_a], mo, show_dir)
    b, kb, ib = parts_of(tb[start:stop_b], mc, show_dir)
    if ia != ib:
        a, ka, _ = parts_of(ta, mo, show_dir)
        b, kb, _ = parts_of(tb, mc, show_dir)
    res = {}
    for inside, level in ((False, 'L1'), (True, 'L2')):
        x, y = a[inside], b[inside]
        d = between(x, y)
        if not d:
            res[level] = None
            continue
        t = Token(key)
        t.add(level, [v.to_bytes(8, 'little') for v in d[0]], [v.to_bytes(8, 'little') for v in d[1]])
        res[level] = t.text()
        if show_dir:
            k = next((k for k in range(min(len(x), len(y))) if x[k] != y[k]), min(len(x), len(y)))
            write_shown(os.path.join(show_dir, level + '.txt'), _limited(level, ka[inside][k:k + len(d[0])], kb[inside][k:k + len(d[1])], len(d[0]), len(d[1])))
    return res


def _limited(where, a, b, n, m, room=4000):
    """A difference to read, of a size that can be read.  Past that the middle is left out, and it says so."""
    yield f'@@ {where}: -{n} +{m}'.encode()
    for sign, lines, count in ((b'- ', a, n), (b'+ ', b, m)):
        for k, line in enumerate(lines):
            if k < room // 2 or k >= count - room // 2:
                yield sign + line[:2000]
            elif k == room // 2:
                yield sign + b'... %d lines not shown ...' % (count - room)


if __name__ == '__main__':
    if sys.argv[1] == 'inst':
        print(json.dumps(inst_tokens(*sys.argv[2:])))
    else:
        sys.exit('difftoken.py inst ...')
