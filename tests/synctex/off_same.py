#!/usr/bin/env -S python3 -I
r"""off_same.py: with SyncTeX off, a run gives what the base's gives, byte for byte.

    off_same.py --base BINARY --engine BINARY [--docs ladder/*,user/*] [-j N] [--literal]

Both are plain binaries, each with its own formats, from tools/prof/proflib.py.  Each document runs once on each in one directory that keeps its path, with the oracle's last-pass side files put back in
between: every file it leaves must be the same bytes, and so must the terminal and the exit status.  Without SOURCE_DATE_EPOCH nothing could be: proflib has it set, as the corpus does.  --literal runs with
the native diagram path off; the default is on, as a user has it.  Only ids and numbers are printed, and everything a run writes stays under target/.
"""
import argparse
import concurrent.futures
import os
import shutil
import subprocess
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.join(REPO, 'tools', 'prof'))
import proflib  # noqa: E402


def outputs(work):
    res = {}
    for root, _, names in os.walk(work):
        for name in names:
            p = os.path.join(root, name)
            with open(p, 'rb') as f:
                res[os.path.relpath(p, work)] = f.read()
    return res


def one_doc(doc, base, engine):
    doc.prepare()
    root = os.path.join(proflib.out_root(), 'synctex-off', doc.name.replace('/', '_'))
    work = os.path.join(root, 'o')
    res = []
    for e in (base, engine):
        shutil.rmtree(work, ignore_errors=True)
        os.makedirs(work)
        proflib.corpus().restore_state(doc.prepare(), work)
        argv, cwd, env = e.command(doc, work)
        p = subprocess.run(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=3600)
        # The banner names the format as given, and each binary has its own.
        fmt = next(a for a in argv if a.startswith('-fmt=')).split('=', 1)[1].encode()
        res.append((p.returncode, p.stdout.replace(fmt, b'FORMAT'), p.stderr, outputs(work)))
    shutil.rmtree(root, ignore_errors=True)
    a, b = res
    problems = []
    if a[0] != b[0]:
        problems.append(f'exit status {a[0]} and {b[0]}')
    if a[1] != b[1]:
        problems.append('terminal')
    if a[2] != b[2]:
        problems.append('stderr')
    for name in sorted(set(a[3]) | set(b[3])):
        if a[3].get(name) != b[3].get(name):
            problems.append('file ' + ('absent' if name not in a[3] or name not in b[3] else name.rsplit('.', 1)[-1]))
    return doc.id, problems, len(a[3])


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--base', required=True)
    ap.add_argument('--engine', required=True)
    ap.add_argument('--docs', default='ladder/*,user/*')
    ap.add_argument('--literal', action='store_true')
    ap.add_argument('-j', type=int, default=4)
    args = ap.parse_args()
    base = proflib.Engine('base', args.base, no_commute=args.literal)
    engine = proflib.Engine('new', args.engine, no_commute=args.literal)
    named = proflib.resolve_docs(args.docs.split(','))
    docs = [d for d in named if os.path.isdir(d.entry['cwd'])]
    if len(docs) != len(named):
        print(f'{len(named) - len(docs)} of {len(named)} documents have no source here, and are left out')
    for name in sorted({d.format for d in docs}):
        base.format_path(name)
        engine.format_path(name)
    with concurrent.futures.ThreadPoolExecutor(args.j) as ex:
        results = list(ex.map(lambda d: one_doc(d, base, engine), docs))
    bad = [r for r in results if r[1]]
    for doc_id, problems, _ in bad:
        print(f'FAIL {doc_id}: {sorted(set(problems))}')
    print(f'{len(results) - len(bad)}/{len(results)} documents the same in every byte, {sum(r[2] for r in results)} files in all')
    print('PASS' if not bad else 'FAIL')
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
