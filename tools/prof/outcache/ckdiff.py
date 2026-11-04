#!/usr/bin/env -S python3 -I
"""What in a preamble checkpoint depends on the main file's name.  hello under three names, and a stub for how far the class line goes."""
import glob, os, shutil, subprocess, sys, struct
W = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
sys.path.insert(0, W + '/tools/prof')
import proflib
engine = proflib.Engine('x', sys.argv[1])
doc = proflib.resolve_docs(['hello'])[0]
root = os.path.join(proflib.out_root(), 'ckdiff')
shutil.rmtree(root, ignore_errors=True)
os.makedirs(root)
fmt = engine.format_path('pdflatex')
src = open(doc.entry['main'], 'rb').read()
base_env = proflib.corpus().entry_env(doc.entry)
def run(name, content=src, cwd='d', policy='always', counted=False, extra=None):
    d = os.path.join(root, cwd)
    os.makedirs(d, exist_ok=True)
    cache = os.path.join(root, 'cache-' + name)
    os.makedirs(cache, exist_ok=True)
    with open(os.path.join(d, name + '.tex'), 'wb') as f:
        f.write(content)
    env = dict(base_env, XDG_CACHE_HOME=cache, RSTEX_CHECKPOINT_POLICY=policy)
    env.update(extra or {})
    argv = [engine.path, '-progname=pdflatex', '-fmt=' + fmt, '-interaction=nonstopmode', name + '.tex']
    if counted:
        return proflib.run_counted(argv, d, env)
    return proflib.run_timed(argv, d, env)
for n in ('aaaaa', 'bbbbb', 'cccccccc'):
    r = run(n)
    assert r['rc'] == 0, r
cks = {n: glob.glob(os.path.join(root, 'cache-' + n, 'rstex', 'checkpoints', '*.ck')) for n in ('aaaaa', 'bbbbb', 'cccccccc')}
b = {n: open(v[0], 'rb').read() for n, v in cks.items() if v}
print({n: len(v) for n, v in b.items()})
a, c = b['aaaaa'], b['bbbbb']
if len(a) == len(c):
    diff = [i for i in range(len(a)) if a[i] != c[i]]
    print('same length of name: bytes that differ', len(diff), 'first offsets', diff[:6])
    # of a page: how many 4K pages differ
    print('pages that differ', len({i >> 12 for i in diff}), 'of', len(a) >> 12)
a, c = b['aaaaa'], b['cccccccc']
n = min(len(a), len(c))
diff = sum(1 for i in range(n) if a[i] != c[i])
print('other length of name: size', len(a), len(c), 'bytes that differ at the same offset', diff, 'pages', len({i >> 12 for i in range(n) if a[i] != c[i]}), 'of', n >> 12)
# instructions up to each point, no cache, no checkpoint
no = {'RSTEX_NO_SNAPSHOT': '1'}
def instr(content, name='probe'):
    run(name, content, counted=False, extra=no, cwd='p')
    r = run(name, content, counted=True, extra=no, cwd='p')
    return round(r['counters']['instructions']['value'] / 1e6, 2), r['rc']
print('format only, \\stop:', instr(b'\\stop\n'))
print('\\documentclass{article} \\stop:', instr(b'\\documentclass{article}\n\\stop\n'))
print('hello as it is:', instr(src))
print('+amsmath:', instr(b'\\documentclass{article}\n\\usepackage{amsmath}\n\\begin{document}\nHello\n\\end{document}\n'))
print('+amsmath,amssymb,graphicx,hyperref:', instr(b'\\documentclass{article}\n\\usepackage{amsmath,amssymb,graphicx}\n\\usepackage{hyperref}\n\\begin{document}\nHello\n\\end{document}\n'))
print('[11pt]{article}:', instr(b'\\documentclass[11pt]{article}\n\\begin{document}\nHello\n\\end{document}\n'))
print('article, begin document only, end:', instr(b'\\documentclass{article}\n\\begin{document}\n\\stop\n'))
