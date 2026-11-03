#!/usr/bin/env -S python3 -I
r"""Candidate engines with one known bug each, for tests/corpus/mutate/matrix.py.

    bugs.py BUG ENGINE-ARGUMENTS...     behave like pdflatex with bug BUG

A self-check that only perturbs what the checker already looks at proves little, so each bug here is a realistic engine
defect, made in one of three ways:

  prelude   TeX run before the document, so everything downstream is what an engine with that defect would produce
  tfm       the real engine reads a TFM with one changed entry: a front-end bug seen by line breaking, boxes and PDF alike
  pdf, log  a same-length byte edit of an uncompressed output file, so every other byte stays what the oracle wrote

A bug that does not apply exits 125, so a mutation that changed nothing cannot be mistaken for one that went undetected.
Each run that applied its bug appends a line to $MUTATE_STATE/applied.
"""
import hashlib
import os
import re
import shutil
import subprocess
import sys


class NotApplied(Exception):
    pass


def bump_last_digit(s):
    """Same length, different value."""
    m = re.search(rb'(\d)(?!.*\d)', s, re.S)
    if not m:
        raise NotApplied(f'no digit in {s!r}')
    d = int(m.group(1))
    return s[:m.start(1)] + str((d + 1) % 10).encode() + s[m.end(1):]


def own_banner(data):
    """Not a bug: an engine that says its own name, as rstex does.  Where the file has a classic cross-reference table, which is under L6, the banner gets another length and the table
    is put right, as such an engine would write it.  Rewriting a cross-reference stream would take a deflate, so elsewhere it keeps the length."""
    old, new = b'TeX Live 2026/Arch Linux', b'rstex 0.1'
    k = data.find(old)
    if k < 0:
        raise NotApplied('no banner')
    m = re.search(rb'\nxref\n(\d+) (\d+)\n', data)
    if not m:
        return data.replace(old, new.ljust(len(old), b'x'))
    delta = len(new) - len(old)
    start, count = m.end(), int(m.group(2))
    table = bytearray(data[start:start + 20 * count])
    for i in range(count):
        entry = bytes(table[20 * i:20 * i + 20])
        offset = int(entry[:10])
        if entry[17:18] == b'n' and offset > k:
            table[20 * i:20 * i + 10] = b'%010d' % (offset + delta)
    tail = re.sub(rb'startxref\n(\d+)', lambda x: b'startxref\n%d' % (int(x.group(1)) + delta), data[start + 20 * count:], count=1)
    return data[:k] + new + data[k + len(old):m.start()] + data[m.start():start] + bytes(table) + tail


def sub_once(data, pattern, repl, flags=0):
    new, n = re.subn(pattern, repl, data, count=1, flags=flags)
    if n != 1 or new == data:
        raise NotApplied(f'pattern {pattern!r} did not apply')
    return new


def stream_of(data, head_pattern):
    """(start, end) of the bytes of the first stream whose dictionary matches."""
    m = re.search(head_pattern + rb'\s*>>\s*stream\n', data, re.S)
    if not m:
        raise NotApplied(f'no stream with {head_pattern!r}')
    end = data.index(b'endstream', m.end())
    return m.end(), end


def edit_content_number(data):
    """One digit of the first TJ kern in the first page's content stream."""
    start, end = stream_of(data, rb'<<\s*/Length \d+')
    body = data[start:end]
    m = re.search(rb'\)(-?\d+)\(', body)
    if not m:
        raise NotApplied('no TJ kern')
    return data[:start + m.start(1)] + bump_last_digit(m.group(1)) + data[start + m.end(1):]


def edit_content_td(data):
    """The last digit of a Td operand: a shift of 0.001 bp, about 65 sp."""
    start, end = stream_of(data, rb'<<\s*/Length \d+')
    body = data[start:end]
    m = re.search(rb'(-?\d+\.\d+) (-?\d+\.\d+) Td', body)
    if not m:
        raise NotApplied('no Td')
    return data[:start + m.start(2)] + bump_last_digit(m.group(2)) + data[start + m.end(2):]


