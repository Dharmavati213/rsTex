#!/usr/bin/env -S python3 -I
r"""What does the corpus exercise that `run.py coverage` cannot see?  Node types, modes, PDF structure, PDF operators, Type 1 features.

    python3 -I tests/corpus/census.py pdf   [--manifest FILE]... [-j N]          # every oracle PDF: structure keys, annotations, fonts, images
    python3 -I tests/corpus/census.py ops   [--sample N] [-j N]                  # a stratified sample: PDF content operators
    python3 -I tests/corpus/census.py nodes [--sample N] [-j N] [--max-ms M]     # a stratified sample, run again with box dumps and command tracing
    python3 -I tests/corpus/census.py type1 [--manifest FILE]...                 # every .pfb any oracle read, and every .pfb the map file names
    python3 -I tests/corpus/census.py report

The point is the lists it ends with: what was looked for and never seen.  Each category has a fixed list of what exists, from tex.web, pdftex.web or the PDF
specification, so "never seen" has a denominator.  Results go to target/corpus/audit/census-*.json.

Blind spots, stated: a math style is not printed by any trace, so styles are only counted through the commands that select them, and tests/corpus/gen/mathmatrix.py is what reaches
them by construction.  A node that is created and then discarded before shipout (say, a \discretionary replaced in the second pass, or material in a box never shipped) is
not in a box dump.  Only real code coverage of the engine can close that: see "Line coverage" in tests/corpus/README.md.
"""
import argparse
import collections
import concurrent.futures as cf
import importlib.util
import json
import os
import random
import re
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location('corpus_audit', os.path.join(HERE, 'audit.py'))
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)
run = audit.run
OUT = os.path.join(run.TARGET, 'audit')

# What exists.  Node display names: tex.web part 12, pdftex.web and e-TeX additions, as printed by show_box.
NODE_KINDS = ['hbox', 'vbox', 'rule', 'glue', 'kern', 'penalty', 'mathon', 'mathoff', 'discretionary', 'mark', 'insert', 'vadjust', 'special', 'write', 'openout', 'closeout',
              'pdfliteral', 'pdfcolorstack', 'pdfsetmatrix', 'pdfsave', 'pdfrestore', 'pdfobj', 'pdfrefobj', 'pdfxform', 'pdfrefxform', 'pdfximage', 'pdfrefximage',
              'pdfannot', 'pdfstartlink', 'pdfendlink', 'pdfdest', 'pdfthread', 'pdfstartthread', 'pdfendthread', 'pdfsavepos', 'pdfsnapy', 'pdfsnaprefpoint', 'pdfsnapycomp',
              'pdfinterwordspaceon', 'pdfinterwordspaceoff', 'pdffakespace', 'pdfrunninglinkoff', 'pdfrunninglinkon',
              'beginL', 'endL', 'beginR', 'endR', 'beginM', 'endM', 'leaders', 'cleaders', 'xleaders', 'setlanguage', 'marks']
NODE_FEATURES = {'glue set stretch': rb'glue set \d', 'glue set shrink': rb'glue set - ', 'glue fil': rb'glue set [-\s\d.]*fil\b', 'glue fill': rb'glue set [-\s\d.]*fill\b', 'glue filll': rb'glue set [-\s\d.]*filll',
                 'shifted box': rb', shifted ', 'display box': rb', display', 'reflected (TeXXeT)': rb"reflected", 'ligature': rb'\(ligature ', 'implicit kern (font)': rb'\\kern-?\d', 'kern (for accent)': rb'\\kern-?[\d.]+ \(for accent\)',
                 'explicit kern': rb'\\kern ', 'math kern': rb'\\kern-?[\d.]+$|\\mkern', 'named glue': rb'\\glue\(\\', 'mskip glue': rb'\\glue\(\\(?:thin|med|thick)muskip', 'non-script': rb'\\nonscript', 'font expansion kern/char': rb'\+-?\d+\)', 'margin kern': rb'\\kern-?[\d.]+ \(left margin\)|\(right margin\)', 'pdfstartlink attr': rb'\\pdfstartlink\(', 'unset box': rb'unset', 'leaders box': rb'leaders [\d.]+', 'penalty': rb'\\penalty -?\d'}
