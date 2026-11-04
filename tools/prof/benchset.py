#!/usr/bin/env -S python3 -I
r"""benchset.py: runs a tier of tests/bench/set.json under the scenarios of tests/bench/README.md, for any number of rstex binaries and both baselines.  One JSON line per measurement.

    benchset.py run --tier quick --engine plain=PATH --engine pgo=PATH [--commute PATH] [--baselines arch,official] [--method counted|timed|both] [--scenarios first,last-cold,...]
                    [--ids REGEX] [-j N] [--cpus LIST] [--cpu K] [--max-load L] [--load-patience S] [--out FILE] [--prepare] [--no-private] [--thin NAME] [--timed-engines NAMES] [--keep-wrong DIR] [--frozen] [--cold]
    benchset.py plan --tier quick [--scenarios ...]                      what a run would measure: rows by scenario, and no run
    benchset.py preflight --tier quick                                   whether this machine can run it: oracles, pins, the tree, the baselines and programs.  Counts and manifest ids, and no run

Everything a run writes is under target/: the rows, each run's output directory, the mirrors the edits go in, formats and caches.  A document's directory is only read.  A private document's rows hold its manifest id,
numbers and verdicts, and never a file name, a line of its log or anything of an edit but its kind.

**What a row is.**  A row is one measurement: a document under a scenario, a variant, a side, a method and a round.  It records the binary's hash, the entry's closures as they are now and whether they are the set's pins, the
tree's version, the machine, its governor, the load before and after, and the verdict of the output check.  `tests/bench/report.py` reads these.  A run that is killed resumes: it appends to the same file, what is there is not
measured again, and a line cut short is dropped.  A row from another binary under the same side's name, or from another input hash for the document, is refused: the run stops, or leaves that document out and says so.

**The run that is compared.**  Argv, environment, side files and the oracle's own files come from tests/corpus/run.py and tools/prof/proflib.py by import.  Counters are proflib's, user mode, from the exec on, with threads and children.  Each run goes through tools/prof/benchrun.py, which
is small, for a reason its first lines give: ru_maxrss from a larger parent is that parent's size.  `first`, `edit` and `pixels` run in a mirror of the document with Lemma's environment and flags, as tests/until_stable/compare.py and
session.py do; those files are not on every branch this runs from, so what this needs of them is here, and tests/bench/selfcheck.py holds that it stays the same where they are.

**Counted and timed.**  `counted` runs documents in parallel with -j, as instructions do not care.  `timed` is serial on the core --cpu, as tools/prof/bench.py pins it, in rounds with a control: each side in turn in an order that turns round each
round, so a warming cache favours nobody.  A row is marked `clock_ok` only if it was timed, the load before and after it was at most --max-load, the governor is `performance`, and the core's sibling was idle: otherwise its instructions stand, and report.py
quotes no clock from it.  A run pinned to one core gives an engine's threads that core alone, which is against rstex and is what bench.py has always done.

**Formats and caches.**  A format per binary, proflib's for rstex and baselines.py's recipes for each pdfTeX.  A binary under test is copied to a directory of its own under the name `rstex`, since its messages print its name.  An empty cache costs rstex
about 780 M instructions once, which is no cost of a document: so a cache per binary is filled once, its result is recorded in the run's header as `empty_cache_minstr`, and each engine, document and run kind gets its own cache from it by hard links, which
are safe as those files are replaced and never changed in place.  A small job then has it hold for that run kind's environment.

**The output check.**  Every row is held against the oracle's output for that state, or for `first` and `edit`, where no oracle exists, against latexmk over Arch's pdfTeX on the same edit at the same path: tests/corpus/run.py's verdicts and its log, side-file and exit rules, in the cheapest
form that is still right.  The PDF is first compared as the normalized bytes run.py's L6 uses (streams inflated, banner, /Producer, /ID, offsets and lengths masked) with `qpdf --check` on what the masks hide; only if those differ does tests/harness/compare_pdf.py run, which judges page content and
structure.  Equal normalized bytes say all that compare_pdf.py would: tests/bench/selfcheck.py checks that on documents.  A verdict is kept per document by the hash of the output, so ten runs with the same bytes cost one check and ten hashes.  L1, L2, L6 and rasters are `tests/corpus/run.py compare`'s job
and are not repeated here.

**Scenarios that use the document's state.**  warm, hits and resume all restore the oracle's last-pass side files and compile a source with no edit, so that each can be checked against the oracle: so they are what a restore can do at best, and say nothing of a save.  A checkpoint can care about a word in the body, where a preamble looked at
the file that holds it, as every Stacks chapter does: warm-word has that word, and `edit` is a whole sitting.

Privacy: progress lines and rows name manifest ids only.  A failure in a private document is recorded by the class of the exception alone.
"""
import argparse
import ctypes
import fcntl
import gzip
import hashlib
import json
import marshal
import multiprocessing
import os
import re
import shlex
import shutil
import signal
import struct
import subprocess
import sys
import time
import traceback

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import proflib  # noqa: E402

REPO = proflib.REPO
BENCH = os.path.join(REPO, 'tests', 'bench')
census = proflib.load_script('bench_census', os.path.join(BENCH, 'census.py'))
edits = census.edits
baselines = proflib.load_script('bench_baselines', os.path.join(BENCH, 'baselines.py'))
report = proflib.load_script('bench_report_tool', os.path.join(BENCH, 'report.py'))

# As tests/until_stable/common.py has them: what crates/lemma-build/src/request.rs puts in the environment and on the command line, and how `rstex --until-stable` speaks.
LEMMA_ENV = {'max_print_line': '10000', 'error_line': '254', 'half_error_line': '238'}
LEMMA_FLAGS = ['-synctex=1', '-interaction=nonstopmode', '-file-line-error', '-recorder']
TOOL_LINE = re.compile(rb'^rstex: until-stable: tool=(\S+) reason=(\S+) file=(.*)$', re.M)
PASSES_LINE = re.compile(rb'^rstex: until-stable: passes=(\d+) result=(\w+)', re.M)
NOT_COMPARED = ('.fdb_latexmk', '.rstex-tools', '.blg', '.ilg')
STATUS_TOOL = 10
OUTPUTS = ('.aux', '.log', '.toc', '.lof', '.lot', '.out', '.fls', '.fdb_latexmk', '.synctex.gz', '.rstex-tools')
RULE_RUNS = re.compile(rb"^Latexmk: applying rule '(\w+)", re.M)
# `first` is a compile from the command line: no SyncTeX, which a resume refuses, and no shell escape, as for all the corpus.
FIRST_FLAGS = ['-interaction=nonstopmode', '-file-line-error', '-recorder', '-no-shell-escape']
TRACE_RESTORED = re.compile(rb'^rstex: checkpoint: restored$', re.M)
TRACE_PAGES = re.compile(rb'pages: (\d+) records, using (\d+)')
PRIME_LINE = r'\shipout\hbox{\font\rstexbenchf=cmr10 \rstexbenchf a}\csname @@end\endcsname\end'
SCENARIOS = ('first', 'last-cold', 'warm', 'warm-word', 'hits', 'resume', 'edit', 'fmt', 'pixels', 'diagram-alone', 'native', 'old-map', 'side-stress')
DOC_SCENARIOS = ('first', 'last-cold', 'warm', 'warm-word', 'hits', 'resume', 'edit', 'native', 'old-map', 'pixels')
FMT_ROUNDS = {'quick': 1, 'standard': 3, 'holdout': 1, 'full': 3}
PIXEL_ROUNDS = 4
SIDE_TOOLS = ('latexmk', 'bibtex', 'biber', 'makeindex', 'qpdf', 'pdftoppm', 'pdftotext', 'pdfinfo')
BANNER_ANY = re.compile(rb'(/PTEX[._]Fullbanner \(This is pdfTeX, Version [^()\\]* )\([^()\\]*\)( kpathsea version [^()\\]*\))')
RAST_DPI, RAST_LEVEL = 200, 64


def die(message):
    sys.stderr.write(f'benchset.py: {message}\n')
    sys.exit(2)


def is_private(doc_id):
    return doc_id.startswith(census.PRIVATE_PREFIXES)


def under_target(path):
    """What this writes and removes has to be under target/, as a real directory: a document's own directories are read and never touched."""
    real = os.path.realpath(path)
    if '/target/' not in real + '/' or os.path.islink(path):
        raise RuntimeError('not under a directory called target: ' + os.path.basename(path))
    return path


# ------------------------------------------------------------------------------------------------ the run of a program
def event_attr(type_, config):
    flags = proflib.F_DISABLED | proflib.F_INHERIT | proflib.F_EXCLUDE_KERNEL | proflib.F_EXCLUDE_HV | proflib.F_ENABLE_ON_EXEC
    head = struct.pack('<IIQQQQQ', type_, proflib.ATTR_SIZE, config, 0, 0, proflib.READ_TIMES, flags)
    return head + bytes(proflib.ATTR_SIZE - len(head))


EVENTS = [('instructions', event_attr(0, 1)), ('cycles', event_attr(0, 0)), ('page-faults', event_attr(1, 2))]


class Runner:
    """tools/prof/benchrun.py, one for each process that measures."""

    def __init__(self):
        self.p = subprocess.Popen([sys.executable, '-I', '-S', os.path.join(HERE, 'benchrun.py')], stdin=subprocess.PIPE, stdout=subprocess.PIPE)

    def run(self, argv, cwd, env, stderr=None, limit=None, merge=False):
        exe = shutil.which(argv[0], path=env.get('PATH')) or argv[0]
        marshal.dump({'exe': exe, 'argv': list(argv), 'cwd': cwd, 'env': dict(env), 'events': EVENTS, 'perf_nr': proflib.SYS_PERF_EVENT_OPEN, 'stderr': stderr, 'merge': merge, 'limit': limit}, self.p.stdin)
        self.p.stdin.flush()
        return marshal.load(self.p.stdout)


def flat(r):
    """A run's numbers for a row."""
    c = r['counters']
    val = lambda name: c[name][0] if name in c else None    # noqa: E731
    return {'rc': r['rc'], 'instructions': val('instructions'), 'cycles': val('cycles'), 'page_faults': val('page-faults'), 'wall_ms': round(r['wall_ms'], 2), 'cpu_ms': round(r['user_ms'] + r['sys_ms'], 2), 'sys_ms': round(r['sys_ms'], 2),
            'maxrss_kb': r['maxrss_kb'], 'timed_out': r['timed_out'], 'multiplexed': any(v[2] < v[1] for v in c.values())}


def add(total, one):
    """Several programs as one measurement: sums, and the peak of any of them."""
    if total is None:
        return dict(one)
    for k in ('instructions', 'cycles', 'page_faults', 'wall_ms', 'cpu_ms', 'sys_ms'):
        total[k] = (total.get(k) or 0) + (one.get(k) or 0)
    total['maxrss_kb'] = max(total.get('maxrss_kb') or 0, one.get('maxrss_kb') or 0)
    total['timed_out'] = total.get('timed_out') or one.get('timed_out')
    total['multiplexed'] = total.get('multiplexed') or one.get('multiplexed')
    total['rc'] = one['rc']
    return total


# ------------------------------------------------------------------------------------------------ sides
class Rstex:
    kind = 'rstex'

    def __init__(self, label, path):
        self.label = label
        src = os.path.realpath(path)
        if not os.path.isfile(src):
            die(f'no such binary for {label}')
        sha = proflib.sha_file(src)
        d = os.path.join(proflib.out_root(), 'bench-bin', sha[:16])
        os.makedirs(d, exist_ok=True)
        dst = os.path.join(d, 'rstex')
        if not os.path.exists(dst):
            tmp = dst + f'.{os.getpid()}'
            shutil.copy2(src, tmp)
            os.replace(tmp, dst)
        self.engine = proflib.Engine(label, dst)
        self.sha = sha[:20]
        self.path = dst
        self.base_cache = None

    def fmt(self, name):
        return self.engine.format_path(name)


class PdfTex:
    kind = 'pdftex'

    def __init__(self, label):
        self.label = label
        self.dir = baselines.bin_dir(label)
        real = os.path.realpath(os.path.join(self.dir, 'pdftex'))
        if not os.path.isfile(real):
            die(f'the baseline {label} has no binary here: run without it by --baselines, and say so in what is quoted')
        self.sha = proflib.sha_file(real)[:20]

    def ensure_fmt(self, names):
        """Formats of the tree alone, by the recipes of tests/bench/baselines.py."""
        d = baselines.fmt_dir(self.label)
        os.makedirs(d, exist_ok=True)
        c = proflib.corpus()
        for name in names:
            if os.path.isfile(os.path.join(d, name + '.fmt')):
                continue
            env = baselines.engine_env(self.label)
            env.update(c.DETERMINISM, **c.NO_MKTEX, **c.HERMETIC)
            proflib.run_timed([os.path.join(self.dir, 'pdftex'), '-ini', '-jobname=' + name, '-progname=' + name] + baselines.RECIPES[name], d, env)
            if not os.path.isfile(os.path.join(d, name + '.fmt')):
                die(f'the baseline {self.label} did not build {name}.fmt')

    def fmt_sha(self, name):
        p = os.path.join(baselines.fmt_dir(self.label), name + '.fmt')
        return proflib.sha_file(p)[:20] if os.path.isfile(p) else None

    def env(self, base=None):
        return baselines.engine_env(self.label, base)

    def latex_exe(self, name='pdflatex'):
        return os.path.join(self.dir, name)