def edit_fontfile_byte(data):
    start, end = stream_of(data, rb'<<\s*/Length1 \d+\s*/Length2 \d+\s*/Length3 \d+\s*/Length \d+')
    k = start + (end - start) * 3 // 4
    return data[:k] + bytes([data[k] ^ 1]) + data[k + 1:]


def edit_tounicode(data):
    m = re.search(rb'beginbfchar\n<[0-9A-Fa-f]+> <([0-9A-Fa-f]+)>', data)
    if not m:
        raise NotApplied('no bfchar')
    return data[:m.start(1)] + bump_last_digit(m.group(1)) + data[m.end(1):]


def _swap_charset(m):
    return b'/CharSet (' + m.group(2) + m.group(1) + m.group(3)


# Each edit works on a PDF written under \pdfcompresslevel=0 \pdfobjcompresslevel=0, the entry mut/plain.
def bump_ref(pattern):
    r"""Same-length change of the object number captured by group 1 of the first match: the reference now names another object."""
    def edit(data):
        m = re.search(pattern, data)
        if not m:
            raise NotApplied(f'pattern {pattern!r} did not apply')
        return data[:m.start(1)] + bump_last_digit(m.group(1)) + data[m.end(1):]
    return edit


def number_in(pattern):
    return bump_ref(pattern)


PDF_EDITS = {
    'kern-pdf': edit_content_number,
    'glue-pdf': edit_content_td,
    'subset-fontfile-byte': edit_fontfile_byte,
    'subset-charset': lambda d: sub_once(d, rb'/CharSet \((/[A-Za-z]+)(/[A-Za-z]+)(/)', _swap_charset),
    'widths': lambda d: sub_once(d, rb'\[607\.3\]', b'[607.4]'),
    'tounicode-entry': edit_tounicode,
    'tounicode-dropped': lambda d: sub_once(d, rb'/ToUnicode (\d+ 0 R)', rb'/ToUnikode \1'),
    'link-rect': number_in(rb'/Rect \[(\d+)\.\d+ \d+\.\d+ \d+\.\d+ \d+\.\d+\]'),
    'link-dest': lambda d: sub_once(d, rb'/A << /S /GoTo /D \(section\.1\)', b'/A << /S /GoTo /D (section.2)'),
    'link-uri': lambda d: sub_once(d, rb'/URI\(https://example\.org/\)', b'/URI(https://example.orh/)'),
    'link-border': lambda d: sub_once(d, rb'/Border\[0 0 1\]/H/I/C\[0 1 1\]', b'/Border[0 0 2]/H/I/C[0 1 1]'),
    'dest-xyz': number_in(rb'/D \[\d+ 0 R /XYZ \d+\.\d+ (\d+)\.\d+ null\]'),
    'dest-page': number_in(rb'/D \[(\d+) 0 R /XYZ \d+\.\d+ \d+\.\d+ null\]'),
    'outline-count': lambda d: sub_once(d, rb'(/Last \d+ 0 R\s*/Count )-1', rb'\g<1>+1'),
    'outline-last': number_in(rb'/First \d+ 0 R\s*/Last (\d+) 0 R\s*/Count -'),
    'outline-prev': number_in(rb'/Parent \d+ 0 R\s*/Prev (\d+) 0 R'),
    'outline-parent': number_in(rb'/Title \d+ 0 R\s*/A \d+ 0 R\s*/Parent (\d+) 0 R\s*>>'),
    'outline-next': number_in(rb'/Parent \d+ 0 R\s*/Next (\d+) 0 R'),
    'outline-action': lambda d: sub_once(d, rb'(obj\s*<<\s*/S /GoTo /D \(subsection\.1\.)1\)', rb'\g<1>2)'),
    'info-creator': lambda d: sub_once(d, rb'/Creator\(LaTeX with hyperref\)', b'/Creator(LaTeX with hyperreg)'),
    'info-title': lambda d: sub_once(d, rb'(\\000M\\000u\\000t\\000)a', rb'\g<1>b'),
    'xform-attr': lambda d: sub_once(d, rb'/Mutmark 111', b'/Mutmark 112'),
    'catalog-pagemode': lambda d: sub_once(d, rb'/PageMode/UseOutlines', b'/PageMode/UseOutlinez'),
    'xref-offset': lambda d: sub_once(d, rb'(xref\n0 \d+\n0000000000 65535 f \n)0000', rb'\g<1>0009'),
    'startxref': lambda d: sub_once(d, rb'startxref\n(\d)', lambda m: b'startxref\n' + str((int(m.group(1)) + 1) % 10).encode()),
    'stream-length': lambda d: sub_once(d, rb'<<\s*/Length (\d+)\s*>>', lambda m: m.group(0).replace(m.group(1), bump_last_digit(m.group(1)))),
    'trailer-size': number_in(rb'/Size (\d+)'),
    'identity-own-banner': lambda d: d.replace(b'TeX Live 2026/Arch Linux', b'rstex 0.1 own banner xxx'),     # not a bug: an engine says its own name, same length, so the offsets stay right
    'identity-own-banner-short': lambda d: own_banner(d),
    'identity-own-banner-pdf2': lambda d: own_banner(d),
    'identity-own-banner-dupkey': lambda d: own_banner(d),
}
LENGTH_MAY_CHANGE = {'identity-own-banner-short', 'identity-own-banner-pdf2', 'identity-own-banner-dupkey'}