MODES = ['vertical mode', 'horizontal mode', 'restricted horizontal mode', 'internal vertical mode', 'math mode', 'display math mode', 'no mode']
MATH_COMMANDS = ['displaystyle', 'textstyle', 'scriptstyle', 'scriptscriptstyle', 'mathchoice', 'nonscript', 'mathord', 'mathop', 'mathbin', 'mathrel', 'mathopen', 'mathclose', 'mathpunct', 'mathinner', 'underline', 'overline', 'mathaccent', 'radical', 'vcenter', 'limits', 'nolimits', 'displaylimits', 'left', 'middle', 'right', 'over', 'atop', 'above', 'overwithdelims', 'atopwithdelims', 'abovewithdelims', 'eqno', 'leqno', 'mkern', 'mskip', 'mathchar', 'delimiter', 'fam', 'mathsurround', 'scriptspace', 'delimitershortfall', 'delimiterfactor', 'nulldelimiterspace', 'binoppenalty', 'relpenalty', 'displaywidowpenalty', 'predisplaypenalty', 'postdisplaypenalty', 'abovedisplayskip', 'belowdisplayskip', 'abovedisplayshortskip', 'belowdisplayshortskip', 'halign', 'noalign', 'omit', 'span', 'valign', 'cr', 'crcr', 'parshape', 'interlinepenalties', 'clubpenalties', 'widowpenalties', 'displaywidowpenalties', 'looseness', 'hangindent', 'hangafter', 'lastlinefit', 'emergencystretch', 'hbadness', 'vsplit', 'insert', 'vadjust', 'mark', 'marks', 'splitfirstmark', 'pagegoal', 'pagetotal', 'pagedepth', 'prevgraf', 'accent', 'char', 'noboundary', 'discretionary', 'beginL', 'beginR', 'TeXXeTstate', 'showlists', 'showbox', 'showhyphens', 'patterns', 'hyphenation', 'lccode', 'uchyph', 'lefthyphenmin', 'righthyphenmin', 'hyphenchar', 'skewchar', 'fontdimen', 'tracingparagraphs', 'tracingpages']
# Keys and values in the PDF specification that pdfTeX can write, by way of a primitive or a package.
PDF_EXPECT = {
    'catalog': ['/Outlines', '/Names', '/Dests', '/PageMode', '/PageLayout', '/OpenAction', '/PageLabels', '/AcroForm', '/Metadata', '/ViewerPreferences', '/OCProperties', '/Threads', '/StructTreeRoot', '/MarkInfo', '/Lang', '/URI', '/AA', '/OutputIntents', '/Extensions', '/Version'],
    'page': ['/Annots', '/Group', '/CropBox', '/TrimBox', '/BleedBox', '/ArtBox', '/Rotate', '/UserUnit', '/Trans', '/Dur', '/Thumb', '/B', '/Tabs', '/Metadata', '/StructParents', '/AA', '/LastModified', '/SeparationInfo', '/PieceInfo'],
    'annot': ['/Link', '/Text', '/FreeText', '/Widget', '/Stamp', '/Highlight', '/Square', '/Circle', '/Line', '/FileAttachment', '/Screen', '/RichMedia', '/Movie', '/Sound', '/3D', '/Popup'],
    'action': ['/GoTo', '/GoToR', '/GoToE', '/URI', '/Launch', '/Named', '/JavaScript', '/Thread', '/SubmitForm', '/ResetForm', '/Hide', '/SetOCGState', '/Rendition', '/Trans'],
    'font': ['/Type1', '/TrueType', '/Type3', '/Type0', '/MMType1', '/CIDFontType0', '/CIDFontType2'],
    'fontfile': ['/FontFile', '/FontFile2', '/FontFile3', 'none (not embedded)'],
    'image_filter': ['/DCTDecode', '/FlateDecode', '/JBIG2Decode', '/JPXDecode', '/CCITTFaxDecode', '/LZWDecode', 'none'],
    'colorspace': ['/DeviceGray', '/DeviceRGB', '/DeviceCMYK', '/Indexed', '/ICCBased', '/CalGray', '/CalRGB', '/Lab', '/Separation', '/DeviceN', '/Pattern'],
    'image_attr': ['/SMask', '/Mask', '/Decode', '/Interpolate', '/Intent', '/ImageMask', '/SMaskInData', '/Alternates', '/OC', '/PTEX.FileName', '/PTEX.PageNumber'],
    'form_attr': ['/Group', '/Matrix', '/PTEX.FileName', '/PTEX.PageNumber', '/PTEX.InfoDict', '/Ref', '/OC', '/LastModified'],
    'trailer': ['/Info', '/ID', '/Root'],
    'info': ['/Title', '/Author', '/Subject', '/Keywords', '/Creator', '/Producer', '/CreationDate', '/ModDate', '/Trapped', '/PTEX.Fullbanner'],
    'extgstate': ['/CA', '/ca', '/BM', '/SMask', '/LW', '/LC', '/LJ', '/ML', '/D', '/AIS', '/TK', '/OP', '/op', '/OPM'],
    'container': ['xref table', 'xref stream', 'object streams', 'PDF 1.3', 'PDF 1.4', 'PDF 1.5', 'PDF 1.6', 'PDF 1.7', 'PDF 2.0', 'uncompressed streams', 'compressed streams'],
    'font_transform': ['Slant', 'Extend', 'whole font embedded (no subset tag)', 'subset', 'custom /Encoding with /Differences', 'font with no ToUnicode'],
}
OPERATORS = ('b B b* B* BDC BI BMC BT BX c cm CS cs d d0 d1 Do DP EI EMC ET EX f F f* G g gs h i ID j J K k l m M MP n q Q re RG rg ri s S SC sc SCN scn sh T* Tc Td TD Tf TJ Tj TL Tm Tr Ts Tw Tz v w W W* y \' "').split()


