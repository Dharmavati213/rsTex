"""What rstex_prof.py and bench.py share: the document set, formats, counters around a child process, and the files a prof build writes.

Documents are entries of the corpus manifests, compiled the way tests/corpus/run.py compiles them: its argv, its environment, and the
oracle's last-pass side files (.aux and the like) in place, so a run measures one final pass.  run.py is imported rather than copied.
Everything a run writes stays under target/, since a document's log and PDF are not for the repository.  Nothing here reads a document,
and what these tools print is timings, counters and function names.
"""
import ctypes
import hashlib
import importlib.util
import json
import os
import secrets
import shutil
import statistics
import struct
import subprocess
import sys
import time
from array import array

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

# Short names for the documents DESIGN.md section 2 sets targets for.  A name that is no key here is taken as a manifest id.
NAMED = {
    'hello': 'ladder/04-latex-hello',
    'plain-hello': 'ladder/00-plain-hello',
    'tikzcd-many': 'ladder/15-tikzcd-many',
    'texweb': 'ladder/03-plain-texweb',
    'sga3': 'local/sga3/SGA3-English',
    'main': 'user/main',
    'LMB': 'user/LMB',
    'summary': 'user/summary',
    'SGA1-VI': 'user/SGA1-VI',
}
SETS = {
    'baseline': ['hello', 'plain-hello', 'tikzcd-many', 'texweb', 'summary', 'SGA1-VI', 'LMB', 'main', 'sga3'],
    'quick': ['hello', 'plain-hello'],
    'user': ['summary', 'SGA1-VI', 'LMB', 'main'],
    'ladder': ['ladder/*'],
}


def die(message):
    sys.stderr.write(f'{os.path.basename(sys.argv[0])}: {message}\n')
    sys.exit(2)


def target_root():
    return os.environ.get('CARGO_TARGET_DIR') or os.path.join(REPO, 'target')


def out_root():
    """Everything these tools write.  A document's output must not land anywhere git could see it."""
    root = os.path.abspath(os.path.join(target_root(), 'prof-runs'))
    if '/target/' not in root + '/':
        die(f'{root} is not under a directory called target')
    os.makedirs(root, exist_ok=True)
    return root


