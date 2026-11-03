#!/usr/bin/env python3
"""compare_pdf.py -- differential PDF comparison for rstex against the pdfTeX oracle.

    compare_pdf.py A.pdf B.pdf [--dpi 100] [--no-raster] [--eps 0] [--jobs N] [--diff-dir DIR] [--no-structure]
                   [--pinned-clock] [--no-container]

Page level: decode /Contents with qpdf, split into operators, rewrite resource names (/F48, /Im1, /R7 ...)
to content-derived labels (BaseFont minus subset tag + encoding hash; md5 of XObject data and dictionary), compare operator
by operator, and report the first divergence.  Also compares every page attribute (boxes, annotations, anything \\pdfpageattr
can add) and the page's resources, and rasterises both sides with pdftoppm to give a percentage of differing pixels.
Document level: Info, catalog, the whole outline tree with its back links, dests, fonts, the page tree, the trailer.
Container level: PDF header, xref kind, object count, object streams, which streams are compressed, objects nothing refers to,
and `qpdf --check`, since qpdf and pdftoppm both repair a damaged file without saying so.

What never counts: object numbers, /ID, subset tags, the engine's own banner, deflate bytes and how hard they were compressed
(DESIGN section 9 chooses its own levels, so only compressed or not is compared).  Dates count under --pinned-clock, which
says both files were written under the same SOURCE_DATE_EPOCH.

Needs qpdf (>= 11) and pdftoppm.  Exit status: 0 all equal, 1 some difference, 2 could not run.
"""
import argparse, base64, hashlib, json, os, re, subprocess, sys, tempfile
from concurrent.futures import ThreadPoolExecutor

REF = re.compile(r'^(\d+) (\d+) R$')
TAG = re.compile(r'\b[A-Z]{6}\+')
BANNERS = {'/Producer', '/PTEX.Fullbanner'}       # the engine's own name, and only when it is the engine's own name
BANNER_VALUE = re.compile(r'^(?:u:)?(?:pdfTeX-|This is pdfTeX|rstex|This is rstex)')
DATES = {'/CreationDate', '/ModDate'}
STRUCTURAL = {'/Parent', '/Prev', '/Last'}         # back links: judged by outline_tree() and pages_shape(), not expanded, which would be quadratic
md5 = lambda b: hashlib.md5(b).hexdigest()[:10]
as_dict = lambda v: v if isinstance(v, dict) else {}     # a damaged link can point at a string or a number: compare it, do not crash on it