def normal_entries(manifests, skip_load_tests=False):
    entries, _, _ = run.load_manifests(manifests)
    res = []
    for e in entries:
        if e.get('excluded') or e['source'] == 'cd-local':
            continue
        if skip_load_tests and re.match(r'coverage/(pkg|cls|babel-)', e['id']):
            continue
        meta = audit.latest_meta(e)
        if meta and run.effective_class(meta, e) != 'oracle-fails':
            res.append((e, meta))
    return res


def walk_values(v):
    if isinstance(v, dict):
        for x in v.values():
            yield from walk_values(x)
    elif isinstance(v, list):
        for x in v:
            yield from walk_values(x)
    else:
        yield v


def pdf_features(path):
    p = subprocess.run(['qpdf', '--json=2', '--json-key=qpdf', '--json-stream-data=none', '--decode-level=none', path, '-'], capture_output=True)
    if not p.stdout:
        return None
    try:
        objs = json.loads(p.stdout)['qpdf'][1]
    except ValueError:
        return None
    f = collections.defaultdict(set)
    trailer = objs['trailer']['value']

    def get(v):
        while isinstance(v, str) and re.match(r'^\d+ \d+ R$', v):
            o = objs.get('obj:' + v, {})
            v = o['stream']['dict'] if 'stream' in o else o.get('value')
        return v
    with open(path, 'rb') as fh:
        head = fh.read(8).decode('latin-1')
    f['container'].add('PDF ' + head[5:8])
    f['container'].add('xref stream' if trailer.get('/Type') == '/XRef' else 'xref table')
    root = get(trailer.get('/Root')) or {}
    font_labels = set()
    f['catalog'] |= set(root)
    names = get(root.get('/Names')) or {}
    f['names'] |= set(names)
    f['trailer'] |= {k for k in trailer if k not in ('/Type', '/W', '/Index', '/Length', '/Filter', '/Size', '/DecodeParms')}
    f['info'] |= set(get(trailer.get('/Info')) or {})
    for k, o in objs.items():
        if not k.startswith('obj:') or not isinstance(o, dict):
            continue
        if 'stream' in o:
            d = o['stream']['dict']
            t = d.get('/Subtype')
            fl = d.get('/Filter')
            f['container'].add('compressed streams' if fl else 'uncompressed streams')
            if d.get('/Type') == '/ObjStm':
                f['container'].add('object streams')
            if t == '/Image':
                f['image_filter'].add(str(fl) if isinstance(fl, str) else ('none' if not fl else json.dumps(fl)))
                cs = get(d.get('/ColorSpace'))
                f['colorspace'].add(cs if isinstance(cs, str) else (str(cs[0]) if isinstance(cs, list) and cs else 'none'))
                f['image_attr'] |= {x for x in d if x not in ('/Type', '/Subtype', '/Width', '/Height', '/BitsPerComponent', '/ColorSpace', '/Filter', '/Length', '/DecodeParms')}
                f['image_bpc'].add(str(d.get('/BitsPerComponent')))
            elif t == '/Form':
                f['form_attr'] |= {x for x in d if x not in ('/Type', '/Subtype', '/BBox', '/FormType', '/Resources', '/Filter', '/Length')}
            continue
        v = o.get('value')
        if not isinstance(v, dict):
            continue
        ty = v.get('/Type')
        if ty == '/Page':
            f['page'] |= {x for x in v if x not in ('/Type', '/Parent', '/Contents', '/Resources', '/MediaBox')}
        elif ty == '/Annot' or ('/Rect' in v and '/Subtype' in v):
            f['annot'].add(v.get('/Subtype', '?'))
            f['annot_key'] |= set(v)
        stack = [v]                 # actions are usually direct objects inside an annotation or an outline item
        while stack:
            x = stack.pop()
            if isinstance(x, dict):
                if isinstance(x.get('/S'), str) and x.get('/Type') != '/Group' and any(k in x for k in ('/D', '/URI', '/F', '/N', '/JS', '/Thread', '/Fields', '/T', '/OCGs', '/Trans', '/R')):
                    f['action'].add(x['/S'])
                stack.extend(x.values())
            elif isinstance(x, list):
                stack.extend(x)
        if ty == '/Font':
            f['font'].add(v.get('/Subtype', '?'))
            bf = str(v.get('/BaseFont', ''))
            if 'Slant' in bf:
                f['font_transform'].add('Slant')
            if 'Extend' in bf:
                f['font_transform'].add('Extend')
            if re.match(r'^/[A-Z]{6}\+', bf):
                f['font_transform'].add('subset')
            elif '/FontDescriptor' in v:
                f['font_transform'].add('whole font embedded (no subset tag)')
            if '/Encoding' in v and isinstance(get(v['/Encoding']), dict):
                f['font_transform'].add('custom /Encoding with /Differences')
            if '/ToUnicode' not in v:
                f['font_transform'].add('font with no ToUnicode')
            label = (re.sub(r'^/[A-Z]{6}\+', '', bf), json.dumps(get(v.get('/Encoding')), sort_keys=True))
            if label in font_labels:
                f['container'].add('two font objects with one BaseFont and encoding')
            font_labels.add(label)
            fd = get(v.get('/FontDescriptor'))
            if isinstance(fd, dict):
                ff = [x for x in ('/FontFile', '/FontFile2', '/FontFile3') if x in fd]
                f['fontfile'] |= set(ff) or {'none (not embedded)'}
        if ty == '/ExtGState' or any(x in v for x in ('/CA', '/ca', '/BM')):
            f['extgstate'] |= {x for x in v if x != '/Type'}
    return f


