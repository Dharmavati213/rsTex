#!/usr/bin/env -S python3 -I
"""rstex_prof.py: where a prof build of rstex spends a document's time, and what changed between two builds.

    rstex_prof.py --build [--no-frame-pointers]                 the prof profile of Cargo.toml, with --features prof
    rstex_prof.py DOC... --binary B [--name NAME] [-n N]        run, symbolise, print; results under target/
                  [--source both|time|instructions] [--hz H] [--period P] [--cpu K] [--cold-tree]
                  [--top N] [--lines K] [--callers K] [--inline-detail]
    rstex_prof.py --report DIR [--top N] ...                    print a result set again
    rstex_prof.py --compare DIR_A DIR_B [--top N]               per-function deltas, in samples and instructions

DOC is a short name (hello, plain-hello, tikzcd-many, texweb, sga3, main, LMB, summary, SGA1-VI), a set (baseline, quick, user, ladder), or a
manifest id; tools/prof/proflib.py has the list.

How a document is sampled.  `--source both` runs it N times under each of two samplers in the binary, crates/engine/src/prof.rs:
  time          setitimer(ITIMER_PROF), default 2000 Hz.  The kernel fires it at most once per tick, so 1000 Hz on this machine.  It counts user and
                system time, and a sample during a system call or page fault lands where the program made it, so this shows what the kernel costs.
                Each sample stands for the CPU time since the one before, since a long system call is one sample however many ticks it spans.  The
                time columns are that time; what comes before the sampler starts and after it stops is in the phase table only.
  instructions  a signal every P user-mode instructions, by default chosen for 30,000 samples a run.  User mode only, and the count does not depend on
                the load of the machine, so this is the column to compare builds by.  Skid moves a sample by a few instructions, not out of a function at this level.

Tables.  A sample's address goes through `addr2line -i`, which gives the chain of inlined calls down to the innermost.  `by function` charges the sample
to the innermost frame that is ported or crate code, looking through the accessors of access.rs and raw.rs and the standard library, because
`self.mem[p]` is the work of the procedure it is written in.  The accessors' own share shows in the `innermost` column of the module table and at
the source lines.  `by symbol` is the function as linked, from nm, with its total including callees from the frame pointers.  Frame pointers come from
-C force-frame-pointers=yes in a prof build; without it, and in libc, which has none, a caller view stops short.
"""
import argparse
import bisect
import collections
import json
import os
import re
import shutil
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import proflib  # noqa: E402

# An instruction sample every this many, unless --period says: enough samples per run, and few enough signals not to disturb it.
SAMPLES_PER_RUN = 30_000
MIN_PERIOD = 200_000
# Where a sample is charged past: code that is not the work itself.
PASS_THROUGH = ('crates/engine/src/access.rs', 'crates/engine/src/raw.rs')
SOURCES = {'time': None, 'instructions': 'instructions'}


# ------------------------------------------------------------------------------------------------ build
def build(args):
    env = dict(os.environ)
    flags = ['-C', 'target-feature=+crt-static', '-C', 'relocation-model=static']
    if not args.no_frame_pointers:
        flags += ['-C', 'force-frame-pointers=yes']
    env['RUSTFLAGS'] = ' '.join(flags)
    cmd = ['cargo', 'build', '--profile', 'prof', '--features', 'prof', '-p', 'rstex']
    if args.jobs:
        cmd += ['-j', str(args.jobs)]
    r = subprocess.run(cmd, cwd=proflib.REPO, env=env, check=False)
    if r.returncode:
        sys.exit(r.returncode)
    print(os.path.join(proflib.target_root(), 'prof', 'rstex'))