def sha_file(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


def load_script(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


def corpus_dir():
    """tests/corpus of this tree or, for a git worktree that does not hold it, of the checkout the worktree belongs to."""
    candidates = [os.environ.get('RSTEX_CORPUS'), os.path.join(REPO, 'tests', 'corpus')]
    r = subprocess.run(['git', 'rev-parse', '--path-format=absolute', '--git-common-dir'], cwd=REPO, capture_output=True, text=True, check=False)
    if r.returncode == 0:
        candidates.append(os.path.join(os.path.dirname(r.stdout.strip()), 'tests', 'corpus'))
    for c in candidates:
        if c and os.path.isfile(os.path.join(c, 'run.py')):
            return c
    die('no tests/corpus/run.py found: set RSTEX_CORPUS')


_corpus = None


def corpus():
    global _corpus
    if _corpus is None:
        _corpus = load_script('rstex_corpus_run', os.path.join(corpus_dir(), 'run.py'))
    return _corpus


_integration = None


def integration():
    """tests/integration/run.py, for FORMAT_RECIPES and the environment a format is built in."""
    global _integration
    if _integration is None:
        _integration = load_script('rstex_integration_run', os.path.join(REPO, 'tests', 'integration', 'run.py'))
    return _integration


# ------------------------------------------------------------------------------------------------ documents
class Doc:
    def __init__(self, name, entry):
        self.name, self.entry = name, entry
        self.meta = None

    @property
    def id(self):
        return self.entry['id']

    @property
    def format(self):
        return self.entry.get('fmt') or ('pdflatex' if self.entry['mode'] == 'latex' else 'pdftex')

    def prepare(self):
        """The oracle's last-pass state, from run.py's cache, or made now by compiling with the oracle."""
        if self.meta is None:
            self.meta, _ = corpus().prepare_entry(self.entry)
        return self.meta


class FormatDoc(Doc):
    """`fmt:pdflatex`: a build of that format, as a document to profile.  It runs in an empty directory with format_env."""

    def __init__(self, fmt):
        super().__init__('fmt-' + fmt, {'id': 'fmt/' + fmt, 'jobname': fmt, 'mode': 'format', 'fmt': fmt})

    def prepare(self):
        return {}


def resolve_docs(names):
    """Named documents, sets, manifest ids, `prefix/*` and `fmt:NAME`, in the order given."""
    entries, _, _ = corpus().load_manifests(None)
    by_id = {e['id']: e for e in entries}
    wanted = []
    for name in names:
        wanted += SETS.get(name, [name])
    docs, seen = [], set()
    short = {v: k for k, v in NAMED.items()}
    for w in wanted:
        if w.startswith('fmt:'):
            docs.append(FormatDoc(w[4:]))
            continue
        if w.endswith('/*'):
            ids = sorted(i for i in by_id if i.startswith(w[:-1]))
            if not ids:
                die(f'no manifest entry matches {w}')
        else:
            ids = [NAMED.get(w, w)]
        for i in ids:
            if i in seen:
                continue
            seen.add(i)
            if i not in by_id:
                die(f'no manifest entry {i!r}')
            docs.append(Doc(short.get(i, i), by_id[i]))
    return docs


class Workdir:
    """One run's output directory, with the side files of the oracle's last pass in place."""

    def __init__(self, doc):
        self.root = os.path.join(out_root(), 'w', secrets.token_hex(8))
        self.out = os.path.join(self.root, 'o')
        os.makedirs(self.out)
        if not isinstance(doc, FormatDoc):
            corpus().restore_state(doc.prepare(), self.out)

    def remove(self):
        shutil.rmtree(self.root, ignore_errors=True)


# ------------------------------------------------------------------------------------------------ engines
class Engine:
    """A binary with its own formats, or the oracle with the installed ones."""

    def __init__(self, label, path=None, no_commute=False):
        self.label = label
        self.path = os.path.abspath(path) if path else None
        # The native diagram path is on unless asked otherwise, as in a run of the program.
        self.no_commute = no_commute
        self.is_oracle = path is None
        self.sha = sha_file(self.path) if self.path else None
        self.cache_home = None

    def tree_cache(self):
        """rstex caches where the TeX tree is, or it asks kpsewhich on every run.  One directory per binary, kept between runs: bench's
        first run fills it, and that one run's cost is not what a document costs.  --cold-tree in rstex_prof.py gives an empty one."""
        if self.cache_home is None:
            self.cache_home = os.path.join(out_root(), 'cache', self.sha[:16])
            os.makedirs(self.cache_home, exist_ok=True)
        return self.cache_home

    def format_path(self, name):
        info = build_format(self, name)
        if info['exit'] != 0 or not info['fmt']:
            die(f'{self.path}: format {name} failed to build: {info}')
        return info['fmt']

    def command(self, doc, out):
        """(argv, cwd, env) for a compile of `doc` into the directory `out`."""
        if isinstance(doc, FormatDoc):
            return format_command(self, doc.format), out, format_env(self)
        c = corpus()
        e = doc.entry
        if self.is_oracle:
            return c.tex_argv(e, c.oracle_engine(e), [], out), e['cwd'], c.entry_env(e)
        fmt = doc.format
        argv = c.tex_argv(e, self.path, [], out)
        flag = '-fmt=' + fmt
        if flag not in argv:
            die(f'{doc.id}: expected {flag} on the command line')
        i = argv.index(flag)
        argv[i:i + 1] = ['-progname=' + fmt, '-fmt=' + self.format_path(fmt)]
        env = c.entry_env(e)
        env['XDG_CACHE_HOME'] = self.tree_cache()
        # These tools measure a cold run, in a new directory each time, which would write a checkpoint each time.  warm.py measures the others.
        env['RSTEX_NO_SNAPSHOT'] = '1'
        if self.no_commute:
            env['RSTEX_NO_COMMUTE'] = '1'
        env.update(json.loads(os.environ.get('RSTEX_PROF_EXTRA_ENV', '{}')))
        return argv, e['cwd'], env


def build_format(engine, name):
    """As fmtutil builds it, once per binary: tests/integration/run.py's FORMAT_RECIPES.  The result records how long that took."""
    recipes = integration().FORMAT_RECIPES
    if name not in recipes:
        die(f'no recipe for the format {name}')
    d = os.path.join(out_root(), 'fmt', engine.sha[:16], name)
    marker = os.path.join(d, 'result.json')
    if os.path.exists(marker):
        with open(marker) as f:
            return json.load(f)
    shutil.rmtree(d, ignore_errors=True)
    os.makedirs(d)
    run = run_format_build(engine, name, d)
    info = {'exit': run['rc'], 'wall_ms': run['wall_ms'], 'cpu_ms': run['cpu_ms'],
            'fmt': os.path.join(d, name + '.fmt') if os.path.exists(os.path.join(d, name + '.fmt')) else None}
    with open(marker, 'w') as f:
        json.dump(info, f)
    return info


def format_env(engine=None):
    env = dict(integration().ENVIRONMENT)
    for k in ('PATH', 'HOME'):
        if k in os.environ:
            env[k] = os.environ[k]
    if engine is not None and not engine.is_oracle:
        env['XDG_CACHE_HOME'] = engine.tree_cache()
    return env


def format_command(engine, name):
    path = shutil.which('pdftex') if engine.is_oracle else engine.path
    return [path] + list(integration().FORMAT_RECIPES[name])


def run_format_build(engine, name, cwd):
    return run_timed(format_command(engine, name), cwd, format_env(engine))


# ------------------------------------------------------------------------------------------------ running
def run_timed(argv, cwd, env, stamp_t0=False):
    """Wall clock from just before the spawn to the reap, and the child's own user and system time.  No counters and no preexec_fn: this
    is the plain path, by vfork.  With stamp_t0 the child is told the time of the spawn as RSTEX_PROF_T0, the zero of its phase table."""
    if stamp_t0:
        env = dict(env)
        env['RSTEX_PROF_T0'] = str(time.monotonic_ns())
    t0 = time.monotonic_ns()
    p = subprocess.Popen(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    _, status, ru = os.wait4(p.pid, 0)
    t1 = time.monotonic_ns()
    p.returncode = os.waitstatus_to_exitcode(status)
    return {'rc': p.returncode, 'wall_ms': (t1 - t0) / 1e6, 'cpu_ms': (ru.ru_utime + ru.ru_stime) * 1e3, 'user_ms': ru.ru_utime * 1e3,
            'sys_ms': ru.ru_stime * 1e3, 'minflt': ru.ru_minflt, 'majflt': ru.ru_majflt, 'maxrss_kb': ru.ru_maxrss, 't0_ns': t0, 'reap_ns': t1}


# perf_event_attr bits.  The attribute block, as PERF_ATTR_SIZE_VER7.
ATTR_SIZE = 128
F_DISABLED, F_INHERIT, F_EXCLUDE_KERNEL, F_EXCLUDE_HV, F_ENABLE_ON_EXEC = 1 << 0, 1 << 1, 1 << 5, 1 << 6, 1 << 12
READ_TIMES = 3
EVENTS = [('instructions', 0, 1), ('cycles', 0, 0), ('branch-misses', 0, 5), ('L1d-misses', 3, 0x10000), ('dTLB-misses', 3, 0x10003),
          ('page-faults', 1, 2)]
# What the sampler of platform/unix.rs takes as RSTEX_PROF_EVENT, with the type and config to count the same thing from outside.  `raw-HEX` is an event of the CPU's own.
SAMPLE_EVENTS = {'instructions': (0, 1), 'cycles': (0, 0), 'l1d-misses': (3, 0x10000), 'branch-misses': (0, 5), 'dtlb-misses': (3, 0x10003), 'cache-misses': (0, 3)}
SYS_PERF_EVENT_OPEN = {'x86_64': 298, 'aarch64': 241}.get(os.uname().machine)
PERF_FLAG_FD_CLOEXEC = 8
_libc = None


def perf_open(pid, type_, config):
    """A user-mode counter on process `pid`, starting at its next exec, that follows its threads."""
    global _libc
    if SYS_PERF_EVENT_OPEN is None:
        return -1
    if _libc is None:
        _libc = ctypes.CDLL(None, use_errno=True)
    flags = F_DISABLED | F_INHERIT | F_EXCLUDE_KERNEL | F_EXCLUDE_HV | F_ENABLE_ON_EXEC
    head = struct.pack('<IIQQQQQ', type_, ATTR_SIZE, config, 0, 0, READ_TIMES, flags)
    buf = ctypes.create_string_buffer(head + bytes(ATTR_SIZE - len(head)), ATTR_SIZE)
    return _libc.syscall(ctypes.c_long(SYS_PERF_EVENT_OPEN), buf, ctypes.c_int(pid), ctypes.c_int(-1), ctypes.c_int(-1), ctypes.c_ulong(PERF_FLAG_FD_CLOEXEC))


def sample_event(name):
    """(perf type, config) of a name the sampler takes."""
    if name.startswith('raw-'):
        return 4, int(name[4:], 16)
    return SAMPLE_EVENTS[name]


def run_counted(argv, cwd, env, events=EVENTS):
    """Runs a command with hardware counters from its exec to its exit, whatever the binary: this is how pdflatex and a shipped build get
    instruction counts.  It forks and waits on a pipe, as perf stat does, since the counters must exist before the exec.  The fork makes the
    start slower than run_timed's, by the exec's teardown of the forked interpreter, so quote times from run_timed.  Returns run_timed's fields
    and `counters`.  Do not point this at a prof build with RSTEX_PROF* set: two sets of counters do not fit the PMU, and they get multiplexed."""
    exe = shutil.which(argv[0]) or argv[0]
    r, w = os.pipe()
    pid = os.fork()
    if pid == 0:
        status = 127
        try:
            os.close(w)
            os.chdir(cwd)
            null = os.open(os.devnull, os.O_RDWR)
            for fd in (0, 1, 2):
                os.dup2(null, fd)
            os.read(r, 1)
            os.close(r)
            os.execve(exe, argv, env)
        finally:
            os._exit(status)
    os.close(r)
    fds = [(name, perf_open(pid, t, c)) for name, t, c in events]
    t0 = time.monotonic_ns()
    os.write(w, b'x')
    os.close(w)
    _, status, ru = os.wait4(pid, 0)
    t1 = time.monotonic_ns()
    counters = {}
    for name, fd in fds:
        if fd < 0:
            continue
        v = struct.unpack('<QQQ', os.read(fd, 24))
        os.close(fd)
        counters[name] = {'value': v[0], 'enabled_ns': v[1], 'running_ns': v[2]}
    return {'rc': os.waitstatus_to_exitcode(status), 'wall_ms': (t1 - t0) / 1e6, 'cpu_ms': (ru.ru_utime + ru.ru_stime) * 1e3, 'user_ms': ru.ru_utime * 1e3,
            'sys_ms': ru.ru_stime * 1e3, 'minflt': ru.ru_minflt, 'majflt': ru.ru_majflt, 'maxrss_kb': ru.ru_maxrss, 'counters': counters,
            't0_ns': t0, 'reap_ns': t1}


def seal_checkpoint(reset, argv, cwd, env):
    """What comes after the run that wrote a checkpoint, for a later run to be what a user gets: a restore checks the image's sum until one has passed on a file old enough for its
    stat to show any change, checkpoint.rs's RACY_NS, and that one leaves a seal."""
    time.sleep(2.1)
    reset()
    run_timed(argv, cwd, env)


# ------------------------------------------------------------------------------------------------ what a prof build writes
def parse_counters(path):
    """The text file of crates/engine/src/prof.rs."""
    res = {'clock': {}, 'phases': [], 'scopes': {}, 'counters': {}, 'rusage': {}, 'sampler': {}}
    with open(path, encoding='utf-8', errors='replace') as f:
        for line in f:
            p = line.rstrip('\n').split('\t')
            if p[0] == 'phase':
                res['phases'].append({'name': p[1], 't_ns': int(p[2]), 'cpu_ns': int(p[3]), 'instructions': int(p[4]), 'cycles': int(p[5]), 'minflt': int(p[6]), 'majflt': int(p[7])})
            elif p[0] == 'scope':
                res['scopes'][p[1]] = {'count': int(p[2]), 'total_ns': int(p[3]), 'instructions': int(p[4]), 'minflt': int(p[5])}
            elif p[0] == 'counter':
                res['counters'][p[1]] = {'value': int(p[2]), 'enabled_ns': int(p[3]), 'running_ns': int(p[4])}
            elif p[0] == 'clock':
                res['clock'][p[1]] = int(p[2])
            elif p[0] == 'rusage':
                res['rusage'][p[1]] = int(p[2])
            elif p[0] == 'sampler':
                res['sampler'][p[1]] = p[2]
    return res


def parse_samples(path):
    """The raw file of platform/unix.rs: (info, program counters, weights in microseconds of CPU, frame offsets, frames), the addresses as the program saw them."""
    with open(path, 'rb') as f:
        data = f.read()
    if data[:8] != b'RSTXPROF':
        die(f'{path}: not a sample file')
    version, source, period, n, dropped, nf, base = struct.unpack_from('<7Q', data, 8)
    if version != 2:
        die(f'{path}: version {version}')
    pos = 8 + 7 * 8
    pcs = array('Q')
    pcs.frombytes(data[pos:pos + n * 8])
    pos += n * 8
    weights = array('I')
    weights.frombytes(data[pos:pos + n * 4])
    pos += n * 4
    starts = array('I')
    starts.frombytes(data[pos:pos + (n + 1) * 4] if n else b'')
    pos += (n + 1) * 4
    frames = array('Q')
    frames.frombytes(data[pos:pos + nf * 8])
    names = ['itimer'] + list(SAMPLE_EVENTS)
    info = {'source': 'raw' if source == 100 else names[source] if source < len(names) else '?', 'period': period, 'samples': n, 'dropped': dropped, 'frames': nf, 'base': base}
    return info, pcs, weights, starts, frames


# ------------------------------------------------------------------------------------------------ small statistics
def percentile(values, p):
    """Nearest rank."""
    s = sorted(values)
    if not s:
        return None
    k = max(0, min(len(s) - 1, int(len(s) * p / 100 + 0.999999) - 1))
    return s[k]


def median(values):
    values = list(values)
    return statistics.median(values) if values else None


def load_now():
    return os.getloadavg()[0]


def wait_for_load(limit, patience):
    """Waits for the one-minute load average to fall under `limit`, for `patience` seconds at most.  Returns the load it stopped at and the wait."""
    t0 = time.monotonic()
    if limit is not None:
        while load_now() > limit and time.monotonic() - t0 < patience:
            time.sleep(2)
    return load_now(), time.monotonic() - t0