def cmd_pdf(args):
    items = normal_entries(args.manifest)
    items = [(e, m) for e, m in items if m.get('pdf')]

    def one(item):
        e, m = item
        return e['id'], e['source'], pdf_features(os.path.join(m['dir'], 'final', m['jobname'] + '.pdf'))

    unreadable = []

    seen = collections.defaultdict(collections.Counter)
    example = {}
    bad = 0
    with cf.ThreadPoolExecutor(args.jobs) as ex:
        for i, src, f in ex.map(one, items):
            if f is None:
                bad += 1
                unreadable.append(i)
                continue
            for cat, values in f.items():
                for x in values:
                    seen[cat][x] += 1
                    example.setdefault(f'{cat}:{x}', i)
    run.write_json(os.path.join(OUT, 'census-pdf.json'), {'documents': len(items), 'unreadable': bad, 'unreadable_entries': unreadable, 'seen': seen, 'example': example})
    print(f'pdf census over {len(items)} PDFs, {bad} unreadable')
    return 0


def content_ops(path):
    p = subprocess.run(['qpdf', '--json=2', '--json-key=qpdf', '--json-stream-data=inline', '--decode-level=generalized', path, '-'], capture_output=True)
    if not p.stdout:
        return {}
    import base64
    try:
        objs = json.loads(p.stdout)['qpdf'][1]
    except ValueError:
        return {}
    counts = collections.Counter()
    tr, tz = set(), set()
    for o in objs.values():
        if not (isinstance(o, dict) and 'stream' in o):
            continue
        d = o['stream']['dict']
        if d.get('/Type') in ('/ObjStm', '/XRef', '/Metadata') or d.get('/Subtype') == '/Image' or '/Length1' in d or '/FunctionType' in d or '/ShadingType' in d:
            continue
        data = o['stream'].get('data')
        if not data:
            continue
        body = base64.b64decode(data)
        if b'begincmap' in body:
            continue
        clean = re.sub(rb'\((?:[^()\\]|\\.)*\)', b'()', body)
        clean = re.sub(rb'<[0-9A-Fa-f\s]*>', b'<>', clean)
        for w in re.findall(rb"(?<![/\w.+-])[A-Za-z'\"][A-Za-z0-9*]*(?![/\w])", clean):
            counts[w.decode('latin-1')] += 1
        tr |= set(re.findall(rb'(\d+) Tr\b', clean))
        for m in re.finditer(rb'([-\d.]+) ([-\d.]+) ([-\d.]+) ([-\d.]+) [-\d.]+ [-\d.]+ Tm', clean):
            if m.group(1) not in (b'1', m.group(4)):
                tz.add(b'x')
    return {'ops': counts, 'tr': sorted(x.decode() for x in tr), 'tm_scaled': bool(tz)}


