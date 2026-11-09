#!/usr/bin/env python3
"""Lookup tool over the merged WEB (refs/merged/pdftex-merged.web).

Run with python3 -I.  Sections are numbered exactly as weave numbers them in
the merged file (checked against weave's output by `web.py check-weave FILE.tex`).

  web.py sec N [M]         print section N (or the range N..M)
  web.py part NAME         list parts whose title or number matches, with section ranges
  web.py parts             list every part
  web.py proc NAME         find a procedure or function: its section and signature
  web.py procs [RANGE]     list every procedure, optionally only sections A-B
  web.py mod "prefix"      find a named module: definition and use sections
  web.py macro NAME        print the @d macro(s) with that name
  web.py grep RE           show matching lines with section numbers
  web.py uses NAME         sections whose Pascal or definition part use identifier NAME
  web.py tangle NAME       a procedure with every reachable module inlined, marked {§N}; macros stay
  web.py landing N [M]     the procedures whose body contains section N

Library: other tools import this file (see the sys.path line in coverage.py).
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_WEB = os.path.join(ROOT, "refs", "merged", "pdftex-merged.web")


class Section:
    __slots__ = (
        "num", "starred", "title", "line", "text", "tex", "defs", "pascal",
        "module", "is_def_of", "part",
    )

    def __init__(self, num, starred, line, text):
        self.num = num
        self.starred = starred
        self.line = line
        self.text = text
        self.title = None
        self.tex = ""
        self.defs = ""
        self.pascal = ""
        self.module = None  # full or abbreviated module name for @<...@>=
        self.is_def_of = None
        self.part = None


class Macro:
    __slots__ = ("name", "kind", "param", "body", "sec", "raw")

    def __init__(self, name, kind, param, body, sec, raw):
        self.name = name
        self.kind = kind  # '=' numeric, '==' text
        self.param = param  # True if name(#)
        self.body = body  # comment-free, whitespace-normalised
        self.sec = sec
        self.raw = raw


class Proc:
    __slots__ = ("name", "kind", "sec", "params", "result", "header", "forward")

    def __init__(self, name, kind, sec, params, result, header, forward):
        self.name = name
        self.kind = kind  # 'procedure' | 'function'
        self.sec = sec
        self.params = params  # list of (var?, name, type)
        self.result = result
        self.header = header
        self.forward = forward


def strip_comments(s):
    """Remove Pascal {...} comments, which WEB allows to nest, from Pascal text.

    Knows about Pascal strings ('...') and WEB strings ("..."), and `@{`/`@}`
    meta-comment brackets, which it leaves out as well.
    """
    out = []
    i = 0
    n = len(s)
    depth = 0
    while i < n:
        c = s[i]
        if depth:
            if c == "\\":
                i += 2
                continue
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
                if depth == 0:
                    out.append(" ")
            i += 1
            continue
        if c == "{":
            depth = 1
            i += 1
            continue
        if c in "\"'":
            j = i + 1
            while j < n:
                if s[j] == c:
                    if j + 1 < n and s[j + 1] == c:
                        j += 2
                        continue
                    break
                if c == '"' and s[j] == "@" and j + 1 < n and s[j + 1] == "@":
                    j += 2
                    continue
                if s[j] == "\n":
                    break
                j += 1
            out.append(s[i : j + 1])
            i = j + 1
            continue
        if c == "@" and i + 1 < n:
            d = s[i + 1]
            if d in "{}":
                i += 2
                continue
            if d == "<":
                j = s.find("@>", i + 2)
                if j < 0:
                    j = n - 2
                out.append(s[i : j + 2])
                i = j + 2
                continue
            if d in ".:^t=" and d != "=":
                # index entries and TeX annotations end at @>
                j = s.find("@>", i + 2)
                if j >= 0:
                    i = j + 2
                    continue
            if d == "=":
                j = s.find("@>", i + 2)
                if j >= 0:
                    i = j + 2
                    continue
            if d in "!?/|#+;,&\\":
                out.append(" ")
                i += 2
                continue
            if d == "'" or d == '"':
                # octal or hex constant
                out.append(s[i : i + 2])
                i += 2
                continue
            out.append(s[i : i + 2])
            i += 2
            continue
        out.append(c)
        i += 1
    return "".join(out)


class Web:
    def __init__(self, path=DEFAULT_WEB):
        self.path = path
        with open(path, encoding="latin-1") as f:
            self.src = f.read()
        self.sections = []  # index i holds section i (0 is limbo)
        self.parts = []  # (num, title, first, last)
        self._split()
        self._macros = None
        self._procs = None
        self._modules = None
        self.macros  # also fills macro_list

    # ---- splitting ------------------------------------------------------
    def _split(self):
        src = self.src
        n = len(src)
        i = 0
        starts = []  # (offset, starred)
        while True:
            i = src.find("@", i)
            if i < 0 or i + 1 >= n:
                break
            d = src[i + 1]
            if d == "@":
                i += 2
                continue
            if d in " \t\n":
                starts.append((i, False))
            elif d == "*":
                starts.append((i, True))
            i += 2
        limbo = Section(0, False, 1, src[: starts[0][0]])
        self.sections.append(limbo)
        line = 1 + src.count("\n", 0, starts[0][0])
        for k, (off, starred) in enumerate(starts):
            end = starts[k + 1][0] if k + 1 < len(starts) else n
            text = src[off:end]
            sec = Section(k + 1, starred, line, text)
            line += text.count("\n")
            self._parse_section(sec)
            self.sections.append(sec)
        cur = None
        for s in self.sections[1:]:
            if s.starred:
                cur = s
                self.parts.append([s.num, s.title, s.num, s.num])
            if cur is not None:
                self.parts[-1][3] = s.num
            s.part = self.parts[-1][0] if self.parts else None

    def _parse_section(self, sec):
        text = sec.text
        body = text[2:]
        if sec.starred:
            # title ends at the first period followed by space or end of line
            m = re.search(r"\.(\s|$)", body)
            sec.title = re.sub(r"\s+", " ", body[: m.start()]).strip() if m else body.strip()
            sec.title = sec.title.lstrip("*").strip()
        # find first @d, @f, @p, or @<...@>= outside TeX |...| is not needed: WEB is simple
        i = 0
        n = len(body)
        tex_end = None
        pascal_start = None
        while i < n:
            j = body.find("@", i)
            if j < 0 or j + 1 >= n:
                break
            d = body[j + 1]
            if d == "@":
                i = j + 2
                continue
            if d in "dDfF" and tex_end is None:
                tex_end = j
                i = j + 2
                continue
            if d in "pP":
                if tex_end is None:
                    tex_end = j
                pascal_start = j
                sec.is_def_of = None
                break
            if d == "<":
                e = body.find("@>", j + 2)
                if e < 0:
                    break
                k = e + 2
                while k < n and body[k] in " \t\n":
                    k += 1
                if k < n and body[k] == "=" and not body.startswith("==", k):
                    if tex_end is None:
                        tex_end = j
                    pascal_start = j
                    sec.module = re.sub(r"\s+", " ", body[j + 2 : e]).strip()
                    sec.is_def_of = sec.module
                    pascal_start = k + 1
                    break
                i = e + 2
                continue
            i = j + 2
        if tex_end is None:
            tex_end = n if pascal_start is None else pascal_start
        sec.tex = body[:tex_end]
        if pascal_start is None:
            sec.defs = body[tex_end:]
        else:
            sec.defs = body[tex_end:pascal_start] if sec.module is None else re.split(r"@<", body[tex_end:], maxsplit=1)[0]
            if sec.module is None:
                sec.pascal = body[pascal_start + 2 :]
            else:
                sec.pascal = body[pascal_start:]

    # ---- macros ---------------------------------------------------------
    @property
    def macros(self):
        if self._macros is None:
            self._macros = {}
            self.macro_list = []
            for s in self.sections[1:]:
                if not s.defs:
                    continue
                d = s.defs
                # split on @d (case-insensitive)
                pieces = re.split(r"@[dD]\b|@[dD](?=\s)", d)
                for piece in pieces[1:]:
                    # a following @f ends the definition
                    piece = re.split(r"@[fF]\b", piece)[0]
                    raw = piece
                    t = strip_comments(piece)
                    m = re.match(r"\s*@?!?([A-Za-z_][A-Za-z_0-9]*)\s*(\(#\))?\s*(==|=)(.*)\Z", t, re.S)
                    if not m:
                        continue
                    name, param, kind, body = m.group(1), bool(m.group(2)), m.group(3), m.group(4)
                    body = re.sub(r"\s+", " ", body).strip()
                    mac = Macro(name, kind, param, body, s.num, raw)
                    self.macro_list.append(mac)
                    self._macros.setdefault(name, []).append(mac)
        return self._macros

    # ---- procedures -----------------------------------------------------
    @property
    def procs(self):
        if self._procs is None:
            self._procs = []
            pat = re.compile(
                r"(?<![A-Za-z_@])(procedure|function)\b(?:@\?|@!|@/|@\||\s)*([A-Za-z_][A-Za-z_0-9]*)", re.S
            )
            for s in self.sections[1:]:
                if not s.pascal:
                    continue
                t = re.sub(r"@<.*?@>", " ", strip_comments(s.pascal), flags=re.S)
                pos = 0
                while True:
                    m = pat.search(t, pos)
                    if not m:
                        break
                    kind, name = m.group(1), m.group(2)
                    pos = m.end()
                    # header: up to the first ';' outside parentheses
                    j = pos
                    depth = 0
                    while j < len(t):
                        ch = t[j]
                        if ch == "(":
                            depth += 1
                        elif ch == ")":
                            depth -= 1
                        elif ch == ";" and depth == 0:
                            break
                        j += 1
                    hdr = t[m.start() : j]
                    rest = t[j + 1 : j + 60]
                    forward = bool(re.match(r"\s*(@t\\4\\hskip-\\fontdimen2\\font@>)?\s*forward\b", rest))
                    params, result = parse_header(hdr)
                    p = Proc(name, kind, s.num, params, result, re.sub(r"\s+", " ", hdr).strip(), forward)
                    self._procs.append(p)
                    pos = j
        return self._procs

    def proc_defs(self):
        """Procedures with a body: every non-forward declaration, one per name."""
        seen = {}
        for p in self.procs:
            if p.forward:
                continue
            seen[p.name] = p
        return seen

    # ---- modules --------------------------------------------------------
    @property
    def modules(self):
        """dict full-name -> dict(defs=[sec...], uses=[sec...])"""
        if self._modules is None:
            names = {}
            occurrences = []
            for s in self.sections[1:]:
                part = s.text[2:]
                for m in re.finditer(r"@<(.*?)@>", part, re.S):
                    nm = re.sub(r"\s+", " ", m.group(1)).strip()
                    after = part[m.end() :]
                    is_def = bool(re.match(r"\s*=(?!=)", after))
                    occurrences.append((nm, s.num, is_def))
                    if not nm.endswith("..."):
                        names.setdefault(nm, dict(defs=[], uses=[]))
            full = sorted(names)
            import bisect

            def resolve(nm):
                if nm.endswith("..."):
                    pre = nm[:-3]
                    k = bisect.bisect_left(full, pre)
                    if k < len(full) and full[k].startswith(pre):
                        return full[k]
                    names.setdefault(nm, dict(defs=[], uses=[]))
                    return nm
                return nm

            for nm, sec, is_def in occurrences:
                r = resolve(nm)
                e = names[r]
                (e["defs"] if is_def else e["uses"]).append(sec)
            self._modules = names
        return self._modules


# ---- where each section's code lands ----------------------------------------
# A procedure is the unit of porting.  Tangle inlines every module reachable from a procedure's body
# wherever that module was defined, and so does the port: a section's code lands in the Rust file of
# the procedure that reaches it.
DECL_MODULES = {
    "Global variables", "Types in the outer block", "Constants in the outer block",
    "Labels in the outer block", "Check the ``constant'' values for consistency",
    "Compiler directives",
}
PROC_RE = re.compile(r"(?<![A-Za-z_@])(?:procedure|function)\b(?:\s)*([A-Za-z_][A-Za-z_0-9]*)")
MODREF_RE = re.compile(r"@<(.*?)@>", re.S)


def _mask_comments(t):
    """The text with comments and strings blanked, offsets unchanged."""
    out = list(t)
    i = 0
    n = len(t)
    depth = 0
    while i < n:
        c = t[i]
        if depth:
            if c == "\\":
                out[i] = " "
                if i + 1 < n:
                    out[i + 1] = " "
                i += 2
                continue
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
            out[i] = " "
            i += 1
            continue
        if c == "{":
            depth = 1
            out[i] = " "
            i += 1
            continue
        if c in "\"'":
            j = i + 1
            while j < n and t[j] != c and t[j] != "\n":
                j += 1
            for k in range(i, min(j + 1, n)):
                out[k] = " "
            i = j + 1
            continue
        i += 1
    return "".join(out)


def _module_full_name(w, nm):
    nm = re.sub(r"\s+", " ", nm).strip()
    if nm.endswith("..."):
        pre = nm[:-3]
        for full in w.sorted_module_names():
            if full.startswith(pre):
                return full
        return nm
    return nm


def _sorted_module_names(self):
    if not hasattr(self, "_mod_names"):
        self._mod_names = sorted(self.modules)
    return self._mod_names


Web.sorted_module_names = _sorted_module_names


def _procs_in_section(w, sec):
    return [p for p in w.procs if p.sec == sec.num and not p.forward]


def _section_is_root(w, sec):
    """True if the section's Pascal text declares a procedure with a body, or is an unnamed @p part."""
    if _procs_in_section(w, sec):
        return True
    return sec.module is None and bool(sec.pascal.strip())


