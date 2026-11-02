#!/usr/bin/env -S python3 -I
"""privacy_guard.py: keep the user's private documents out of everything this repository publishes.

    python3 -I tools/privacy_guard.py --staged                  index, what a commit would add
    python3 -I tools/privacy_guard.py --range A..B              every blob introduced in A..B, plus commit messages
    python3 -I tools/privacy_guard.py --tree REF                every blob of a tree
    python3 -I tools/privacy_guard.py --worktree                tracked plus untracked non-ignored files

    --range takes anything git rev-list takes, for example 'SHA --not --remotes=origin'.
    --ref-name NAME     also apply rule f to a branch or tag name (repeatable)
    --report-limit N    findings shown per rule and path, default 5, 0 for all.  The cap is there so that a
                        whole copied document does not get printed back 40 characters at a time.

Private sources are read at run time from the directories in RSTEX_PRIVATE_DIRS (separated by ':'), default the
thesis directory and ~/.local/share/rstex-private, skipping .git.  Nothing derived from them is written anywhere:
every set below lives in this process and dies with it.  This file holds no private content.

Rules.  A blob is scanned as a whole, so a match anywhere in a modified file counts.
    a  a non-boilerplate private line, whitespace-insensitive.  Short lines must match a whole line, long ones
       (>= LONG_LINE_CHARS) match anywhere, so they are also found inside a string or a reflowed paragraph.
    b  WORD_RUN or more private prose words in order, across line breaks, comments and whitespace.
    c  a tikzcd or equation body, whitespace-insensitive, or a tikzcd whose structure equals a private one.
    d  a distinctive title, heading, label, bib key, tikzcd alias, user macro or private file name.
    e  a blob with the SHA-256 of a private file.
    f  a path or ref name containing the thesis words or '.local.', and thesis paths mentioned in text.  A local
       manifest's bare name in text is fine (the corpus runner has to name its own); the file itself is what blocks.
    g  any binary not listed with path and SHA-256 in tools/privacy-allowed-binaries.txt, and long
       base64-like, hex-like or high-entropy runs in text files (the same list exempts a file from those).
Rules f and g need no private sources; on a machine without them only those two run, with a warning.

Report lines: rule, path:line, then at most 40 characters of the scanned text at the match.
Exit status: 0 clean, 1 match, 2 internal error (hooks treat that as a stop too).
"""
import argparse
import bisect
import hashlib
import math
import os
import re
import subprocess
import sys
import unicodedata

# Written in two pieces because rule f has no exemption for this file.
DEFAULT_PRIVATE_DIRS = ("~/" + "Master" "arbeit", "~/.local/share/rstex-private")
ALLOWED_BINARIES = "tools/privacy-allowed-binaries.txt"
TEXT_EXTENSIONS = (".tex", ".bib", ".bbl", ".aux", ".toc", ".out", ".sty")

PREFIX_CHARS = 40
LINE_MIN_CHARS = 24       # rule a: length of a line with all whitespace removed
LINE_MIN_LETTERS = 12     # rule a: letters left after structural commands and control-sequence names are gone
LONG_LINE_CHARS = 48      # rule a: from here on a line also matches as a substring
WORD_RUN = 8              # rule b
DIAGRAM_MIN_BODY = 64     # rule c: tikzcd body, whitespace removed; a bare square is shorter
EQUATION_MIN_BODY = 40    # rule c: equation-like body, whitespace removed
FP_MIN_ARROWS = 4         # rule c: structural fingerprints apply from here
FP_MIN_TOKENS = 16        # rule c: cell and label tokens for the relettered form: twice a labelled square of letters
FP_MIN_SHAPE_SCORE = 8    # rule c: complexity needed for the shape-only form (a plain square scores 4)
TITLE_MIN_WORDS = 4       # rule d: non-generic words in a title; three-word headings are stock terms of the field
NAME_MIN_CHARS = 5        # rule d: label, key or alias stem
MACRO_MIN_CHARS = 7       # rule d: user macro name
HEX_RUN = 129             # rule g: one more than SHA-512
BASE64_RUN = 120          # rule g
WRAPPED_LINE = 40         # rule g: a line of an encoded block
ENCODED_BITS = 4.5        # rule g: entropy per character of a base64-like run; true base64 is close to 6
DUMP_BYTES = 300          # rule g: separated hex or decimal bytes; a 256-entry table stays below this
DENSE_RUN = 400           # rule g: non-whitespace run ...
DENSE_BITS = 5.0          # rule g: ... with at least this Shannon entropy per character
MAX_SCAN_BYTES = 64 << 20  # rule g: larger blobs are not scanned, so they must be listed


