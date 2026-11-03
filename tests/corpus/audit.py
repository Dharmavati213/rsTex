#!/usr/bin/env -S python3 -I
r"""Which entries of the corpus really test something?  A denominator can be inflated by entries that exercise almost nothing.

    python3 -I tests/corpus/audit.py [--manifest FILE]... [--out FILE] [--list CATEGORY]

Reads the cached oracle of every entry (run `run.py prepare` first), compiles nothing, and gives each entry one verdict:

  substantive   the rest
  vacuous       stops or produces nothing before doing work, by the rules below
  duplicate     the same test as another entry: same output once the entry's own name is masked

Rules, each with its reason:

  fatal-at-load   The oracle ends the job without a PDF, on a missing file or a bad option, having read nothing beyond the format and the class
                  or package that asked for it.  Every such entry takes one error path, and the rest of the log is the same.
  no-judged-text  A regression test whose judged log form has at most 2 lines, nothing printed between START-TEST-LOG and its end, and which ships no page either.
                  A page is an output like the log: it is judged at L2, L4 and L7, so such a test is not empty.
  empty-run       No PDF and a normalized log of at most 3 lines: the run did nothing to compare.
  blank-output    A PDF with no text, images or graphics on any page, whose log has no error, read from the content stream.
                  Only counted where the log has at most 20 lines: otherwise the log is a real output of the test.
  same-closure    Every file read, by content, equals an earlier entry's, and so do the PDF and the log: the same document under another name.
  same-output     PDF and log equal an earlier entry's once the entry's own name is masked in the log: tests identical up to names.

What is not called vacuous: a document that fails after a lot of work.  Its error path is behaviour to reproduce.  Nor coverage/prim-fatal, whose subject is ending the job.  And `prim-bare` entries that
print the same message with a different primitive's name count as `same-output` here, but keep one count per family, listed separately,
because for a literal port each name is its own branch in one big switch: that family is reported, not hidden.

The result goes to target/corpus/audit/substance.json, which `run.py compare` reads for its "substantive" line.  Results are keyed by the
oracle key of the entry, so an entry whose oracle changed counts as not audited until this runs again.
"""
import argparse
import collections
import hashlib
import importlib.util
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location('corpus_run', os.path.join(HERE, 'run.py'))
run = importlib.util.module_from_spec(spec)
spec.loader.exec_module(run)

ID_RE = re.compile(rb'/ID \[<[0-9A-Fa-f]*> ?<[0-9A-Fa-f]*>\]')
WS = re.compile(rb'\s+')
BASE_FILES = {'pdflatex.fmt', 'pdftex.fmt', 'texmf.cnf', 'pdftex.map', 'mktex.cnf', 'pdftexconfig.tex'}


def latest_meta(e):
    """The current oracle of this checkout: the valid one is expensive to check on 13 000 entries, and the audit only reads.  Another checkout's oracle of the same
    entry sits in the same directory under another configuration hash, and it counts for nothing here."""
    base = run.oracle_base(e)
    best = None
    prefix = run.config_hash(e) + '-'
    try:
        names = os.listdir(base)
    except OSError:
        return None
    for n in names:
        p = os.path.join(base, n, 'meta.json')
        if not n.startswith(prefix) or not os.path.isfile(p):
            continue
        if best is None or os.path.getmtime(p) > os.path.getmtime(best):
            best = p
    if not best:
        return None
    meta = run.read_json(best)
    meta['dir'] = os.path.dirname(best)
    stats = run.log_stats(os.path.join(meta['dir'], 'final', meta['jobname'] + '.log'))     # a cached count may be older than the rules in log_stats
    if meta.get('log') and not stats.get('missing'):
        meta['log'].update({k: stats[k] for k in ('errors', 'first_error', 'pages')})
    return meta


def rejoin_wraps(data, width=79):
    """A longer name moves TeX's line wraps, so wrapped lines are joined before comparing."""
    out, cur = [], b''
    for line in data.split(b'\n'):
        cur += line
        if len(line) != width:
            out.append(cur)
            cur = b''
    out.append(cur)
    return out


def masked_log_key(e, meta, data):
    """The log, as one hash, with what names this entry taken out: job name, the last part of its id, its paths and its work directory."""
    names = {e['jobname'], e['id'].rsplit('/', 1)[-1]}
    if e.get('main'):
        names.add(os.path.basename(e['main']))
        names.add(os.path.splitext(os.path.basename(e['main']))[0])
    wd = meta.get('workdir_path')
    if wd:
        data = run.mask_path(data, wd)
    lines = rejoin_wraps(data)
    joined = []
    for raw in lines:
        joined.append(raw)
    text = b'\n'.join(joined)
    if e.get('cwd'):
        text = text.replace(os.fsencode(e['cwd']), b'<cwd>')
    for n in sorted(names, key=len, reverse=True):
        if n and len(n) > 1:
            text = re.sub(rb'(?<![A-Za-z0-9])' + re.escape(os.fsencode(n)) + rb'(?![A-Za-z0-9])', b'@@', text)
    norm = [x[1] for x in run.normalize_log(text, [])]
    return hashlib.sha1(b'\n'.join(norm)).hexdigest(), len(norm)