def landing(w):
    """dict section -> set of procedure names (or '@p<N>' for code outside any procedure)."""
    if hasattr(w, "_landing"):
        return w._landing
    out = {}
    roots = [s for s in w.sections[1:] if s.pascal.strip() and _section_is_root(w, s) and s.is_def_of not in DECL_MODULES]

    def visit(sec, owner, seen):
        for m in MODREF_RE.finditer(strip_comments(sec.pascal)):
            nm = _module_full_name(w, m.group(1))
            if nm in DECL_MODULES:
                continue
            e = w.modules.get(nm)
            if not e:
                continue
            for d in e["defs"]:
                ds = w.sections[d]
                if _section_is_root(w, ds):
                    continue  # its procedures are ported where they are declared
                key = (d, owner)
                if key in seen:
                    continue
                seen.add(key)
                out.setdefault(d, set()).add(owner)
                visit(ds, owner, seen)

    seen = set()
    for s in roots:
        ps = _procs_in_section(w, s)
        names = [p.name for p in ps] or ["@p%d" % s.num]
        # In a section with several procedures, every module reference belongs to one of them;
        # all are in the same Rust file, which is all that matters here.
        out.setdefault(s.num, set()).update(names)
        visit(s, names[0], seen)
    w._landing = out
    return out


def landing_files(w):
    """dict section -> set of Rust modules where its code goes."""
    from modmap import module_of

    if hasattr(w, "_landing_files"):
        return w._landing_files
    procmod = {p.name: module_of(p.sec) for p in w.procs if not p.forward}
    out = {}
    for k, owners in landing(w).items():
        out[k] = set(module_of(int(o[2:])) if o.startswith("@p") else procmod[o] for o in owners)
    w._landing_files = out
    return out


