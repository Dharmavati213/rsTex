#!/usr/bin/env -S python3 -I
"""privacy_guard.py: keep the user's private documents out of everything this repository publishes.

    python3 -I tools/privacy_guard.py --staged                  index, what a commit would add, plus the author
    python3 -I tools/privacy_guard.py --range A..B              every blob introduced in A..B, plus commit metadata
    python3 -I tools/privacy_guard.py --tree REF                every blob of a tree
    python3 -I tools/privacy_guard.py --worktree                tracked plus untracked non-ignored files
    python3 -I tools/privacy_guard.py --message-file FILE       one commit message

    --range takes anything git rev-list takes, for example 'SHA --not --remotes=origin'.
    --ref-name NAME     also apply rule f to a branch or tag name (repeatable)
    --report-limit N    findings shown per rule and path, default 5, 0 for all.  The cap is there so that a
                        whole copied document does not get printed back 40 characters at a time.
    --sources-optional  run without private sources (rules f, g and h only).  Hooks never pass this.

    Hooks use --hook with --staged or --message-file, --pre-push REMOTE URL, or --pre-receive; see install-hooks.sh
    and install-gate.sh.  With --hook the private directories come from this clone's git config, recorded at
    install time: RSTEX_PRIVATE_DIRS and HOME are ignored, and a missing directory or a large drop in what the sources
    hold stops the commit.  Without --hook they come from RSTEX_PRIVATE_DIRS (separated by ':'), default the thesis
    directory and the private store, and a missing directory is an error unless --sources-optional is given.

Sources are the text files of those directories (.git skipped) and every version of them in a repository's history.
Nothing derived from them is written anywhere: every set below lives in this process and dies with it.  This file
holds no private content.

Rules.  A blob is scanned as a whole, so a match anywhere in a modified file counts.  It is also read as what an
escaped string, a per-line prefix, markup or an encoded run holds, and a few cheap scramblings are undone.
    a  a non-boilerplate private line, whitespace-insensitive.  Short lines must match a whole line, long ones
       (>= LONG_LINE_CHARS) match anywhere, so they are also found inside a string or a reflowed paragraph.
       a/cluster: SOFT_CLUSTER or more distinct short private lines in one blob.
       a/error-line: the source line TeX quotes in an error message.
    b  WORD_RUN or more private prose words in order, across line breaks, comments and whitespace, or SHORT_RUN words
       when GRAM_MIN_CONTENT of them are more than stock phrasing.
       b/near: a window of NEAR_WINDOW words of which NEAR_SHARE or more are private three-word runs: light rewording.
    c  a tikzcd or equation body, whitespace-insensitive, or a tikzcd whose structure equals a private one.
       c/fuzzy: a tikzcd holding most of the cell and label strings of a private one.
    d  a distinctive title, heading, label, bib key, tikzcd alias, user macro, private file name or a name from the
       title page or the store's identity.terms.
    e  a blob with the SHA-256 of a private file; e/text, such a digest written out in text.
    f  a path or ref name containing the thesis words or '.local.', and thesis paths mentioned in text.  A local
       manifest's bare name in text is fine (the corpus runner has to name its own); the file itself is what blocks.
    g  any binary, SVG or oversized file not listed with path and SHA-256 (see read_allowed), and encoded data in text:
       long base64-like, hex-like or high-entropy runs, and any shorter run or table that decodes to text, a known
       file format, compressed data or more than OPAQUE_BYTES of anything.
    h  engine output whatever it says, because matching cannot recognise a few glyphs: PDF text operators, box logs,
       and runner records of a private entry.  The allowlist of rule g applies.
Rules f, g and h need no private sources.

Report lines: rule, path:line, then at most 40 characters of the scanned text at the match.
Exit status: 0 clean, 3 match, anything else means nothing was checked; hooks stop on all but 0.
"""
import argparse
import base64
import binascii
import bisect
import bz2
import fnmatch
import hashlib
import html
import lzma
import math
import multiprocessing
import os
import re
import subprocess
import sys
import unicodedata
import zlib

# Written in pieces because rule f has no exemption for this file.
PRIVATE_DIRNAME = "Master" "arbeit"
DEFAULT_PRIVATE_DIRS = ("~/" + PRIVATE_DIRNAME, "~/.local/share/rstex-private")
ALLOWED_BINARIES = "tools/privacy-allowed-binaries.txt"
STORE_DIR = "rstex-guard"                       # inside the git directory, so never part of a tree
APPROVED_BINARIES = "approved-binaries.txt"     # in STORE_DIR
TERMS_FILE = "identity.terms"                   # in a private directory: names the sources do not spell out
TEXT_EXTENSIONS = (".tex", ".bib", ".bbl", ".aux", ".toc", ".out", ".sty")
PLAIN_EXTENSIONS = (".rhai", ".record")         # private, but not TeX: lines and words only

EXIT_FINDINGS = 3
EXIT_ERROR = 2

PREFIX_CHARS = 40
LINE_MIN_CHARS = 24       # rule a: length of a line with all whitespace removed
LINE_MIN_LETTERS = 6      # rule a: letters left after structural commands and control-sequence names are gone
LINE_MIN_WORDS = 5        # rule a: a line of plain prose, with GRAM_MIN_CONTENT words that are not stock phrasing
LONG_LINE_CHARS = 48      # rule a: from here on a line also matches as a substring
SOFT_MIN_CHARS = 14       # rule a/cluster: a short line counts from this length ...
SOFT_MIN_LETTERS = 7      # ... with this many letters, as for LINE_MIN_LETTERS ...
SOFT_MIN_WORDS = 3        # ... or this many words in a line of plain prose
SOFT_CLUSTER = 3          # rule a/cluster: distinct short lines; one or two also occur in other people's documents
ERROR_MAX_FRAGMENTS = 400  # rule a/error-line: per blob; a log with more than this is rule h's business
WORD_RUN = 8              # rule b
SHORT_RUN = 6             # rule b, for a run with GRAM_MIN_CONTENT words outside GRAM_STOP ...
GRAM_MIN_CONTENT = 2      # ... since 'let be a morphism of schemes' is in every paper of the field
PLAIN_WORD_RUN = 10       # rule b for scripts and .bbl: they name paths, packages and publishers that others name too
NEAR_RUN = 3              # rule b/near
NEAR_WINDOW = 30          # rule b/near
NEAR_SHARE = 0.5          # rule b/near: ordinary text stays below 0.25, other people's mathematics below 0.75 at worst
DIAGRAM_MIN_BODY = 64     # rule c: tikzcd body, whitespace removed; a bare square is shorter
EQUATION_MIN_BODY = 40    # rule c: equation-like body, whitespace removed
FP_MIN_ARROWS = 4         # rule c: structural fingerprints apply from here
FP_MIN_TOKENS = 16        # rule c: cell and label tokens for the relettered form: twice a labelled square of letters
FP_MIN_SHAPE_SCORE = 8    # rule c: complexity needed for the shape-only form (a plain square scores 4)
FUZZY_MIN_CHARS = 3       # rule c/fuzzy: a cell or label string, whitespace removed
FUZZY_MIN_STRINGS = 3     # rule c/fuzzy: a private diagram with fewer has nothing to recognise it by
FUZZY_SHARE = 0.6         # rule c/fuzzy: share of the private strings that must be present ...
FUZZY_MIN_MATCHED = 16    # ... with this many characters among them, not counting a lone \alpha: those are in every paper
FUZZY_PART_SHARE = 0.75   # rule c/fuzzy: the same for the cells alone or the labels alone ...
FUZZY_PART_MATCHED = 12   # ... where more of them must agree, and for strings with their letters masked
TEMPLATE_MIN_CHARS = 6    # rule c/fuzzy: a string this long says something with its letters masked
TITLE_MIN_WORDS = 4       # rule d: non-generic words in a title; three-word headings are stock terms of the field ...
TITLE_SHORT_WORDS = 3     # ... but three of them in a title of four or more words are not
TITLE_MAX_TERM_WORDS = 8  # rule d: a longer title-page line is a sentence, for rule b
NAME_MIN_CHARS = 5        # rule d: label, key or alias stem
MACRO_MIN_CHARS = 7       # rule d: user macro name
HEX_RUN = 129             # rule g: one more than SHA-512
HEX_ANY = 32              # rule g: a shorter hex run is looked at only if it decodes to text
BASE64_RUN = 120          # rule g
BASE64_ANY = 24           # rule g: a shorter base64 run is looked at only if it decodes to text
WRAPPED_LINE = 16         # rule g: a line of an encoded block
ENCODED_BITS = 4.5        # rule g: entropy per character of a base64-like run; true base64 is close to 6
TABLE_MIN_ITEMS = 16      # rule g: separated numbers
TEXT_MIN_BYTES = 16       # rule g: decoded data shorter than this is not looked at as text
TEXT_ALWAYS_BYTES = 64    # rule g: decoded text from here on blocks by itself; shorter text only if a content rule says so,
                          # because a test vector such as the hex of 'The quick brown fox' is what such a run usually is
OPAQUE_BYTES = 16 << 10   # rule g: decoded data of no recognisable kind is allowed up to here (a font table) ...
RANDOM_BYTES = 4 << 10    # ... and from here on must not look compressed or encrypted ...
RANDOM_BITS = 7.5         # ... which means this many bits per byte; a [u8; 256] permutation stays below the size
DENSE_RUN = 400           # rule g: non-whitespace run ...
DENSE_BITS = 5.0          # rule g: ... with at least this Shannon entropy per character
MAX_SCAN_BYTES = 64 << 20  # rule g: larger blobs are not scanned, so they must be listed
TJ_MIN_LETTERS = 24       # rule h: letters in one PDF text operator; '[(Hello)-250(world)]TJ' in a unit test stays below
TJ_TOTAL_LETTERS = 120    # rule h: letters in all of a blob's operators; a set of short test streams stays below
TJ_MIN_KERNS = 8          # rule h: strings separated by a number
BOX_MIN_LINES = 40        # rule h: box-log lines; a record of a few lines holds a few glyphs
VIEW_MAX_BYTES = 4 << 20  # alternate readings are skipped above this; a blob this size is rule g's concern anyway
SCRAMBLE_MAX_BYTES = 256 << 10
PARALLEL_MIN_BYTES = 8 << 10
PARALLEL_JOBS = 8


class GuardError(Exception):
    """Anything that stops the guard from giving a real answer: hooks stop."""


# ------------------------------------------------------------------------------------------------ TeX structure

# Commands whose whole invocation carries no document text.  Sectioning, \label and \bibitem are here because
# rule d looks at them with its own idea of what is distinctive.
STRUCTURAL = frozenset("""
    documentclass usepackage RequirePackage LoadClass ProvidesPackage ProvidesClass PassOptionsToPackage
    usetikzlibrary tikzset tikzcdset pgfplotsset pgfkeys newtheorem theoremstyle numberwithin counterwithin
    newcommand renewcommand providecommand DeclareMathOperator DeclareRobustCommand DeclarePairedDelimiter
    def gdef edef xdef let newcount newdimen newskip newbox newtoks newif newlength setlength addtolength
    setcounter addtocounter stepcounter hypersetup geometry KOMAoptions setkomafont addtokomafont captionsetup
    bibliographystyle bibliography addbibresource printbibliography input include includeonly includegraphics
    graphicspath DeclareGraphicsExtensions makeatletter makeatother pagestyle thispagestyle pagenumbering
    begin end item centering raggedright raggedleft noindent indent par newpage clearpage cleardoublepage
    pagebreak linebreak nopagebreak newline bigskip medskip smallskip vspace hspace vfill hfill quad qquad
    maketitle tableofcontents listoffigures listoftables appendix frontmatter mainmatter backmatter qed qedhere
    title subtitle date part chapter section subsection subsubsection paragraph subparagraph
    label ref eqref pageref autoref cref Cref nameref cite citep citet nocite bibitem
    arrow ar toprule midrule bottomrule hline cline
    relax expandafter ifx else fi csname endcsname providecommand selectlanguage
    """.split())

# Commands whose arguments are never prose; everything else keeps its arguments as text.
NON_PROSE = STRUCTURAL - frozenset("""
    title subtitle part chapter section subsection subsubsection paragraph subparagraph item
    centering raggedright raggedleft noindent indent par quad qquad
    """.split())

# These take a control sequence and a parameter text before the replacement text.
DEF_LIKE = frozenset("""newcommand renewcommand providecommand DeclareMathOperator DeclareRobustCommand
    DeclarePairedDelimiter def gdef edef xdef let""".split())

_MATH_ENVS = (r"tikzcd|cd|CD|tikzpicture|equation\*?|align\*?|alignat\*?|flalign\*?|gather\*?|multline\*?"
              r"|eqnarray\*?|displaymath|math|array|matrix|[pbvBV]matrix|cases")
