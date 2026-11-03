#!/usr/bin/env -S python3 -I
"""gen_local_manifest.py -- sweep the owner's LaTeX documents into tests/corpus/manifests/local.local.json.

The manifest is gitignored because it names private paths.  This script holds no document text and no personal data, only
directory names under the home directory.

    python3 -I tests/corpus/gen_local_manifest.py --extra-root thesis=DIR     # phase 1: every master, tiers provisional
    python3 -I tests/corpus/run.py prepare --manifest tests/corpus/manifests/local.local.json -j 8 --run-id local-1
    python3 -I tests/corpus/gen_local_manifest.py --extra-root thesis=DIR --results target/corpus/runs/local-1/results.json     # phase 2

The thesis directory is given on the command line because tools/privacy_guard.py keeps its name out of tracked text.  The
generated manifest, which is gitignored, holds it.

A master is a .tex file that has an uncommented \\documentclass or \\documentstyle and \\begin{document}; or \\begin{document}
and an \\input of a file with \\documentclass (the EGA style); or \\documentclass or \\bye, when no other file of the same
tree \\inputs it (so a preamble is not a master, and a fixture deliberately missing \\begin{document} is one).  Everything
else is a fragment.  Phase 2 checks each fragment against the .fls of the compiled masters and reports whether it is read
by one.

Phase 2 reads the oracle results and:
- marks an entry `excluded` when its PDF is byte-identical, /ID aside, to that of an earlier entry and it reads the same
  private inputs by content.  Copies of a document would inflate the denominator without testing anything new.
- assigns tiers: `full` for over FULL_CPU_MS of CPU per pass or over FULL_PAGES pages; `smoke` for the fastest clean document
  of each family; `standard` for the rest.
- records `expected_class` from what the oracle did, and pages, time and errors in `notes`.

Everything left out is named, with a count and a reason, in the manifest's `exclusions`.
"""
import argparse
import hashlib
import json
import os
import re
import sys

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(REPO, 'tests', 'corpus', 'manifests', 'local.local.json')
SWEEP = os.path.join(REPO, 'target', 'corpus', 'local-sweep.json')
HOME = os.path.expanduser('~')
SLOP = '~/Projects/SlopdeGeometrieAlgebrique'
SHIM_DIR = 'target/corpus/sga4-shim'
FR_SHIM_PRELUDE = os.path.join(REPO, 'target', 'corpus', 'sga4-fr-shim', 'prelude.txt')
FR_TREE = '/source/SGA4/SGA4_orig/tree/'
STANDINS = 'target/corpus/standins'
AURICAL = r"""\NeedsTeXFormat{LaTeX2e}
\ProvidesPackage{aurical}[2099/01/01 stand-in written by tests/corpus/gen_local_manifest.py; the real package is not installed here]
\newcommand\Fontauri{\itshape}
\newcommand\Fontlukas{\itshape}
\newcommand\Fontskrivan{\itshape}
\newcommand\Fontamici{\itshape}
\endinput
"""

FULL_CPU_MS = 5000      # a pass using more CPU than this makes the entry full
FULL_PAGES = 200

# Entries that need a stated bound.  Each stays in the corpus; the note says what was bounded and why.
SPECIAL = {
    'local/sga-fr/SGA1/SGA1_orig/sga1-smf': {
        'timeout': 3000, 'passes': 2,
        'note': 'passes bounded to 2 because one pass takes many minutes under load, with over 90% of profile samples in \\hook_gput_next_code:nn under \\@thm.  '
                'A real token-machine stress test.  babel-french is missing, hence the errors'},
}