class GuardError(Exception):
    """Anything that stops the guard from giving a real answer: exit 2, and the hooks stop."""


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
_MATH_ENV = re.compile(r"\\begin\{(" + _MATH_ENVS + r")\}.*?\\end\{\1\}", re.S)
_INLINE = re.compile(r"(?<![\\$])\$(?:[^$\\]|\\.){1,1500}?\$", re.S)
_BODY_ENV = re.compile(r"\\begin\{(tikzcd|cd|CD|equation\*?|align\*?|alignat\*?|flalign\*?|gather\*?|multline\*?"
                       r"|eqnarray\*?|displaymath)\}(.*?)\\end\{\1\}", re.S)
_BODY_DISPLAY = re.compile(r"(?<!\\)\\\[(.*?)(?<!\\)\\\]|\$\$(.*?)\$\$", re.S)
_TIKZCD = re.compile(r"\\begin\{tikzcd\}(.*?)\\end\{tikzcd\}", re.S)

_CS = re.compile(r"\\(?:([A-Za-z@]+)\*?|(.))", re.S)
_ACCENT_BRACED = re.compile(r"""\\(["'`^~=.])\{(\w)\}""")
_ACCENT_MARKS = frozenset("\"'`^~=.-")
_COMMENT = re.compile(r"(?<!\\)((?:\\\\)*)%([^\n]*)")
_COMMENT_MARK = re.compile(r"(?<!\\)((?:\\\\)*)%")
_LEADING_MARK = re.compile(r"^\s*(?:%+|//+|#+|>+|\*+|;+|--+)")
_WORD = re.compile(r"[^\W\d_](?:[^\W\d_]|\x00)*")
_TEX_MARK = re.compile(r"[\\${}_^&]")
_OPTION_LINE = re.compile(r"^[A-Za-z@][\w@ -]*=\s*\{?[^\s{}]*\}?\s*,?\s*(?:%.*)?$")
_BIB_ENTRY = re.compile(r"^\s*@\w+\s*\{[^,\s]*,?\s*$")
_BIB_METADATA = re.compile(r"^\s*(?:publisher|address|location|year|date|month|volume|number|pages|issn|isbn|doi|url"
                           r"|urldate|journal|journaltitle|booktitle|series|edition|language|langid|eprint"
                           r"|eprinttype|archiveprefix|primaryclass|mrclass|mrnumber|zbl|keywords|timestamp|file"
                           r"|biburl|bibsource)\s*=", re.I)


def _blank(s):
    """Same length, same line breaks, nothing else: offsets into the blanked text stay valid in the original."""
    return re.sub(r"[^\n]", " ", s)


def _blank_match(m):
    return _blank(m.group())


def _blank_between(text, opener, closer):
    """Blank from each unescaped opener to the next closer.  str.find keeps this linear on files that are not TeX."""
    parts, i = [], 0
    while True:
        a = text.find(opener, i)
        while a > 0 and text[a - 1] == "\\":
            a = text.find(opener, a + 1)
        b = text.find(closer, a + len(opener)) if a >= 0 else -1
        if b < 0:
            parts.append(text[i:])
            return "".join(parts)
        b += len(closer)
        parts.append(text[i:a])
        parts.append(_blank(text[a:b]))
        i = b


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
    text = _MATH_ENV.sub(_blank_match, text)
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
    """Prose words of a blanked text with their offsets.  One-letter tokens are out, on both sides of a comparison:
    they are mostly math, so '$X$' in the source and 'X' in a copy agree."""
    words, offsets = [], []
    for m in _WORD.finditer(blanked):
        w = m.group().replace("\0", "")
        if len(w) < 2:
            continue
        if not w.isascii():
            w = "".join(c for c in unicodedata.normalize("NFKD", w) if not unicodedata.combining(c))
        words.append(w.casefold())
        offsets.append(m.start())
    return words, offsets


