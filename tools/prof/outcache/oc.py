#!/usr/bin/env -S python3 -I
"""The output cache against the same binary without it.  Instructions from outside, all threads; a document is named by its id and nothing of it is printed.

    oc.py BINARY DOCS [--edit] [--no-pdf-compare]

Runs, in one work directory and one cache directory:
  base      RSTEX_NO_OUTCACHE=1, with a checkpoint: the reference, its PDF and log kept
  first     empty output cache: what it adds to a first run
  second    all from the first
  third     the same again, counted
With --edit, a copy of the sources with a word added in the middle of the body, with and without the cache from the unedited run.
"""
import argparse, hashlib, json, os, re, shutil, statistics, subprocess, sys, time
W = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
sys.path.insert(0, W + '/tools/prof')
import proflib

class Run:
    def __init__(self, engine, doc, root, cwd=None):
        self.engine, self.doc, self.root = engine, doc, root
        self.work = os.path.join(root, 'o')
        self.cache = os.path.join(root, 'cache')
        self.cwd = cwd
        os.makedirs(self.cache, exist_ok=True)
        os.makedirs(self.work, exist_ok=True)
        proflib.corpus().restore_state(doc.prepare(), self.work)
        self.env = proflib.corpus().entry_env(doc.entry)
        self.env['XDG_CACHE_HOME'] = self.cache
        self.env['RSTEX_CHECKPOINT_POLICY'] = 'always'

    def side_files(self):
        """Back to the last pass's state; the PDF stays, as it does between two runs of a document."""
        keep = self.doc.entry['jobname'] + '.pdf'
        for n in os.listdir(self.work):
            if n != keep:
                p = os.path.join(self.work, n)
                shutil.rmtree(p) if os.path.isdir(p) else os.remove(p)
        proflib.corpus().restore_state(self.doc.prepare(), self.work)

    def go(self, extra=None, counted=True, tag=None):
        argv, cwd, _ = self.engine.command(self.doc, self.work)
        if self.cwd:
            cwd = self.cwd
        env = dict(self.env)
        env.update(extra or {})
        if counted:
            r = proflib.run_counted(argv, cwd, env)
        else:
            r = proflib.run_timed(argv, cwd, env)
        return r

    def out(self):
        res = {}
        j = self.doc.entry['jobname']
        for ext in ('.pdf', '.log', '.aux'):
            p = os.path.join(self.work, j + ext)
            res[ext] = hashlib.sha256(open(p, 'rb').read()).hexdigest() if os.path.exists(p) else None
        return res