def stratified(items, n, seed):
    by = collections.defaultdict(list)
    for x in items:
        by[x[0]['source']].append(x)
    rng = random.Random(seed)
    quota = {s: max(3, round(n * len(v) / len(items))) for s, v in by.items()}
    res = []
    for s, v in sorted(by.items()):
        v = sorted(v, key=lambda x: x[0]['id'])
        res += rng.sample(v, min(quota[s], len(v)))
    return res


def cmd_ops(args):
    items = [(e, m) for e, m in normal_entries(args.manifest) if m.get('pdf')]
    sample = stratified(items, args.sample, args.seed)

    def one(item):
        e, m = item
        return content_ops(os.path.join(m['dir'], 'final', m['jobname'] + '.pdf'))

    total = collections.Counter()
    tr = collections.Counter()
    scaled = 0
    with cf.ThreadPoolExecutor(args.jobs) as ex:
        for r in ex.map(one, sample):
            total.update(r.get('ops', {}))
            for t in r.get('tr', []):
                tr[t] += 1
            scaled += bool(r.get('tm_scaled'))
    run.write_json(os.path.join(OUT, 'census-ops.json'), {'sample': len(sample), 'of': len(items), 'ops': total, 'text_render_modes': tr, 'documents_with_scaled_Tm': scaled})
    print(f'content operators in a sample of {len(sample)} of {len(items)} PDFs')
    return 0


BOX_LINE = re.compile(rb'^\.*\\([A-Za-z]+)')
TRACE = re.compile(rb'^\{([^}\n]*)')


def node_census(e, meta):
    w = run.new_workdir()
    out = os.path.join(w, 'o')
    env = run.entry_env(e)
    env.update(max_print_line='1000000', error_line='254', half_error_line='238')
    prelude = run.BOX_PRELUDE + r'\tracingcommands=2 '
    res = {'nodes': collections.Counter(), 'features': collections.Counter(), 'modes': collections.Counter(), 'commands': collections.Counter(), 'lines': 0}
    try:
        run.restore_state(meta, out)
        r = run.run_proc(run.tex_argv(e, run.oracle_engine(e), [], out, prelude=prelude), e['cwd'], env, e['timeout'] * 10, os.path.join(w, 'stdout'))
        res['rc'] = r['rc']
        path = os.path.join(out, e['jobname'] + '.log')
        if not os.path.isfile(path):
            return res
        features = [(k, re.compile(v)) for k, v in NODE_FEATURES.items()]
        with open(path, 'rb') as f:
            for line in f:
                res['lines'] += 1
                if line.startswith((b'.', b'\\')):
                    m = BOX_LINE.match(line)
                    if m and (line.startswith(b'.') or line.startswith((b'\\hbox(', b'\\vbox('))):
                        res['nodes'][m.group(1).decode('latin-1')] += 1
                    if line.startswith(b'.'):
                        for k, rx in features:
                            if rx.search(line):
                                res['features'][k] += 1
                elif line.startswith(b'{'):
                    m = TRACE.match(line)
                    if m:
                        body = m.group(1)
                        for mode in MODES:
                            if body.startswith(mode.encode() + b':'):
                                res['modes'][mode] += 1
                                body = body[len(mode) + 2:]
                                break
                        c = re.match(rb'^[^A-Za-z]?([A-Za-z]+)', body)
                        if c:
                            res['commands'][c.group(1).decode('latin-1')] += 1
        return res
    finally:
        shutil.rmtree(w, ignore_errors=True)