# (family, root, directory names not to descend into).  Order decides which copy of a duplicate is kept.
ROOTS = [
    ('sga1', SLOP + '/translation/SGA1', ()),
    ('sga2', SLOP + '/translation/SGA2', ()),
    ('sga3', SLOP + '/translation/SGA3', ()),
    ('sga4-shim', SLOP + '/translation/SGA4', ('tools',)),
    ('sga-fr', SLOP + '/source', ('build',)),
    ('math', '~/math', ()),
    ('docs-math', '~/Documents/math', ()),
    ('docs', '~/Documents', ('math',)),
    ('apply', '~/berwerbung', ()),
    ('work', '~/Work', ()),
    ('lemma', '~/Projects/lemma', ('target',)),
    ('lemma-installed', '~/.local/share/lemma', ()),
]
FORBIDDEN = ('.git', 'node_modules', '__pycache__')

# Stated in the manifest with the number the sweep measures.  {n} is filled in at generation time.
NOT_WALKED = [
    (SLOP + '/source/SGA4/build', 'generated wrappers around single chunks, build output rather than documents.  Their text is exactly what the 21 '
     'English and 24 French SGA 4 masters compile, so they would add correlated entries and no new text.  As written they are LuaLaTeX and stop at fontspec on pdflatex'),
    ('~/Projects/lemma/target', "temporary trees written by Lemma's own test suite, not documents"),
    (SLOP + '/translation/SGA4/tools', 'scripts, no documents'),
]
STATEMENTS = [
    {'what': '~/.claude-science', 'why': 'another application\'s private state, not in the owner\'s list of document directories.  About 24 .tex files, '
     'draft snapshots of the thesis.  Not compiled'},
    {'what': 'other hidden directories and conda packages under the home directory', 'why': 'application data and third-party package files, not the owner\'s documents'},
    {'what': 'SGA 4 on pdfTeX', 'why': 'both versions are written for LuaLaTeX and stop at fontspec on pdflatex as written.  The 21 English and 24 French masters compile with generated stand-ins for '
     'their macro files (tests/corpus/gen_sga4_shim.py), sources unmodified.  Fonts, language and line breaks are not what the owner reads; this tests pdfTeX fidelity on this mathematics'},
    {'what': 'ryankeleti EGA sources', 'why': 'aurical.sty is not installed on this machine, and the sources use it for one font switch in sheaf names, so a one-line stand-in is first in TEXINPUTS '
     '(written by this generator to target/corpus/standins).  Those names use the italic shape.  The book target of its Makefile is generated by a script from the download and was not run; each part compiles on its own.  '
     'Cross references between parts stay undefined because no .aux of another part exists beside the sources'},
    {'what': '~/math/tom1 and ~/Documents/math/tom1', 'why': 'PDFs only, no LaTeX source'},
    {'what': '~/.local/share/rstex-private', 'why': 'gitignored excerpts of tests/cd, fragments, belonging to the diagram tests'},
    {'what': 'French SGA 1 and SGA 2 originals', 'why': 'compiled as written.  babel-french is not installed on this machine, so \\og, \\fg, \\no and \\No are undefined; SGA 1 still gives a PDF, SGA 2 stops at 100 errors in one paragraph and is oracle-fails'},
]

CLASS_RE = re.compile(rb'^[^%\n]*\\document(?:class|style)\b', re.M)
BEGIN_RE = re.compile(rb'^[^%\n]*\\begin\s*\{document\}', re.M)
BYE_RE = re.compile(rb'^[^%\n]*\\bye\b', re.M)
INPUT_RE = re.compile(rb'^[^%\n]*?\\(?:input|include|subfile)\s*\{?\s*([^\s{}]+?)\s*(?:\}|\s|$)', re.M)
UP_RE = re.compile(rb'\.\./|\\input@path|\\(?:input|include|documentclass|usepackage)(?:\[[^]]*\])?\s*\{/')
INDEX_RE = re.compile(rb'^[^%\n]*\\makeindex\b', re.M)


def tilde(p):
    return '~' + p[len(HOME):] if p == HOME or p.startswith(HOME + '/') else p


