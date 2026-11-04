#!/usr/bin/env -S python3 -I
r"""Does a run that takes streams and fonts from the last PDF give what it would have given without?

    python3 -I tests/corpus/snapshot/outcache.py --engine ENGINE [--filter REGEX] [-v]

Each case: a run that leaves a PDF and an index, a change to the world, then a run with the cache, whose PDF, log and .aux must equal those of a run in the same state without it
(RSTEX_NO_OUTCACHE=1).  The trace says how much was taken, and a case can ask for at least or at most so many hits, so that one that takes nothing does not pass for one that
takes everything.  Everything is written under a temporary directory.
"""
import argparse, hashlib, os, re, shutil, struct, subprocess, sys, tempfile, time, zlib

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
sys.path.insert(0, os.path.join(REPO, 'tools', 'prof'))
import proflib  # noqa: E402

DOC = r'''\documentclass{article}
%s
\begin{document}
%s
\end{document}
'''
BODY = 'Hello world. ' * 40 + '\n\n\\newpage\nA second page. ' + 'More text. ' * 30 + '\n\n\\newpage\nThird page.\n'


def png(w, h, seed, alpha=True):
    """A small PNG with an alpha channel, which pdfTeX cannot copy."""
    def chunk(t, d):
        return struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d))
    raw = bytearray()
    for y in range(h):
        raw.append(0)
        for x in range(w):
            raw += bytes(((x * seed) & 255, (y * seed) & 255, (x ^ y) & 255, (x + y) & 255))
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(bytes(raw))) + chunk(b'IEND', b'')


