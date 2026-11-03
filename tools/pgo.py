#!/usr/bin/env -S python3 -I
"""pgo.py: the release binary.  Instruments rstex, trains it on documents, merges, rebuilds with the profile, and measures what that gained.

    python3 -I tools/pgo.py [-j N] [--train auto|public|all|NAME,...] [--use public|NAME,...] [--weights equal|sqrt|raw] [--name NAME] [--flags=RUSTFLAGS] [--order]
                            [--reuse-training] [--no-bench] [--no-plain] [--bench-docs NAME,...] [-n N] [--counters N] [--compare NAME=PATH]...

With no options this runs start to finish on stable Rust and leaves the binary at <target>/pgo-build/final/rstex.  docs/BUILD.md says how a release uses it, and what was
measured for each choice made here.

Steps, each a directory under <target>/pgo-build, which is under target/ since a profile derives from what it was trained on:
    gen/      cargo build --release with -Cprofile-generate
    raw/      a .profraw per training run
    data/     a .profdata per training document, kept so that another weighting needs no new training (--reuse-training)
    merged/   the merged profile, named by its content so that cargo sees a new one as a change of flags
    final/    the release build with -Cprofile-use, or another NAME with --name
    plain/    the release build with no profile, for the table

The flags are .cargo/config.toml's and these, by --config, so there is one place for +crt-static.  --target keeps them off build scripts, which would write profiles too.

Training.  The public set is the ladder, which is in the repository, both format builds, and those public corpus entries that refs/corpus holds: anyone can build it.
The private set is what a *.local.json manifest names, when it is there.  --train auto, the default, takes both, and --train public is for a release.  A run is one last
pass, as bench.py runs it.

Weights.  llvm-profdata adds the counters, so a document weighs what it runs, and hello is 1/370 of SGA3.  --weights equal scales each document to the same total, and a
format build counts as one document.  That is what repeating a small document in training would do, without the time, since a run is deterministic.

Reading the table: compare by instructions, then cycles; tools/prof/bench.py's docstring has why, and its timed runs alternate between the engines.
"""
import argparse
import glob
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), 'prof'))
import proflib  # noqa: E402

REPO = proflib.REPO
# In the repository, so always there.  The formats are a target of DESIGN.md section 2 themselves, and the longest public macro run.
TRAIN_ALWAYS = ['ladder/*']
FORMATS = ['pdftex', 'pdflatex']
# Long public documents, when refs/corpus has them: a book like SGA3, a macro-heavy manual, papers like the thesis.  All end without an error, so that training does not run the error paths.
TRAIN_PUBLIC_OPTIONAL = ['books/hott/hott-a4', 'manuals/tikz-cd/tikz-cd-doc', 'arxiv/math.AG/1912.07119v3', 'arxiv/math.AG/2007.09646v1']
TRAIN_PRIVATE = ['main', 'LMB', 'summary', 'SGA1-VI', 'sga3']
BENCH_DOCS = ['hello', 'plain-hello', 'tikzcd-many', 'texweb', 'main', 'LMB', 'sga3']
# On the profile.  Block placement by ext-TSP: 4 to 6 % of the cycles, docs/BUILD.md.  A hidden LLVM option, so a later rustc may refuse it, and this tool then stops.
USE_FLAGS = ['-Cllvm-args=-enable-ext-tsp-block-placement=true']


def say(message):
    print(f'pgo: {message}', flush=True)


def run(cmd, **kw):
    r = subprocess.run(cmd, check=False, **kw)
    if r.returncode:
        proflib.die(f'failed, exit {r.returncode}: {" ".join(cmd)}')
    return r


def host_triple():
    out = subprocess.run(['rustc', '-vV'], capture_output=True, text=True, check=True).stdout
    return re.search(r'^host: (\S+)', out, re.M).group(1)


def profdata_tool():
    """rustc's own: a profile must be in the format of the LLVM that reads it, and the system's is another version."""
    sysroot = subprocess.run(['rustc', '--print', 'sysroot'], capture_output=True, text=True, check=True).stdout.strip()
    path = os.path.join(sysroot, 'lib', 'rustlib', host_triple(), 'bin', 'llvm-profdata')
    if not os.path.exists(path):
        proflib.die('no llvm-profdata in the sysroot: rustup component add llvm-tools')
    return path


def cargo_build(where, flags, jobs):
    """A release build of rstex under <where>, with `flags` added to the configuration's.  Returns the binary."""
    triple = host_triple()
    config = 'target.' + triple + '.rustflags=' + json.dumps(flags)
    cmd = ['cargo', 'build', '--release', '-p', 'rstex', '--target', triple, '--target-dir', where, '--config', config, '-j', str(jobs)]
    run(cmd, cwd=REPO)
    return os.path.join(where, triple, 'release', 'rstex')