def squeeze(s):
    return "".join(s.split())


def code_of(line):
    return _COMMENT.sub(lambda m: m.group(1), line)


def unmarked(line):
    """A line with a leading comment or quote marker gone and % dropped, comment text kept."""
    return _COMMENT_MARK.sub(lambda m: m.group(1), _LEADING_MARK.sub("", line, count=1))


def line_variants(line):
    return {squeeze(line), squeeze(code_of(line)), squeeze(unmarked(line))}


def is_boilerplate(line, ext):
    """True when a line carries no document text.  The test is structural: remove commands in STRUCTURAL with their
    arguments and the names of all other control sequences, and count the letters that remain."""
    s = line.strip()
    if ext == ".bib" and (_BIB_ENTRY.match(s) or _BIB_METADATA.match(s)):
        return True
    if _OPTION_LINE.match(s):
        return True
    if not _TEX_MARK.search(s) and len(words_of(s)[0]) < WORD_RUN:
        return True     # plain prose this short is rule b's call, and rule b says it is too short to mean anything
    s = _LEADING_MARK.sub("", s, count=1)
    letters, i = 0, 0
    while True:
        m = _CS.search(s, i)
        if not m:
            letters += sum(c.isalpha() for c in s[i:])
            return letters < LINE_MIN_LETTERS
        letters += sum(c.isalpha() for c in s[i:m.start()])
        name = m.group(1)
        i = _skip_args(s, m.end(), name in DEF_LIKE) if name in STRUCTURAL else m.end()


# ------------------------------------------------------------------------------------------------ diagrams

_ARROW = re.compile(r"\\(?:arrow|ar)(?![A-Za-z@])")
_CELL_TOKEN = re.compile(r"\\[A-Za-z@]+|\\.|[^\W\d_]|\d+|\S", re.S)
_CELL_NODE_OPTS = re.compile(r"\|\[.*?\]\|", re.S)


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


def equation_bodies(text):
    """(squeezed body, is_diagram) for every tikzcd and equation-like environment of a comment-free text."""
    for m in _BODY_ENV.finditer(text):
        body = m.group(2).lstrip()
        if body.startswith("["):
            body = body[_skip_group(body, 0, "[", "]"):]
        yield squeeze(body), m.group(1).lower() in ("tikzcd", "cd")
    for m in _BODY_DISPLAY.finditer(text):
        yield squeeze(m.group(1) or m.group(2) or ""), False


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

_NAME_TOKEN = re.compile(r"[^\s{}\[\]()<>,;\"'`=|\\/$&%~^*#!?]+")
_MACRO_USE = re.compile(r"\\([A-Za-z@]+)")
_LABEL = re.compile(r"\\(?:label|newlabel)\s*\{([^{}\s]+)\}")
_CITE = re.compile(r"\\(?:cite[a-zA-Z]*|nocite|citation|bibcite|bibitem)\*?\s*(?:\[[^\]]*\]\s*)*\{([^{}]+)\}")
_BIB_KEY = re.compile(r"^\s*@(\w+)\s*\{\s*([^,\s{}]+)\s*,", re.M)
_ALIAS = re.compile(r"\b(?:alias|name)\s*=\s*([A-Za-z][\w-]*)")
_HEADING = re.compile(r"\\(?:title|subtitle|part|chapter|section|subsection|subsubsection|paragraph|subparagraph)"
                      r"\*?\s*(?:\[([^\]]*)\])?\s*(?=\{)")
