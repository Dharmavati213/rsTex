# Proves that sample.py refuses what it must on crafted archives, and that its main-file rules hold.
# python3 -I tests/corpus/arxiv/check_extract.py tests/corpus/arxiv/sample.py WORKDIR     (WORKDIR is deleted and recreated)
import gzip, importlib.util, io, os, shutil, sys, tarfile, json
spec = importlib.util.spec_from_file_location('sample', sys.argv[1]); s = importlib.util.module_from_spec(spec); spec.loader.exec_module(s)
work = sys.argv[2]; shutil.rmtree(work, ignore_errors=True); os.makedirs(work + '/papers'); os.makedirs(work + '/outside')
open(work + '/outside/secret', 'w').write('keep')
def tar(members):
    b = io.BytesIO()
    with tarfile.open(fileobj=b, mode='w') as t:
        for name, kind, data in members:
            ti = tarfile.TarInfo(name)
            if kind == 'file':
                ti.size = len(data); ti.mode = 0o755; t.addfile(ti, io.BytesIO(data))
            elif kind == 'sym':
                ti.type = tarfile.SYMTYPE; ti.linkname = data; t.addfile(ti)
            elif kind == 'lnk':
                ti.type = tarfile.LNKTYPE; ti.linkname = data; t.addfile(ti)
            elif kind == 'chr':
                ti.type = tarfile.CHRTYPE; ti.devmajor = 1; ti.devminor = 3; t.addfile(ti)
            elif kind == 'fifo':
                ti.type = tarfile.FIFOTYPE; t.addfile(ti)
            elif kind == 'dir':
                ti.type = tarfile.DIRTYPE; t.addfile(ti)
    return b.getvalue()
DOC = rb'\documentclass{article}\begin{document}hi\end{document}'
evil = tar([('main.tex', 'file', DOC), ('/etc/evil', 'file', b'x'), ('../escape', 'file', b'x'), ('a/../../escape2', 'file', b'x'),
            ('link', 'sym', '../outside/secret'), ('link2', 'sym', '/etc/passwd'), ('hard', 'lnk', 'main.tex'), ('dev', 'chr', ''), ('pipe', 'fifo', ''),
            ('pdflatex.fmt', 'file', b'x'), ('texmf.cnf', 'file', b'x'), ('sub/ls-R', 'file', b'x'), ('back\\slash', 'file', b'x'), ('ctl\x01name', 'file', b'x'),
            ('d', 'dir', ''), ('d/ok.tex', 'file', b'ok')])
raw = work + '/evil'; open(raw, 'wb').write(gzip.compress(evil))
r = s.extract_one('9999.00001v1', raw, work + '/papers')
print(json.dumps(r, indent=None))
root = work + '/papers/9999.00001v1'
found = sorted(os.path.relpath(os.path.join(d, n), root) for d, ds, fs in os.walk(root) for n in fs + ds)
print(found)
assert found == ['d', 'd/ok.tex', 'main.tex'], found
assert r['status'] == 'ok' and r['main'] == 'main.tex'
assert sum(r['refused'].values()) == 13, r['refused']
assert not os.path.exists(work + '/escape') and not os.path.exists(work + '/papers/escape') and not os.path.exists(work + '/papers/escape2')
assert not os.access(root + '/main.tex', os.X_OK), 'exec bit must not survive'
assert not any(os.path.islink(os.path.join(d, n)) for d, ds, fs in os.walk(root) for n in fs + ds)
# a directory that is a symlink planted by an earlier member cannot exist, because symlinks are refused; check a file used as a directory fails softly
r = tar([('x', 'file', b'a'), ('x/y.tex', 'file', DOC), ('m.tex', 'file', DOC)])
raw = work + '/fd'; open(raw, 'wb').write(r); r = s.extract_one('9999.00002v1', raw, work + '/papers'); print(r); assert r['main'] == 'm.tex' and r['refused']
# bombs and forms
raw = work + '/bomb'; open(raw, 'wb').write(gzip.compress(b'\0' * (s.MAX_UNPACKED + 10))); r = s.extract_one('9999.00003v1', raw, work + '/papers'); print(r); assert r['status'] == 'unusable'
big = tar([('m.tex', 'file', DOC)] + [(f'b{i}', 'file', b'\0' * (40 * 1024 * 1024)) for i in range(8)])
raw = work + '/bigtar'; open(raw, 'wb').write(gzip.compress(big)); r = s.extract_one('9999.00004v1', raw, work + '/papers'); print(r); assert r['status'] == 'unusable' and 'unpacked' in r['reason']
for name, blob, want in [('pdf', b'%PDF-1.5 xxx', 'PDF only'), ('gzpdf', gzip.compress(b'%PDF-1.5 xxx'), 'PDF only'), ('ps', gzip.compress(b'%!PS-Adobe'), 'PostScript'),
                         ('html', gzip.compress(b'<html><body>x'), 'HTML'), ('corrupt', b'\x1f\x8bgarbage', 'gzip error'), ('empty', tar([]), 'no TeX main')]:
    raw = work + '/' + name; open(raw, 'wb').write(blob); r = s.extract_one('9999.00005v1', raw, work + '/papers'); print(name, r); assert r['status'] == 'unusable' and want in r['reason'], r
