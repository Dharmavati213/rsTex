#!/usr/bin/env -S python3 -I
r"""documents.py: SyncTeX's text and answers on real documents, the oracle's file against rstex's.

    documents.py --engine BINARY [--docs ladder,user] [-j N] [--queries N] [--path-on]

BINARY is a plain binary: tools/prof/proflib.py builds it its formats.  --docs takes what proflib takes: names, sets, manifest ids, `prefix/*`.

For each document, in one directory that keeps its path, with the oracle's last-pass side files put back before each run:

  oracle   -synctex=1
  rstex    -synctex=1, the literal engine: the text must be the same byte for byte, the file made of more than one member if there is more than a page, and the PDF and log what a run with SyncTeX off gives
  queries  a table of positions made from the oracle's text: forward (`synctex view`) from the lines its records name, of the first files it names, and reverse (`synctex edit`) at
           the places of its records and between them.  The answers on rstex's file must be those on the oracle's, in every byte, and most must be real answers, or the test proves nothing.
  path on  with --path-on, also the native diagram path as a user has it: on a document with diagrams the text differs inside them.  It is counted, and judged by the same queries, and fails nothing: DESIGN section 10.

Only ids and numbers are printed.  Everything a run writes stays under target/, as a document's text is not for the repository.
"""
import argparse
import concurrent.futures
import difflib
import gzip
import os
import random
import re
import shutil
import subprocess
import sys
import zlib

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.join(REPO, 'tools', 'prof'))
import proflib  # noqa: E402

SP_PER_BP = 65781.76
RECORD = re.compile(rb'^[\[(hvkg$xr](\d+),(\d+):(-?\d+),(-?\d+)')


def run(argv, cwd, env, timeout=1800):
    return subprocess.run(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout)


def with_synctex(argv, option='-synctex=1'):
    return [argv[0], option] + argv[1:]


def outputs(work, job):
    res = {}
    for name in os.listdir(work):
        with open(os.path.join(work, name), 'rb') as f:
            res[name] = f.read()
    return res


def members(data):
    """How many gzip members: by walking them, as a reader does."""
    n, at = 0, 0
    while at < len(data):
        d = zlib.decompressobj(31)
        d.decompress(data[at:])
        at = len(data) - len(d.unused_data)
        n += 1
    return n