# ------------------------------------------------------------------------------------------------ runs
def run_doc(doc, engine, dest, args):
    os.makedirs(dest, exist_ok=True)
    doc.prepare()
    if args.cold_tree:
        engine.cache_home = os.path.join(dest, 'empty-cache')
    # The same build with the hooks dormant, counted from outside: what profiling itself adds is the difference, and this sets the sampling period.
    work = proflib.Workdir(doc)
    argv, cwd, env = engine.command(doc, work.out)
    outside = {k: v['value'] for k, v in proflib.run_counted(argv, cwd, env)['counters'].items()}
    work.remove()
    period = args.period or max(MIN_PERIOD, outside.get('instructions', 0) // SAMPLES_PER_RUN)
    meta = {'doc': doc.id, 'name': doc.name, 'binary': engine.path, 'sha': engine.sha, 'runs': [], 'period': period, 'hz': args.hz, 'outside_counters': outside}
    kinds = ['time', 'instructions'] if args.source == 'both' else [args.source]
    for kind in kinds:
        for i in range(args.n):
            if args.cold_tree:
                shutil.rmtree(engine.cache_home, ignore_errors=True)
                os.makedirs(engine.cache_home)
            work = proflib.Workdir(doc)
            argv, cwd, env = engine.command(doc, work.out)
            raw = os.path.join(dest, f'{kind}-{i}.raw')
            text = os.path.join(dest, f'{kind}-{i}.txt')
            env = dict(env, RSTEX_PROF_OUT=raw, RSTEX_PROF_COUNTERS=text, RSTEX_PROF_HZ=str(args.hz))
            if kind == 'instructions':
                env['RSTEX_PROF_EVENT'] = f'instructions:{period}'
            r = proflib.run_timed(argv, cwd, env, stamp_t0=True)
            has_pdf = os.path.exists(os.path.join(work.out, doc.entry['jobname'] + '.pdf'))
            work.remove()
            if not os.path.exists(text):
                proflib.die(f'{doc.id}: no counters from the run, rc {r["rc"]}: is {engine.path} a prof build?')
            meta['runs'].append({'kind': kind, 'i': i, 'rc': r['rc'], 'pdf': has_pdf, 'wall_ms': r['wall_ms'], 'cpu_ms': r['cpu_ms'], 'user_ms': r['user_ms'],
                                 'sys_ms': r['sys_ms'], 'minflt': r['minflt'], 'majflt': r['majflt'], 't0_ns': r['t0_ns'], 'reap_ns': r['reap_ns'],
                                 'load': proflib.load_now()})
    with open(os.path.join(dest, 'meta.json'), 'w') as f:
        json.dump(meta, f)
    return meta


# ------------------------------------------------------------------------------------------------ symbols
class Symbols:
    """nm for the function as linked, addr2line -i for what was inlined there."""

    def __init__(self, binary):
        self.binary = binary
        out = subprocess.run(['nm', '-n', '--defined-only', '-C', binary], capture_output=True, text=True, check=True).stdout
        self.addrs, self.names = [], []
        for line in out.splitlines():
            p = line.split(' ', 2)
            if len(p) == 3 and p[1] in 'tTwWiI':
                self.addrs.append(int(p[0], 16))
                self.names.append(clean_name(p[2]))
        self.text_end = None
        out = subprocess.run(['nm', '-n', '--defined-only', binary], capture_output=True, text=True, check=True).stdout
        for line in out.splitlines():
            p = line.split(' ', 2)
            if len(p) == 3 and p[2] in ('_etext', 'etext', '__etext'):
                self.text_end = int(p[0], 16)
        self.chains = {}

    def symbol(self, addr):
        i = bisect.bisect_right(self.addrs, addr) - 1
        if i < 0 or (self.text_end is not None and addr >= self.text_end):
            return None
        return self.names[i]

    def resolve(self, addrs):
        """Fills the chain, innermost first, of each address: [(function, file, line)]."""
        todo = sorted({a for a in addrs if a not in self.chains})
        for k in range(0, len(todo), 2000):
            part = todo[k:k + 2000]
            r = subprocess.run(['addr2line', '-i', '-f', '-a', '-C', '-e', self.binary], input='\n'.join(hex(a) for a in part) + '\n',
                               capture_output=True, text=True, check=True)
            chain, addr = None, None
            lines = r.stdout.split('\n')
            j = 0
            while j < len(lines):
                line = lines[j]
                if line.startswith('0x'):
                    addr = int(line, 16)
                    chain = self.chains[addr] = []
                    j += 1
                    continue
                if not line or j + 1 >= len(lines):
                    j += 1
                    continue
                where = lines[j + 1].split(' (discriminator')[0]
                file, _, no = where.rpartition(':')
                chain.append((clean_name(line), normal_path(file), int(no) if no.isdigit() else 0))
                j += 2
            # The first function name addr2line gives is the linked symbol, from the symbol table, even when the line under it is in code
            # inlined there: only the later names come from the inlining records, each the function holding that frame's line.  So with
            # inlining the innermost frame is named from its source.
            for a in part:
                c = self.chains.get(a)
                if c and len(c) > 1 and c[0][1]:
                    if c[0][2] == 0:        # line 0, code the compiler made: it belongs to the frame that holds it
                        c[0] = (c[1][0], c[0][1], 0)
                    else:
                        c[0] = (enclosing_fn(c[0][1], c[0][2]) or library_name(c[0][1]), c[0][1], c[0][2])

    def chain(self, addr):
        return self.chains.get(addr) or []


_source_lines = {}


def read_source(file):
    """A source file's lines, or None: ours, a registry crate's, or the standard library's if rust-src is installed."""
    if file in _source_lines:
        return _source_lines[file]
    path = file if os.path.isabs(file) else os.path.join(proflib.REPO, file)
    m = re.match(r'/rustc/[0-9a-f]+/(library/.*)', file)
    if m:
        r = subprocess.run(['rustc', '--print', 'sysroot'], capture_output=True, text=True, check=False)
        path = os.path.join(r.stdout.strip(), 'lib', 'rustlib', 'src', 'rust', m.group(1))
    try:
        with open(path, encoding='utf-8', errors='replace') as f:
            _source_lines[file] = f.readlines()
    except OSError:
        _source_lines[file] = None
    return _source_lines[file]


def library_name(file):
    """A frame with no source to look at: the standard library, without rust-src.  Its path says what it is."""
    m = re.search(r'/library/(.*)\.rs$', file)
    return 'std: ' + m.group(1).replace('/src/', '/') if m else os.path.basename(file)


FN_LINE = re.compile(r'\bfn\s+([A-Za-z_][A-Za-z_0-9]*)')


def enclosing_fn(file, line):
    """The function a source line is in, as far as a look back at the text can say."""
    src = read_source(file)
    if not src:
        return None
    for i in range(min(line, len(src)) - 1, -1, -1):
        m = FN_LINE.search(src[i])
        if m:
            return m.group(1)
    return None


def clean_name(name):
    """nm and addr2line spell a method differently, `<rstex_engine::state::Tex>::get_next`, `rstex_engine::getnext::<impl rstex_engine::state::Tex>::get_next`
    or just `get_next`, and the name is needed to say one thing."""
    name = re.sub(r'::h[0-9a-f]{16}$', '', name.strip())
    name = re.sub(r'<impl [^>]*Tex[^>]*>::', '', name)
    name = name.replace('<rstex_engine::state::Tex>::', '').replace('<rstex_kpse::Kpse>::', '')
    name = re.sub(r'^rstex_engine::[a-z_0-9]+::', '', name)
    return name.replace('{{closure}}', '{closure}')


def normal_path(path):
    """A path that does not depend on where the tree was built.  glibc's static objects have no line tables, and addr2line gives a compilation unit's name."""
    if not path or path.startswith('?') or not path.endswith(('.rs', '.c', '.h', '.S')):
        return ''
    if path.startswith('/rustc/') or '/.cargo/registry/' in path:
        return path
    i = path.find('/crates/')
    if i >= 0:
        return path[i + 1:]
    if path.startswith('crates/'):
        return path
    return path


def module_of(path, symbol=None):
    if not path:
        if symbol and (symbol.startswith('__') or symbol.islower()):
            return 'libc'
        return 'unknown'
    m = re.match(r'crates/engine/src/platform/', path)
    if m:
        return 'platform'
    m = re.match(r'crates/engine/src/([a-z0-9_]+)\.rs', path)
    if m:
        return m.group(1) + '.rs'
    m = re.match(r'crates/([a-z0-9_-]+)/', path)
    if m:
        return 'crate ' + m.group(1)
    if '/rustc/' in path or path.startswith('library/') or '/library/' in path:
        return 'rust std'
    m = re.search(r'/registry/src/[^/]+/([A-Za-z0-9_-]+?)-\d', path)
    if m:
        return 'crate ' + m.group(1)
    return 'other'


def is_pass_through(path):
    return path in PASS_THROUGH or '/rustc/' in path or path.startswith('library/')


def attribute(chain, symbol):
    """(function, file, line) the sample is charged to, and the innermost frame."""
    if not chain or not chain[0][1]:
        return (symbol or '??', '', 0), (symbol or '??', '', 0)
    for fr in chain:
        if fr[1] and not is_pass_through(fr[1]):
            return fr, chain[0]
    return chain[0], chain[0]


# ------------------------------------------------------------------------------------------------ analysis
def analyse(dest, callers_wanted):
    """Reads a result directory and writes summary.json, the tables' data; returns it."""
    with open(os.path.join(dest, 'meta.json')) as f:
        meta = json.load(f)
    syms = Symbols(meta['binary'])
    per_kind = {}
    for run in meta['runs']:
        raw = os.path.join(dest, f'{run["kind"]}-{run["i"]}.raw')
        info, pcs, weights, starts, frames = proflib.parse_samples(raw)
        run['sampler'] = info
        base = info['base']
        k = per_kind.setdefault(run['kind'], {'leaf': collections.Counter(), 'incl': collections.Counter(), 'callers': collections.defaultdict(collections.Counter), 'samples': 0, 'with_frames': 0, 'dropped': 0})
        k['samples'] += info['samples']
        k['dropped'] += info['dropped']
        for s, pc in enumerate(pcs):
            a = pc - base
            # A time sample counts as the microseconds of CPU since the one before, an instruction sample as one sample.
            w = weights[s] if run['kind'] == 'time' else 1
            k['leaf'][a] += w
            sym = syms.symbol(a)
            on_stack = {sym}
            lo, hi = starts[s], starts[s + 1]
            if hi > lo:
                k['with_frames'] += 1
                first = frames[lo] - base - 1
                k['callers'][sym][first] += w
                for j in range(lo, hi):
                    on_stack.add(syms.symbol(frames[j] - base - 1))
            for x in on_stack:
                if x:
                    k['incl'][x] += w
    all_addrs = set()
    for k in per_kind.values():
        all_addrs.update(k['leaf'])
        for c in k['callers'].values():
            all_addrs.update(a for a, _ in c.most_common(30))
    syms.resolve(all_addrs)

    funcs = collections.defaultdict(lambda: {'time': 0, 'instructions': 0, 'file': ''})
    modules = collections.defaultdict(lambda: {'time': 0, 'instructions': 0, 'time_inner': 0, 'instructions_inner': 0})
    symbols_flat = collections.defaultdict(lambda: {'time': 0, 'instructions': 0, 'time_incl': 0, 'instructions_incl': 0})
    lines = collections.defaultdict(lambda: collections.defaultdict(lambda: {'time': 0, 'instructions': 0, 'inner': collections.Counter()}))
    for kind, k in per_kind.items():
        for a, n in k['leaf'].items():
            sym = syms.symbol(a) or '[outside the binary: vdso]'
            fr, inner = attribute(syms.chain(a), sym)
            name, file, line = fr
            f = funcs[f'{name}\t{file}']
            f['file'] = file
            f[kind] += n
            modules[module_of(file, name)][kind] += n
            modules[module_of(inner[1], inner[0])][kind + '_inner'] += n
            symbols_flat[sym][kind] += n
            l = lines[f'{name}\t{file}'][line]
            l[kind] += n
            if inner != fr:
                l['inner'][f'{os.path.basename(inner[1])}:{inner[2]} {inner[0]}'] += n
        for sym, n in k['incl'].items():
            symbols_flat[sym][kind + '_incl'] += n
    callers = {}
    for kind, k in per_kind.items():
        for sym, c in k['callers'].items():
            d = callers.setdefault(sym, collections.Counter())
            for a, n in c.items():
                fr, _ = attribute(syms.chain(a), syms.symbol(a))
                d[f'{fr[0]} ({os.path.basename(fr[1])}:{fr[2]})'] += n

    def runs_of(kind):
        return [r for r in meta['runs'] if r['kind'] == kind]
    n_time = len(runs_of('time'))
    n_instr = len(runs_of('instructions'))
    counters_runs, phase_runs, scope_runs = [], [], []
    for run in meta['runs']:
        c = proflib.parse_counters(os.path.join(dest, f'{run["kind"]}-{run["i"]}.txt'))
        run['parsed'] = c
    use = runs_of('time') or meta['runs']
    for run in use:
        counters_runs.append(run['parsed']['counters'])
        phase_runs.append(run)
        scope_runs.append(run['parsed']['scopes'])
    counters = {name: proflib.median(r[name]['value'] for r in counters_runs if name in r) for name in {n for r in counters_runs for n in r}}
    outside = meta['outside_counters']
    multiplexed = any(v['running_ns'] < v['enabled_ns'] for r in counters_runs for v in r.values())
    names_in_order = []
    for run in phase_runs:
        for p in run['parsed']['phases']:
            if p['name'] not in names_in_order:
                names_in_order.append(p['name'])
    order = {}
    for name in names_in_order:
        order[name] = proflib.median(p['t_ns'] for run in phase_runs for p in run['parsed']['phases'] if p['name'] == name)
    names_in_order.sort(key=lambda n: order[n])
    # Each phase's own time, run by run.  Its low percentile is the phase on a good run: on a busy machine a run that found its sibling hyperthread idle.
    deltas = collections.defaultdict(list)
    for run in phase_runs:
        at = {p['name']: p for p in run['parsed']['phases']}
        prev_t, prev_c = 0, 0
        for name in names_in_order:
            if name in at:
                deltas[name].append(((at[name]['t_ns'] - prev_t) / 1e6, (at[name]['cpu_ns'] - prev_c) / 1e6))
                prev_t, prev_c = at[name]['t_ns'], at[name]['cpu_ns']
    phases = []
    for name in names_in_order:
        ps = [p for run in phase_runs for p in run['parsed']['phases'] if p['name'] == name]
        phases.append({'name': name, 't_ms': proflib.median(p['t_ns'] for p in ps) / 1e6, 'cpu_ms': proflib.median(p['cpu_ns'] for p in ps) / 1e6,
                       'delta_p10_ms': proflib.percentile([d[0] for d in deltas[name]], 10),
                       'instructions': proflib.median(p['instructions'] for p in ps), 'cycles': proflib.median(p['cycles'] for p in ps),
                       'minflt': proflib.median(p['minflt'] for p in ps), 'majflt': proflib.median(p['majflt'] for p in ps)})
    reap = proflib.median((r['reap_ns'] - r['parsed']['clock'].get('t0_abs_ns', r['t0_ns'])) / 1e6 for r in phase_runs)
    finish = proflib.median((r['parsed']['clock'].get('finish_abs_ns', 0) - r['parsed']['clock'].get('t0_abs_ns', r['t0_ns'])) / 1e6 for r in phase_runs)
    scope_names = sorted({n for r in scope_runs for n in r})
    scopes = []
    for name in scope_names:
        xs = [r[name] for r in scope_runs if name in r]
        scopes.append({'name': name, 'count': proflib.median(x['count'] for x in xs), 'ms': proflib.median(x['total_ns'] for x in xs) / 1e6,
                       'instructions': proflib.median(x['instructions'] for x in xs), 'minflt': proflib.median(x['minflt'] for x in xs)})
    line_table = {}
    for key, ls in lines.items():
        line_table[key] = {str(no): {'time': v['time'], 'instructions': v['instructions'], 'inner': v['inner'].most_common(3)} for no, v in ls.items()}
    summary = {
        'doc': meta['doc'], 'name': meta['name'], 'binary': meta['binary'], 'sha': meta['sha'], 'period': meta['period'], 'hz': meta['hz'],
        'n_time': n_time, 'n_instructions': n_instr,
        'samples': {kind: k['samples'] for kind, k in per_kind.items()},
        'weight': {kind: sum(k['leaf'].values()) for kind, k in per_kind.items()},
        'with_frames': {kind: k['with_frames'] for kind, k in per_kind.items()},
        'dropped': {kind: k['dropped'] for kind, k in per_kind.items()},
        'wall_ms': proflib.median(r['wall_ms'] for r in use), 'cpu_ms': proflib.median(r['cpu_ms'] for r in use),
        'user_ms': proflib.median(r['user_ms'] for r in use), 'sys_ms': proflib.median(r['sys_ms'] for r in use),
        'minflt': proflib.median(r['minflt'] for r in use), 'majflt': proflib.median(r['majflt'] for r in use),
        'load': max(r['load'] for r in meta['runs']), 'pdf': all(r['pdf'] for r in meta['runs']), 'rcs': sorted({r['rc'] for r in meta['runs']}),
        'counters': counters, 'outside_counters': outside, 'multiplexed': multiplexed,
        'phases': phases, 'reap_ms': reap, 'finish_ms': finish, 'scopes': scopes,
        'functions': {k: dict(v) for k, v in funcs.items()},
        'modules': {k: dict(v) for k, v in modules.items()},
        'symbols': {k: dict(v) for k, v in symbols_flat.items()},
        'lines': line_table,
        'callers': {k: c.most_common(callers_wanted) for k, c in callers.items()},
    }
    with open(os.path.join(dest, 'summary.json'), 'w') as f:
        json.dump(summary, f)
    return summary


def load_summary(dest, callers_wanted=8):
    path = os.path.join(dest, 'summary.json')
    if os.path.exists(path):
        with open(path) as f:
            return json.load(f)
    return analyse(dest, callers_wanted)


# ------------------------------------------------------------------------------------------------ printing
def print_header(s):
    print(f'== {s["name"]} ({s["doc"]}): binary {s["binary"]}, sha {s["sha"][:12]}')
    print(f'   {s["n_time"]} time runs at {s["hz"]} Hz asked, {s["samples"].get("time", 0)} samples; {s["n_instructions"]} instruction runs every {s["period"]}, {s["samples"].get("instructions", 0)} samples;'
          f' dropped {sum(s["dropped"].values())}; exit status {s["rcs"]}; PDF written {s["pdf"]}; load up to {s["load"]:.1f}')
    frames = s['with_frames'].get('time', 0)
    if s['samples'].get('time'):
        print(f'   {100 * frames / s["samples"]["time"]:.0f}% of time samples have a frame-pointer chain')
    if s['n_time']:
        print(f'   the samples of a time run account for {s["weight"].get("time", 0) / s["n_time"] / 1000:.1f} ms of CPU each, from the sampler\'s start')
    print(f'   median of the time runs: wall {s["wall_ms"]:.1f} ms, user {s["user_ms"]:.1f} + sys {s["sys_ms"]:.1f} = {s["cpu_ms"]:.1f} ms, minflt {s["minflt"]}, majflt {s["majflt"]}')


def user_ghz(s):
    """User cycles per nanosecond in the phase with the most user cycles.  A phase's CPU time beyond its user cycles at that rate is the kernel's, faults and system calls."""
    best, prev = None, None
    for p in s['phases']:
        if prev is not None and p['cpu_ms'] > prev['cpu_ms']:
            d = (p['cycles'] - prev['cycles'], p['cpu_ms'] - prev['cpu_ms'])
            if best is None or d[0] > best[0]:
                best = d
        prev = p
    return best[0] / best[1] / 1e6 if best and best[0] else None


def print_phases(s):
    ghz = user_ghz(s)
    print(f'\nphases (median, ms from just before the spawn; delta is up to the row from the row before, p10 its 10th percentile over the runs; kernel = CPU beyond the user cycles at {ghz or 0:.2f} GHz, an estimate):')
    print(f'  {"phase":<28}{"t":>10}{"delta":>10}{"p10":>9}{"cpu":>9}{"kernel":>9}{"Minstr":>11}{"Mcycles":>10}{"IPC":>6}{"faults":>8}')
    print(f'  {"process start":<28}{0:>10.3f}')
    prev_t, prev_c, prev_i, prev_y, prev_f = 0, 0, 0, 0, 0
    for p in s['phases']:
        cpu, instr, cyc = p['cpu_ms'] - prev_c, p['instructions'] - prev_i, p['cycles'] - prev_y
        kernel = max(0, cpu - cyc / (ghz * 1e6)) if ghz else 0
        ipc = instr / cyc if cyc > 0 else 0
        print(f'  {p["name"]:<28}{p["t_ms"]:>10.3f}{p["t_ms"] - prev_t:>10.3f}{p.get("delta_p10_ms", 0):>9.3f}{cpu:>9.3f}{kernel:>9.3f}{instr / 1e6:>11.3f}{cyc / 1e6:>10.3f}{ipc:>6.2f}{p["minflt"] - prev_f:>8.0f}')
        prev_t, prev_c, prev_i, prev_y, prev_f = p['t_ms'], p['cpu_ms'], p['instructions'], p['cycles'], p['minflt']
    print(f'  {"(writing these results)":<28}{s["finish_ms"]:>10.3f}{s["finish_ms"] - prev_t:>10.3f}')
    print(f'  {"reaped: exit, teardown":<28}{s["reap_ms"]:>10.3f}{s["reap_ms"] - s["finish_ms"]:>10.3f}')
    if s['scopes']:
        print('\nscopes (median; inclusive, nested in the phases and in one another):')
        print(f'  {"scope":<34}{"count":>8}{"ms":>10}{"Minstr":>10}{"faults":>8}')
        for x in sorted(s['scopes'], key=lambda x: -x['ms']):
            counted = f'{x["instructions"] / 1e6:>10.3f}{x["minflt"]:>8.0f}' if x['instructions'] else ''  # crates/kpse keeps time only
            print(f'  {x["name"]:<34}{x["count"]:>8.0f}{x["ms"]:>10.3f}{counted}')


def print_counters(s):
    print('\ncounters, user mode (median of the time runs):')
    print(f'  {"":<16}{"in the binary":>18}{"outside, dormant":>18}')
    for name, _, _ in proflib.EVENTS:
        print(f'  {name:<16}{s["counters"].get(name) or 0:>18,.0f}{s["outside_counters"].get(name) or 0:>18,.0f}')
    c = s['counters']
    if c.get('instructions') and c.get('cycles'):
        i = c['instructions']
        print(f'  IPC {i / c["cycles"]:.2f}; per 1000 instructions: branch misses {1000 * c.get("branch-misses", 0) / i:.2f}, L1d misses {1000 * c.get("L1d-misses", 0) / i:.2f}, dTLB misses {1000 * c.get("dTLB-misses", 0) / i:.3f}')
    if s['multiplexed']:
        print('  note: a counter was multiplexed, and its value is what ran')


def fraction(n, total):
    return 100 * n / total if total else 0


def print_functions(s, top):
    t_total = s['weight'].get('time', 0)
    i_total = s['samples'].get('instructions', 0)
    per_sample_ms = 1 / 1000
    per_instr = s['period'] / max(1, s['n_instructions'])
    print(f'\nby function, with inlined code charged where it was written (top {top} by time):')
    print(f'  {"time%":>6}{"ms":>9}{"instr%":>8}{"Minstr":>10}  function')
    rows = sorted(s['functions'].items(), key=lambda kv: -(kv[1]['time'] or kv[1]['instructions'] * 1e-6))
    for key, f in rows[:top]:
        name, file = key.split('\t')
        print(f'  {fraction(f["time"], t_total):>6.2f}{f["time"] * per_sample_ms / max(1, s["n_time"]):>9.2f}{fraction(f["instructions"], i_total):>8.2f}{f["instructions"] * per_instr / 1e6:>10.2f}  {name}  [{os.path.basename(file) or "no line info"}]')


def print_modules(s):
    t_total = s['weight'].get('time', 0)
    i_total = s['samples'].get('instructions', 0)
    per_sample_ms = 1 / 1000
    print('\nby module; "innermost" charges inlined accessors and the standard library to themselves:')
    print(f'  {"module":<26}{"time%":>7}{"ms":>9}{"instr%":>8}{"innermost time%":>17}{"innermost instr%":>18}')
    for name, m in sorted(s['modules'].items(), key=lambda kv: -(kv[1]['time'] or kv[1]['instructions'] * 1e-6)):
        print(f'  {name:<26}{fraction(m["time"], t_total):>7.2f}{m["time"] * per_sample_ms / max(1, s["n_time"]):>9.2f}{fraction(m["instructions"], i_total):>8.2f}{fraction(m["time_inner"], t_total):>17.2f}{fraction(m["instructions_inner"], i_total):>18.2f}')


def print_symbols(s, top):
    t_total = s['weight'].get('time', 0)
    i_total = s['samples'].get('instructions', 0)
    print(f'\nby symbol as linked, from nm; "with callees" comes from the frame pointers (top {top} by time):')
    print(f'  {"self time%":>11}{"with callees":>13}{"self instr%":>12}{"with callees":>13}  symbol')
    rows = sorted(s['symbols'].items(), key=lambda kv: -(kv[1]['time'] or kv[1]['instructions'] * 1e-6))
    for name, f in rows[:top]:
        print(f'  {fraction(f["time"], t_total):>11.2f}{fraction(f["time_incl"], t_total):>13.2f}{fraction(f["instructions"], i_total):>12.2f}{fraction(f["instructions_incl"], i_total):>13.2f}  {name}')


def source_line(file, no):
    src = read_source(file)
    return src[no - 1].strip()[:70] if src and 0 < no <= len(src) else ''


def print_lines(s, k, inline_detail):
    t_total = s['weight'].get('time', 0)
    i_total = s['samples'].get('instructions', 0)
    rows = sorted(s['functions'].items(), key=lambda kv: -(kv[1]['time'] or kv[1]['instructions'] * 1e-6))
    print(f'\nby source line, for the {k} hottest functions:')
    for key, f in rows[:k]:
        name, file = key.split('\t')
        print(f'  {name} ({file or "no line info"}): {fraction(f["time"], t_total):.2f}% of time, {fraction(f["instructions"], i_total):.2f}% of instructions')
        if not file:
            continue
        ls = s['lines'].get(key, {})
        for no, v in sorted(ls.items(), key=lambda kv: -(kv[1]['time'] or kv[1]['instructions'] * 1e-6))[:8]:
            print(f'    {fraction(v["time"], t_total):>6.2f}%{fraction(v["instructions"], i_total):>7.2f}%  :{no:<6} {source_line(file, int(no))}')
            if inline_detail:
                for what, n in v['inner']:
                    print(f'             {n:>9} of the samples or microseconds are inside {what}')


def print_callers(s, k):
    if not s['callers']:
        return
    print(f'\ncallers of the {k} hottest symbols, from the frame pointers (weight: microseconds of the time runs plus samples of the instruction runs):')
    rows = sorted(s['symbols'].items(), key=lambda kv: -(kv[1]['time'] + kv[1]['instructions']))
    for name, _ in rows[:k]:
        c = s['callers'].get(name)
        if not c:
            continue
        print(f'  {name}:')
        for who, n in c[:5]:
            print(f'    {n:>8}  {who}')


def print_report(s, args):
    print_header(s)
    print_phases(s)
    print_counters(s)
    print_functions(s, args.top)
    print_modules(s)
    print_symbols(s, args.top)
    print_lines(s, args.lines, args.inline_detail)
    print_callers(s, args.callers)


def compare(a_dir, b_dir, top):
    a, b = load_summary(a_dir), load_summary(b_dir)
    print(f'A {a["name"]}: {a["binary"]} sha {a["sha"][:12]}\nB {b["name"]}: {b["binary"]} sha {b["sha"][:12]}')
    if a['doc'] != b['doc']:
        print('note: these are different documents')
    print(f'\n{"":<18}{"A":>18}{"B":>18}{"B-A":>16}{"B/A":>8}')

    def row(name, x, y, fmt='{:,.0f}'):
        if x is None or y is None:
            return
        print(f'{name:<18}{fmt.format(x):>18}{fmt.format(y):>18}{fmt.format(y - x):>16}{(y / x if x else 0):>8.3f}')
    for name, _, _ in proflib.EVENTS:
        row(name, a['counters'].get(name), b['counters'].get(name))
    row('wall ms', a['wall_ms'], b['wall_ms'], '{:,.2f}')
    row('user+sys ms', a['cpu_ms'], b['cpu_ms'], '{:,.2f}')
    print(f'\nload up to {a["load"]:.1f} and {b["load"]:.1f}: wall and CPU time are as good as that; instructions do not depend on it')

    def per_run(s, f):
        time_ = f['time'] / 1000 / max(1, s['n_time'])
        instr = f['instructions'] * s['period'] / max(1, s['n_instructions'])
        return time_, instr
    keys = set(a['functions']) | set(b['functions'])
    zero = {'time': 0, 'instructions': 0}
    rows = []

    def noise(s, f):
        """One sigma of a function's instructions per run: the samples are counts."""
        return f['instructions'] ** 0.5 * s['period'] / max(1, s['n_instructions'])
    for k in keys:
        fa, fb = a['functions'].get(k, zero), b['functions'].get(k, zero)
        ta, ia = per_run(a, fa)
        tb, ib = per_run(b, fb)
        rows.append((ib - ia, tb - ta, ia, ib, ta, tb, k, (noise(a, fa) ** 2 + noise(b, fb) ** 2) ** 0.5))
    rows.sort(key=lambda r: -abs(r[0]))
    print(f'\nby function, per run, by the size of the change in instructions (top {top}); sigma is the sampling noise of that delta, and a delta within 3 sigma is nothing:')
    print(f'  {"Minstr A":>10}{"Minstr B":>10}{"delta":>10}{"sigma":>8}{"ms A":>9}{"ms B":>9}{"delta":>9}{"isamples A":>12}{"isamples B":>12}  function')
    for di, dt, ia, ib, ta, tb, k, sigma in rows[:top]:
        name, file = k.split('\t')
        sa, sb = a['functions'].get(k, zero)['instructions'] / max(1, a['n_instructions']), b['functions'].get(k, zero)['instructions'] / max(1, b['n_instructions'])
        print(f'  {ia / 1e6:>10.2f}{ib / 1e6:>10.2f}{di / 1e6:>+10.2f}{sigma / 1e6:>8.2f}{ta:>9.2f}{tb:>9.2f}{dt:>+9.2f}{sa:>12.1f}{sb:>12.1f}  {name}  [{os.path.basename(file) or "no line info"}]')
    ma = {x['name']: x for x in a['phases']}
    print('\nphases, Minstr up to the row and ms from the spawn:')
    print(f'  {"phase":<28}{"Minstr A":>12}{"Minstr B":>12}{"ms A":>10}{"ms B":>10}')
    for x in b['phases']:
        y = ma.get(x['name'])
        if y:
            print(f'  {x["name"]:<28}{y["instructions"] / 1e6:>12.2f}{x["instructions"] / 1e6:>12.2f}{y["t_ms"]:>10.2f}{x["t_ms"]:>10.2f}')


# ------------------------------------------------------------------------------------------------ the command
def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('docs', nargs='*')
    ap.add_argument('--build', action='store_true')
    ap.add_argument('--no-frame-pointers', action='store_true')
    ap.add_argument('--jobs', '-j', type=int, default=None)
    ap.add_argument('--binary')
    ap.add_argument('--name', help='the result set under target/prof-runs/profiles/; default: the first 12 hex of the binary')
    ap.add_argument('-n', type=int, default=None, help='runs per sampler; default 5, and 100 for hello, which has few samples a run')
    ap.add_argument('--source', choices=['both', 'time', 'instructions'], default='both')
    ap.add_argument('--hz', type=int, default=2000)
    ap.add_argument('--period', type=int, default=None, help='instructions per sample; default about 30000 samples per run, at most one per 200000 instructions')
    ap.add_argument('--cpu', type=int, default=None)
    ap.add_argument('--cold-tree', action='store_true', help='an empty cache each run, so that rstex finds the tree again: the first run on a machine')
    ap.add_argument('--top', type=int, default=30)
    ap.add_argument('--lines', type=int, default=8)
    ap.add_argument('--callers', type=int, default=8)
    ap.add_argument('--inline-detail', action='store_true')
    ap.add_argument('--report', metavar='DIR')
    ap.add_argument('--compare', nargs=2, metavar=('DIR_A', 'DIR_B'))
    args = ap.parse_args()
    if args.build:
        return build(args)
    if args.compare:
        return compare(args.compare[0], args.compare[1], args.top)
    if args.report:
        return print_report(load_summary(args.report, args.callers), args)
    if not args.docs or not args.binary:
        ap.error('need documents and --binary, or --build, --report or --compare')
    engine = proflib.Engine('prof', args.binary)
    if args.cpu is not None:
        os.sched_setaffinity(0, {args.cpu})
    name = args.name or engine.sha[:12]
    n_given = args.n
    for doc in proflib.resolve_docs(args.docs):
        dest = os.path.join(proflib.out_root(), 'profiles', name, doc.id.replace('/', '_'))
        shutil.rmtree(dest, ignore_errors=True)
        args.n = n_given or (100 if doc.name in ('hello', 'plain-hello') else 5)
        run_doc(doc, engine, dest, args)
        s = analyse(dest, args.callers)
        print_report(s, args)
        print(f'results: {dest}\n')


if __name__ == '__main__':
    main()