class Pdf:
    def __init__(self, path, pinned_clock=False):
        self.path = path
        self.pinned_clock = pinned_clock
        out = subprocess.run(['qpdf', '--json=2', '--json-key=qpdf', '--json-stream-data=inline',
                              '--decode-level=generalized', path, '-'], capture_output=True)
        if not out.stdout:
            sys.exit(f'qpdf failed on {path}: {out.stderr.decode(errors="replace")}')
        self.objs = json.loads(out.stdout)['qpdf'][1]
        self.trailer = self.objs['trailer']['value']
        self.root = self.get(self.trailer['/Root'])
        self.pages, self.page_index = [], {}
        self._walk(self.root['/Pages'], {})
        self._font_labels = None
        self._xobject_labels = {}

    def raw(self, ref):
        return self.objs.get(f'obj:{ref}', {'value': None})

    def get(self, v):
        """Follow references; stream objects resolve to their dictionary."""
        while isinstance(v, str) and REF.match(v):
            o = self.raw(v)
            v = o['stream']['dict'] if 'stream' in o else o.get('value')
        return v

    def data(self, ref):
        o = self.raw(ref)
        return base64.b64decode(o['stream'].get('data', '')) if 'stream' in o else b''

    def _walk(self, ref, inherited):
        node = self.get(ref)
        attrs = {**inherited, **{k: node[k] for k in ('/Resources', '/MediaBox', '/CropBox', '/Rotate') if k in node}}
        if node.get('/Type') == '/Pages' or '/Kids' in node:
            for kid in self.get(node['/Kids']):
                self._walk(kid, attrs)
        else:
            self.page_index[ref] = len(self.pages) + 1
            self.pages.append((ref, {**attrs, **node}))

    def deep(self, v, seen=()):
        """Canonical form of a value: references expanded, page refs become page numbers, streams hashed."""
        if isinstance(v, str):
            if REF.match(v):
                if v in self.page_index:
                    return f'page#{self.page_index[v]}'
                if v in seen:
                    return '<cycle>'
                o = self.raw(v)
                if 'stream' in o:
                    return {'dict': self.deep(o['stream']['dict'], seen + (v,)), 'md5': md5(self.data(v))}
                return self.deep(o.get('value'), seen + (v,))
            return TAG.sub('', v)
        if isinstance(v, list):
            return [self.deep(x, seen) for x in v]
        if isinstance(v, dict):
            return {k: self.deep(x, seen) for k, x in sorted(v.items()) if self.counts(k, x)}
        return v

    def counts(self, key, value):
        if key in STRUCTURAL:
            return False
        if key == '/P':                               # an annotation's page counts; a structure element's parent is a back link, and expanding it is exponential
            return isinstance(value, str) and value in self.page_index
        if key in DATES:
            return self.pinned_clock
        if key in BANNERS:                            # a document can set /Producer itself: then it is the document's text
            return not (isinstance(value, str) and BANNER_VALUE.match(value))
        return True

    # ---------------- resource names -> content-derived labels ----------------
    def base_font_label(self, ref):
        f = self.get(ref) or {}
        enc = self.deep(f.get('/Encoding'))
        return TAG.sub('', str(f.get('/BaseFont', '?'))) + '~' + md5(json.dumps(enc, sort_keys=True).encode())[:4]

    def font_objects(self):
        return [k[4:] for k, o in self.objs.items() if isinstance(o, dict) and isinstance(o.get('value'), dict) and o['value'].get('/Type') == '/Font']

    def font_label(self, ref):
        """BaseFont and encoding, and the font's own hash when two font objects in this file share those: a page that picks the
        wrong one of two such fonts would otherwise look the same, and a dictionary keyed by the label would keep only one."""
        if self._font_labels is None:
            groups = {}
            for r in self.font_objects():
                groups.setdefault(self.base_font_label(r), []).append(r)
            self._font_labels = {}
            for label, refs in groups.items():
                for r in refs:
                    self._font_labels[r] = label if len(refs) == 1 else label + '#' + md5(json.dumps(self.deep(self.objs[f'obj:{r}']['value']), sort_keys=True).encode())[:6]
        return self._font_labels[ref] if isinstance(ref, str) and ref in self._font_labels else self.base_font_label(ref)

    def res_label(self, kind, name, res, seen):
        table = self.get((self.get(res) or {}).get(kind)) or {}
        ref = table.get(name)
        if ref is None:
            return f'{name}?unresolved'
        if kind == '/Font':
            return '/' + self.font_label(ref).lstrip('/')
        if kind == '/XObject' and isinstance(ref, str) and ref not in seen:
            got = self._xobject_labels.get(ref)
            if got:
                return got
            d = self.get(ref)
            attrs = json.dumps(self.deep({k: x for k, x in d.items() if k != '/Resources'}), sort_keys=True).encode()
            if d.get('/Subtype') == '/Form':
                ops = self.normalize(self.data(ref), d.get('/Resources', res), seen + (ref,))
                body = '\n'.join(' '.join(o) for o in ops).encode()
                got = f'/Form:{md5(attrs + body)}'
            else:
                got = f"/Image:{md5(self.data(ref))}:{d.get('/Width')}x{d.get('/Height')}:{md5(attrs)}"
            self._xobject_labels[ref] = got
            return got
        return f'/{kind[1:]}:' + md5(json.dumps(self.deep(ref), sort_keys=True).encode())

    def normalize(self, stream, res, seen=()):
        ops = []
        for op in split_ops(stream):
            kind = {'Tf': '/Font', 'Do': '/XObject', 'gs': '/ExtGState', 'sh': '/Shading',
                    'cs': '/ColorSpace', 'CS': '/ColorSpace', 'scn': '/Pattern', 'SCN': '/Pattern'}.get(op[-1])
            if kind:
                op = [self.res_label(kind, t, res, seen) if t.startswith('/') else t for t in op]
            ops.append(op)
        return ops

    def page_ops(self, n):
        _, page = self.pages[n]
        contents = page.get('/Contents')
        refs = contents if isinstance(contents, list) else [contents] if contents else []
        if isinstance(self.get(contents), list):
            refs = self.get(contents)
        stream = b'\n'.join(self.data(r) for r in refs)
        return self.normalize(stream, page.get('/Resources', {}))

    def page_meta(self, n):
        """Every attribute of the page, so that nothing \\pdfpageattr adds can pass unseen."""
        _, page = self.pages[n]
        meta = {k: self.deep(v) for k, v in sorted(page.items()) if k not in ('/Parent', '/Contents', '/Resources', '/Type')}
        meta['/Resources'] = self.resources(page.get('/Resources', {}))
        return meta

    def resources(self, res):
        """Names do not count, but what the names stand for does, used on the page or not."""
        res = self.get(res) or {}
        out = {}
        for kind, table in sorted(res.items()):
            if kind in ('/Font', '/XObject', '/ExtGState', '/Shading', '/Pattern', '/ColorSpace'):
                t = self.get(table) or {}
                out[kind] = sorted(self.res_label(kind, name, {kind: t}, ()) for name in t)
            else:
                out[kind] = self.deep(table)
        return out

    def outline_tree(self):
        """The outline as a table of nodes numbered in reading order, each link written as such a number: /Last, /Prev and /Parent
        are checked here, which a walk along /First and /Next cannot do.  A link leaving the tree shows as 'outside'."""
        root = self.root.get('/Outlines')
        if root is None:
            return None
        if not isinstance(self.get(root), dict):
            return 'not a dictionary'
        index, order = {}, []
        stack = [root]
        while stack:                                  # preorder without recursion: outlines can be long
            ref = stack.pop()
            if ref in index or not isinstance(ref, str):
                continue
            index[ref] = len(index)
            order.append(ref)
            kids, child = [], as_dict(self.get(ref)).get('/First')
            while child and child not in kids and child not in index:
                kids.append(child)
                child = as_dict(self.get(child)).get('/Next')
            stack.extend(reversed(kids))
        link = lambda v: None if v is None else index.get(v, 'outside')
        table = []
        for ref in order:
            node = as_dict(self.get(ref))
            row = {k: self.deep(x) for k, x in sorted(node.items()) if k not in ('/First', '/Next', '/Prev', '/Last', '/Parent')}
            for k in ('/First', '/Next', '/Prev', '/Last', '/Parent'):
                row[k] = link(node.get(k))
            table.append(row)
        return table

    def pages_shape(self, ref=None):
        """The page tree: its attributes (\\pdfpagesattr goes here) and how it is balanced."""
        node = as_dict(self.get(self.root['/Pages'] if ref is None else ref))
        if node.get('/Type') != '/Pages' and '/Kids' not in node:
            return 'page'
        attrs = {k: self.deep(x) for k, x in sorted(node.items()) if k not in ('/Kids', '/Parent', '/Type')}
        return {'attrs': attrs, 'kids': [self.pages_shape(k) for k in self.get(node['/Kids'])]}

    def document(self):
        groups = {}
        for ref in self.font_objects():
            groups.setdefault(self.base_font_label(ref), []).append(self.deep(self.objs[f'obj:{ref}']['value']))
        fonts = {}
        for label, values in groups.items():
            if len(values) == 1:
                fonts[label] = values[0]
            else:
                for k, v in enumerate(sorted(values, key=lambda x: json.dumps(x, sort_keys=True))):
                    fonts[f'{label}#{k}'] = v
        skip = ('/Root', '/Info', '/ID', '/Size', '/Prev', '/XRefStm', '/Type', '/W', '/Index', '/Length', '/Filter', '/DecodeParms')
        return {'Info': self.deep(self.trailer.get('/Info')),
                'Catalog': self.deep({k: x for k, x in self.root.items() if k not in ('/Pages', '/Outlines')}),
                'Outline': self.outline_tree(),
                'PageTree': self.pages_shape(),
                'Trailer': self.deep({k: x for k, x in self.trailer.items() if k not in skip}),
                'PageCount': len(self.pages), 'Fonts': fonts}

    def reachable(self):
        seen, stack = set(), [v for k, v in self.trailer.items() if k in ('/Root', '/Info', '/Encrypt')]
        stack.append({k: v for k, v in self.trailer.items() if k not in ('/ID', '/W', '/Index', '/Length', '/Filter', '/DecodeParms', '/Type', '/Size')})
        while stack:
            v = stack.pop()
            if isinstance(v, str):
                if REF.match(v) and v not in seen:
                    seen.add(v)
                    o = self.raw(v)
                    stack.append(o['stream']['dict'] if 'stream' in o else o.get('value'))
            elif isinstance(v, list):
                stack.extend(v)
            elif isinstance(v, dict):
                stack.extend(v.values())
        return seen

    def container(self):
        """How the file is built.  L4 and L5 read through all of this, and L6 turns compression off first, so this is the only place
        a wrong \\pdfobjcompresslevel, \\pdfcompresslevel=0 or \\pdfminorversion can show."""
        with open(self.path, 'rb') as f:
            header = f.read(16).split(b'\n')[0].decode('latin-1')
        raw = subprocess.run(['qpdf', '--json=2', '--json-key=qpdf', '--json-stream-data=none', '--decode-level=none', self.path, '-'], capture_output=True)
        objs = json.loads(raw.stdout)['qpdf'][1]
        trailer = objs['trailer']['value']
        streams = {k: v['stream']['dict'] for k, v in objs.items() if isinstance(v, dict) and 'stream' in v}
        objstm = [d.get('/N') for d in streams.values() if d.get('/Type') == '/ObjStm']
        by_filter = {}
        for d in streams.values():
            f = d.get('/Filter')
            kind = str(d.get('/Type') or d.get('/Subtype') or 'plain')
            key = f'{kind}:{f if isinstance(f, str) else json.dumps(f)}'
            by_filter[key] = by_filter.get(key, 0) + 1
        reach = self.reachable()
        orphans = []
        for k, v in objs.items():
            ref = k[4:] if k.startswith('obj:') else None
            if ref and ref not in reach and self.raw(ref) is not None:
                t = streams.get(k, {}).get('/Type')
                if t not in ('/ObjStm', '/XRef') and ref not in ('0 0 R',):
                    orphans.append(md5(json.dumps(self.deep(ref), sort_keys=True).encode()))
        return {'header': header, 'xref': 'stream' if trailer.get('/Type') == '/XRef' else 'table', 'size': trailer.get('/Size'),
                'objects': sum(k.startswith('obj:') for k in objs), 'object_streams': objstm, 'streams_by_filter': by_filter,
                'unreferenced_objects': sorted(orphans)}

    def check(self):
        """qpdf --check, whose warnings are what qpdf and pdftoppm otherwise repair in silence: a wrong xref offset, a bad startxref."""
        p = subprocess.run(['qpdf', '--check', self.path], capture_output=True)
        text = (p.stdout + p.stderr).decode('utf-8', 'replace').replace(self.path, '<file>')
        problems = [line for line in text.splitlines() if line.startswith(('WARNING', 'ERROR')) or ' error' in line.lower() and 'No syntax' not in line]
        return {'rc': p.returncode, 'problems': problems}