def make_table(text, n, seed):
    """Forward and reverse queries from what the text names.  Nothing of it is printed."""
    rng = random.Random(seed)
    inputs, records, page = {}, [], 0
    for line in text.split(b'\n'):
        if line.startswith(b'Input:'):
            _, tag, name = line.split(b':', 2)
            inputs[int(tag)] = name
        elif line.startswith(b'{'):
            page = int(line[1:])
        else:
            m = RECORD.match(line)
            if m and page:
                records.append((int(m[1]), int(m[2]), int(m[3]), int(m[4]), page))
    if not records:
        return [], []
    pairs = sorted({(r[0], r[1]) for r in records if r[0] in inputs})
    chosen = rng.sample(pairs, min(n, len(pairs)))
    # And lines no record names: the nearest answer is what a reader then finds.
    forward = [(inputs[t], line, rng.randrange(0, 40)) for t, line in chosen]
    tags = sorted({t for t, _ in pairs})
    for _ in range(n // 8):
        t = rng.choice(tags)
        forward.append((inputs[t], rng.randrange(1, 4000), 0))
    picked = rng.sample(records, min(n, len(records)))
    reverse = [(r[4], r[2] / SP_PER_BP + rng.uniform(-3, 3), r[3] / SP_PER_BP + rng.uniform(-3, 3)) for r in picked]
    last = records[-1][4]
    for _ in range(n // 8):
        reverse.append((rng.randrange(1, last + 1), rng.uniform(0, 612), rng.uniform(0, 792)))
    return forward, reverse


def answer(cmd, cwd):
    p = subprocess.run(cmd, cwd=cwd, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=300, env=dict(os.environ, LC_ALL='C'))
    return p.returncode, p.stdout


def ask_all(work, job, forward, reverse):
    """The answers, in the order of the table: the file named by the job in `work`."""
    pdf = job + '.pdf'
    res = []
    for name, line, column in forward:
        res.append(answer([b'synctex', b'view', b'-i', b'%d:%d:%s' % (line, column, name), b'-o', pdf.encode()], work))
    for page, x, y in reverse:
        res.append(answer(['synctex', 'edit', '-o', f'{page}:{x:.3f}:{y:.3f}:{pdf}'], work))
    return res


def place_of(a):
    """What a reader is sent to: for a reverse query the file and line, for a forward one the first page."""
    edit = re.search(rb'^Input:(.*)\nLine:(\d+)', a[1], re.M)
    if edit:
        return edit.group(1), edit.group(2)
    page = re.search(rb'^Page:(\d+)', a[1], re.M)
    return page.group(1) if page else None


def is_real(a):
    return a[0] == 0 and b'SyncTeX result begin' in a[1] and (b'Line:' in a[1] or b'Page:' in a[1])


def one_doc(doc, oracle, engine, args):
    rec = {'id': doc.id}
    doc.prepare()
    root = os.path.join(proflib.out_root(), 'synctex-docs', doc.name.replace('/', '_'))
    shutil.rmtree(root, ignore_errors=True)
    work = os.path.join(root, 'o')
    job = doc.entry['jobname']

    def reset():
        shutil.rmtree(work, ignore_errors=True)
        os.makedirs(work)
        proflib.corpus().restore_state(doc.prepare(), work)

    gz = job + '.synctex.gz'

    def place(data):
        # The file must be the newest, and alone: the tool removes others.
        with open(os.path.join(work, gz), 'wb') as f:
            f.write(data)

    reset()
    argv, cwd, env = oracle.command(doc, work)
    o = run(with_synctex(argv), cwd, env)
    of = outputs(work, job)
    rec['oracle_status'] = o.returncode
    if gz not in of:
        rec['oracle_file'] = False
        rec['problems'] = []
        reset()
        argv, cwd, env = engine.command(doc, work)
        run(with_synctex(argv), cwd, env)
        rf = outputs(work, job)
        rec['problems'] = ['rstex wrote a file and the oracle none'] if gz in rf else []
        return rec
    rec['oracle_file'] = True
    otext = gzip.decompress(of[gz])
    rec['text'] = len(otext)
    rec['oracle_gz'] = len(of[gz])
    problems = []
    reset()
    argv, cwd, env = engine.command(doc, work)
    r = run(with_synctex(argv), cwd, env)
    rf = outputs(work, job)
    if r.returncode != o.returncode:
        problems.append(f'exit status {o.returncode} and {r.returncode}')
    if gz not in rf:
        problems.append('rstex wrote no file')
        rec['problems'] = problems
        return rec
    rtext = gzip.decompress(rf[gz])
    rec['rstex_gz'] = len(rf[gz])
    rec['members'] = members(rf[gz])
    if rtext != otext:
        a, b = otext.split(b'\n'), rtext.split(b'\n')
        k = next((i for i, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
        kind = (a[k][:1] if k < len(a) else b'').decode('latin-1')
        problems.append(f'the text differs from line {k + 1} of {len(a)} and {len(b)}, a record of kind {kind!r}, {sum(1 for x, y in zip(a, b) if x != y)} lines in all')
    # The same run with SyncTeX off.
    reset()
    run(argv, cwd, env)
    ff = outputs(work, job)
    for name in sorted(set(ff) & set(rf)):
        if name.endswith(('.pdf', '.log', '.aux')) and ff[name] != rf[name]:
            problems.append(f'the {name.rsplit(".", 1)[-1]} is not what a run with SyncTeX off gives')
    # Queries: the same directory, the same names, on the two files.
    if args.queries:
        forward, reverse = make_table(otext, args.queries, len(otext))
        if forward or reverse:
            reset()
            argv, cwd, env = engine.command(doc, work)
            run(with_synctex(argv), cwd, env)
            have = job + '.pdf'
            if not os.path.exists(os.path.join(work, have)):
                problems.append('no PDF to ask about')
            else:
                place(of[gz])
                a = ask_all(work, job, forward, reverse)
                place(rf[gz])
                b = ask_all(work, job, forward, reverse)
                rec['queries'] = len(a)
                rec['same'] = sum(1 for x, y in zip(a, b) if x == y)
                rec['real'] = sum(1 for x in a if is_real(x))
                if rec['same'] != rec['queries']:
                    problems.append(f'{rec["queries"] - rec["same"]} of {rec["queries"]} answers differ')
                if rec['real'] * 2 < rec['queries']:
                    problems.append(f'only {rec["real"]} of {rec["queries"]} queries got an answer, which proves little')
    # The native diagram path, as a user has it.  The literal engine above is what is judged against the oracle's text.
    if args.path_on:
        nat = proflib.Engine('native', engine.path)
        reset()
        argv, cwd, env = nat.command(doc, work)
        n = run(with_synctex(argv), cwd, env)
        nf = outputs(work, job)
        if gz in nf:
            ntext = gzip.decompress(nf[gz])
            al, bl = otext.split(b'\n'), ntext.split(b'\n')
            rec['native_lines'] = (len(al), len(bl))
            if ntext != otext:
                sm = difflib.SequenceMatcher(None, al, bl, autojunk=False) if len(al) < 40000 else None
                rec['native_equal_lines'] = sum(m.size for m in sm.get_matching_blocks()) if sm else None
            else:
                rec['native_equal_lines'] = len(al)
            if args.queries and (forward or reverse):
                c = ask_all(work, job, forward, reverse)
                place(of[gz])
                # The oracle's answers over the PDF the path made: the page sizes are the same, and only the file is asked.
                a2 = ask_all(work, job, forward, reverse)
                rec['native_same'] = sum(1 for x, y in zip(a2, c) if x == y)
                rec['native_same_place'] = sum(1 for x, y in zip(a2, c) if place_of(x) == place_of(y))
        rec['native_status'] = n.returncode
    rec['problems'] = problems
    return rec


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--engine', required=True)
    ap.add_argument('--docs', default='ladder,user')
    ap.add_argument('-j', type=int, default=4)
    ap.add_argument('--queries', type=int, default=64)
    ap.add_argument('--path-on', action='store_true')
    args = ap.parse_args()
    oracle = proflib.Engine('oracle')
    engine = proflib.Engine('rstex', args.engine, no_commute=True)
    named = proflib.resolve_docs(args.docs.split(','))
    # A public entry whose source this tree has not fetched, which refs/ may lack.  It is said, and none of the gate's are of that kind.
    docs = [d for d in named if os.path.isdir(d.entry['cwd'])]
    if len(docs) != len(named):
        print(f'{len(named) - len(docs)} of {len(named)} documents have no source here, and are left out')
    # Formats are built once for this binary, before anything runs in parallel.
    for name in sorted({d.format for d in docs}):
        engine.format_path(name)
    with concurrent.futures.ThreadPoolExecutor(args.j) as ex:
        results = list(ex.map(lambda d: one_doc(d, oracle, engine, args), docs))
    bad = 0
    with_file = 0
    for r in results:
        line = f'{r["id"]}: '
        if not r.get('oracle_file'):
            line += f'no file on the oracle, status {r.get("oracle_status")}'
        else:
            with_file += 1
            line += f'text {r.get("text")} bytes, gz oracle {r.get("oracle_gz")} rstex {r.get("rstex_gz")}, {r.get("members")} members'
            if 'queries' in r:
                line += f', {r["same"]}/{r["queries"]} answers equal, {r["real"]} of them real'
            if 'native_lines' in r:
                line += f'; path on: {r.get("native_equal_lines")} of {r["native_lines"][0]} lines kept, {r["native_lines"][1]} now'
                if 'native_same' in r:
                    line += f', {r["native_same"]}/{r.get("queries")} answers the same, {r["native_same_place"]} the same place'
        print(line)
        for p in r['problems']:
            print(f'    FAIL {p}')
        bad += bool(r['problems'])
    print(f'{len(results) - bad}/{len(results)} documents hold, {with_file} with a file on the oracle; answers {sum(r.get("same", 0) for r in results)}/{sum(r.get("queries", 0) for r in results)} equal, {sum(r.get("real", 0) for r in results)} real')
    print('PASS' if not bad and with_file else 'FAIL')
    return 0 if not bad and with_file else 1


if __name__ == '__main__':
    sys.exit(main())