def trace(r, doc, extra=None):
    """One more run, with the trace on, that says what it hit."""
    env = dict(extra or {})
    env['RSTEX_OUTCACHE_TRACE'] = '1'
    argv, cwd, _ = r.engine.command(r.doc, r.work)
    if r.cwd: cwd = r.cwd
    e = dict(r.env); e.update(env)
    p = subprocess.run(argv, cwd=cwd, env=e, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    for line in p.stderr.decode('latin-1').splitlines():
        if line.startswith('rstex: outcache:'):
            return line[len('rstex: outcache: '):]
    return None

def instr(res):
    c = res['counters']
    return c['instructions']['value'] / 1e6, c['cycles']['value'] / 1e6, c['page-faults']['value'], res['cpu_ms'], res['wall_ms']

def show(label, res, base=None):
    i, cy, f, cpu, wall = instr(res)
    ratio = f'{i / base:6.3f}' if base else ''
    print(f'  {label:<26}{i:>11.2f} M {cy:>11.1f} Mcyc {f:>7} faults {cpu:>9.1f} cpu {wall:>9.1f} wall  {ratio}', flush=True)

def edit_target(live, doc):
    """The file and the line to change: a line of ordinary text about the middle of the body.  Nothing is printed of it."""
    main = os.path.join(live, doc.entry['main_rel'])
    text = open(main, 'rb').read().decode('latin-1')
    m = re.search(r'\\begin\{document\}', text)
    start = m.end() if m else 0
    target = main
    if len(text) - start < 2000:
        best = None
        for root_, dirs, names in os.walk(live):
            for n in names:
                if n.endswith('.tex'):
                    q = os.path.join(root_, n)
                    z = os.path.getsize(q)
                    if q != main and (best is None or z > best[0]):
                        best = (z, q)
        if best:
            target = best[1]
            text = open(target, 'rb').read().decode('latin-1')
            start = 0
    lines = text[start:].split('\n')
    mid = len(lines) // 2
    for k in list(range(mid, len(lines))) + list(range(mid, 0, -1)):
        if re.match(r'^[A-Za-z][a-z ,.]{20,}$', lines[k]):
            return target, text, start, lines, k
    return None

def apply_edit(t, what):
    target, text, start, lines, k = t
    new = list(lines)
    new[k] += what
    open(target, 'wb').write((text[:start] + '\n'.join(new)).encode('latin-1'))

def log_counts(path):
    c = {}
    for line in open(path):
        kind, i, o, h = line.rstrip('\n').split('\t')
        e = c.setdefault(kind, [0, 0, 0])
        e[0] += 1
        e[1] += int(h)
        e[2] += int(i)
    return {k: {'streams': v[0], 'hits': v[1], 'bytes_in': v[2]} for k, v in sorted(c.items())}

def one(engine, doc, args):
    doc.prepare()
    print(f'{doc.name} ({doc.id})', flush=True)
    root = os.path.join(proflib.out_root(), 'oc', doc.name.replace('/', '_'))
    shutil.rmtree(root, ignore_errors=True)
    r = Run(engine, doc, root)
    no = {'RSTEX_NO_OUTCACHE': '1'}
    # checkpoint in place, sealed
    r.go(no, counted=False)
    time.sleep(2.2)
    r.side_files()
    r.go(no, counted=False)
    r.side_files()
    b = r.go(no)
    ref = r.out()
    show('no cache, warm', b)
    base = instr(b)[0]
    # first run with the cache
    r.side_files()
    f = r.go()
    show('first, writes index', f, base)
    same = r.out() == ref
    # second
    r.side_files()
    s = r.go()
    show('second, hits', s, base)
    same2 = r.out() == ref
    r.side_files()
    print('  trace:', trace(r, doc))
    same3 = r.out() == ref
    idx = 0
    d = os.path.join(r.cache, 'rstex', 'outcache')
    if os.path.isdir(d):
        idx = sum(os.path.getsize(os.path.join(d, n)) for n in os.listdir(d))
    pdfp = os.path.join(r.work, doc.entry["jobname"] + ".pdf")
    print(f'  index bytes {idx}, PDF bytes {os.path.getsize(pdfp) if os.path.exists(pdfp) else None}; identical: first {same}, second {same2}, third {same3}', flush=True)
    if not (same and same2 and same3):
        print('  DIFFERENT OUTPUT', {k: (ref[k] == r.out()[k]) for k in ref})
    return r, base, ref

def edited(engine, doc, args):
    src = doc.entry['cwd']
    size = 0
    for r_, _, names in os.walk(src):
        for n in names:
            try:
                size += os.path.getsize(os.path.join(r_, n))
            except OSError:
                pass
    if size > 400 << 20:
        print(f'  edit: the directory has {size >> 20} MB, skipped')
        return
    root = os.path.join(proflib.out_root(), 'oc', 'e-' + doc.name.replace('/', '_'))
    shutil.rmtree(root, ignore_errors=True)
    os.makedirs(root)
    live = os.path.join(root, 'live')
    shutil.copytree(src, live, symlinks=True, ignore=shutil.ignore_patterns('*.pdf.keep', '.git'))
    t = edit_target(live, doc)
    if t is None:
        print('  edit: no line of plain text found')
        return
    rc = Run(engine, doc, os.path.join(root, 'r'), cwd=live)
    orig_cmd = engine.command
    def cmd(d, out):
        argv, cwd, env = orig_cmd(d, out)
        return [a.replace(src, live) for a in argv], live, env
    rc.engine = type('E', (), {'command': staticmethod(cmd)})
    no = {'RSTEX_NO_OUTCACHE': '1'}
    for scen, what in (('a word', ' moreover'), ('a word and a new symbol', r' moreover $\aleph\Join$')):
        # state: unedited, index in place
        apply_edit(t, '')
        rc.side_files(); rc.go(counted=False)
        time.sleep(2.2)
        rc.side_files(); rc.go(counted=False)
        rc.side_files(); u = rc.go()
        show('unedited, hits', u)
        apply_edit(t, what)
        rc.side_files()
        log = os.path.join(root, 'log.tsv')
        h = rc.go({'RSTEX_OUTCACHE_LOG': log})
        got = rc.out()
        pages_h = None
        show(f'{scen}, with cache', h)
        print('   ', log_counts(log) if os.path.exists(log) else 'no log', 'exit', h['rc'])
        rc.side_files()
        n = rc.go(no)
        ref = rc.out()
        show(f'{scen}, no cache', n)
        print(f'    identical to no cache: {got == ref}; ratio {instr(h)[0] / instr(n)[0]:.3f}', flush=True)
    # An edit of the preamble: the checkpoint is no use, the last PDF still is.
    apply_edit(t, '')
    rc.side_files(); rc.go(counted=False)
    rc.side_files(); rc.go(counted=False)
    main = os.path.join(live, doc.entry['main_rel'])
    text = open(main, 'rb').read().decode('latin-1')
    if '\\begin{document}' in text:
        with_edit = text.replace('\\begin{document}', '\\newcommand{\\zzzqq}{}\n\\begin{document}', 1)
        open(main, 'wb').write(with_edit.encode('latin-1'))
        rc.side_files()
        log = os.path.join(root, 'log.tsv')
        h = rc.go({'RSTEX_OUTCACHE_LOG': log, 'RSTEX_NO_SNAPSHOT_X': '1'})
        got = rc.out()
        show('preamble edited, with cache', h)
        print('   ', log_counts(log) if os.path.exists(log) else 'no log', 'exit', h['rc'])
        rc.side_files()
        # the checkpoint from that run would be used now: a second edit makes the comparison cold again
        open(main, 'wb').write(with_edit.replace('zzzqq', 'zzzqr').encode('latin-1'))
        n = rc.go(no)
        show('preamble edited, no cache', n)
        print(f'    ratio {instr(h)[0] / instr(n)[0]:.3f}', flush=True)
    shutil.rmtree(root, ignore_errors=True)

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('binary')
    ap.add_argument('docs')
    ap.add_argument('--edit', action='store_true')
    args = ap.parse_args()
    engine = proflib.Engine('x', args.binary)
    for doc in proflib.resolve_docs(args.docs.split(',')):
        one(engine, doc, args)
        if args.edit:
            edited(engine, doc, args)

main()