def tangled(w, name):
    """A procedure's Pascal with every reachable module inlined, marked {§N}.  Macros stay."""
    p = w.proc_defs().get(name)
    if p is None:
        return None
    sec = w.sections[p.sec]
    text = sec.pascal
    # Cut the procedure out of its section, finding keywords only outside comments and strings.
    mask = _mask_comments(text)
    ms = list(re.finditer(r"(?<![A-Za-z_@])(?:procedure|function)\b", mask))
    start = None
    end = len(text)
    for k, m in enumerate(ms):
        hd = PROC_RE.match(re.sub(r"@[?!/|]", "  ", mask[m.start():m.start() + 200]))
        if hd and hd.group(1) == name:
            start = m.start()
            # A forward declaration of something else may follow; the next keyword ends this body.
            if k + 1 < len(ms):
                end = ms[k + 1].start()
            break
    body = text[start:end] if start is not None else text
    out = []

    def expand(t):
        pos = 0
        for m in MODREF_RE.finditer(t):
            out.append(t[pos:m.start()])
            pos = m.end()
            nm = _module_full_name(w, m.group(1))
            e = w.modules.get(nm)
            if nm in DECL_MODULES or not e or not e["defs"]:
                out.append("@<%s@>" % m.group(1))
                continue
            for d in e["defs"]:
                ds = w.sections[d]
                if _section_is_root(w, ds):
                    continue
                out.append("\n{§%d %s}\n" % (d, nm[:60]))
                expand(ds.pascal.strip())
                out.append("\n{end §%d}\n" % d)
        out.append(t[pos:])

    out.append("{§%d %s}\n" % (p.sec, name))
    expand(body)
    return "".join(out)