class World:
    def __init__(self, engine, fmt, base):
        self.engine, self.fmt, self.d = engine, fmt, os.path.join(base, 'w')
        self.cache = os.path.join(base, 'cache')
        os.makedirs(self.d)
        os.makedirs(self.cache)
        self.env = {k: os.environ[k] for k in ('PATH', 'HOME') if k in os.environ}
        self.env.update(XDG_CACHE_HOME=self.cache, SOURCE_DATE_EPOCH='1700000000', FORCE_SOURCE_DATE='1', TZ='UTC', LC_ALL='C', RSTEX_CHECKPOINT_POLICY='always')

    def write(self, name, data):
        p = os.path.join(self.d, name)
        with open(p, 'wb') as f:
            f.write(data.encode() if isinstance(data, str) else data)

    def run(self, extra=None, name='main'):
        env = dict(self.env, RSTEX_OUTCACHE_TRACE='1')
        env.update(extra or {})
        argv = [self.engine, '-progname=pdflatex', '-fmt=' + self.fmt, '-interaction=nonstopmode', name + '.tex']
        p = subprocess.run(argv, cwd=self.d, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        err = p.stderr.decode('latin-1')
        m = re.search(r'rstex: outcache: streams (\d+) hits (\d+) .*fonts (\d+) font_hits (\d+) sum_failures (\d+)', err)
        return {'rc': p.returncode, 'stdout': p.stdout, 'streams': int(m.group(1)) if m else 0, 'hits': int(m.group(2)) if m else 0, 'fonts': int(m.group(3)) if m else 0,
                'font_hits': int(m.group(4)) if m else 0, 'sum_failures': int(m.group(5)) if m else 0}

    def outputs(self, name='main'):
        res = {}
        for ext in ('.pdf', '.log', '.aux'):
            p = os.path.join(self.d, name + ext)
            res[ext] = open(p, 'rb').read() if os.path.exists(p) else None
        return res


def index_of(w):
    d = os.path.join(w.cache, 'rstex', 'outcache')
    return [os.path.join(d, n) for n in sorted(os.listdir(d))] if os.path.isdir(d) else []


CASES = []


def case(name, change, files=None, preamble='', body=BODY, min_hits=None, max_hits=None, min_fonts=None, max_fonts=None, sum_failures=None, second_body=None, wait=False, rounds=1):
    CASES.append(dict(name=name, change=change, files=files or {}, preamble=preamble, body=body, min_hits=min_hits, max_hits=max_hits, min_fonts=min_fonts, max_fonts=max_fonts, sum_failures=sum_failures, wait=wait, rounds=rounds))


def edit_main(old, new):
    def f(w):
        p = os.path.join(w.d, 'main.tex')
        t = open(p).read()
        assert old in t, old
        open(p, 'w').write(t.replace(old, new, 1))
    return f


def tree_font(name):
    return subprocess.run(['kpsewhich', name], capture_output=True, text=True, check=True).stdout.strip()


def flip_in_stream(w):
    """One bit in the bytes of a stream of the PDF, with the size unchanged."""
    p = os.path.join(w.d, 'main.pdf')
    b = bytearray(open(p, 'rb').read())
    at = b.find(b'stream\n') + 20
    b[at] ^= 1
    open(p, 'wb').write(b)


def flip_index(w):
    p = index_of(w)[0]
    b = bytearray(open(p, 'rb').read())
    b[len(b) // 2] ^= 4
    open(p, 'wb').write(b)


def cut_pdf(w):
    p = os.path.join(w.d, 'main.pdf')
    b = open(p, 'rb').read()
    open(p, 'wb').write(b[:len(b) // 2])


def other_pdf_same_size(w):
    p = os.path.join(w.d, 'main.pdf')
    b = bytearray(open(p, 'rb').read())
    for i in range(16, len(b)):
        b[i] ^= 0x55
    open(p, 'wb').write(b)


def same_stat_other_font(w):
    """Other bytes in a font in the project, same size and mtime: only the change time shows."""
    p = os.path.join(w.d, 'myfont.pfb')
    st = os.stat(p)
    b = bytearray(open(p, 'rb').read())
    i = b.find(b'/ItalicAngle 0')
    assert i >= 0
    b[i + 13] = ord('1')
    open(p, 'wb').write(b)
    os.utime(p, ns=(st.st_atime_ns, st.st_mtime_ns))


def nothing(w):
    pass


CMR = None
case('nothing changed', nothing, min_hits=4, min_fonts=1, sum_failures=0)
case('a word in the body', edit_main('Third page.', 'Third page. More text.'), min_hits=2, min_fonts=1)
case('a word that needs new glyphs', edit_main('Third page.', 'Third page, with a word.'), min_hits=2, max_fonts=0)
case('a new glyph', edit_main('Third page.', 'Third page Q $\\aleph$.'), max_fonts=1)
case('the page order', edit_main('A second page.', '\\newpage A second page.'), min_fonts=1)
case('a bit of the old PDF', flip_in_stream, sum_failures=None, min_hits=1)
case('the index damaged', flip_index, max_hits=0)
case('the PDF cut short', cut_pdf, max_hits=0)
case('another file of the same size for the PDF', other_pdf_same_size, max_hits=0)
case('no PDF at all', lambda w: os.remove(os.path.join(w.d, 'main.pdf')), max_hits=0)
case('compression off for the run', edit_main('\\begin{document}', '\\pdfcompresslevel=0 \\begin{document}'), max_hits=0)
case('compression too high for the run', edit_main('\\begin{document}', '\\pdfcompresslevel=10 \\begin{document}'))
case('compression off at the last moment', edit_main('Third page.', 'Third page.\\AtEndDocument{\\clearpage\\pdfcompresslevel=0 }'))
case('compression off for the run, with images', edit_main('\\begin{document}', '\\pdfcompresslevel=0 \\begin{document}'), preamble='\\usepackage{graphicx}', body='\\includegraphics{a.png} Hello.\n', files={'a.png': png(32, 32, 3)}, max_hits=0)
case('PDF 1.4', edit_main('\\begin{document}', '\\pdfminorversion=4 \\begin{document}'))
case('PDF 1.4, with images', edit_main('\\begin{document}', '\\pdfminorversion=4 \\begin{document}'), preamble='\\usepackage{graphicx}', body='\\includegraphics{a.png} Hello.\n', files={'a.png': png(32, 32, 3)})
case('no charset in the descriptors', edit_main('\\begin{document}', '\\pdfomitcharset=1 \\begin{document}'), min_fonts=1)
case('a font reencoded next', edit_main('\\pdfmapline{=cmr10 CMR10 <cmr10.pfb}', '\\pdfmapline{=cmr10 CMR10 <8r.enc <cmr10.pfb}'), preamble='\\pdfmapline{=cmr10 CMR10 <cmr10.pfb}', max_fonts=0)
case('the whole font next', edit_main('\\pdfmapline{=cmr10 CMR10 <cmr10.pfb}', '\\pdfmapline{=cmr10 CMR10 <<cmr10.pfb}'), preamble='\\pdfmapline{=cmr10 CMR10 <cmr10.pfb}', max_fonts=0)
case('object streams off', edit_main('\\begin{document}', '\\pdfobjcompresslevel=0 \\begin{document}'))
case('a font stated in the map, slanted next', edit_main('\\pdfmapline{=cmr10 CMR10 <cmr10.pfb}', '\\pdfmapline{=cmr10 CMR10 " .2 SlantFont " <cmr10.pfb}'), preamble='\\pdfmapline{=cmr10 CMR10 <cmr10.pfb}', max_fonts=0)
case('a font of the project, other bytes under the same mtime', same_stat_other_font, preamble='\\pdfmapline{=cmr10 CMR10 <./myfont.pfb}', files={'myfont.pfb': 'cmr10.pfb'}, wait=True, max_fonts=0)
case('a font of the project, not changed', nothing, preamble='\\pdfmapline{=cmr10 CMR10 <./myfont.pfb}', files={'myfont.pfb': 'cmr10.pfb'}, wait=True, min_fonts=1)
case('tounicode on, next', edit_main('\\begin{document}', '\\pdfgentounicode=1 \\begin{document}'))
PNG_PRE = '\\usepackage{graphicx}'
PNG_BODY = 'Before. \\includegraphics{a.png} ' + BODY + ' \\includegraphics{b.png} After.\n'
PNGS = {'a.png': png(64, 40, 3), 'b.png': png(30, 90, 7)}
case('images, nothing changed', nothing, preamble=PNG_PRE, body=PNG_BODY, files=PNGS, min_hits=6, sum_failures=0)
case('an image changed', lambda w: w.write('a.png', png(64, 40, 5)), preamble=PNG_PRE, body=PNG_BODY, files=PNGS, min_hits=3)
case('an image before the others', edit_main('Before.', 'Before. \\includegraphics{c.png}'), preamble=PNG_PRE, body=PNG_BODY, files=dict(PNGS, **{'c.png': png(20, 20, 9)}), min_hits=3)
case('the order of the images', edit_main('\\includegraphics{a.png}', '\\includegraphics{b.png}'), preamble=PNG_PRE, body=PNG_BODY, files=PNGS, min_hits=2)
case('gamma on for images', edit_main('\\begin{document}', '\\pdfimageapplygamma=1 \\pdfgamma=2200 \\begin{document}'), preamble=PNG_PRE, body=PNG_BODY, files=PNGS)
case('the colour space of an image', edit_main('\\begin{document}', '\\immediate\\pdfobj{[/CalGray<</WhitePoint[1 1 1]>>]} \\begin{document}'), preamble=PNG_PRE, body=PNG_BODY, files=PNGS)
case('a fatal error, then a good run', None, body=BODY, min_hits=0)


def run_case(c, engine, fmt, verbose):
    base = tempfile.mkdtemp(prefix='oc-', dir=proflib.out_root())
    try:
        w = World(engine, fmt, base)
        for name, data in c['files'].items():
            if isinstance(data, str) and data.endswith('.pfb'):
                shutil.copy(tree_font(data), os.path.join(w.d, name))
                os.chmod(os.path.join(w.d, name), 0o644)
            else:
                w.write(name, data)
        doc = DOC % (c['preamble'], c['body'])
        w.write('main.tex', doc)
        if c['wait']:
            time.sleep(2.2)
        first = w.run()
        if first['rc'] != 0 and c['change'] is not None:
            pass
        change = c['change']
        if change is None:
            # A run that dies in the middle, with its PDF open, then a good one.
            w.write('main.tex', DOC % (c['preamble'], c['body'].replace('Third page.', 'Third page.\\input{/nonexistent/x}')))
            w.run()
            w.write('main.tex', doc)
        else:
            change(w)
        keep = {}
        pdf = os.path.join(w.d, 'main.pdf')
        if os.path.exists(pdf):
            keep = open(pdf, 'rb').read()
        hit = w.run()
        got = w.outputs()
        no = w.run({'RSTEX_NO_OUTCACHE': '1'})
        ref = w.outputs()
        problems = [k for k in ref if ref[k] != got[k]]
        if hit['rc'] != no['rc'] or hit['stdout'] != no['stdout']:
            problems.append('stdout or status')
        for key, field, low in (('min_hits', 'hits', True), ('max_hits', 'hits', False), ('min_fonts', 'font_hits', True), ('max_fonts', 'font_hits', False), ('sum_failures', 'sum_failures', False)):
            if c[key] is not None and ((low and hit[field] < c[key]) or (not low and hit[field] > c[key])):
                problems.append(f'{field} {hit[field]}, asked {key} {c[key]}')
        if verbose or problems:
            print(f'  {c["name"]}: streams {hit["streams"]} hits {hit["hits"]}, fonts {hit["fonts"]} hits {hit["font_hits"]}, sum failures {hit["sum_failures"]}, rc {hit["rc"]}')
        return problems
    finally:
        shutil.rmtree(base, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--engine', required=True)
    ap.add_argument('--filter')
    ap.add_argument('-v', action='store_true')
    args = ap.parse_args()
    engine = proflib.Engine('x', args.engine)
    fmt = engine.format_path('pdflatex')
    bad = 0
    for c in CASES:
        if args.filter and not re.search(args.filter, c['name']):
            continue
        p = run_case(c, engine.path, fmt, args.v)
        print(('FAIL ' if p else 'ok   ') + c['name'] + (': ' + ', '.join(p) if p else ''), flush=True)
        bad += bool(p)
    print(f'{len(CASES)} cases, {bad} failed')
    sys.exit(1 if bad else 0)


main()