LOG_EDITS = {
    'line-number': lambda d: sub_once(d, rb'(?m)^l\.5 ', b'l.6 '),
    'input-lineno': lambda d: sub_once(d, rb'a message on input line 8', b'a message on input line 9'),
    'log-message-word': lambda d: sub_once(d, rb'Undefined control sequence', b'Undefined control sequencf'),
    'log-only-warning-dropped': lambda d: sub_once(d, rb'(?m)^LaTeX Font Info: .*\n', b''),
}

MUT_FONT = r'\font\mutf=cmr10 '
PRELUDES = {
    'hyph-visible': r'\AtBeginDocument{\hyphenation{extraordinarily}}',     # a word the oracle breaks at a line end
    'hyph-hidden': r'\AtBeginDocument{\hyphenation{r-e-p-r-e-s-e-n-t-a-t-i-o-n}}',     # a word never at a line end here
    'hyph-righthyphenmin': r'\AtBeginDocument{\righthyphenmin=6 }',
    'hyph-word-list': r'\hyphenation{abbreviate}',       # one word wrong, in the list of extra/hyph
    'glue-1sp': MUT_FONT + r'\fontdimen2\mutf=\dimexpr\fontdimen2\mutf+1sp\relax ',
    'glue-1000sp': MUT_FONT + r'\fontdimen2\mutf=\dimexpr\fontdimen2\mutf+1000sp\relax ',
    'parskip-1sp': r'\AtBeginDocument{\advance\parskip 1sp }',
    'baseline-1sp': r'\AtBeginDocument{\advance\baselineskip 1sp }',
    'subset-extra-glyph': MUT_FONT + r'\pdfincludechars\mutf{Q}',
    'objcompress-off': r'\pdfobjcompresslevel=0 ',
    'compress-off': r'\pdfcompresslevel=0 ',
    'minor-version': r'\pdfminorversion=6 ',
    'macro-invisible': r'\AtBeginDocument{\gdef\labelitemiv{wrong}}',
    'register-invisible': r'\AtBeginDocument{\global\advance\@ne 0 \global\count200=7 }',
    'catcode-invisible': r'\AtBeginDocument{\catcode`\^^A=12 }',
    'page-trimbox': r'\pdfpageattr{/TrimBox [0 0 100 100]}',
    'page-userunit': r'\pdfpageattr{/UserUnit 2}',
    'pages-attr': r'\pdfpagesattr{/Mutated 1}',
    'catalog-attr': r'\pdfcatalog{/Mutated 1}',
    'names-extra': r'\pdfnames{/Mutated 1}',
    'trailer-extra': r'\pdftrailer{/Mutated 1}',
    'orphan-object': r'\immediate\pdfobj{(not referenced)}',
    'resources-extra': r'\pdfpageresources{/Mutated << >>}',
    'moddate': r'\pdfinfo{/ModDate (D:20000101000000Z)}',
    'pdfinfo-creator': r'\pdfinfo{/Creator (mutated)}',
    'identity-with-prelude': r'\relax ',
}