def parse_header(hdr):
    """Parse 'procedure foo(a,b:integer; var c:real): boolean' into (params, result)."""
    hdr = re.sub(r"@[!?/|#+;,]", " ", hdr)
    hdr = re.sub(r"\s+", " ", hdr).strip()
    params = []
    result = None
    m = re.match(r"(?:procedure|function) [A-Za-z_0-9]+\s*(\((.*)\))?\s*(?::\s*(.*))?$", hdr)
    if not m:
        return params, result
    plist = m.group(2)
    if m.group(3):
        result = m.group(3).strip()
    if plist:
        for group in plist.split(";"):
            group = group.strip()
            if not group:
                continue
            var = False
            if group.startswith("var "):
                var = True
                group = group[4:]
            if ":" not in group:
                continue
            names, ty = group.split(":", 1)
            for nm in names.split(","):
                params.append((var, nm.strip(), ty.strip()))
    return params, result


# ---- CLI ----------------------------------------------------------------
def cmd_sec(w, args):
    a = int(args[0])
    b = int(args[1]) if len(args) > 1 else a
    for k in range(a, b + 1):
        s = w.sections[k]
        print(f"--- §{k} (line {s.line}, part {s.part}) ---")
        print(s.text.rstrip())


def cmd_parts(w, args):
    for num, title, first, last in w.parts:
        print(f"{first:5d}-{last:<5d} {title}")