def merge(tool, inputs, out):
    run([tool, 'merge', '-o', out] + inputs)


def total_count(tool, path):
    out = subprocess.run([tool, 'show', '--detailed-summary', path], capture_output=True, text=True, check=True).stdout
    return int(re.search(r'^Total count: (\d+)', out, re.M).group(1))


# ------------------------------------------------------------------------------------------------ training
def training_docs(spec):
    """The documents to train on, and those of the optional sets that are not there, which is no error."""
    entries, _, _ = proflib.corpus().load_manifests(None)
    by_id = {e['id']: e for e in entries}

    def have(name):
        e = by_id.get(proflib.NAMED.get(name, name))
        return e is not None and os.path.exists(e.get('main') or '')

    if spec in ('auto', 'public', 'all'):
        names = list(TRAIN_ALWAYS) + [n for n in TRAIN_PUBLIC_OPTIONAL if have(n)]
        missing = [n for n in TRAIN_PUBLIC_OPTIONAL if not have(n)]
        if spec != 'public':
            names += [n for n in TRAIN_PRIVATE if have(n)]
            missing += [n for n in TRAIN_PRIVATE if not have(n)]
        if spec == 'all' and any(not have(n) for n in TRAIN_PRIVATE):
            proflib.die('--train all and not every private document is there')
    else:
        names, missing = spec.split(','), []
    return proflib.resolve_docs(names), missing


def key_of(doc_id):
    return doc_id.replace('/', '_').replace('#', '_')


def train_doc(eng, doc, raw, data, tool):
    key = key_of(doc.id)
    d = os.path.join(raw, key)
    shutil.rmtree(d, ignore_errors=True)
    os.makedirs(d)
    work = proflib.Workdir(doc)
    argv, cwd, env = eng.command(doc, work.out)
    env['LLVM_PROFILE_FILE'] = os.path.join(d, 'p%p.profraw')
    r = proflib.run_timed(argv, cwd, env)
    work.remove()
    out = os.path.join(data, key + '.profdata')
    merge(tool, sorted(glob.glob(os.path.join(d, '*.profraw'))), out)
    shutil.rmtree(d, ignore_errors=True)
    return key, r['rc'], r['cpu_ms']


def train(gen, docs, args, tool, root):
    """One data/KEY.profdata per document and per format.  The format builds go first: the documents need them."""
    raw, data = os.path.join(root, 'raw'), os.path.join(root, 'data')
    shutil.rmtree(raw, ignore_errors=True)
    shutil.rmtree(data, ignore_errors=True)
    os.makedirs(raw)
    os.makedirs(data)
    eng = proflib.Engine('gen', gen)
    # A format is built once per binary and its marker says so.  This binary's profile is new, so its formats are too.
    shutil.rmtree(os.path.join(proflib.out_root(), 'fmt', eng.sha[:16]), ignore_errors=True)
    default_dir = os.path.join(root, 'gen-default')
    for name in FORMATS:
        t0 = time.monotonic()
        eng.format_path(name)
        inputs = sorted(glob.glob(os.path.join(default_dir, '*.profraw')))
        if not inputs:
            proflib.die(f'the format build of {name} wrote no profile to {default_dir}')
        merge(tool, inputs, os.path.join(data, 'fmt-' + name + '.profdata'))
        for f in inputs:
            os.remove(f)
        say(f'trained on the format {name}, {time.monotonic() - t0:.0f} s')
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        # The oracle's side files, from run.py's cache.  On a first run this is the slow part.
        list(pool.map(lambda d: d.prepare(), docs))
        say(f'the oracle has run, {time.monotonic() - t0:.0f} s for the last format and this')
        # The first run of a binary fills its tree cache: let a throwaway run do that.
        if docs:
            work = proflib.Workdir(docs[0])
            argv, cwd, env = eng.command(docs[0], work.out)
            env['LLVM_PROFILE_FILE'] = os.path.join(default_dir, 'discard-%p.profraw')
            proflib.run_timed(argv, cwd, env)
            work.remove()
        for key, rc, cpu_ms in pool.map(lambda d: train_doc(eng, d, raw, data, tool), docs):
            say(f'trained on {key}: exit {rc}, {cpu_ms / 1e3:.1f} s')
    shutil.rmtree(raw, ignore_errors=True)
    shutil.rmtree(default_dir, ignore_errors=True)