def scan():
    """Every .tex below the roots, classified.  Returns (records in root order, tree counts)."""
    records, seen = [], set()
    for family, root, skip in ROOTS:
        base = os.path.expanduser(root)
        for d, subdirs, names in os.walk(base):
            subdirs[:] = sorted(s for s in subdirs if s not in skip and s not in FORBIDDEN)
            for n in sorted(names):
                if not n.endswith('.tex'):
                    continue
                p = os.path.join(d, n)
                real = os.path.realpath(p)
                if real in seen:
                    continue
                seen.add(real)
                with open(p, 'rb') as f:
                    data = f.read()
                records.append({'family': family, 'root': base, 'path': p, 'size': len(data), 'sha': hashlib.sha256(data).hexdigest(),
                                'class': bool(CLASS_RE.search(data)), 'begin': bool(BEGIN_RE.search(data)), 'bye': bool(BYE_RE.search(data)),
                                'up': bool(UP_RE.search(data)), 'index': bool(INDEX_RE.search(data)),
                                'inputs': [m.decode('latin-1') for m in INPUT_RE.findall(data)]})
    return records


def resolve_input(rec, name, index):
    """The record an \\input{name} of rec refers to, relative to its directory, or None."""
    d = os.path.dirname(rec['path'])
    for cand in (name, name + '.tex'):
        p = os.path.normpath(os.path.join(d, cand))
        if p in index:
            return index[p]
    return None


def classify(records):
    index = {r['path']: r for r in records}
    referenced = set()
    for r in records:
        for name in r['inputs']:
            t = resolve_input(r, name, index)
            if t:
                referenced.add(t['path'])
    for r in records:
        reaches_class = False
        for name in r['inputs']:
            t = resolve_input(r, name, index)
            if t and t['class']:
                reaches_class = True
        if r['class'] and r['begin']:
            r['master'] = 'documentclass and begin'
        elif r['begin'] and reaches_class:
            r['master'] = 'begin, class in an input'
        elif (r['class'] or r['bye']) and r['path'] not in referenced:
            r['master'] = 'class or bye, not input by another file'
        else:
            r['master'] = None
    return records


def entry_id(r):
    rel = os.path.relpath(r['path'], r['root'])
    return f'local/{r["family"]}/{os.path.splitext(rel)[0]}'


def build_entries(records, user_mains):
    entries = []
    for r in records:
        if not r['master']:
            continue
        e = {'id': entry_id(r), 'main': tilde(r['path'])}
        if r['up'] or r['family'] == 'sga4-shim':
            e['trusted'] = True
        if r['family'] == 'sga4-shim':
            e['texinputs'] = [SHIM_DIR]
        if '/ryankeleti-ega' in r['path'] and any(os.path.basename(i) == 'preamble' for i in r['inputs']):
            e['texinputs'] = [STANDINS]
        if FR_TREE in r['path'] and r['up']:
            if os.path.isfile(FR_SHIM_PRELUDE):
                with open(FR_SHIM_PRELUDE) as f:
                    e['prelude'] = f.read()
            e['trusted'] = True
        special = SPECIAL.get(e['id'])
        if special:
            e.update({k: v for k, v in special.items() if k != 'note'})
        if r['index']:
            e['index'] = 'makeindex'
        entries.append(e)
    return entries


def oracle_meta(result):
    """The meta.json behind one prepare result, or None."""
    d = result.get('dir')
    if not d:
        return None
    try:
        with open(os.path.join(d, 'meta.json')) as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


ID_RE = re.compile(rb'/ID \[<[0-9A-Fa-f]*> ?<[0-9A-Fa-f]*>\]')


def pdf_digest(result, job):
    try:
        with open(os.path.join(result['dir'], 'final', job + '.pdf'), 'rb') as f:
            return hashlib.sha256(ID_RE.sub(b'', f.read())).hexdigest()
    except (OSError, KeyError):
        return None


def private_closure(meta):
    """Content hashes of what the run read outside the TeX tree, by content: a copy in another directory has the same set."""
    return frozenset(h for p, h in meta['inputs'].items()
                     if not p.startswith(('/usr/share/', '/var/lib/', '/etc/', '/usr/local/', HOME + '/.texlive')))