def cmd_part(w, args):
    pat = " ".join(args).lower()
    hit = False
    for num, title, first, last in w.parts:
        tag = re.match(r"\\\[([^\]]*)\]", title)
        tagv = tag.group(1).lower() if tag else ""
        if pat == tagv or pat in title.lower():
            print(f"{first:5d}-{last:<5d} {title}")
            hit = True
    if not hit:
        print("no such part", file=sys.stderr)
        sys.exit(1)


def cmd_proc(w, args):
    name = args[0]
    hit = False
    for p in w.procs:
        if p.name == name:
            print(f"§{p.sec}{' (forward)' if p.forward else ''}: {p.header}")
            hit = True
    if not hit:
        print("no such procedure", file=sys.stderr)
        sys.exit(1)


def cmd_procs(w, args):
    a, b = 0, 10**9
    if args:
        x = args[0].split("-")
        a, b = int(x[0]), int(x[-1])
    for p in w.procs:
        if not p.forward and a <= p.sec <= b:
            print(f"§{p.sec}: {p.header}")


def cmd_mod(w, args):
    pre = args[0].rstrip(".")
    for nm, e in sorted(w.modules.items()):
        if nm.startswith(pre):
            print(f"@<{nm}@>")
            print(f"  defined in: {' '.join('§%d' % d for d in e['defs'])}")
            print(f"  used in:    {' '.join('§%d' % d for d in e['uses'])}")