def blank_pdf(path):
    """No text, images, or painting operators on any page."""
    p = subprocess.run(['qpdf', '--json=2', '--json-key=qpdf', '--json-stream-data=inline', '--decode-level=generalized', path, '-'], capture_output=True)
    if not p.stdout:
        return False
    try:
        objs = json.loads(p.stdout)['qpdf'][1]
    except ValueError:
        return False
    import base64
    for k, v in objs.items():
        if isinstance(v, dict) and 'stream' in v:
            d = v['stream']['dict']
            if d.get('/Type') in ('/ObjStm', '/XRef', '/Metadata') or '/Length1' in d or '/FontFile' in d or d.get('/Subtype') in ('/Type1C', '/XML'):
                continue
            if v['stream'].get('data'):
                body = base64.b64decode(v['stream']['data'])
                if b'begincmap' in body:
                    continue
                if re.search(rb'\bTJ\b|\bTj\b|\bDo\b|\bre\b|\bl\b|\bc\b', body):
                    return False
    return True


def audit_entry(e):
    meta = latest_meta(e)
    if meta is None:
        return {'id': e['id'], 'verdict': 'unprepared', 'source': e['source']}
    cls = run.effective_class(meta, e)
    rec = {'id': e['id'], 'source': e['source'], 'tier': e['tier'], 'class': cls, 'oracle_key': meta['key'], 'pages': (meta.get('log') or {}).get('pages') or 0,
           'errors': (meta.get('log') or {}).get('errors') or 0, 'judged_failure': bool(meta.get('judged_failure')), 'pdf_bytes': (meta.get('pdf') or {}).get('bytes', 0),
           'compare': e['compare'], 'first_error': (meta.get('log') or {}).get('first_error')}
    if cls != 'normal':
        rec['verdict'] = 'not-comparable'
        return rec
    job = meta['jobname']
    final = os.path.join(meta['dir'], 'final')
    log = run.open_bytes(os.path.join(final, job + '.log'))
    key, nlines = masked_log_key(e, meta, log)
    rec['log_key'] = key
    rec['log_lines'] = nlines
    if e['log_normalize'] == 'l3build':
        judged = run.judged_log(e, log, [meta.get('workdir_path')])
        rec['judged_lines'] = len(judged)
        rec['judged_key'] = hashlib.sha1(b'\n'.join(x[1] for x in judged)).hexdigest()
    pdf = os.path.join(final, job + '.pdf')
    if os.path.isfile(pdf) and os.path.getsize(pdf):
        data = run.open_bytes(pdf)
        rec['pdf_key'] = hashlib.sha1(ID_RE.sub(b'', data)).hexdigest()
    inputs = [p for p in meta['inputs'] if not any(os.path.basename(p) == b for b in BASE_FILES)]
    # Files in the entry's closure beyond its own main file and the format's own configuration, by content.
    closure = sorted(meta['inputs'][p] for p in meta['inputs'] if p != e.get('main') and os.path.basename(p) not in BASE_FILES)
    rec['closure_key'] = hashlib.sha1('\n'.join(closure).encode()).hexdigest()
    rec['main_key'] = meta['inputs'].get(e.get('main')) if e.get('main') else e.get('first_line')
    rec['files_read'] = len(inputs)
    rec['tds_only'] = sum(p.startswith(run.TDS_PREFIXES) for p in inputs)
    return rec


def classify(records, entries):
    by_id = {e['id']: e for e in entries}
    seen_output, seen_closure = {}, {}
    for r in sorted(records, key=lambda x: x['id']):
        if r.get('verdict'):
            continue
        e = by_id[r['id']]
        why = None
        verdict = 'substantive'
        if r['judged_failure'] and '/prim-fatal/' not in r['id'] and r['files_read'] <= 12 and r['pages'] == 0 and re.search(r"not found|can't find|Fatal error|Emergency|cannot be loaded|Option clash", (r.get('first_error') or ''), re.I):
            verdict, why = 'vacuous', 'fatal-at-load'
        elif r.get('judged_lines') is not None and r['judged_lines'] <= 2 and not r['judged_failure'] and not r.get('pdf_key'):
            verdict, why = 'vacuous', 'no-judged-text'
        elif not r.get('pdf_key') and r['log_lines'] <= 3:
            verdict, why = 'vacuous', 'empty-run'
        r['verdict'], r['why'] = verdict, why
        if verdict != 'substantive':
            continue
        # A regression test's own subject is its judged text; a package-load test's subject is what it read.
        subject = r.get('judged_key') or r['log_key']
        ckey = (r['closure_key'], r.get('main_key'), r.get('pdf_key'), subject)
        okey = (r.get('pdf_key'), subject, r['closure_key'])
        if ckey in seen_closure:
            r['verdict'], r['why'], r['same_as'] = 'duplicate', 'same-closure', seen_closure[ckey]
        elif okey in seen_output:
            r['verdict'], r['why'], r['same_as'] = 'duplicate', 'same-output', seen_output[okey]
        else:
            seen_closure[ckey] = r['id']
            seen_output[okey] = r['id']
        _ = e