def apply_results(entries, results_path, user_ids):
    with open(results_path) as f:
        results = {r['id']: r for r in json.load(f)['results']}
    canon, stats = {}, {'duplicates': 0, 'no_result': []}
    if any(r.get('class') == 'excluded' for r in results.values()):
        sys.exit(f'{results_path} comes from a phase 2 manifest, whose copies were not compiled.  Regenerate the phase 1 manifest and run prepare on that; '
                 'its oracles are cached.')
    for e in entries:
        r = results.get(e['id'])
        if not r:
            stats['no_result'].append(e['id'])
            continue
        job = os.path.splitext(os.path.basename(e['main']))[0]
        cls = r.get('class')
        notes = []
        if r.get('pages') is not None:
            notes.append(f'{r["pages"]} pp')
        if r.get('pass_ms'):
            notes.append(f'{r["passes_run"]} passes, last {r["pass_ms"][-1] / 1000:.2f} s')
        if r.get('errors'):
            notes.append(f'{r["errors"]} TeX errors, first: {r.get("first_error")}')
        if cls in ('oracle-fails', 'nondeterministic'):
            e['expected_class'] = cls
            notes.append(f'{cls}: {(r.get("reason") or "")[:200]}')
        special = SPECIAL.get(e['id'])
        if special:
            notes.append(special['note'])
        if notes:
            e['notes'] = '; '.join(str(n) for n in notes)
        if cls == 'normal':
            meta = oracle_meta(r)
            digest = pdf_digest(r, job)
            if meta and digest:
                key = (digest, private_closure(meta))
                if key in canon:
                    e['excluded'] = f'copy of {canon[key]}: same PDF apart from /ID, same private inputs by content'
                    stats['duplicates'] += 1
                else:
                    canon[key] = e['id']
                    if key in user_ids:
                        e['excluded'] = f'copy of {user_ids[key]} in manifests/user.local.json'
                        stats['duplicates'] += 1
    return results, stats