_BIB_TITLE = re.compile(r"^\s*(?:title|subtitle|booktitle)\s*=\s*(?=[{\"])", re.M | re.I)
_TOC_LINE = re.compile(r"\\contentsline\s*\{[^{}]*\}\s*(?=\{)")
_BOOKMARK = re.compile(r"\\BOOKMARK\s*\[[^\]]*\]\s*\[[^\]]*\]\s*\{[^{}]*\}\s*(?=\{)")
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


def distinctive_file(name):
    stem, ext = os.path.splitext(name)
    return bool(ext) and len(stem) >= NAME_MIN_CHARS and re.search(r"[-_\d]", stem) is not None


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
        self.literals = {}          # first LITERAL_KEY characters -> [(literal, kind)]
        self.grams = set()
        self.gram_first = set()
        self.titles = {}            # (first word, second word) -> [word tuple]
        self.names = {}             # token -> kind
        self.word_keys = set()      # bib keys made of words only, which match only where a key is expected
        self.macros = set()
        self.shapes = set()
        self.relettered = set()

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
        return missing

    def _walk(self, root):
        def fail(err):
            raise GuardError("cannot read private directory: %s" % err)
        for dirpath, dirnames, filenames in os.walk(root, onerror=fail):
            dirnames[:] = [x for x in dirnames if x != ".git"]
            for name in filenames:
                path = os.path.join(dirpath, name)
                if not os.path.isfile(path):
                    continue
                self._add_file(path, name)

    def _add_file(self, path, name):
        digest = hashlib.sha256()
        keep = os.path.splitext(name)[1].lower() in TEXT_EXTENSIONS
        chunks = []
        try:
            with open(path, "rb") as f:
                while True:
                    chunk = f.read(1 << 20)
                    if not chunk:
                        break
                    digest.update(chunk)
                    if keep:
                        chunks.append(chunk)
        except OSError as e:
            raise GuardError("cannot read private file: %s" % e.strerror)
        if os.path.getsize(path) > 0:
            self.hashes.add(digest.hexdigest())
        if distinctive_file(name):
            self.names[name] = "file"
            stem = os.path.splitext(name)[0]
            if re.search(r"[-_]", stem) and len(stem) >= 6:
                self.names[stem] = "file"
        if keep:
            self.text_files += 1
            self._add_text(os.path.splitext(name)[1].lower(), b"".join(chunks).decode("utf-8", "replace"))

    def _add_text(self, ext, text):
        if ext in (".tex", ".bbl", ".sty", ".bib"):
            for line in text.splitlines():
                for v, source in zip((squeeze(line), squeeze(code_of(line)), squeeze(unmarked(line))),
                                     (line, code_of(line), unmarked(line))):
                    if len(v) >= LINE_MIN_CHARS and not is_boilerplate(source, ext):
                        self.exact_lines.add(v)
                        if len(v) >= LONG_LINE_CHARS:
                            self._add_literal(v, "a")
        code = strip_comments(text)
        if ext in (".tex", ".bbl", ".sty"):
            for stream in (code, open_comments(text)):
                self._add_grams(words_of(tex_prose(stream))[0])
        if ext in (".tex", ".sty"):
            for body, is_diagram in equation_bodies(code):
                if len(body) >= (DIAGRAM_MIN_BODY if is_diagram else EQUATION_MIN_BODY):
                    self._add_literal(body, "c/body")
            for m in _TIKZCD.finditer(code):
                shape, relettered = diagram_keys(m.group(1))
                if shape:
                    self.shapes.add(shape)
                if relettered:
                    self.relettered.add(relettered)
            for m in _NEW_MACRO.finditer(code):
                name = m.group(1) or m.group(2)
                if len(name) >= MACRO_MIN_CHARS:
                    self.macros.add(name)
            for m in _ALIAS.finditer(code):
                if distinctive_name(m.group(1)):
                    self.names.setdefault(m.group(1), "alias")
        self._add_names(ext, code)

    def _add_literal(self, literal, kind):
        self.literals.setdefault(literal[:self.LITERAL_KEY], []).append((literal, kind))

    def _add_grams(self, words):
        grams = self.grams
        for i in range(len(words) - WORD_RUN + 1):
            grams.add(tuple(words[i:i + WORD_RUN]))
            self.gram_first.add(words[i])

    def _add_title(self, tex):
        """Words are counted between spaces, so 'quasi-coherent sheaves' is two words and a term of the field."""
        blanked = tex_prose(re.sub(r"\\\d{3}", "", tex))
        content = 0
        for chunk in blanked.split():
            if any(w not in GENERIC_WORDS for w in words_of(chunk)[0]):
                content += 1
        if content >= TITLE_MIN_WORDS:
            words = tuple(words_of(blanked)[0])
            self.titles.setdefault(words[:2], []).append(words)

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


