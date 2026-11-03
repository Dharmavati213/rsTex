#!/usr/bin/env -S python3 -I
r"""Every tikzcd diagram the corpus has, one per page, for comparing the native path of DESIGN section 10 against real PGF.

    python3 -I tests/corpus/cd/extract.py [--no-private]        # writes the documents and the manifests, prints what it found
    python3 -I tests/corpus/run.py prepare --manifest tests/corpus/manifests/cd.json -j 8
    python3 -I tests/corpus/cd/extract.py --after-prepare       # per diagram: did it compile, dimensions, what keys it uses
    python3 -I tests/corpus/cd/compare_native.py --engine E     # the comparison itself

Where the diagrams come from: every document of the corpus (the public sources, the arXiv sample, the LaTeX and pgf test suites, tests/cd, and the owner's documents)
whose oracle read a project file holding \begin{tikzcd}.  So the set is the diagrams people wrote, not ones written for this test.

For a document with diagrams this writes one file: its own preamble, read from its main file, then the simple definitions found in its body (a diagram may use them), then
one page per diagram:

    \setbox\cdunit=\hbox{\begin{tikzcd}...\end{tikzcd}}
    \typeout{[[unit NAME \the\wd\cdunit\space\the\ht\cdunit\space\the\dp\cdunit]]}
    \shipout\box\cdunit

Diagrams with the same text under the same preamble are written once.  What is not written is counted with its reason: a diagram inside a macro body (it has #), a document whose
\begin{document} is not in its main file.  Everything here reads downloads as bytes and runs nothing from them.  Output of the owner's documents goes to target/ and to a
.local manifest, both untracked, and this program prints no diagram text from them.
"""
import argparse
import collections
import hashlib
import importlib.util
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
CORPUS = os.path.dirname(HERE)
spec = importlib.util.spec_from_file_location('corpus_audit', os.path.join(CORPUS, 'audit.py'))
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)
run = audit.run
GEN = os.path.join(run.TARGET, 'gen', 'cd')
GEN_LOCAL = os.path.join(run.TARGET, 'gen', 'cd-local')
MANIFEST = os.path.join(run.MANIFEST_DIR, 'cd.json')
MANIFEST_LOCAL = os.path.join(run.MANIFEST_DIR, 'cd.local.json')
REPORT = os.path.join(run.TARGET, 'audit', 'cd.json')
SPLIT = os.path.join(run.TARGET, 'audit', 'cd-split.json')
DIAGRAM = re.compile(rb'\\begin\{tikzcd\}.*?\\end\{tikzcd\}', re.S)
DEFINITION = re.compile(rb'^\s*\\(?:newcommand|renewcommand|providecommand|DeclareMathOperator|def|let|tikzset|tikzcdset|newcount|newlength|setlength|definecolor)\b[^\n]*$', re.M)
KEY = re.compile(rb'\\(?:arrow|ar)\s*\[([^\]]*)\]|\\begin\{tikzcd\}\s*\[([^\]]*)\]')


def strip_comments(data):
    return re.sub(rb'(?m)(?<!\\)%[^\n]*$', b'', data)


def balanced(line):
    return line.count(b'{') == line.count(b'}')


def tex_files_with_diagrams(e, meta):
    """Project files the oracle read that hold a diagram, in reading order."""
    seen = []
    for path in meta['inputs']:
        if path.startswith(run.TDS_PREFIXES) or not path.endswith(('.tex', '.ltx', '.inc', '.sty', '.cd')):
            continue
        data = run.open_bytes(path)
        if b'\\begin{tikzcd}' in data:
            seen.append(path)
    return seen


def slug(entry_id):
    return re.sub(r'[^A-Za-z0-9._-]+', '_', entry_id)[:90] + '-' + hashlib.sha1(entry_id.encode()).hexdigest()[:8]


def main_preamble(e):
    if not e.get('main'):
        return None
    data = run.open_bytes(e['main'])
    m = re.search(rb'^[^%\n]*\\begin\s*\{document\}', data, re.M)
    if not m:
        return None
    return data[:m.start()], data[m.end():]


def entry_private(e):
    """Sources that read the owner's documents stay in the .local manifest."""
    return e['manifest'].endswith('.local.json') or e['source'] in ('user', 'local')


