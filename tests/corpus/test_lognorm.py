#!/usr/bin/env -S python3 -I
"""test_lognorm.py: the fingerprint of lognorm.py hides what the normal form hides at the two ends of a log, the engine's banner first of all, and nothing else, whatever the log's length.

A compare takes two equal fingerprints for an equal log and asks for the oracle's log only when they differ, so a difference here costs a run of the oracle for each entry.  The last line says PASS or FAIL.
"""
import os
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lognorm  # noqa: E402

BANNER = {'oracle': b'This is pdfTeX, Version 3.141592653-2.6-1.40.29 (TeX Live 2026/Arch Linux) (preloaded format=pdflatex 2026.1.1)  1 JAN 2026 00:00\n',
          'rstex': b'This is pdfTeX, Version 3.141592653-2.6-1.40.29 (rstex) (preloaded format=pdflatex)  2 FEB 2026 12:34\n'}


def log(banner, lines, end=b'Output written on a.pdf (1 page, 1234 bytes).\nTranscript written on a.log.\n'):
    return BANNER[banner] + b'entering extended mode\n' + b''.join(b'line %d of the middle of it\n' % k for k in range(lines)) + end


def fingerprint(data):
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, 'a.log')
        with open(path, 'wb') as f:
            f.write(data)
        return lognorm.fingerprint(path, d)


def main():
    failed = 0

    def check(what, cond):
        nonlocal failed
        print(('ok ' if cond else 'FAILED ') + what)
        failed += not cond
    # Under the 4 KiB that the head takes, in the head and the tail, and in a long log with the middle hashed as it stands.
    for lines in (3, 150, 300, 20000, 40000):
        a = fingerprint(log('oracle', lines))
        check(f'{lines} lines: the banner is hidden', a == fingerprint(log('rstex', lines)))
        check(f'{lines} lines: the size of the PDF is hidden', a == fingerprint(log('oracle', lines, b'Output written on a.pdf (1 page, 99 bytes).\nTranscript written on a.log.\n')))
        check(f'{lines} lines: a line more is not', a != fingerprint(log('oracle', lines + 1)))
        changed = log('oracle', lines).replace(b'line 1 of', b'line one of')
        check(f'{lines} lines: another word is not', a != fingerprint(changed))
    # Only as the first line.
    second = log('oracle', 300).replace(b'entering extended mode\n', BANNER['oracle'])
    other = log('oracle', 300).replace(b'entering extended mode\n', BANNER['rstex'])
    check('a banner further down is a line like any other', fingerprint(second) != fingerprint(other))
    print('FAIL' if failed else 'PASS')
    sys.exit(1 if failed else 0)


if __name__ == '__main__':
    main()