# ------------------------------------------------------------------------------------------------ rules f and g

_BENIGN_WORDS = re.compile(r"(?:paren|hypo|photosyn|syn|anti|pros|meta|epen|dia)thesis")
_PRIVATE_PATH = re.compile(r"master[\s_-]?arbeit|thesis|\.local\.")
_MASTER_TEXT = re.compile(r"master[\s_-]?arbeit", re.I)
# In text the word must stand alone or follow a common owner: public files such as a university's class named
# <initials>thesis can be mentioned, and the corpus manifests do.  As a real path they are still blocked.
_STANDALONE_WORD = re.compile(r"(?<![a-z])(?:my|masters?|msc|ma|phd|bachelors?|ba|diplom)?thesis")
_PATH_TOKEN = re.compile(r"[\w.~$/+\-]+")
_TOKEN_EXT = re.compile(r"\.(?:[a-z0-9]{1,8}|[A-Z0-9]{1,8})$")
_TOKEN_GLUED = re.compile(r"(?:\d[-_]?|_)thesis|thesis(?:[-_]?\d|_)")

_MAGIC = (b"%PDF-", b"%!PS", b"\x89PNG", b"\xff\xd8\xff", b"GIF8", b"PK\x03\x04", b"PK\x05\x06", b"\x1f\x8b", b"BZh",
          b"\xfd7zXZ", b"7z\xbc\xaf", b"\x28\xb5\x2f\xfd", b"II*\0", b"MM\0*", b"RIFF", b"Rar!", b"\x04\x22\x4d\x18")
_CONTROL = re.compile(rb"[\x00-\x08\x0b\x0e-\x1f\x7f]")
_HEX_RUN = re.compile(r"(?<![0-9A-Fa-f])[0-9A-Fa-f]{%d,}" % HEX_RUN)
_B64_RUN = re.compile(r"[A-Za-z0-9+/]{%d,}={0,2}|[A-Za-z0-9_-]{%d,}={0,2}" % (BASE64_RUN, BASE64_RUN))
_B64_ANY = re.compile(r"[A-Za-z0-9+/]{%d,}={0,2}|[A-Za-z0-9_-]{%d,}={0,2}" % (WRAPPED_LINE, WRAPPED_LINE))
_HEX_LINE = re.compile(r"^\s*[0-9A-Fa-f]{%d,}\s*$" % WRAPPED_LINE)
_DUMP = re.compile(r"(?:(?<![\w.])(?:0[xX])?(?:[0-9a-fA-F]{2}){1,8}(?![\w.])[\s,]*){64,}")
_DECIMAL = re.compile(r"(?:(?<![\w.])\d{1,3}(?![\w.])[\s,]+){%d,}" % DUMP_BYTES)
_DENSE = re.compile(r"\S{%d,}" % DENSE_RUN)


def path_is_private(path):
    low = _BENIGN_WORDS.sub("", path.lower())
    return _PRIVATE_PATH.search(low) is not None


def is_binary(data):
    return data.startswith(_MAGIC) or _CONTROL.search(data) is not None