raw = work + '/single'; open(raw, 'wb').write(gzip.compress(DOC)); r = s.extract_one('9999.00006v1', raw, work + '/papers'); print(r); assert r['main'] == 'paper.tex' and r['kind'] == 'gz+single'
# main-file rules
def main_of(files, readme=None):
    members = [(n, 'file', d) for n, d in files.items()]
    raw = work + '/m'; open(raw, 'wb').write(tar(members)); r = s.extract_one('9999.00007v1', raw, work + '/papers'); return r
SD = rb'\documentclass{standalone}\begin{document}x\end{document}'
assert main_of({'a.tex': DOC, 'fig.tex': SD + b' ' * 500})['main'] == 'a.tex'
assert main_of({'supp.tex': DOC + b' ' * 500, 'body.tex': DOC})['main'] == 'body.tex'
assert main_of({'a.tex': DOC + b'\\input{b}', 'b.tex': DOC + b' ' * 500})['main'] == 'a.tex'
assert main_of({'main.tex': DOC, 'zz.tex': DOC + b' ' * 500})['main'] == 'main.tex'
assert main_of({'c.tex': b'% \\documentclass{article}\n% \\begin{document}\n', 'd.tex': DOC})['main'] == 'd.tex'
assert main_of({'x.tex': DOC, 'y.tex': DOC + b' ' * 100, '00README.XXX': b'x.tex toplevelfile\n'})['main'] == 'x.tex'
assert main_of({'x.tex': DOC, 'y.tex': DOC + b' ' * 100, '00README.json': json.dumps({'sources': [{'filename': 'y.tex', 'usage': 'toplevel'}], 'process': {'compiler': 'lualatex'}}).encode()})['compiler_hint'] == 'lualatex'
r = main_of({'x.tex': DOC, '00README.json': b'{not json'}); assert r['main'] == 'x.tex' and r.get('readme_error')
r = main_of({'x.tex': DOC, '00README.XXX': b'../../etc/passwd toplevelfile\n'}); assert r['main'] == 'x.tex' and r.get('readme_error'), r
r = main_of({'p.tex': b'\\input amstex\n\\documentstyle{amsppt}\n\\document\nhi\n\\enddocument\n\\bye\n'}); assert r['mode'] == 'plain', r
r = main_of({'p.tex': b'hello \\bye\n'}); assert r['mode'] == 'plain'
r = main_of({'q.tex': b'\\documentstyle{article}\n\\input epsf\n\\begin{document}x\\end{document}\n'}); assert r['mode'] == 'latex', r
r = main_of({'i.tex': b'%auto-ignore\n' + DOC, 'j.tex': DOC}); assert r['main'] == 'j.tex'
r = main_of({'quo"te.tex': DOC}); assert r['status'] == 'unusable' and 'quote' in r['reason'], r
print('ALL EXTRACTION CHECKS PASS')
assert open(work + '/outside/secret').read() == 'keep'