def build(args):
    entries, _, _ = run.load_manifests(None)
    public, private = [], []
    stats = collections.Counter()
    per_source = collections.defaultdict(collections.Counter)
    seen_text = set()
    keys = collections.Counter()
    try:
        split = set(run.read_json(SPLIT))
    except (OSError, ValueError):
        split = set()
    def emit(e, files, priv):
        src = e['source']
        parts = main_preamble(e)
        total_here = sum(len(DIAGRAM.findall(strip_comments(run.open_bytes(p)))) for p in files)
        stats['documents with diagrams'] += 1
        stats['diagrams found'] += total_here
        per_source[src]['documents'] += 1
        per_source[src]['diagrams'] += total_here
        if parts is None:
            stats['skipped: \\begin{document} not in the main file'] += total_here
            per_source[src]['no document line in main'] += total_here
            return
        preamble, body = parts
        preamble_hash = hashlib.sha1(preamble).hexdigest()
        defs = [m.group().strip() for m in DEFINITION.finditer(strip_comments(body)) if balanced(m.group()) and b'#' not in m.group().replace(b'#1', b'').replace(b'#2', b'')]
        units, pages = [], []
        for path in files:
            for raw in DIAGRAM.findall(strip_comments(run.open_bytes(path))):
                norm = re.sub(rb'\s+', b' ', raw)
                if b'#' in raw:
                    stats['skipped: inside a macro body (has #)'] += 1
                    per_source[src]['in a macro body'] += 1
                    continue
                k = hashlib.sha1(preamble_hash.encode() + norm).hexdigest()
                if k in seen_text:
                    stats['skipped: same text under the same preamble'] += 1
                    per_source[src]['same text'] += 1
                    continue
                seen_text.add(k)
                name = f'd{len(units) + 1:04d}-{k[:8]}'
                units.append(name)
                pages.append(b'\\typeout{[[unit-start ' + name.encode() + b']]}\n\\setbox\\cdunit=\\hbox{' + raw + b'}\n'
                             b'\\typeout{[[unit ' + name.encode() + b' \\the\\wd\\cdunit\\space\\the\\ht\\cdunit\\space\\the\\dp\\cdunit]]}\n\\shipout\\box\\cdunit\n')
                for m in KEY.finditer(raw):
                    for key in re.split(rb',', (m.group(1) or m.group(2) or b'')):
                        key = re.sub(rb'=.*', b'', key, flags=re.S).strip()
                        if key and not priv:
                            keys[key.decode('latin-1')[:40]] += 1
                stats['diagrams written'] += 1
                per_source[src]['written'] += 1
        if not units:
            return
        root = GEN_LOCAL if priv else GEN
        base_id = ('cd-local/' if priv else 'cd/') + e['id']
        # One bad diagram (it ends a group it did not open, or uses something from the body) takes all later ones in the same file with it.  Such a document is
        # written again with one file per diagram.
        groups = [(base_id + '#' + u, [u], [p]) for u, p in zip(units, pages)] if base_id in split else [(base_id, units, pages)]
        for gid, gunits, gpages in groups:
            d = os.path.join(root, slug(gid))
            os.makedirs(d, exist_ok=True)
            out = preamble + b'\n\\begin{document}\n\\newbox\\cdunit\n' + b'\n'.join(defs) + b'\n' + b''.join(gpages) + b'\\end{document}\n'
            with open(os.path.join(d, 'diagrams.tex'), 'wb') as f:
                f.write(out)
            where = (lambda x: x) if priv else (lambda x: os.path.relpath(x, run.REPO))     # the committed manifest carries no path of this machine
            item = {'id': gid, 'main': where(os.path.join(d, 'diagrams.tex')), 'cwd': where(d), 'jobname': 'diagrams', 'units': gunits,
                    'texinputs': sorted(where(x) for x in {e['cwd'], os.path.dirname(e['main'])} | set(e['texinputs'])), 'tier': 'full', 'passes': 1,
                    'notes': f'{len(gunits)} diagrams from {e["id"]}'}
            (private if priv else public).append(item)

    for e in entries:
        if e.get('excluded') or e['source'].startswith(('coverage', 'mutation', 'cd')):
            continue
        priv = entry_private(e)
        if priv and args.no_private:
            continue
        meta = audit.latest_meta(e)
        if meta is None:
            continue
        files = tex_files_with_diagrams(e, meta)
        if files:
            emit(e, files, priv)
    # tests/cd is written for this, one file to a topic, and is in no other manifest: judged as whole documents, where the text around a diagram matters, and per diagram.
    cd_dir = os.path.join(run.REPO, 'tests', 'cd')
    for name in sorted(os.listdir(cd_dir)):
        path = os.path.join(cd_dir, name)
        if name.endswith('.tex') and os.path.isfile(path):
            private_copy = os.path.islink(path)
            if private_copy and args.no_private:
                continue
            count = len(DIAGRAM.findall(strip_comments(run.open_bytes(path))))
            rel = path if private_copy else os.path.relpath(path, run.REPO)
            (private if private_copy else public).append({'id': ('cd-local/' if private_copy else 'cd/') + 'tests-cd/' + name[:-4], 'main': rel, 'tier': 'standard', 'passes': 'auto', 'trusted': True, 'notes': f'{count} diagrams, judged as one document'})
            emit({'id': 'tests-cd-units/' + name[:-4], 'source': 'tests-cd', 'main': path, 'cwd': cd_dir, 'texinputs': []}, [path], private_copy)
    run.write_json(MANIFEST, {'source': 'cd', 'description': 'Every tikzcd diagram in the corpus, one per page. Written by tests/corpus/cd/extract.py.',
                              'exclusions': [{'what': 'tikzcd in macro bodies, and documents whose \\begin{document} is not in the main file', 'why': 'No literal diagram text to take out; they are compared as part of their whole document in the other sources. Counted by extract.py.'},
                                             {'what': 'Documents the oracle cannot compile', 'why': 'Only files the oracle read are searched, so a document that dies early contributes what it reached.'}],
                              'defaults': {'gate_log': True}, 'entries': public})
    if private:
        run.write_json(MANIFEST_LOCAL, {'source': 'cd-local', 'description': 'The owner\'s tikzcd diagrams. Not committed; the documents live under target/.', 'defaults': {'gate_log': True}, 'entries': private})
    print(f'documents: {len(public)} public, {len(private)} private')
    for k, v in stats.most_common():
        print(f'  {k}: {v}')
    print('per source:')
    for s, row in sorted(per_source.items()):
        print(f'  {s:<24} ' + ', '.join(f'{k} {v}' for k, v in row.items()))
    print('keys on \\arrow and \\begin{tikzcd} in public sources, most common first:')
    print('  ' + ', '.join(f'{k}:{v}' for k, v in keys.most_common(80)))
    run.write_json(REPORT, {'stats': stats, 'per_source': per_source, 'keys': keys})
    return 0