def merged_profile(args, tool, root):
    """All of data/, weighted.  Returns its path, named by its content.  The weights are written beside it, and printed."""
    data = os.path.join(root, 'data')
    inputs = sorted(glob.glob(os.path.join(data, '*.profdata')))
    if args.use == 'public':
        private = {key_of(proflib.NAMED.get(n, n)) for n in TRAIN_PRIVATE}
        inputs = [f for f in inputs if os.path.basename(f)[:-9] not in private]
    elif args.use:
        keep = {key_of(proflib.NAMED.get(n, n)) for n in args.use.split(',')}
        inputs = [f for f in inputs if os.path.basename(f)[:-9] in keep]
    if not inputs:
        proflib.die(f'nothing to merge in {data}: run without --reuse-training')
    totals = {f: total_count(tool, f) for f in inputs}
    top = max(totals.values())
    exponent = {'raw': 0, 'sqrt': 0.5, 'equal': 1}[args.weights]
    weights = {f: max(1, round((top / t) ** exponent)) for f, t in totals.items()}
    merged_dir = os.path.join(root, 'merged')
    os.makedirs(merged_dir, exist_ok=True)
    tmp = os.path.join(merged_dir, f'tmp-{os.getpid()}.profdata')
    run([tool, 'merge', '-o', tmp] + [f'-weighted-input={weights[f]},{f}' for f in inputs])
    h = hashlib.sha256()
    with open(tmp, 'rb') as f:
        h.update(f.read())
    final = os.path.join(merged_dir, h.hexdigest()[:16] + '.profdata')
    os.replace(tmp, final)
    print(f'{"training document":<44}{"total count":>18}{"weight":>8}')
    for f in inputs:
        print(f'{os.path.basename(f)[:-9]:<44}{totals[f]:>18}{weights[f]:>8}')
    return final


def function_counts(tool, path):
    """{symbol: sum of its block counts}.  A profile writes a function local to its module as module;symbol."""
    out = subprocess.run([tool, 'show', '--all-functions', '--counts', path], capture_output=True, text=True, check=True).stdout
    res, name = {}, None
    for line in out.splitlines():
        if line.startswith('  ') and not line.startswith('    ') and line.endswith(':'):
            name = line.strip()[:-1].rsplit(';', 1)[-1]
        elif line.startswith('    Block counts: ['):
            total = sum(int(x) for x in line[line.index('[') + 1:line.index(']')].split(', ') if x)
            res[name] = res.get(name, 0) + total
    return res


def symbol_order(tool, root, profile):
    """An lld --symbol-ordering-file: what the small documents run, hottest first, so that they touch few pages, then what the others run."""
    data = os.path.join(root, 'data')
    small = set()
    for key in ('ladder_04-latex-hello', 'ladder_00-plain-hello'):
        small |= {n for n, c in function_counts(tool, os.path.join(data, key + '.profdata')).items() if c > 0}
    hot = function_counts(tool, profile)
    first = sorted((n for n in hot if n in small), key=lambda n: -hot[n])
    rest = sorted((n for n in hot if n not in small and hot[n] > 0), key=lambda n: -hot[n])
    text = '\n'.join(first + rest) + '\n'
    path = os.path.join(root, 'merged', hashlib.sha256(text.encode()).hexdigest()[:16] + '.order')
    with open(path, 'w') as f:
        f.write(text)
    say(f'{path}: {len(first)} functions of the small documents, {len(rest)} others')
    return path


# ------------------------------------------------------------------------------------------------ the table
def bench(engines, docs, args, json_path):
    cmd = [sys.executable, '-I', os.path.join(REPO, 'tools', 'prof', 'bench.py'), '--no-oracle', '--docs', ','.join(docs), '-n', str(args.n), '--counters', str(args.counters), '--format-build', '1', '--json', json_path]
    for label, path in engines:
        cmd += ['--engine', f'{label}={path}']
    run(cmd, stdout=subprocess.DEVNULL)
    with open(json_path) as f:
        return json.load(f)