_MATH_ENV = re.compile(r"\\begin\{(" + _MATH_ENVS + r")\}")
_INLINE = re.compile(r"(?<![\\$])\$(?:[^$\\]|\\.){1,1500}?\$", re.S)
_BODY_ENV = re.compile(r"\\begin\{(tikzcd|cd|CD|equation\*?|align\*?|alignat\*?|flalign\*?|gather\*?|multline\*?"
                       r"|eqnarray\*?|displaymath)\}")
_TIKZCD = re.compile(r"\\begin\{(tikzcd)\}")
_TITLEPAGE = re.compile(r"\\begin\{(titlepage)\}")

_CS = re.compile(r"\\(?:([A-Za-z@]+)\*?|(.))", re.S)
_ACCENT_BRACED = re.compile(r"""\\(["'`^~=.])\{(\w)\}""")
_ACCENT_MARKS = frozenset("\"'`^~=.-")
_COMMENT = re.compile(r"(?<!\\)((?:\\\\)*)%([^\n]*)")
_COMMENT_MARK = re.compile(r"(?<!\\)((?:\\\\)*)%")
_LEADING_MARK = re.compile(r"^\s*(?:%+|//+|#+|>+|\*+|;+|--+)")
_LEADING_CHARS = frozenset("%/#>*;-")
_WORD = re.compile(r"[^\W\d_](?:[^\W\d_]|\x00)*")
_TEX_MARK = re.compile(r"[\\${}_^&]")
_OPTION_LINE = re.compile(r"^[A-Za-z@][\w@ -]*=\s*\{?[^\s{}]*\}?\s*,?\s*(?:%.*)?$")
_BIB_ENTRY = re.compile(r"^\s*@\w+\s*\{[^,\s]*,?\s*$")
_BIB_METADATA = re.compile(r"^\s*(?:publisher|address|location|year|date|month|volume|number|pages|issn|isbn|doi|url"
                           r"|urldate|journal|journaltitle|booktitle|series|edition|language|langid|eprint"
                           r"|eprinttype|archiveprefix|primaryclass|mrclass|mrnumber|zbl|keywords|timestamp|file"
                           r"|biburl|bibsource)\s*=", re.I)


def _gram_content(words):
    return sum(1 for w in words if len(w) >= 4 and w not in GRAM_STOP)


_PATH_LIKE = re.compile(r"\S*/\S*")


def _blank(s):
    """Same length, same line breaks, nothing else: offsets into the blanked text stay valid in the original."""
    return re.sub(r"[^\n]", " ", s)


def _blank_match(m):
    return _blank(m.group())


def _pairs(text, opener, closer):
    """(start of opener, start of closer) of each unescaped pair.  str.find keeps this linear on files that are not TeX."""
    i = 0
    while True:
        a = text.find(opener, i)
        while a > 0 and text[a - 1] == "\\":
            a = text.find(opener, a + 1)
        if a < 0:
            return
        b = text.find(closer, a + len(opener))
        while b > 0 and text[b - 1] == "\\":
            b = text.find(closer, b + 1)
        if b < 0:
            return
        yield a, b
        i = b + len(closer)


def _blank_between(text, opener, closer):
    """Blank from each unescaped opener to the next closer."""
    parts, i = [], 0
    for a, b in _pairs(text, opener, closer):
        b += len(closer)
        parts.append(text[i:a])
        parts.append(_blank(text[a:b]))
        i = b
    parts.append(text[i:])
    return "".join(parts)


def iter_envs(text, begin):
    """(start, end, name, inner start, inner end) of each environment opened by the regex begin.  Linear: an
    environment that never closes is dropped once, instead of being searched for from each of its openers."""
    pos, dead = 0, set()
    for m in begin.finditer(text):
        name = m.group(1)
        if m.start() < pos or name in dead:
            continue
        closer = "\\end{%s}" % name
        j = text.find(closer, m.end())
        if j < 0:
            dead.add(name)
            continue
        pos = j + len(closer)
        yield m.start(), pos, name, m.end(), j


def _skip_group(s, i, opener, closer):
    """Index just past the balanced group opening at s[i]; the end of s when it never closes."""
    depth, n = 0, len(s)
    while i < n:
        c = s[i]
        if c == "\\":
            i += 2
            continue
        if c == opener:
            depth += 1
        elif c == closer:
            depth -= 1
            if depth == 0:
                return i + 1
        i += 1
    return n


def _skip_space(s, i):
    n = len(s)
    while i < n and s[i] in " \t\r\n":
        i += 1
    return i


def _skip_args(s, i, def_like=False):
    """Index past the arguments of a command that ends at s[i]."""
    while True:
        j = _skip_space(s, i)
        if j >= len(s):
            return i
        c = s[j]
        if c == "{":
            i = _skip_group(s, j, "{", "}")
        elif c == "[":
            i = _skip_group(s, j, "[", "]")
        elif def_like and c == "\\":
            m = _CS.match(s, j)
            i = m.end() if m else j + 1
        elif def_like and c == "#":
            i = j + 2
        elif def_like and c == "=":
            i = j + 1
        else:
            return i


def strip_comments(text):
    """Comment text blanked."""
    return _COMMENT.sub(lambda m: m.group(1) + " " * (len(m.group(2)) + 1), text)


def open_comments(text):
    """Comment text kept and only the marker blanked, so that a commented-out copy still reads as text."""
    return _COMMENT_MARK.sub(lambda m: m.group(1) + " ", text)


def tex_prose(text):
    """Blank everything that is not document prose: math, diagrams, control sequences, option lists and the
    arguments of commands that do not take text.  Offsets are preserved."""
    parts, i = [], 0
    for a, b, _, _, _ in iter_envs(text, _MATH_ENV):
        parts.append(text[i:a])
        parts.append(_blank(text[a:b]))
        i = b
    parts.append(text[i:])
    text = "".join(parts)
    for opener, closer in (("\\[", "\\]"), ("\\(", "\\)"), ("$$", "$$")):
        text = _blank_between(text, opener, closer)
    text = _INLINE.sub(_blank_match, text)
    text = _ACCENT_BRACED.sub(lambda m: "\0\0\0" + m.group(2) + "\0", text)
    parts, i = [], 0
    while True:
        m = _CS.search(text, i)
        if not m:
            parts.append(text[i:])
            return "".join(parts)
        parts.append(text[i:m.start()])
        name = m.group(1)
        if name is None:
            parts.append("\0\0" if m.group(2) in _ACCENT_MARKS else _blank(m.group()))
            i = m.end()
            continue
        end = m.end()
        if name in NON_PROSE:
            end = _skip_args(text, end, name in DEF_LIKE)
        else:
            j = _skip_space(text, end)
            if j < len(text) and text[j] == "[":
                k = _skip_group(text, j, "[", "]")
                if k - j < 400:
                    end = k
        parts.append(_blank(text[m.start():end]))
        i = end


def raw_prose(text):
    """Control-sequence names blanked and nothing else; for files where $ and % do not mean what they mean in TeX."""
    def repl(m):
        if m.group(1) is None and m.group(2) in _ACCENT_MARKS:
            return "\0\0"
        return _blank(m.group())
    return _CS.sub(repl, text)


def words_of(blanked):
    """Prose words of a blanked text with their offsets.  One-letter tokens other than the article are out, on both
    sides of a comparison: they are mostly math, so '$X$' in the source and 'X' in a copy agree."""
    words, offsets = [], []
    for m in _WORD.finditer(blanked):
        w = m.group().replace("\0", "")
        if len(w) < 2 and w != "a":
            continue
        if not w.isascii():
            w = "".join(c for c in unicodedata.normalize("NFKD", w) if not unicodedata.combining(c))
        words.append(w.casefold())
        offsets.append(m.start())
    return words, offsets


def squeeze(s):
    return "".join(s.split())


def code_of(line):
    return _COMMENT.sub(lambda m: m.group(1), line) if "%" in line else line


def unmarked(line):
    """A line with a leading comment or quote marker gone and % dropped, comment text kept."""
    if line.lstrip()[:1] in _LEADING_CHARS:
        line = _LEADING_MARK.sub("", line, count=1)
    return _COMMENT_MARK.sub(lambda m: m.group(1), line) if "%" in line else line


def line_variants(line):
    return {squeeze(line), squeeze(code_of(line)), squeeze(unmarked(line))}


def is_boilerplate(line, ext, min_letters=LINE_MIN_LETTERS, min_words=LINE_MIN_WORDS, min_content=GRAM_MIN_CONTENT):
    """True when a line carries no document text.  The test is structural: remove commands in STRUCTURAL with their
    arguments and the names of all other control sequences, and count the letters that remain."""
    s = line.strip()
    if ext == ".bib" and (_BIB_ENTRY.match(s) or _BIB_METADATA.match(s)):
        return True
    if _OPTION_LINE.match(s):
        return True
    if not _TEX_MARK.search(s):
        words = words_of(s)[0]
        if len(words) < min_words or _gram_content(words) < min_content:
            return True     # plain prose this short is rule b's call, and rule b says it is too short to mean anything
    s = _LEADING_MARK.sub("", s, count=1)
    letters, i = 0, 0
    while True:
        m = _CS.search(s, i)
        if not m:
            letters += sum(c.isalpha() for c in s[i:])
            return letters < min_letters
        letters += sum(c.isalpha() for c in s[i:m.start()])
        name = m.group(1)
        i = _skip_args(s, m.end(), name in DEF_LIKE) if name in STRUCTURAL else m.end()


# ------------------------------------------------------------------------------------------------ diagrams

_ARROW = re.compile(r"\\(?:arrow|ar)(?![A-Za-z@])")
_CELL_TOKEN = re.compile(r"\\[A-Za-z@]+|\\.|[^\W\d_]|\d+|\S", re.S)
_CELL_NODE_OPTS = re.compile(r"\|\[.{0,400}?\]\|", re.S)


def _split_grid(body):
    rows, cells, cur, depth, i, n = [], [], [], 0, 0, len(body)
    while i < n:
        c = body[i]
        if c == "\\":
            if body[i + 1:i + 2] == "\\" and depth == 0:
                cells.append("".join(cur))
                rows.append(cells)
                cells, cur = [], []
                i = _skip_space(body, i + 2)
                if body[i:i + 1] == "[":
                    i = _skip_group(body, i, "[", "]")
                continue
            cur.append(body[i:i + 2])
            i += 2
            continue
        if c in "{[":
            depth += 1
        elif c in "}]":
            depth = max(0, depth - 1)
        elif c == "&" and depth == 0:
            cells.append("".join(cur))
            cur = []
            i += 1
            continue
        cur.append(c)
        i += 1
    cells.append("".join(cur))
    if any(x.strip() for x in cells):
        rows.append(cells)
    return rows


def _split_options(opts):
    items, cur, depth, quoted = [], [], 0, False
    for c in opts:
        if c == '"':
            quoted = not quoted
        elif not quoted:
            if c in "{[(":
                depth += 1
            elif c in "}])":
                depth -= 1
            elif c == "," and depth <= 0:
                items.append("".join(cur).strip())
                cur = []
                continue
        cur.append(c)
    items.append("".join(cur).strip())
    return [x for x in items if x]


def _parse_arrow(opts, braces):
    direction, styles, labels, swapped = "", [], [], False
    if braces:
        direction = braces[0].strip()
        labels.extend(braces[1:])
    for o in opts:
        for item in _split_options(o):
            if item.startswith('"'):
                end = item.find('"', 1)
                end = len(item) if end < 0 else end
                labels.append(item[1:end])
                tail = item[end + 1:]
                swapped = swapped or tail.lstrip().startswith("'") or "swap" in tail
            elif re.fullmatch(r"[rlud]+", item):
                direction = item
            elif item in ("'", "swap"):
                swapped = True
            else:
                styles.append(item.split("=", 1)[0].strip())
    return (direction, tuple(sorted(styles)), len(labels), swapped), labels


def _parse_cell(cell):
    text, arrows, labels, i = [], [], [], 0
    while True:
        m = _ARROW.search(cell, i)
        if not m:
            text.append(cell[i:])
            return "".join(text), arrows, labels
        text.append(cell[i:m.start()])
        j, opts, braces = m.end(), [], []
        while True:
            j = _skip_space(cell, j)
            if cell[j:j + 1] == "[":
                k = _skip_group(cell, j, "[", "]")
                opts.append(cell[j + 1:k - 1])
            elif cell[j:j + 1] == "{":
                k = _skip_group(cell, j, "{", "}")
                braces.append(cell[j + 1:k - 1])
            else:
                break
            j = k
        record, arrow_labels = _parse_arrow(opts, braces)
        arrows.append(record)
        labels.extend(arrow_labels)
        i = j