def cmd_nodes(args):
    items = normal_entries(args.manifest)
    items = [(e, m) for e, m in items if e['mode'] in ('latex', 'plain') and e['compare'] == 'pdf' and sum(m.get('pass_ms') or [0]) / max(1, len(m.get('pass_ms') or [1])) <= args.max_ms]
    if args.filter:
        items = [x for x in items if re.search(args.filter, x[0]['id'])]
    sample = stratified(items, args.sample, args.seed) if args.sample < len(items) else items
    run.install_limits()
    total = {k: collections.Counter() for k in ('nodes', 'features', 'modes', 'commands')}
    n = 0
    with cf.ThreadPoolExecutor(args.jobs) as ex:
        for r in ex.map(lambda x: node_census(*x), sample):
            for k in total:
                total[k].update(r[k])
            n += 1
    run.write_json(os.path.join(OUT, f'census-{args.name}.json'), {'sample': n, 'of': len(items), 'max_ms': args.max_ms, 'filter': args.filter, **total})
    print(f'node census over {n} of {len(items)} entries under {args.max_ms} ms per pass')
    return 0


def decrypt(data, key, skip):
    out = bytearray()
    r = key
    for c in data:
        out.append(c ^ (r >> 8))
        r = ((c + r) * 52845 + 22719) & 0xFFFF
    return bytes(out[skip:])


def pfb_clear(path):
    data = run.open_bytes(path)
    if data[:1] != b'\x80':
        k = data.find(b'eexec')
        return data[:k], bytes.fromhex(re.sub(rb'\s', b'', data[k + 6:]).decode('latin-1').split('0000000000')[0][:len(data)]) if False else data[k + 6:], 'pfa'
    clear, enc, i = b'', b'', 0
    while i + 6 <= len(data) and data[i] == 0x80 and data[i + 1] != 3:
        kind = data[i + 1]
        n = int.from_bytes(data[i + 2:i + 6], 'little')
        if kind == 1 and not enc:
            clear += data[i + 6:i + 6 + n]
        elif kind == 2:
            enc += data[i + 6:i + 6 + n]
        i += 6 + n
    return clear, enc, 'pfb'


