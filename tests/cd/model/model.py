#!/usr/bin/env python3
"""Reference model of tikz-cd layout (floating point, independent of PGF).
Reads a test .tex, compiles it with instrumentation, recomputes every path from the
spec's formulas using only cell/label box dimensions from the log, and compares
against the drawing operators in the PDF that pdflatex produced.
"""
import sys, os, re, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import run as runner, pdfops

BP = 0.99627   # pgf's pt->bp factor


def pt(s):
    return float(s.replace('pt', ''))


# ---------------------------------------------------------------- dim eval
class F:
    def __init__(s, ex, em, rt, ax):
        s.ex, s.em, s.rt, s.ax = ex, em, rt, ax


UNITS = {'pt': 1.0, 'cm': 28.45274, 'mm': 2.84527, 'bp': 1.00375, 'in': 72.27, 'pc': 12.0, 'sp': 1 / 65536.}


def has_units(expr):
    return re.search(r'\d\s*(pt|cm|mm|bp|in|pc|sp|ex|em)\b', expr) is not None


def dim(expr, f, lw=None):
    """evaluate a pgfmath length expression (subset). Unitless => pt."""
    e = expr.strip().replace('\\pgflinewidth', '*%r' % (lw if lw is not None else 0.4))

    def rep(m):
        v = m.group(1)
        u = m.group(2)
        if u == 'ex':
            k = f.ex
        elif u == 'em':
            k = f.em
        else:
            k = UNITS[u]
        return '(%s*%r)' % (v, k)

    e = re.sub(r'(\d*\.?\d+)\s*(pt|cm|mm|bp|in|pc|sp|ex|em)\b', rep, e)
    e = e.replace('^', '**')
    return float(eval(e))


# ---------------------------------------------------------------- tiny tex scanners
def match_brace(s, i, o='{', c='}'):
    d = 0
    n = len(s)
    while i < n:
        ch = s[i]
        if ch == '\\':
            i += 2
            continue
        if ch == o:
            d += 1
        elif ch == c:
            d -= 1
            if d == 0:
                return i
        i += 1
    raise ValueError('unbalanced')


def split_top(s, sep=','):
    out = []
    d = 0
    cur = ''
    i = 0
    while i < len(s):
        ch = s[i]
        if ch == '\\' and i + 1 < len(s):
            cur += s[i:i + 2]
            i += 2
            continue
        if ch == '{':
            d += 1
        elif ch == '}':
            d -= 1
        if ch == sep and d == 0:
            out.append(cur)
            cur = ''
        else:
            cur += ch
        i += 1
    out.append(cur)
    return [x.strip() for x in out if x.strip() != '']


def unbrace(s):
    s = s.strip()
    if s.startswith('{') and match_brace(s, 0) == len(s) - 1:
        return s[1:-1]
    return s


def keyval(item):
    d = 0
    for i, ch in enumerate(item):
        if ch == '{':
            d += 1
        elif ch == '}':
            d -= 1
        elif ch == '=' and d == 0:
            return item[:i].strip(), unbrace(item[i + 1:])
    return item.strip(), None


def parse_label_item(item):
    """"text"... -> (text, [node opts])"""
    assert item[0] == '"'
    d = 0
    i = 1
    while True:
        ch = item[i]
        if ch == '\\':
            i += 2
            continue
        if ch == '{':
            d += 1
        elif ch == '}':
            d -= 1
        elif ch == '"' and d == 0:
            break
        i += 1
    text = item[1:i]
    rest = item[i + 1:].strip()
    opts = []
    if rest.startswith("'"):
        opts.append("'")
        rest = rest[1:].strip()
    if rest.startswith('{'):
        j = match_brace(rest, 0)
        opts += split_top(rest[1:j])
    elif rest:
        opts += split_top(rest)
    return unbrace(text), opts


def scan_diagrams(src):
    src = re.sub(r'(?<!\\)%.*', '', src)
    out = []
    for m in re.finditer(r'(?:\{\\tikzcdset\{((?:[^{}]|\{[^{}]*\})*)\}\s*)?\\begin\{tikzcd\}(.*?)\\end\{tikzcd\}', src, re.S):
        body = m.group(2).lstrip()
        pre = split_top(m.group(1)) if m.group(1) else []
        opts = list(pre)
        if body.startswith('['):
            j = match_brace(body, 0, '[', ']')
            opts += split_top(body[1:j])
            body = body[j + 1:]
        out.append((opts, body))
    return out


def split_cells(body):
    """returns rows: list of list of raw cell strings, plus per-row extras"""
    rows = []
    cur = []
    cell = ''
    d = 0
    i = 0
    extras_row = []
    extras_col = {}
    n = len(body)
    colidx = 0
    while i < n:
        ch = body[i]
        if body.startswith('\\begin{', i):
            d += 1
            cell += '\\begin{'
            i += 7
            j = body.index('}', i)
            cell += body[i:j + 1]
            i = j + 1
            continue
        if body.startswith('\\end{', i):
            d -= 1
            cell += '\\end{'
            i += 5
            j = body.index('}', i)
            cell += body[i:j + 1]
            i = j + 1
            continue
        if ch == '\\':
            if body[i + 1] == '\\' and d == 0:
                cur.append(cell)
                cell = ''
                i += 2
                ex = None
                k = i
                while k < n and body[k] in ' \n\t':
                    k += 1
                if k < n and body[k] == '[':
                    j = match_brace(body, k, '[', ']')
                    ex = body[k + 1:j]
                    i = j + 1
                rows.append(cur)
                extras_row.append(ex)
                cur = []
                colidx = 0
                continue
            if body[i + 1] == '&' and d == 0:
                # ampersand replacement \&
                cur.append(cell)
                cell = ''
                i += 2
                colidx += 1
                k = i
                if k < n and body[k] == '[':
                    j = match_brace(body, k, '[', ']')
                    extras_col.setdefault((len(rows), colidx), body[k + 1:j])
                    i = j + 1
                continue
            cell += body[i:i + 2]
            i += 2
            continue
        if ch == '{':
            d += 1
        elif ch == '}':
            d -= 1
        if ch == '&' and d == 0:
            cur.append(cell)
            cell = ''
            i += 1
            colidx += 1
            if i < n and body[i] == '[':
                j = match_brace(body, i, '[', ']')
                extras_col.setdefault((len(rows), colidx), body[i + 1:j])
                i = j + 1
            continue
        cell += ch
        i += 1
    cur.append(cell)
    rows.append(cur)
    extras_row.append(None)
    return rows, extras_row, extras_col


ARROW_RE = re.compile(r'\\(arrow|ar|rar|lar|dar|uar|urar|ular|drar|dlar)(?![a-zA-Z])')
SHORT = dict(rar='r', lar='l', dar='d', uar='u', urar='ur', ular='ul', drar='dr', dlar='dl')


def scan_cell(raw):
    """extract arrows (as ordered option lists) and remaining content from a cell."""
    arrows = []
    s = raw
    res = ''
    i = 0
    copts = None
    t = s.strip()
    if t.startswith('|'):
        j = t.index('|', 1)
        inner = t[1:j].strip()
        if inner.startswith('['):
            k = match_brace(inner, 0, '[', ']')
            copts = split_top(inner[1:k])
        s = t[j + 1:]
    while True:
        m = ARROW_RE.search(s, i)
        if not m:
            res += s[i:]
            break
        res += s[i:m.start()]
        name = m.group(1)
        k = m.end()

        def ws(k):
            while k < len(s) and s[k] in ' \n\t':
                k += 1
            return k

        opts = []
        if name in SHORT:
            opts.append(SHORT[name])
            k2 = ws(k)
            if k2 < len(s) and s[k2] == '[':
                j = match_brace(s, k2, '[', ']')
                opts += split_top(s[k2 + 1:j])
                k = j + 1
                k2 = ws(k)
        else:
            k2 = ws(k)
            if k2 < len(s) and s[k2] == '[':
                j = match_brace(s, k2, '[', ']')
                opts += split_top(s[k2 + 1:j])
                k = j + 1
                k2 = ws(k)
            if k2 < len(s) and s[k2] == '{':
                j = match_brace(s, k2)
                opts = [s[k2 + 1:j]] + opts   # old syntax direction
                k = j + 1
            else:
                arrows.append(opts)
                i = k
                continue
        # old-syntax labels
        while True:
            k2 = ws(k)
            lopts = []
            if k2 < len(s) and s[k2] == '[':
                j = match_brace(s, k2, '[', ']')
                lopts = split_top(s[k2 + 1:j])
                k2 = ws(j + 1)
            if k2 < len(s) and s[k2] == '{':
                j = match_brace(s, k2)
                opts.append(('LABEL', s[k2 + 1:j], lopts))
                k = j + 1
            else:
                break
        arrows.append(opts)
        i = k
    return arrows, res.strip(), copts


# ---------------------------------------------------------------- tips
class Tip:
    """one tip instance: ends in local coords and draw ops.
    drawn: list of (('m',x,y)|('l',x,y)|('c',x1,y1,x2,y2,x3,y3)), lw, cap, join."""

    def __init__(s):
        s.tip = s.back = s.line = 0.0
        s.sep = 0.0
        s.hull = []
        s.ops = []
        s.lw = None
        s.cap = 0
        s.join = 0
        s.swap = False
        s.fill = False


def lwdep(a, b, c, L, I):
    """\\pgfarrowslinewidthdependent a b c"""
    if I > 0:
        return a + b * ((1 - c / 2) * L - (c / 2) * I)
    return a + b * L