ENTRY = {
    # The twelve kinds asked for, then the ones found while reading the comparators.
    'kern-tfm': 'mut/default', 'kern-tfm-1sp': 'mut/default', 'kern-pdf': 'mut/plain',
    'ligature-tfm': 'mut/default',
    'hyph-visible': 'mut/default', 'hyph-hidden': 'mut/default', 'hyph-righthyphenmin': 'mut/default', 'hyph-word-list': 'mut/hyph',
    'glue-1sp': 'mut/default', 'glue-1000sp': 'mut/default', 'glue-pdf': 'mut/plain', 'parskip-1sp': 'mut/default', 'baseline-1sp': 'mut/default',
    'subset-extra-glyph': 'mut/default', 'subset-fontfile-byte': 'mut/plain', 'subset-charset': 'mut/plain', 'widths': 'mut/plain',
    'tounicode-entry': 'mut/plain', 'tounicode-dropped': 'mut/plain',
    'link-rect': 'mut/plain', 'link-dest': 'mut/plain', 'link-uri': 'mut/plain', 'link-border': 'mut/plain', 'dest-xyz': 'mut/plain', 'dest-page': 'mut/plain',
    'outline-count': 'mut/plain', 'outline-last': 'mut/plain', 'outline-prev': 'mut/plain', 'outline-parent': 'mut/plain', 'outline-next': 'mut/plain', 'outline-action': 'mut/plain',
    'objcompress-off': 'mut/default', 'compress-off': 'mut/default', 'minor-version': 'mut/default',
    'line-number': 'mut/errors', 'input-lineno': 'mut/errors', 'log-message-word': 'mut/errors', 'log-only-warning-dropped': 'mut/default',
    'macro-invisible': 'mut/default', 'register-invisible': 'mut/default', 'catcode-invisible': 'mut/default',
    'stale-aux-first-read': 'mut/default', 'stale-aux-ignored': 'mut/default',
    'format-macro-invisible': 'mut/state', 'format-macro-on-ordinary-doc': 'mut/default',
    'info-creator': 'mut/plain', 'info-title': 'mut/plain', 'xform-attr': 'mut/plain', 'catalog-pagemode': 'mut/plain',
    'xref-offset': 'mut/plain', 'startxref': 'mut/plain', 'stream-length': 'mut/plain', 'trailer-size': 'mut/plain',
    'page-trimbox': 'mut/default', 'page-userunit': 'mut/default', 'pages-attr': 'mut/default', 'catalog-attr': 'mut/default', 'names-extra': 'mut/default',
    'trailer-extra': 'mut/default', 'orphan-object': 'mut/default', 'resources-extra': 'mut/default', 'moddate': 'mut/default', 'pdfinfo-creator': 'mut/default',
    'identity': 'mut/default', 'identity-with-prelude': 'mut/default', 'identity-own-banner': 'mut/default',
    'identity-own-banner-short': 'mut/default', 'identity-own-banner-pdf2': 'mut/pdf2', 'identity-own-banner-dupkey': 'mut/dupkey',
}

