#!/usr/bin/env python3
"""compare_pdf.py -- differential PDF comparison for rstex against the pdfTeX oracle.

    compare_pdf.py A.pdf B.pdf [--dpi 100] [--no-raster] [--eps 0] [--jobs N] [--diff-dir DIR] [--no-structure]

Page level: decode /Contents with qpdf, split into operators, rewrite resource names (/F48, /Im1, /R7 ...)
to content-derived labels (BaseFont minus subset tag + encoding hash; md5 of XObject data), compare operator
by operator, and report the first divergence.  Also compares page boxes and annotations, and rasterises both
sides with pdftoppm to give a percentage of differing pixels.
Document level (facts that object numbering and compression may not change): Info minus dates/producer,
catalog, outlines, dests, fonts.  Object numbers, /ID, dates, compression, subset tags never count.

Needs qpdf (>= 11) and pdftoppm.  Exit status: 0 all equal, 1 some difference, 2 could not run.
"""
import argparse, base64, hashlib, json, os, re, subprocess, sys, tempfile
from concurrent.futures import ThreadPoolExecutor

REF = re.compile(r'^(\d+) (\d+) R$')
TAG = re.compile(r'\b[A-Z]{6}\+')
VOLATILE = {'/CreationDate', '/ModDate', '/Producer', '/PTEX.Fullbanner', '/Creator'}
md5 = lambda b: hashlib.md5(b).hexdigest()[:10]


class Pdf:
    def __init__(self, path):
        out = subprocess.run(['qpdf', '--json=2', '--json-key=qpdf', '--json-stream-data=inline',
                              '--decode-level=generalized', path, '-'], capture_output=True)
        if not out.stdout:
            sys.exit(f'qpdf failed on {path}: {out.stderr.decode(errors="replace")}')
        self.objs = json.loads(out.stdout)['qpdf'][1]
        self.trailer = self.objs['trailer']['value']
        self.root = self.get(self.trailer['/Root'])
        self.pages, self.page_index = [], {}
        self._walk(self.root['/Pages'], {})

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
            skip = VOLATILE | {'/Parent', '/P', '/Prev', '/Last'}   # keep /First and /Next: enough to walk outlines
            return {k: self.deep(x, seen) for k, x in sorted(v.items()) if k not in skip}
        return v

    # ---------------- resource names -> content-derived labels ----------------
    def font_label(self, ref):
        f = self.get(ref) or {}
        enc = self.deep(f.get('/Encoding'))
        return TAG.sub('', str(f.get('/BaseFont', '?'))) + '~' + md5(json.dumps(enc, sort_keys=True).encode())[:4]

    def res_label(self, kind, name, res, seen):
        table = self.get((self.get(res) or {}).get(kind)) or {}
        ref = table.get(name)
        if ref is None:
            return f'{name}?unresolved'
        if kind == '/Font':
            return '/' + self.font_label(ref).lstrip('/')
        if kind == '/XObject' and isinstance(ref, str) and ref not in seen:
            d = self.get(ref)
            if d.get('/Subtype') == '/Form':
                ops = self.normalize(self.data(ref), d.get('/Resources', res), seen + (ref,))
                body = '\n'.join(' '.join(o) for o in ops).encode()
                return f"/Form:{md5(json.dumps([d.get('/BBox'), d.get('/Matrix')]).encode() + body)}"
            return f"/Image:{md5(self.data(ref))}:{d.get('/Width')}x{d.get('/Height')}"
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
        _, page = self.pages[n]
        return {k: self.deep(page.get(k)) for k in ('/MediaBox', '/CropBox', '/Rotate', '/Annots', '/Group')}

    def document(self):
        fonts = {}
        for ref, o in self.objs.items():
            v = o.get('value') if isinstance(o, dict) else None
            if isinstance(v, dict) and v.get('/Type') == '/Font' and '/Widths' in v:
                fonts[self.font_label(ref[4:])] = self.deep(v)
        return {'Info': self.deep(self.trailer.get('/Info')),
                'Catalog': self.deep({k: x for k, x in self.root.items() if k != '/Pages'}),
                'PageCount': len(self.pages), 'Fonts': fonts}


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
        if not m:
            i += 1; continue
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
    args = ap.parse_args()
    A, B = Pdf(args.a), Pdf(args.b)
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