def tip_cm(L, I, length=None, reversed_=False, harpoon=False, sep=0.0, swap=False):
    t = Tip()
    length_v = lwdep(0, 6.2, 0, L, I) if length is None else length
    width = 2.096774 * length_v
    lw = lwdep(0, 1, 1, L, I)
    a = length_v - lw
    b = width - lw
    t.lw = lw
    t.cap = 1
    t.join = 1
    tip = .5 * lw
    back = -a - .5 * lw
    if reversed_:
        line = .5 * lw if harpoon else -.5 * lw
    else:
        line = -.5 * lw
    sx = 1.0
    if reversed_:
        sx = -1.0
        tip, back = -back, -tip
        line = -line
    t.tip, t.back, t.line = tip, back, line
    ops = [('m', sx * -a, .5 * b),
           ('c', sx * -0.81731 * a, .2 * b, sx * -0.41019 * a, 0.05833333 * b, 0.0, 0.0)]
    if harpoon:
        ops.append(('l', sx * ((.5 if reversed_ else -1) * lw), 0.0))
    else:
        ops.append(('c', sx * -0.41019 * a, -0.05833333 * b, sx * -0.81731 * a, -.2 * b, sx * -a, -.5 * b))
    t.ops = ops
    # hull
    h = [(.5 * lw, .5 * lw), (.5 * lw, -.5 * lw),
         (-a + .5 * lw, .5 * b + .5 * lw), (-a - .5 * lw, .5 * b + .5 * lw)]
    if not harpoon:
        h += [(-a + .5 * lw, -(.5 * b + .5 * lw)), (-a - .5 * lw, -(.5 * b + .5 * lw))]
    if harpoon:
        h += [(-lw, -.5 * lw), (-a - .5 * lw, .5 * b - .5 * lw)]
    t.hull = [(sx * x, y) for x, y in h]
    t.sep = sep
    t.swap = swap
    return t


def tip_to_default(L, I):
    """plain /tikz/-> tip: Computer Modern Rightarrow with its defaults (length=1.6pt 2.2)"""
    length = lwdep(1.6, 2.2, 0, L, I)
    return tip_cm(L, I, length=length)


def tip_bar(L, I, w_coef=8.2, w_c=0.89):
    t = Tip()
    width = lwdep(0, w_coef, w_c, L, I)
    lw = lwdep(0, 1, 1, L, I)
    t.lw = lw
    t.cap = 1
    t.join = 0
    t.tip = .5 * lw
    t.back = -.5 * lw
    t.line = -.25 * lw
    t.ops = [('m', 0.0, .5 * width), ('l', 0.0, -.5 * width)]
    t.hull = [(.5 * lw, .5 * width), (-.5 * lw, .5 * width), (.5 * lw, -.5 * width), (-.5 * lw, -.5 * width)]
    return t


def tip_hook(L, I, swap=False, wc=10.8, lc=3.6):
    t = Tip()
    length = lwdep(0, lc, 0, L, I)
    width = lwdep(0, wc, 0, L, I)
    lw = lwdep(0, 1, 1, L, I)
    t.lw = lw
    t.cap = 1
    t.join = 0
    # hull is computed with unadjusted values
    len0, w0 = length, width
    h = [(.5 * len0, -.5 * lw), (len0, -.5 * lw + .125 * w0), (len0, .375 * w0), (.5 * len0, .5 * w0),
         (-.5 * lw, .25 * w0), (-.5 * lw, .5 * w0)]
    a = length - .5 * lw
    b = width - lw
    t.tip = a + .5 * lw
    t.back = -.5 * lw
    t.line = 0.0
    # arc 180 -> adjusted 90: quarter ellipses with radii rx=a, ry=.25*b, center (0,.25*b) ; pgfpatharc approximates
    k = 0.55228
    ry = .25 * b
    t.ops = [('m', 0.0, 2 * ry),
             ('c', k * a, 2 * ry, a, ry + k * ry, a, ry),
             ('c', a, ry - k * ry, k * a, 0.0, 0.0, 0.0)]
    t.hull = h
    t.swap = swap
    return t


def tip_implies(L, I):
    t = Tip()
    a = .25 * L + .25 * I
    b = .5 * L - .5 * I
    t.tip = 2.06 * a + .5 * b
    t.back = -1.36 * a + .5 * b
    t.line = 0.0
    t.lw = b
    t.cap = 1
    t.join = 1
    dx = .06 * a
    t.ops = [('m', -1.4 * a + dx, 2.65 * a),
             ('c', -.75 * a + dx, 1.25 * a, 1 * a + dx, .05 * a, 2 * a + dx, 0.0),
             ('c', 1 * a + dx, -.05 * a, -.75 * a + dx, -1.25 * a, -1.4 * a + dx, -2.65 * a)]
    t.hull = [(-1.4 * a, 2.8 * a), (-1.4 * a, -2.8 * a), (2 * a, .5 * b), (2 * a, -.5 * b)]
    return t


STYLE = [None]

def tip_seq(name, L, I):
    """arrow-tip names as used by tikz-cd styles. returns ordered list (back to front)"""
    if STYLE[0] == 'Latin Modern' and name in ('bar', 'implies bar'):
        return [tip_bar(L, I, 12, .992)]
    if STYLE[0] == 'Latin Modern' and name in ('left hook', 'right hook'):
        t = Tip()
        lw = lwdep(0, 1, 1, L, I)
        length = lwdep(0, 4.2, 0, L, I)
        width = lwdep(0, 15, 0, L, I)
        a = length - .5 * lw
        b = width - lw
        ry = .25 * b
        t.lw = lw; t.cap = 1; t.join = 0
        t.tip = a + .5 * lw
        t.back = math.sin(math.radians(190)) * a - .5 * lw
        t.line = 0.0
        def E_(deg):
            return (a * math.cos(math.radians(deg)), ry + ry * math.sin(math.radians(deg)))
        def arc(d0, d1):
            kk = 4 / 3 * math.tan(math.radians(d0 - d1) / 4)
            c0, s0 = math.cos(math.radians(d0)), math.sin(math.radians(d0))
            c1, s1 = math.cos(math.radians(d1)), math.sin(math.radians(d1))
            p0 = E_(d0); p1 = E_(d1)
            return ('c', p0[0] + kk * a * s0, p0[1] - kk * ry * c0, p1[0] - kk * a * s1, p1[1] + kk * ry * c1, p1[0], p1[1])
        p0 = E_(100)
        t.ops = [('m', p0[0], p0[1]), arc(100, 10), arc(10, -50), arc(-50, -90)]
        t.hull = [(.5 * length, -.5 * lw), (length, -.5 * lw + .125 * width), (length, .375 * width), (.5 * length, .5 * width), (-.5 * length, .5 * width), (-length, .375 * width), (-length, .25 * width)]
        t.swap = (name == 'right hook')
        return [t]
    if STYLE[0] == 'tikz':
        if name in ('bar', 'implies bar'):
            t = tip_bar(L, I)
            width = lwdep(3, 4, .9, L, I)
            t.ops = [('m', 0.0, .5 * width), ('l', 0.0, -.5 * width)]
            t.cap = 0
            lw = t.lw
            t.tip = .5 * lw
            t.back = -.5 * lw
            t.line = -.25 * lw
            t.hull = [(.5 * lw, .5 * width), (-.5 * lw, .5 * width), (.5 * lw, -.5 * width), (-.5 * lw, -.5 * width)]
            return [t]
        if name == 'to':
            return [tip_to_default(L, I)]
        if name == 'double to':
            return [tip_to_default(L, I), tip_to_default(L, I)]
        if name in ('left hook', 'right hook'):
            t = Tip()
            lw = lwdep(0, 1, 1, L, I)
            length = lwdep(.75, 1.25, 0, L, I)
            width = 4 * length - L
            t.lw = lw; t.cap = 0; t.join = 0
            a = length - .5 * lw
            b = width - lw
            ry = .25 * b
            t.tip = a + .5 * lw
            t.back = 0.0
            t.line = 0.0
            k = 0.55228
            t.ops = [('m', 0.0, 2 * ry), ('c', k * a, 2 * ry, a, ry + k * ry, a, ry), ('c', a, ry - k * ry, k * a, 0.0, 0.0, 0.0)]
            t.hull = [(.5 * length, -.5 * lw), (length, -.5 * lw + .125 * width), (length, .375 * width), (.5 * length, .5 * width), (0.0, .25 * width), (0.0, .5 * width)]
            t.swap = (name == 'right hook')
            return [t]
    if name == 'cap' or name == 'implies cap':
        return []
    if name == 'to':
        return [tip_cm(L, I)]
    if name == 'implies':
        return [tip_implies(L, I)]
    if name == 'bar' or name == 'implies bar':
        return [tip_bar(L, I)]
    if name == 'left hook':
        return [tip_hook(L, I)]
    if name == 'right hook':
        return [tip_hook(L, I, swap=True)]
    if name == 'double to':
        return [tip_cm(L, I, sep=lwdep(0, -2.6, 0, L, I)), tip_cm(L, I)]
    if name == 'left to':
        return [tip_cm(L, I, harpoon=True)]
    if name == 'right to':
        return [tip_cm(L, I, harpoon=True, swap=True)]
    if name == 'to reversed':
        return [tip_cm(L, I, reversed_=True)]
    if name == 'plain ->':
        return [tip_to_default(L, I)]
    if name == 'old |':
        return None
    raise KeyError(name)