def after_prepare(args):
    manifests = [MANIFEST] + ([MANIFEST_LOCAL] if os.path.isfile(MANIFEST_LOCAL) and not args.no_private else [])
    entries, _, _ = run.load_manifests(manifests)
    rows = collections.defaultdict(collections.Counter)
    units = {}
    failing_docs = []
    for e in entries:
        meta = audit.latest_meta(e)
        if meta is None:
            rows[e['source']]['unprepared documents'] += 1
            continue
        if run.effective_class(meta, e) != 'normal':
            rows[e['source']]['documents the oracle fails'] += 1
            rows[e['source']]['diagrams in those'] += len(e.get('units', []))
            failing_docs.append((e['id'], (meta.get('class_reason') or '')[:120]))
            continue
        rows[e['source']]['documents'] += 1
        data = run.open_bytes(os.path.join(meta['dir'], 'final', 'diagrams.log'))
        lines = re.sub(rb'\n(?=[^\n]{0,78}\]\])', b'', data)
        starts = {m.group(1).decode(): m.start() for m in re.finditer(rb'\[\[unit-start ([^\]]+)\]\]', lines)}
        ends = {m.group(1).decode(): (m.start(), m.group(2).decode()) for m in re.finditer(rb'\[\[unit ([^ \]]+) ([^\]]*)\]\]', lines)}
        for u in e.get('units', []):
            rows[e['source']]['diagrams'] += 1
            if u not in starts or u not in ends:
                rows[e['source']]['diagrams never reached'] += 1
                continue
            has_error = b'\n! ' in lines[starts[u]:ends[u][0]]
            good = not has_error and not ends[u][1].startswith('0.0pt')
            rows[e['source']]['diagrams clean' if good else 'diagrams with TeX errors']+= 1
            units[e['id'] + '#' + u] = {'dims': ends[u][1], 'clean': good}
    for s, row in sorted(rows.items()):
        print(f'{s:<10} ' + ', '.join(f'{k} {v}' for k, v in row.items()))
    for i, r in failing_docs[:40]:
        print(f'  oracle fails: {i}: {r}')
    if len(failing_docs) > 40:
        print(f'  ... {len(failing_docs) - 40} more')
    run.write_json(os.path.join(run.TARGET, 'audit', 'cd-units.json'), units)
    # A document that fails on a missing package fails however it is cut.  Others get one file per diagram on the next build.
    missing = re.compile(r"not found|not loadable|Cannot find the file|fontspec|Unknown option")
    try:
        before = set(run.read_json(SPLIT))
    except (OSError, ValueError):
        before = set()
    new = before | {i for i, r in failing_docs if not missing.search(r) and '#' not in i and len(next((e for e in entries if e['id'] == i), {}).get('units', [])) > 1}
    if new != before:
        run.write_json(SPLIT, sorted(new))
        print(f'{len(new) - len(before)} documents will be split per diagram: run extract.py and prepare again')
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--no-private', action='store_true', help='leave out the owner\'s documents')
    ap.add_argument('--after-prepare', action='store_true')
    args = ap.parse_args()
    return after_prepare(args) if args.after_prepare else build(args)


if __name__ == '__main__':
    sys.exit(main())