# Which kind of bug the request named, for the table.
KIND = {
    'kern-tfm': '1 wrong kern', 'kern-tfm-1sp': '1 wrong kern', 'kern-pdf': '1 wrong kern',
    'ligature-tfm': '2 dropped ligature',
    'hyph-visible': '3 wrong hyphenation', 'hyph-hidden': '3 wrong hyphenation', 'hyph-righthyphenmin': '3 wrong hyphenation', 'hyph-word-list': '3 wrong hyphenation',
    'glue-1sp': '4 off by 1 sp glue', 'glue-1000sp': '4 off by 1 sp glue', 'glue-pdf': '4 off by 1 sp glue', 'parskip-1sp': '4 off by 1 sp glue', 'baseline-1sp': '4 off by 1 sp glue',
    'subset-extra-glyph': '5 wrong font subset', 'subset-fontfile-byte': '5 wrong font subset', 'subset-charset': '5 wrong font subset', 'widths': '5 wrong font subset',
    'tounicode-entry': '6 ToUnicode', 'tounicode-dropped': '6 ToUnicode',
    'link-rect': '7 link annotation', 'link-dest': '7 link annotation', 'link-uri': '7 link annotation', 'link-border': '7 link annotation', 'dest-xyz': '7 link annotation', 'dest-page': '7 link annotation',
    'outline-count': '8 outline', 'outline-last': '8 outline', 'outline-prev': '8 outline', 'outline-parent': '8 outline', 'outline-next': '8 outline', 'outline-action': '8 outline',
    'objcompress-off': '9 compression level', 'compress-off': '9 compression level', 'minor-version': '9 compression level',
    'line-number': '10 line number in message', 'input-lineno': '10 line number in message', 'log-message-word': '10 line number in message', 'log-only-warning-dropped': '10 line number in message',
    'macro-invisible': '11 invisible macro meaning', 'register-invisible': '11 invisible macro meaning', 'catcode-invisible': '11 invisible macro meaning',
    'stale-aux-first-read': '12 stale aux', 'stale-aux-ignored': '12 stale aux',
    'format-macro-invisible': '11 invisible macro meaning', 'format-macro-on-ordinary-doc': '11 invisible macro meaning',
    'identity': 'control', 'identity-with-prelude': 'control', 'identity-own-banner': 'control',
    'identity-own-banner-short': 'control', 'identity-own-banner-pdf2': 'control', 'identity-own-banner-dupkey': 'control',
}


def real_engine(args):
    for a in args:
        if a.startswith('-fmt=') and a != '-fmt=pdflatex':
            return shutil.which('pdftex')
    return shutil.which('pdflatex')