def assign_tiers(entries, results):
    best = {}
    for e in entries:
        e['tier'] = 'standard'
        r = results.get(e['id'])
        if not r or e.get('excluded') or r.get('class') != 'normal':
            continue
        meta = oracle_meta(r)
        ms = (meta.get('pass_cpu_ms') or [0])[-1] if meta else 0
        pages = r.get('pages') or 0
        if ms > FULL_CPU_MS or pages > FULL_PAGES:
            e['tier'] = 'full'
        fam = e['id'].split('/')[1]
        if pages >= 1 and not r.get('errors') and (fam not in best or ms < best[fam][0]):
            best[fam] = (ms, e)
    for fam, (ms, e) in best.items():
        if e['tier'] != 'full':
            e['tier'] = 'smoke'


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--results', help='results.json of a prepare run of this manifest: excludes duplicates, assigns tiers')
    ap.add_argument('--user-results', help='results.json of a prepare run that covers manifests/user.local.json')
    ap.add_argument('--out', default=OUT)
    ap.add_argument('--extra-root', action='append', default=[], metavar='FAMILY=DIR',
                    help='another directory to sweep, for one whose name must not appear in this tracked file; repeatable')
    args = ap.parse_args()
    for spec in args.extra_root:
        family, _, path = spec.partition('=')
        if not family or not path:
            sys.exit(f'--extra-root wants FAMILY=DIR, got {spec!r}')
        ROOTS.append((family, path, ()))
    os.makedirs(os.path.join(REPO, STANDINS), exist_ok=True)
    with open(os.path.join(REPO, STANDINS, 'aurical.sty'), 'w') as f:
        f.write(AURICAL)
    records = classify(scan())
    entries = build_entries(records, ())
    exclusions = []
    for where, why in NOT_WALKED:
        p = os.path.expanduser(where)
        n = m = 0
        for d, subdirs, names in os.walk(p):
            subdirs[:] = [s for s in subdirs if s not in FORBIDDEN]
            for name in names:
                if name.endswith('.tex'):
                    n += 1
                    with open(os.path.join(d, name), 'rb') as f:
                        m += bool(CLASS_RE.search(f.read()))
        exclusions.append({'what': tilde(p), 'why': why, 'tex_files': n, 'with_documentclass': m})
    exclusions += STATEMENTS
    fragments = [r for r in records if not r['master']]
    exclusions.append({'what': f'{len(fragments)} fragments, .tex files that are no master by the rule in gen_local_manifest.py',
                       'why': 'body text or preamble with no \\begin{document} of its own, so not compilable alone.  Their text is compiled through the masters that \\input them; target/corpus/local-sweep.json lists each'})
    stats = {}
    results = {}
    if args.results:
        user_ids = {}
        if args.user_results:
            with open(args.user_results) as f:
                for r in json.load(f)['results']:
                    if r.get('class') == 'normal' and r['id'].startswith('user/'):
                        meta = oracle_meta(r)
                        job = next((x for x in [r['id'].split('/')[-1]]), None)
                        for jname in {job, meta.get('jobname')} if meta else ():
                            d = pdf_digest(r, jname)
                            if d:
                                user_ids[(d, private_closure(meta))] = r['id']
        results, stats = apply_results(entries, args.results, user_ids)
        assign_tiers(entries, results)
        exclusions.append({'what': f'{stats["duplicates"]} entries marked excluded as copies', 'why': 'each names the entry it copies; a copy tests nothing new and would inflate the denominator'})
        # Which fragments do the compiled masters read?
        read = set()
        for r in results.values():
            meta = oracle_meta(r)
            if meta:
                read.update(os.path.realpath(p) for p in meta['inputs'])
        for f in fragments:
            f['read_by_a_master'] = os.path.realpath(f['path']) in read
    manifest = {
        'source': 'local',
        'description': 'Every LaTeX document the owner already has, read from their own directories and never copied into the repo. '
                       'Generated by tests/corpus/gen_local_manifest.py.  This file is gitignored.',
        'defaults': {'passes': 'auto', 'bib': 'auto', 'l6': True},
        'exclusions': exclusions,
        'entries': entries,
    }
    with open(args.out, 'w') as f:
        json.dump(manifest, f, indent=1)
    os.makedirs(os.path.dirname(SWEEP), exist_ok=True)
    fams = {}
    for r in records:
        c = fams.setdefault(r['family'], {'tex': 0, 'bytes': 0, 'masters': 0, 'fragments': 0, 'fragments_read': 0, 'fragments_unread': 0})
        c['tex'] += 1
        c['bytes'] += r['size']
        c['masters' if r['master'] else 'fragments'] += 1
        if not r['master'] and 'read_by_a_master' in r:
            c['fragments_read' if r['read_by_a_master'] else 'fragments_unread'] += 1
    with open(SWEEP, 'w') as f:
        json.dump({'families': fams, 'stats': stats, 'exclusions': exclusions,
                   'fragments': [{'path': tilde(r['path']), 'bytes': r['size'], 'read_by_a_master': r.get('read_by_a_master')} for r in fragments],
                   'masters': [{'id': entry_id(r), 'why': r['master'], 'bytes': r['size']} for r in records if r['master']]}, f, indent=1)
    print(f'{len(records)} .tex files: {sum(1 for r in records if r["master"])} masters, {len(fragments)} fragments', file=sys.stderr)
    for fam, c in fams.items():
        print(f'  {fam:<16} {c}', file=sys.stderr)
    if stats:
        print(f'  duplicates excluded: {stats["duplicates"]}; entries without a result: {stats["no_result"]}', file=sys.stderr)
    print(f'wrote {args.out}', file=sys.stderr)


if __name__ == '__main__':
    main()
