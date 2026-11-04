#!/usr/bin/env -S python3 -I
"""What the output cache adds to a run with no earlier one: same binary, in turns, with no PDF and no index before each."""
import os, shutil, statistics, sys
W = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
sys.path.insert(0, W + '/tools/prof')
import proflib
engine = proflib.Engine('x', sys.argv[1])
n = int(sys.argv[3])
for doc in proflib.resolve_docs(sys.argv[2].split(',')):
    doc.prepare()
    root = os.path.join(proflib.out_root(), 'firstrun', doc.name.replace('/', '_'))
    shutil.rmtree(root, ignore_errors=True)
    def setup(label, extra):
        work = os.path.join(root, label, 'o'); cache = os.path.join(root, label, 'cache')
        os.makedirs(work); os.makedirs(cache)
        env = proflib.corpus().entry_env(doc.entry)
        env.update(XDG_CACHE_HOME=cache, RSTEX_NO_SNAPSHOT_='')
        env.update(extra)
        return work, env, cache
    a = setup('a', {'RSTEX_NO_OUTCACHE': '1'})
    b = setup('b', {})
    def one(s, counted):
        work, env, cache = s
        shutil.rmtree(work); os.makedirs(work)
        for x in ('outcache', 'checkpoints'):
            shutil.rmtree(os.path.join(cache, 'rstex', x), ignore_errors=True)
        try:
            os.remove(os.path.join(cache, 'rstex', 'seen'))
        except OSError:
            pass
        proflib.corpus().restore_state(doc.prepare(), work)
        argv, cwd, _ = engine.command(doc, work)
        return proflib.run_counted(argv, cwd, env) if counted else proflib.run_timed(argv, cwd, env)
    one(a, False); one(b, False)
    ra, rb = [], []
    for i in range(n):
        for s, res in (((a, ra), (b, rb)) if i % 2 == 0 else ((b, rb), (a, ra))):
            res.append(one(s, False))
    ca = one(a, True)['counters']['instructions']['value'] / 1e6
    cb = one(b, True)['counters']['instructions']['value'] / 1e6
    print(f'{doc.name}, first run, {n} turns: instructions {ca:.2f} to {cb:.2f} M ({cb/ca:.4f})')
    for key in ('cpu_ms', 'user_ms', 'sys_ms', 'wall_ms'):
        ma, mb = statistics.median(r[key] for r in ra), statistics.median(r[key] for r in rb)
        la, lb = min(r[key] for r in ra), min(r[key] for r in rb)
        print(f'  {key:<8} median {ma:.2f} to {mb:.2f} ({mb/ma:.3f}), min {la:.2f} to {lb:.2f} ({lb/la:.3f})', flush=True)