# ---------------------------------------------------------------- shapes
class Node:
    def __init__(s, w, h, d, ixs, iys, ox, oy, shape, ax):
        s.w, s.h, s.d = w, h, d
        s.ixs, s.iys, s.ox, s.oy = ixs, iys, ox, oy
        s.shape = shape
        s.ax = ax
        # minimum 1pt
        W = max(w + 2 * ixs, 1.0)
        H = max(h + d + 2 * iys, 1.0)
        s.NE = (.5 * W + .5 * w + ox, .5 * H - .5 * d + .5 * h + oy)
        s.SW = (-.5 * W + .5 * w - ox, -.5 * H - .5 * d + .5 * h - oy)
        s.T = (1, 0, 0, 1, 0, 0)

    def anchor_local(s, a):
        NE, SW = s.NE, s.SW
        cx = .5 * (NE[0] + SW[0])
        cy = .5 * (NE[1] + SW[1])
        if s.shape == 'asym' and a in ('center', 'west', 'east'):
            y = s.ax
        else:
            y = cy
        t = {'center': (cx, y), 'west': (SW[0], y), 'east': (NE[0], y),
             'north': (cx, NE[1]), 'south': (cx, SW[1]),
             'north east': NE, 'south west': SW, 'north west': (SW[0], NE[1]), 'south east': (NE[0], SW[1]),
             'base': (cx, 0.0), 'base west': (SW[0], 0.0), 'base east': (NE[0], 0.0),
             'text': (0.0, 0.0),
             'real center': (cx, cy), 'real west': (SW[0], cy), 'real east': (NE[0], cy)}
        return t[a]

    def to_world(s, p):
        T = s.T
        return (T[0] * p[0] + T[2] * p[1] + T[4], T[1] * p[0] + T[3] * p[1] + T[5])

    def anchor(s, a, mid_ex=None):
        if a == 'mid':
            return s.to_world((.5 * (s.NE[0] + s.SW[0]), .5 * mid_ex))
        if a == 'mid east':
            return s.to_world((s.NE[0], .5 * mid_ex))
        if a == 'mid west':
            return s.to_world((s.SW[0], .5 * mid_ex))
        if re.fullmatch(r'-?\d+(\.\d+)?', a):
            ang = math.radians(int(float(a)))
            return s.border_dir((math.cos(ang), math.sin(ang)))
        return s.to_world(s.anchor_local(a))

    def border_dir(s, v):
        """border point in direction v (given in *node local* frame relative to center anchor)"""
        NE, SW = s.NE, s.SW
        hx = .5 * (NE[0] - SW[0])
        cx = .5 * (NE[0] + SW[0])
        if s.shape == 'asym':
            cy = s.ax
            if v[1] > 0:
                hy = NE[1] - s.ax
            else:
                hy = s.ax - SW[1]
        else:
            cy = .5 * (NE[1] + SW[1])
            hy = .5 * (NE[1] - SW[1])
        x, y = abs(v[0]), abs(v[1])
        if x < 1e-9 and y < 1e-9:
            return s.to_world((cx, cy))
        if x * hy >= y * hx:
            px, py = hx, (y * hx / x)
        else:
            px, py = (x * hy / y), hy
        px = px if v[0] >= 0 else -px
        py = py if v[1] >= 0 else -py
        return s.to_world((cx + px, cy + py))

    def border_toward(s, P):
        T = s.T
        # invert T (rigid)
        det = T[0] * T[3] - T[1] * T[2]
        dx, dy = P[0] - T[4], P[1] - T[5]
        lx = (T[3] * dx - T[2] * dy) / det
        ly = (-T[1] * dx + T[0] * dy) / det
        c = s.anchor_local('center')
        return s.border_dir((lx - c[0], ly - c[1]))


class Coord:
    """shape=coordinate"""

    def __init__(s, P):
        s.P = P

    def anchor(s, a, mid_ex=None):
        return s.P

    def border_toward(s, P):
        return s.P


def bez(t, P0, P1, P2, P3):
    u = 1 - t
    a = (u * P0[0] + t * P1[0], u * P0[1] + t * P1[1])
    b = (u * P1[0] + t * P2[0], u * P1[1] + t * P2[1])
    c = (u * P2[0] + t * P3[0], u * P2[1] + t * P3[1])
    d = (u * a[0] + t * b[0], u * a[1] + t * b[1])
    e = (u * b[0] + t * c[0], u * b[1] + t * c[1])
    return (u * d[0] + t * e[0], u * d[1] + t * e[1]), d, e


def norm(v):
    l = math.hypot(*v)
    if l == 0:
        return (0.0, 1.0)   # pgfmathatan2(0,0) path: (0,1)
    return (v[0] / l, v[1] / l)


def control_length(S, E):
    """\\tikz@to@compute@distance@main up to the factor .3915, with PGF's integer quantisation."""
    dx, dy = abs(E[0] - S[0]), abs(E[1] - S[1])
    if dx == 0 and dy == 0:
        return 0.0
    n = norm((dx, dy))
    n = (round(n[0], 5), round(n[1], 5))
    if n[0] > n[1]:
        c = int(n[0] * 65536) // 255
        if c == 0:
            return dx
        return dx * 256 / c
    else:
        c = int(n[1] * 65536) // 255
        if c == 0:
            return dy
        return dy * 256 / c


# ---------------------------------------------------------------- diagram
class Arrow:
    pass


def rot(v, deg):
    r = math.radians(deg)
    c, s = math.cos(r), math.sin(r)
    return (c * v[0] - s * v[1], s * v[0] + c * v[1])


TIPSTYLES = {
    'rightarrow': ('cap', 'to', False, None), 'Rightarrow': ('implies cap', 'implies', True, None),
    'leftarrow': ('to', 'cap', False, None), 'Leftarrow': ('implies', 'implies cap', True, None),
    'leftrightarrow': ('to', 'to', False, None), 'Leftrightarrow': ('implies', 'implies', True, None),
    'mapsto': ('bar', 'to', False, None), 'mapsfrom': ('to', 'bar', False, None),
    'Mapsto': ('implies bar', 'implies', True, None), 'Mapsfrom': ('implies', 'implies bar', True, None),
    'hookrightarrow': ('right hook', 'to', False, None), 'hookleftarrow': ('to', 'left hook', False, None),
    'rightharpoonup': ('cap', 'left to', False, None), 'rightharpoondown': ('cap', 'right to', False, None),
    'leftharpoonup': ('right to', 'cap', False, None), 'leftharpoondown': ('left to', 'cap', False, None),
    'rightarrowtail': ('to reversed', 'to', False, None), 'leftarrowtail': ('to', 'to reversed', False, None),
    'twoheadrightarrow': ('cap', 'double to', False, None), 'twoheadleftarrow': ('double to', 'cap', False, None),
    'dash': ('cap', 'cap', False, None), 'equal': ('implies cap', 'implies cap', True, None),
    'equals': ('implies cap', 'implies cap', True, None),
    'dashrightarrow': ('cap', 'to', False, 'dashed'), 'dashleftarrow': ('to', 'cap', False, 'dashed'),
    'rightsquigarrow': ('cap', 'to', False, 'squiggly'), 'leftsquigarrow': ('to', 'cap', False, 'squiggly'),
    'leftrightsquigarrow': ('to', 'to', False, 'squiggly'),
}
ONE_END = {'no tail': ('s', 'cap'), 'to head': ('e', 'to'), 'maps to': ('s', 'bar'), 'hook': ('s', 'right hook'),
           "hook'": ('s', 'left hook'), 'harpoon': ('e', 'left to'), "harpoon'": ('e', 'right to'),
           'two heads': ('e', 'double to'), 'tail': ('s', 'to reversed'), 'no head': ('e', 'cap')}
POS = {'at start': 0.0, 'very near start': .125, 'near start': .25, 'midway': .5, 'near end': .75,
       'very near end': .875, 'at end': 1.0}
THICK = {'ultra thin': .1, 'very thin': .2, 'thin': .4, 'semithick': .6, 'thick': .8, 'very thick': 1.2,
         'ultra thick': 1.6}

class Unsupported(Exception):
    pass


