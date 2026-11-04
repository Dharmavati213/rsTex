#!/usr/bin/env -S python3 -I
"""CPU and wall in turns: warm with the output cache hitting, warm without it.  One work directory each, sealed; only ratios mean anything at this load."""
import os, shutil, statistics, sys, time
W = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
sys.path.insert(0, W + '/tools/prof')
import proflib
engine = proflib.Engine('x', sys.argv[1])
n = int(sys.argv[3])
counted = len(sys.argv) > 4 and sys.argv[4] == 'counted'
for doc in proflib.resolve_docs(sys.argv[2].split(',')):
    doc.prepare()
    root = os.path.join(proflib.out_root(), 'turns', doc.name.replace('/', '_'))
    shutil.rmtree(root, ignore_errors=True)
    sides = {}
    def setup(label, extra):
        work = os.path.join(root, label, 'o'); cache = os.path.join(root, label, 'cache')
        os.makedirs(work); os.makedirs(cache)
        env = proflib.corpus().entry_env(doc.entry)
        env.update(XDG_CACHE_HOME=cache, RSTEX_CHECKPOINT_POLICY='always')
        env.update(extra)
        return work, env
    def side(work):
        keep = doc.entry['jobname'] + '.pdf'
        for x in os.listdir(work):
            if x != keep:
                p = os.path.join(work, x)
                shutil.rmtree(p) if os.path.isdir(p) else os.remove(p)
        proflib.corpus().restore_state(doc.prepare(), work)
    a = setup('a', {'RSTEX_NO_OUTCACHE': '1'})
    b = setup('b', {})
    for w, e in (a, b):
        side(w)
        argv, cwd, _ = engine.command(doc, w)
        proflib.run_timed(argv, cwd, e)
    time.sleep(2.2)
    for w, e in (a, b):
        for _ in range(2):
            side(w)
            argv, cwd, _ = engine.command(doc, w)
            proflib.run_timed(argv, cwd, e)
    ra, rb = [], []
    for i in range(n):
        order = ((a, ra), (b, rb)) if i % 2 == 0 else ((b, rb), (a, ra))
        for (w, e), res in order:
            side(w)
            argv, cwd, _ = engine.command(doc, w)
            r = proflib.run_counted(argv, cwd, e) if counted else proflib.run_timed(argv, cwd, e)
            if counted:
                r['instr'] = r['counters']['instructions']['value'] / 1e6
                r['cyc'] = r['counters']['cycles']['value'] / 1e6
            res.append(r)
    def s(key, rs):
        v = [r[key] for r in rs]
        return statistics.median(v), min(v)
    print(f'{doc.name}: {n} turns, load {os.getloadavg()[0]:.0f}', flush=True)
    for key in ('cpu_ms', 'user_ms', 'sys_ms', 'wall_ms', 'minflt') + (('instr', 'cyc') if counted else ()):
        (ma, mina), (mb, minb) = s(key, ra), s(key, rb)
        print(f'  {key:<8} no cache median {ma:>9.2f} min {mina:>9.2f}   cache median {mb:>9.2f} min {minb:>9.2f}   ratio median {mb/ma:6.3f} min {minb/max(mina,1e-9):6.3f}', flush=True)