def table(results, labels):
    base = labels[0]
    print(f'\n{"doc":<14}{"build":<14}{"Ginstr":>9}{"ratio":>7}{"Gcycles":>9}{"ratio":>7}{"IPC":>6}{"Mbr-miss":>10}{"faults":>8}{"CPU ms":>10}{"ratio":>7}')
    for rec in results['docs']:
        b = rec['engines'][base]
        for label in labels:
            s = rec['engines'][label]
            c = s['counters']
            ci, cc = c.get('instructions', 0), c.get('cycles', 0)
            bi, bc = b['counters'].get('instructions', 1), b['counters'].get('cycles', 1)
            print(f'{rec["name"]:<14}{label:<14}{ci / 1e9:>9.3f}{ci / bi:>7.3f}{cc / 1e9:>9.3f}{cc / bc:>7.3f}{ci / cc if cc else 0:>6.2f}{c.get("branch-misses", 0) / 1e6:>10.2f}'
                  f'{s["minflt"]:>8.0f}{s["cpu_ms"]["median"]:>10.1f}{s["cpu_ms"]["median"] / b["cpu_ms"]["median"]:>7.3f}')
    b = results['format_build'][base]
    for label in labels:
        s = results['format_build'][label]
        c = s['counters']
        ci, cc = c.get('instructions', 0), c.get('cycles', 0)
        bi, bc = b['counters'].get('instructions', 1), b['counters'].get('cycles', 1)
        print(f'{"pdflatex.fmt":<14}{label:<14}{ci / 1e9:>9.3f}{ci / bi:>7.3f}{cc / 1e9:>9.3f}{cc / bc:>7.3f}{ci / cc if cc else 0:>6.2f}{c.get("branch-misses", 0) / 1e6:>10.2f}'
              f'{s["minflt"]:>8.0f}{s["cpu_ms"]["median"]:>10.1f}{s["cpu_ms"]["median"] / b["cpu_ms"]["median"]:>7.3f}')


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('-j', '--jobs', type=int, default=6)
    ap.add_argument('--train', default='auto', help='auto, public, all, or names and manifest ids separated by commas')
    ap.add_argument('--use', default='', help='of what was trained, merge only these: public, or names, manifest ids and fmt-pdflatex, fmt-pdftex')
    ap.add_argument('--weights', choices=['equal', 'sqrt', 'raw'], default='equal')
    ap.add_argument('--name', default='final', help='the directory of this build, under pgo-build')
    ap.add_argument('--flags', default='', help='more rustflags for this build')
    ap.add_argument('--order', action='store_true', help='a symbol order from the profile, in place of the one lld makes from the call graph: docs/BUILD.md')
    ap.add_argument('--reuse-training', action='store_true', help='keep data/, from an earlier run on the same code')
    ap.add_argument('--no-bench', action='store_true')
    ap.add_argument('--no-plain', action='store_true', help='in the table, no build without a profile')
    ap.add_argument('--bench-docs', default=','.join(BENCH_DOCS))
    ap.add_argument('--compare', action='append', default=[], metavar='NAME=PATH', help='another binary for the table')
    ap.add_argument('-n', type=int, default=3)
    ap.add_argument('--counters', type=int, default=3)
    args = ap.parse_args()
    t0 = time.monotonic()
    root = os.path.join(proflib.target_root(), 'pgo-build')
    tool = profdata_tool()
    os.makedirs(root, exist_ok=True)
    if not args.reuse_training:
        docs, missing = training_docs(args.train)
        say(f'training on {len(docs)} documents and {len(FORMATS)} format builds; not found, so left out: {", ".join(missing) or "nothing"}')
        default_dir = os.path.join(root, 'gen-default')
        shutil.rmtree(default_dir, ignore_errors=True)
        gen = cargo_build(os.path.join(root, 'gen'), [f'-Cprofile-generate={default_dir}'], args.jobs)
        say(f'instrumented build done, {time.monotonic() - t0:.0f} s in')
        train(gen, docs, args, tool, root)
        say(f'training done, {time.monotonic() - t0:.0f} s in')
    profile = merged_profile(args, tool, root)
    flags = [f'-Cprofile-use={profile}'] + USE_FLAGS + args.flags.split()
    if args.order:
        flags += [f'-Clink-arg=-Wl,--symbol-ordering-file={symbol_order(tool, root, profile)}', '-Clink-arg=-Wl,--no-warn-symbol-ordering']
    binary = cargo_build(os.path.join(root, args.name), flags, args.jobs)
    out = os.path.join(root, args.name, 'rstex')
    shutil.copy2(binary, out)
    say(f'{out} built, sha {proflib.sha_file(out)[:12]}, {os.path.getsize(out)} bytes, {time.monotonic() - t0:.0f} s in')
    if args.no_bench:
        return
    engines = []
    if not args.no_plain:
        engines.append(('plain', cargo_build(os.path.join(root, 'plain'), [], args.jobs)))
    for spec in args.compare:
        label, _, path = spec.partition('=')
        engines.append((label, path))
    engines.append((args.name, out))
    entries, _, _ = proflib.corpus().load_manifests(None)
    ids = {e['id'] for e in entries}
    docs = [d for d in args.bench_docs.split(',') if proflib.NAMED.get(d, d) in ids]
    results = bench(engines, docs, args, os.path.join(root, args.name + '-bench.json'))
    table(results, [label for label, _ in engines])
    say(f'{out}: all done in {time.monotonic() - t0:.0f} s')


if __name__ == '__main__':
    main()