class Diagram:
    def __init__(s, opts, body, info, font, nodes_log):
        s.opts = opts
        s.body = body
        s.f = font
        s.log = nodes_log
        s.problems = []

    # ------------------------------------- layout
    def layout(s):
        f = s.f
        s.rowsep = dim('1.8em', f)
        s.colsep = dim('2.4em', f)
        s.row_fixed = False
        s.col_fixed = False
        s.ixs = f.ex
        s.iys = .85 * f.ex
        s.mat_inner = 0.0
        s.every_arrow = []
        s.every_label = []
        s.baseline = 0.0
        s.baseline_spec = None
        s.bg = (1, 1, 1)
        s.clearance = 1.5 * f.ex
        s.mathmode = True
        s.arrow_style = None
        s.cell_lw_env = 0.4
        for o in s.opts:
            k, v = keyval(o)
            if k == 'row sep/normal':
                s.rowsep = dim(v, f)
                continue
            if k == 'column sep/normal':
                s.colsep = dim(v, f)
                continue
            if k == 'diagrams':
                raise Unsupported('diagrams key')
            if k == 'arrow style' and v == 'math font':
                raise Unsupported('math font style')
            if k in ('row sep', 'column sep', 'sep'):
                targets = ['row', 'column'] if k == 'sep' else [k.split()[0]]
                for tg in targets:
                    NAMED = dict(row=dict(huge=3.6, large=2.7, normal=1.8, scriptsize=1.35, small=.9, tiny=.45),
                                 column=dict(huge=4.8, large=3.6, normal=2.4, scriptsize=1.8, small=1.2, tiny=.6))
                    fixed = False
                    if v in NAMED[tg]:
                        val = NAMED[tg][v] * f.em
                    else:
                        parts = split_top(v)
                        val = 0.0
                        for p in parts:
                            if p == 'between origins':
                                fixed = True
                            elif p == 'between borders':
                                fixed = False
                            else:
                                val += dim(p, f)
                    if tg == 'row':
                        s.rowsep, s.row_fixed = val, fixed
                    else:
                        s.colsep, s.col_fixed = val, fixed
            elif k == 'cramped':
                s.mat_inner = -.3 * f.em
                s.ixs = s.iys = .3 * f.em
            elif k in ('arrows',):
                s.every_arrow += split_top(v)
            elif k in ('labels',):
                s.every_label += split_top(v)
            elif k == 'every arrow/.append style':
                s.every_arrow += split_top(v)
            elif k == 'every label/.append style':
                s.every_label += split_top(v)
            elif k == 'ampersand replacement':
                pass
            elif k in ('nodes in empty cells', 'font', 'scale', 'every node/.style', 'column 2/.style', 'row 1/.style'):
                pass
            elif k == 'line width':
                s.cell_lw_env = dim(v, f)
            elif k == 'arrow style':
                s.arrow_style = v
            elif k == 'cells':
                for kv in split_top(v):
                    kk, vv = keyval(kv)
                    if kk == 'font':
                        pass
                    elif kk == 'nodes':
                        if 'draw' in vv:
                            s.cells_draw = True
                    elif kk == 'inner xsep':
                        s.ixs = dim(vv, f)
                    elif kk == 'inner ysep':
                        s.iys = dim(vv, f)
                    elif kk == 'inner sep':
                        s.ixs = s.iys = dim(vv, f)
                    else:
                        raise Unsupported('cells ' + kk)
            elif k == 'every cell/.append style':
                for kv in split_top(v):
                    kk, vv = keyval(kv)
                    if kk == 'inner sep':
                        s.ixs = s.iys = dim(vv, f)
                    else:
                        raise Unsupported('every cell ' + kk)
            elif k == 'background color':
                s.bg = v
            elif k == 'crossing over clearance':
                s.clearance = dim(v, f)
            elif k == 'baseline':
                s.baseline_spec = v
            elif k == 'math mode':
                s.mathmode = (v != 'false')
            else:
                raise Unsupported('env option ' + k)
        rows, ex_row, ex_col = split_cells(s.body)
        s.cells = {}
        s.arrow_list = []
        s.grid = []
        for r, row in enumerate(rows, 1):
            for c, raw in enumerate(row, 1):
                arrows, content, copts = scan_cell(raw)
                for a in arrows:
                    s.arrow_list.append((r, c, a))
                if copts:
                    s.cell_opts = getattr(s, 'cell_opts', {})
                    s.cell_opts[(r, c)] = copts
                    for co in copts:
                        kk, vv = keyval(co)
                        if kk == 'alias':
                            s.aliases = getattr(s, 'aliases', {})
                            s.aliases[vv] = (r, c)
                empty = raw.strip() == ''
                s.grid.append((r, c, empty))
        s.nrows = len(rows)
        s.ncols = max(len(r) for r in rows)
        s.ex_row, s.ex_col = ex_row, ex_col
        # cell boxes from log, in order
        cell_logs = [n for n in s.log if re.fullmatch(r'tikz@f@\d+-\d+-\d+', n['name'])]
        matlog = [n for n in s.log if re.fullmatch(r'tikz@f@\d+', n['name'])][0]
        s.boxes = {}
        s.cell_lw = 0.4
        for n in cell_logs:
            _, _, rc = n['name'].partition('@f@')
            parts = rc.split('-')
            s.boxes[(int(parts[1]), int(parts[2]))] = n
        s.named = {}
        row_fx_after = {}
        for r_ in range(1, s.nrows):
            fx_ = s.row_fixed
            ex_ = ex_row[r_ - 1]
            if ex_:
                for p_ in split_top(unbrace(ex_)):
                    if p_ == 'between origins':
                        fx_ = True
                    elif p_ == 'between borders':
                        fx_ = False
            row_fx_after[r_] = fx_
        # column metrics
        minx = {}
        maxx = {}
        maxy = {i: 0.0 for i in range(1, s.nrows + 1)}
        miny = {i: 0.0 for i in range(1, s.nrows + 1)}
        s.nodes = {}
        colsep_for = {}
        fixed_for = {}
        ncols_seen = 0
        order = []
        for r, row in enumerate(rows, 1):
            for c in range(1, len(row) + 1):
                n = s.boxes.get((r, c))
                # at the end of cell c (not last in row), first time column c+1 is reached: decide sep/fixed of c+1
                if c < len(row) and (c + 1) not in colsep_for:
                    extra = ex_col.get((r - 1, c))
                    if extra:
                        extra = unbrace(extra)
                    val = s.colsep
                    fx = s.col_fixed
                    if extra:
                        for p in split_top(extra):
                            if p == 'between origins':
                                fx = True
                            elif p == 'between borders':
                                fx = False
                            else:
                                val += dim(p, f)
                    colsep_for[c + 1] = val
                    fixed_for[c + 1] = fx
                ixs_c, iys_c = s.ixs, s.iys
                cell_draw = False
                if (r, c) in getattr(s, 'cell_opts', {}):
                    for co in s.cell_opts[(r, c)]:
                        kk, vv = keyval(co)
                        if kk == 'inner sep':
                            ixs_c = iys_c = dim(vv, f)
                        elif kk == 'draw':
                            s.cell_strokes = getattr(s, 'cell_strokes', 0) + 1
                            cell_draw = True
                if n is not None and getattr(s, 'cells_draw', False):
                    s.cell_strokes = getattr(s, 'cell_strokes', 0) + 1
                if n is None:
                    # empty cell: zero box
                    l, rr, t, b = 0.0, 0.0, 0.0, 0.0
                    node = None
                else:
                    node = Node(n['wd'], n['ht'], n['dp'], ixs_c, iys_c, .5 * s.cell_lw_env, .5 * s.cell_lw_env, 'asym', f.ax)
                    cx = .5 * (node.NE[0] + node.SW[0])
                    # anchor=base: origin at (cx,0): text box shifted by -cx
                    node.tx = -cx
                    ox = node.ox
                    l = node.SW[0] + ox - cx
                    rr = node.NE[0] - ox - cx
                    t = node.NE[1] - node.oy
                    b = node.SW[1] + node.oy
                    s.nodes[(r, c)] = node
                    if cell_draw or getattr(s, 'cells_draw', False):
                        l -= .2; rr += .2; t += .2; b -= .2
                # fixed columns: this cell is the column's right finish if next column fixed
                if fixed_for.get(c + 1, False):
                    rr = 0.0
                if fixed_for.get(c, False):
                    l = 0.0
                if c > 1:
                    l -= colsep_for[c]
                # row fixed: all rows after the first have height 0
                if r > 1 and row_fx_after.get(r - 1):
                    t = 0.0
                if c not in minx:
                    minx[c] = l
                    maxx[c] = rr
                else:
                    minx[c] = min(minx[c], l)
                    maxx[c] = max(maxx[c], rr)
                maxy[r] = max(maxy[r], t)
                miny[r] = min(miny[r], b)
        s.rawminy = dict(miny)
        # end of rows: subtract seps
        for r in range(1, s.nrows):
            val = s.rowsep
            fx = s.row_fixed
            ex = ex_row[r - 1]
            if ex:
                for p in split_top(unbrace(ex)):
                    if p == 'between origins':
                        fx = True
                    elif p == 'between borders':
                        fx = False
                    else:
                        val += dim(p, f)
            if fx:
                miny[r] = -val
            else:
                miny[r] -= val
        # origin prefix sums
        lastdepth = -miny[s.nrows]
        ys = {s.nrows: 0.0}
        for r in range(s.nrows, 1, -1):
            ys[r - 1] = ys[r] + maxy[r] - miny[r - 1]
        xs = {1: -minx[1]}
        for c in range(1, s.ncols):
            xs[c + 1] = xs[c] + maxx[c] - minx[c + 1]
        W = sum(maxx[c] - minx[c] for c in range(1, s.ncols + 1))
        Ht = ys[1] + maxy[1]
        Dp = lastdepth
        # matrix node
        mat = Node(W, Ht, Dp, s.mat_inner, s.mat_inner, .5 * s.cell_lw_env, .5 * s.cell_lw_env, 'rect', f.ax)
        mc = mat.anchor_local('center')
        offx, offy = -mc[0], -mc[1]
        mat.T = (1, 0, 0, 1, offx, offy)
        s.matrix = mat
        s.problems_mat = (W, Ht, Dp, matlog['wd'], matlog['ht'], matlog['dp'])
        for (r, c), node in s.nodes.items():
            node.T = (1, 0, 0, 1, xs[c] + offx + node.tx, ys[r] + offy)
        s.xs = {c: xs[c] + offx for c in xs}
        s.ys = {r: ys[r] + offy for r in ys}
        # bbox
        s.bb = [1e9, 1e9, -1e9, -1e9]
        ms = mat.ox
        s.bbox_add(offx - 0.0 + mat.SW[0] + ms, offy + mat.SW[1] + ms)
        s.bbox_add(offx + mat.NE[0] - ms, offy + mat.NE[1] - ms)
        s.ops = []
        s.textpos = []
        for (r, c), node in sorted(s.nodes.items()):
            s.textpos.append(('cell', (r, c), node.T[4], node.T[5], node.w, (1.0, 0.0)))

    def bbox_add(s, x, y, r=0.0):
        b = s.bb
        b[0] = min(b[0], x - r)
        b[1] = min(b[1], y - r)
        b[2] = max(b[2], x + r)
        b[3] = max(b[3], y + r)

    # ------------------------------------- arrows
    def draw_arrows(s, ltags):
        """ltags: iterator of label log entries"""
        STYLE[0] = s.arrow_style
        for (r, c, opts) in s.arrow_list:
            s.draw_arrow(r, c, opts, ltags)

    def cellname(s, r, c):
        return s.nodes.get((r, c))

    def resolve_end(s, spec, r, c):
        spec = spec.strip()
        m = re.fullmatch(r'(\d+)-(\d+)', spec)
        if m:
            return s.nodes[(int(m.group(1)), int(m.group(2)))]
        if re.fullmatch(r'[rdlu]*', spec):
            rr, cc = r, c
            for ch in spec:
                if ch == 'r':
                    cc += 1
                elif ch == 'l':
                    cc -= 1
                elif ch == 'd':
                    rr += 1
                elif ch == 'u':
                    rr -= 1
            return s.nodes[(rr, cc)]
        if spec in getattr(s, 'aliases', {}):
            return s.nodes[s.aliases[spec]]
        return s.named[spec]

    def draw_arrow(s, r, c, opts, ltags):
        f = s.f
        A = Arrow()
        A.start = s.nodes.get((r, c))
        A.end = s.nodes.get((r, c))
        A.sa = None
        A.ea = None
        A.lw = f.rt
        A.tips = ['cap', 'to']
        A.double = False
        A.double_default = None
        A.dash = None
        A.squiggly = False
        A.draw = True
        A.short_s = 0.0
        A.short_e = 0.0
        A.shifts = []         # ordered shift-left values and translation tuples
        A.xlate_s = (0.0, 0.0)
        A.xlate_e = (0.0, 0.0)
        A.shiftable = False
        A.pos = .5
        A.labels = []
        A.force_label_font = None
        A.curve = None        # dict(relative, out, in, ol, il, mins...)
        A.crossing = False
        A.stroke = (0,)
        A.canvas = (0.0, 0.0)
        A.phantom = False
        A.swapdir = False
        A.path_lab_opts = []
        A.loop = None
        A.anchor_override = None
        A.sloped = False
        A.dashlater = None
        queue = list(s.every_arrow) + list(opts)
        for o in queue:
            if isinstance(o, tuple):
                A.labels.append((o[1], list(o[2])))
                continue
            if o.startswith('"'):
                t, lo = parse_label_item(o)
                A.labels.append((t, lo))
                continue
            k, v = keyval(o)
            s.arrow_key(A, k, v, r, c)
        s.path_of(A)
        s.render(A, ltags)

    def arrow_key(s, A, k, v, r, c):
        f = s.f
        if re.fullmatch(r'[rdlu]+', k) and v is None:
            A.end = s.resolve_end(k, r, c)
        elif k == 'to':
            A.end = s.resolve_end(v, r, c)
        elif k == 'from':
            A.start = s.resolve_end(v, r, c)
        elif k in TIPSTYLES:
            st, en, dbl, extra = TIPSTYLES[k]
            A.tips = [st, en]
            if dbl:
                A.double = True
            if extra == 'dashed':
                A.dash = (7 * A.lw, 4 * A.lw)
            if extra == 'squiggly':
                A.squiggly = True
        elif k in ONE_END:
            which, name = ONE_END[k]
            A.tips[0 if which == 's' else 1] = name
        elif k == 'double line':
            A.double = True
        elif k == 'dashed':
            A.dash = (7 * A.lw, 4 * A.lw)
        elif k == 'dotted':
            A.dash = (A.lw, 2.0)
        elif k == 'squiggly':
            A.squiggly = True
            if v:
                for kv in split_top(v):
                    kk, vv = keyval(kv)
                    if kk == 'pre length':
                        A.sq_pre = dim(vv, f)
                    elif kk == 'post length':
                        A.sq_post = dim(vv, f)
        elif k in THICK:
            A.lw = THICK[k]
        elif k == 'line width':
            A.lw = dim(v, f)
        elif k == 'phantom':
            A.draw = False
            A.phantom = True
        elif k == 'draw':
            if v == 'none':
                A.draw = False
            elif v is not None:
                A.stroke = v
        elif k == 'shift left':
            A.shiftable = True
            A.shifts.append(('L', v if v is not None else '1'))
        elif k == 'shift right':
            A.shiftable = True
            A.shifts.append(('L', '-(%s)' % (v if v is not None else '1')))
        elif k in ('xshift', 'yshift'):
            A.shiftable = True
            d = dim(v, f)
            dd = (d, 0.0) if k == 'xshift' else (0.0, d)
            A.shifts.append(('T', dd))
        elif k == 'shift':
            A.shiftable = True
            m = re.fullmatch(r'\{?\(([^,]+),([^)]+)\)\}?', v.strip())
            dd = (dim(m.group(1), f), dim(m.group(2), f))
            A.shifts.append(('T', dd))
        elif k == 'transform canvas':
            kk, vv = keyval(v)
            d = dim(vv, f)
            A.canvas = (d, 0.0) if kk == 'xshift' else (0.0, d)
        elif k == 'shorten':
            A.short_s = A.short_e = dim(v, f)
        elif k == 'shorten <':
            A.short_s = dim(v, f)
        elif k == 'shorten >':
            A.short_e = dim(v, f)
        elif k in ('start anchor', 'end anchor'):
            sp = v.strip()
            xf = (0.0, 0.0)
            if sp.startswith('['):
                j = match_brace(sp, 0, '[', ']')
                kk, vv = keyval(sp[1:j])
                d = dim(vv, f)
                xf = (d, 0.0) if kk == 'xshift' else (0.0, d)
                A.shiftable = True
                sp = sp[j + 1:].strip()
            if k == 'start anchor':
                A.sa = sp or None
                A.xlate_s = (A.xlate_s[0] + xf[0], A.xlate_s[1] + xf[1])
            else:
                A.ea = sp or None
                A.xlate_e = (A.xlate_e[0] + xf[0], A.xlate_e[1] + xf[1])
        elif k in ('bend left', 'bend right'):
            ang = float(v) if v is not None else 30.0
            cv = A.curve or dict(l_out=1.0, l_in=1.0, min=0.0)
            if k == 'bend left':
                cv.update(relative=True, out=ang, inn=180 - ang)
            else:
                cv.update(relative=True, out=-ang, inn=180 + ang)
            A.curve = cv
        elif k in ('in', 'out'):
            cv = A.curve or dict(l_out=1.0, l_in=1.0, min=0.0, relative=False, out=45.0, inn=135.0)
            if k == 'in':
                cv['inn'] = float(v)
            else:
                cv['out'] = float(v)
            A.curve = cv
        elif k == 'looseness':
            cv = A.curve or dict(min=0.0, relative=False, out=45.0, inn=135.0)
            cv['l_out'] = cv['l_in'] = float(v)
            A.curve = cv
        elif k == 'distance':
            cv = A.curve or dict(l_out=1.0, l_in=1.0, relative=False, out=45.0, inn=135.0)
            cv['distance'] = dim(v, f)
            A.curve = cv
        elif k in ('in looseness', 'out looseness', 'min distance', 'max distance'):
            cv = A.curve or dict(l_out=1.0, l_in=1.0, relative=False, out=45.0, inn=135.0)
            if k == 'in looseness':
                cv['l_in'] = float(v)
            elif k == 'out looseness':
                cv['l_out'] = float(v)
            elif k == 'min distance':
                cv['min'] = dim(v, f)
            else:
                cv['max'] = dim(v, f)
            A.curve = cv
        elif k in ('loop', 'loop right', 'loop left', 'loop above', 'loop below'):
            cv = A.curve or dict(l_out=1.0, l_in=1.0, relative=False, out=45.0, inn=135.0)
            if k != 'loop':
                o_, i_ = dict(right=(15, -15), above=(105, 75), left=(195, 165), below=(285, 255))[k.split()[1]]
                cv['out'], cv['inn'] = float(o_), float(i_)
                A.dir_side = k.split()[1]
            cv['l_out'] = cv['l_in'] = 8.0
            cv['min'] = dim('5mm', f)
            A.curve = cv
            A.loop = True
        elif k == 'crossing over':
            A.crossing = True
        elif k == '->':
            A.tips = ['cap', 'plain ->']
        elif k == '<-':
            A.tips = ['plain ->', 'cap']
        elif k == '<->':
            A.tips = ['plain ->', 'plain ->']
        elif k == '-':
            A.tips = ['cap', 'cap']
        elif k == 'double':
            A.double_default = True
        elif k == 'double distance':
            A.double_default = dim(v, f)
        elif k in POS:
            A.pos = POS[k]
        elif k == 'pos':
            A.pos = float(v)
        elif k in ('swap', "'"):
            A.swapdir = not A.swapdir
        elif k == 'sloped':
            A.sloped = True
        elif k in ('red', 'blue', 'green', 'black', 'cyan', 'yellow', 'orange') or '!' in k:
            A.stroke = k
        elif k == 'color':
            A.stroke = v
        elif k in ('every arrow',):
            pass
        elif k == 'opacity':
            pass
        else:
            raise Unsupported('arrow key ' + k)

    # compute path S,c1,c2,E; dry path bbox contributions
    def path_of(s, A):
        f = s.f
        st, en = A.start, A.end
        if A.loop:
            en = st

        def base(st, en, sa, ea, xs=(0, 0), xe=(0, 0)):
            cv = A.curve
            mid = f.ex
            if cv is None:
                # straight
                sp = st.anchor(sa, mid) if sa else None
                ep = en.anchor(ea, mid) if ea else None
                if ep is None:
                    ref = sp if sp is not None else st.anchor('center')
                    E = en.border_toward(ref)
                else:
                    E = ep
                S = sp if sp is not None else st.border_toward(E)
                return S, None, None, E
            if cv.get('relative'):
                A0 = st.anchor(sa, mid) if sa else st.anchor('center')
                B0 = en.anchor(ea, mid) if ea else en.anchor('center')
                if sa or isinstance(st, Coord):
                    S = A0
                else:
                    tt = rot((B0[0] - A0[0], B0[1] - A0[1]), cv['out'])
                    S = st.border_toward((A0[0] + tt[0], A0[1] + tt[1]))
                if ea or isinstance(en, Coord):
                    E = B0
                else:
                    tt = rot((A0[0] - B0[0], A0[1] - B0[1]), 180 + cv['inn'])
                    E = en.border_toward((B0[0] + tt[0], B0[1] + tt[1]))
                d = control_length(S, E) * .3915
                xa = min(max(d * cv.get('l_out', 1.0), cv.get('min', 0.0)), cv.get('max', 1e4))
                xb = min(max(d * cv.get('l_in', 1.0), cv.get('min', 0.0)), cv.get('max', 1e4))
                if 'distance' in cv:
                    xa = xb = cv['distance']
                u = norm((E[0] - S[0], E[1] - S[1]))
                v1 = rot(u, cv['out'])
                v2 = rot(u, cv['inn'])
                c1 = (S[0] + xa * v1[0], S[1] + xa * v1[1])
                c2 = (E[0] + xb * v2[0], E[1] + xb * v2[1])
                return S, c1, c2, E
            else:
                A0 = st.anchor(sa, mid) if sa else st.anchor(str(cv['out']))
                B0 = en.anchor(ea, mid) if ea else en.anchor(str(cv['inn']))
                d = control_length(A0, B0) * .3915
                xa = min(max(d * cv.get('l_out', 1.0), cv.get('min', 0.0)), cv.get('max', 1e4))
                xb = min(max(d * cv.get('l_in', 1.0), cv.get('min', 0.0)), cv.get('max', 1e4))
                if 'distance' in cv:
                    xa = xb = cv['distance']
                v1 = rot((1, 0), cv['out'])
                v2 = rot((1, 0), cv['inn'])
                c1 = (A0[0] + xa * v1[0], A0[1] + xa * v1[1])
                c2 = (B0[0] + xb * v2[0], B0[1] + xb * v2[1])
                S = A0 if sa else st.border_toward(c1)
                E = B0 if ea else en.border_toward(c2)
                return S, c1, c2, E

        # `loop`'s `\let\tikztotarget=\tikztostart` takes the start's anchor with it, and `end anchor` goes unread.
        S, c1, c2, E = base(st, en, A.sa, A.sa if A.loop else A.ea)
        A.dry = None
        if A.shiftable:
            A.dry = (S, c1, c2, E)
            a = (S[0] + A.xlate_s[0], S[1] + A.xlate_s[1])
            b = (E[0] + A.xlate_e[0], E[1] + A.xlate_e[1])
            canvas_t = (0.0, 0.0)
            for kind, val in A.shifts:
                if kind == 'T':
                    a = (a[0] + val[0], a[1] + val[1])
                    b = (b[0] + val[0], b[1] + val[1])
            for kind, val in A.shifts:
                if kind == 'L':
                    if has_units(val):
                        rr = dim(val, f)
                    else:
                        rr = dim(val, f) * .56 * f.ex
                    n = norm((a[0] - b[0], a[1] - b[1]))
                    a = (a[0] + rr * n[1], a[1] - rr * n[0])
                    b = (b[0] + rr * n[1], b[1] - rr * n[0])
            # The same `\let` on this run: from `tikzcd@nodea` back to it, wherever the dry path's end went.
            if A.loop:
                b = a
            if A.curve is None:
                S, c1, c2, E = a, None, None, b
            else:
                S, c1, c2, E = base(Coord(a), Coord(b), None, None)
        A.S, A.c1, A.c2, A.E = S, c1, c2, E

    def transform_tip(s, P, ref, total, t, cum):
        """returns CTM for tip whose local origin at cum-back along direction ref->P."""
        u = norm((P[0] - ref[0], P[1] - ref[1]))
        x0 = cum - t.back
        sy = -1.0 if t.swap else 1.0
        return (u, (P[0] + x0 * u[0], P[1] + x0 * u[1]), sy)

    def emit_tips(s, seq, P, ref, shorten, out):
        if not seq:
            return
        total = sum(t.tip + t.sep - t.back for t in seq) + shorten
        cum = -total
        for t in seq:
            u, O, sy = s.transform_tip(P, ref, total, t, cum)

            def W(x, y):
                y = y * sy
                return (O[0] + x * u[0] - y * u[1], O[1] + x * u[1] + y * u[0])

            ops = []
            for op in t.ops:
                if op[0] in 'ml':
                    ops.append((op[0],) + W(op[1], op[2]))
                else:
                    ops.append(('c',) + W(op[1], op[2]) + W(op[3], op[4]) + W(op[5], op[6]))
            if s.relevant:
                for hx, hy in t.hull:
                    wx, wy = W(hx, hy)
                    s.bbox_add(wx + s.canvas_dx, wy + s.canvas_dy) if False else s.bbox_add(wx, wy)
            out.append(dict(kind='S', w=t.lw, cap=t.cap, join=t.join, dash=None, path=ops, what='tip'))
            cum += t.tip + t.sep - t.back

    def render(s, A, ltags):
        f = s.f
        L = A.lw
        I = 0.0
        outer = L
        if A.double:
            q = .45 * f.ex
            outer = L + q
            I = q - L
        if A.double_default is True:
            outer = 2 * L + .6
            I = .6
        elif A.double_default is not None:
            outer = 2 * L + A.double_default
            I = A.double_default
        S, c1, c2, E = A.S, A.c1, A.c2, A.E
        s.relevant = (A.canvas == (0.0, 0.0))
        s.canvas_dx, s.canvas_dy = 0, 0
        # label placement uses unshortened S,E
        # path contributions to bbox
        if A.dry is not None and s.relevant:
            for P in A.dry:
                if P is not None:
                    s.bbox_add(*P)
        pts = [S, E] + ([c1, c2] if c1 is not None else [])
        if s.relevant:
            for P in pts:
                s.bbox_add(*P)
        # decoration
        path = None
        s.seg_coef, s.amp_coef, s.prepost_coef = 9.25, 1.9, 6.0
        s.pre_abs = A.__dict__.get('sq_pre')
        s.post_abs = A.__dict__.get('sq_post')
        if A.squiggly:
            path = s.zigzag(S, E, L, c1, c2)
            if path is None:
                path = None
        ops = []
        if A.crossing:
            w = s.clearance
            if c1 is None and not path:
                u_ = norm((E[0] - S[0], E[1] - S[1]))
                Sh = (S[0] + A.short_s * u_[0], S[1] + A.short_s * u_[1])
                Eh = (E[0] - A.short_e * u_[0], E[1] - A.short_e * u_[1])
                seg = [('m',) + Sh, ('l',) + Eh]
            elif c1 is not None and not path:
                us_ = norm((c1[0] - S[0], c1[1] - S[1]))
                ue_ = norm((c2[0] - E[0], c2[1] - E[1]))
                Sh = (S[0] + A.short_s * us_[0], S[1] + A.short_s * us_[1])
                Eh = (E[0] + A.short_e * ue_[0], E[1] + A.short_e * ue_[1])
                seg = [('m',) + Sh, ('c',) + c1 + c2 + Eh]
            if path:
                seg = path
            ops.append(dict(kind='S', w=w, cap=0, join=0, dash=None, path=seg, what='halo'))
            if s.relevant:
                for P in pts:
                    s.bbox_add(P[0], P[1], .5 * w)
        if A.draw:
            sta = tip_seq(A.tips[0], outer, I) or []
            ena = tip_seq(A.tips[1], outer, I) or []
            sh_e = A.short_e
            sh_s = A.short_s
            xa_e = 0.0
            if ena:
                t = ena[-1]
                xa_e = t.tip + t.sep - t.line
            xa_s = 0.0
            if sta:
                t = sta[-1]
                xa_s = t.tip + t.sep - t.line
            d_e = xa_e + sh_e
            d_s = xa_s + sh_s
            ref_e = c2 if c1 is not None else S
            ref_s = c1 if c1 is not None else E
            if path:
                ref_e = path[-2][1:3]
                ref_s = path[1][1:3]
            ue = norm((ref_e[0] - E[0], ref_e[1] - E[1]))
            E2 = (E[0] + d_e * ue[0], E[1] + d_e * ue[1])
            # straight: start's "second point" is the shortened end, same direction
            if c1 is None and not path:
                ref_s = E2
            us = norm((ref_s[0] - S[0], ref_s[1] - S[1]))
            S2 = (S[0] + d_s * us[0], S[1] + d_s * us[1])
            if path:
                seg = [('m',) + S2] + path[1:-1] + [('l',) + E2]
            elif c1 is None:
                seg = [('m',) + S2, ('l',) + E2]
            else:
                seg = [('m',) + S2, ('c',) + c1 + c2 + E2]
            dash = A.dash
            ops.append(dict(kind='S', w=outer, cap=0, join=0, dash=dash, path=seg, what='line'))
            if I > 0:
                ops.append(dict(kind='S', w=I, cap=0, join=0, dash=dash, path=seg, what='inner'))
            if s.relevant:
                for P in pts:
                    s.bbox_add(P[0], P[1], .5 * outer)
            s.emit_tips(sta, S, ref_s if (c1 is not None or path) else E2, A.short_s, ops)
            s.emit_tips(ena, E, ref_e, A.short_e, ops)
        s.ops += [dict(o, canvas=A.canvas) for o in ops]
        # labels
        for text, lo in A.labels:
            s.place_label(A, text, lo, ltags)

    def zigzag(s, S, E, L, c1=None, c2=None):
        seglen = s.seg_coef * L
        amp = s.amp_coef * L
        pre = post = s.prepost_coef * L
        if s.pre_abs is not None:
            pre = s.pre_abs
        if s.post_abs is not None:
            post = s.post_abs
        hs = .5 * seglen
        if c1 is None:
            D = math.hypot(E[0] - S[0], E[1] - S[1])
            u = norm((E[0] - S[0], E[1] - S[1]))
            def at(x):
                return (S[0] + x * u[0], S[1] + x * u[1]), u
        else:
            # arc length table
            N = 2000
            tab = [0.0]
            prev = S
            for i in range(1, N + 1):
                P, _, _ = bez(i / N, S, c1, c2, E)
                tab.append(tab[-1] + math.hypot(P[0] - prev[0], P[1] - prev[1]))
                prev = P
            D = tab[-1]
            import bisect
            def at(x):
                i = bisect.bisect_left(tab, x)
                t = min(max(i, 0), N) / N
                P, a_, b_ = bez(t, S, c1, c2, E)
                return P, norm((b_[0] - a_[0], b_[1] - a_[1]))
        M = D - pre - post
        k = int(M // hs) if M >= hs else 0
        if D < 8 * L and c1 is None:
            return None    # degenerate: treat as straight line (collinear points)
        pts = [('m',) + S]

        def W(x, y):
            P, u = at(x)
            return (P[0] - y * u[1], P[1] + y * u[0])

        pts.append(('l',) + W(pre, 0))
        x0 = pre
        sign = 1
        for i in range(k):
            pts.append(('l',) + W(x0 + .25 * seglen, sign * amp) if False else ('l',) + s._zz(at, x0, .25 * seglen, sign * amp))
            x0 += hs
            sign = -sign
        if k > 0:
            pts.append(('l',) + W(x0, 0))
        # 'final' state: \pgfpointdecoratedpathlast = current point for a straight input segment,
        # but the true end of the section (arc length D-post) for a curved one.
        if c1 is None:
            pts.append(('l',) + W(x0, 0))
        else:
            pts.append(('l',) + W(D - post, 0))
        pts.append(('l',) + E)
        return pts

    def _zz(s, at, x0, dx, y):
        # the state's coordinate system: origin and tangent at x0; the point is (dx,y) in that frame
        P, u = at(x0)
        return (P[0] + dx * u[0] - y * u[1], P[1] + dx * u[1] + y * u[0])

    def place_label(s, A, text, lo, ltags):
        f = s.f
        n = next(ltags, None)
        if n is None:
            raise Unsupported('no log for label ' + text)
        # filter out coordinate nodes
        while n['shape'] == 'coordinate':
            n = next(ltags)
        ixs = iys = .5 * f.ex
        pos = A.pos
        swap = A.swapdir
        anchor = None
        auto = True
        shape = 'rect'
        fill = None
        name = None
        sloped = A.sloped
        shift = (0.0, 0.0)
        rotate = 0.0
        upside = False
        for o in list(s.every_label) + lo:
            k, v = keyval(o)
            if k in ('above', 'below', 'left', 'right'):
                auto = False
                anchor = dict(above='south', below='north', left='east', right='west')[k]
                if v is not None:
                    d = dim(v, f)
                    dd = dict(above=(0, d), below=(0, -d), left=(-d, 0), right=(d, 0))[k]
                    shift = (shift[0] + dd[0], shift[1] + dd[1])
            elif k == 'xshift':
                shift = (shift[0] + dim(v, f), shift[1])
            elif k == 'yshift':
                shift = (shift[0], shift[1] + dim(v, f))
            elif k == 'shift':
                m = re.fullmatch(r'\{?\(([^,]+),([^)]+)\)\}?', v.strip())
                shift = (shift[0] + dim(m.group(1), f), shift[1] + dim(m.group(2), f))
            elif k == 'rotate':
                rotate = float(v)
            elif k == 'allow upside down':
                upside = True
            elif k in ("'", 'swap'):
                swap = not swap
            elif k in POS:
                pos = POS[k]
            elif k == 'pos':
                pos = float(v)
            elif k == 'inner sep':
                ixs = iys = dim(v, f)
            elif k == 'description':
                auto = False
                anchor = 'center'
                fill = 'bg'
            elif k == 'marking':
                auto = False
                anchor = 'center'
                shape = 'asym'
                sloped = True
            elif k == 'anchor':
                auto = False
                anchor = v
            elif k == 'sloped':
                sloped = True
            elif k == 'name':
                name = v
            elif k in ('font', 'text', 'red', 'blue', 'green', 'black', 'near start'):
                pass
            elif k == 'fill':
                fill = v
            else:
                raise Unsupported('label key ' + k)
        if A.phantom:
            auto = False
            anchor = 'center'
        if A.loop and hasattr(A, 'dir_side'):
            pass
        S, c1, c2, E = A.S, A.c1, A.c2, A.E
        if c1 is None:
            P = (S[0] + pos * (E[0] - S[0]), S[1] + pos * (E[1] - S[1]))
            tan = norm((E[0] - S[0], E[1] - S[1]))
        else:
            P, a, b = bez(pos, S, c1, c2, E)
            tan = norm((b[0] - a[0], b[1] - a[1]))
        lw = n['lw']
        node = Node(n['wd'], n['ht'], n['dp'], ixs, iys, .5 * lw, .5 * lw, shape, f.ax)
        if auto:
            t = (1.0, 0.0) if sloped else tan
            anchor = auto_anchor(t, swap)
        if n['anchor'] != anchor:
            s.problems.append(f"label anchor mismatch text={text!r} model={anchor} oracle={n['anchor']}")
        # rotation when sloped
        rotv = (1.0, 0.0)
        if sloped:
            rotv = tan
            if rotv[0] < 0 and not upside:
                rotv = (-rotv[0], -rotv[1])
        P = (P[0] + rotv[0] * shift[0] - rotv[1] * shift[1], P[1] + rotv[1] * shift[0] + rotv[0] * shift[1])
        if rotate:
            rotv = rot(rotv, rotate)
        if anchor == 'center' and shape == 'rect':
            ap = node.anchor_local('center')
        else:
            ap = node.anchor_local(anchor)
        c, sn = rotv
        ox = P[0] - (c * ap[0] - sn * ap[1])
        oy = P[1] - (sn * ap[0] + c * ap[1])
        node.T = (c, sn, -sn, c, ox, oy)
        # log check of transform position (P)
        ctx, cty = A.canvas
        if abs(n['tx'] - P[0]) > .04 or abs(n['ty'] - P[1]) > .04:
            s.problems.append(f"label pos mismatch text={text!r} model=({P[0]:.3f},{P[1]:.3f}) oracle=({n['tx']:.3f},{n['ty']:.3f})")
        s.textpos.append(('label', text, ox + A.canvas[0], oy + A.canvas[1], n['wd'], (c, sn)))
        if name:
            s.named[name] = node
        # bbox: background corners
        if A.canvas == (0.0, 0.0):
            xs_ = (node.SW[0] + node.ox, node.NE[0] - node.ox)
            ys_ = (node.SW[1] + node.oy, node.NE[1] - node.oy)
            for lx in xs_:
                for ly in ys_:
                    s.bbox_add(*node.to_world((lx, ly)))
        if fill:
            s.ops.append(dict(kind='fill', node=node, what='labelfill', canvas=A.canvas))
            s.fillcount = getattr(s, 'fillcount', 0) + 1

    def result(s):
        return s.ops


def auto_anchor(v, swap):
    x, y = v
    e = 0.05
    if not swap:
        if x > e:
            if y > e:
                return 'south east'
            if y < -e:
                return 'south west'
            return 'south'
        if x < -e:
            if y > e:
                return 'north east'
            if y < -e:
                return 'north west'
            return 'north'
        return 'east' if y > 0 else 'west'
    else:
        if x > e:
            if y > e:
                return 'north west'
            if y < -e:
                return 'north east'
            return 'north'
        if x < -e:
            if y > e:
                return 'south west'
            if y < -e:
                return 'south east'
            return 'south'
        return 'west' if y > 0 else 'east'


# ---------------------------------------------------------------- log parse
def parse_log(log):
    items = []
    for ln in log.split('\n'):
        if not ln.startswith('CD'):
            continue
        parts = ln.split('|')
        d = {'type': parts[0]}
        for p in parts[1:]:
            if '=' in p:
                k, _, v = p.partition('=')
                d[k] = v
        items.append(d)
    return items


def group_pictures(items):
    pics = []
    cur = []
    for it in items:
        if it['type'] == 'CDPICEND':
            continue
        cur.append(it)
        if it['type'] == 'CDPIC':
            pics.append(cur)
            cur = []
    return pics


def node_rec(it):
    m = re.fullmatch(r'\{(.*?)\}\{(.*?)\}\{(.*?)\}\{(.*?)\}\{(.*?)pt\}\{(.*?)pt\}', it['t'])
    return dict(shape=it['shape'], anchor=it['anchor'], name=it['name'], wd=pt(it['wd']), ht=pt(it['ht']),
                dp=pt(it['dp']), lw=pt(it['lw']), tx=float(m.group(5)), ty=float(m.group(6)))


def compare_ops(model_ops, pdf_ops, origin):
    """model ops in pt relative to picture origin; pdf ops in page bp. returns (max_dev, messages)"""
    ox, oy = origin
    msgs = []
    maxd = 0.0
    strokes = [o for o in pdf_ops if o['kind'] == 'path']
    i = 0
    mo = [o for o in model_ops]
    for m in mo:
        if m['kind'] == 'fill':
            # expect a rectangle fill
            if i < len(strokes) and strokes[i]['op'] in ('f', 'B'):
                sg = strokes[i]['path']
                node = m['node']
                cdx, cdy = m.get('canvas', (0.0, 0.0))
                x0, y0 = node.to_world((node.SW[0] + node.ox, node.SW[1] + node.oy))
                xx = (node.NE[0] - node.ox - node.SW[0] - node.ox)
                yy = (node.NE[1] - node.oy - node.SW[1] - node.oy)
                if sg[0][0] == 're':
                    dd = [abs((x0 + cdx) * BP - (sg[0][1] - ox)), abs((y0 + cdy) * BP - (sg[0][2] - oy)), abs(xx * BP - sg[0][3]), abs(yy * BP - sg[0][4])]
                    maxd = max(maxd, max(dd))
                    if max(dd) > .15:
                        msgs.append(f'fill rect dev {dd}')
                i += 1
            else:
                msgs.append('missing fill')
            continue
        if i >= len(strokes):
            msgs.append(f"missing stroke for {m['what']}")
            continue
        p = strokes[i]
        i += 1
        if p['op'] != 'S':
            msgs.append(f"op mismatch {p['op']} for {m['what']}")
            continue
        if abs(p['w'] - m['w'] * BP) > 1e-3:
            msgs.append(f"w mismatch {m['what']}: model {m['w']*BP:.5f} pdf {p['w']:.5f}")
        if m['cap'] != p['cap'] or m['join'] != p['join']:
            msgs.append(f"cap/join mismatch {m['what']}: model {m['cap']}/{m['join']} pdf {p['cap']}/{p['join']}")
        if m.get('dash'):
            if len(p['dash'][0]) != 2 or abs(p['dash'][0][0] - m['dash'][0] * BP) > 2e-3 or abs(p['dash'][0][1] - m['dash'][1] * BP) > 2e-3:
                msgs.append(f"dash mismatch: {m['dash']} vs {p['dash']}")
        elif p['dash'][0]:
            msgs.append('unexpected dash %r' % (p['dash'],))
        pp = p['path']
        # remove doubled initial moveto
        if len(pp) >= 2 and pp[0][0] == 'm' and pp[1][0] == 'm':
            pp = pp[1:]
        if len(pp) != len(m['path']):
            msgs.append(f"seg count mismatch {m['what']}: model {len(m['path'])} pdf {len(pp)}")
            continue
        cdx, cdy = m.get('canvas', (0.0, 0.0))
        for ms, ps in zip(m['path'], pp):
            if ms[0] != ps[0]:
                msgs.append(f"seg kind mismatch {ms[0]} {ps[0]}")
                break
            mc = ms[1:]
            pc = ps[1:]
            for j in range(0, len(mc), 2):
                dx = (mc[j] + cdx) * BP - (pc[j] - ox)
                dy = (mc[j + 1] + cdy) * BP - (pc[j + 1] - oy)
                d = max(abs(dx), abs(dy))
                if d > maxd:
                    maxd = d
                if d > 0.15:
                    msgs.append(f"coord dev {d:.4f} in {m['what']} pt {j}: model ({(mc[j]+cdx)*BP:.4f},{(mc[j+1]+cdy)*BP:.4f}) pdf ({pc[j]-ox:.4f},{pc[j+1]-oy:.4f})")
    if i != len(strokes):
        msgs.append(f"extra pdf strokes: {len(strokes)-i}")
    return maxd, msgs


SUMMARY = []

def check(texpath, verbose=True):
    pdf, log, rc = runner.compile(texpath)
    logtxt = open(pdf.replace('.pdf', '.log'), errors='replace').read()
    src = open(texpath).read()
    diags = scan_diagrams(src)
    pics = group_pictures(parse_log(logtxt))
    conts = runner.contents(pdf)
    all_ops = []
    for c in conts:
        all_ops += pdfops.parse(c)
    # split into pictures
    picsops = []
    cur = None
    for o in all_ops:
        if o['kind'] == 'pic':
            cur = dict(origin=(o['ctm'][4], o['ctm'][5]), ops=[])
            picsops.append(cur)
        elif cur is not None:
            cur['ops'].append(o)
    results = []
    worst = 0.0
    for i, (opts, body) in enumerate(diags):
        if i >= len(pics):
            print('  no log for diagram', i)
            continue
        pic = pics[i]
        if i >= len(picsops):
            print(f'  diagram {i+1}: no PDF output (box not shipped)')
            continue
        info = [p for p in pic if p['type'] == 'CDINFO'][0]
        f = F(pt(info['ex']), pt(info['em']), pt(info['rt']), pt(info['ax']))
        nodes = [node_rec(p) for p in pic if p['type'] == 'CDNODE']
        picinfo = [p for p in pic if p['type'] == 'CDPIC'][0]
        try:
            d = Diagram(opts, body, info, f, nodes)
            d.layout()
            if abs(d.problems_mat[0] - d.problems_mat[3]) > .02 or abs(d.problems_mat[1] - d.problems_mat[4]) > .02 or abs(d.problems_mat[2] - d.problems_mat[5]) > .02:
                d.problems.append(f"matrix dims model {d.problems_mat[:3]} oracle {d.problems_mat[3:]}")
            labels = iter([n for n in nodes if not re.fullmatch(r'tikz@f@\d+(-\d+-\d+)?', n['name'])])
            # check cell positions against text ops later
            d.draw_arrows(labels)
            pops = picsops[i]['ops']
            nskip = getattr(d, 'cell_strokes', 0)
            if nskip:
                kept = []
                for o in pops:
                    if o['kind'] == 'path' and nskip > 0:
                        nskip -= 1
                        continue
                    kept.append(o)
                pops = kept
            maxd, msgs = compare_ops(d.result(), pops, picsops[i]['origin'])
            # bbox
            oracle_bb = (pt(picinfo['minx']), pt(picinfo['miny']), pt(picinfo['maxx']), pt(picinfo['maxy']))
            bbd = max(abs(a - b) for a, b in zip(d.bb, oracle_bb))
            worst = max(worst, maxd)
            texts = [o for o in picsops[i]['ops'] if o['kind'] == 'text']
            ox_, oy_ = picsops[i]['origin']
            tdev = 0.0
            tmiss = []
            for tp in d.textpos:
                wd = tp[-2]
                if wd == 0:
                    continue
                x_, y_ = tp[-4], tp[-3]
                best = min((max(abs(x_ * .996264 - (t['at'][0] - ox_)), abs(y_ * .996264 - (t['at'][1] - oy_))) for t in texts), default=1e9)
                if best > .03:
                    tmiss.append((tp[0], tp[1], round(x_ * .996264, 3), round(y_ * .996264, 3), round(best, 3)))
                else:
                    tdev = max(tdev, best)
            status = 'OK' if (not msgs and not d.problems and bbd < .15) else 'DIFF'
            if tmiss:
                msgs.append(f'text offset (first glyph not at box origin): {tmiss}')
                d.textnote = tmiss
            print(f"  diagram {i+1}: {status} maxdev={maxd:.4f}bp textdev={tdev:.4f}bp bboxdev={bbd:.4f}pt ops={len(d.result())}")
            SUMMARY.append((os.path.basename(texpath), i + 1, status, maxd, tdev, bbd, len(d.result()), len(tmiss)))
            if status != 'OK' or verbose > 1:
                for m in d.problems + msgs:
                    print('     ', m)
                if bbd >= .15:
                    print(f"      bbox model {[round(x,3) for x in d.bb]} oracle {oracle_bb}")
        except Unsupported as e:
            print(f"  diagram {i+1}: SKIP unsupported {e}")
        except Exception as e:
            import traceback
            print(f"  diagram {i+1}: EXC {e!r}")
            traceback.print_exc(limit=3)
    return worst


if __name__ == '__main__':
    for t in sys.argv[1:]:
        print(os.path.basename(t))
        check(t)
    n = len(SUMMARY)
    print('TOTAL', n, 'OK', sum(1 for x in SUMMARY if x[2] == 'OK'), 'maxdev', max(x[3] for x in SUMMARY), 'maxtext', max(x[4] for x in SUMMARY), 'maxbbox', max(x[5] for x in SUMMARY), 'strokes', sum(x[6] for x in SUMMARY))