def _mixed(s):
    """Upper case, lower case and digits, spread the way encoded data spreads them: paths and long names are far
    below ENCODED_BITS even when they use the right alphabet."""
    return (any(c.isupper() for c in s) and any(c.islower() for c in s) and any(c.isdigit() for c in s)
            and _entropy(s) >= ENCODED_BITS)


def _entropy(s):
    counts = {}
    for c in s:
        counts[c] = counts.get(c, 0) + 1
    n = len(s)
    return -sum(k / n * math.log2(k / n) for k in counts.values())


def text_path_mentions(text):
    """Offsets of path-like mentions.  The plain word 'thesis' stays allowed: it is a benchmark name in the docs."""
    for m in _MASTER_TEXT.finditer(text):
        yield m.start()
    for m in _PATH_TOKEN.finditer(text):
        tok = m.group().rstrip(".,:;-")
        low = _BENIGN_WORDS.sub("", tok.lower())
        if _STANDALONE_WORD.search(low):
            rooted = tok.startswith(("/", "~/", "./", "../", "$")) and "/" in tok
            if _TOKEN_EXT.search(tok) or rooted or _TOKEN_GLUED.search(low):
                yield m.start()


def encoded_runs(text):
    """Offsets of runs that look like encoded data."""
    for m in _HEX_RUN.finditer(text):
        yield m.start()
    for m in _B64_RUN.finditer(text):
        if _mixed(m.group()):
            yield m.start()
    for m in _DUMP.finditer(text):
        run = m.group()
        digits = sum(len(t) - (2 if t[:2] in ("0x", "0X") else 0) for t in re.split(r"[\s,]+", run) if t)
        if digits >= 2 * DUMP_BYTES and ("x" in run or "X" in run or re.search(r"[a-fA-F]", run)):
            yield m.start()
    for m in _DECIMAL.finditer(text):
        if all(int(t) < 256 for t in re.split(r"[\s,]+", m.group()) if t):
            yield m.start()
    for m in _DENSE.finditer(text):
        if _entropy(m.group()) >= DENSE_BITS:
            yield m.start()
    # Blocks wrapped over several lines, as base64 and xxd -p produce them.
    pos, run_start, run_len, run_text, hex_run = 0, 0, 0, [], 0
    for line in text.split("\n"):
        hits = _B64_ANY.findall(line)
        if hits and sum(map(len, hits)) >= WRAPPED_LINE:
            if not run_len:
                run_start = pos
            run_len += sum(map(len, hits))
            run_text.extend(hits)
        else:
            if run_len >= BASE64_RUN and _mixed("".join(run_text)):
                yield run_start
            run_len, run_text = 0, []
        if _HEX_LINE.match(line):
            hex_run += 1
            if hex_run == 3:
                yield pos
        else:
            hex_run = 0
        pos += len(line) + 1
    if run_len >= BASE64_RUN and _mixed("".join(run_text)):
        yield run_start


# ------------------------------------------------------------------------------------------------ scanning


def _printable_prefix(s):
    s = "".join(c if c.isprintable() else " " for c in s).strip()
    return s[:PREFIX_CHARS]