def diagram_keys(body):
    """Structural fingerprints of a tikzcd body as (shape, relettered), either possibly None.  shape keeps the grid,
    which cells are filled, and each arrow's position, direction, style keys, label count and label side.  relettered
    adds cell and label text with letters and control sequences renamed in order of first use, which survives a
    consistent rename but not a rewrite.  Only diagrams with enough structure to mean anything get a key: a plain
    commutative square has the same shape in every paper, so matching it would say nothing."""
    body = strip_comments(body)
    body = body.lstrip()
    if body.startswith("["):
        body = body[_skip_group(body, 0, "[", "]"):]
    rows = _split_grid(_CELL_NODE_OPTS.sub(" ", body))
    arrows, skeleton, rename, occupancy, tokens = [], [], {}, [], 0

    def tokenize(s):
        nonlocal tokens
        out = []
        for t in _CELL_TOKEN.findall(s):
            tokens += 1
            if t[0] == "\\" or t[0].isalpha():
                t = rename.setdefault(t, "#%d" % len(rename))
            out.append(t)
        return tuple(out)

    for r, cells in enumerate(rows):
        row_fill = []
        for c, cell in enumerate(cells):
            text, cell_arrows, labels = _parse_cell(cell)
            row_fill.append("x" if text.strip() else ".")
            skeleton.append(tokenize(text))
            skeleton.extend(tokenize(x) for x in labels)
            arrows.extend((r, c) + a for a in cell_arrows)
        occupancy.append("".join(row_fill))
    if len(arrows) < FP_MIN_ARROWS:
        return None, None
    arrows.sort()
    shape = (tuple(occupancy), tuple(arrows))
    relettered = (shape, tuple(skeleton)) if tokens >= FP_MIN_TOKENS else None
    score = len(arrows) + sum(1 for a in arrows if a[3]) + sum(1 for a in arrows if len(a[2]) != 1)
    score += max(0, len(rows) - 2) + max(0, max(len(x) for x in occupancy) - 2)
    return (shape if score >= FP_MIN_SHAPE_SCORE else None), relettered


_LONE_LETTER = re.compile(r"(?<![\\A-Za-z])[A-Za-z](?![A-Za-z])")
_LONE_MACRO = re.compile(r"\\[A-Za-z]+")


def diagram_strings(body):
    """(cell strings, label strings, both with letters masked) of a tikzcd body, whitespace removed: what a copy keeps
    when its options, arrow spelling or a few arrows differ, or its letters but nothing else."""
    body = strip_comments(body).lstrip()
    if body.startswith("["):
        body = body[_skip_group(body, 0, "[", "]"):]
    texts, labels = set(), set()
    for cells in _split_grid(_CELL_NODE_OPTS.sub(" ", body)):
        for cell in cells:
            text, _, cell_labels = _parse_cell(cell)
            text = squeeze(text)
            if len(text) >= FUZZY_MIN_CHARS:
                texts.add(text)
            labels.update(x for x in map(squeeze, cell_labels) if len(x) >= FUZZY_MIN_CHARS)
    masked = {_LONE_LETTER.sub("?", x) for x in texts | labels if len(x) >= TEMPLATE_MIN_CHARS}
    return texts, labels, masked


def _strings_share(private, candidate, min_matched):
    if len(private) < FUZZY_MIN_STRINGS:
        return 0.0
    common = private & candidate
    specific = sum(len(x) for x in common if not _LONE_MACRO.fullmatch(x))
    return len(common) / len(private) if specific >= min_matched else 0.0


def equation_bodies(text):
    """(squeezed body, is_diagram) for every tikzcd and equation-like environment of a comment-free text."""
    for _, _, name, a, b in iter_envs(text, _BODY_ENV):
        body = text[a:b].lstrip()
        if body.startswith("["):
            body = body[_skip_group(body, 0, "[", "]"):]
        yield squeeze(body), name.lower() in ("tikzcd", "cd")
    for opener, closer in (("\\[", "\\]"), ("$$", "$$")):
        for a, b in _pairs(text, opener, closer):
            yield squeeze(text[a + len(opener):b]), False


# ------------------------------------------------------------------------------------------------ rule d extraction

GENERIC_WORDS = frozenset("""
    the an of and or in on for to with from by at as is are be over under via some its their this that these those
    our we not no all any each new one two three four
    introduction preliminaries preliminary notation notations convention conventions background motivation overview
    summary conclusion conclusions outlook acknowledgements acknowledgments references bibliography appendix abstract
    contents result results main theorem theorems proof proofs lemma lemmas definition definitions example examples
    remark remarks proposition propositions corollary corollaries section chapter part first second third general
    case cases basic facts recollection recollections further discussion application applications property properties
    structure structures problem problems construction constructions setup statement statements
    und der die das des dem den von mit fuer einleitung zusammenfassung grundlagen literatur literaturverzeichnis
    anhang ergebnisse ausblick
    """.split())

GENERIC_STEMS = GENERIC_WORDS | frozenset("""diagram square figure table equation intro prelim concl thm prop
    defn page label note notes""".split())

# Rule b: words that do not count towards GRAM_MIN_CONTENT.  Words of under four letters do not count either.  A run
# needs words that say what a text is about, and these are said in every text, mathematical or not.
GRAM_STOP = frozenset("""
    about above after again also always another because been before being below between both cannot could does done
    each either every first following from further given have having hence here however into itself just like made
    make many more most much must need only other over same shall should show shown since some such than that their
    them then there therefore these they this those thus under until very were what when where whereas which while
    will with would your
    able above according actually along already although among apply applied arbitrary assume assumed assuming below
    call called case cases certain change changed clear close common complete condition conditions consider considered
    contain contains containing correct corresponding define defined defines definition denote denoted denotes describe
    described different direct directly done easy element elements empty equal equals equivalent even example exactly
    exist exists fact false field fields find finally finite first follow follows form formula four function functions
    general given goes group groups hand hold holds hypothesis idea identity image implies imply important indeed
    induce induced induces instead inverse just know known last least left length less line lines list look main make
    means might more name natural necessary needed next notation note notice number numbers object objects obtain
    obtained once open order part particular point points possible precisely previous proof property proposition prove
    proved recall remark require required result results right ring rings rule same section sequence side similar
    simple since small so some space spaces state statement step such sufficient suppose take taken term terms text
    that theorem thing three true two unique usual value values want well whole work written zero
    adjoint affine algebra algebraic base bundle bundles canonical canonically category categories change cohomology
    commutative commutes compact complex connected curve curves diagram diagrams dimension exact factor flat full
    functor functors homomorphism ideal injective isomorphic isomorphism isomorphisms kernel lemma line local locally
    manifold matrix module modules morphism morphisms natural pair projective quotient rank rational scheme schemes
    sheaf sheaves smooth stable subcategory subset subspace surjective topological topology variety vector
    """.split())

# Words on a title page that say what a name is, not whose it is.
TITLEPAGE_LABELS = frozenset("""prof professor herr frau rer nat phil habil ing msc bsc betreuer betreuerin gutachter
    gutachterin erstgutachter zweitgutachter referent referentin korreferent korreferentin supervisor advisor author
    submitted vorgelegt eingereicht date datum""".split())

# ... and words every thesis's title page has: a line of nothing else names nobody.
TITLEPAGE_STOCK = GENERIC_WORDS | frozenset("""department faculty fakultaet fakultat school institute institut chair lehrstuhl
    university universitaet universitat college mathematics mathematik mathematical mathematisches computer science
    informatik physics physik thesis master masters bachelor doctoral diploma arbeit bachelorarbeit dissertation degree
    grade fulfilment fulfillment requirements""".split()) | frozenset(("master" "arbeit",))

_NAME_TOKEN = re.compile(r"[^\s{}\[\]()<>,;\"'`=|\\/$&%~^*#!?]+")
_MACRO_USE = re.compile(r"\\([A-Za-z@]+)")
_LABEL = re.compile(r"\\(?:label|newlabel)\s*\{([^{}\s]+)\}")
_CITE = re.compile(r"\\(?:cite[a-zA-Z]*|nocite|citation|bibcite|bibitem)\*?\s*(?:\[[^\]]*\]\s*)*\{([^{}]+)\}")
_BIB_KEY = re.compile(r"^[ \t]*@(\w+)\s*\{\s*([^,\s{}]+)\s*,", re.M)
_ALIAS = re.compile(r"\b(?:alias|name)\s*=\s*([A-Za-z][\w-]*)")
_HEADING = re.compile(r"\\(?:title|subtitle|part|chapter|section|subsection|subsubsection|paragraph|subparagraph)"
                      r"\*?\s*(?:\[([^\]]*)\])?\s*(?=\{)")
_BIB_TITLE = re.compile(r"^[ \t]*(?:title|subtitle|booktitle)\s*=\s*(?=[{\"])", re.M | re.I)
_TOC_LINE = re.compile(r"\\contentsline\s*\{[^{}]*\}\s*(?=\{)")
_BOOKMARK = re.compile(r"\\BOOKMARK\s*\[[^\]]*\]\s*\[[^\]]*\]\s*\{[^{}]*\}\s*(?=\{)")
_AUTHOR = re.compile(r"\\author\s*(?:\[[^\]]*\]\s*)?(?=\{)")
_NEW_MACRO = re.compile(r"\\(?:newcommand|providecommand|DeclareMathOperator|DeclareRobustCommand)\*?\s*\{?\s*\\([A-Za-z@]+)"
                        r"|\\[gex]?def\s*\\([A-Za-z@]+)")


def distinctive_name(name):
    """Is a label, bib key or alias specific enough that seeing it elsewhere means something?"""
    stem = name.rsplit(":", 1)[-1]
    if len(stem) < NAME_MIN_CHARS or stem.lower() in GENERIC_STEMS:
        return False
    if stem.isalpha() and stem.islower() and len(stem) < 8:
        return False
    return True


def distinctive_stem(name):
    """The part after a 'lem:' style prefix counts alone only when it could not be a word of ordinary text."""
    stem = name.rsplit(":", 1)[-1]
    return stem != name and distinctive_name(name) and (len(stem) >= 10 or re.search(r"[-_\d]", stem) is not None)


_GENERIC_FILE = re.compile(r"(?:page|fig|figure|image|img|pic|picture|scan|photo|test|out|output|tmp|temp|file|doc|main"
                           r"|draft|sheet|slide|table|plot|graph|chart|sample|example)[-_]?\d*", re.I)


def distinctive_file(name):
    """A name this could only be: page1.png and a camera's IMG_4749.jpg are not, whoever's disk they are on."""
    stem, ext = os.path.splitext(name)
    return (bool(ext) and len(stem) >= NAME_MIN_CHARS and re.search(r"[-_\d]", stem) is not None
            and not _GENERIC_FILE.fullmatch(stem))


def _brace_arg(text, i):
    if text[i:i + 1] not in ('{', '"'):
        return ""
    if text[i] == '"':
        end = text.find('"', i + 1)
        return text[i + 1:end] if end > 0 else ""
    return text[i + 1:_skip_group(text, i, "{", "}") - 1]


