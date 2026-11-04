#!/usr/bin/env -S python3 -I
"""resume_edit.py: small edits to a document and what a resume does with them: crates/engine/src/checkpoint/pages.rs.

    resume_edit.py --engine PATH --docs main,... [--every N] [--at 0.1,0.5,0.9] [--types letter,word,para] [--json FILE]

A document's directory is never written to.  It is seen through a tree of symbolic links in a new directory under target/, and only the edited file there is a real file, for as long as the edit lasts.
The edit is generated: the first line of plain prose after the read position of the record it aims at, with one letter changed (`letter`), a word added (`word`), or a paragraph
added before it (`para`).  Then, from the same state of the output directory: a cold run of the edited document, which is the reference, then a run that resumes from the chain written before the edit,
byte-compared with it.  What is printed is numbers: no name, no line, no position in a file.
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import proflib  # noqa: E402
import resume as resume_tool  # noqa: E402

PROSE = re.compile(rb"^[A-Za-z][A-Za-z ,.;:'-]{30,}$")
STREAM = re.compile(rb'<<\n/Length (\d+) *\n/Filter /FlateDecode\n>>\nstream\n')
PARA = (b'\nthe quick brown fox jumps over the lazy dog and then it goes on for some time with no more to say about\n'
        b'what the dog thought of it all or where the fox went after that which is a question for another day and\n'
        b'another document entirely since this one only needs a paragraph of a certain length to push what follows\n'
        b'further down the page and perhaps over to the next one where it will have to find a new place to break\n\n')


def farm(src, dst):
    for root, dirs, names in os.walk(src):
        dirs[:] = [d for d in dirs if d != '.git']
        rel = os.path.relpath(root, src)
        d = os.path.normpath(os.path.join(dst, rel))
        os.makedirs(d, exist_ok=True)
        for n in names:
            os.symlink(os.path.join(root, n), os.path.join(d, n))


def page_streams(pdf):
    res = []
    at = 0
    while True:
        m = STREAM.search(pdf, at)
        if not m:
            return res
        n = int(m.group(1))
        res.append(pdf[m.end():m.end() + n])
        at = m.end() + n


def stream_diff(old, new):
    """(first stream that differs, streams left after it in the new run, how many at the end are the same as the old ones, in the same order)."""
    k = 0
    while k < len(old) and k < len(new) and old[k] == new[k]:
        k += 1
    tail = 0
    while tail < len(old) - k and tail < len(new) - k and old[-1 - tail] == new[-1 - tail]:
        tail += 1
    return k, len(new) - k, tail


class Doc2(proflib.Doc):
    pass


def edit_bytes(data, pos, kind):
    """The file with the edit, at the first prose line from `pos` on, or `None`; and where the edit is."""
    at = pos
    n = len(data)
    lines = 0
    while at < n and lines < 20000:
        e = data.find(b'\n', at)
        if e < 0:
            e = n
        line = data[at:e]
        if PROSE.match(line) and at > 0 and data[at - 1:at] == b'\n':
            if kind == 'letter':
                k = line.find(b'e', 5)
                if k >= 0:
                    return data[:at + k] + b'a' + data[at + k + 1:], at + k
            elif kind == 'word':
                k = line.find(b' ', 5)
                if k >= 0:
                    return data[:at + k] + b' indeed' + data[at + k:], at + k
            elif kind == 'para':
                return data[:at] + PARA + data[at:], at
        at = e + 1
        lines += 1
    return None


def one(engine, doc, args):
    doc.prepare()
    root = os.path.join(proflib.out_root(), 'resume-edit', doc.name.replace('/', '_'))
    shutil.rmtree(root, ignore_errors=True)
    src = os.path.join(root, 'src')
    farm(doc.entry['cwd'], src)
    e2 = dict(doc.entry, cwd=src)
    d2 = Doc2(doc.name, e2)
    d2.meta = doc.meta
    R = resume_tool.Runner(engine, d2, root)
    print(doc.name)
    rec = {'id': doc.id, 'every': args.every, 'cases': []}
    write_env = {'RSTEX_PAGE_CK': str(args.every)} if args.every else {}
    # The chain, on the document as it is: the first run only writes a preamble checkpoint.
    R.empty_cache()
    R.run({'RSTEX_NO_PAGES': '1'}, counted=False)
    w = R.run(write_env)
    print(f'  write: {w["instructions"] / 1e6:.1f} Minstr; chain {sum(R.sizes().values()) / 1e6:.1f} MB')
    rec['write'] = w
    rec['chain_bytes'] = sum(R.sizes().values())
    base_state = resume_tool.state(R.work)
    base_streams = page_streams(next((v for k, v in base_state.items() if k.endswith('.pdf')), b''))
    # Where each record had read to: the file on top of the input stack.  Kept here only.
    R.drop_chain()
    t = R.run(dict(write_env, RSTEX_CHECKPOINT_TRACE='1', RSTEX_PAGE_INPUTS='1'), counted=False)
    where = []
    for line in t['trace']:
        m = re.match(r'pages: rec (\d+) page (\d+) at (\d+) (.*)$', line)
        if m:
            where.append((int(m.group(1)), int(m.group(2)), int(m.group(3)), m.group(4)))
    n = len(where)
    print(f'  {n} records')
    print(f'  {"edit":<14}{"record":>7}{"used":>7}{"pages":>7}{"Minstr cold":>12}{"resume":>10}{"/cold":>7}{"same":>6}{"aux":>5}{"next":>6}{"differ":>8}{"tail":>6}')
    cold_cache = {}
    def read_file(path):
        with open(path, 'rb') as f:
            return f.read()
    for frac in [float(x) for x in args.at.split(',')]:
        for kind in args.types.split(','):
            k0 = max(0, min(n - 1, int(frac * n)))
            found = find_edit(where, src, k0, kind, read_file)
            if not found:
                print(f'  {kind + "@" + str(frac):<14} no prose line to edit')
                continue
            k, page, full, edited = found
            os.unlink(full)
            with open(full, 'wb') as f:
                f.write(edited)
            case = {'kind': kind, 'at': frac, 'record': k + 1, 'page': page}
            try:
                ref_run = R.run({'RSTEX_NO_SNAPSHOT': '1'})
                ref = resume_tool.state(R.work)
                case['cold'] = ref_run
                # The state before the run is put back by R.run; that is the oracle's last pass, which the writer's run also read.
                res = R.run({'RSTEX_PAGE_READONLY': '1'})
                got = resume_tool.state(R.work)
                case['resume'] = res
                case['differs'] = resume_tool.differs(ref, got)
                t2 = R.run({'RSTEX_PAGE_READONLY': '1', 'RSTEX_CHECKPOINT_TRACE': '1'}, counted=False)
                used = 0
                for line in t2['trace']:
                    m = re.match(r'pages: (\d+) records, using (\d+)', line)
                    if m:
                        used = int(m.group(2))
                case['used'] = used
                aux_changed = any(ref.get(x) != base_state.get(x) for x in ref if x.endswith(('.aux', '.toc', '.out', '.lof', '.lot', '.bbl', '.idx')))
                case['aux_changed'] = aux_changed
                # The next run after that: the output directory as this run left it, no reset.
                argv, cwd = R.argv()
                nx = subprocess.run(argv, cwd=cwd, env=dict(R.base_env, RSTEX_PAGE_READONLY='1', RSTEX_CHECKPOINT_TRACE='1'), stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
                nu = 0
                for line in nx.stderr.decode('latin-1').splitlines():
                    m = re.match(r'rstex: checkpoint: pages: (\d+) records, using (\d+)', line)
                    if m:
                        nu = int(m.group(2))
                case['next_used'] = nu
                new_streams = page_streams(next((v for k_, v in ref.items() if k_.endswith('.pdf')), b''))
                fd, left, tail = stream_diff(base_streams, new_streams)
                case['first_differ'], case['left'], case['tail_same'] = fd, left, tail
                case['pages_old'], case['pages_new'] = len(base_streams), len(new_streams)
                ci, ri = ref_run['instructions'], res['instructions']
                print(f'  {kind + "@" + str(frac):<14}{k + 1:>7}{used:>7}{page:>7}{ci / 1e6:>12.1f}{ri / 1e6:>10.1f}{ri / ci:>7.3f}{"yes" if not case["differs"] else "NO":>6}{"yes" if aux_changed else "no":>5}{nu:>6}{left - tail:>8}{tail:>6}')
                sys.stdout.flush()
                if case['differs']:
                    print('    differs:', case['differs'])
            finally:
                os.unlink(full)
                os.symlink(os.path.join(doc.entry['cwd'], os.path.relpath(full, src)), full)
            rec['cases'].append(case)
    if not args.keep:
        shutil.rmtree(root, ignore_errors=True)
    return rec


def find_edit(where, src, k0, kind, read):
    n = len(where)
    for k in list(range(k0, n)) + list(range(k0 - 1, -1, -1)):
        _, page, pos, path = where[k]
        full = os.path.join(src, path) if not os.path.isabs(path) else path
        if not os.path.normpath(full).startswith(src):
            continue
        data = read(full)
        got = edit_bytes(data, pos, kind)
        if got:
            return k, page, full, got[0]
    return None


def session(engine, doc, args):
    """A sitting at the editor: edits one after another, each kept, each run resuming from what the last left and writing on.  Every run is compared with a cold one of that source and that output directory."""
    doc.prepare()
    root = os.path.join(proflib.out_root(), 'resume-session', doc.name.replace('/', '_'))
    shutil.rmtree(root, ignore_errors=True)
    src = os.path.join(root, 'src')
    farm(doc.entry['cwd'], src)
    d2 = Doc2(doc.name, dict(doc.entry, cwd=src))
    d2.meta = doc.meta
    R = resume_tool.Runner(engine, d2, root)
    trace_file = os.path.join(root, 'trace')
    every = {'RSTEX_PAGE_CK': str(args.every)} if args.every else {}

    def go(extra):
        argv, cwd = R.argv()
        r = proflib.run_counted(argv, cwd, dict(R.base_env, **extra), stderr_path=trace_file)
        with open(trace_file, 'rb') as f:
            said = [line[len('rstex: checkpoint: '):] for line in f.read().decode('latin-1').splitlines() if line.startswith('rstex: checkpoint: ')]
        return r, said

    def put_back(s):
        shutil.rmtree(R.work, ignore_errors=True)
        os.makedirs(R.work)
        for name, data in s.items():
            p = os.path.join(R.work, name)
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, 'wb') as f:
                f.write(data)

    reset = resume_tool.reset
    reset(R.work, d2)
    R.empty_cache()
    print(doc.name)
    # A cold run for the tables of kpathsea in the cache, and one that writes the preamble's checkpoint.
    go({'RSTEX_NO_SNAPSHOT': '1'})
    reset(R.work, d2)
    go({'RSTEX_NO_PAGES': '1'})
    reset(R.work, d2)
    r, said = go(dict(every, RSTEX_CHECKPOINT_TRACE='1', RSTEX_PAGE_INPUTS='1'))
    where = []
    for line in said:
        m = re.match(r'pages: rec (\d+) page (\d+) at (\d+) (.*)$', line)
        if m:
            where.append((int(m.group(1)), int(m.group(2)), int(m.group(3)), m.group(4)))
    n = len(where)
    print(f'  first run, writes all: {r["counters"]["instructions"]["value"] / 1e6:.1f} Minstr; {n} records, chain {sum(R.sizes().values()) / 1e6:.1f} MB')
    print(f'  {"step":<14}{"used":>6}{"of":>6}{"Minstr cold":>12}{"resume, writing":>16}{"/cold":>7}{"same":>6}{"aux":>5}{"chain MB":>10}')
    steps = args.steps.split(',')
    current = {}

    def read(path):
        if path in current:
            return current[path]
        with open(path, 'rb') as f:
            return f.read()
    bad = 0
    for step in steps:
        kind, frac = step.split('@')
        got = find_edit(where, src, max(0, min(n - 1, int(float(frac) * n))), kind, read)
        if not got:
            print(f'  {step:<14} no prose line')
            continue
        k, page, full, edited = got
        current[full] = edited
        if os.path.islink(full):
            os.unlink(full)
        with open(full, 'wb') as f:
            f.write(edited)
        before = resume_tool.state(R.work)
        c, _ = go({'RSTEX_NO_SNAPSHOT': '1'})
        ref = resume_tool.state(R.work)
        put_back(before)
        r, said = go(dict(every, RSTEX_CHECKPOINT_TRACE='1'))
        got_state = resume_tool.state(R.work)
        d = resume_tool.differs(ref, got_state)
        bad += bool(d)
        used, total = 0, 0
        for line in said:
            m = re.match(r'pages: (\d+) records, using (\d+)', line)
            if m:
                total, used = int(m.group(1)), int(m.group(2))
        aux = any(ref.get(x) != before.get(x) for x in ref if x.endswith(('.aux', '.toc', '.out', '.lof', '.lot')))
        ci, ri = c['counters']['instructions']['value'], r['counters']['instructions']['value']
        print(f'  {step:<14}{used:>6}{total:>6}{ci / 1e6:>12.1f}{ri / 1e6:>16.1f}{ri / ci:>7.3f}{"yes" if not d else "NO":>6}{"yes" if aux else "no":>5}{sum(R.sizes().values()) / 1e6:>10.1f}')
        sys.stdout.flush()
        if d:
            print('    differs:', d)
    print(f'  {len(steps)} steps, {bad} differ')
    if not args.keep:
        shutil.rmtree(root, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--engine', required=True)
    ap.add_argument('--docs', required=True)
    ap.add_argument('--every', type=int, default=0, help='a record every so many pages; by default what the program does by itself')
    ap.add_argument('--at', default='0.1,0.5,0.9')
    ap.add_argument('--types', default='letter,word,para')
    ap.add_argument('--keep', action='store_true')
    ap.add_argument('--json')
    ap.add_argument('--session', action='store_true')
    ap.add_argument('--steps', default='word@0.9,word@0.9,letter@0.92,para@0.95,word@0.95,word@0.95,word@0.5,letter@0.5,para@0.2,word@0.2,word@0.9')
    args = ap.parse_args()
    engine = proflib.Engine('new', args.engine)
    recs = []
    for doc in proflib.resolve_docs(args.docs.split(',')):
        if args.session:
            session(engine, doc, args)
        else:
            recs.append(one(engine, doc, args))
    if args.json:
        with open(args.json, 'w') as f:
            json.dump(recs, f, indent=1)


if __name__ == '__main__':
    main()