def cmd_macro(w, args):
    name = args[0]
    ms = w.macros.get(name)
    if not ms:
        print("no such macro", file=sys.stderr)
        sys.exit(1)
    for m in ms:
        p = "(#)" if m.param else ""
        print(f"§{m.sec}: @d {m.name}{p} {m.kind} {m.body}")


def cmd_grep(w, args):
    rx = re.compile(args[0])
    for s in w.sections[1:]:
        for k, line in enumerate(s.text.split("\n")):
            if rx.search(line):
                print(f"§{s.num}:{s.line + k}: {line}")


def cmd_uses(w, args):
    rx = re.compile(r"(?<![A-Za-z_0-9])" + re.escape(args[0]) + r"(?![A-Za-z_0-9])")
    out = []
    for s in w.sections[1:]:
        if rx.search(strip_comments(s.defs + s.pascal)):
            out.append(s.num)
    print(" ".join(f"§{n}" for n in out))


def cmd_tangle(w, args):
    t = tangled(w, args[0])
    if t is None:
        print("no such procedure", file=sys.stderr)
        sys.exit(1)
    print(t)


def cmd_landing(w, args):
    L = landing(w)
    F = landing_files(w)
    a = int(args[0])
    b = int(args[1]) if len(args) > 1 else a
    for k in range(a, b + 1):
        where = " ".join(sorted(F.get(k, []))) or "-"
        print("§%d: in %s: %s" % (k, where, " ".join(sorted(L.get(k, []))) or "-"))


def cmd_check_weave(w, args):
    """Compare our numbering with weave's .tex output: \\M<n>. and \\N<n>. lines."""
    nums = {}
    with open(args[0], encoding="latin-1") as f:
        for line in f:
            m = re.match(r"\\([MN])(\d+)\.", line)
            if m:
                nums[int(m.group(2))] = m.group(1)
    bad = 0
    for s in w.sections[1:]:
        kind = nums.get(s.num)
        expect = "N" if s.starred else "M"
        if kind != expect:
            print(f"mismatch at §{s.num}: weave {kind}, parser {expect}")
            bad += 1
    print(f"parser sections: {len(w.sections) - 1}; weave sections: {len(nums)}; mismatches: {bad}")
    sys.exit(1 if bad or len(nums) != len(w.sections) - 1 else 0)


CMDS = {
    "sec": cmd_sec,
    "part": cmd_part,
    "parts": cmd_parts,
    "proc": cmd_proc,
    "procs": cmd_procs,
    "mod": cmd_mod,
    "macro": cmd_macro,
    "grep": cmd_grep,
    "uses": cmd_uses,
    "tangle": cmd_tangle,
    "landing": cmd_landing,
    "check-weave": cmd_check_weave,
}


def main():
    if len(sys.argv) < 2 or sys.argv[1] not in CMDS:
        print(__doc__)
        sys.exit(2)
    w = Web()
    CMDS[sys.argv[1]](w, sys.argv[2:])


if __name__ == "__main__":
    main()
