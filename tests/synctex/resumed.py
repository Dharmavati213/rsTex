#!/usr/bin/env -S python3 -I
r"""resumed.py: what a reader gets of SyncTeX's file from a run that came back from a page record.

    resumed.py --engine BINARY [--docs ladder/*,user/*] [-j N] [--queries N] [--flags "..."]

BINARY is a plain binary: tools/prof/proflib.py builds it its formats.  --docs takes what proflib takes.  A run has an editor's flags unless --flags says otherwise.

For each document, in one directory that keeps its path, with the oracle's last-pass side files put back before each run:

  cold     no snapshot: the file to hold the others against
  chain    the preamble's checkpoint, then a run from it that writes a chain with a record on every second page, and its file must be the same
  from k   a run from the record at the end of \document, one in the middle and the last, writing nothing.  Each file must be the cold run's byte for byte, so its text too, and its PDF

Then the queries of documents.py, made from the cold run's text, forward (`synctex view`) from the lines its records name and reverse (`synctex edit`) at the places of its records, by TeX Live's
synctex, on the cold run's file and on that of each run from a record, beside that run's PDF: every answer the same, in every byte, and most of them real answers, or this proves nothing.

Those files being equal, the answers cannot differ unless the tool looks at something else: this is the tool's word for what an editor is told, not a second comparison of the bytes.  Only ids and numbers are printed, and all a run writes
stays under target/.
"""
import argparse
import concurrent.futures
import gzip
import os
import shutil
import sys
import time

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.join(REPO, 'tools', 'prof'))
sys.path.insert(0, HERE)
import proflib  # noqa: E402
import documents  # noqa: E402

LEMMA = '-synctex=1 -interaction=nonstopmode -file-line-error -recorder'