class Scanner:
    def __init__(self, sources, allowed):
        self.src = sources
        self.allowed = allowed
        self.findings = []      # (rule, path, line, prefix)
        self.seen_blobs = {}    # content digest -> findings relative to the blob
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

    def check_ref(self, name):
        if path_is_private(name):
            self.add("f/ref", name, 0, name)

    def check_blob(self, path, data):
        """Rules e, g and all text rules on one blob.  Text results are kept per digest, the allowlist is per path."""
        self.blobs += 1
        self.bytes += len(data)
        digest = hashlib.sha256(data).hexdigest()
        listed = (path, digest) in self.allowed
        if digest in self.src.hashes:
            self.add("e", path, 0, "identical to a private file")
        too_big = len(data) > MAX_SCAN_BYTES
        if too_big or is_binary(data):
            if not listed:
                self.add("g/binary", path, 0, "too large to scan" if too_big else "binary, not listed")
            return
        try:
            text = data.decode("utf-8")
        except UnicodeDecodeError:
            if not listed:
                self.add("g/binary", path, 0, "not UTF-8, not listed")
            return
        cached = self.seen_blobs.get(digest)
        if cached is None:
            cached = self.seen_blobs[digest] = self.text_rules(text, path)
        for rule, line, prefix in cached:
            if rule == "g/encoded" and listed:
                continue
            self.add(rule, path, line, prefix)

    def check_message(self, label, text):
        self.messages += 1
        for rule, line, prefix in self.text_rules(text, label):
            self.add(rule, label, line, prefix)

    def text_rules(self, text, path):
        hits = {}
        lines = text.split("\n")
        starts = [0]
        for line in lines[:-1]:
            starts.append(starts[-1] + len(line) + 1)

        def at(offset, rule):
            n = bisect.bisect_right(starts, offset) - 1
            end = text.find("\n", offset)
            hits.setdefault((rule, n + 1), text[offset:end if end >= 0 else None][:PREFIX_CHARS + 8])

        for off in text_path_mentions(text):
            at(off, "f/text")
        for off in encoded_runs(text):
            at(off, "g/encoded")
        if self.src.available:
            self._private_rules(text, lines, starts, at)
            decoded = _decode_escapes(text, path)
            if decoded is not None:
                self._embedded_rules(decoded, hits)
        return [(rule, line, prefix) for (rule, line), prefix in sorted(hits.items(), key=lambda kv: (kv[0][1], kv[0][0]))]

    def _embedded_rules(self, decoded, hits):
        """Text held in a JSON or string-literal form: lines no longer map back, so everything lands on line 1."""
        lines = decoded.split("\n")
        starts = [0]
        for line in lines[:-1]:
            starts.append(starts[-1] + len(line) + 1)

        def at(offset, rule):
            end = decoded.find("\n", offset)
            hits.setdefault((rule, 1), decoded[offset:end if end >= 0 else None][:PREFIX_CHARS + 8])

        self._private_rules(decoded, lines, starts, at)

    def _private_rules(self, text, lines, starts, at):
        src = self.src
        # a: whole lines
        exact = src.exact_lines
        for n, line in enumerate(lines):
            if len(line) >= LINE_MIN_CHARS and not exact.isdisjoint(line_variants(line)):
                at(starts[n] + len(line) - len(line.lstrip()), "a")
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
            self._gram_scan(words, offsets, at)
            self._title_scan(words, offsets, at)
        # c: structure
        if src.shapes or src.relettered:
            code = strip_comments(text)
            for m in _TIKZCD.finditer(code):
                shape, relettered = diagram_keys(m.group(1))
                if relettered in src.relettered and relettered is not None:
                    at(m.start(), "c/relettered")
                elif shape in src.shapes and shape is not None:
                    at(m.start(), "c/shape")
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

    def _gram_scan(self, words, offsets, at):
        grams, first = self.src.grams, self.src.gram_first
        i, last = 0, len(words) - WORD_RUN
        while i <= last:
            if words[i] in first and tuple(words[i:i + WORD_RUN]) in grams:
                at(offsets[i], "b")
                j = i + 1
                while j <= last and tuple(words[j:j + WORD_RUN]) in grams:
                    j += 1
                i = j + WORD_RUN - 1
            else:
                i += 1

    def _title_scan(self, words, offsets, at):
        titles = self.src.titles
        if not titles:
            return
        for i in range(len(words) - 1):
            bucket = titles.get((words[i], words[i + 1]))
            if bucket:
                for title in bucket:
                    if tuple(words[i:i + len(title)]) == title:
                        at(offsets[i], "d/title")


def _decode_escapes(text, path):
    """A report or string literal holds TeX with doubled backslashes and \\n for newline; read it once decoded too."""
    if not (path.endswith((".json", ".jsonl")) or "\\\\begin" in text or "\\\\section" in text):
        return None
    return re.sub(r"\\(.)", lambda m: {"n": "\n", "t": " ", "r": " "}.get(m.group(1), m.group(1)), text, flags=re.S)