class Sources:
    """Everything known about the private documents, in memory only."""

    def __init__(self):
        self.available = False
        self.text_files = 0
        self.hashes = set()
        self.exact_lines = set()
        self.soft_lines = set()     # non-boilerplate lines too short for rule a alone
        self.line_text = ""         # all squeezed lines, for finding a fragment of one
        self._line_parts = []
        self.literals = {}          # first LITERAL_KEY characters -> [(literal, kind)]
        self.grams = set()
        self.short_grams = set()
        self.long_grams = set()     # runs of PLAIN_WORD_RUN from sources that are not documents
        self.near = set()
        self.titles = {}            # (first word, second word) -> [(word tuple, kind)]
        self.term_words = set()     # single-word identity terms
        self.names = {}             # token -> kind
        self.word_keys = set()      # bib keys made of words only, which match only where a key is expected
        self.macros = set()
        self.shapes = set()
        self.relettered = set()
        self.diagrams = []          # (cell strings, label strings, masked strings)

    LITERAL_KEY = 16

    def load(self, directories):
        missing = []
        for d in directories:
            root = os.path.expanduser(d)
            if os.path.isdir(root):
                self._walk(root)
                self.available = True
            else:
                missing.append(d)
        self.line_text = "\0".join(self._line_parts)
        self._line_parts = []
        return missing

    def _walk(self, root):
        def fail(err):
            raise GuardError("cannot read private directory: %s" % err)
        for dirpath, dirnames, filenames in os.walk(root, onerror=fail):
            if ".git" in dirnames:
                self._add_history(dirpath)
            dirnames[:] = [x for x in dirnames if x != ".git"]
            for name in filenames:
                path = os.path.join(dirpath, name)
                if not os.path.isfile(path):
                    continue
                self._add_file(path, name)

    def _add_history(self, repo):
        """Every version a private repository ever committed: a draft that was rewritten is as private as the text now."""
        env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
        listed = subprocess.run(("git", "-C", repo, "rev-list", "--all", "--objects"), env=env,
                                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        if listed.returncode:
            raise GuardError("cannot read the history of a private repository")
        wanted = {}
        for line in listed.stdout.decode("utf-8", "surrogateescape").splitlines():
            sha, _, path = line.partition(" ")
            if path:
                wanted.setdefault(sha, path)
        batch = subprocess.Popen(("git", "-C", repo, "cat-file", "--batch"), env=env, stdin=subprocess.PIPE,
                                 stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        try:
            for sha, path in wanted.items():
                batch.stdin.write(sha.encode() + b"\n")
                batch.stdin.flush()
                header = batch.stdout.readline().split()
                if len(header) != 3:
                    raise GuardError("cannot read the history of a private repository")
                data = batch.stdout.read(int(header[2]))
                batch.stdout.read(1)
                if header[1] == b"blob":
                    self._add_content(data, os.path.basename(path))
        finally:
            batch.stdin.close()
            batch.wait()

    def _add_file(self, path, name):
        try:
            with open(path, "rb") as f:
                ext = os.path.splitext(name)[1].lower()
                if ext in TEXT_EXTENSIONS or ext in PLAIN_EXTENSIONS or name == TERMS_FILE:
                    self._add_content(f.read(), name)
                    return
                digest = hashlib.sha256()
                while True:
                    chunk = f.read(1 << 20)
                    if not chunk:
                        break
                    digest.update(chunk)
        except OSError as e:
            raise GuardError("cannot read private file: %s" % e.strerror)
        if os.path.getsize(path) > 0:
            self.hashes.add(digest.hexdigest())
        self._add_file_name(name)

    def _add_content(self, data, name):
        ext = os.path.splitext(name)[1].lower()
        if name == TERMS_FILE:
            self._add_terms(data.decode("utf-8", "replace"))
            return
        digest = hashlib.sha256(data).hexdigest()
        seen = digest in self.hashes
        if data:
            self.hashes.add(digest)
        self._add_file_name(name)
        if ext in TEXT_EXTENSIONS or ext in PLAIN_EXTENSIONS:
            if not seen:
                self.text_files += 1
                text = data.decode("utf-8", "replace")
                if ext in PLAIN_EXTENSIONS:
                    self._add_plain(text)
                else:
                    self._add_text(ext, text)

    def _add_file_name(self, name):
        if distinctive_file(name):
            self.names[name] = "file"
            stem = os.path.splitext(name)[0]
            if re.search(r"[-_]", stem) and len(stem) >= 6:
                self.names[stem] = "file"

    def _add_line(self, ext, source):
        v = squeeze(source)
        if len(v) < SOFT_MIN_CHARS:
            return
        self._line_parts.append(v)
        if len(v) >= LINE_MIN_CHARS and not is_boilerplate(source, ext):
            self.exact_lines.add(v)
            if len(v) >= LONG_LINE_CHARS:
                self._add_literal(v, "a")
        elif not is_boilerplate(source, ext, SOFT_MIN_LETTERS, SOFT_MIN_WORDS, 0):
            self.soft_lines.add(v)

    def _add_plain(self, text):
        """A file that is private but not TeX.  Only whole sentences and long lines say anything here."""
        for line in text.splitlines():
            v = squeeze(line)
            if len(v) >= LONG_LINE_CHARS:
                self.exact_lines.add(v)
                self._add_literal(v, "a")
        self._add_long_grams(words_of(raw_prose(_PATH_LIKE.sub(" ", text)))[0])

    def _add_long_grams(self, words):
        for i in range(len(words) - PLAIN_WORD_RUN + 1):
            self.long_grams.add(tuple(words[i:i + PLAIN_WORD_RUN]))

    def _add_text(self, ext, text):
        if ext in (".tex", ".bbl", ".sty", ".bib"):
            for line in text.splitlines():
                for source in dict.fromkeys((line, code_of(line), unmarked(line))):
                    self._add_line(ext, source)
        code = strip_comments(text)
        if ext in (".tex", ".sty"):
            for stream in (code, open_comments(text)):
                self._add_grams(words_of(tex_prose(stream))[0])
        if ext == ".bbl":
            self._add_long_grams(words_of(tex_prose(code))[0])
        if ext in (".tex", ".sty"):
            for body, is_diagram in equation_bodies(code):
                if len(body) >= (DIAGRAM_MIN_BODY if is_diagram else EQUATION_MIN_BODY):
                    self._add_literal(body, "c/body")
            for _, _, _, a, b in iter_envs(code, _TIKZCD):
                self._add_diagram(code[a:b])
            for m in _NEW_MACRO.finditer(code):
                name = m.group(1) or m.group(2)
                if len(name) >= MACRO_MIN_CHARS:
                    self.macros.add(name)
            for m in _ALIAS.finditer(code):
                if distinctive_name(m.group(1)):
                    self.names.setdefault(m.group(1), "alias")
            self._add_title_page(code)
        self._add_names(ext, code)

    def _add_diagram(self, body):
        shape, relettered = diagram_keys(body)
        if shape:
            self.shapes.add(shape)
        if relettered:
            self.relettered.add(relettered)
        strings = diagram_strings(body)
        if strings not in self.diagrams:
            self.diagrams.append(strings)

    def _add_title_page(self, code):
        """Names on the title page.  Two words are enough for a person, so these match where a title would not."""
        bodies = [code[a:b] for _, _, _, a, b in iter_envs(code, _TITLEPAGE)]
        bodies.extend(_brace_arg(code, m.end()) for m in _AUTHOR.finditer(code))
        for body in bodies:
            for line in re.split(r"\n|\\\\", body):
                everything = tuple(words_of(tex_prose(line))[0])
                words = tuple(w for w in everything if w not in TITLEPAGE_LABELS)
                if 2 <= len(words) <= TITLE_MAX_TERM_WORDS and any(w not in TITLEPAGE_STOCK for w in words):
                    self._add_title_words(words, "d/identity")
                elif len(words) == 1 and len(everything) == 2 and words[0] not in TITLEPAGE_STOCK:
                    self._add_title_words(everything, "d/identity")     # 'Prof. Name'; the surname alone is too common

    def _add_terms(self, text):
        for line in text.splitlines():
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            words = tuple(words_of(line)[0])
            if len(words) == 1:
                self.term_words.add(words[0])
            elif words:
                self._add_title_words(words, "d/identity")

    def _add_literal(self, literal, kind):
        self.literals.setdefault(literal[:self.LITERAL_KEY], []).append((literal, kind))

    def _add_grams(self, words):
        grams = self.grams
        for i in range(len(words) - WORD_RUN + 1):
            grams.add(tuple(words[i:i + WORD_RUN]))
        for i in range(len(words) - SHORT_RUN + 1):
            gram = tuple(words[i:i + SHORT_RUN])
            if _gram_content(gram) >= GRAM_MIN_CONTENT:
                self.short_grams.add(gram)
        for i in range(len(words) - NEAR_RUN + 1):
            self.near.add(tuple(words[i:i + NEAR_RUN]))

    def _add_title_words(self, words, kind):
        self.titles.setdefault(words[:2], []).append((words, kind))

    def _add_title(self, tex):
        """Words are counted between spaces, so 'quasi-coherent sheaves' is two words and a term of the field."""
        blanked = tex_prose(re.sub(r"\\\d{3}", "", tex))
        content = 0
        for chunk in blanked.split():
            if any(w not in GENERIC_WORDS for w in words_of(chunk)[0]):
                content += 1
        words = tuple(words_of(blanked)[0])
        if content >= TITLE_MIN_WORDS or (content >= TITLE_SHORT_WORDS and len(words) > TITLE_SHORT_WORDS):
            self._add_title_words(words, "d/title")

    def _add_name(self, name, kind):
        name = name.strip()
        if distinctive_name(name):
            self.names.setdefault(name, kind)
            if distinctive_stem(name):
                self.names.setdefault(name.rsplit(":", 1)[-1], kind)

    def _add_key(self, key):
        """A key with a digit in it is a coined token and counts anywhere.  One made of words alone can also be the
        ordinary name of something public, so it counts only as a citation or a bib entry."""
        key = key.strip()
        if re.search(r"\d", key):
            self._add_name(key, "bibkey")
        elif distinctive_name(key):
            self.word_keys.add(key)

    def _add_names(self, ext, code):
        for m in _LABEL.finditer(code):
            self._add_name(m.group(1), "label")
        for m in _CITE.finditer(code):
            for key in m.group(1).split(","):
                self._add_key(key)
        if ext == ".bib":
            for m in _BIB_KEY.finditer(code):
                if m.group(1).lower() not in ("comment", "string", "preamble"):
                    self._add_key(m.group(2))
            for m in _BIB_TITLE.finditer(code):
                self._add_title(_brace_arg(code, m.end()))
        for pattern in (_HEADING, _TOC_LINE, _BOOKMARK):
            for m in pattern.finditer(code):
                self._add_title(_brace_arg(code, m.end()))
                if pattern is _HEADING and m.group(1):
                    self._add_title(m.group(1))


# ------------------------------------------------------------------------------------------------ rule f

_BENIGN_WORDS = re.compile(r"(?:paren|hypo|syn|anti|pros|meta|epen|dia|anaes|anes)thesis")
_PRIVATE_NAME = re.compile(r"master[\s_-]?arbeit|\.local\.")
_MASTER_TEXT = re.compile(r"master[\s_-]?arbeit", re.I)
_CODE_FILE = re.compile(r"\.(?:rs|py|sh|toml)$")
# In text the word must stand alone or follow a common owner: public files such as a university's class named
# <initials>thesis can be mentioned, and the corpus manifests do.  As a real path they are still blocked.
_STANDALONE_WORD = re.compile(r"(?<![a-z])(?:my|masters?|msc|ma|phd|bachelors?|ba|diplom)?thesis")
_PATH_TOKEN = re.compile(r"[\w.~$/+\-]+")
_TOKEN_EXT = re.compile(r"\.(?:[a-z0-9]{1,8}|[A-Z0-9]{1,8})$")
_TOKEN_GLUED = re.compile(r"\d[-_]?thesis|thesis[-_]?\d")
# Fetched or generated from public packages, under the name they have.
_PUBLIC_TOKEN = re.compile(r"(?:refs/|target/corpus/gen/|/usr/(?:local/)?share/)")
_GLOB_TOKEN = re.compile(r"(?:~|\$HOME|\$\{HOME\}|/home/\w+)/[\w.\-*?\[\]]*[*?\[][\w.\-*?\[\]]*")
_INVISIBLE = re.compile("[\u00ad\u034f\u061c\u115f\u1160\u17b4\u17b5\u180e\u200b-\u200f\u202a-\u202e\u2060-\u206f\ufeff]")


def _normal_name(name):
    return unicodedata.normalize("NFKC", _INVISIBLE.sub("", name)).lower()


def path_is_private(path, strict=False):
    """A source file may be named after what it benchmarks, since its content is scanned like any other: a data file
    or a directory may not, and a ref name (strict) has no extension to tell."""
    low = _BENIGN_WORDS.sub("", _normal_name(path))
    if _PRIVATE_NAME.search(low):
        return True
    if "thesis" not in low:
        return False
    head, tail = os.path.split(low)
    return strict or "thesis" in head or not _CODE_FILE.search(tail)


def text_path_mentions(text):
    """Offsets of path-like mentions.  The plain word 'thesis' stays allowed: it is a benchmark name in the docs,
    and so does an identifier such as bench_thesis, and the name of a source file."""
    for m in _MASTER_TEXT.finditer(text):
        yield m.start()
    if "thesis" in text.lower():
        for m in _PATH_TOKEN.finditer(text):
            tok = m.group().rstrip(".,:;-")
            low = tok.lower()
            if "thesis" not in low:
                continue
            low = _BENIGN_WORDS.sub("", low)
            if _STANDALONE_WORD.search(low) and not _PUBLIC_TOKEN.match(tok):
                rooted = tok.startswith(("/", "~/", "./", "../", "$")) and "/" in tok
                if rooted or _TOKEN_GLUED.search(low) or (_TOKEN_EXT.search(tok) and not _CODE_FILE.search(low)):
                    yield m.start()
    if "*" in text or "?" in text or "[" in text:
        target = PRIVATE_DIRNAME.lower()
        for m in _GLOB_TOKEN.finditer(text):
            for part in m.group().lower().split("/")[1:]:
                if part.strip("*?") and fnmatch.fnmatchcase(target, part):
                    yield m.start()


# ------------------------------------------------------------------------------------------------ rule g

_MAGIC = (b"%PDF-", b"%!PS", b"\x89PNG", b"\xff\xd8\xff", b"GIF8", b"PK\x03\x04", b"PK\x05\x06", b"\x1f\x8b", b"BZh",
          b"\xfd7zXZ", b"7z\xbc\xaf", b"\x28\xb5\x2f\xfd", b"II*\0", b"MM\0*", b"RIFF", b"Rar!", b"\x04\x22\x4d\x18")
_CONTROL = re.compile(rb"[\x00-\x08\x0b\x0e-\x1f\x7f]")
_HEX_ANY = re.compile(r"(?<![0-9A-Fa-f])[0-9A-Fa-f]{%d,}(?![0-9A-Fa-f])" % HEX_ANY)
_B64_ANY = re.compile(r"[A-Za-z0-9+/]{%d,}={0,2}|[A-Za-z0-9_-]{%d,}={0,2}" % (BASE64_ANY, BASE64_ANY))
_B64_LINE = re.compile(r"""^\s*["'`]?([A-Za-z0-9+/_=-]{%d,})["'`]?\s*[,;]?\s*$""" % WRAPPED_LINE)
_HEX_LINE = re.compile(r"""^\s*["'`]?([0-9A-Fa-f]{%d,})["'`]?\s*[,;]?\s*$""" % WRAPPED_LINE)
_HEXDUMP_LINE = re.compile(r"^\s*[0-9a-fA-F]{4,8}:?\s+((?:[0-9a-fA-F]{2,4}\s+){3,}[0-9a-fA-F]{0,4})")
_DENSE_LINE = re.compile(r"^\s*(\S{40,})\s*$")
_UU_LINE = re.compile(r"^M[!-`]{60}$")
_UU_BEGIN = re.compile(r"^begin [0-7]{3} \S", re.M)
_HASH_LENGTHS = (40, 56, 64, 96, 128)
_NUMBER = r"(?:0[xX][0-9a-fA-F_]{1,18}|\d{1,10})(?:_?[iu](?:8|16|32|64|size))?"
_TABLE = re.compile(r"(?:(?<![\w.])%s(?![\w.])[\s,;]*){%d,}" % (_NUMBER, TABLE_MIN_ITEMS))
_TABLE_ITEM = re.compile(r"(?<![\w.])(0[xX][0-9a-fA-F_]+|\d+)")
_ESCAPED_BYTES = re.compile(r"(?:\\x[0-9a-fA-F]{2}){%d,}|(?:\\[0-7]{3}){%d,}" % (TEXT_MIN_BYTES, TEXT_MIN_BYTES))
_DENSE = re.compile(r"\S{%d,}" % DENSE_RUN)
_DECOMPRESS = ((b"\x1f\x8b", lambda d: zlib.decompress(d, 31)), (b"BZh", bz2.decompress), (b"\xfd7zXZ", lzma.decompress))


def is_binary(data, path=""):
    head = data[:4096].lstrip(b"\xef\xbb\xbf \t\r\n")
    is_svg = path.lower().endswith(".svg") or (head.startswith((b"<svg", b"<?xml", b"<!DOCTYPE svg")) and b"<svg" in head)
    return is_svg or data.startswith(_MAGIC) or _CONTROL.search(data) is not None


def _entropy(s):
    counts = {}
    for c in s:
        counts[c] = counts.get(c, 0) + 1
    n = len(s)
    return -sum(k / n * math.log2(k / n) for k in counts.values())


def _mixed(s):
    """Upper case, lower case and digits, spread the way encoded data spreads them: paths and long names are far
    below ENCODED_BITS even when they use the right alphabet."""
    return (any(c.isupper() for c in s) and any(c.islower() for c in s) and any(c.isdigit() for c in s)
            and _entropy(s) >= ENCODED_BITS)


def _as_text(data):
    """Decoded bytes as text when they read like text.  Random bytes essentially never do: a third of them are not even
    ASCII, and identifiers and hashes that happen to fit an encoding's alphabet decode to random bytes."""
    if len(data) < TEXT_MIN_BYTES:
        return None
    for encoding in ("utf-8", "utf-16"):
        if encoding == "utf-16" and data.count(0) * 3 < len(data):
            continue
        try:
            text = data.decode(encoding)
        except (UnicodeDecodeError, ValueError):
            continue
        printable = sum(c.isprintable() or c in "\n\r\t" for c in text)
        letters = sum(c.isalpha() for c in text)
        if printable >= 0.97 * len(text) and letters >= 0.5 * len(text) and any(c in " \n\t" for c in text):
            return text
    return None


def inspect_payload(data, depth=0):
    """(blocks by itself, text): does decoded data hold something other than numbers, and what does it read as?"""
    for magic, decompress in _DECOMPRESS:
        if data.startswith(magic) and depth < 3:
            try:
                return True, inspect_payload(decompress(data), depth + 1)[1]
            except (OSError, EOFError, ValueError, zlib.error, lzma.LZMAError):
                return True, None
    if data.startswith(_MAGIC):
        return True, None
    text = _as_text(data)
    if text is not None:
        return len(data) >= TEXT_ALWAYS_BYTES, text
    if depth < 3 and len(data) >= 64:
        for wbits in (15, -15):     # zlib, raw deflate: neither has a magic number worth the name
            try:
                unpacked = zlib.decompress(data, wbits)
            except zlib.error:
                continue
            if len(unpacked) > len(data) // 2:
                return True, inspect_payload(unpacked, depth + 1)[1]
    if len(data) >= OPAQUE_BYTES or (len(data) >= RANDOM_BYTES and _entropy(data) >= RANDOM_BITS):
        return True, None
    return False, None


def _b64_decode(s):
    s = s.rstrip("=")
    if len(s) % 4 == 1:
        s = s[:-1]
    try:
        return base64.b64decode(s.replace("-", "+").replace("_", "/") + "=" * (-len(s) % 4))
    except (binascii.Error, ValueError):
        return b""


def _hex_decode(s):
    return bytes.fromhex(s[:len(s) & ~1])


def _table_payloads(run):
    """A table of numbers as bytes, in each byte order its largest value allows."""
    items = []
    for tok in _TABLE_ITEM.findall(run):
        items.append(int(tok.replace("_", ""), 16) if tok[:2] in ("0x", "0X") else int(tok))
    top = max(items)
    if top < 1 << 8:
        return [bytes(items)]
    for width in (2, 4, 8):
        if top < 1 << (8 * width):
            return [b"".join(x.to_bytes(width, order) for x in items) for order in ("big", "little")]
    return []


def _blocks(lines, pattern):
    """Runs of at least two lines that all match pattern, with the equal widths a wrapping tool leaves: (line index,
    matched parts)."""
    i, n = 0, len(lines)
    while i < n:
        m = pattern.match(lines[i])
        if not m:
            i += 1
            continue
        parts, width = [m.group(1)], len(m.group(1))
        j = i + 1
        while j < n:
            m = pattern.match(lines[j])
            if not m or len(parts[-1]) != width:
                break
            parts.append(m.group(1))
            j += 1
        if len(parts) >= 2:
            yield i, parts
        i = j


def encoded_runs(text):
    """(offset, decoded text or None, whether the run blocks whatever it says) of each run that holds encoded data."""
    for m in _HEX_ANY.finditer(text):
        recognised, payload = inspect_payload(_hex_decode(m.group()))
        if recognised or payload or len(m.group()) >= HEX_RUN:
            yield m.start(), payload, recognised or len(m.group()) >= HEX_RUN
    for m in _B64_ANY.finditer(text):
        recognised, payload = inspect_payload(_b64_decode(m.group()))
        long_run = len(m.group()) >= BASE64_RUN and _mixed(m.group())
        if recognised or payload or long_run:
            yield m.start(), payload, recognised or long_run
    for m in _ESCAPED_BYTES.finditer(text):
        run = m.group()
        data = bytes.fromhex(run.replace("\\x", "")) if run[1] == "x" else bytes(int(x, 8) & 255 for x in run.split("\\")[1:])
        recognised, payload = inspect_payload(data)
        if recognised or payload:
            yield m.start(), payload, recognised
    for m in _TABLE.finditer(text):
        for data in _table_payloads(m.group()):
            recognised, payload = inspect_payload(data)
            if recognised or payload:
                yield m.start(), payload, recognised
                break
    for m in _DENSE.finditer(text):
        if _entropy(m.group()) >= DENSE_BITS:
            yield m.start(), None, True
    # Blocks wrapped over several lines, as base64, xxd, uuencode and base85 tools produce them.
    lines, starts = _line_table(text)
    for n, parts in _blocks(lines, _HEX_LINE):
        joined = "".join(parts)
        recognised, payload = inspect_payload(_hex_decode(joined))
        long_run = len(joined) >= HEX_RUN and len(parts[0]) not in _HASH_LENGTHS
        if recognised or payload or long_run:
            yield starts[n], payload, recognised or long_run
    for n, parts in _blocks(lines, _B64_LINE):
        joined = "".join(parts)
        if _HEX_LINE.match(parts[0]):
            continue
        recognised, payload = inspect_payload(_b64_decode(joined))
        long_run = len(joined) >= BASE64_RUN and _mixed(joined)
        if recognised or payload or long_run:
            yield starts[n], payload, recognised or long_run
    for n, parts in _blocks(lines, _DENSE_LINE):
        joined = "".join(parts)
        for decode in (base64.a85decode, base64.b85decode):
            try:
                data = decode(joined)
            except ValueError:
                continue
            recognised, payload = inspect_payload(data)
            if recognised or payload:
                yield starts[n], payload, recognised
                break
    run = 0
    hex_parts = []
    for n, line in enumerate(lines + [""]):
        m = _HEXDUMP_LINE.match(line)
        if m:
            run = run if hex_parts else n
            hex_parts.append("".join(m.group(1).split()))
        elif hex_parts:
            if len(hex_parts) >= 3:
                recognised, payload = inspect_payload(_hex_decode("".join(hex_parts)))
                if recognised or payload:
                    yield starts[run], payload, recognised
            hex_parts = []
    for n, parts in _uu_blocks(lines):
        data = b""
        for part in parts:
            try:
                data += binascii.a2b_uu(part)
            except binascii.Error:
                pass
        yield starts[n], inspect_payload(data)[1], True


def _uu_blocks(lines):
    i, n = 0, len(lines)
    while i < n:
        begun = _UU_BEGIN.match(lines[i]) is not None
        j = i + 1 if begun else i
        parts = []
        while j < n and _UU_LINE.match(lines[j]):
            parts.append(lines[j])
            j += 1
        if begun or len(parts) >= 3:
            yield i, parts
        i = max(j, i + 1)


# ------------------------------------------------------------------------------------------------ rule h

_TJ_OP = re.compile(r"\[((?:\((?:[^()\\\n]|\\.)*\)|[-\d. \t])*)\]\s*TJ\b|\(((?:[^()\\\n]|\\.)*)\)\s*Tj\b")
_TJ_STRING = re.compile(r"\((?:[^()\\\n]|\\.)*\)")
_TJ_KERN = re.compile(r"\)\s?-?\d+(?:\.\d+)?\s?\(")     # '(har)28(b)' inside an array, also when a clipped record lost its end
_FONT_ID = r"\\+[A-Za-z][A-Za-z0-9]*/[A-Za-z0-9]+/[a-z]+/[a-z]+/[\d.]+"
_BOX_LINE = re.compile(r"^[^\n\\]{0,12}[.|]+\\+(?:hbox\(|vbox\(|glue\b|kern\b|penalty\b|mathon\b|mathoff\b|discretionary\b"
                       r"|rule\(|pdfcolorstack\b|pdfliteral\b|[A-Za-z][A-Za-z0-9]*/[A-Za-z0-9]+/)", re.M)
_LOG_TEXT_LINE = re.compile(r"(?:^|\")" + _FONT_ID + r" [^\n]*?[A-Za-z]{3}[^\n]* [^\n]*$", re.M)
_USER_RECORD = re.compile(r"\"id\"\s*:\s*\"user/|^[ \t]*user/[\w.-]+[ :]", re.M)


def output_shapes(text):
    """(offset, rule) where a blob holds engine output: text in PDF operators, a box log, a private entry's record."""
    if "TJ" in text or "Tj" in text:
        total, longest, first = 0, 0, None
        for m in _TJ_OP.finditer(text):
            if first is None:
                first = m.start()
            parts = _TJ_STRING.findall(m.group(1)) if m.group(1) is not None else (m.group(2),)
            letters = sum(c.isalpha() for part in parts for c in part)
            total += letters
            longest = max(longest, letters)
        if longest >= TJ_MIN_LETTERS or total >= TJ_TOTAL_LETTERS:
            yield first, "h/pdf-text"
    if ")" in text:
        kerns = _TJ_KERN.findall(text)
        if len(kerns) >= TJ_MIN_KERNS:
            yield _TJ_KERN.search(text).start(), "h/pdf-text"
    if "\\" in text:
        first = _BOX_LINE.search(text)
        if first:
            count = sum(1 for _ in _BOX_LINE.finditer(text))
            if count >= BOX_MIN_LINES:
                yield first.start(), "h/box-log"
        logged = list(_LOG_TEXT_LINE.finditer(text))
        if len(logged) >= 2:
            yield logged[0].start(), "h/box-log"
    m = _USER_RECORD.search(text)
    if m:
        yield m.start(), "h/record"


# ------------------------------------------------------------------------------------------------ other readings

_ESCAPE = re.compile(r"\\(u[0-9a-fA-F]{4}|x[0-9a-fA-F]{2}|.)", re.S)
_ESCAPE_HINT = re.compile(r"\\[nrt\"\\/u]")
_ESCAPE_MAP = {"n": "\n", "t": " ", "r": " ", "\\": "\\", '"': '"', "'": "'", "/": "/"}
_TEX_FILE = re.compile(r"\.(?:tex|sty|cls|ltx|dtx|ins|bib|bbl|aux)$")
_GREP_HEAD = re.compile(r"^[\w./\\-]+:\d+:")
_NUMBER_HEAD = re.compile(r"^\s*\d+[\t:|]\s?")
_MARK_HEAD = re.compile(r"^\s*(?:[-+>|!#;*]+|//[/!]?|--)[ \t]?")
_QUOTED_LINE = re.compile(r"""^\s*[bBrRuUfF]*\#*(["'`])(.*?)(?:\\n)?\1\#*[ \t]*\\?[,;)\]+]*[ \t]*$""")
_STRING_JOIN = re.compile(r"\"\s*,\s*\"")
_TAG = re.compile(r"</?[A-Za-z][^<>\n]{0,400}>")
_CAMEL = re.compile(r"(?<=[a-z])(?=[A-Z])")
_ROT13 = str.maketrans("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz",
                       "NOPQRSTUVWXYZABCDEFGHIJKLMnopqrstuvwxyzabcdefghijklm")


def _unescape(m):
    e = m.group(1)
    if len(e) > 1:
        return chr(int(e[1:], 16))
    return _ESCAPE_MAP.get(e, m.group())     # anything else was not an escape, such as the \\a of \\alpha


def decode_escapes(text, path):
    """What string literals and JSON hold, as a report or a test would write it: backslashes doubled and \\n for newline.
    Only lines holding an escape are worth reading twice.  TeX files are full of '\\n' that means no such thing, so
    there the marker of doubled backslashes is required."""
    if _ESCAPE_HINT.search(text) is None:
        return None
    if _TEX_FILE.search(path) and not ("\\\\begin" in text or "\\\\section" in text):
        return None
    kept = [line for line in text.split("\n") if _ESCAPE_HINT.search(line)]
    return _ESCAPE.sub(_unescape, "\n".join(kept))


def strip_line_heads(text):
    """Lines without what tools put in front of them: diff and list marks, numbers, file:line, comment marks, and the
    quotes of one string per line."""
    out = []
    for line in text.split("\n"):
        line = _GREP_HEAD.sub("", line, count=1)
        line = _NUMBER_HEAD.sub("", line, count=1)
        line = _MARK_HEAD.sub("", line, count=1)
        m = _QUOTED_LINE.match(line)
        out.append(m.group(2) if m else line)
    return "\n".join(out)


def _normalized_lines(text):
    """Lines that invisible characters or compatibility forms change, normalized."""
    if text.isascii():
        return None
    changed = []
    for line in text.split("\n"):
        if not line.isascii():
            normal = unicodedata.normalize("NFKC", _INVISIBLE.sub("", line))
            if normal != line:
                changed.append(normal)
    return "\n".join(changed) if changed else None


def _environment_regions(text):
    """Display environments once per-line decoration is gone: marks and quotes, then the escapes inside the quotes, and
    the separators of strings that a JSON array keeps on one line.  Marks are no obstacle to the word rules, only here."""
    regions = []
    stripped = strip_line_heads(text)
    unescaped = _ESCAPE.sub(_unescape, stripped) if "\\\\begin" in stripped else None
    if stripped == text and not unescaped:
        return ""
    for candidate in filter(None, (stripped, unescaped, unescaped and _STRING_JOIN.sub("\n", unescaped))):
        regions.extend(candidate[a:b] for a, b, _, _, _ in iter_envs(candidate, _BODY_ENV))
    return "\n".join(dict.fromkeys(regions))


def alternate_readings(text, path):
    """Other things this blob can be read as, with all rules applied.  Lines no longer map back to the blob.  Each
    holds only what reads differently, so that an ordinary file costs little more than one pass."""
    if len(text) > VIEW_MAX_BYTES:
        return []
    normal = _normalized_lines(text)
    decoded = decode_escapes(text, path)
    views = [v for v in (normal, decoded) if v]
    for base in [text] + views:
        lines = base.split("\n")
        tagged = sum(1 for line in lines if _GREP_HEAD.match(line) or _NUMBER_HEAD.match(line))
        if tagged * 2 >= len(lines) and tagged >= 2:
            views.append(strip_line_heads(base))    # a listing: every line starts with something that is not text
    if "\\begin" in text:
        views.append(_environment_regions(text))
    if len(_TAG.findall(text)) >= 8:
        views.append(html.unescape(_TAG.sub(" ", text)))
    return list(dict.fromkeys(v for v in views if v and v != text))


def scrambled_readings(text):
    """Cheap undoing of cheap scrambling; only the word rules see these."""
    if len(text) > SCRAMBLE_MAX_BYTES:
        return []
    views = [text[::-1], "\n".join(line[::-1] for line in text.split("\n")), text.translate(_ROT13)]
    if _CAMEL.search(text):
        views.append(_CAMEL.sub(" ", text))
    return views


# ------------------------------------------------------------------------------------------------ scanning


def _printable_prefix(s):
    s = "".join(c if c.isprintable() else " " for c in s).strip()
    return s[:PREFIX_CHARS]


def _line_table(text):
    lines = text.split("\n")
    starts = [0]
    for line in lines[:-1]:
        starts.append(starts[-1] + len(line) + 1)
    return lines, starts


_LISTABLE_RULES = ("g/encoded", "h/pdf-text", "h/box-log")
_ERROR_LINE = re.compile(r"^(?:[^\n]{0,12}\s)?l\.\d+ (.*)$", re.M)
_NAME_SEPARATORS = re.compile(r"[/_.\-+~]+")
_HASH_TOKEN = re.compile(r"(?<![0-9a-f])[0-9a-f]{64}(?![0-9a-f])")
_SCANNING = None    # the scanner, for pool workers: they inherit it across fork


def _scan_in_worker(text, path):
    return _SCANNING.text_rules(text, path)


class Scanner:
    def __init__(self, sources, allowed, parallel=False):
        self.src = sources
        self.allowed = allowed
        self.findings = []      # (rule, path, line, prefix)
        self.seen_blobs = {}    # content digest -> findings relative to the blob, or a pending result
        self.seen_names = {}
        self.seen_messages = {}
        self.pending = []       # (result, path, listed)
        self.parallel = parallel
        self.pool = None
        self.blobs = 0
        self.bytes = 0
        self.messages = 0

    def add(self, rule, path, line, prefix):
        self.findings.append((rule, path, line, _printable_prefix(prefix)))

    def check_path(self, path):
        if path_is_private(path):
            self.add("f/path", path, 0, path)
        if self.src.names.get(os.path.basename(path)) == "file":
            self.add("d/file", path, 0, path)
        self._check_name(path, path)

    def check_ref(self, name):
        if path_is_private(name, strict=True):
            self.add("f/ref", name, 0, name)
        self._check_name(name, name)

    def _check_name(self, label, name):
        """A file or ref name holding a title or a sentence."""
        rules = self.seen_names.get(name)
        if rules is None:
            hits = {}

            def at(offset, rule):
                hits.setdefault(rule, name)

            words, offsets = words_of(_NAME_SEPARATORS.sub(" ", name))
            self._word_rules(words, offsets, at)
            rules = self.seen_names[name] = sorted(hits)
        for rule in rules:
            self.add(rule, label, 0, name)

    def check_blob(self, path, data):
        """Rules e, g, h and all text rules on one blob.  Text results are kept per digest, the allowlist is per path."""
        self.blobs += 1
        self.bytes += len(data)
        digest = hashlib.sha256(data).hexdigest()
        listed = (path, digest) in self.allowed
        if digest in self.src.hashes:
            self.add("e", path, 0, "identical to a private file")
        too_big = len(data) > MAX_SCAN_BYTES
        if too_big or is_binary(data, path):
            if not listed:
                reason = "too large to scan" if too_big else "binary or SVG, not listed"
                self.add("g/binary", path, 0, reason)
            return
        try:
            text = data.decode("utf-8")
        except UnicodeDecodeError:
            if not listed:
                self.add("g/binary", path, 0, "not UTF-8, not listed")
            return
        cached = self.seen_blobs.get(digest)
        if cached is None:
            cached = self.seen_blobs[digest] = self._submit(text, path)
        if isinstance(cached, list):
            self._record(cached, path, listed)
        else:
            self.pending.append((cached, path, listed))

    def _record(self, rows, path, listed):
        for rule, line, prefix in rows:
            if not (listed and rule in _LISTABLE_RULES):
                self.add(rule, path, line, prefix)

    def _submit(self, text, path):
        if self.parallel and len(text) >= PARALLEL_MIN_BYTES:
            if self.pool is None:
                self._start_pool()
            if self.pool is not None:
                return self.pool.apply_async(_scan_in_worker, (text, path))
        return self.text_rules(text, path)

    def _start_pool(self):
        global _SCANNING
        _SCANNING = self
        try:
            self.pool = multiprocessing.get_context("fork").Pool(min(PARALLEL_JOBS, os.cpu_count() or 1))
        except (OSError, ValueError):
            self.parallel = False

    def finish(self):
        for result, path, listed in self.pending:
            self._record(result.get(), path, listed)
        self.pending = []

    def stop_pool(self):
        """Before git.close(): workers hold copies of the pipe to 'git cat-file', which waits for all of them to close."""
        if self.pool is not None:
            self.pool.terminate()
            self.pool.join()
            self.pool = None

    def check_message(self, label, text):
        self.messages += 1
        rows = self.seen_messages.get(text)
        if rows is None:
            rows = self.seen_messages[text] = self.text_rules(text, label)
        for rule, line, prefix in rows:
            self.add(rule, label, line, prefix)

    def text_rules(self, text, path):
        hits = {}
        lines, starts = _line_table(text)

        def at(offset, rule):
            n = bisect.bisect_right(starts, offset) - 1
            end = text.find("\n", offset)
            hits.setdefault((rule, n + 1), text[offset:end if end >= 0 else None][:PREFIX_CHARS + 8])

        for off in text_path_mentions(text):
            at(off, "f/text")
        for off, rule in output_shapes(text):
            at(off, rule)
        payloads = []
        for off, payload, blocks in encoded_runs(text):
            if blocks:
                at(off, "g/encoded")
            if payload:
                payloads.append(payload)
        if self.src.available:
            self._private_rules(text, lines, starts, at)
            quoted_log = _ERROR_LINE.search(text) is None     # a log's own error lines were just checked as they stand
            for reading in alternate_readings(text, path) + payloads:
                self._reading_rules(reading, hits, quoted_log)
            for reading in scrambled_readings(text):
                self._scrambled_rules(reading, hits)
        return [(rule, line, prefix) for (rule, line), prefix in sorted(hits.items(), key=lambda kv: (kv[0][1], kv[0][0]))]

    def _reading_rules(self, reading, hits, error_lines):
        """Text held in some other form: lines no longer map back, so everything lands on line 1."""
        lines, starts = _line_table(reading)

        def at(offset, rule):
            end = reading.find("\n", offset)
            hits.setdefault((rule, 1), reading[offset:end if end >= 0 else None][:PREFIX_CHARS + 8])

        for off in text_path_mentions(reading):
            at(off, "f/text")
        for off, rule in output_shapes(reading):
            at(off, rule)
        self._private_rules(reading, lines, starts, at, error_lines)

    def _scrambled_rules(self, reading, hits):
        words, offsets = words_of(raw_prose(reading))

        def at(offset, rule):
            hits.setdefault((rule, 1), "scrambled text")

        self._gram_scan(words, offsets, at)
        self._near_scan(words, offsets, at)

    def _private_rules(self, text, lines, starts, at, error_lines=True):
        src = self.src
        # a: whole lines
        exact, soft = src.exact_lines, src.soft_lines
        short = {}
        for n, line in enumerate(lines):
            if len(line) >= SOFT_MIN_CHARS:
                variants = line_variants(line)
                if not exact.isdisjoint(variants):
                    at(starts[n] + len(line) - len(line.lstrip()), "a")
                for v in variants & soft:
                    short.setdefault(v, n)
        if len(short) >= SOFT_CLUSTER:
            n = min(short.values())
            at(starts[n] + len(lines[n]) - len(lines[n].lstrip()), "a/cluster")
        if error_lines:
            self._error_lines(text, at)
        # a and c: literals anywhere, with comments removed and with comment text kept
        for transform in (code_of, unmarked):
            parts = [squeeze(transform(line)) for line in lines]
            squeezed = "".join(parts)
            if transform is unmarked and squeezed == "".join(squeeze(code_of(line)) for line in lines):
                break
            self._literal_scan(squeezed, parts, lines, starts, at)
        # b and d: word streams
        streams = [strip_comments(text)]
        if "%" in text:
            streams.append(open_comments(text))
        prose = [tex_prose(s) for s in streams]
        if "\\" in text or "$" in text:
            prose.append(raw_prose(text))
        for blanked in prose:
            words, offsets = words_of(blanked)
            self._word_rules(words, offsets, at)
        # c: structure
        if src.shapes or src.relettered or src.diagrams:
            code = strip_comments(text)
            for start, _, _, a, b in iter_envs(code, _TIKZCD):
                body = code[a:b]
                shape, relettered = diagram_keys(body)
                if relettered in src.relettered and relettered is not None:
                    at(start, "c/relettered")
                elif shape in src.shapes and shape is not None:
                    at(start, "c/shape")
                elif self._near_diagram(body):
                    at(start, "c/fuzzy")
        # d: names and macros
        names = src.names
        if names:
            for m in _NAME_TOKEN.finditer(text):
                tok = m.group().rstrip(".:+-_")
                kind = names.get(tok)
                if kind:
                    at(m.start(), "d/" + kind)
        if src.macros:
            for m in _MACRO_USE.finditer(text):
                if m.group(1) in src.macros:
                    at(m.start(), "d/macro")
        if src.word_keys:
            for m in _CITE.finditer(text):
                if not src.word_keys.isdisjoint(key.strip() for key in m.group(1).split(",")):
                    at(m.start(), "d/bibkey")
            for m in _BIB_KEY.finditer(text):
                if m.group(2) in src.word_keys:
                    at(m.start(2), "d/bibkey")
        # e: a private file's digest written out
        for m in _HASH_TOKEN.finditer(text):
            if m.group() in src.hashes:
                at(m.start(), "e/text")

    def _error_lines(self, text, at):
        """The source line TeX quotes in an error, in two parts around the error's position: too short for rule a,
        but nothing but a private document has this line to quote."""
        if "l." not in text:
            return
        seen = 0
        for m in _ERROR_LINE.finditer(text):
            seen += 1
            if seen > ERROR_MAX_FRAGMENTS:
                return
            parts = [m.group(1)]
            end = text.find("\n", m.end() + 1)
            parts.append(text[m.end() + 1:end if end >= 0 else None])
            for part in parts:
                v = squeeze(part)
                if (len(v) >= SOFT_MIN_CHARS and v in self.src.line_text
                        and not is_boilerplate(part, ".tex", SOFT_MIN_LETTERS, 1, 0)):
                    at(m.start(), "a/error-line")
                    break

    def _near_diagram(self, body):
        if not self.src.diagrams:
            return False
        texts, labels, masked = diagram_strings(body)
        for private_texts, private_labels, private_masked in self.src.diagrams:
            if (_strings_share(private_texts | private_labels, texts | labels, FUZZY_MIN_MATCHED) >= FUZZY_SHARE
                    or _strings_share(private_texts, texts, FUZZY_PART_MATCHED) >= FUZZY_PART_SHARE
                    or _strings_share(private_labels, labels, FUZZY_PART_MATCHED) >= FUZZY_PART_SHARE
                    or _strings_share(private_masked, masked, FUZZY_PART_MATCHED) >= FUZZY_SHARE):
                return True
        return False

    def _literal_scan(self, squeezed, parts, lines, starts, at):
        table = self.src.literals
        if not table:
            return
        cumulative = []
        total = 0
        for p in parts:
            total += len(p)
            cumulative.append(total)
        key = self.src.LITERAL_KEY
        get = table.get
        for i in range(len(squeezed) - key + 1):
            bucket = get(squeezed[i:i + key])
            if bucket:
                for literal, kind in bucket:
                    if squeezed.startswith(literal, i):
                        n = bisect.bisect_right(cumulative, i)
                        at(starts[n] + len(lines[n]) - len(lines[n].lstrip()), kind)

    def _word_rules(self, words, offsets, at):
        self._gram_scan(words, offsets, at)
        self._near_scan(words, offsets, at)
        self._title_scan(words, offsets, at)

    def _gram_scan(self, words, offsets, at):
        src = self.src
        for run, grams in ((SHORT_RUN, src.short_grams), (WORD_RUN, src.grams), (PLAIN_WORD_RUN, src.long_grams)):
            if not grams:
                continue
            i, last = 0, len(words) - run
            while i <= last:
                if tuple(words[i:i + run]) in grams:
                    at(offsets[i], "b")
                    j = i + 1
                    while j <= last and tuple(words[j:j + run]) in grams:
                        j += 1
                    i = j + run - 1
                else:
                    i += 1

    def _near_scan(self, words, offsets, at):
        """Sliding share of private three-word runs: what stays of a copied passage after light rewording."""
        near = self.src.near
        n = len(words) - NEAR_RUN + 1
        if n < NEAR_WINDOW or not near:
            return
        need = NEAR_SHARE * NEAR_WINDOW
        flags = [tuple(words[i:i + NEAR_RUN]) in near for i in range(n)]
        count = sum(flags[:NEAR_WINDOW])
        i = NEAR_WINDOW
        while True:
            if count >= need:
                at(offsets[i - NEAR_WINDOW], "b/near")
                return
            if i >= n:
                return
            count += flags[i] - flags[i - NEAR_WINDOW]
            i += 1

    def _title_scan(self, words, offsets, at):
        src = self.src
        if src.term_words and not src.term_words.isdisjoint(words):
            for i, w in enumerate(words):
                if w in src.term_words:
                    at(offsets[i], "d/identity")
        titles = src.titles
        if not titles:
            return
        for i in range(len(words) - 1):
            bucket = titles.get((words[i], words[i + 1]))
            if bucket:
                for title, kind in bucket:
                    if tuple(words[i:i + len(title)]) == title:
                        at(offsets[i], kind)


# ------------------------------------------------------------------------------------------------ git access


class Git:
    def __init__(self):
        self.root = None
        p = self.run("rev-parse", "--show-toplevel", check=False)
        if p.returncode == 0:
            self.root = p.stdout.decode("utf-8", "surrogateescape").strip()
        self.store = os.path.join(os.path.abspath(self.text("rev-parse", "--git-common-dir").strip()), STORE_DIR)
        self._batch = None

    def run(self, *args, check=True):
        try:
            p = subprocess.run(("git",) + args, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        except OSError as e:
            raise GuardError("cannot run git: %s" % e.strerror)
        if check and p.returncode:
            raise GuardError("git %s failed: %s" % (args[0], p.stderr.decode("utf-8", "replace").strip()[:200]))
        return p

    def text(self, *args):
        return self.run(*args).stdout.decode("utf-8", "surrogateescape")

    def config_all(self, key):
        """Values from this clone's own config only: 'git -c' and the environment cannot change what hooks check."""
        return self.run("config", "--local", "--get-all", key, check=False).stdout.decode("utf-8", "surrogateescape").splitlines()

    def blob(self, sha):
        if self._batch is None:
            self._batch = subprocess.Popen(("git", "cat-file", "--batch"), stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        self._batch.stdin.write(sha.encode() + b"\n")
        self._batch.stdin.flush()
        header = self._batch.stdout.readline().split()
        if len(header) != 3:
            raise GuardError("cannot read object %s" % sha)
        data = self._batch.stdout.read(int(header[2]))
        self._batch.stdout.read(1)
        return data

    def commits_among(self, shas):
        """Those of these objects that are commits this repository holds."""
        p = subprocess.run(("git", "cat-file", "--batch-check"), input="\n".join(shas).encode() + b"\n", stdout=subprocess.PIPE,
                           stderr=subprocess.DEVNULL)
        found = []
        for line in p.stdout.decode().splitlines():
            parts = line.split()
            if len(parts) == 3 and parts[1] == "commit":
                found.append(parts[0])
        return found

    def close(self):
        if self._batch is not None:
            self._batch.stdin.close()
            self._batch.wait()


def _parse_raw(output):
    """(mode, sha, path) from 'git diff --raw -z --no-abbrev --no-renames' output."""
    fields = output.split("\0")
    for i in range(0, len(fields) - 1, 2):
        meta = fields[i].split()
        yield meta[1], meta[3], fields[i + 1]


def _symlink_escapes(path, target):
    """A link out of the tree: the private excerpts are linked in this way, and the blob would publish the path."""
    normal = os.path.normpath(os.path.join(os.path.dirname(path), target))
    return os.path.isabs(target) or target.startswith("~") or normal == ".." or normal.startswith("../")


def _scan_entry(git, scanner, mode, sha, path):
    scanner.check_path(path)
    if set(sha) == {"0"}:
        return
    if mode == "160000":
        scanner.add("g/binary", path, 0, "submodule, contents not inspectable")
        return
    data = git.blob(sha)
    if mode == "120000" and _symlink_escapes(path, data.decode("utf-8", "surrogateescape")):
        scanner.add("f/path", path, 0, "symlink out of the repository")
    scanner.check_blob(path, data)


def scan_staged(git, scanner):
    raw = git.text("diff", "--cached", "--raw", "-z", "--no-abbrev", "--no-renames", "--diff-filter=ACMT")
    for mode, sha, path in _parse_raw(raw):
        _scan_entry(git, scanner, mode, sha, path)
    for ident, label in (("GIT_AUTHOR_IDENT", "author"), ("GIT_COMMITTER_IDENT", "committer")):
        p = git.run("var", ident, check=False)
        if p.returncode == 0:
            value = p.stdout.decode("utf-8", "replace").rsplit(" ", 2)[0]
            scanner.check_message(label, value)


def scan_tree(git, scanner, ref):
    for entry in git.text("ls-tree", "-r", "-z", "--full-tree", ref).split("\0"):
        if entry:
            meta, path = entry.split("\t", 1)
            mode, _, sha = meta.split()
            _scan_entry(git, scanner, mode, sha, path)


def scan_worktree(git, scanner):
    listed = git.text("-C", git.root, "ls-files", "-z", "--cached", "--others", "--exclude-standard")
    for path in dict.fromkeys(x for x in listed.split("\0") if x):
        scanner.check_path(path)
        full = os.path.join(git.root, path)
        try:
            data = os.readlink(full).encode("utf-8", "surrogateescape") if os.path.islink(full) else None
            if data is not None and _symlink_escapes(path, data.decode("utf-8", "surrogateescape")):
                scanner.add("f/path", path, 0, "symlink out of the repository")
            if data is None:
                if os.path.isdir(full):     # an embedded repository shows up as a directory
                    scanner.add("g/binary", path, 0, "nested repository, not inspectable")
                    continue
                with open(full, "rb") as f:
                    data = f.read()
        except FileNotFoundError:           # staged deletion, or another process just removed it
            continue
        except OSError as e:
            raise GuardError("cannot read %s: %s" % (path, e.strerror))
        scanner.check_blob(path, data)


_IDENT_HEADER = re.compile(rb"^(author|committer|tagger) (.*?) <(.*?)> \d+ [-+]\d+$", re.M)


def _check_object_text(scanner, label, body):
    """A commit or tag object: its message and the names in its header, where any text can be put."""
    head, _, message = body.partition(b"\n\n")
    scanner.check_message(label, message.decode("utf-8", "replace"))
    for m in _IDENT_HEADER.finditer(head):
        for part in (m.group(2), m.group(3)):
            scanner.check_message("%s %s" % (label, m.group(1).decode()), part.decode("utf-8", "replace"))
    return head


def _peel_positive(git, scanner, token):
    """Follow tag objects, scanning their messages.  Returns False when the token is not a commit-ish for rev-list."""
    obj = token
    while True:
        kind = git.text("cat-file", "-t", obj).strip()
        if kind == "tag":
            sha = git.text("rev-parse", obj).strip()
            head = _check_object_text(scanner, "tag:%s" % sha[:12], git.blob(sha))
            target = re.search(rb"^object ([0-9a-f]+)$", head, re.M)
            if not target:
                raise GuardError("malformed tag object")
            obj = target.group(1).decode()
        elif kind == "commit":
            return True
        elif kind == "tree":
            scan_tree(git, scanner, obj)
            return False
        else:
            scanner.check_blob("blob:%s" % obj[:12], git.blob(git.text("rev-parse", obj).strip()))
            return False


def scan_range(git, scanner, spec):
    args, negative = [], False
    for token in spec.split():
        if token == "--not":
            negative = not negative
            args.append(token)
        elif token.startswith(("-", "^")) or negative:
            args.append(token)
        elif ".." in token:
            right = re.split(r"\.\.\.?", token, maxsplit=1)[1] or "HEAD"
            if _peel_positive(git, scanner, right):
                args.append(token)
        elif _peel_positive(git, scanner, token):
            args.append(token)
    positive, negative = False, False
    for a in args:
        if a == "--not":
            negative = not negative
        elif not a.startswith(("-", "^")) and not negative:
            positive = True
    if not positive:
        return
    seen = set()
    for commit in git.text("rev-list", *args).split():
        _check_object_text(scanner, "commit:%s" % commit[:12], git.blob(commit))
        raw = git.text("diff-tree", "-r", "-m", "--root", "--no-commit-id", "--raw", "-z", "--no-abbrev",
                       "--no-renames", "--diff-filter=ACMT", commit)
        for mode, sha, path in _parse_raw(raw):
            if (sha, path) not in seen:
                seen.add((sha, path))
                _scan_entry(git, scanner, mode, sha, path)


def _range_spec(positive, negative):
    return " ".join(list(positive) + (["--not"] + list(negative) if negative else []))


def _is_zero(sha):
    return set(sha) == {"0"}


def _advertised(git, url):
    """Commits the other side really holds, asked of the other side: a local refs/remotes/<name>/* can say anything."""
    p = git.run("ls-remote", "--", url, check=False)
    if p.returncode:
        raise GuardError("cannot list the refs of the remote, so what it already holds is unknown")
    shas = {line.split()[0] for line in p.stdout.decode("utf-8", "replace").splitlines() if line.strip()}
    return git.commits_among(sorted(shas)) if shas else []


def scan_pre_push(git, scanner, url):
    """One line per ref on stdin: local-ref local-sha remote-ref remote-sha."""
    negative = _advertised(git, url)
    scanner.allowed = read_allowed(git, negative)
    positive = []
    for line in sys.stdin.read().splitlines():
        fields = line.split()
        if len(fields) != 4:
            raise GuardError("unexpected pre-push input")
        local_ref, local_sha, remote_ref, _ = fields
        if _is_zero(local_sha):
            continue    # deleting a remote ref sends no objects
        scanner.check_ref(local_ref)
        scanner.check_ref(remote_ref)
        positive.append(local_sha)
    if positive:
        scan_range(git, scanner, _range_spec(dict.fromkeys(positive), negative))


def scan_pre_receive(git, scanner):
    """One line per ref on stdin: old-sha new-sha ref.  What this repository already holds has been through this."""
    refs = git.text("for-each-ref", "--format=%(objectname)").split()
    negative = git.commits_among(sorted(set(refs))) if refs else []
    scanner.allowed = read_allowed(git, negative)
    positive = []
    for line in sys.stdin.read().splitlines():
        fields = line.split()
        if len(fields) != 3:
            raise GuardError("unexpected pre-receive input")
        scanner.check_ref(fields[2])
        if not _is_zero(fields[1]):
            positive.append(fields[1])
    if positive:
        scan_range(git, scanner, _range_spec(dict.fromkeys(positive), negative))


def _parse_allowed(text, label):
    allowed = set()
    for number, line in enumerate(text.splitlines(), 1):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        m = re.fullmatch(r"([0-9a-f]{64})\s+(\S.*)", line)
        if not m:
            raise GuardError("%s:%d is not 'SHA256  path'" % (label, number))
        allowed.add((m.group(2), m.group(1)))
    return allowed


def read_allowed(git, revisions):
    """Listed binaries, as lines of 'SHA256  path' (the format of sha256sum), '#' starting a comment line.  A line counts
    if a revision we already trust holds it, meaning HEAD for a commit and what the remote holds for a push, or if it is in
    this clone's git directory.  The tree being checked has no say, or a block could be answered in the same commit."""
    allowed = set()
    for rev in revisions:
        p = git.run("show", "%s:%s" % (rev, ALLOWED_BINARIES), check=False)
        if p.returncode == 0:
            allowed |= _parse_allowed(p.stdout.decode("utf-8", "replace"), ALLOWED_BINARIES)
    try:
        with open(os.path.join(git.store, APPROVED_BINARIES), encoding="utf-8") as f:
            allowed |= _parse_allowed(f.read(), APPROVED_BINARIES)
    except FileNotFoundError:
        pass
    return allowed


# ------------------------------------------------------------------------------------------------ main


def report(findings, limit):
    per_key, suppressed = {}, {}
    for rule, path, line, prefix in sorted(findings, key=lambda x: (x[1], x[2], x[0])):
        key = (rule, path)
        per_key[key] = per_key.get(key, 0) + 1
        if limit and per_key[key] > limit:
            suppressed[key] = suppressed.get(key, 0) + 1
            continue
        where = "%s:%d" % (path, line) if line else path
        print("%-14s %s  %s" % (rule, where, prefix))
    for (rule, path), n in sorted(suppressed.items()):
        print("%-14s %s  ... and %d more" % (rule, path, n))


def _plain_directories():
    raw = os.environ.get("RSTEX_PRIVATE_DIRS")
    return [d for d in raw.split(os.pathsep) if d] if raw is not None else list(DEFAULT_PRIVATE_DIRS)


def load_hook_sources(git):
    """What a hook checks against: the directories recorded at install time, all of them there, none much smaller."""
    if "true" in git.config_all("rstex.nosources"):
        for d in DEFAULT_PRIVATE_DIRS:
            if os.path.isdir(os.path.expanduser(d)):
                raise GuardError("this clone was installed with --no-private-sources, but private directories exist here. "
                                 "Run tools/install-hooks.sh again.")
        print("privacy_guard: WARNING: installed without private sources; only rules f, g and h run.", file=sys.stderr)
        return Sources()
    directories = git.config_all("rstex.privatedir")
    if not directories:
        raise GuardError("no private directories are recorded for this clone. Run tools/install-hooks.sh.")
    sources = Sources()
    missing = sources.load(directories)
    if missing:
        raise GuardError("private directory %s is not there, so nothing can be compared against it" % missing[0])
    if not sources.text_files:
        raise GuardError("the private directories hold no text sources")
    for key, value in (("rstex.privategrams", len(sources.grams)), ("rstex.privatetextfiles", sources.text_files)):
        recorded = git.config_all(key)
        if recorded and value < int(recorded[-1]) // 2:
            raise GuardError("the private sources shrank by more than half since tools/install-hooks.sh recorded them. "
                             "Restore them, or run it again if that was meant.")
    return sources


def load_plain_sources(args):
    sources = Sources()
    missing = sources.load(_plain_directories())
    if args.sources_optional:
        if not sources.text_files:
            print("privacy_guard: WARNING: no private sources; only rules f, g and h run.", file=sys.stderr)
        return sources
    if missing:
        raise GuardError("private directory %s is not there. Pass --sources-optional only on a machine that never had "
                         "the private documents." % missing[0])
    if not sources.text_files:
        raise GuardError("the private directories hold no text sources")
    return sources


def install_config(git, no_sources):
    """Write what load_hook_sources will insist on into this clone's config."""
    for key in ("rstex.privatedir", "rstex.privategrams", "rstex.privatetextfiles", "rstex.nosources"):
        git.run("config", "--local", "--unset-all", key, check=False)
    if no_sources:
        for d in DEFAULT_PRIVATE_DIRS:
            if os.path.isdir(os.path.expanduser(d)):
                raise GuardError("--no-private-sources is for machines without the private documents, and they are here")
        git.run("config", "--local", "rstex.nosources", "true")
        print("install-hooks: no private sources on this machine: only rules f, g and h will run")
        return 0
    directories = [os.path.abspath(os.path.expanduser(d)) for d in _plain_directories()]
    sources = Sources()
    missing = sources.load(directories)
    if missing:
        raise GuardError("private directory %s is not there. If this machine never had the private documents, use "
                         "tools/install-hooks.sh --no-private-sources." % missing[0])
    if not sources.text_files:
        raise GuardError("the private directories hold no text sources")
    for d in directories:
        git.run("config", "--local", "--add", "rstex.privatedir", d)
    git.run("config", "--local", "rstex.privategrams", str(len(sources.grams)))
    git.run("config", "--local", "rstex.privatetextfiles", str(sources.text_files))
    print("install-hooks: recorded %d private directories, %d text sources" % (len(directories), sources.text_files))
    return 0


def run(args):
    git = Git()
    if args.install_config:
        return install_config(git, args.no_private_sources)
    hook = args.hook or args.pre_push or args.pre_receive
    sources = load_hook_sources(git) if hook else load_plain_sources(args)
    scanner = Scanner(sources, set(), parallel=True)
    for name in dict.fromkeys(args.ref_name):
        scanner.check_ref(name)
    try:
        if args.staged:
            scanner.allowed = read_allowed(git, ["HEAD"])
            scan_staged(git, scanner)
        elif args.message_file:
            scan_message_file(scanner, args.message_file)
        elif args.pre_push:
            scan_pre_push(git, scanner, args.pre_push[1])
        elif args.pre_receive:
            scan_pre_receive(git, scanner)
        elif args.range:
            scanner.allowed = read_allowed(git, _trusted_revisions(args.range))
            scan_range(git, scanner, args.range)
        elif args.tree:
            scanner.allowed = read_allowed(git, [args.tree, "HEAD"])
            scan_tree(git, scanner, args.tree)
        else:
            scanner.allowed = read_allowed(git, ["HEAD"])
            scan_worktree(git, scanner)
        scanner.finish()
    finally:
        scanner.stop_pool()
        git.close()

    report(scanner.findings, args.report_limit)
    if any(rule[0] in "gh" for rule, _, _, _ in scanner.findings):
        print("privacy_guard: a binary, encoded data or engine output needs a person to look at it before it can be listed; "
              "see docs/DESIGN.md, section 13. Agents ask, they do not add the line.", file=sys.stderr)
    print("privacy_guard: %d blobs, %d messages, %d bytes; sources: %d text files, %d file hashes; %d findings"
          % (scanner.blobs, scanner.messages, scanner.bytes, sources.text_files, len(sources.hashes),
             len(scanner.findings)), file=sys.stderr)
    return EXIT_FINDINGS if scanner.findings else 0


def _trusted_revisions(spec):
    """The excluded side of a range: what is already out there."""
    revisions, negative = [], False
    for token in spec.split():
        if token == "--not":
            negative = not negative
        elif token.startswith("^"):
            revisions.append(token[1:])
        elif negative and not token.startswith("-"):
            revisions.append(token)
        elif ".." in token:
            left = re.split(r"\.\.\.?", token, maxsplit=1)[0]
            revisions.append(left or "HEAD")
    return revisions


def scan_message_file(scanner, path):
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            raw = f.read()
    except OSError as e:
        raise GuardError("cannot read the commit message: %s" % e.strerror)
    kept = []
    for line in raw.split("\n"):
        if line.startswith("# ------------------------ >8"):
            break
        if not line.startswith("#"):
            kept.append(line)
    scanner.check_message("commit message", "\n".join(kept))


def main():
    # What gets sent is the real object; a replace ref must not show this scan something else.
    os.environ["GIT_NO_REPLACE_OBJECTS"] = "1"
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--staged", action="store_true", help="scan what the index would add to the next commit")
    mode.add_argument("--range", metavar="A..B", help="scan every blob introduced in a revision range, and messages")
    mode.add_argument("--tree", metavar="REF", help="scan every blob of a tree")
    mode.add_argument("--worktree", action="store_true", help="scan tracked and untracked non-ignored files")
    mode.add_argument("--message-file", metavar="FILE", help="scan one commit message")
    mode.add_argument("--pre-push", nargs=2, metavar=("REMOTE", "URL"), help="the pre-push hook: ref lines on stdin")
    mode.add_argument("--pre-receive", action="store_true", help="the gate's pre-receive hook: ref lines on stdin")
    mode.add_argument("--install-config", action="store_true", help="record the private directories for the hooks")
    parser.add_argument("--hook", action="store_true", help="private directories from this clone's config, fail closed")
    parser.add_argument("--sources-optional", action="store_true", help="only for machines without the private documents")
    parser.add_argument("--no-private-sources", action="store_true", help="with --install-config")
    parser.add_argument("--ref-name", action="append", default=[], metavar="NAME",
                        help="also apply rule f to this branch or tag name")
    parser.add_argument("--report-limit", type=int, default=5, metavar="N",
                        help="findings shown per rule and path, 0 for all (default 5)")
    args = parser.parse_args()
    try:
        return run(args)
    except GuardError as e:
        print("privacy_guard: ERROR: %s" % e, file=sys.stderr)
    except Exception as e:
        # Exception text can quote its input, so only the type and the place are printed.
        tb = e.__traceback__
        while tb.tb_next:
            tb = tb.tb_next
        print("privacy_guard: ERROR: internal failure: %s at line %d" % (type(e).__name__, tb.tb_lineno), file=sys.stderr)
    return EXIT_ERROR


if __name__ == "__main__":
    sys.exit(main())