# ------------------------------------------------------------------------------------------------ the machine
def cpu_line(cpu):
    try:
        with open('/proc/stat') as f:
            for line in f:
                if line.startswith(f'cpu{cpu} '):
                    v = [int(x) for x in line.split()[1:]]
                    return sum(v), v[3] + v[4]
    except OSError:
        pass
    return None


def sibling_of(cpu):
    try:
        with open(f'/sys/devices/system/cpu/cpu{cpu}/topology/thread_siblings_list') as f:
            text = f.read().strip()
    except OSError:
        return None
    ids = set()
    for part in text.split(','):
        a, _, b = part.partition('-')
        ids.update(range(int(a), int(b or a) + 1))
    ids.discard(cpu)
    return min(ids) if ids else None


def governor(cpu):
    try:
        with open(f'/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor') as f:
            return f.read().strip()
    except OSError:
        return None


def cpu_model():
    try:
        with open('/proc/cpuinfo') as f:
            for line in f:
                if line.startswith('model name'):
                    return line.split(':', 1)[1].strip()
    except OSError:
        pass
    return None


TREE_CORE = ('oracle_banner', 'kpathsea', 'pdftex_sha', 'pdftex_map_sha', 'texmf_cnf_sha', 'ls_R_sha')


def tree_hash(tree):
    """The tree's version, in a row: what is the same on every machine that holds the same tree."""
    return hashlib.sha256(json.dumps({k: tree.get(k) for k in TREE_CORE}, sort_keys=True).encode()).hexdigest()[:20]


# ------------------------------------------------------------------------------------------------ the output check
def pdf_norm(d):
    """The bytes run.py's L6 compares, whatever deflate did, and with any pdfTeX's or rstex's name in the banner."""
    c = proflib.corpus()
    if any(k in d for k in (b'/FlateDecode', b'/XRef', b'/ObjStm')):
        d = c.inflate_streams(d)
    d = BANNER_ANY.sub(lambda m: m.group(1) + b'()' + m.group(2), d, count=1)
    d = c.l6_mask(d)
    return c._ID_RE.sub(b'', d)


def compare_pdfs(a, b, problems_cache=None):
    """(verdict, how).  None for no difference, 'content' or 'structure', as run.py compare gives them, or 'crash' for a PDF nothing can read."""
    c = proflib.corpus()
    with open(a, 'rb') as f:
        x = f.read()
    with open(b, 'rb') as f:
        y = f.read()
    if pdf_norm(x) == pdf_norm(y):
        known = problems_cache.get(a) if problems_cache is not None else None
        if known is None:
            known = c.pdf_problems(a)
            if problems_cache is not None:
                problems_cache[a] = known
        if not [line for line in c.pdf_problems(b) if line not in known]:
            return None, 'normalized'
    res = c.run_compare_pdf(a, b)
    if res['status'] == 'unreadable-pdf':
        return 'crash', 'compare_pdf'
    if res['status'] == 'tool-error':
        return 'tool', 'compare_pdf'
    if res.get('first_content') or res.get('pages_a') != res.get('pages_b'):
        return 'content', 'compare_pdf'
    if res['status'] != 'identical':
        paths = res.get('structure_paths') or ()
        # As run.py: an embedded file the run wrote itself has the date of that run, on both sides.
        if paths and len(paths) == res.get('structure_diffs') and all(p.endswith('//Params//ModDate') for p in paths):
            return None, 'compare_pdf'
        return 'structure', 'compare_pdf'
    return None, 'compare_pdf'


def hash_bytes(path):
    h = hashlib.sha256()
    try:
        with open(path, 'rb') as f:
            for chunk in iter(lambda: f.read(1 << 20), b''):
                h.update(chunk)
    except OSError:
        return None
    return h.hexdigest()


def snapshot(out, job):
    """Side files, by run.py's rule, less what is a driver's own or what a program says on the way."""
    c = proflib.corpus()
    return {k: v for k, v in c.snapshot_side(out, job).items() if not k.endswith(NOT_COMPARED)}


def synctex_lines(path):
    try:
        with gzip.open(path, 'rb') as f:
            return f.read()
    except (OSError, EOFError):
        return None


# ------------------------------------------------------------------------------------------------ mirrors, as tests/until_stable/common.py has them
def document_env(entry, cache=None):
    env = proflib.corpus().entry_env(entry)
    env.update(LEMMA_ENV)
    env.pop('openin_any', None)
    env.pop('openout_any', None)
    if cache:
        env['XDG_CACHE_HOME'] = cache
    return env


def mirror(entry, dst):
    """A tree under target/ that reads as the document's, with its main file a copy.  Each other entry is a symbolic link, and what is hidden or the job's earlier output stays out."""
    shutil.rmtree(under_target(dst), ignore_errors=True)
    rel = os.path.relpath(entry['main'], entry['cwd'])
    os.makedirs(dst)
    src, to = entry['cwd'], dst
    parts = rel.split(os.sep)
    for i, part in enumerate(parts):
        for name in os.listdir(src):
            if name == part or name.startswith('.') or name.startswith(entry['jobname'] + '.') and name.endswith(OUTPUTS) or name == entry['jobname'] + '.pdf':
                continue
            os.symlink(os.path.join(src, name), os.path.join(to, name))
        if i + 1 < len(parts):
            src, to = os.path.join(src, part), os.path.join(to, part)
            os.makedirs(to)
    shutil.copy2(entry['main'], os.path.join(dst, rel))
    return rel


def mirror_dir_list(src):
    """A \\include'd file's directory under the output directory is the caller's to make.  Through links, as far as a document's own tree can reasonably go."""
    res = []
    for root, dirs, _ in os.walk(src, followlinks=True):
        rel = os.path.relpath(root, src)
        dirs[:] = [d for d in dirs if not d.startswith('.') and rel.count(os.sep) < 6]
        if rel != '.':
            res.append(rel)
    return res


def make_own(src, rel):
    """The mirror's file `rel` as a real file of its own, its directories real too, so that a save can never go through a link to the document's own tree."""
    path = src
    parts = rel.split(os.sep)
    for part in parts[:-1]:
        path = os.path.join(path, part)
        if os.path.islink(path):
            target = os.readlink(path)
            os.unlink(path)
            os.makedirs(path)
            for name in os.listdir(target):
                os.symlink(os.path.join(target, name), os.path.join(path, name))
    path = os.path.join(path, parts[-1])
    if os.path.islink(path):
        target = os.readlink(path)
        os.unlink(path)
        shutil.copy2(target, path)
    return path


def save(path, data):
    """As an editor does: a new file, then a rename.  Never through a link."""
    if os.path.islink(path):
        raise RuntimeError('a save would go through a link')
    tmp = path + '.saving'
    with open(tmp, 'wb') as f:
        f.write(data)
    os.replace(tmp, path)


def latexmk_only(ref, cand, job):
    """Takes out what latexmk leaves and rstex rightly does not: its `.aux` before a first run, if the document never wrote one, and a `.ind` that makeindex made and no pass read."""
    ref = dict(ref)
    aux = job + '.aux'
    if aux not in cand and aux in ref:
        ref.pop(aux)
    for p in [p for p in ref if p.endswith('.ind') and p not in cand]:
        ref.pop(p)
    return ref


