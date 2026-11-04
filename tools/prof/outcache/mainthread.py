#!/usr/bin/env -S python3 -I
"""Instructions of the engine's thread alone and of all threads, warm, with deflate workers as they come: without the cache, and with it hitting."""
import os, shutil, sys, time
W = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
sys.path.insert(0, W + '/tools/prof')
import proflib
INHERIT = proflib.F_INHERIT
engine = proflib.Engine('x', sys.argv[1])
for doc in proflib.resolve_docs(sys.argv[2].split(',')):
    doc.prepare()
    root = os.path.join(proflib.out_root(), 'mt', doc.name.replace('/', '_'))
    shutil.rmtree(root, ignore_errors=True)
    work = os.path.join(root, 'o'); cache = os.path.join(root, 'cache'); os.makedirs(work); os.makedirs(cache)
    env = proflib.corpus().entry_env(doc.entry)
    env.update(XDG_CACHE_HOME=cache, RSTEX_CHECKPOINT_POLICY='always')
    def side():
        keep = doc.entry['jobname'] + '.pdf'
        for x in os.listdir(work):
            if x != keep:
                p = os.path.join(work, x)
                shutil.rmtree(p) if os.path.isdir(p) else os.remove(p)
        proflib.corpus().restore_state(doc.prepare(), work)
    def run(extra, inherit):
        proflib.F_INHERIT = INHERIT if inherit else 0
        side()
        argv, cwd, _ = engine.command(doc, work)
        e = dict(env); e.update(extra)
        r = proflib.run_counted(argv, cwd, e, events=[('instructions', 0, 1), ('cycles', 0, 0)])
        return r['counters']['instructions']['value'] / 1e6, r['counters']['cycles']['value'] / 1e6
    no = {'RSTEX_NO_OUTCACHE': '1'}
    run(no, True); time.sleep(2.2); run(no, True); run({}, True)
    a_all = run(no, True); a_main = run(no, False)
    run({}, True)
    b_all = run({}, True); b_main = run({}, False)
    print(f'{doc.name}: all threads {a_all[0]:.1f} to {b_all[0]:.1f} M ({b_all[0]/a_all[0]:.3f}), engine thread {a_main[0]:.1f} to {b_main[0]:.1f} M ({b_main[0]/a_main[0]:.3f}); cycles all {a_all[1]:.0f} to {b_all[1]:.0f}, engine {a_main[1]:.0f} to {b_main[1]:.0f}', flush=True)