# ------------------------------------------------------------------------------------------------ git access


class Git:
    def __init__(self):
        self.root = self.text("rev-parse", "--show-toplevel").strip()
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


def _peel_positive(git, scanner, token):
    """Follow tag objects, scanning their messages.  Returns False when the token is not a commit-ish for rev-list."""
    obj = token
    while True:
        kind = git.text("cat-file", "-t", obj).strip()
        if kind == "tag":
            body = git.blob(git.text("rev-parse", obj).strip())
            head, _, message = body.partition(b"\n\n")
            scanner.check_message("tag:%s" % git.text("rev-parse", obj).strip()[:12], message.decode("utf-8", "replace"))
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
    if not any(not a.startswith(("-", "^")) for a in args):
        return
    seen = set()
    for commit in git.text("rev-list", *args).split():
        body = git.blob(commit)
        message = body.partition(b"\n\n")[2].decode("utf-8", "replace")
        scanner.check_message("commit:%s" % commit[:12], message)
        raw = git.text("diff-tree", "-r", "-m", "--root", "--no-commit-id", "--raw", "-z", "--no-abbrev",
                       "--no-renames", "--diff-filter=ACMT", commit)
        for mode, sha, path in _parse_raw(raw):
            if (sha, path) not in seen:
                seen.add((sha, path))
                _scan_entry(git, scanner, mode, sha, path)


def read_allowed(root):
    """Lines of 'SHA256  path', the format of sha256sum; '#' starts a comment line.  A malformed line is an error."""
    allowed = set()
    try:
        with open(os.path.join(root, ALLOWED_BINARIES), encoding="utf-8") as f:
            for number, line in enumerate(f, 1):
                line = line.strip()
                if not line or line.startswith("#"):
                    continue
                m = re.fullmatch(r"([0-9a-f]{64})\s+(\S.*)", line)
                if not m:
                    raise GuardError("%s:%d is not 'SHA256  path'" % (ALLOWED_BINARIES, number))
                allowed.add((m.group(2), m.group(1)))
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


def run(args):
    raw_dirs = os.environ.get("RSTEX_PRIVATE_DIRS")
    directories = [d for d in raw_dirs.split(os.pathsep) if d] if raw_dirs is not None else list(DEFAULT_PRIVATE_DIRS)
    sources = Sources()
    missing = sources.load(directories)
    for d in missing:
        print("privacy_guard: WARNING: private directory %s does not exist" % d, file=sys.stderr)
    if not sources.available:
        print("privacy_guard: WARNING: no private sources on this machine. Only rules f and g run; content rules "
              "a-e are NOT being checked.", file=sys.stderr)
    elif not sources.text_files:
        print("privacy_guard: WARNING: the private directories hold no text sources; rules a-d have nothing to "
              "compare against.", file=sys.stderr)

    git = Git()
    scanner = Scanner(sources, read_allowed(git.root))
    for name in dict.fromkeys(args.ref_name):
        scanner.check_ref(name)
    try:
        if args.staged:
            scan_staged(git, scanner)
        elif args.range:
            scan_range(git, scanner, args.range)
        elif args.tree:
            scan_tree(git, scanner, args.tree)
        else:
            scan_worktree(git, scanner)
    finally:
        git.close()

    report(scanner.findings, args.report_limit)
    print("privacy_guard: %d blobs, %d messages, %d bytes; sources: %d text files, %d file hashes; %d findings"
          % (scanner.blobs, scanner.messages, scanner.bytes, sources.text_files, len(sources.hashes),
             len(scanner.findings)), file=sys.stderr)
    return 1 if scanner.findings else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--staged", action="store_true", help="scan what the index would add to the next commit")
    mode.add_argument("--range", metavar="A..B", help="scan every blob introduced in a revision range, and messages")
    mode.add_argument("--tree", metavar="REF", help="scan every blob of a tree")
    mode.add_argument("--worktree", action="store_true", help="scan tracked and untracked non-ignored files")
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
    return 2


if __name__ == "__main__":
    sys.exit(main())