def type1_features(path):
    clear, enc, form = pfb_clear(path)
    f = {'form:' + form}
    if form == 'pfa':
        return f
    priv = decrypt(enc, 55665, 4)
    m = re.search(rb'/lenIV\s+(-?\d+)', priv)
    leniv = int(m.group(1)) if m else 4
    f.add(f'lenIV {leniv}')
    if b'StandardEncoding def' in clear:
        f.add('StandardEncoding')
    elif b'/Encoding' in clear:
        f.add('custom built-in encoding')
    if b'/FontMatrix' in clear and not re.search(rb'/FontMatrix\s*\[\s*0?\.001\s+0\s+0\s+0?\.001\s+0\s+0\s*\]', clear):
        f.add('FontMatrix other than 0.001')
    if re.search(rb'/FontType\s+1\b', clear) is None:
        f.add('FontType not 1')
    if b'/WeightVector' in clear or b'/BlendDesignPositions' in clear:
        f.add('multiple master')
    if b'/UniqueID' not in clear:
        f.add('no UniqueID')
    if re.search(rb'/PaintType\s+[12]', clear):
        f.add('PaintType stroked')
    if b'/Subrs' not in priv:
        f.add('no Subrs')
    if b'/ForceBold true' in priv:
        f.add('ForceBold')
    for rd, _ in ((rb'RD', 0), (rb'-\|', 0)):
        if re.search(rb'\d+ ' + rd + rb' ', priv):
            f.add('charstring marker ' + rd.decode().replace('\\', ''))
    ops = collections.Counter()
    pos = 0
    pat = re.compile(rb'(?:/[^\s/]+|dup \d+) (\d+) (?:RD|-\||\S+) ')
    while True:
        m = pat.search(priv, pos)
        if not m:
            break
        n = int(m.group(1))
        cs = priv[m.end():m.end() + n]
        pos = m.end() + n
        plain = decrypt(cs, 4330, leniv) if leniv >= 0 else cs
        i, last_num = 0, None
        while i < len(plain):
            b = plain[i]
            if b >= 32:
                if b <= 246:
                    last_num = b - 139
                    i += 1
                elif b <= 250:
                    last_num = (b - 247) * 256 + plain[i + 1] + 108 if i + 1 < len(plain) else 0
                    i += 2
                elif b <= 254:
                    last_num = -(b - 251) * 256 - (plain[i + 1] if i + 1 < len(plain) else 0) - 108
                    i += 2
                else:
                    last_num = int.from_bytes(plain[i + 1:i + 5], 'big', signed=True)
                    i += 5
            elif b == 12:
                e = plain[i + 1] if i + 1 < len(plain) else -1
                ops[f'escape {e}'] += 1
                if e == 16:
                    ops[f'othersubr {last_num}'] += 1
                i += 2
            else:
                ops[f'op {b}'] += 1
                i += 1
    names = {'escape 0': 'dotsection', 'escape 1': 'vstem3', 'escape 2': 'hstem3', 'escape 6': 'seac', 'escape 7': 'sbw', 'escape 12': 'div', 'escape 16': 'callothersubr', 'escape 17': 'pop', 'escape 33': 'setcurrentpoint',
             'othersubr 0': 'flex end (othersubr 0)', 'othersubr 1': 'flex start (othersubr 1)', 'othersubr 2': 'flex point (othersubr 2)', 'othersubr 3': 'hint replacement (othersubr 3)'}
    for k in ops:
        if k in names:
            f.add(names[k])
        elif k.startswith('othersubr'):
            f.add(k + ' (nonstandard)')
    return f


def cmd_type1(args):
    entries = normal_entries(args.manifest)
    read = set()
    for e, m in entries:
        read |= {p for p in m['inputs'] if p.endswith('.pfb')}
    mapped = set()
    data = run.open_bytes('/var/lib/texmf/fonts/map/pdftex/updmap/pdftex.map')
    names = set(x.decode() for x in re.findall(rb'<<?\[?([A-Za-z0-9._+-]+\.pfb)', data))
    maplines = collections.Counter()
    for line in data.split(b'\n'):
        if b'SlantFont' in line:
            maplines['SlantFont'] += 1
        if b'ExtendFont' in line:
            maplines['ExtendFont'] += 1
        if b'ReEncodeFont' in line:
            maplines['ReEncodeFont'] += 1
        if b'<<' in line:
            maplines['whole-font embed (<<)'] += 1
        if b'<[' in line:
            maplines['encoding only (<[)'] += 1
        if line and not line.startswith(b'%') and b'<' not in line:
            maplines['no font file at all'] += 1
    for n in names:
        p = subprocess.run(['kpsewhich', n], capture_output=True, text=True).stdout.strip()
        if p:
            mapped.add(p)
    feats_read, feats_all = collections.Counter(), collections.Counter()
    example = {}
    for path in sorted(mapped | read):
        try:
            fs = type1_features(path)
        except Exception as ex:
            fs = {f'parse error: {type(ex).__name__}'}
        for x in fs:
            feats_all[x] += 1
            example.setdefault(x, os.path.basename(path))
            if path in read:
                feats_read[x] += 1
    run.write_json(os.path.join(OUT, 'census-type1.json'), {'pfb_read': len(read), 'pfb_mapped': len(mapped), 'read': feats_read, 'all': feats_all, 'example': example, 'map_lines': maplines})
    print(f'type 1 census: {len(read)} .pfb read by some oracle, {len(mapped)} found through the map file')
    return 0


