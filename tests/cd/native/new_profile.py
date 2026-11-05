#!/usr/bin/env -S python3 -I
r"""One command for a TeX tree the native paths do not know: a table entry, and a verdict on whether it may stay.  DESIGN section 10, "Versions"; docs/BUILD.md says when to run it.

    python3 -I tests/cd/native/new_profile.py [--name ID] [-j N] [--work DIR] [--only STEP]... [--probe BIN] [--no-build] [--keep-work] [--keep-table]

The tree is the one on PATH: its kpsewhich finds the files and its pdflatex is the oracle.  Nothing of any other TeX may be on PATH.  cargo is wanted there too, unless --no-build.

  table      a build with the feature commute-probe runs tests/cd/native/gen_table.py and tests/native_font/gen_table.py on this tree: the profile goes into crates/engine/src/native_table.rs beside the others,
             and into native_font_table.in if the macros there are another version.  Then a plain build, with those tables, called rstex.
  armed      `rstex --check-tree` says tikz-cd and \pickup@font are armed on this tree, by this profile.
  geometry   tests/corpus/cd/compare_native.py geometry: the public diagrams of tests/corpus/manifests/cd.json against real PGF on this tree: dimensions, text, pixels at 400 dpi.  What the oracle cannot compile here, for a
             package this tree lacks, is out of the denominator and counted.
  streams    the same diagrams that took the path: the content stream, byte for byte.
  negative   compare_native.py check-negative with this tree's pdflatex: a shift of 0.05 bp fails, so that a pass means something.
  cases      tests/cd/native/run.py: its cases one requirement at a time, and what a document can see after a diagram.
  adversarial tests/cd/native/adversarial/run.py: some 8500 documents written to break the path, against this tree's oracle, and cost.py.
  warm       tests/corpus/snapshot/warm_equals_cold.py --path-on on the ladder: cold, written, restored, sealed and resumed runs equal byte for byte.
  documents  compare_native.py documents on the public arXiv papers that use tikz-cd: the whole log, the pages, each content stream.
  font       tests/native_font/run.py, its cases and the corpus: the native \pickup@font against the real macros.

The last line says VERDICT pass or VERDICT DECLINE, and the exit status agrees.  On DECLINE the tables go back to what they were, unless --keep-table: a profile that fails any of this is not to be used, and that is the whole of what a failure
costs, since a tree in no profile only runs slower.  Fix what the step says, in the model under `commute::BpRule` and the like, and run it again; a macro that cannot be proved stays out.  Only ids and counts of public documents are printed.
All of it stays under --work, by default target/new-profile.  What it adds to the tree of sources is the two table files, for a commit.
"""
import argparse
import os
import re
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
TABLES = [os.path.join(REPO, 'crates', 'engine', 'src', n) for n in ('native_table.rs', 'native_font_table.in')]
STEPS = ('table', 'armed', 'geometry', 'streams', 'negative', 'cases', 'adversarial', 'warm', 'documents', 'font')
TOOLS = {'compare': os.path.join(REPO, 'tests', 'corpus', 'cd', 'compare_native.py'), 'cases': os.path.join(HERE, 'run.py'), 'adversarial': os.path.join(HERE, 'adversarial', 'run.py'), 'cost': os.path.join(HERE, 'adversarial', 'cost.py'),
         'warm': os.path.join(REPO, 'tests', 'corpus', 'snapshot', 'warm_equals_cold.py'), 'font': os.path.join(REPO, 'tests', 'native_font', 'run.py'), 'table': os.path.join(HERE, 'gen_table.py'), 'font-table': os.path.join(REPO, 'tests', 'native_font', 'gen_table.py')}


def step_out(args, name):
    return os.path.join(args.work, name + '.out')


def run_step(args, name, argv, env=None):
    """Runs one program of a step, with its output in a file of the work directory.  Its status."""
    with open(step_out(args, name), 'wb') as f:
        return subprocess.run(argv, cwd=REPO, env=env or os.environ, stdin=subprocess.DEVNULL, stdout=f, stderr=subprocess.STDOUT).returncode


def tail(args, name, n=1, width=200):
    with open(step_out(args, name), errors='replace') as f:
        lines = f.read().rstrip('\n').split('\n')
    return [line[:width] for line in lines[-n:]]


def head(args, name, n=1, width=200):
    with open(step_out(args, name), errors='replace') as f:
        return [line[:width] for line in f.read().split('\n')[:n]]


def the_tree():
    latex = shutil.which('pdflatex')
    if not latex or not shutil.which('kpsewhich'):
        sys.exit('no pdflatex and kpsewhich on PATH: this judges the tree they belong to')
    ltx = subprocess.run(['kpsewhich', 'latex.ltx'], capture_output=True, text=True).stdout.strip()
    version = subprocess.run([latex, '--version'], capture_output=True, text=True).stdout.split('\n')[0]
    print(f'the tree: {ltx}\nthe oracle: {latex}, {version}', flush=True)
    return os.path.dirname(os.path.realpath(latex))