def category_of(entry_id):
    parts = entry_id.split('/')
    if parts[0] == 'coverage' and len(parts) > 1:
        return parts[1].split('=')[0]
    return parts[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--manifest', action='append')
    ap.add_argument('--out', default=os.path.join(run.TARGET, 'audit', 'substance.json'))
    ap.add_argument('--list', help='print the entries with this verdict or reason')
    ap.add_argument('--blank-check', action='store_true', help='also open PDFs under 8 KB with qpdf and look for pages with nothing on them')
    args = ap.parse_args()
    entries, _, files = run.load_manifests(args.manifest)
    records = []
    for k, e in enumerate(entries):
        if e.get('excluded'):
            records.append({'id': e['id'], 'source': e['source'], 'verdict': 'excluded', 'why': e['excluded']})
            continue
        records.append(audit_entry(e))
        if (k + 1) % 2000 == 0:
            print(f'[{k + 1}/{len(entries)}]', file=sys.stderr, flush=True)
    classify(records, entries)
    if args.blank_check:
        by_id = {e['id']: e for e in entries}
        for r in records:
            if r.get('verdict') == 'substantive' and r.get('pdf_key') and r['pdf_bytes'] < 8000 and r['errors'] == 0 and r['log_lines'] <= 20:
                meta = latest_meta(by_id[r['id']])
                if blank_pdf(os.path.join(meta['dir'], 'final', meta['jobname'] + '.pdf')):
                    r['verdict'], r['why'] = 'vacuous', 'blank-output'
    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    run.write_json(args.out, {'entries': {r['id']: {k: r.get(k) for k in ('verdict', 'why', 'same_as', 'oracle_key', 'source')} for r in records}, 'generated': run.now()})

    verdicts = collections.Counter(r['verdict'] for r in records)
    print(f'{len(records)} entries from {len(files)} manifests: ' + ', '.join(f'{k} {v}' for k, v in verdicts.most_common()))
    rows = collections.defaultdict(collections.Counter)
    for r in records:
        rows[r['source']][r['verdict'] if r['verdict'] != 'vacuous' and r['verdict'] != 'duplicate' else f'{r["verdict"]}:{r.get("why")}'] += 1
        if r['source'] == 'coverage':
            rows['coverage/' + category_of(r['id'])][r['verdict'] if r['verdict'] not in ('vacuous', 'duplicate') else f'{r["verdict"]}:{r.get("why")}'] += 1
    cols = ['substantive', 'vacuous:fatal-at-load', 'vacuous:no-judged-text', 'vacuous:empty-run', 'vacuous:blank-output', 'duplicate:same-closure', 'duplicate:same-output', 'not-comparable', 'excluded', 'unprepared']
    print(f'{"source":<30}{"total":>7}' + ''.join(f'{c.split(":")[-1][:12]:>14}' for c in cols))
    for s in sorted(rows):
        row = rows[s]
        print(f'{s:<30}{sum(row.values()):>7}' + ''.join(f'{row[c]:>14}' for c in cols))
    comparable = [r for r in records if r['verdict'] in ('substantive', 'vacuous', 'duplicate')]
    subst = [r for r in comparable if r['verdict'] == 'substantive']
    print(f'\ndenominator: {len(comparable)} comparable entries, of which {len(subst)} substantive ({100 * len(subst) / max(1, len(comparable)):.1f}%)')
    print(f'pages in substantive entries: {sum(r["pages"] for r in subst)}; pages in all comparable: {sum(r["pages"] for r in comparable)}')
    print(f'oracle errors: {sum(1 for r in subst if r["errors"])} substantive entries have TeX errors in their logs')
    families = collections.Counter(r.get('same_as') for r in records if r.get('why') == 'same-output')
    print(f'largest same-output families: ' + ', '.join(f'{k}: {v + 1}' for k, v in families.most_common(12)))
    if args.list:
        for r in records:
            if args.list in (r['verdict'], r.get('why')):
                print(f'  {r["id"]}' + (f'  = {r["same_as"]}' if r.get('same_as') else '') + (f'  {r.get("first_error")}' if r.get('first_error') else ''))
    print(f'details: {args.out}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