# ------------------------------------------------------------------------------------------------ Poppler, as crates/lemma-pdf has it and tools/prof/loop.py measures it
class Poppler:
    def __init__(self):
        self.glib = ctypes.CDLL('libglib-2.0.so.0')
        self.gobject = ctypes.CDLL('libgobject-2.0.so.0')
        self.cairo = ctypes.CDLL('libcairo.so.2')
        self.pop = ctypes.CDLL('libpoppler-glib.so.8')
        vp, d, i = ctypes.c_void_p, ctypes.c_double, ctypes.c_int
        self.glib.g_bytes_new_static.restype = vp
        self.glib.g_bytes_new_static.argtypes = [ctypes.c_char_p, ctypes.c_size_t]
        self.glib.g_bytes_unref.argtypes = [vp]
        self.gobject.g_object_unref.argtypes = [vp]
        self.pop.poppler_document_new_from_bytes.restype = vp
        self.pop.poppler_document_new_from_bytes.argtypes = [vp, ctypes.c_char_p, vp]
        self.pop.poppler_document_get_n_pages.argtypes = [vp]
        self.pop.poppler_document_get_page.restype = vp
        self.pop.poppler_document_get_page.argtypes = [vp, i]
        self.pop.poppler_page_get_size.argtypes = [vp, ctypes.POINTER(d), ctypes.POINTER(d)]
        self.pop.poppler_page_render.argtypes = [vp, vp]
        self.cairo.cairo_image_surface_create.restype = vp
        self.cairo.cairo_image_surface_create.argtypes = [i, i, i]
        self.cairo.cairo_surface_destroy.argtypes = [vp]
        self.cairo.cairo_create.restype = vp
        self.cairo.cairo_create.argtypes = [vp]
        self.cairo.cairo_destroy.argtypes = [vp]
        self.cairo.cairo_scale.argtypes = [vp, d, d]
        self.cairo.cairo_set_source_rgb.argtypes = [vp, d, d, d]
        self.cairo.cairo_paint.argtypes = [vp]

    def open_and_render(self, data, scale=2.0):
        """Wall and this thread's CPU for the open and for the middle page."""
        t0, c0 = time.monotonic_ns(), time.thread_time_ns()
        b = self.glib.g_bytes_new_static(data, len(data))
        doc = self.pop.poppler_document_new_from_bytes(b, None, None)
        self.glib.g_bytes_unref(b)
        if not doc:
            raise RuntimeError('poppler could not open the PDF')
        n = self.pop.poppler_document_get_n_pages(doc)
        t1, c1 = time.monotonic_ns(), time.thread_time_ns()
        page = self.pop.poppler_document_get_page(doc, n // 2)
        w, h = ctypes.c_double(), ctypes.c_double()
        self.pop.poppler_page_get_size(page, ctypes.byref(w), ctypes.byref(h))
        surface = self.cairo.cairo_image_surface_create(0, int(w.value * scale + 0.999), int(h.value * scale + 0.999))
        cr = self.cairo.cairo_create(surface)
        self.cairo.cairo_set_source_rgb(cr, 1.0, 1.0, 1.0)
        self.cairo.cairo_paint(cr)
        self.cairo.cairo_scale(cr, scale, scale)
        self.pop.poppler_page_render(page, cr)
        self.cairo.cairo_destroy(cr)
        self.cairo.cairo_surface_destroy(surface)
        self.gobject.g_object_unref(page)
        t2, c2 = time.monotonic_ns(), time.thread_time_ns()
        self.gobject.g_object_unref(doc)
        return {'open_wall_ms': (t1 - t0) / 1e6, 'open_cpu_ms': (c1 - c0) / 1e6, 'render_wall_ms': (t2 - t1) / 1e6, 'render_cpu_ms': (c2 - c1) / 1e6, 'pages': n}


# ------------------------------------------------------------------------------------------------ one document
class Skip(Exception):
    """A scenario that cannot run on this document here: the reason is the row."""


class Job:
    """What one document, format or diagram has of the run: its work directory, its caches and its rows."""

    def __init__(self, g, runner, ident):
        self.g, self.runner, self.ident = g, runner, ident
        self.c = g.c
        self.memo = {}
        self.problems = {}
        self.t_check = 0.0
        self.t_sleep = 0.0

    def sleep(self, seconds):
        time.sleep(seconds)
        self.t_sleep += seconds

    # -------------------------------- rows
    def key(self, scenario, variant, side, method, rnd):
        return f'{self.ident}|{scenario}|{variant or ""}|{side}|{method}|{rnd}'

    def have(self, scenario, variant, side, method, rnd):
        return self.key(scenario, variant, side, method, rnd) in self.g.done

    def clock_ok(self, method, m):
        args = self.g.args
        if method != 'timed' or m.get('load_before') is None:
            return False
        if args.max_load is not None and max(m['load_before'], m['load_after']) > args.max_load:
            return False
        return self.g.governor == 'performance' and (m.get('sibling_busy') is None or m['sibling_busy'] <= 0.1)

    def write(self, scenario, variant, side, method, rnd, m, **more):
        """A row, unless that row is there already."""
        k = self.key(scenario, variant, side, method, rnd)
        if k in self.g.done:
            return
        row = {'kind': 'm', 'key': k, 'id': self.ident, 'scenario': scenario, 'variant': variant, 'side': side, 'method': method, 'round': rnd, 'tier': self.g.args.tier, 'machine': self.g.machine, 'governor': self.g.governor, 'tree': self.g.tree, 'cpu': self.g.pinned}
        row.update(self.pins())
        s = self.g.side(side)
        if s is not None:
            row['binary_sha'] = s.sha
        row.update(m)
        row.update(more)
        row['clock_ok'] = self.clock_ok(method, m)
        self.g.sink.write(row)
        self.g.done.add(k)

    def pins(self):
        return {}

    def unavailable(self, scenario, variant, side, method, rnd, why):
        self.write(scenario, variant, side, method, rnd, {}, unavailable=why, check='unavailable')

    # -------------------------------- running
    def measured(self, argv, cwd, env, limit, stderr=None, merge=False):
        cpu = self.g.pinned
        sibling = self.g.sibling
        s0 = cpu_line(sibling) if sibling is not None else None
        load0 = os.getloadavg()[0]
        out = next((a.split('=', 1)[1] for a in argv if a.startswith(('-output-directory=', '-outdir='))), cwd)
        out = os.path.normpath(os.path.join(cwd, out))
        env = {k: v.replace('@OUT@', out) for k, v in env.items()}     # as run.py's run_proc does
        r = self.runner.run(argv, cwd, env, stderr=stderr, limit=limit, merge=merge)
        m = flat(r)
        m['load_before'] = round(load0, 2)
        m['load_after'] = round(os.getloadavg()[0], 2)
        m['limit_s'] = limit
        if s0 is not None:
            s1 = cpu_line(sibling)
            if s1 and s1[0] - s0[0] >= 10:
                m['sibling_busy'] = round(1 - (s1[1] - s0[1]) / (s1[0] - s0[0]), 3)
        return m

    def limit(self, seconds, passes=1):
        a = self.g.args
        return max(a.min_limit, a.limit_scale * seconds * passes)

    def verdict(self, fp, judge):
        """A verdict by the hash of what it is of."""
        v = self.memo.get(fp)
        t0 = time.monotonic()
        how = 'memo'
        if v is None:
            v = judge()
            self.memo[fp] = v
            how = v[1]
        self.t_check += time.monotonic() - t0
        return {'check': v[0], 'check_how': how, 'check_ms': round((time.monotonic() - t0) * 1e3, 1), **v[2]}


class DocJob(Job):
    def __init__(self, g, runner, row):
        super().__init__(g, runner, row['id'])
        self.row = row
        self.e = g.by_id.get(row['id'])
        self.meta = None
        self.w = None
        self.out = None
        self.closure = None
        self.seconds = row['minstr'] * 1e6 / g.rate / 1000.0
        self.kept = {}
        self.caches = {}

    def pins(self):
        c = self.closure or (None, None)
        return {'source_closure': c[0], 'tree_closure': c[1], 'pin_ok': c == (self.row.get('source_closure'), self.row.get('tree_closure'))}

    def scenario_list(self, method):
        g, row = self.g, self.row
        plan = g.plan.get(method, {})
        res = []
        for name in DOC_SCENARIOS:
            if name in ('pixels',):
                if method == 'timed' and row.get('pixels') and g.args.tier == 'standard':
                    res.append(name)
                continue
            most = plan.get(name, 0)
            if name not in plan or (most is not None and self.seconds > most):
                continue
            if row['kind'] == 'testfile' and name not in ('last-cold', 'warm'):
                continue
            if name == 'edit' and not (g.args.tier == 'full' or row.get('edit')):
                continue
            if method == 'timed' and g.args.tier == 'full' and row['kind'] == 'testfile':
                continue
            res.append(name)
        return [s for s in res if s in g.wanted]

    def run(self):
        c = self.c
        method = self.g.args.method_now
        names = self.scenario_list(method)
        if not names:
            return
        if self.e is None:
            for s in names:
                self.unavailable(s, None, '*', method, 0, 'not in the manifests here')
            return
        self.meta = c.load_valid_oracle(self.e)
        if self.meta is None and self.g.args.prepare:
            self.meta, _ = c.prepare_entry(self.e)
        if self.meta is None or c.effective_class(self.meta, self.e) != 'normal':
            for s in names:
                self.unavailable(s, None, '*', method, 0, 'no valid normal oracle here')
            return
        self.closure = census.closures(self.meta['inputs'], c)
        prior = self.g.prior_closure.get(self.ident)
        if prior and prior != list(self.closure):
            self.g.refused(self.ident)
            return
        self.doc = proflib.Doc(self.ident, self.e)
        self.doc.meta = self.meta
        self.job = self.meta['jobname']
        self.final = os.path.join(self.meta['dir'], 'final')
        self.w = c.new_workdir(self.meta.get('workdir_path'))
        self.out = os.path.join(self.w, 'o')
        self.cwd_before = c.names_of(self.e['cwd'])
        self.seed_note = None
        try:
            group_done = False
            for name in names:
                try:
                    if name in ('warm', 'hits', 'resume'):
                        if not group_done:
                            self.do_warm_group(method)
                        group_done = True
                    else:
                        getattr(self, 'do_' + name.replace('-', '_'))(method)
                except Skip as s:
                    self.note_skip(name, method, str(s))
        finally:
            shutil.rmtree(self.w, ignore_errors=True)

    def note_skip(self, scenario, method, why):
        self.unavailable(scenario, None, '*', method, 0, why)

    # -------------------------------- caches
    def cache(self, side, kind, env):
        """This engine's cache for this run kind on this document: the files of the engine's filled cache, by hard link, and a small job to have them hold in this environment."""
        key = (side.label, kind)
        if key in self.caches:
            return self.caches[key]
        d = os.path.join(self.w, 'c', side.label, kind)
        os.makedirs(d)
        link_tree(side.base_cache, d)
        self.prime(side, d, env)
        self.caches[key] = d
        return d

    def prime(self, side, cache, env):
        fmt = self.doc.format
        tmp = os.path.join(self.w, 'prime')
        os.makedirs(tmp, exist_ok=True)
        e = {k: v for k, v in env.items() if not k.startswith('RSTEX_CHECKPOINT')}
        e.update(XDG_CACHE_HOME=cache, RSTEX_NO_SNAPSHOT='1')
        # The path the measured run will have: a search path is part of what the index in the cache holds for, so with another one the first measured run makes a new index, up to 28% of a small document, and a second does not.
        e = {k: v.replace('@OUT@', self.out) for k, v in e.items()}
        self.runner.run([side.path, '-progname=' + fmt, '-fmt=' + side.fmt(fmt), '-interaction=batchmode', '-no-shell-escape', '-output-directory=' + tmp, '-jobname=rstexbenchprime', PRIME_LINE], tmp, e, limit=120)
        shutil.rmtree(tmp, ignore_errors=True)

    # -------------------------------- the state of a last pass
    def reset(self, keep=None):
        shutil.rmtree(self.out, ignore_errors=True)
        os.makedirs(self.out)
        self.c.restore_state(self.meta, self.out)
        if keep is not None and keep in self.kept:
            shutil.copyfile(self.kept[keep], os.path.join(self.out, self.job + '.pdf'))

    def keep_pdf(self, label):
        pdf = os.path.join(self.out, self.job + '.pdf')
        if os.path.isfile(pdf):
            d = os.path.join(self.w, 'kept-' + label + '.pdf')
            shutil.copyfile(pdf, d)
            self.kept[label] = d

    def cold_command(self, side, kind='cold', extra=None):
        c, e = self.c, self.e
        if side.kind == 'rstex':
            argv, cwd, env = side.engine.command(self.doc, self.out)
            env = dict(env, RSTEX_NO_SNAPSHOT='1', RSTEX_NO_PAGES='1')
            env.update(extra or {})
            env['XDG_CACHE_HOME'] = self.cache(side, kind, env)
            return argv, cwd, env
        env = side.env(c.entry_env(e))
        env.update(extra or {})
        return baselines.argv_of(c, e, side.label, self.out), e['cwd'], env

    # -------------------------------- the check against the oracle
    def not_rewritten(self):
        """run.py's not_rewritten for state restored with the oracle's own times: a side file the oracle wrote in its last pass that this run left alone."""
        c, meta = self.c, self.meta
        _, _, outputs = c.parse_fls(os.path.join(self.final, self.job + '.fls'))
        wd = (meta.get('workdir_path') or '') + '/'
        n = 0
        for path in outputs:
            rel = path[len(wd):] if path.startswith(wd) else None
            if rel and rel in meta['side_files']:
                try:
                    if os.stat(os.path.join(self.out, rel)).st_mtime_ns == os.stat(os.path.join(meta['dir'], 'pre_last', rel)).st_mtime_ns:
                        n += 1
                except OSError:
                    pass
        return n

    def check_oracle(self, m):
        """The verdicts of run.py compare, for the run that has just left its files in self.out."""
        c, e, meta, job, final = self.c, self.e, self.meta, self.job, self.final
        pdf = os.path.join(self.out, job + '.pdf')
        log = os.path.join(self.out, job + '.log')
        rc = m['rc']
        if m.get('timed_out'):
            return {'check': 'candidate-timeout', 'check_how': 'status'}
        if rc is not None and (rc < 0 or rc >= 100):
            return {'check': 'candidate-crash', 'check_how': 'status'}
        o_pdf = os.path.join(final, job + '.pdf')
        has_o = os.path.isfile(o_pdf) and os.path.getsize(o_pdf) > 0
        has_c = os.path.isfile(pdf) and os.path.getsize(pdf) > 0
        if has_o and not has_c:
            return {'check': 'candidate-crash', 'check_how': 'no-pdf'}
        side = c.snapshot_side(self.out, job)
        rewritten = self.not_rewritten()
        fp = (hash_bytes(pdf), hash_bytes(log), tuple(sorted(side.items())), rc, rewritten, tuple(sorted(c.names_of(e['cwd']) - self.cwd_before)))
        verdict = self.verdict(fp, lambda: self.judge_oracle(pdf, o_pdf, has_o, has_c, log, side, rewritten, rc))
        if has_c:
            verdict['pdf_bytes'] = os.path.getsize(pdf)
        return verdict

    def judge_oracle(self, pdf, o_pdf, has_o, has_c, log, side, rewritten, rc):
        c, e, meta, job = self.c, self.e, self.meta, self.job
        sev, how, det = [], 'none', {}
        if has_o and has_c and e['compare'] == 'pdf':
            v, how = compare_pdfs(o_pdf, pdf, self.problems)
            if v == 'crash':
                return 'candidate-crash', how, det
            if v == 'tool':
                return 'harness-error', how, det
            if v:
                sev.append(0 if v == 'content' else 1)
        elif has_o != has_c and e['compare'] == 'pdf':
            sev.append(0)
        stable = (meta.get('determinism') or {}).get('log_equal', True)
        la = c.judged_log(e, c.open_bytes(os.path.join(self.final, job + '.log')), [meta.get('workdir_path')])
        lb = c.judged_log(e, c.open_bytes(log), [self.out])
        ld = c.first_log_divergence(la, lb)
        det['log'] = 'equal' if ld is None else ('differs' if stable else 'differs-unstable')
        if ld is not None and stable:
            sev.append(0 if (e['gate_log'] or e['compare'] == 'log' or not has_o) else 2)
        diff = [k for k in meta['side_files'] if side.get(k) != meta['side_files'][k]] + [k for k in side if k not in meta['side_files']]
        det['side_diff'] = len(diff)
        if diff:
            sev.append(1)
        det['not_rewritten'] = rewritten
        if rewritten:
            sev.append(1)
        if c.names_of(e['cwd']) - self.cwd_before:
            det['scribbled'] = True
            sev.append(1)
        if meta['last_rc'] != rc:
            det['exit_differs'] = True
            sev.append(1)
        return ('identical' if not sev else {0: 'differs-content', 1: 'differs-structure-only', 2: 'differs-log-only'}[min(sev)]), how, det

    # -------------------------------- last-cold
    def do_last_cold(self, method):
        g = self.g
        if method == 'counted':
            order = rotated(g.sides, self.ident)
            for s in order:
                if self.have('last-cold', None, s.label, method, 0):
                    continue
                argv, cwd, env = self.cold_command(s)
                self.reset()
                m = self.measured(argv, cwd, env, self.limit(self.seconds))
                self.write('last-cold', None, s.label, method, 0, m, **self.check_oracle(m))
            return
        for rnd in range(g.rounds_for(self.seconds)):
            # Each rstex twice, the control, and each pdfTeX, in an order that turns round.
            seq = [(s, 'a1') for s in g.rstex] + [(s, None) for s in g.tex] + [(s, 'a2') for s in g.rstex]
            for s, variant in (seq if rnd % 2 == 0 else list(reversed(seq))):
                if self.have('last-cold', variant, s.label, method, rnd):
                    continue
                argv, cwd, env = self.cold_command(s)
                self.reset()
                if g.args.cold:
                    # As tests/corpus/run.py bench --cold: the page cache emptied of what a compile reads.  Keep these in a file of their own, as they are another measurement of the same row.
                    os.sync()
                    self.c.evict_from_page_cache(self.c.cold_closure(self.e, self.meta, argv[0]))
                m = self.measured(argv, cwd, env, self.limit(self.seconds))
                self.write('last-cold', variant, s.label, method, rnd, m, cold=g.args.cold, **self.check_oracle(m))

    # -------------------------------- warm, hits and resume: a checkpoint, a seal, what restores it
    def warm_env(self, side):
        argv, cwd, env = side.engine.command(self.doc, self.out)
        env = {k: v for k, v in env.items() if k != 'RSTEX_NO_SNAPSHOT'}
        env.update(RSTEX_CHECKPOINT_POLICY='always', RSTEX_NO_PAGES='1')
        env['XDG_CACHE_HOME'] = self.cache(side, 'warm', env)
        return argv, cwd, env

    def traced(self, argv, cwd, env, extra=None, limit=None):
        """A measured run that says what the checkpoint did.  The trace costs too little to count, and nothing of what it says is kept but whether and how much."""
        err = os.path.join(self.w, 'trace')
        m = self.measured(argv, cwd, dict(env, RSTEX_CHECKPOINT_TRACE='1', **(extra or {})), limit or self.limit(self.seconds), stderr=err)
        data = self.c.open_bytes(err)
        remove(err)
        m['restored'] = TRACE_RESTORED.search(data) is not None
        p = TRACE_PAGES.search(data)
        if p:
            m['records'], m['used_record'] = int(p.group(1)), int(p.group(2))
        # Why not, for a document that is not private, as tools/prof/warm.py does: a reason can name a file.
        if not m['restored'] and not is_private(self.ident):
            m['checkpoint_said'] = [line[len(b'rstex: checkpoint: '):].decode('latin-1')[:120] for line in data.splitlines() if line.startswith(b'rstex: checkpoint: ')][:2]
        return m

    def prep_warm(self, side, method, rows=False):
        """A write, then a seal: as seal_checkpoint in proflib says, a restore checks the image's sum until one has passed on a file old enough."""
        argv, cwd, env = self.warm_env(side)
        self.reset()
        m = self.measured(argv, cwd, env, self.limit(self.seconds))
        if rows:
            self.write('warm', 'write', side.label, method, 0, m, **self.check_oracle(m))
        self.sleep(2.1)
        self.reset()
        self.traced(argv, cwd, env)
        return argv, cwd, env

    def prep_warm_all(self, method):
        """The same for every binary, with one wait: in a serial run a wait per binary is minutes over a tier."""
        states = {}
        for s in self.g.rstex:
            argv, cwd, env = self.warm_env(s)
            self.reset()
            self.measured(argv, cwd, env, self.limit(self.seconds))
            states[s.label] = argv, cwd, env
        self.sleep(2.1)
        for s in self.g.rstex:
            argv, cwd, env = states[s.label]
            self.reset()
            self.traced(argv, cwd, env)
            self.keep_pdf(s.label)
        return states

    def do_warm_group(self, method):
        # warm, hits and resume go together per engine, as each builds on the last.
        g = self.g
        wanted = [s for s in ('warm', 'hits', 'resume') if s in self.scenario_list(method)]
        if method == 'counted':
            for s in rotated(g.rstex, self.ident):
                keys = [('warm', 'write'), ('warm', 'restore')] * ('warm' in wanted) + [('hits', None)] * ('hits' in wanted) + [('resume', 'write'), ('resume', 'last')] * ('resume' in wanted)
                if all(self.have(a, b, s.label, method, 0) for a, b in keys):
                    continue
                self.warm_counted(s, wanted, method)
            return
        states = self.prep_warm_all(method)
        for rnd in range(g.rounds_for(self.seconds)):
            seq = [(s, 'a1') for s in g.rstex] + [(s, 'a2') for s in g.rstex]
            for s, variant in (seq if rnd % 2 == 0 else list(reversed(seq))):
                argv, cwd, env = states[s.label]
                if 'warm' in wanted and not self.have('warm', variant, s.label, method, rnd):
                    self.reset()
                    m = self.traced(argv, cwd, env)
                    self.write('warm', variant, s.label, method, rnd, m, **self.check_oracle(m))
                    self.keep_pdf(s.label)
                if 'hits' in wanted and not self.have('hits', variant, s.label, method, rnd):
                    self.reset(keep=s.label)
                    m = self.traced(argv, cwd, env)
                    self.write('hits', variant, s.label, method, rnd, m, **self.check_oracle(m))
                    self.keep_pdf(s.label)

    def warm_counted(self, s, wanted, method):
        argv, cwd, env = self.prep_warm(s, method, 'warm' in wanted)
        self.reset()
        m = self.traced(argv, cwd, env)
        if 'warm' in wanted:
            self.write('warm', 'restore', s.label, method, 0, m, **self.check_oracle(m))
        self.keep_pdf(s.label)
        if 'hits' in wanted:
            self.reset(keep=s.label)
            m = self.traced(argv, cwd, env)
            self.write('hits', None, s.label, method, 0, m, **self.check_oracle(m))
            self.keep_pdf(s.label)
        if 'resume' in wanted:
            chained = {k: v for k, v in env.items() if k != 'RSTEX_NO_PAGES'}
            self.reset(keep=s.label)
            m = self.traced(argv, cwd, chained)
            self.write('resume', 'write', s.label, method, 0, m, **self.check_oracle(m))
            self.keep_pdf(s.label)
            self.reset(keep=s.label)
            m = self.traced(argv, cwd, chained, {'RSTEX_PAGE_READONLY': '1'})
            self.write('resume', 'last', s.label, method, 0, m, **self.check_oracle(m))

    # -------------------------------- a word in the body, the preamble restored where it can be
    def do_warm_word(self, method):
        """What `warm` does not say: its source is the one the checkpoint was made from, so a file a preamble only looked at has the same stat, which a save would change.  Here a word goes in the middle of the body, in a mirror, after what a user has by then, and each side makes one pass from
        the oracle's last-pass side files.  There is no oracle for this source, so all are held against Arch's pdfTeX on the same source at the same path."""
        g, e, c = self.g, self.e, self.c
        sides = [s.label for s in g.tex] + [s.label for s in g.rstex]
        if all(self.have('warm-word', None, x, method, 0) for x in sides):
            return
        if not g.arch:
            raise Skip('warm-word: no reference without the baseline arch')
        src, main_rel, _ = self.mirror_ready('warm-word')
        files, bib = self.own_files()
        plan = edits.plan(files, main_rel, 'word-middle', None, bib) if main_rel in dict(files) else None
        if plan is None:
            for x in sides:
                self.write('warm-word', None, x, method, 0, {}, applied=False, check='not-applicable')
            shutil.rmtree(src, ignore_errors=True)
            return
        name, edited = plan
        doc = proflib.Doc(self.ident, dict(e, cwd=src, main=os.path.join(src, main_rel)))
        doc.meta = self.meta
        limit = self.limit(self.seconds)
        states = {}
        for s in g.rstex:
            argv, cwd, env = s.engine.command(doc, self.out)
            env = self.texinputs_for({k: v for k, v in env.items() if k != 'RSTEX_NO_SNAPSHOT'}, src)
            env.update(RSTEX_CHECKPOINT_POLICY='always', RSTEX_NO_PAGES='1')
            env['XDG_CACHE_HOME'] = self.cache(s, 'warm-word', env)
            self.reset()
            self.measured(argv, cwd, env, limit)
            states[s.label] = argv, cwd, env
        self.sleep(2.1)
        for s in g.rstex:
            argv, cwd, env = states[s.label]
            self.reset()
            self.traced(argv, cwd, env)
        original = hash_bytes(os.path.join(os.path.realpath(e['cwd']), name))
        save(make_own(src, name), edited)
        if hash_bytes(os.path.join(os.path.realpath(e['cwd']), name)) != original:
            raise RuntimeError('an original changed: this must never happen')
        ref = os.path.join(self.w, 'word-ref')
        ref_m = None
        for s in sorted(g.tex, key=lambda s: s.label != 'arch'):
            self.reset()
            m = self.measured(baselines.argv_of(c, dict(e, cwd=src), s.label, self.out), src, self.texinputs_for(s.env(c.entry_env(e)), src), limit)
            if s is g.arch:
                ref_m = m
                shutil.rmtree(ref, ignore_errors=True)
                shutil.copytree(self.out, ref)
                self.write('warm-word', None, s.label, method, 0, m, applied=True, check='reference', check_how='reference')
            else:
                self.write('warm-word', None, s.label, method, 0, m, applied=True, **self.check_pair(ref, ref_m['rc'], self.out, m, reference_is_latexmk=False))
        for s in rotated(g.rstex, self.ident):
            argv, cwd, env = states[s.label]
            self.reset()
            m = self.traced(argv, cwd, env)
            self.write('warm-word', None, s.label, method, 0, m, applied=True, **self.check_pair(ref, ref_m['rc'], self.out, m, reference_is_latexmk=False))
        shutil.rmtree(ref, ignore_errors=True)
        shutil.rmtree(src, ignore_errors=True)

    # -------------------------------- native and the old map
    def do_native(self, method):
        if not any(os.path.basename(p) == 'tikz-cd.sty' for p in self.meta['inputs']):
            return      # a row that does not read tikz-cd.sty has no such row: no row, as the plan says
        rounds = 1 if method == 'counted' else self.g.rounds_for(self.seconds)
        for rnd in range(rounds):
            seq = [(s, 'native', None) for s in self.g.rstex] + [(s, 'pgf', {'RSTEX_NO_COMMUTE': '1'}) for s in self.g.rstex]
            for s, variant, extra in (seq if rnd % 2 == 0 else list(reversed(seq))):
                if self.have('native', variant, s.label, method, rnd):
                    continue
                argv, cwd, env = self.cold_command(s, 'native-' + variant, extra)
                self.reset()
                m = self.measured(argv, cwd, env, self.limit(self.seconds))
                self.write('native', variant, s.label, method, rnd, m, **self.check_oracle(m))

    def do_old_map(self, method):
        if not os.path.exists(baselines.OLD_MAP):
            raise Skip('no old map on this machine')
        for s in rotated(self.g.tex, self.ident):
            if self.have('old-map', None, s.label, method, 0):
                continue
            argv, cwd, env = self.cold_command(s, extra={'TEXFONTMAPS': baselines.OLD_MAP})
            self.reset()
            m = self.measured(argv, cwd, env, self.limit(self.seconds))
            verdict = self.check_oracle(m)
            if verdict.get('check') == 'differs-log-only':
                # The log names the map it read, and that map is another here.  The PDF is what says whether a document renders the same under it.
                verdict.update(check='identical', log='names another map')
            self.write('old-map', None, s.label, method, 0, m, **verdict)

    # -------------------------------- in a mirror
    def own_files(self):
        """The document's own text in the order its oracle read it, and its .bib files, as census.py's edit_kinds reads them: names under the directory of the document, nothing printed."""
        c, e, meta = self.c, self.e, self.meta
        main = e.get('main')
        base = os.path.realpath(e['cwd'])
        _, inputs, _ = c.parse_fls(os.path.join(self.final, self.job + '.fls'))
        only = census.group_of(e['id'], e['source'])[0] == 'stacks' and e['id'] != 'stacks/book'
        names = list(inputs)
        if main not in names:
            names.insert(0, main)
        files = []
        for p in names:
            ext = os.path.splitext(p)[1].lower()
            if census.is_tree(p, c) or ext not in ('.tex', '.ltx', '.dtx') or p.startswith(c.REAL_TARGET + '/corpus/w/') or (only and p != main):
                continue
            real = os.path.realpath(p)
            if not real.startswith(base + os.sep) or not os.path.isfile(real):
                continue
            with open(real, 'rb') as f:
                files.append((os.path.relpath(real, base), f.read()))
        bib = []
        for p in meta['inputs']:
            if p.endswith('.bib') and not census.is_tree(p, c) and os.path.isfile(p):
                with open(p, 'rb') as f:
                    bib.append(f.read())
        return files, bib

    def texinputs_for(self, env, src):
        """A search path into the document's directory goes to the mirror's, or an edit of a file found that way is not seen."""
        if 'TEXINPUTS' in env:
            env['TEXINPUTS'] = env['TEXINPUTS'].replace(self.e['cwd'], src)
        return env

    def latexmk_argv(self, exe, flags, main_rel):
        """latexmk over `exe`, as a document's prelude asks: \\pdfsetrandomseed, for what would differ from run to run."""
        pre = self.e.get('prelude')
        cmd = f'{exe} %O %P' if pre else f'{exe} %O %S'
        return ['latexmk', '-norc', '-pdf', '-pdflatex=' + cmd] + ([f'-usepretex={pre}'] if pre else []) + flags + ['-outdir=.build', './' + main_rel]

    def direct_tail(self, main_rel):
        """The same first line as latexmk gives it."""
        pre = self.e.get('prelude')
        if not pre:
            return ['./' + main_rel]
        return ['-jobname=' + self.e['jobname'], f'{pre}\\input{{./{main_rel}}}']

    def mirror_ready(self, what):
        e = self.e
        if not e.get('main') or not os.path.isfile(e['main']) or e.get('args') or e['mode'] not in ('latex', 'plain'):
            raise Skip(f'{what}: this entry has no plain command line')
        src = os.path.join(self.w, 'src')
        main_rel = mirror(e, src)
        return src, main_rel, mirror_dir_list(src)

    def reset_slot(self, slot, dirs):
        shutil.rmtree(under_target(slot), ignore_errors=True)
        os.makedirs(slot)
        for d in dirs:
            os.makedirs(os.path.join(slot, d), exist_ok=True)
        for d in self.e['mirror_dirs'] + self.meta.get('mirrored', []):
            os.makedirs(os.path.join(slot, d), exist_ok=True)
        self.c.seed_workdir(self.e, slot)

    def run_tool(self, tool, file, src, out, env, limit):
        """What the line says, in the output directory: as tests/until_stable/common.py's run_tool."""
        name = os.path.relpath(os.path.join(src, os.fsdecode(file)), out)
        if tool == 'makeindex':
            argv = ['makeindex', '-q', name]
        elif tool == 'bibtex':
            argv = ['bibtex', name[:-4] if name.endswith('.aux') else name]
        else:
            argv = ['biber', '--quiet', name]
        e = dict(env, BIBINPUTS=src + ':', BSTINPUTS=src + ':')
        m = self.measured(argv, out, e, limit)
        return m, m['rc'] <= (1 if tool == 'bibtex' else 0)

    def run_direct(self, side, argv, src, out, env, limit):
        """rstex --until-stable until it says stop, running what it names, as tests/until_stable/compare.py does."""
        total, passes, tools = None, 0, {}
        err = os.path.join(self.w, 'stderr')
        for _ in range(8):
            m = self.measured(argv, src, env, limit, stderr=err)
            total = add(total, m)
            data = self.c.open_bytes(err)
            remove(err)
            done = PASSES_LINE.search(data)
            passes += int(done.group(1)) if done else 0
            if m['rc'] != STATUS_TOOL or m['timed_out']:
                break
            for tool, _, file in TOOL_LINE.findall(data):
                tool = tool.decode()
                tm, ok = self.run_tool(tool, file, src, out, env, limit)
                tools[tool] = tools.get(tool, 0) + 1
                rc = total['rc']
                total = add(total, tm)
                total['rc'] = rc
                if not ok:
                    total.update(rc=3, tool_failed=tool)
                    return total, passes, tools
        total.update(load_before=m['load_before'], load_after=m['load_after'])
        return total, passes, tools

    def run_latexmk(self, argv, src, env, limit):
        err = os.path.join(self.w, 'stderr')
        m = self.measured(argv, src, env, limit, stderr=err, merge=True)
        data = self.c.open_bytes(err)
        remove(err)
        rules = [x.decode() for x in RULE_RUNS.findall(data)]
        tools = {}
        for x in rules:
            if not x.endswith('latex'):
                tools[x] = tools.get(x, 0) + 1
        return m, sum(1 for x in rules if x.endswith('latex')), tools

    def check_pair(self, ref, ref_rc, cand, m, reference_is_latexmk=True):
        """`cand` against `ref`, at one path while each ran: the PDF, the log, the side files and whether it ended well, with none of it kept."""
        c, e, job = self.c, self.e, self.job
        if m.get('timed_out'):
            return {'check': 'candidate-timeout', 'check_how': 'status'}
        rc = m['rc']
        if rc is not None and (rc < 0 or rc >= 100) and not m.get('tool_failed'):
            return {'check': 'candidate-crash', 'check_how': 'status'}
        a, b = os.path.join(ref, job + '.pdf'), os.path.join(cand, job + '.pdf')
        has_a, has_b = os.path.isfile(a) and os.path.getsize(a) > 0, os.path.isfile(b) and os.path.getsize(b) > 0
        if has_a and not has_b:
            return {'check': 'candidate-crash', 'check_how': 'no-pdf'}
        sa, sb = snapshot(ref, job), snapshot(cand, job)
        if reference_is_latexmk:
            sa = latexmk_only(sa, sb, job)
        fp = (hash_bytes(a), hash_bytes(b), hash_bytes(os.path.join(ref, job + '.log')), hash_bytes(os.path.join(cand, job + '.log')), tuple(sorted(sa.items())), tuple(sorted(sb.items())), rc == 0, ref_rc == 0)

        def judge():
            sev, how, det = [], 'none', {}
            if has_a and has_b:
                v, how = compare_pdfs(a, b, self.problems)
                if v == 'crash':
                    return 'candidate-crash', how, det
                if v == 'tool':
                    return 'harness-error', how, det
                if v:
                    sev.append(0 if v == 'content' else 1)
            elif has_a != has_b:
                sev.append(0)
            # One path while each ran, so nothing of it differs but what the program wrote.
            la = c.judged_log(e, c.open_bytes(os.path.join(ref, job + '.log')), [])
            lb = c.judged_log(e, c.open_bytes(os.path.join(cand, job + '.log')), [])
            ld = c.first_log_divergence(la, lb)
            det['log'] = 'equal' if ld is None else 'differs'
            if ld is not None:
                sev.append(2)
            diff = [k for k in set(sa) | set(sb) if sa.get(k) != sb.get(k)]
            det['side_diff'] = len(diff)
            if diff:
                sev.append(1)
            if (rc == 0) != (ref_rc == 0):
                det['exit_differs'] = True
                sev.append(1)
            return ('identical' if not sev else {0: 'differs-content', 1: 'differs-structure-only', 2: 'differs-log-only'}[min(sev)]), how, det
        verdict = self.verdict(fp, judge)
        if has_b:
            verdict['pdf_bytes'] = os.path.getsize(b)
        return verdict

    def keep_wrong(self, verdict, what, ref, cand):
        """--keep-wrong, for a public document: what the two sides left, under target/, for someone to look at."""
        d = self.g.args.keep_wrong
        if not d or is_private(self.ident) or verdict.get('check') in ('identical', 'reference', 'not-applicable', 'not-comparable'):
            return
        base = os.path.join(d, re.sub(r'[^A-Za-z0-9_.-]', '_', self.ident), what)
        for name, src in (('ref', ref), ('cand', cand)):
            shutil.copytree(src, os.path.join(base, name), dirs_exist_ok=True, symlinks=True)

    # -------------------------------- first
    def do_first(self, method):
        g, e, c = self.g, self.e, self.c
        sides = [s.label for s in g.tex] + [s.label for s in g.rstex]
        if all(self.have('first', None, x, method, 0) for x in sides):
            return
        src, main_rel, dirs = self.mirror_ready('first')
        slot = os.path.join(src, '.build')
        done = {}
        base = self.texinputs_for(document_env(e), src)
        fmt = self.doc.format
        passes_of_oracle = self.meta.get('passes_run') or 1
        limit = self.limit(self.seconds, max(passes_of_oracle, 3) + 3)
        for s in rotated(g.sides, self.ident):
            self.reset_slot(slot, dirs)
            if s.kind == 'pdftex':
                env = s.env(dict(base))
                if e['mode'] == 'latex':
                    argv = self.latexmk_argv(shlex.quote(s.latex_exe()), FIRST_FLAGS, main_rel)
                    m, passes, tools = self.run_latexmk(argv, src, env, limit)
                else:
                    # No latexmk for plain TeX: the passes the oracle took.
                    argv = [s.latex_exe('pdftex')] + FIRST_FLAGS + ['-output-directory=.build'] + self.direct_tail(main_rel)
                    m, passes, tools = None, 0, {}
                    for _ in range(passes_of_oracle):
                        m = add(m, self.measured(argv, src, env, limit))
                        passes += 1
            else:
                cache = self.cache(s, 'first', base)
                env = dict(base, XDG_CACHE_HOME=cache)
                if e['mode'] == 'latex':
                    argv = [s.path, '-progname=' + fmt, '-fmt=' + s.fmt(fmt), '--until-stable'] + FIRST_FLAGS + ['-output-directory=.build'] + self.direct_tail(main_rel)
                    m, passes, tools = self.run_direct(s, argv, src, slot, env, limit)
                else:
                    # Plain TeX again: the same passes, as tests/bench/README.md has it, so that the two logs are of the same pass.
                    argv = [s.path, '-progname=' + fmt, '-fmt=' + s.fmt(fmt)] + FIRST_FLAGS + ['-output-directory=.build'] + self.direct_tail(main_rel)
                    m, passes, tools = None, 0, {}
                    for _ in range(passes_of_oracle):
                        m = add(m, self.measured(argv, src, env, limit))
                        passes += 1
            kept = os.path.join(self.w, 'first-' + s.label)
            shutil.rmtree(kept, ignore_errors=True)
            shutil.move(slot, kept)
            done[s.label] = (kept, m, passes, tools)
        ref_label = 'arch' if 'arch' in done else g.tex[0].label if g.tex else None
        for label, (kept, m, passes, tools) in done.items():
            extra = dict(passes=passes, tools=tools)
            if label == ref_label:
                # What the others are held against, held against the oracle in its turn.  Their paths and their logs differ, and a biblatex document's oracle has no biber: this says, and does not judge.
                a, b = os.path.join(self.final, self.job + '.pdf'), os.path.join(kept, self.job + '.pdf')
                against = None
                if os.path.isfile(a) and os.path.isfile(b):
                    against = 'same' if compare_pdfs(a, b, self.problems)[0] is None else 'differs'
                self.write('first', None, label, method, 0, m, check='reference', check_how='reference', against_oracle=against, **extra)
            elif ref_label is None:
                self.write('first', None, label, method, 0, m, check='no-reference', **extra)
            else:
                ref = done[ref_label]
                verdict = self.check_pair(ref[0], ref[1]['rc'], kept, m)
                self.keep_wrong(verdict, 'first-' + label, ref[0], kept)
                self.write('first', None, label, method, 0, m, **verdict, **extra)
        for kept, _, _, _ in done.values():
            shutil.rmtree(kept, ignore_errors=True)
        shutil.rmtree(src, ignore_errors=True)

    # -------------------------------- edit: a sitting at the editor
    def do_edit(self, method):
        g, e = self.g, self.e
        sides = [s.label for s in g.rstex] + (['arch'] if g.arch else [])
        keys = [(k, x) for k in ('settle',) + edits.KINDS for x in sides]
        if all(self.have('edit', k, x, method, 0) for k, x in keys):
            return
        if not g.arch:
            raise Skip('edit: no reference without the baseline arch')
        src, main_rel, dirs = self.mirror_ready('edit')
        files, bib = self.own_files()
        names = [n for n, _ in files]
        if main_rel not in names:
            raise Skip('edit: the main file is not among those its oracle read')
        current = dict(files)
        originals = {n: hash_bytes(os.path.join(os.path.realpath(e['cwd']), n)) for n in names}
        base = self.texinputs_for(document_env(e), src)
        fmt = self.doc.format
        limit = self.limit(self.seconds, 8)
        run_dir = os.path.join(src, '.build')
        plain = e['mode'] == 'plain'
        builders = []
        for s in [g.arch] + g.rstex:
            slot = os.path.join(src, '.slot-' + s.label)
            self.reset_slot(slot, dirs)
            if s.kind == 'pdftex':
                env = s.env(dict(base))
                if plain:
                    argv = [s.latex_exe('pdftex')] + LEMMA_FLAGS + ['-output-directory=.build'] + self.direct_tail(main_rel)
                else:
                    argv = self.latexmk_argv(shlex.quote(s.latex_exe()), LEMMA_FLAGS, main_rel)
            else:
                env = dict(base, XDG_CACHE_HOME=self.cache(s, 'edit', base))
                argv = [s.path, '-progname=' + fmt, '-fmt=' + s.fmt(fmt)] + ([] if plain else ['--until-stable']) + LEMMA_FLAGS + ['-output-directory=.build'] + self.direct_tail(main_rel)
            builders.append({'s': s, 'slot': slot, 'argv': argv, 'env': env, 'rc0': None, 'pdf': None})

        def build(b, passes=1):
            os.rename(b['slot'], run_dir)
            try:
                if plain:
                    # No latexmk for plain TeX: one pass of each to a save, and the oracle's number of them to begin with.
                    total = None
                    for _ in range(passes):
                        total = add(total, self.measured(b['argv'], src, b['env'], limit))
                    return total, passes, {}
                if b['s'].kind == 'pdftex':
                    return self.run_latexmk(b['argv'], src, b['env'], limit)
                return self.run_direct(b['s'], b['argv'], src, run_dir, b['env'], limit)
            finally:
                os.rename(run_dir, b['slot'])

        def compare_and_write(kind, results, more):
            ref = builders[0]
            for b in builders:
                m, passes, tools = results[b['s'].label]
                pdf = os.path.join(b['slot'], self.job + '.pdf')
                sha = hash_bytes(pdf)
                extra = dict(more, passes=passes, tools=tools, pdf_changed=None if b['pdf'] is None else sha != b['pdf'])
                b['pdf'] = sha
                if b is ref:
                    self.write('edit', kind, b['s'].label, method, 0, m, check='reference', check_how='reference', **extra)
                    continue
                verdict = self.check_pair(ref['slot'], results[ref['s'].label][0]['rc'], b['slot'], m)
                if more.get('no_pass_on_reference') and verdict['check'] != 'identical':
                    verdict['check'] = 'not-comparable'
                self.keep_wrong(verdict, f'edit-{kind}-{b["s"].label}', ref['slot'], b['slot'])
                st = synctex_lines(os.path.join(b['slot'], self.job + '.synctex.gz'))
                sr = synctex_lines(os.path.join(ref['slot'], self.job + '.synctex.gz'))
                verdict['synctex'] = None if sr is None else 'same' if st == sr else 'differs'
                self.write('edit', kind, b['s'].label, method, 0, m, **verdict, **extra)

        # From nothing to where it settles, then what a user has from the second or third save on: the checkpoint, and its seal.
        results = {b['s'].label: build(b, self.meta.get('passes_run') or 1) for b in rotated(builders, self.ident)}
        for b in builders:
            b['rc0'] = results[b['s'].label][0]['rc']
        compare_and_write('settle', results, {'applied': True})
        # A document that ends in an error stops after one pass on both sides, without having settled, and latexmk with nothing changed does not run again where rstex does.  So for rstex these runs leave
        # their caches and not their outputs there, or a later save would hold passes of different number against each other: what a table of contents says depends on how many came before.
        # Plain TeX takes one pass to a save on each side, and the same goes for it.
        with_errors = builders[0]['rc0'] != 0
        for b in builders[1:]:
            kept = b['slot'] + '.kept'
            if with_errors or plain:
                shutil.copytree(b['slot'], kept, symlinks=True)
            build(b)
            self.sleep(2.2)
            build(b)
            if with_errors or plain:
                shutil.rmtree(b['slot'])
                os.rename(kept, b['slot'])
        for kind in edits.KINDS:
            # A resumed session saves each edit again, rows there or not: what comes after has to be what it was.
            aux = self.c.open_bytes(os.path.join(builders[0]['slot'], self.job + '.aux')) or None
            plan = edits.plan([(n, current[n]) for n in names], main_rel, kind, aux, bib)
            if plan is None:
                for x in sides:
                    self.write('edit', kind, x, method, 0, {}, applied=False, check='not-applicable')
                continue
            name, new = plan
            path = make_own(src, name)
            before = current[name]
            save(path, new)
            results = {b['s'].label: build(b) for b in rotated(builders, self.ident + kind)}
            ref_failed = results[builders[0]['s'].label][0]['rc'] != builders[0]['rc0']
            if ref_failed:
                # What the edit broke is no matter here: back to where it was, and counted.
                for x in sides:
                    self.write('edit', kind, x, method, 0, {'rc': results[x][0]['rc']}, applied=True, taken_back=True, check='taken-back')
                save(path, before)
                for b in builders:
                    build(b)
                continue
            current[name] = new
            # latexmk with nothing changed ran no pass after an error, where rstex did: if those two differ, it says nothing of rstex.
            compare_and_write(kind, results, {'applied': True, 'no_pass_on_reference': True} if with_errors and not plain and kind == 'noop' else {'applied': True})
        for n, sha in originals.items():
            if hash_bytes(os.path.join(os.path.realpath(e['cwd']), n)) != sha:
                raise RuntimeError('an original changed: this must never happen')
        shutil.rmtree(src, ignore_errors=True)

    # -------------------------------- pixels: from the save to the pixels
    def do_pixels(self, method):
        g, e = self.g, self.e
        try:
            pop = Poppler()
        except OSError:
            raise Skip('pixels: no Poppler here')
        if not g.arch:
            raise Skip('pixels: no reference without the baseline arch')
        if e['mode'] != 'latex':
            raise Skip('pixels: this measures rstex behind latexmk, and latexmk is no way to run plain TeX')
        src, main_rel, dirs = self.mirror_ready('pixels')
        files, bib = self.own_files()
        plan = edits.plan(files, main_rel, 'sentence', None, bib) if main_rel in dict(files) else None
        if plan is None:
            raise Skip('pixels: no place for the edit in this document')
        name, edited = plan
        path = make_own(src, name)
        original = dict(files)[name]
        base = self.texinputs_for(document_env(e), src)
        fmt = self.doc.format
        limit = self.limit(self.seconds, 8)
        run_dir = os.path.join(src, '.build')
        for s in g.rstex:
            configs = []
            for variant in ('direct', 'direct-aa', 'latexmk'):
                slot = os.path.join(src, f'.slot-{s.label}-{variant}')
                self.reset_slot(slot, dirs)
                env = dict(base, XDG_CACHE_HOME=self.cache(s, 'pixels-' + variant, base))
                if variant == 'latexmk':
                    argv = self.latexmk_argv(f'{shlex.quote(s.path)} -progname={fmt} -fmt={shlex.quote(s.fmt(fmt))}', LEMMA_FLAGS, main_rel)
                else:
                    argv = [s.path, '-progname=' + fmt, '-fmt=' + s.fmt(fmt), '--until-stable'] + LEMMA_FLAGS + ['-output-directory=.build'] + self.direct_tail(main_rel)
                configs.append((variant, slot, argv, env))

            def build(cfg):
                variant, slot, argv, env = cfg
                os.rename(slot, run_dir)
                try:
                    if variant == 'latexmk':
                        return self.measured(argv, src, env, limit)
                    return self.run_direct(s, argv, src, run_dir, env, limit)[0]
                finally:
                    os.rename(run_dir, slot)
            refs = {}
            for state, data in (('edited', edited), ('original', original)):
                slot = os.path.join(src, f'.slot-ref-{state}')
                if not os.path.isdir(slot):
                    self.reset_slot(slot, dirs)
                    save(path, data)
                    os.rename(slot, run_dir)
                    try:
                        self.run_latexmk(self.latexmk_argv(shlex.quote(g.arch.latex_exe()), LEMMA_FLAGS, main_rel), src, g.arch.env(dict(base)), limit)
                    finally:
                        os.rename(run_dir, slot)
                refs[state] = slot
            # What a user has from the second or third save on, as tools/prof/loop.py has it.
            for data in (edited, original, edited, original):
                save(path, data)
                for cfg in configs:
                    build(cfg)
            self.sleep(2.2)
            for data in (edited, original):
                save(path, data)
                for cfg in configs:
                    build(cfg)
            state = 0
            for rnd in range(PIXEL_ROUNDS):
                state ^= 1
                t0 = time.monotonic_ns()
                save(path, edited if state else original)
                save_ms = (time.monotonic_ns() - t0) / 1e6
                order = rnd % len(configs)
                for cfg in configs[order:] + configs[:order]:
                    m = build(cfg)
                    verdict = self.check_pair(refs['edited' if state else 'original'], 0, cfg[1], m)
                    self.write('pixels', cfg[0], s.label, method, rnd, m, **verdict)
                t0 = time.monotonic_ns()
                with open(os.path.join(configs[0][1], self.job + '.pdf'), 'rb') as f:
                    data = f.read()
                read_ms = (time.monotonic_ns() - t0) / 1e6
                p = pop.open_and_render(data)
                self.write('pixels', 'poppler', s.label, method, rnd, {'load_before': round(os.getloadavg()[0], 2), 'load_after': round(os.getloadavg()[0], 2)}, save_ms=round(save_ms, 2), read_ms=round(read_ms, 2), check='not-applicable', **{k: round(v, 2) for k, v in p.items()})
            save(path, original)
            for variant, slot, _, _ in configs:
                shutil.rmtree(slot, ignore_errors=True)
        shutil.rmtree(src, ignore_errors=True)


class FmtJob(Job):
    """pdflatex.fmt built, from an empty directory, on each side.  Its bytes are nobody's to compare, as run.py says: it has to exist, and the log to agree."""

    def __init__(self, g, runner):
        super().__init__(g, runner, 'fmt/pdflatex')

    def run(self):
        g = self.g
        method = g.args.method_now
        c = self.c
        rounds = 1 if method == 'counted' else FMT_ROUNDS[g.args.tier]
        root = os.path.join(proflib.out_root(), 'bench-fmt', f'{os.getpid()}')
        for rnd in range(rounds):
            seq = list(g.tex) + list(g.rstex)
            logs = {}
            rows = []
            for s in (seq if rnd % 2 == 0 else list(reversed(seq))):
                if self.have('fmt', None, s.label, method, rnd):
                    continue
                d = os.path.join(root, 'o')
                shutil.rmtree(d, ignore_errors=True)
                os.makedirs(d)
                if s.kind == 'rstex':
                    argv, env = proflib.format_command(s.engine, 'pdflatex'), proflib.format_env(s.engine)
                else:
                    env = s.env()
                    env.update(c.DETERMINISM, **c.NO_MKTEX, **c.HERMETIC)
                    argv = [s.latex_exe('pdftex'), '-ini', '-jobname=pdflatex', '-progname=pdflatex'] + baselines.RECIPES['pdflatex']
                m = self.measured(argv, d, env, 900)
                logs[s.label] = c.normalize_log(c.open_bytes(os.path.join(d, 'pdflatex.log')), [d])
                ok = m['rc'] == 0 and os.path.isfile(os.path.join(d, 'pdflatex.fmt'))
                rows.append((s, m, ok))
            ref = logs.get('arch')
            for s, m, ok in rows:
                if s.label == 'arch':
                    check = 'reference' if ok else 'candidate-crash'
                elif not ok:
                    check = 'candidate-crash'
                elif ref is None:
                    check = 'no-reference'
                else:
                    check = 'identical' if c.first_log_divergence(ref, logs[s.label]) is None else 'differs-log-only'
                self.write('fmt', None, s.label, method, rnd, m, check=check, check_how='log')
        shutil.rmtree(root, ignore_errors=True)


class DiagramJob(Job):
    """A figure for a paper: standalone commute against a `standalone` document with real tikz-cd.  Their files are not the same bytes, one being plain with base fonts: the check is the page's size, its text in order and its pixels."""

    def __init__(self, g, runner, row):
        super().__init__(g, runner, row['entry'] + '#' + row['unit'])
        self.row = row

    def pins(self):
        return {'pin_ok': True, 'stratum': self.row.get('stratum')}

    def run(self):
        g, c = self.g, self.c
        method = g.args.method_now
        e = g.by_id.get(self.row['entry'])
        sides = ['commute'] + [s.label for s in g.tex]
        if e is None or not e.get('main') or not os.path.isfile(e['main']):
            for x in sides:
                self.unavailable('diagram-alone', None, x, method, 0, 'not in the manifests here')
            return
        rounds = 1
        if all(self.have('diagram-alone', None, x, method, r) for x in sides for r in range(rounds)):
            return
        body = None
        for m in census.DIAGRAM.finditer(c.open_bytes(e['main'])):
            if m.group(1).decode() == self.row['unit']:
                body = m.group(2)
                break
        if body is None:
            for x in sides:
                self.unavailable('diagram-alone', None, x, method, 0, 'no such diagram in its entry')
            return
        w = c.new_workdir()
        try:
            source = os.path.join(w, 'd.tex')
            with open(source, 'wb') as f:
                f.write(census.STANDALONE % body)
            env0 = {k: os.environ[k] for k in ('PATH', 'HOME') if k in os.environ}
            env0.update(c.DETERMINISM, **c.NO_MKTEX, **c.HERMETIC)
            rows, pdfs = [], {}
            for rnd in range(rounds):
                seq = list(g.tex) + ['commute']
                for s in (seq if rnd % 2 == 0 else list(reversed(seq))):
                    d = os.path.join(w, 'o')
                    shutil.rmtree(d, ignore_errors=True)
                    os.makedirs(d)
                    label = s if s == 'commute' else s.label
                    if s == 'commute':
                        if not g.commute:
                            self.unavailable('diagram-alone', None, 'commute', method, rnd, 'no --commute given')
                            continue
                        m = self.measured([g.commute.path, '-o', d, '--stem', 'd', '--pdf', 'd.tex'], w, env0, 120)
                    else:
                        m = self.measured([s.latex_exe(), '-interaction=batchmode', '-no-shell-escape', '-output-directory=' + d, 'd.tex'], w, s.env(dict(env0)), 300)
                    pdf = os.path.join(d, 'd.pdf')
                    keep = os.path.join(w, f'{label}.pdf')
                    if os.path.isfile(pdf):
                        shutil.move(pdf, keep)
                        pdfs[label] = keep
                    rows.append((label, rnd, m))
            ref = pdfs.get('arch')
            for label, rnd, m in rows:
                if label == 'arch':
                    self.write('diagram-alone', None, label, method, rnd, m, check='reference' if ref else 'candidate-crash', pdf_bytes=os.path.getsize(ref) if ref else None)
                elif label == 'commute' and m['rc'] == 1:
                    self.write('diagram-alone', None, label, method, rnd, m, check='declined')
                elif label not in pdfs:
                    self.write('diagram-alone', None, label, method, rnd, m, check='candidate-crash')
                elif ref is None:
                    self.write('diagram-alone', None, label, method, rnd, m, check='no-reference')
                else:
                    self.write('diagram-alone', None, label, method, rnd, m, check=self.same_picture(ref, pdfs[label]), check_how='picture', pdf_bytes=os.path.getsize(pdfs[label]))
        finally:
            shutil.rmtree(w, ignore_errors=True)

    @staticmethod
    def page_size(path):
        o = subprocess.run(['pdfinfo', path], capture_output=True, text=True).stdout
        m = re.search(r'Page size:\s+([\d.]+) x ([\d.]+)', o)
        return (float(m.group(1)), float(m.group(2))) if m else None

    @staticmethod
    def page_text(path):
        o = subprocess.run(['pdftotext', '-layout', path, '-'], capture_output=True).stdout
        return re.sub(rb'\s+', b' ', o).strip()

    @staticmethod
    def raster(path):
        return subprocess.run(['pdftoppm', '-r', str(RAST_DPI), path], capture_output=True).stdout

    @classmethod
    def same_picture(cls, a, b):
        sa, sb = cls.page_size(a), cls.page_size(b)
        if sa is None or sb is None or max(abs(sa[0] - sb[0]), abs(sa[1] - sb[1])) > 0.002:
            return 'differs-content'
        if cls.page_text(a) != cls.page_text(b):
            return 'differs-content'
        x, y = cls.raster(a), cls.raster(b)
        if len(x) != len(y):
            return 'differs-content'
        return 'same-picture' if all(abs(p - q) <= RAST_LEVEL for p, q in zip(x, y)) else 'differs-picture'


class StressJob(DocJob):
    """`side_stress`: last-cold alone, in `full`, on how things scale: never in a stratum and never pooled."""

    def scenario_list(self, method):
        return ['last-cold'] if method == 'counted' else []


def remove(path):
    try:
        os.unlink(path)
    except OSError:
        pass


def rotated(items, salt):
    items = list(items)
    if not items:
        return items
    k = int(hashlib.sha256(salt.encode()).hexdigest(), 16) % len(items)
    return items[k:] + items[:k]


def link_tree(src, dst):
    for name in os.listdir(src):
        p = os.path.join(src, name)
        q = os.path.join(dst, name)
        if os.path.isdir(p):
            os.makedirs(q, exist_ok=True)
            link_tree(p, q)
        else:
            os.link(p, q)


# ------------------------------------------------------------------------------------------------ the run
class Sink:
    """One line, one write, with O_APPEND: several workers append, and a line is short enough not to be cut in two by another."""

    def __init__(self, path):
        self.fd = os.open(path, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o644)

    def write(self, row):
        os.write(self.fd, (json.dumps(row, sort_keys=True) + '\n').encode())


class Commute:
    def __init__(self, path):
        self.path = os.path.realpath(path)
        if not os.path.isfile(self.path):
            die('no such binary for --commute')
        self.sha = proflib.sha_file(self.path)[:20]
        self.label = 'commute'
        self.kind = 'commute'


class Global:
    """What every job shares, set before the workers fork."""

    def __init__(self, args):
        self.args = args
        self.c = proflib.corpus()
        self.done = set()
        self.prior_closure = {}
        self.pinned = None
        self.sibling = None
        self.sink = None
        self.machine = f'{os.uname().nodename}, {cpu_model()}'
        self.governor = governor(0)
        self.refused_ids = None
        self.rstex, self.tex = [], []
        self.commute = None
        self.arch = None

    def side(self, label):
        for s in self.rstex + self.tex:
            if s.label == label:
                return s
        return self.commute if label == 'commute' else None

    @property
    def sides(self):
        return self.tex + self.rstex

    def rounds_for(self, seconds):
        for most, n in self.plan.get('rounds', [[None, 1]]):
            if most is None or seconds <= most:
                return n
        return 1

    def refused(self, ident):
        with open(self.args.out + '.refused', 'a') as f:
            f.write(ident + '\n')


def load_set(args):
    """The set's rows for a tier, and its plan."""
    with open(args.set) as f:
        public = json.load(f)
    entries = {e['id']: e for e in public['entries'] if args.tier in e['tiers']}
    private = 0
    if not args.no_private and os.path.exists(args.set_local):
        with open(args.set_local) as f:
            local = json.load(f)
        for e in local['entries']:
            if args.tier in e['tiers']:
                entries[e['id']] = e
                private += 1
    return public, entries, private


def thinned_plan(public, args):
    return report.thin_plan(public['plan'][args.tier], args.tier, args.thin)


def read_rows(path):
    """The rows in a file, after dropping a line a kill cut short."""
    rows = []
    if not os.path.exists(path):
        return rows
    with open(path, 'rb') as f:
        data = f.read()
    if data and not data.endswith(b'\n'):
        cut = data.rfind(b'\n') + 1
        with open(path, 'r+b') as f:
            f.truncate(cut)
        data = data[:cut]
    for line in data.split(b'\n'):
        if line:
            rows.append(json.loads(line))
    return rows


def refuse_other_binaries(rows, sides):
    """A row from another binary under a side's name would mix two programs in one figure."""
    for r in rows:
        s = sides.get(r.get('side'))
        if r.get('kind') == 'm' and s is not None and r.get('binary_sha') and r['binary_sha'] != s:
            die(f'{r["side"]}: the file holds rows of another binary ({r["binary_sha"]}, this one is {s}).  Refused: name it otherwise, or use another --out')


def worker(k, g, jobs, counter, lock, cpus):
    ctypes.CDLL(None).prctl(1, signal.SIGKILL)
    if cpus:
        os.sched_setaffinity(0, {cpus[k % len(cpus)]})
    g.c.install_limits()
    os.nice(g.args.nice)
    runner = Runner()
    while True:
        with lock:
            i = counter.value
            counter.value += 1
        if i >= len(jobs):
            break
        cls, arg = jobs[i]
        ident = arg['id'] if 'id' in arg else arg.get('entry', 'fmt')
        t0 = time.monotonic()
        try:
            if cls == 'doc':
                job = DocJob(g, runner, arg)
            elif cls == 'stress':
                job = StressJob(g, runner, arg)
            elif cls == 'diagram':
                job = DiagramJob(g, runner, arg)
            else:
                job = FmtJob(g, runner)
            if g.args.method_now == 'timed':
                proflib.wait_for_load(g.args.max_load, g.args.load_patience)
            job.run()
            g.sink.write({'kind': 'job', 'id': ident, 'method': g.args.method_now, 'seconds': round(time.monotonic() - t0, 2), 'check_s': round(job.t_check, 2), 'sleep_s': round(job.t_sleep, 2)})
            if not g.args.quiet:
                print(f'[{i + 1}/{len(jobs)}] {g.args.method_now} {ident}: {time.monotonic() - t0:.1f} s, check {job.t_check:.1f} s', flush=True)
        except Exception as ex:     # noqa: BLE001
            # One document's failure is one document's row: a private document's says the class and no more.
            why = type(ex).__name__
            g.sink.write({'kind': 'harness-error', 'id': ident, 'method': g.args.method_now, 'error': why, 'seconds': round(time.monotonic() - t0, 1)})
            if not is_private(ident):
                traceback.print_exc()
            print(f'[{i + 1}/{len(jobs)}] {ident}: harness error, {why}', flush=True)


def make_jobs(g, entries, public, method):
    args = g.args
    pattern = re.compile(args.ids) if args.ids else None
    jobs = []
    for i, e in sorted(entries.items(), key=lambda kv: (kv[1]['minstr'], kv[0])):
        if not pattern or pattern.search(i):
            jobs.append(('doc', e))
    if args.tier == 'full' and 'side-stress' in g.wanted and method == 'counted':
        for i in public.get('side_stress', ()):
            if not pattern or pattern.search(i):
                jobs.append(('stress', {'id': i, 'kind': 'stress', 'size': 'stress', 'minstr': 0, 'source_closure': None, 'tree_closure': None, 'tiers': ['full']}))
    if 'fmt' in g.wanted and (not pattern or pattern.search('fmt/pdflatex')):
        jobs.append(('fmt', {'id': 'fmt/pdflatex'}))
    if 'diagram-alone' in g.wanted:
        d = public.get('diagrams', {})
        rows = d.get(args.tier) if args.tier != 'full' else full_diagrams(public)
        for row in rows or ():
            if not pattern or pattern.search(row['entry']):
                jobs.append(('diagram', row))
    return jobs


def full_diagrams(public):
    """`full` has every usable diagram of a family not kept apart.  That needs what census.py made of them: the rule, by import, on that file."""
    path = os.path.join(REPO, 'target', 'bs-design', 'diagrams.jsonl')
    if not os.path.exists(path):
        die('the full tier needs target/bs-design/diagrams.jsonl, from census.py diagrams')
    select = proflib.load_script('bench_select', os.path.join(BENCH, 'select.py'))
    with open(path) as f:
        rows = [json.loads(line) for line in f]
    usable, _, _, _ = select.pick_diagrams(rows, set(public['families_kept_apart']), set())
    return [{'entry': d['entry'], 'unit': d['unit'], 'stratum': d['stratum']} for d in usable if not d['apart']]


def prepare_sides(g, jobs):
    """Formats, filled caches and what they cost: all before any worker starts."""
    fmts = set()
    c = g.c
    for cls, arg in jobs:
        if cls in ('doc', 'stress'):
            e = g.by_id.get(arg['id'])
            if e and e['mode'] in ('latex', 'plain'):
                fmts.add(e.get('fmt') or ('pdflatex' if e['mode'] == 'latex' else 'pdftex'))
    fmts |= {'pdflatex'}
    proflib.integration().FORMAT_RECIPES.setdefault('latex', ['-ini', '-jobname=latex', '-progname=latex', '-translate-file=cp227.tcx', '*latex.ini'])
    info = {}
    runner = Runner()
    probe = next((e for i, e in g.by_id.items() if i == 'ladder/00-plain-hello'), None)
    for s in g.tex:
        s.ensure_fmt(sorted(fmts))
        info[s.label] = {'sha': s.sha, 'kind': s.kind, 'formats': {n: s.fmt_sha(n) for n in sorted(fmts)}}
    for s in g.rstex:
        formats = {}
        for n in sorted(fmts):
            t0 = time.monotonic()
            path = s.fmt(n)
            formats[n] = {'sha': proflib.sha_file(path)[:20], 'seconds': round(time.monotonic() - t0, 2)}
        # A cache filled once for this binary, for each format's program name, in a document's environment.
        base = os.path.join(proflib.out_root(), 'bench-cache', s.engine.sha[:16])
        os.makedirs(base, exist_ok=True)
        s.base_cache = base
        cost = {}
        tmp = os.path.join(proflib.out_root(), 'bench-cache', f'tmp-{os.getpid()}')
        env = c.entry_env(probe) if probe else {k: os.environ[k] for k in ('PATH', 'HOME') if k in os.environ}
        env['RSTEX_NO_SNAPSHOT'] = '1'
        for n in sorted(fmts):
            argv = [s.path, '-progname=' + n, '-fmt=' + s.fmt(n), '-interaction=batchmode', '-no-shell-escape', '-output-directory=' + tmp, '-jobname=rstexbenchprime', PRIME_LINE]
            os.makedirs(os.path.join(tmp, 'empty'))
            r = runner.run(argv, tmp, dict(env, XDG_CACHE_HOME=os.path.join(tmp, 'empty')), limit=300)
            cost[n] = round(r['counters']['instructions'][0] / 1e6, 1) if 'instructions' in r['counters'] else None
            shutil.rmtree(tmp, ignore_errors=True)
            os.makedirs(tmp)
            runner.run(argv, tmp, dict(env, XDG_CACHE_HOME=base), limit=300)
            shutil.rmtree(tmp, ignore_errors=True)
        info[s.label] = {'sha': s.sha, 'kind': s.kind, 'formats': formats, 'empty_cache_minstr': cost}
    # What a run costs that does nothing, and what a measured peak cannot go below.
    floor = flat(runner.run(['/bin/true'], '/', {}))
    runner.p.stdin.close()
    runner.p.wait()
    return info, {'maxrss_kb': floor['maxrss_kb'], 'wall_ms': floor['wall_ms'], 'instructions': floor['instructions']}


def run_cmd(args):
    t_start = time.monotonic()
    if args.tier == 'holdout' and not args.frozen:
        die('the hold-out is measured once, on the frozen engine, after the report of `standard` is written: tests/bench/README.md.  Say --frozen if that is now')
    args.out = os.path.abspath(args.out or os.path.join(proflib.out_root(), 'bench', args.tier + '.jsonl'))
    if '/target/' not in os.path.realpath(args.out):
        die('--out has to be under a directory called target')
    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    lock_file = open(args.out + '.lock', 'w')
    try:
        fcntl.flock(lock_file, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        die('another run holds this file')
    g = Global(args)
    public, entries, private = load_set(args)
    g.rate = public['pc_rate_instr_per_ms']
    g.plan = thinned_plan(public, args)
    g.wanted = set(args.scenarios.split(',')) if args.scenarios else set(SCENARIOS)
    bad = g.wanted - set(SCENARIOS)
    if bad:
        die(f'no such scenario: {sorted(bad)}')
    entries_all, _, _ = g.c.load_manifests(None)
    g.by_id = {e['id']: e for e in entries_all}
    for spec in args.engine:
        label, _, path = spec.partition('=')
        if not path:
            die('--engine takes NAME=PATH')
        if label in ('arch', 'official', 'commute', '*'):
            die(f'{label} is a name kept for the baselines')
        g.rstex.append(Rstex(label, path))
    g.tex = [PdfTex(b) for b in args.baselines.split(',') if b]
    g.arch = next((s for s in g.tex if s.label == 'arch'), None)
    g.commute = Commute(args.commute) if args.commute else None
    if not g.rstex and not g.tex:
        die('no engine and no baseline')
    tree = census.tree_info()
    g.tree = tree_hash(tree)
    tree_pin = public.get('tree', {})
    rows = read_rows(args.out)
    sides = {s.label: s.sha for s in g.rstex + g.tex}
    if g.commute:
        sides['commute'] = g.commute.sha
    refuse_other_binaries(rows, sides)
    for r in rows:
        if r.get('kind') == 'm':
            g.done.add(r['key'])
            if r.get('source_closure'):
                g.prior_closure[r['id']] = [r['source_closure'], r.get('tree_closure')]
    g.sink = Sink(args.out)
    methods = ['counted', 'timed'] if args.method == 'both' else [args.method]
    missing = [t for t in SIDE_TOOLS if not shutil.which(t)]
    all_jobs = {m: make_jobs(g, entries, public, m) for m in methods}
    side_info, floor = prepare_sides(g, [j for m in methods for j in all_jobs[m]])
    head = {'kind': 'run', 'tier': args.tier, 'seed': public.get('seed'), 'started_s': round(time.time()), 'set_sha': proflib.sha_file(args.set)[:20], 'set_local': private, 'machine': g.machine, 'cpus': os.cpu_count(), 'governor': g.governor, 'load': round(os.getloadavg()[0], 2), 'sides': side_info,
            'tree': g.tree, 'tree_is_pin': g.tree == tree_hash(tree_pin), 'tree_detail': {k: tree.get(k) for k in TREE_CORE + ('pdflatex_fmt_sha', 'pdftex_map_lines', 'packages')}, 'floor': floor, 'methods': methods, 'scenarios': sorted(g.wanted), 'jobs': args.jobs, 'cpu': args.cpu, 'cpus_list': args.cpus, 'max_load': args.max_load,
            'thin': args.thin, 'timed_engines': args.timed_engines, 'missing_tools': missing, 'commute': g.commute.sha if g.commute else None, 'plan': g.plan, 'resumed_rows': len(g.done)}
    g.sink.write(head)
    if not args.quiet:
        print(f'{args.tier}: {len(entries)} documents ({private} private), {len(g.done)} rows there already, sides {", ".join(sides)}, tree {g.tree}{"" if head["tree_is_pin"] else " (NOT the tree the set was made on)"}, load {head["load"]}', flush=True)
    ctx = multiprocessing.get_context('fork')
    for method in methods:
        args.method_now = method
        jobs = all_jobs[method]
        counter, lock = ctx.Value('i', 0), ctx.Lock()
        if method == 'timed':
            cpu = args.cpu if args.cpu is not None else max(os.sched_getaffinity(0))
            g.pinned, g.sibling = cpu, sibling_of(cpu)
            g.governor = governor(cpu)
            cpus, n = [cpu], 1
            if args.timed_engines:
                names = args.timed_engines.split(',')
                if set(names) - {s.label for s in g.rstex}:
                    die('--timed-engines names a binary this run has not')
                g.rstex = [s for s in g.rstex if s.label in names]
        else:
            g.pinned, g.sibling = None, None
            cpus = [int(x) for x in args.cpus.split(',')] if args.cpus else None
            n = args.jobs
        workers = [ctx.Process(target=worker, args=(k, g, jobs, counter, lock, cpus)) for k in range(n)]
        for w in workers:
            w.start()
        for w in workers:
            w.join()
    elapsed = time.monotonic() - t_start
    usage = os.times()
    g.sink.write({'kind': 'end', 'elapsed_s': round(elapsed, 1), 'children_cpu_s': round(usage.children_user + usage.children_system, 1), 'load': round(os.getloadavg()[0], 2)})
    refused = 0
    if os.path.exists(args.out + '.refused'):
        with open(args.out + '.refused') as f:
            refused = len(set(f.read().split()))
    if not args.quiet:
        print(f'done in {elapsed:.0f} s; {refused} documents refused for another input hash')
    return 3 if refused else 0


def plan_cmd(args):
    """Which rows a run would make, by scenario, from the set alone."""
    args.out = '/dev/null'
    g = Global(args)
    public, entries, private = load_set(args)
    g.rate = public['pc_rate_instr_per_ms']
    g.plan = thinned_plan(public, args)
    g.wanted = set(args.scenarios.split(',')) if args.scenarios else set(SCENARIOS)
    entries_all, _, _ = g.c.load_manifests(None)
    g.by_id = {e['id']: e for e in entries_all}
    for method in ('counted', 'timed'):
        args.method_now = method
        count = {}
        for i, e in entries.items():
            job = DocJob(g, None, e)
            for s in job.scenario_list(method):
                count[s] = count.get(s, 0) + 1
        print(f'{args.tier}, {method}: ' + ', '.join(f'{k} {v}' for k, v in sorted(count.items())))
    print(f'{len(entries)} documents, {private} private; diagrams {len(public.get("diagrams", {}).get(args.tier, ()) or ())}')


def preflight_cmd(args):
    """What would come out as rows that say they could not run, or as rows off the pins, an hour into a run.  Read only: a missing oracle is only counted."""
    args.out = '/dev/null'
    c = proflib.corpus()
    public, entries, private = load_set(args)
    entries_all, _, _ = c.load_manifests(None)
    by_id = {e['id']: e for e in entries_all}
    missing, no_oracle, off_pin, fine = [], [], [], 0
    for i, row in sorted(entries.items()):
        e = by_id.get(i)
        if e is None:
            missing.append(i)
            continue
        meta = c.load_valid_oracle(e)
        if meta is None or c.effective_class(meta, e) != 'normal':
            no_oracle.append(i)
            continue
        if census.closures(meta['inputs'], c) != (row.get('source_closure'), row.get('tree_closure')):
            off_pin.append(i)
            continue
        fine += 1
    tree = census.tree_info()
    same = tree_hash(tree) == tree_hash(public.get('tree', {}))
    print(f'{args.tier}: {len(entries)} documents, {private} of them private: {fine} ready, {len(missing)} not in the manifests here, {len(no_oracle)} with no valid normal oracle here, {len(off_pin)} on another input or tree than the pin')
    for label, ids in (('not in the manifests', missing), ('no valid normal oracle: run with --prepare, or prepare them', no_oracle), ('on another closure', off_pin)):
        if ids:
            print(f'  {label}: ' + ' '.join(ids[:40]) + (' ...' if len(ids) > 40 else ''))
    print(f'tree {tree_hash(tree)}: {"the one the set was made on" if same else "NOT the one the set was made on: " + ", ".join(k for k in TREE_CORE if tree.get(k) != public.get("tree", {}).get(k))}')
    print(f'official pdfTeX: {"here" if os.path.isfile(baselines.OFFICIAL_BIN) else "MISSING"}; old map: {"here" if os.path.exists(baselines.OLD_MAP) else "MISSING"}; programs missing: {", ".join(t for t in SIDE_TOOLS if not shutil.which(t)) or "none"}')
    diagrams_ok = args.tier != 'full' or os.path.exists(os.path.join(REPO, 'target', 'bs-design', 'diagrams.jsonl'))
    if not diagrams_ok:
        print('MISSING target/bs-design/diagrams.jsonl, which the full tier reads for its diagrams: a run stops at the start without it')
    if not private and os.path.exists(args.set_local) is False:
        print('no set.local.json here: the private part will not run, and a report has no such part')
    try:
        Poppler()
        print('Poppler: here')
    except OSError:
        print('Poppler: MISSING, so no pixels')
    print(f'load {os.getloadavg()[0]:.2f}, governor {governor(0)}, {os.cpu_count()} CPUs, {cpu_model()}')
    return 0 if fine == len(entries) and same and diagrams_ok else 1


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest='cmd', required=True)
    for name in ('run', 'plan', 'preflight'):
        p = sub.add_parser(name)
        p.add_argument('--tier', required=True, choices=('quick', 'standard', 'holdout', 'full'))
        p.add_argument('--set', default=os.path.join(BENCH, 'set.json'))
        p.add_argument('--set-local', default=os.path.join(BENCH, 'set.local.json'))
        p.add_argument('--no-private', action='store_true')
        p.add_argument('--scenarios')
        p.add_argument('--thin', help='a thinned plan, by name, from tests/bench/report.py')
        if name == 'run':
            p.add_argument('--engine', action='append', default=[], metavar='NAME=PATH')
            p.add_argument('--commute')
            p.add_argument('--baselines', default='arch,official')
            p.add_argument('--method', choices=('counted', 'timed', 'both'), default='both')
            p.add_argument('--ids', help='only documents whose manifest id this matches')
            p.add_argument('-j', '--jobs', type=int, default=4)
            p.add_argument('--cpus', help='counted: pin each worker to one of these, a comma-separated list')
            p.add_argument('--cpu', type=int, help='timed: the one core; by default the last this process may use')
            p.add_argument('--max-load', type=float, default=1.5, help='timed: wait for the one-minute load to fall under this; a row over it has no clock figures')
            p.add_argument('--load-patience', type=float, default=300)
            p.add_argument('--timed-engines', help='timed: only these binaries, by name, separated by commas; the others are counted alone')
            p.add_argument('--nice', type=int, default=0)
            p.add_argument('--out')
            p.add_argument('--prepare', action='store_true', help='make a missing oracle: without it nothing is written to the oracle cache, and a document without one is a row that says so')
            p.add_argument('--min-limit', type=float, default=300, help='seconds a run may take at the least')
            p.add_argument('--limit-scale', type=float, default=40, help='seconds a run may take, as a multiple of the oracle\'s pass on the PC')
            p.add_argument('--keep-wrong', help='a directory under target/ for what each side left where a public document\'s rows differ, in `first` and `edit`')
            p.add_argument('--frozen', action='store_true', help='for the hold-out: this is the frozen engine, and the one time')
            p.add_argument('--cold', action='store_true', help='timed, last-cold: empty the page cache of what a compile reads before each run.  Use a file of its own')
            p.add_argument('--quiet', action='store_true')
    args = ap.parse_args()
    sys.exit({'run': run_cmd, 'plan': plan_cmd, 'preflight': preflight_cmd}[args.cmd](args))


if __name__ == '__main__':
    main()