TOKEN = re.compile(rb'''[\x00\t\n\f\r ]+|%[^\r\n]*|(?P<str>\()|(?P<hex><[0-9A-Fa-f\s]*>)|(?P<name>/[^\x00\t\n\f\r ()<>\[\]{}/%]*)
                        |(?P<delim>\[|\]|<<|>>)|(?P<word>[^\x00\t\n\f\r ()<>\[\]{}/%]+)''', re.X)


def read_string(b, i):
    """Literal string starting after '(' at i; returns canonical printable text and index after ')'."""
    out, depth = bytearray(), 1
    while i < len(b):
        c = b[i]; i += 1
        if c == 0x5c:
            e = b[i:i + 1]; i += 1
            if e in (b'(', b')', b'\\'): out += e
            elif e and e in b'nrtbf': out.append(b'\n\r\t\b\f'[b'nrtbf'.index(e)])
            elif e.isdigit():
                j = i
                while j < min(i + 2, len(b)) and b[j:j + 1].isdigit(): j += 1
                out.append(int(b[i - 1:j], 8) & 255); i = j
            continue
        if c == 0x28: depth += 1
        elif c == 0x29:
            depth -= 1
            if depth == 0: break
        out.append(c)
    return '(' + ''.join(chr(x) if 32 <= x < 127 and x not in (40, 41, 92) else f'\\{x:03o}' for x in out) + ')', i


