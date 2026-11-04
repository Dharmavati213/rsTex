"""The output cache on documents the scout did not pick.  In turns: warm with no cache, warm with hits.  Output compared."""
import os, random, shutil, sys, json, statistics
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import crit
proflib = crit.proflib
E = crit.engine('rstex-outcache')
ids = sys.argv[3:] 
seed, n = int(sys.argv[1]), int(sys.argv[2])
if not ids:
    es, _, _ = proflib.corpus().load_manifests(None)
    pool = [e['id'] for e in es if e['id'].startswith('arxiv/') and e['mode'] == 'latex']
    random.Random(seed).shuffle(pool)
    ids = pool[:n]
rows = []
for i in ids:
    try:
        doc = proflib.resolve_docs([i])[0]
        doc.prepare()
    except BaseException as x:
        print(i, 'no', x); continue
    if not doc.meta or doc.meta.get('observed_class') not in (None, 'ok', 'pass', 'stable', 'converged') and False:
        continue
    root = crit.fresh('oc')
    out = root + '/o'; os.makedirs(out); cache = root + '/c'; os.makedirs(cache)
    env = proflib.corpus().entry_env(doc.entry); env['XDG_CACHE_HOME'] = cache; env['RSTEX_CHECKPOINT_POLICY'] = 'always'
    proflib.corpus().restore_state(doc.meta, out)
    pre = crit.snapshot(out)
    argv, cwd, _ = E.command(doc, out)
    j = doc.entry['jobname']
    def put(keep_pdf=None):
        shutil.rmtree(out); os.makedirs(out)
        for k, v in pre.items():
            p = os.path.join(out, k); os.makedirs(os.path.dirname(p), exist_ok=True); open(p, 'wb').write(v)
        if keep_pdf is not None:
            open(os.path.join(out, j + '.pdf'), 'wb').write(keep_pdf)
    try:
        r = crit.counted(argv, cwd, env)      # writes checkpoint
        pdf = open(os.path.join(out, j + '.pdf'), 'rb').read() if os.path.exists(os.path.join(out, j + '.pdf')) else None
        if pdf is None or r['rc'] not in (0, 1):
            print(i, 'no pdf'); shutil.rmtree(root); continue
        put(pdf); crit.counted(argv, cwd, env)    # index
        pdf = open(os.path.join(out, j + '.pdf'), 'rb').read()
        A, B = [], []
        same = True
        ref = None
        for t in range(int(os.environ.get('ROUNDS','3'))):
            put(pdf); a = crit.counted(argv, cwd, dict(env, RSTEX_NO_OUTCACHE='1')); ra = crit.snapshot(out)
            put(pdf); b = crit.counted(argv, cwd, env); rb = crit.snapshot(out)
            if crit.differs(ra, rb): same = False; print('DIFF', i, crit.differs(ra, rb))
            A.append(a); B.append(b)
        def med(L, k): return statistics.median(x[k] for x in L)
        row = dict(id=i, pdf_kb=len(pdf)//1024, instr=med(A,'instr'), instr_r=med(B,'instr')/med(A,'instr'), cyc_r=med(B,'cyc')/med(A,'cyc'), cpu_r=med(B,'cpu')/med(A,'cpu'), sys_a=med(A,'sys'), sys_b=med(B,'sys'), wall_r=med(B,'wall')/med(A,'wall'), rss_a=med(A,'rss')/1024, rss_b=med(B,'rss')/1024, same=same)
        rows.append(row)
        print(f"{i:<38} pdf {row['pdf_kb']:>7} KB  warm {row['instr']:>8.0f} M  instr {row['instr_r']:.3f}  cycles {row['cyc_r']:.3f}  cpu {row['cpu_r']:.3f}  wall {row['wall_r']:.3f}  sys {row['sys_a']:.0f}->{row['sys_b']:.0f}  rss {row['rss_a']:.0f}->{row['rss_b']:.0f}  {'same' if same else 'DIFF'}", flush=True)
    except Exception as x:
        print(i, 'failed', x)
    shutil.rmtree(root, ignore_errors=True)
if rows:
    for k in ('instr_r', 'cyc_r', 'cpu_r', 'wall_r'):
        v = sorted(r[k] for r in rows)
        print(k, 'median', round(statistics.median(v), 3), 'p10', round(v[len(v)//10], 3), 'p90', round(v[len(v)*9//10], 3), 'min', round(v[0], 3), 'max', round(v[-1], 3))
    print(len(rows), 'docs; differ', sum(not r['same'] for r in rows))