def main():
    bug = sys.argv[1]
    args = sys.argv[2:]
    out = next((a.split('=', 1)[1] for a in args if a.startswith('-output-directory=')), '.')
    last = args[-1]
    file_arg = last
    if last.startswith('\\'):
        m = re.search(r'\\input "([^"]*)"$', last)
        file_arg = m.group(1) if m else last
    job = next((a.split('=', 1)[1] for a in args if a.startswith('-jobname=')), os.path.splitext(os.path.basename(file_arg))[0])
    state = os.environ.get('MUTATE_STATE') or os.path.join(os.path.dirname(os.path.abspath(sys.argv[0])), 'state-unset')
    env = dict(os.environ)
    env.pop('MUTATE_STATE', None)
    bugdir = os.path.join(state, bug)
    os.makedirs(bugdir, exist_ok=True)

    def applied(note=''):
        with open(os.path.join(bugdir, 'applied'), 'a') as f:
            f.write(f'{job} {note}\n')

    shim = None
    if bug in PRELUDES:
        # Not on the first line: TeX echoes that line in the log, and with a backslash there \everyjob runs before the log opens.
        # Either would make a log comparison "detect" the injection instead of the bug.  So a copy of the main file with the
        # prelude glued to its first line is found first through TEXINPUTS, and its path is put back in the log and .fls afterwards.
        cwd = os.getcwd()
        rel = re.sub(r'^\.\/', '', file_arg)
        # Short on purpose: a path past TeX's line width would wrap in the log, and the wrap could not be put back.
        shim = f'/dev/shm/m{os.getpid()}'
        os.makedirs(os.path.join(shim, os.path.dirname(rel)), exist_ok=True)
        with open(os.path.join(cwd, rel), 'rb') as f:
            original = f.read()
        with open(os.path.join(shim, rel), 'wb') as f:
            f.write(PRELUDES[bug].encode() + original)
        env['TEXINPUTS'] = shim + ':' + env.get('TEXINPUTS', '')
        applied()
    if bug.startswith(('kern-tfm', 'ligature-tfm')):
        env['TFMFONTS'] = os.path.join(state, 'tfm', bug) + ':'
        applied()
    if bug.startswith('format-macro-'):
        # A format with one wrong macro that no ordinary document uses, built by matrix.py: an engine whose format build goes wrong this way.
        env['TEXFORMATS'] = os.path.join(state, 'format') + ':'
        args = ['-fmt=mutfmt' if a == '-fmt=pdflatex' else a for a in args]
        applied()

    aux = os.path.join(out, job + '.aux')
    if bug == 'stale-aux-first-read':
        # The engine remembers the .aux it saw the first time it ran in this output directory, as a cache keyed on the job would.
        # A last-pass comparison runs once per directory, so it can never see the bug; passes in the same directory can.
        key = hashlib.sha1(os.fsencode(os.path.abspath(out))).hexdigest()[:12]
        saved, absent = os.path.join(bugdir, key + '.aux'), os.path.join(bugdir, key + '.absent')
        applied()
        if os.path.exists(saved):
            shutil.copy2(saved, aux)
        elif os.path.exists(absent):
            if os.path.exists(aux):
                os.unlink(aux)
        elif os.path.exists(aux):
            shutil.copy2(aux, saved)
        else:
            open(absent, 'w').close()
    if bug == 'stale-aux-ignored':
        if os.path.exists(aux):
            os.unlink(aux)
        applied()

    p = subprocess.run([real_engine(args)] + args, env=env)
    rc = p.returncode

    try:
        pdf = os.path.join(out, job + '.pdf')
        log = os.path.join(out, job + '.log')
        if shim:
            pat = b'\n?'.join(re.escape(bytes([c])) for c in os.fsencode(shim + '/'))
            for path in (log, os.path.join(out, job + '.fls')):
                if os.path.exists(path):
                    with open(path, 'rb') as f:
                        data = f.read()
                    if path == log:
                        # TeX starts a new line before a file name too long to fit, so the long path also left a blank line.
                        data = re.sub(rb'(\n)\n(\()' + pat, rb'\1\2./', data, count=1)
                        data = re.sub(pat, b'./', data)
                    else:
                        data = re.sub(pat, b'', data)
                    with open(path, 'wb') as f:
                        f.write(data)
            shutil.rmtree(shim, ignore_errors=True)
        if bug.startswith(('kern-tfm', 'ligature-tfm')):
            fls = os.path.join(out, job + '.fls')
            if os.path.exists(fls):
                with open(fls, 'rb') as f:
                    data = f.read()
                with open(fls, 'wb') as f:
                    f.write(data.replace(os.fsencode(os.path.join(state, 'tfm', bug)), b'/usr/share/texmf-dist/fonts/tfm/public/cm'))
        if bug in PDF_EDITS and os.path.exists(pdf):
            with open(pdf, 'rb') as f:
                data = f.read()
            new = PDF_EDITS[bug](data)
            if len(new) != len(data) and bug not in LENGTH_MAY_CHANGE:
                raise NotApplied(f'edit changed the length by {len(new) - len(data)}')
            with open(pdf, 'wb') as f:
                f.write(new)
            applied()
        if bug in LOG_EDITS:
            with open(log, 'rb') as f:
                data = f.read()
            with open(log, 'wb') as f:
                f.write(LOG_EDITS[bug](data))
            applied()
    except NotApplied as ex:
        print(f'mutation {bug} did not apply: {ex}', file=sys.stderr)
        return 125
    return rc


if __name__ == '__main__':
    sys.exit(main())
