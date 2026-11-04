#!/usr/bin/env -S python3 -I
"""Three passes from nothing, as latexmk would run them, with and without the output cache.  Instructions of all threads, and whether the last PDFs agree."""
import hashlib, os, shutil, subprocess, sys
W = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
sys.path.insert(0, W + '/tools/prof')
import proflib
engine = proflib.Engine('x', sys.argv[1])
for doc in proflib.resolve_docs(sys.argv[2].split(',')):
    doc.prepare()
    root = os.path.join(proflib.out_root(), 'passes', doc.name.replace('/', '_'))
    res = {}
    for label, extra in (('no cache', {'RSTEX_NO_OUTCACHE': '1'}), ('cache', {})):
        shutil.rmtree(root, ignore_errors=True)
        work = os.path.join(root, 'o'); cache = os.path.join(root, 'cache'); os.makedirs(work); os.makedirs(cache)
        env = proflib.corpus().entry_env(doc.entry)
        env.update(XDG_CACHE_HOME=cache)
        env.update(extra)
        counts = []
        for i in range(3):
            argv, cwd, _ = engine.command(doc, work)
            r = proflib.run_counted(argv, cwd, env)
            counts.append(r['counters']['instructions']['value'] / 1e6)
        p = os.path.join(work, doc.entry['jobname'] + '.pdf')
        res[label] = (counts, hashlib.sha256(open(p, 'rb').read()).hexdigest() if os.path.exists(p) else None)
    a, b = res['no cache'], res['cache']
    print(doc.name, 'no cache', [round(x, 1) for x in a[0]], 'cache', [round(x, 1) for x in b[0]], 'ratio', [round(y / x, 3) for x, y in zip(a[0], b[0])], 'sum', round(sum(b[0]) / sum(a[0]), 3), 'same PDF', a[1] == b[1], flush=True)