def build(args, features, target):
    cargo = shutil.which('cargo')
    if not cargo:
        sys.exit('no cargo on PATH: add its directory, or say --no-build and give what it would make under --work')
    argv = [cargo, 'build', '--release', '-p', 'rstex', f'-j{args.jobs}', '--target-dir', target] + features
    if run_step(args, 'build', argv):
        print('\n'.join(tail(args, 'build', 12)))
        sys.exit('the build failed')
    return os.path.join(target, 'release', 'rstex')


def wrapper(args, engine):
    """As tests/gate.sh leaves it: the program's name is rstex, and it finds its own formats, built on this tree."""
    root = os.path.join(args.work, 'engine')
    for d in ('fmt', 'fmt-pdftex', 'fmt-latex'):
        os.makedirs(os.path.join(root, d), exist_ok=True)
    binary = os.path.join(root, 'rstex-bin')
    shutil.copyfile(engine, binary)
    os.chmod(binary, 0o755)
    os.makedirs(os.path.join(root, 'bin'), exist_ok=True)
    plain = os.path.join(root, 'bin', 'rstex')
    shutil.copyfile(engine, plain)
    os.chmod(plain, 0o755)
    for d, job, ini in (('fmt', 'pdflatex', 'pdflatex.ini'), ('fmt-pdftex', 'pdftex', 'pdfetex.ini'), ('fmt-latex', 'latex', 'latex.ini')):
        subprocess.run([binary, '-ini', f'-jobname={job}', f'-progname={job}', '-translate-file=cp227.tcx', '*' + ini], cwd=os.path.join(root, d), stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    w = os.path.join(root, 'rstex')
    with open(w, 'w') as f:
        f.write(f'''#!/bin/sh
# Finds this engine's own formats.
for x in "$@"; do
  case "$x" in
    -fmt=pdflatex) set -- "$@" -fmt={root}/fmt/pdflatex -progname=pdflatex ;;
    -fmt=pdftex) set -- "$@" -fmt={root}/fmt-pdftex/pdftex -progname=pdftex ;;
    -fmt=latex) set -- "$@" -fmt={root}/fmt-latex/latex -progname=latex ;;
    *) set -- "$@" "$x" ;;
  esac
  shift
done
exec {binary} "$@"
''')
    os.chmod(w, 0o755)
    return w, plain


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--name', help='what to call the profile, by default from the versions of PGF and LaTeX')
    ap.add_argument('-j', '--jobs', type=int, default=4)
    ap.add_argument('--work', default=os.path.join(REPO, 'target', 'new-profile'))
    ap.add_argument('--only', action='append', choices=STEPS, help='only these; the tables and the build from a run before stay')
    ap.add_argument('--probe', help='a build with the feature commute-probe that exists already')
    ap.add_argument('--no-build', action='store_true', help='judge what the last run built, with the tables as they stand')
    ap.add_argument('--keep-work', action='store_true', help='leave all of --work behind, the adversarial step\'s oracle too')
    ap.add_argument('--keep-table', action='store_true', help='leave the tables as this tree made them, whatever the verdict')
    args = ap.parse_args()
    args.work = os.path.abspath(args.work)
    os.makedirs(args.work, exist_ok=True)
    steps = args.only or STEPS
    oracle_dir = the_tree()
    saved = {t: open(t, 'rb').read() for t in TABLES}

    env = dict(os.environ, RSTEX_ORACLE_DIR=oracle_dir, RSTEX_CORPUS_TARGET=os.path.join(args.work, 'corpus'))
    py = sys.executable
    rows, failed = [], []

    def verdict(step, ok, text):
        rows.append((step, 'pass' if ok else 'FAIL', text))
        print(f"{step:<12} {'pass' if ok else 'FAIL'}  {text}", flush=True)
        if not ok:
            failed.append(step)

    profile = None
    built = os.path.join(args.work, 'build', 'release', 'rstex')
    if 'table' in steps and not args.no_build:
        probe = args.probe or build(args, ['--features', 'rstex-engine/commute-probe'], os.path.join(args.work, 'probe'))
        probe = os.path.abspath(probe)
        generate = [py, '-I', TOOLS['table'], '--engine', probe] + (['--name', args.name] if args.name else [])
        ok = run_step(args, 'table', generate) == 0
        if ok:
            profile = next((m.group(1) for m in (re.match(r'PROFILE (.*)', line) for line in tail(args, 'table', 3)) if m), None)
        ok2 = run_step(args, 'font-table', [py, '-I', TOOLS['font-table'], '--engine', probe]) == 0
        verdict('table', ok and ok2, f"profile {profile}; {tail(args, 'font-table')[0]}" if ok and ok2 else ' / '.join(tail(args, 'table', 2) + tail(args, 'font-table', 1)))
        if not (ok and ok2):
            return finish(args, rows, failed, saved)
        built = build(args, [], os.path.join(args.work, 'build'))
    if not os.path.exists(built):
        sys.exit(f'no {built}: run the step table first')
    w, plain = wrapper(args, built)
    os.makedirs(env['RSTEX_CORPUS_TARGET'], exist_ok=True)

    def on(step):
        return step in steps

    if on('armed'):
        code = run_step(args, 'armed', [plain, '--check-tree'], dict(env, XDG_CACHE_HOME=os.path.join(args.work, 'cache')))
        with open(step_out(args, 'armed'), errors='replace') as f:
            said = f.read()
        armed = re.search(r'^path=tikz-cd state=armed profile=(\S+)', said, re.M)
        font = re.search(r'^path=pickup@font state=armed', said, re.M)
        ok = bool(armed and font) and (not profile or armed.group(1) == profile)
        verdict('armed', ok, f'status {code}: ' + ' '.join(line for line in said.split('\n') if line.startswith('path='))[:200])
    # The public diagrams only: nothing private exists on any tree but one, and nothing of it would be for this report.
    public = ['--filter', '^cd/']
    if on('geometry'):
        code = run_step(args, 'geometry', [py, '-I', TOOLS['compare'], 'geometry', '--engine', w, '--engine-arg=--commute-report', '-j', str(args.jobs), '--show', '40'] + public, env)
        verdict('geometry', code == 0, head(args, 'geometry')[0])
    if on('streams'):
        code = run_step(args, 'streams', [py, '-I', TOOLS['compare'], 'streams', '--engine', w, '-j', str(args.jobs)] + public, env)
        verdict('streams', code == 0, head(args, 'streams')[0])
    if on('negative'):
        code = run_step(args, 'negative', [py, '-I', TOOLS['compare'], 'check-negative', '-j', str(args.jobs)], env)
        verdict('negative', code == 0, tail(args, 'negative')[0])
    if on('cases'):
        code = run_step(args, 'cases', [py, '-I', TOOLS['cases'], '--engine', w, '-j', str(args.jobs), '--work', os.path.join(args.work, 'cases')], env)
        verdict('cases', code == 0, tail(args, 'cases')[0])
        shutil.rmtree(os.path.join(args.work, 'cases'), ignore_errors=True)
    if on('adversarial'):
        code = run_step(args, 'adversarial', [py, '-I', TOOLS['adversarial'], '--engine', w, '-j', str(args.jobs), '--work', os.path.join(args.work, 'adversarial'), '--oracle-cache', os.path.join(args.work, 'adversarial-oracle')], env)
        cost = run_step(args, 'cost', [py, '-I', TOOLS['cost'], '--engine', w, '--work', os.path.join(args.work, 'cost')], env)
        verdict('adversarial', code == 0 and cost == 0, tail(args, 'adversarial')[0] + '; ' + tail(args, 'cost')[0])
    if on('warm'):
        code = run_step(args, 'warm', [py, '-I', TOOLS['warm'], '--engine', w, '--path-on', '--filter', '^ladder/', '-j', str(args.jobs), '--run-id', f'new-profile-warm-{os.getpid()}'], env)
        verdict('warm', code == 0, tail(args, 'warm')[0].split(';')[0])
    if on('documents'):
        code = run_step(args, 'documents', [py, '-I', TOOLS['compare'], 'documents', '--engine', w, '-j', str(args.jobs), '--filter', '^arxiv/'], env)
        verdict('documents', code == 0, tail(args, 'documents')[0])
    if on('font'):
        one = run_step(args, 'font', [py, '-I', TOOLS['font'], '--engine', w, '-j', str(args.jobs), '--work', os.path.join(args.work, 'font-cases')], env)
        two = run_step(args, 'font-corpus', [py, '-I', TOOLS['font'], '--engine', w, '-j', str(args.jobs), '--corpus', r'^(ladder/|manuals/microtype/|arxiv/math\.AG/)'], env)
        verdict('font', one == 0 and two == 0, tail(args, 'font')[0] + '; ' + tail(args, 'font-corpus')[0])
    return finish(args, rows, failed, saved)


def finish(args, rows, failed, saved):
    print('---')
    for step, state, text in rows:
        print(f'{step:<12} {state:<5} {text}')
    if failed:
        if not args.keep_table:
            for t, data in saved.items():
                with open(t, 'wb') as f:
                    f.write(data)
        print(f"VERDICT DECLINE: {' '.join(failed)}.  What each said is in {args.work}/<step>.out" + ('; the tables stay as this tree made them' if args.keep_table else '; the tables are back as they were'))
        return 1
    print('VERDICT pass: the tables hold this tree, and may be committed.')
    if not args.keep_work:
        for d in ('adversarial', 'adversarial-oracle', 'cases', 'cost', 'font-cases', 'probe'):
            shutil.rmtree(os.path.join(args.work, d), ignore_errors=True)
    return 0


if __name__ == '__main__':
    sys.exit(main())