def split_ops(b):
    """Yield operators as token lists: [operand..., operator]."""
    i, cur = 0, []
    while i < len(b):
        m = TOKEN.match(b, i)
        if not m:                                         # a stray byte from a \\pdfliteral: it must still compare, never vanish
            cur.append(f'<byte {b[i]:02x}>'); i += 1; continue
        i = m.end()
        if m.lastgroup == 'str':
            tok, i = read_string(b, i); cur.append(tok)
        elif m.lastgroup in ('hex', 'name', 'delim'):
            cur.append(m.group().decode('latin-1'))
        elif m.lastgroup == 'word':
            w = m.group().decode('latin-1')
            if re.fullmatch(r'[-+]?(\d+\.?\d*|\.\d+)|true|false|null', w):
                cur.append(w)
            elif w == 'ID':                               # inline image: bytes up to EI are opaque
                end = b.find(b'EI', i); cur += [md5(b[i:end]), 'ID']; i = end + 2; yield cur; cur = []
            else:
                cur.append(w); yield cur; cur = []
    if cur:
        yield cur


def same(x, y, eps):
    if x == y: return True
    if not eps: return False
    try: return abs(float(x) - float(y)) <= eps
    except ValueError: return False


def first_diff(a, b, eps):
    for k in range(min(len(a), len(b))):
        if len(a[k]) != len(b[k]) or not all(same(s, t, eps) for s, t in zip(a[k], b[k])):
            return k
    return None if len(a) == len(b) else min(len(a), len(b))


