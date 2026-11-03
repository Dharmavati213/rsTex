#!/usr/bin/env -S python3 -I
"""resume_phases.py: where a resume's instructions go, by the phases and scopes of a prof build.  A prototype's tool.

    resume_phases.py --engine PROF_BUILD --docs main,... [--every N] [--k latest] [--write]
"""
import argparse
import os
import shutil
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import proflib  # noqa: E402
import resume as resume_tool  # noqa: E402


def show(path, what):
    res = proflib.parse_counters(path)
    print(f'  {what}')
    last = 0
    for p in res['phases']:
        d = p['instructions'] - last
        last = p['instructions']
        if d > 200000:
            print(f'    {p["name"]:<34}{d / 1e6:>10.2f} Minstr {p["minflt"]:>7} faults')
    for k, s in sorted(res['scopes'].items(), key=lambda kv: -kv[1]['instructions'])[:12]:
        print(f'    scope {k:<28}{s["instructions"] / 1e6:>10.2f} Minstr {s["count"]:>7} times {s["total_ns"] / 1e6:>9.1f} ms')
    c = res['counters'].get('instructions')
    if c:
        print(f'    total {c["value"] / 1e6:.2f} Minstr')


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--engine', required=True)
    ap.add_argument('--docs', required=True)
    ap.add_argument('--every', default='1')
    ap.add_argument('--k', default='latest')
    args = ap.parse_args()
    engine = proflib.Engine('prof', args.engine)
    for doc in proflib.resolve_docs(args.docs.split(',')):
        doc.prepare()
        root = os.path.join(proflib.out_root(), 'resume-phases', doc.name.replace('/', '_'))
        shutil.rmtree(root, ignore_errors=True)
        R = resume_tool.Runner(engine, doc, root)
        print(doc.name)
        R.run({'RSTEX_NO_SNAPSHOT': '1'}, counted=False)
        text = os.path.join(root, 'counters.txt')
        R.empty_cache()
        R.run({'RSTEX_PAGE_CK': args.every, 'RSTEX_PROF_COUNTERS': text}, counted=False)
        show(text, f'writer, every {args.every}')
        for k in args.k.split(','):
            R.run({'RSTEX_RESUME': k, 'RSTEX_PROF_COUNTERS': text}, counted=False)
            show(text, f'resume {k}')
        R.run({'RSTEX_RESUME': '0', 'RSTEX_PROF_COUNTERS': text}, counted=False)
        show(text, 'warm')
        shutil.rmtree(root, ignore_errors=True)


if __name__ == '__main__':
    main()