def one_doc(doc, engine, args):
    rec = {'id': doc.id, 'problems': []}
    doc.prepare()
    root = os.path.join(proflib.out_root(), 'synctex-resumed', doc.name.replace('/', '_'))
    shutil.rmtree(root, ignore_errors=True)
    work, cache = os.path.join(root, 'o'), os.path.join(root, 'c')
    os.makedirs(cache)
    job = doc.entry['jobname']
    gz = job + '.synctex.gz'

    def reset():
        shutil.rmtree(work, ignore_errors=True)
        os.makedirs(work)
        proflib.corpus().restore_state(doc.prepare(), work)

    def place(data):
        with open(os.path.join(work, gz), 'wb') as f:
            f.write(data)

    reset()
    argv, cwd, _ = engine.command(doc, work)
    argv = [argv[0], *args.flags.split(), *argv[1:]]
    env = dict(proflib.corpus().entry_env(doc.entry), XDG_CACHE_HOME=cache, RSTEX_CHECKPOINT_POLICY='always', RSTEX_CHECKPOINT_TRACE='1')
    cold = documents.run(argv, cwd, dict(env, RSTEX_NO_SNAPSHOT='1'))
    cf = documents.outputs(work, job)
    if gz not in cf:
        rec['no_file'] = True
        return rec
    text = gzip.decompress(cf[gz])
    rec['text'] = len(text)
    rec['gz'] = len(cf[gz])
    rec['members'] = documents.members(cf[gz])
    reset()
    documents.run(argv, cwd, dict(env, RSTEX_NO_PAGES='1'))
    time.sleep(2.2)
    reset()
    documents.run(argv, cwd, dict(env, RSTEX_NO_PAGES='1'))
    reset()
    t = documents.run(argv, cwd, dict(env, RSTEX_PAGE_CK='2'))
    if documents.outputs(work, job).get(gz) != cf[gz]:
        rec['problems'].append('the run that wrote the chain has another file')
    said = [line for line in t.stderr.decode('latin-1').split('\n') if 'pages: ' in line and ' records, ' in line and 'using' not in line]
    n = int(said[0].split('pages: ')[1].split()[0]) if said else 0
    rec['records'] = n
    forward, reverse = documents.make_table(text, args.queries, len(text))
    # Plain TeX has no \\document to take a checkpoint at, and so no chain.
    asked = sorted({1, (n + 1) // 2, n}) if n else []
    rec['from'] = asked
    if forward or reverse:
        place(cf[gz])
        # The cold run's PDF and file: `synctex` looks at the PDF's name, and the table is the same for all.
        with open(os.path.join(work, job + '.pdf'), 'wb') as f:
            f.write(cf.get(job + '.pdf', b''))
        want = documents.ask_all(work, job, forward, reverse)
        rec['queries'] = len(want) * len(asked)
        rec['real'] = sum(1 for x in want if documents.is_real(x))
    rec['same'] = 0
    for k in asked:
        reset()
        p = documents.run(argv, cwd, dict(env, RSTEX_RESUME=str(k), RSTEX_PAGE_READONLY='1'))
        got = documents.outputs(work, job)
        if f'using {k}' not in p.stderr.decode('latin-1'):
            rec['problems'].append(f'it did not come back from record {k}')
        if p.returncode != cold.returncode:
            rec['problems'].append(f'from {k}: exit status {cold.returncode} and {p.returncode}')
        if gz not in got:
            rec['problems'].append(f'from {k}: no file')
            continue
        if gzip.decompress(got[gz]) != text:
            rec['problems'].append(f'from {k}: the text differs')
        if got[gz] != cf[gz]:
            rec['problems'].append(f'from {k}: the file differs')
        if got.get(job + '.pdf') != cf.get(job + '.pdf'):
            rec['problems'].append(f'from {k}: the PDF differs')
        if forward or reverse:
            answers = documents.ask_all(work, job, forward, reverse)
            rec['same'] += sum(1 for x, y in zip(want, answers) if x == y)
    if forward or reverse:
        if rec['same'] != rec['queries']:
            rec['problems'].append(f'{rec["queries"] - rec["same"]} of {rec["queries"]} answers differ')
        if rec['real'] * 2 < len(want):
            rec['problems'].append(f'only {rec["real"]} of {len(want)} queries got an answer, which proves little')
    shutil.rmtree(root, ignore_errors=True)
    return rec


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--engine', required=True)
    ap.add_argument('--docs', default='ladder/*,user/*')
    ap.add_argument('-j', type=int, default=4)
    ap.add_argument('--queries', type=int, default=32)
    ap.add_argument('--flags', default=LEMMA)
    args = ap.parse_args()
    engine = proflib.Engine('rstex', args.engine, no_commute=True)
    named = proflib.resolve_docs(args.docs.split(','))
    docs = [d for d in named if os.path.isdir(d.entry['cwd'])]
    if len(docs) != len(named):
        print(f'{len(named) - len(docs)} of {len(named)} documents have no source here, and are left out')
    for name in sorted({d.format for d in docs}):
        engine.format_path(name)
    with concurrent.futures.ThreadPoolExecutor(args.j) as ex:
        results = list(ex.map(lambda d: one_doc(d, engine, args), docs))
    bad = 0
    with_records = 0
    queries = 0
    for r in results:
        if r.get('no_file'):
            print(f'{r["id"]}: no file')
            continue
        with_records += bool(r.get('records'))
        queries += r.get('queries', 0)
        print(f'{r["id"]}: text {r["text"]} bytes, file {r["gz"]}, {r["members"]} members, {r["records"]} records, from {r["from"]}, {r.get("same", 0)} of {r.get("queries", 0)} answers the same, {r.get("real", 0)} real'
              + ('' if not r['problems'] else '; ' + '; '.join(r['problems'])))
        bad += bool(r['problems'])
    ok = not bad and with_records > 0 and queries > 0
    print(f'{"PASS" if ok else "FAIL"} from a record: {len(results)} documents, {with_records} with records, {bad} with a difference, {queries} answers')
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main())