def render(path, page, dpi, out):
    subprocess.run(['pdftoppm', '-r', str(dpi), '-singlefile', '-f', str(page), '-l', str(page), path, out], check=True)
    with open(out + '.ppm', 'rb') as f:
        data = f.read()
    m = re.match(rb'P6\s+(\d+)\s+(\d+)\s+(\d+)\s', data)
    return int(m[1]), int(m[2]), data[m.end():]


def raster_diff(args):
    pa, pb, page, dpi, tmp, diff_dir = args
    wa, ha, a = render(pa, page, dpi, f'{tmp}/a{page}')
    wb, hb, b = render(pb, page, dpi, f'{tmp}/b{page}')
    if (wa, ha) != (wb, hb):
        return page, 100.0
    if a == b:
        os.unlink(f'{tmp}/a{page}.ppm'); os.unlink(f'{tmp}/b{page}.ppm')
        return page, 0.0
    d = sum(a[i:i + 3] != b[i:i + 3] for i in range(0, len(a), 3))
    if diff_dir:
        subprocess.run(['magick', 'compare', f'{tmp}/a{page}.ppm', f'{tmp}/b{page}.ppm', f'{diff_dir}/page{page}.png'], capture_output=True)
    os.unlink(f'{tmp}/a{page}.ppm'); os.unlink(f'{tmp}/b{page}.ppm')
    return page, 100.0 * d / (wa * ha)


def flat_diff(x, y, path=''):
    """Paths at which two canonical values differ."""
    if type(x) != type(y): return [path or '/']
    if isinstance(x, dict):
        return [d for k in sorted(set(x) | set(y)) for d in flat_diff(x.get(k), y.get(k), f'{path}/{k}')]
    if isinstance(x, list):
        if len(x) != len(y): return [f'{path}[len {len(x)} vs {len(y)}]']
        return [d for i, (p, q) in enumerate(zip(x, y)) for d in flat_diff(p, q, f'{path}[{i}]')]
    return [] if x == y else [path]


