#!/usr/bin/env -S python3 -I
"""phases and scopes of a prof build, cold and warm, with deflate on the main thread."""
import json, os, shutil, subprocess, sys, time
W = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
sys.path.insert(0, W + '/tools/prof')
import proflib

def run(engine, doc, work, env, extra):
    os.makedirs(work, exist_ok=True)
    keep = doc.entry['jobname'] + '.pdf'
    for x in os.listdir(work):
        if x != keep:
            p = os.path.join(work, x)
            shutil.rmtree(p) if os.path.isdir(p) else os.remove(p)
    proflib.corpus().restore_state(doc.prepare(), work)
    argv, cwd, _ = engine.command(doc, work)
    e = dict(env); e.update(extra)
    return proflib.run_timed(argv, cwd, e)

def main():
    binary = sys.argv[1]
    names = sys.argv[2].split(',')
    engine = proflib.Engine('x', binary)
    for doc in proflib.resolve_docs(names):
        doc.prepare()
        root = os.path.join(proflib.out_root(), 'bd', doc.name.replace('/', '_'))
        shutil.rmtree(root, ignore_errors=True)
        work = os.path.join(root, 'o'); cache = os.path.join(root, 'cache'); os.makedirs(cache)
        env = proflib.corpus().entry_env(doc.entry)
        env['XDG_CACHE_HOME'] = cache
        env['RSTEX_CHECKPOINT_POLICY'] = 'always'
        out = {}
        txt = os.path.join(root, 'c.txt')
        for mode, base in (('warm', {}),):
            if mode == 'warm':
                run(engine, doc, work, env, {'RSTEX_DEFLATE_WORKERS': '0'})
                time.sleep(2.2)
                run(engine, doc, work, env, {'RSTEX_DEFLATE_WORKERS': '0'})
            extra = dict(base); extra['RSTEX_DEFLATE_WORKERS'] = '0'; extra['RSTEX_PROF_COUNTERS'] = txt
            r = run(engine, doc, work, env, extra)
            res = proflib.parse_counters(txt)
            out[mode] = res
            total = res['counters'].get('instructions', {}).get('value', 0)
            print(f'== {doc.name} {mode}: rc {r["rc"]} cpu {r["cpu_ms"]:.1f} ms, instr {total/1e6:.2f} M, pdf {os.path.getsize(os.path.join(work, doc.entry["jobname"] + ".pdf"))/1e6:.2f} MB')
            prev = None
            for ph in res['phases']:
                if prev is not None:
                    print(f'   phase {ph["name"]:<28} {(ph["instructions"]-prev)/1e6:>10.2f} M  (at {ph["instructions"]/1e6:>10.2f})')
                prev = ph['instructions']
            for k, v in sorted(res['scopes'].items(), key=lambda kv: -kv[1]['instructions']):
                print(f'   scope {k:<28} {v["instructions"]/1e6:>10.2f} M  count {v["count"]:>7}  {v["total_ns"]/1e6:>9.2f} ms  {100*v["instructions"]/max(total,1):5.1f}%')
        with open(os.path.join(root, 'res.json'), 'w') as f:
            json.dump(out, f)
main()