def cmd_report(args):
    def load(name):
        try:
            return run.read_json(os.path.join(OUT, f'census-{name}.json'))
        except (OSError, ValueError):
            return None
    pdf = load('pdf')
    if pdf:
        print(f'PDF structure, over {pdf["documents"]} oracle PDFs (coverage load tests included): how many documents have each')
        for cat, expected in PDF_EXPECT.items():
            seen = pdf['seen'].get(cat, {})
            never = [x for x in expected if x not in seen]
            print(f'  {cat}: seen ' + ', '.join(f'{k}:{v}' for k, v in sorted(seen.items(), key=lambda x: -x[1])[:16]))
            print(f'     NEVER: {", ".join(never) or "nothing on the list"}')
        print(f'  names trees: {pdf["seen"].get("names")}')
        print(f'  image bits: {pdf["seen"].get("image_bpc")}')
    ops = load('ops')
    if ops:
        print(f'\nPDF content operators, in a sample of {ops["sample"]} of {ops["of"]} PDFs:')
        print('  seen: ' + ', '.join(f'{k}:{v}' for k, v in sorted(ops['ops'].items(), key=lambda x: -x[1]) if k in OPERATORS))
        print('  NEVER: ' + ', '.join(x for x in OPERATORS if x not in ops['ops']))
        print(f'  text render modes: {ops["text_render_modes"]}; documents with a horizontally scaled Tm: {ops["documents_with_scaled_Tm"]}')
    for name in ('nodes', 'nodes-real', 'nodes-generated'):
        nodes = load(name)
        if not nodes:
            continue
        print(f'\nNode types in box dumps ({name}), over {nodes["sample"]} of {nodes["of"]} entries of at most {nodes["max_ms"]} ms per pass' + (f', ids matching {nodes["filter"]}' if nodes.get('filter') else '') + ':')
        print('  seen: ' + ', '.join(f'{k}:{v}' for k, v in sorted(nodes['nodes'].items(), key=lambda x: -x[1]) if k in NODE_KINDS))
        print('  NEVER: ' + ', '.join(x for x in NODE_KINDS if x not in nodes['nodes']))
        print('  node features: ' + ', '.join(f'{k}:{v}' for k, v in sorted(nodes['features'].items(), key=lambda x: -x[1])))
        print('  NEVER (features): ' + ', '.join(x for x in NODE_FEATURES if x not in nodes['features']))
        print('  modes: ' + ', '.join(f'{k}:{v}' for k, v in sorted(nodes['modes'].items(), key=lambda x: -x[1])))
        print('  math and paragraph commands never traced: ' + ', '.join(x for x in MATH_COMMANDS if x not in nodes['commands']))
    t1 = load('type1')
    if t1:
        print(f'\nType 1, {t1["pfb_read"]} .pfb files read by some oracle of {t1["pfb_mapped"]} that the map file can reach:')
        for k, v in sorted(t1['all'].items(), key=lambda x: -x[1]):
            print(f'  {k:<38} {t1["read"].get(k, 0):>5} read of {v:>5} that exist; e.g. {t1["example"].get(k)}')
        print(f'  map lines: {t1["map_lines"]}')
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest='cmd', required=True)
    for name, func in (('pdf', cmd_pdf), ('ops', cmd_ops), ('nodes', cmd_nodes), ('type1', cmd_type1), ('report', cmd_report)):
        p = sub.add_parser(name)
        p.add_argument('--manifest', action='append')
        p.add_argument('-j', '--jobs', type=int, default=4)
        p.add_argument('--sample', type=int, default=300)
        p.add_argument('--seed', type=int, default=1)
        p.add_argument('--max-ms', type=float, default=2000)
        p.add_argument('--filter', help='nodes: only ids matching this')
        p.add_argument('--name', default='nodes', help='nodes: results go to census-NAME.json')
        p.set_defaults(func=func)
    args = ap.parse_args()
    os.makedirs(OUT, exist_ok=True)
    os.makedirs(run.WORK_ROOT, exist_ok=True)
    os.makedirs(run.EMPTY_TEXMF, exist_ok=True)
    return args.func(args)


if __name__ == '__main__':
    sys.exit(main())