def main():
    sys.setrecursionlimit(100000)
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('a'); ap.add_argument('b')
    ap.add_argument('--dpi', type=int, default=100)
    ap.add_argument('--no-raster', action='store_true')
    ap.add_argument('--no-structure', action='store_true')
    ap.add_argument('--eps', type=float, default=0.0, help='numeric tolerance for operands (default: byte-exact)')
    ap.add_argument('--jobs', type=int, default=os.cpu_count())
    ap.add_argument('--context', type=int, default=2)
    ap.add_argument('--diff-dir', help='write ImageMagick diff images for pages that differ')
    ap.add_argument('--pinned-clock', action='store_true', help='both files were written under the same SOURCE_DATE_EPOCH, so dates count')
    ap.add_argument('--no-container', action='store_true', help='skip the header, xref, object-stream and qpdf --check comparison')
    args = ap.parse_args()
    A, B = Pdf(args.a, args.pinned_clock), Pdf(args.b, args.pinned_clock)
    if args.diff_dir: os.makedirs(args.diff_dir, exist_ok=True)
    n = min(len(A.pages), len(B.pages))
    raster = {}
    if not args.no_raster and n:
        with tempfile.TemporaryDirectory() as tmp, ThreadPoolExecutor(args.jobs) as ex:
            raster = dict(ex.map(raster_diff, [(args.a, args.b, p + 1, args.dpi, tmp, args.diff_dir) for p in range(n)]))
    bad_content, bad_meta, first = [], [], None
    print(f'{"page":>4} {"content":8} {"ops":>6} {"meta":6} {"raster%":>8}')
    for p in range(n):
        x, y = A.page_ops(p), B.page_ops(p)
        k = first_diff(x, y, args.eps)
        meta = flat_diff(A.page_meta(p), B.page_meta(p))
        if k is not None:
            bad_content.append(p + 1)
            first = first or (p + 1, k, x, y)
        if meta: bad_meta.append((p + 1, meta))
        r = f'{raster[p + 1]:8.4f}' if raster else '       -'
        print(f'{p + 1:4d} {"EQUAL" if k is None else "DIFF":8} {len(x):6d} {"EQUAL" if not meta else "DIFF":6} {r}')
    print(f'pages: {len(A.pages)} vs {len(B.pages)}; content-equal {n - len(bad_content)}/{n}; '
          f'meta-equal {n - len(bad_meta)}/{n}' + (f'; max raster diff {max(raster.values()):.4f}%' if raster else ''))
    if first:
        page, k, x, y = first
        print(f'FIRST CONTENT DIVERGENCE: page {page}, operator #{k} of {len(x)}/{len(y)}')
        for i in range(max(0, k - args.context), k):
            print(f'    = {" ".join(x[i])[:200]}')
        print(f'    A {" ".join(x[k])[:400] if k < len(x) else "<end of stream>"}')
        print(f'    B {" ".join(y[k])[:400] if k < len(y) else "<end of stream>"}')
    if bad_meta:
        print(f'FIRST META DIVERGENCE: page {bad_meta[0][0]}: {bad_meta[0][1][:5]}')
    struct = []
    if not args.no_structure:
        da, db = A.document(), B.document()
        if not args.no_container:
            da['Container'], db['Container'] = A.container(), B.container()
            ca, cb = A.check(), B.check()
            # A file the oracle itself damages is reported only if the candidate's damage is different.
            da['Check'], db['Check'] = ca['problems'][:20], cb['problems'][:20]
        struct = flat_diff(da, db)
        print(f'document structure: {"EQUAL" if not struct else f"{len(struct)} DIFFERENCES"} '
              f'({len(da["Fonts"])} vs {len(db["Fonts"])} fonts)')
        for s in struct[:10]:
            print('    ', s)
    ok = len(A.pages) == len(B.pages) and not bad_content and not bad_meta and not struct \
        and not any(raster.values())
    print('RESULT:', 'IDENTICAL after normalization' if ok else 'DIFFERENT')
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main())
