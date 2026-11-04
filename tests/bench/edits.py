#!/usr/bin/env -S python3 -I
r"""edits.py: the edits of the editing scenario in tests/bench/README.md, made by a rule from a document's bytes and from nothing chosen by hand.

`plan(files, main, kind, aux)` takes a document's own text files, in the order the oracle read them, and says which file changes and to what, or None where the kind does not apply, which a report counts.  Nothing
here prints a document's text, and a caller may print only the kind and whether it applied.  The text an edit adds is this file's own:

    word-start, word-middle, word-end   one word between two plain words, at 5%, 50% and 95% of the places in the document: moves no line, or few
    sentence                            a long sentence and a blank line at the blank line nearest the middle: adds lines, so pages may move
    cite                                a \cite, at that same place, of the first key by sorting that the .bib holds and the .aux does not cite; else of the first \bibitem
    preamble                            an unused \newcommand just before \begin{document}: what a checkpoint cannot reuse
    noop                                the same bytes, saved again: what a build with nothing to do costs

A place is in the body of the main file or in any own file the body reads, not on a comment line.  A word-gap is three lower-case words in a row, so that no macro argument, math or verbatim of any note is
split in practice; a pass that fails on an edit all the same has that edit taken back and counted, as tests/until_stable/session.py does.
"""
import re

KINDS = ('word-start', 'word-middle', 'word-end', 'sentence', 'cite', 'preamble', 'noop')
WORD = b' neutral'
SENTENCE = (b'Here are a few neutral words that an edit has put in, and they go on for long enough to take more than one line of the page, so that the paragraph below them moves down '
            b'and the page breaks after them may well move too.\n\n')
PREAMBLE = b'\\newcommand{\\rstexbenchunused}{}\n'
WORD_GAP = re.compile(rb'[a-z] (?=[a-z]+ [a-z]+ [a-z])')
BLANK = re.compile(rb'\n[ \t]*\n')
BEGIN = b'\\begin{document}'
END = b'\\end{document}'
BIB_KEY = re.compile(rb'^[ \t]*@(?!comment|string|preamble)[A-Za-z]+[ \t]*[{(][ \t]*([^,\s]+)[ \t]*,', re.M | re.I)
BIBITEM = re.compile(rb'\\bibitem(?:\[[^\]]*\])?\{([^}\s]+)\}')
CITED = re.compile(rb'\\citation\{([^}]*)\}')
MIN_GAPS = 20


def body_range(text, is_main):
    """Where the body is in a file: between the markers of the main file, the whole of another."""
    if not is_main:
        return 0, len(text)
    begin = text.find(BEGIN)
    if begin < 0:
        return 0, len(text)
    end = text.find(END, begin)
    return begin + len(BEGIN), (len(text) if end < 0 else end)


def not_comment(text, pos):
    start = text.rfind(b'\n', 0, pos) + 1
    return not text[start:pos].lstrip().startswith(b'%')


def places(text, is_main, pattern, offset=0):
    a, b = body_range(text, is_main)
    return [m.start() + 1 + offset for m in pattern.finditer(text, a, b) if not_comment(text, m.start())]


def word_places(files, main):
    """[(file, position)] over the document in reading order."""
    res = []
    for name, text in files:
        res += [(name, p) for p in places(text, name == main, WORD_GAP)]
    return res


def blank_places(files, main):
    res = []
    for name, text in files:
        res += [(name, p) for p in places(text, name == main, BLANK)]
    return res


def insert(files, name, pos, what):
    text = dict(files)[name]
    return name, text[:pos] + what + text[pos:]


def cite_key(files, aux, bib):
    cited = set()
    for m in CITED.finditer(aux or b''):
        cited.update(k.strip() for k in m.group(1).split(b','))
    keys = sorted({k for text in (bib or []) for k in BIB_KEY.findall(text)} - cited)
    if keys:
        return keys[0]
    items = sorted({k for _, text in files for k in BIBITEM.findall(text)})
    return items[0] if items else None


def plan(files, main, kind, aux=None, bib=None):
    """(file name, its new bytes), or None.  `files` is [(name, bytes)] in reading order, `main` the name of the main file, `aux` the bytes of the last .aux, `bib` the bytes of its .bib files."""
    d = dict(files)
    if kind == 'noop':
        return main, d[main]
    if kind == 'preamble':
        i = d[main].find(BEGIN)
        return None if i < 0 else insert(files, main, i, PREAMBLE)
    if kind in ('word-start', 'word-middle', 'word-end'):
        where = word_places(files, main)
        if len(where) < MIN_GAPS:
            return None
        share = {'word-start': 5, 'word-middle': 50, 'word-end': 95}[kind]
        name, pos = where[len(where) * share // 100]
        return insert(files, name, pos, WORD)
    if kind in ('sentence', 'cite'):
        where = blank_places(files, main)
        if not where:
            return None
        name, pos = where[len(where) // 2]
        if kind == 'sentence':
            return insert(files, name, pos, SENTENCE)
        key = cite_key(files, aux, bib)
        return None if key is None else insert(files, name, pos, b'\\cite{' + key + b'}\n\n')
    raise ValueError(kind)


def applicable(files, main, aux=None, bib=None):
    """The kinds that apply to a document."""
    return [k for k in KINDS if plan(files, main, k, aux, bib) is not None]
