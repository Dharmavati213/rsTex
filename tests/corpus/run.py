#!/usr/bin/env -S python3 -I
"""run.py -- differential corpus runner for rstex against the pdfTeX oracle.

    run.py list      [selection]
    run.py prepare   [selection] [--force] [--retry-oracle-fails] [--run-id NAME]
    run.py compare   [selection] --engine PATH [--engine-arg ARG]... [--mode last-pass|from-scratch|both]
                     [--raster all|none] [--dpi N] [--no-l6] [--no-l1] [--fls] [--box-log never|failed|always]
                     [--no-gate-log] [--bringup] [--retry-oracle-fails] [--resume [RUN]] [--run-id NAME] [--timeout-scale X]
    run.py smoke     --engine PATH [--manifest FILE]... [--filter REGEX] [--timeout-cap S] [--timeout-scale X] [-j N]
    run.py boxlog    ID --engine PATH [--engine-arg ARG]... [--manifest FILE]... [--timeout-scale X]
    run.py bench     [selection] --engine PATH [--engine-arg ARG]... [-n N] [--max-load L] [--load-patience S]
                     [--cpu K] [--allow-unverified] [--cold] [--all-passes] [--keep-engine-cache]
                     [--resume [RUN]] [--run-id NAME] [--timeout-scale X]
    run.py coverage  [selection] [--trace-sample N | --trace-all | --no-trace] [--seed S] [--primitives FILE] [--out FILE]
    run.py report    [RUN] [--runs-dir DIR]
    run.py selfcheck [-j N]

    selection: [--manifest FILE]... [--tier smoke|standard|full] [--tier-exact] [--source S]... [--filter REGEX]
               [--only-failed-last-run] [-j N] [--runs-dir DIR] [--disk-budget-gb G]

The manifest schema, stage semantics and result categories are in tests/corpus/README.md.
Standard library only, plus calls to pdftex/pdflatex, bibtex, makeindex, qpdf, pdftoppm and
tests/harness/compare_pdf.py.  Exit status of compare: 0 when every comparable entry is identical, 1 otherwise.
"""
import argparse
import collections
import concurrent.futures as cf
import fcntl
import glob
import hashlib
import json
import math
import os
import random
import re
import resource
import secrets
import select
import shutil
import signal
import statistics
import subprocess
import sys
import threading
import time
import traceback
import zlib

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import difftoken  # noqa: E402
import lognorm  # noqa: E402

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
# A worktree links target/ to the main tree's.  An entry's paths count as the main tree's in the key of its oracle, so that worktrees share one oracle,
# as they share the cache, and do not each make their own and throw out the others'.  The real path, because a work directory's length shows in how the log wraps.
REAL_TARGET = os.path.realpath(os.path.join(REPO, 'target'))
MAIN_ROOT = os.path.dirname(REAL_TARGET)
# RSTEX_CORPUS_TARGET gives a run its own oracle cache.
TARGET = os.environ.get('RSTEX_CORPUS_TARGET') or os.path.join(REAL_TARGET, 'corpus')
ORACLE_ROOT = os.path.join(TARGET, 'oracle')
WORK_ROOT = os.path.join(TARGET, 'w')
DEFAULT_RUNS = os.path.join(TARGET, 'runs')
MANIFEST_DIR = os.path.join(REPO, 'tests', 'corpus', 'manifests')
FIXTURES = os.path.join(REPO, 'tests', 'corpus', 'fixtures')
COMPARE_PDF = os.path.join(REPO, 'tests', 'harness', 'compare_pdf.py')
INVENTORY_FALLBACK = ('/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/'
                      'scratchpad/design/verification/inv/inventory.json')

SCHEMA = 1                     # bump to invalidate every cached oracle
DEFAULT_JOBS = 4
DEFAULT_TIMEOUT = 300          # seconds per TeX pass
TIERS = ('smoke', 'standard', 'full')
MODES = ('latex', 'plain', 'ini', 'ini-format')
CLASSES = ('normal', 'oracle-fails', 'nondeterministic')
OUTCOMES = ('identical', 'differs-content', 'differs-structure-only', 'differs-log-only', 'candidate-crash', 'candidate-timeout',
            'known-divergence', 'oracle-fails', 'nondeterministic', 'harness-error', 'excluded', 'skipped')
NOT_COMPARABLE = ('known-divergence', 'oracle-fails', 'nondeterministic', 'excluded', 'skipped')
# The outcomes that can be accepted through known_divergences.txt: the document ran and gave another answer.  A crash or a timeout never is.
ACCEPTABLE = ('differs-content', 'differs-structure-only', 'differs-log-only')
KNOWN_DIVERGENCES = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'known_divergences.txt')
ENTRY_KEYS = {'id', 'tier', 'source', 'mode', 'fmt', 'main', 'first_line', 'prelude', 'jobname', 'cwd', 'texinputs',
              'env', 'args', 'bib', 'index', 'seed_files', 'passes', 'max_passes', 'timeout', 'expected_class',
              'trusted', 'l6', 'gate_log', 'compare', 'mirror_dirs', 'excluded', 'notes', 'judge_failure', 'units',
              'log_normalize', 'index_style'}
DETERMINISM = {'SOURCE_DATE_EPOCH': '1700000000', 'FORCE_SOURCE_DATE': '1', 'TZ': 'UTC', 'LC_ALL': 'C', 'LANG': 'C'}
# mktex* is off: rstex does not port it (DESIGN section 3), and generating fonts would write outside the work dir.
NO_MKTEX = {'MKTEXTFM': '0', 'MKTEXPK': '0', 'MKTEXMF': '0', 'MKTEXTEX': '0', 'MKTEXFMT': '0'}
# Nothing from the user's own trees.  A cmr10.tfm that mktextfm dropped in ~/.texlive shadows the distribution's for every later run, with other bytes, and
# no oracle cache can see that: the closure hash only knows the files a run did read.  These three are empty, always.
EMPTY_TEXMF = os.path.join(TARGET, 'empty-texmf')
HERMETIC = {'TEXMFHOME': EMPTY_TEXMF, 'TEXMFVAR': EMPTY_TEXMF, 'TEXMFCONFIG': EMPTY_TEXMF}
BOX_PRELUDE = '\\tracingoutput=1 \\showboxbreadth=2147483647 \\showboxdepth=2147483647 '
L6_PRELUDE = '\\pdfcompresslevel=0 \\pdfobjcompresslevel=0 \\pdftrailerid{} '
# L1 in the runner: what a macro or register became, at each assignment and each restore at the end of a group.  A meaning that never
# reaches the page shows here and nowhere else.  The full macro trace stays on demand: it is 50 times the size.
L1_PRELUDE = '\\tracingassigns=1 \\tracingrestores=1 '
TRACE_PRELUDE = '\\tracingcommands=2 '
FILE_LIMIT = 2 << 30           # RLIMIT_FSIZE inherited by every child: a runaway document cannot fill the disk
CPU_LIMIT = 3600               # RLIMIT_CPU: bounds a TeX orphaned by SIGKILL of this runner
SIDE_EXT = {'.aux', '.toc', '.lof', '.lot', '.out', '.bbl', '.ind', '.idx', '.nav', '.snm', '.vrb', '.bcf', '.glo', '.gls'}
TDS_PREFIXES = ('/usr/share/texmf-dist/', '/usr/share/texmf/', '/var/lib/texmf/', '/etc/texmf/',
                '/usr/local/share/texmf/', os.path.expanduser('~/.texlive/'), os.path.expanduser('~/texmf/'))
# RSTEX_ORACLE_DIR: a TeX Live of its own as the oracle, where its binaries are, as tests/integration/run.py takes it.  Its programs come first on PATH and its tree counts as the distribution's.
# That needs RSTEX_CORPUS_TARGET too, so that the main oracle cache stays clean, and known_divergences.txt is not read: its reasons are about Arch's pdfTeX, and every difference is to be seen.
SECOND_ORACLE = os.environ.get('RSTEX_ORACLE_DIR')
if SECOND_ORACLE:
    if not os.environ.get('RSTEX_CORPUS_TARGET'):
        sys.exit('run.py: RSTEX_ORACLE_DIR needs a RSTEX_CORPUS_TARGET of its own')
    os.environ['PATH'] = SECOND_ORACLE + os.pathsep + os.environ.get('PATH', '')
    TDS_PREFIXES += (os.path.dirname(os.path.dirname(os.path.realpath(SECOND_ORACLE))) + '/',)


class ManifestError(Exception):
    pass


# ------------------------------------------------------------------------------------------------ small helpers
_hash_cache = {}


def hash_file(path):
    """sha256 of a file, or 'missing'.  Cached on (path, size, mtime) for the life of the process."""
    try:
        st = os.stat(path)
    except OSError:
        return 'missing'
    if not os.path.isfile(path):
        return 'not-a-file'
    key = (path, st.st_size, st.st_mtime_ns)
    got = _hash_cache.get(key)
    if got:
        return got
    h = hashlib.sha256()
    try:
        with open(path, 'rb') as f:
            for chunk in iter(lambda: f.read(1 << 20), b''):
                h.update(chunk)
    except OSError:
        return 'unreadable'
    _hash_cache[key] = h.hexdigest()
    return _hash_cache[key]


def sha_text(s):
    return hashlib.sha256(s.encode('utf-8', 'surrogateescape')).hexdigest()


def write_json(path, obj):
    tmp = f'{path}.tmp{os.getpid()}.{threading.get_ident()}'
    with open(tmp, 'w') as f:
        json.dump(obj, f, indent=1, sort_keys=True)
    os.replace(tmp, path)


def read_json(path):
    with open(path) as f:
        return json.load(f)


def slug(entry_id):
    return re.sub(r'[^A-Za-z0-9._+-]+', '_', entry_id)[:80] + '-' + hashlib.sha1(entry_id.encode()).hexdigest()[:8]


def clip(b, n=240):
    b = b or ''
    if isinstance(b, bytes):
        b = b.decode('latin-1')
    b = ''.join(c if c.isprintable() else '.' for c in b)
    return b if len(b) <= n else b[:n] + '...'


def now():
    return time.strftime('%Y-%m-%dT%H:%M:%S%z')


def log_progress(msg):
    print(msg, file=sys.stderr, flush=True)


def new_workdir(like=None):
    """Every TeX run happens in WORK_ROOT/<16 hex>/o, so all output paths have equal length: TeX wraps its log at
    the same columns on both sides, and masking the path afterwards leaves comparable lines.  A worktree reaches WORK_ROOT through a
    link, and its path would be longer, so the real one is used.  like: an oracle's output path, of what length this has to be, for an oracle
    that a worktree made before this was so."""
    root = os.path.realpath(WORK_ROOT)
    n = max(16, len(like) - len(root) - 3) if like else 16
    w = os.path.join(root, secrets.token_hex((n + 1) // 2)[:n])
    os.makedirs(os.path.join(w, 'o'))
    return w


def purge_old_workdirs(age=86400):
    try:
        for name in os.listdir(WORK_ROOT):
            p = os.path.join(WORK_ROOT, name)
            if time.time() - os.stat(p).st_mtime > age:
                shutil.rmtree(p, ignore_errors=True)
    except OSError:
        pass


def install_limits():
    """Limits inherited by children.  The soft CPU limit also applies to this process, so SIGXCPU gets a Python
    handler here; a handler is reset to the default at exec, which means a runaway child dies on it."""
    hard_f = resource.getrlimit(resource.RLIMIT_FSIZE)[1]
    resource.setrlimit(resource.RLIMIT_FSIZE, (FILE_LIMIT, hard_f))
    hard_c = resource.getrlimit(resource.RLIMIT_CORE)[1]
    resource.setrlimit(resource.RLIMIT_CORE, (0, hard_c))
    signal.signal(signal.SIGXCPU, lambda *_: None)
    signal.signal(signal.SIGXFSZ, signal.SIG_DFL)
    hard_t = resource.getrlimit(resource.RLIMIT_CPU)[1]
    resource.setrlimit(resource.RLIMIT_CPU, (CPU_LIMIT, hard_t))


# ------------------------------------------------------------------------------------------------ manifests
def resolve_path(p, base=REPO):
    if p is None:
        return None
    p = os.path.expanduser(p)
    p = os.path.normpath(p if os.path.isabs(p) else os.path.join(base, p))
    # A worktree links refs/ and target/, and a log prints the paths under them: an entry's texinputs may lie there.  Those wrap where the oracle's did, and read as the oracle's do, only if they are the main tree's.
    for link in ('refs', 'target'):
        top = os.path.join(REPO, link) + os.sep
        if p.startswith(top):
            return os.path.join(os.path.realpath(top), p[len(top):])
    return p


def normalize_entry(raw, defaults, manifest_path):
    e = {**defaults, **raw}
    where = f'{manifest_path}: entry {raw.get("id", "?")!r}'
    unknown = set(e) - ENTRY_KEYS
    if unknown:
        raise ManifestError(f'{where}: unknown fields {sorted(unknown)}')
    if not isinstance(e.get('id'), str) or not e['id']:
        raise ManifestError(f'{where}: id is required')
    if not e.get('source'):
        raise ManifestError(f'{where}: source is required (entry field, manifest "source", or defaults)')
    e.setdefault('tier', 'standard')
    e.setdefault('mode', 'latex')
    e.setdefault('bib', 'none')
    e.setdefault('index', 'none')
    e.setdefault('passes', 'auto')
    e.setdefault('max_passes', 6)
    e.setdefault('timeout', DEFAULT_TIMEOUT)
    e.setdefault('expected_class', 'normal')
    e.setdefault('trusted', False)
    e.setdefault('l6', False)
    e.setdefault('gate_log', False)
    e.setdefault('judge_failure', False)
    e.setdefault('args', [])
    e.setdefault('env', {})
    e.setdefault('texinputs', [])
    e.setdefault('seed_files', {})
    e.setdefault('mirror_dirs', [])
    e.setdefault('prelude', '')
    e.setdefault('compare', 'log' if e['mode'] in ('ini', 'ini-format') else 'pdf')
    if e['tier'] not in TIERS:
        raise ManifestError(f'{where}: tier must be one of {TIERS}')
    if e['mode'] not in MODES:
        raise ManifestError(f'{where}: mode must be one of {MODES}')
    if e['expected_class'] not in CLASSES:
        raise ManifestError(f'{where}: expected_class must be one of {CLASSES}')
    if e['compare'] not in ('pdf', 'log'):
        raise ManifestError(f'{where}: compare must be pdf or log')
    if e.setdefault('log_normalize', 'default') not in ('default', 'l3build'):
        raise ManifestError(f'{where}: log_normalize must be default or l3build')
    if not isinstance(e['judge_failure'], bool):
        raise ManifestError(f'{where}: judge_failure must be true or false')
    if 'units' in e and not (isinstance(e['units'], list) and all(isinstance(u, str) for u in e['units'])):
        raise ManifestError(f'{where}: units must be a list of strings')
    if not (e['passes'] == 'auto' or (isinstance(e['passes'], int) and e['passes'] >= 1)):
        raise ManifestError(f'{where}: passes must be "auto" or a positive integer')
    if not (e['bib'] in ('none', 'auto', 'bibtex') or (isinstance(e['bib'], dict) and set(e['bib']) == {'bbl'})):
        raise ManifestError(f'{where}: bib must be "none", "auto", "bibtex" or {{"bbl": PATH}}')
    if e['index'] not in ('none', 'makeindex'):
        raise ManifestError(f'{where}: index must be "none" or "makeindex"')
    if not e.get('main') and not e.get('first_line'):
        raise ManifestError(f'{where}: one of main, first_line is required')
    if e.get('first_line') and not e.get('jobname'):
        raise ManifestError(f'{where}: first_line needs jobname')
    if e.get('first_line') and not e.get('cwd'):
        raise ManifestError(f'{where}: first_line needs cwd')
    if e.get('main'):
        e['main'] = resolve_path(e['main'])
    e['cwd'] = resolve_path(e['cwd']) if e.get('cwd') else os.path.dirname(e['main'])
    if e.get('main'):
        rel = os.path.relpath(e['main'], e['cwd'])
        if rel.startswith('..') and not e['trusted']:
            raise ManifestError(f'{where}: main must be under cwd (openin_any=p refuses "..")')
        if '"' in rel:
            raise ManifestError(f'{where}: main path contains a double quote')
        e['main_rel'] = './' + rel if rel[0] in '-&\\*' else rel
    e['jobname'] = e.get('jobname') or os.path.splitext(os.path.basename(e['main']))[0]
    e['texinputs'] = [resolve_path(p) for p in e['texinputs']]
    e['seed_files'] = {rel: resolve_path(src) for rel, src in e['seed_files'].items()}
    if isinstance(e['bib'], dict):
        e['bib'] = {'bbl': resolve_path(e['bib']['bbl'])}
    for d in e['mirror_dirs']:
        if os.path.isabs(d) or '..' in d.split('/'):
            raise ManifestError(f'{where}: mirror_dirs must be relative and without ..')
    return e


def load_manifests(paths):
    """Returns (entries, exclusions, loaded files).  exclusions are manifest-level statements of bounded coverage."""
    files = []
    for p in paths or [os.path.join(MANIFEST_DIR, '*.json')]:
        p = resolve_path(p, os.getcwd())
        got = sorted(glob.glob(p))
        if not got and paths:
            raise ManifestError(f'no manifest matches {p}')
        files += got
    entries, seen, exclusions = [], {}, []
    for path in files:
        m = read_json(path)
        if not isinstance(m, dict) or 'entries' not in m:
            raise ManifestError(f'{path}: top level must be an object with "entries"')
        unknown = set(m) - {'source', 'description', 'defaults', 'entries', 'exclusions'}
        if unknown:
            raise ManifestError(f'{path}: unknown top-level fields {sorted(unknown)}')
        defaults = dict(m.get('defaults', {}))
        if m.get('source'):
            defaults.setdefault('source', m['source'])
        for x in m.get('exclusions', []):
            exclusions.append({**x, 'manifest': os.path.basename(path)})
        for raw in m['entries']:
            e = normalize_entry(raw, defaults, path)
            if e['id'] in seen:
                raise ManifestError(f'duplicate id {e["id"]!r} in {path} and {seen[e["id"]]}')
            seen[e['id']] = path
            e['manifest'] = os.path.relpath(path, REPO) if path.startswith(REPO) else path
            entries.append(e)
    return entries, exclusions, files


_LIST_TOKEN = r'(?:-\d+\+\d+:[0-9a-f]{16}|\d+/\d+)'
NOTHING_LEFT = '-0+0:' + '0' * 16
_LIST_GROUP = re.compile(r'(both|last-pass|from-scratch):((?:%s)=%s(?:,(?:%s)=%s)*)' % ('|'.join(difftoken.LEVELS), _LIST_TOKEN, '|'.join(difftoken.LEVELS), _LIST_TOKEN))


def known_divergences(path=None):
    """{id: {'tokens': {mode: {level: token}}, 'reason': text}}, from known_divergences.txt, whose header has the form.  A line that does not have it is an error, so nothing is accepted by accident."""
    out = {}
    path = path or KNOWN_DIVERGENCES
    if SECOND_ORACLE:
        return out
    try:
        f = open(path, encoding='utf-8')
    except FileNotFoundError:
        return out
    with f:
        for n, line in enumerate(f, 1):
            line = line.strip()
            if not line or line.startswith('#'):
                continue
            parts = line.split(' ')
            tokens, k = {}, 1
            while k < len(parts) and (m := _LIST_GROUP.fullmatch(parts[k])):
                for mode in (('last-pass', 'from-scratch') if m.group(1) == 'both' else (m.group(1),)):
                    if mode in tokens:
                        raise ManifestError(f'{path}:{n}: {mode} twice')
                    tokens[mode] = dict(t.split('=', 1) for t in m.group(2).split(','))
                k += 1
            reason = ' '.join(parts[k:]).strip()
            if not tokens or not reason:
                raise ManifestError(f'{path}:{n}: an id, what differs in which mode, and a reason, please: the header of the file has the form')
            if parts[0] in out:
                raise ManifestError(f'{path}:{n}: {parts[0]} again')
            out[parts[0]] = {'tokens': tokens, 'reason': reason}
    return out


def tokens_text(tokens):
    """How a list entry writes the tokens of one mode."""
    order = lambda k: difftoken.LEVELS.index(k) if k in difftoken.LEVELS else 99
    return ','.join(f'{k}={tokens[k]}' for k in sorted(tokens, key=order)) if tokens else 'no difference'


def list_expected(listed, mode):
    """What the entry says of this mode.  The instrumented pass starts from the last pass's state, whichever mode runs it, and the list has its levels under last-pass or both."""
    want = dict(listed['tokens'].get(mode, {}))
    for level in difftoken.INST_LEVELS:
        if level in listed['tokens'].get('last-pass', {}):
            want[level] = listed['tokens']['last-pass'][level]
    return want


def list_verdict(res, e, opts, mode, now, judged):
    """A level the entry names and this run judged has to give that token, and a level it does not name has to agree.  Any other difference, or one at a level that now agrees while others do not,
    fails, and says what it is now.  An entry all of whose judged levels agree is identical, and is reported."""
    if getattr(opts, 'tokens', False) or now:
        res['tokens'] = dict(now)
    listed = getattr(opts, 'known', {}).get(e['id'])
    if listed is None:
        return
    said = {k: v for k, v in list_expected(listed, mode).items() if k in judged}
    # -0+0 is a level that differs only in what a token leaves out, such as the date of a format in \pdfuniqueresname's prefix: on the day that date is the same, it agrees.
    want = {k: v for k, v in said.items() if k in now or v != NOTHING_LEFT}
    if not now:
        if said and res['outcome'] in ('identical', 'oracle-fails'):
            res['list_agrees'] = tokens_text(said)
        return
    if now == want:
        res.update(differs_as=res['outcome'] if res['outcome'] in ACCEPTABLE else 'differs-log-only', known_reason=listed['reason'], outcome='known-divergence')
        return
    res['list_moved'] = {'said': tokens_text(want), 'now': tokens_text(now)}
    if res['outcome'] in ('identical', 'oracle-fails'):
        res['outcome'] = 'differs-log-only'
    if not (res.get('first_divergence') or {}).get('text'):
        res['first_divergence'] = {'text': 'on known_divergences.txt: it differs otherwise than the entry says'}


def machine_paths(e):
    """The places that depend on the machine, for difftoken.Masks: where the document runs, the tree, the home directory."""
    home = os.environ.get('HOME')
    return [(e['cwd'], b'<CWD>'), (REPO, b'<TREE>'), (MAIN_ROOT, b'<TREE>'), (home, b'<HOME>')]


def engine_dirs(opts):
    """Where the candidate is, by the path it was given and by its real one: its formats are there, and a wrapper may know them by either."""
    path = shutil.which(opts.engine) or opts.engine
    return [os.path.dirname(os.path.abspath(path)), os.path.dirname(os.path.realpath(path))]


def token_masks(e, opts, oracle_out, cand_out):
    """(the oracle's masks, the candidate's).  normalize_log leaves an output directory as a run of # of its length.  The candidate's own directory holds its formats."""
    common = machine_paths(e)
    o = [(oracle_out, b'<OUT>'), (b'#' * len(os.fsencode(oracle_out or '')), b'<OUT>')]
    c = [(cand_out, b'<OUT>'), (b'#' * len(os.fsencode(cand_out)), b'<OUT>')] + [(d, b'<ENGINE>') for d in engine_dirs(opts)]
    return difftoken.Masks(common + o), difftoken.Masks(common + c)


def private_key(e):
    """For an entry on the owner's documents, the key of its tokens, from the document itself: DESIGN section 13.  It is hex, and it stays in this process and its children."""
    if not e['id'].startswith(PRIVATE_IDS):
        return None
    main = e.get('main') or e['id']
    return hash_file(main) if os.path.isfile(main) else hashlib.sha256(main.encode()).hexdigest()


class Shown:
    """--show-differences: what a token covers, in a file per level, so that it can be read before it goes on the list.  Never for a private entry: its log stays where it is."""

    def __init__(self, e, opts, mode):
        root = getattr(opts, 'show_differences', None)
        self.dir = os.path.join(root, slug(e['id']), mode) if root and not e['id'].startswith(PRIVATE_IDS) else None

    def sink(self):
        return [] if self.dir else None

    def write(self, level, lines):
        if self.dir and lines:
            difftoken.write_shown(os.path.join(self.dir, level + '.txt'), lines)


def selected(args):
    entries, exclusions, files = load_manifests(args.manifest)
    if args.tier:
        allowed = {args.tier} if args.tier_exact else set(TIERS[:TIERS.index(args.tier) + 1])
        entries = [e for e in entries if e['tier'] in allowed]
    if args.source:
        entries = [e for e in entries if e['source'] in args.source]
    if args.filter:
        rx = re.compile(args.filter)
        entries = [e for e in entries if rx.search(e['id'])]
    if getattr(args, 'only_failed_last_run', False):
        run = latest_run(runs_dir(args), 'compare')
        if run is None:
            raise ManifestError('--only-failed-last-run: no earlier compare run found in ' + runs_dir(args))
        results = read_results(run)
        bad = {r['id'] for r in results if r['outcome'] not in ('identical', 'known-divergence', 'oracle-fails', 'nondeterministic', 'excluded')}
        log_progress(f'--only-failed-last-run: {len(bad)} failed entries in {os.path.basename(run)}')
        entries = [e for e in entries if e['id'] in bad]
    return entries, exclusions, files


# ------------------------------------------------------------------------------------------------ running TeX
_active = {}
_active_lock = threading.Lock()
_stop = threading.Event()


def kill_group(pid):
    try:
        os.killpg(pid, signal.SIGKILL)
    except (ProcessLookupError, PermissionError):
        pass


def kill_all_active():
    with _active_lock:
        for pid in list(_active):
            kill_group(pid)


def expand_out(argv, cwd, env):
    """An entry's env may name the output directory of this very run, as `@OUT@`, which is the directory the command line says, or where the run is.  Everything that runs a document has to do this, or it runs another document:
    tools/prof/proflib.py does it for what it runs, and tests/bench/test_select.py holds the tools to it."""
    if not env or not any('@OUT@' in v for v in env.values()):
        return env
    out = next((a.split('=', 1)[1] for a in argv if a.startswith(('-output-directory=', '-outdir='))), cwd)
    out = os.path.normpath(os.path.join(cwd, out))
    return {k: v.replace('@OUT@', out) for k, v in env.items()}


def run_proc(argv, cwd, env, timeout, stdout_path=None):
    """Run in its own session, wall-clock timed, killed with its whole group on timeout."""
    res = {'rc': None, 'timed_out': False, 'ms': None, 'cpu_ms': None, 'signal': None}
    env = expand_out(argv, cwd, env)
    sink = open(stdout_path, 'wb') if stdout_path else subprocess.DEVNULL
    timer = None
    try:
        t0 = time.perf_counter_ns()
        try:
            p = subprocess.Popen(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=sink,
                                 stderr=subprocess.STDOUT, start_new_session=True)
        except OSError as ex:
            res.update(rc=127, ms=0.0, cpu_ms=0.0, exec_error=str(ex))
            return res
        with _active_lock:
            _active[p.pid] = True

        def on_timeout():
            res['timed_out'] = True
            kill_group(p.pid)
        timer = threading.Timer(timeout, on_timeout)
        timer.start()
        _, status, ru = os.wait4(p.pid, 0)
        t1 = time.perf_counter_ns()
        p.returncode = os.waitstatus_to_exitcode(status)
        res.update(rc=p.returncode, ms=(t1 - t0) / 1e6, cpu_ms=(ru.ru_utime + ru.ru_stime) * 1000)
        if p.returncode < 0:
            res['signal'] = -p.returncode
        kill_group(p.pid)
        return res
    finally:
        if timer:
            timer.cancel()
        with _active_lock:
            if 'p' in locals():
                _active.pop(p.pid, None)
        if stdout_path:
            sink.close()


def entry_env(e):
    env = {k: os.environ[k] for k in ('PATH', 'HOME') if k in os.environ}
    env.update(DETERMINISM)
    env.update(NO_MKTEX)
    env.update(HERMETIC)
    env['openout_any'] = 'p'
    if not e['trusted']:
        env['openin_any'] = 'p'
    if e['texinputs']:
        env['TEXINPUTS'] = ':'.join(e['texinputs']) + ':'
    env.update(e['env'])
    return env


def candidate_env(e, out):
    """An engine that keeps a cache (the preamble snapshot of DESIGN section 7) gets an empty one per work directory, inside it, which is
    deleted with it.  Otherwise a compare after a first run would judge the restored path while claiming a cold one, and min-of-N in bench
    would quietly become the warm time.  tests/corpus/snapshot/ is where restored runs are judged.  A first run that always writes a checkpoint, whatever its preamble took,
    is what shows that writing one disturbs nothing."""
    env = entry_env(e)
    cache = os.path.join(os.path.dirname(out), 'engine-cache')
    os.makedirs(cache, exist_ok=True)
    env['XDG_CACHE_HOME'] = cache
    # A compare judges the literal engine, byte for byte.  The native diagram path is judged by cd/compare_native.py.
    env['RSTEX_NO_COMMUTE'] = '1'
    env['RSTEX_CHECKPOINT_POLICY'] = 'always'
    return env


def oracle_engine(e):
    return shutil.which('pdflatex' if e['mode'] == 'latex' else 'pdftex')


def tex_argv(e, engine, extra, outdir, recorder=False, prelude=''):
    """The one way TeX is invoked, for the oracle and for candidates alike."""
    argv = [engine] + list(extra)
    if e['mode'] in ('ini', 'ini-format'):
        argv.append('-ini')
    else:
        argv.append('-fmt=' + (e.get('fmt') or ('pdflatex' if e['mode'] == 'latex' else 'pdftex')))
    argv += ['-interaction=batchmode', '-no-shell-escape', '-output-directory=' + outdir]
    if recorder:
        argv.append('-recorder')
    if e.get('main') is None or os.path.splitext(os.path.basename(e['main']))[0] != e['jobname']:
        argv.append('-jobname=' + e['jobname'])
    argv += e['args']
    full_prelude = prelude + e['prelude']
    if e.get('first_line'):
        line = e['first_line']
        if full_prelude:                     # a leading * (extended mode in INITEX) has to stay first on the line
            star = '*' if line.startswith('*') else ''
            body = line[len(star):]
            line = star + full_prelude + (body if body.startswith('\\') else '\\input ' + body)
        # web2c quotes a lone argument that has a space and does not start with a backslash, which would turn
        # "*\foo \bar" into a file name.  Words given as separate arguments are joined into the first line unquoted.
        argv += line.split(' ') if line.startswith('*') else [line]
    elif full_prelude:
        argv.append(f'{full_prelude}\\input "{e["main_rel"]}"')
    else:
        argv.append(e['main_rel'])
    return argv


def snapshot_side(outdir, job):
    """Relative path -> sha256 for every produced file except the job's PDF, log, .fls, .fmt and SyncTeX file."""
    skip = {f'{job}.pdf', f'{job}.log', f'{job}.fls', f'{job}.fmt', f'{job}.synctex', f'{job}.synctex.gz'}
    res = {}
    for root, _, names in os.walk(outdir):
        for n in names:
            p = os.path.join(root, n)
            rel = os.path.relpath(p, outdir)
            if rel in skip or not os.path.isfile(p):
                continue
            res[rel] = hash_contents(p)
    return res


def hash_contents(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


def copy_side(outdir, job, dest):
    os.makedirs(dest, exist_ok=True)
    for rel in snapshot_side(outdir, job):
        d = os.path.join(dest, rel)
        os.makedirs(os.path.dirname(d), exist_ok=True)
        shutil.copy2(os.path.join(outdir, rel), d)


def overlay(src, dest):
    if os.path.isdir(src):
        shutil.copytree(src, dest, dirs_exist_ok=True)


def parse_fls(path):
    """(pwd, inputs, outputs) with absolute normalized paths, in first-occurrence order."""
    inputs, outputs, pwd = {}, {}, None
    try:
        with open(path, 'rb') as f:
            data = f.read()
    except OSError:
        return None, {}, {}
    for line in data.split(b'\n'):
        kind, _, val = line.partition(b' ')
        val = os.fsdecode(val)
        if kind == b'PWD':
            pwd = val
        elif kind in (b'INPUT', b'OUTPUT') and val:
            p = os.path.normpath(val if os.path.isabs(val) else os.path.join(pwd or '.', val))
            (inputs if kind == b'INPUT' else outputs)[p] = True
    return pwd, inputs, outputs


# ------------------------------------------------------------------------------------------------ log handling
def mask_path(data, path):
    """Mask an absolute output path; TeX may have wrapped it across lines, so newlines inside it are preserved."""
    pat = b'\n?'.join(re.escape(bytes([c])) for c in os.fsencode(path))
    return re.sub(pat, lambda m: bytes(10 if c == 10 else 35 for c in m.group()), data)


_STAT_LINE = re.compile(rb'^(\d+ strings of total length \d+|\d+ memory locations dumped.*|\d+ multiletter control sequences.*'
                        rb'|\d+ words of font info for \d+ preloaded fonts?|\d+ preloaded fonts?|Memory usage before:.*'
                        rb'|\d+ hyphenation exceptions?|Hyphenation trie of length.*)$')
_DROP_LINE = {b'entering extended mode', b' restricted \\write18 enabled.', b' \\write18 enabled.',
              b' %&-line parsing enabled.', b'\\write18 disabled.'}


_ENGINE_MESSAGE = re.compile(rb'^(!pdfTeX error: |pdfTeX warning: )[^ :]+(?=:| \(file )')
_SUBSET_TAG = re.compile(rb'make_subset_tag\(\): subset-tag collision, resolved in round [0-9]+\.')


def subset_tag_message(lines, j):
    r"""make_subset_tag's is the one message of pdfTeX that begins and ends with a newline of its own: utils.c prints "\nmake_subset_tag(): ... round 3.\n".  Under plain TeX that byte is not
    \newlinechar, so print_char writes it as it is and counts a column, and the column of the line before goes on through it.  So where the program's name puts the end of the text decides
    what the log holds, with c the column of the text's last character:
        c < 78    the text, then one empty line, from the byte and pdftex_warn's println
        c = 78    the byte reaches column 79, and TeX breaks there: one empty line more
        c = 79    TeX breaks after the text, the byte is alone on its line, then the println: one empty line more
        c > 79    the text is cut in two, and the rest goes on from column 0
    Under \newlinechar=10, as in LaTeX, the two bytes are print_ln, the column starts again, and the text is whole with one empty line after it.  Lines j and on: lines[j] ends in the ": " before the
    message.  Returns (the text, the last line that belongs to it), or None if this is not that message: then nothing here applies and the lines are compared as they are."""
    n, k, col = len(lines), j + 1, len(lines[j]) + 1
    if col == 79 and k + 1 < n and not lines[k]:          # the first byte reached column 79
        k, col = k + 1, 0
    if k >= n:
        return None
    text = lines[k]
    if _SUBSET_TAG.fullmatch(text):
        col += len(text)
    elif k + 1 < n and _SUBSET_TAG.fullmatch(text + lines[k + 1]):
        k, text, col = k + 1, text + lines[k + 1], len(lines[k + 1])
    else:
        return None
    if col in (78, 79) and k + 2 < n and not lines[k + 1] and not lines[k + 2]:
        k += 1
    return text, k


def normalize_log(data, workdirs):
    """List of (raw line number, line) with the engine-specific parts removed: banner, shell-escape and %& notes,
    output directory, memory statistics, PDF statistics and the byte count of the output file."""
    for w in workdirs:
        if w:
            data = mask_path(data, w)
    lines = data.split(b'\n')
    out, i, n = [], 0, len(lines)
    while i < n:
        line = lines[i]
        if (i == 0 and line.startswith(b'This is ')) or line in _DROP_LINE or _STAT_LINE.match(line):
            i += 1
            continue
        if line.startswith(b"Here is how much of TeX's memory") or line.startswith(b'PDF statistics:'):
            i += 1
            while i < n and lines[i].startswith(b' '):
                i += 1
            continue
        if line.startswith(b'Output written on '):
            j, buf = i, line
            while b'bytes).' not in buf and j + 1 < n and j - i < 50:
                j += 1
                buf += lines[j]
            m = re.search(rb'\((\d+ pages?), \d+ bytes\)\.', buf)
            out.append((i + 1, b'Output written on <pdf> (' + (m.group(1) if m else b'?') + b').'))
            i = j + 1
            continue
        if line.startswith((b'!pdfTeX error: ', b'pdfTeX warning: ')):
            # These name the program as it was invoked, and then wrap at a column that depends on the length of that name.  TeX breaks the line just after column 79, so a message
            # that ends at that very column is followed by an empty line, its own end of line, which is part of it and appears under no other name.
            # After "(file x): " the message goes on on the next line, and TeX still counts the characters of the line before it, so it wraps early.
            j, buf = i, line
            while j + 1 < n and j - i < 50 and len(lines[j]) >= 79:
                j += 1
                buf += lines[j]
                if not lines[j]:
                    break
            if buf.endswith(b': '):
                own = subset_tag_message(lines, j)
                if own:
                    buf += own[0]
                    j = own[1]
            out.append((i + 1, _ENGINE_MESSAGE.sub(rb'\1<engine>', buf, count=1)))
            i = j + 1
            continue
        out.append((i + 1, line))
        i += 1
    return out


# l3build's normal form, for the LaTeX team's regression tests.  A port of normalize_log in l3build-check.lua, the rules
# that can fire on a pdfTeX log.  tests/corpus/public/check_normalize.py runs both on real logs to keep them equal.
_L3_PATH = rb'[A-Za-z0-9]?:?/[^ <>]*/([^/()]*\.[A-Za-z0-9]*)'
_L3_SUBS = [(re.compile(rb'\(\./'), b'('), (re.compile(rb'\(' + _L3_PATH), rb'(../\1'), (re.compile(b'<' + _L3_PATH + b'>'), rb'<../\1>'),
            (re.compile(b'from ' + _L3_PATH + b'$'), rb'from ../\1'), (re.compile(b': ' + _L3_PATH + rb'\)'), rb': ../\1)')]
_L3_ENC = re.compile(rb'\{' + _L3_PATH + rb'\}')
_L3_DATE = re.compile(rb'[0-9]{4}[/\-][0-9]{2}[/\-][0-9]{2}')
_L3_DATE_GUARD = re.compile(rb'[^<][0-9]{4}[/\-][0-9]{2}[/\-][0-9]{2}')
_L3_SEMVER = re.compile(rb'v[0-9]+\.[0-9]+\.[0-9]+[0-9A-Za-z.+\-]*')
_L3_VERSION = re.compile(rb'v[0-9]+\.?[0-9]?[0-9]?[0-9A-Za-z]?')
_L3_FD = re.compile(rb'^ *\([./0-9A-Za-z]+\.fd[^)]*$')
_L3_WRAP = re.compile(rb'^\(([0-9A-Za-z]+)\)(\s+)[0-9]+\.$')
_L3_REG = re.compile(rb'^\\[^\]]+=\\([a-z]+)[0-9]+$')
_L3_REG_TYPES = {b'attribute', b'box', b'bytecode', b'catcodetable', b'count', b'dimen', b'insert', b'language', b'luabytecode',
                 b'luachunk', b'luafunction', b'marks', b'muskip', b'read', b'skip', b'toks', b'whatsit', b'write', b'XeTeXcharclass'}
_L3_DROP = re.compile(rb'^\.*\\special\{(?:pdf: docinfo << /Creator|ps: /setdistillerparams|! <</........UUID|header=|dvipdfmx:config|papersize)')
_L3_OPEN = re.compile(rb'^\\open(?:in|out)[0-9][0-9]? = ')
_L3_CTRL = re.compile(rb'[\x00-\x1f]')
_L3_HIGH = re.compile(rb'[\x80-\xff]')


def _l3_line(line, last, max_line):
    """One raw line.  Returns (text, carry, in_fd): the text to keep, what a wrapped line carries over, an .fd being skipped."""
    if line.startswith(b'just a message  > '):
        line = line[18:]
    line = re.sub(rb'^l\.[0-9]+ ', b'l. ...', line)
    line = re.sub(rb'lua:[0-9]+:', b'lua:...:', line)
    if len(line) == max_line and not line.endswith(b'...'):
        return b'', last + line, False
    line = last + line
    for rx, rep in _L3_SUBS:
        line = rx.sub(rep, line)
    if b'.enc}' in line:
        line = _L3_ENC.sub(b'', line)
    if _L3_DATE_GUARD.search(line):
        line = _L3_DATE.sub(b'....-..-..', line)
        line = _L3_SEMVER.sub(b'v...', line)
        line = _L3_VERSION.sub(b'v...', line)
    line = re.sub(rb'^ *\[([0-9])', rb'[\1', line)
    line = re.sub(rb'^ *\(', b'(', line)
    if _L3_FD.match(line):
        return b'', b'', True
    line = line.replace(b'cm, mm, dd, cc, bp, or sp', b'cm, mm, dd, cc, nd, nc, bp, or sp')
    line = line.replace(b'\\csname\\endcsname ', b'\\csname\\endcsname')
    line = re.sub(rb'on line [0-9]*', b'on line ...', line)
    line = re.sub(rb'on input line [0-9]*', b'on input line ...', line)
    line = _L3_WRAP.sub(rb'(\1)\2....', line)
    line = re.sub(rb'at lines [0-9]*--[0-9]*', b'at lines ...', line)
    line = _L3_CTRL.sub(lambda m: b'^^' + bytes([m.group()[0] + 64]), line)
    m = _L3_REG.match(line)
    if m and m.group(1) in _L3_REG_TYPES:
        line = re.sub(rb'[0-9]+$', b'...', line)
    if re.match(rb'^> \\box[0-9]+=(?:void)?$', line):
        line = re.sub(rb'[0-9]+=', b'...=', line)
    line = re.sub(rb',? yoko direction,?', b'', line)
    line = re.sub(rb',? yoko\(math\) direction,?', b'', line)
    if re.match(rb'^\.*\\displace 0\.0$', line) or _L3_DROP.match(line):
        return b'', b'', False
    line = line.replace(b'save cache:', b'load cache:')
    line = line.replace(b'\x7f', b'^^?')
    # l3build gets ^^ notation for the top half from -translate-file=ascii.tcx.  This runner passes no TCX, so the same
    # form is made here; the mapping is one to one and loses nothing.
    line = _L3_HIGH.sub(lambda m: b'^^%02x' % m.group()[0], line)
    return line, b'', False


def l3build_normalize_log(data, workdirs, max_line=9999):
    """List of (raw line number, line) as l3build would store it in a .tlg: the part between START-TEST-LOG and END-TEST-LOG,
    without OMIT...TIMO, with lines rejoined and paths, dates, versions, line numbers and register numbers masked.  The output
    directory counts as the current directory, which is where l3build runs a test.  A log with no START-TEST-LOG (the test died
    earlier, or is not an l3build test at all) is not a test body, so it falls back to the ordinary normal form: an empty list here
    would make every such entry pass."""
    if b'START-TEST-LOG' not in data:
        return normalize_log(data, workdirs)
    for w in workdirs:
        if w:
            pat = b'\n?'.join(re.escape(bytes([c])) for c in os.fsencode(w) + b'/')
            data = re.sub(pat, b'./', data)
    out, last, in_fd, started, skipping = [], b'', False, False, False
    for no, line in enumerate(data.replace(b'\r\n', b'\n').split(b'\n'), 1):
        if line == b'START-TEST-LOG':
            started = True
        elif line == b'END-TEST-LOG' or re.match(rb"^Here is how much of .?.?.?TeX's memory you used:", line):
            break
        elif line in (b'OMIT', b'just a message  > OMIT'):
            skipping = True
        elif line in (b'TIMO', b')TIMO', b'just a message  > TIMO'):
            skipping = False
        elif started and not skipping:
            if in_fd:
                in_fd = b')' not in line
                continue
            text, last, in_fd = _l3_line(line, last, max_line)
            if text.strip(b' ') and not _L3_OPEN.match(text):
                out.append((no, text))
    return out


def judged_log(e, data, workdirs):
    """The normal form an entry's log is judged on."""
    if e['log_normalize'] == 'l3build':
        return [(no, _ENGINE_MESSAGE.sub(rb'\1<engine>', text, count=1)) for no, text in l3build_normalize_log(data, workdirs)]
    return normalize_log(data, workdirs)


def box_lines(lines):
    """Keep only \\tracingoutput dumps: from 'Completed box being shipped out' to the blank line ending the dump."""
    out, inside = [], False
    for k, (no, line) in enumerate(lines):
        if line.startswith(b'Completed box being shipped out ['):
            inside = True
        elif inside and not line:
            nxt = lines[k + 1][1] if k + 1 < len(lines) else b''
            if not nxt.startswith((b'.', b'\\')):
                inside = False
        if inside:
            out.append((no, line))
    return out


_ENGINE_LINE = (b'!pdfTeX error: <engine>', b'pdfTeX warning: <engine>')


def same_message_rewrapped(a, b, i, j):
    """Two engine messages that may have been cut at other columns, because the two programs have names of other lengths: (next i, next j) if they are one text, else None.
    Lines are added to the shorter side for as long as one is the start of the other.  normalize_log rejoins what it can from one log, and this settles the rest: under an entry's own
    max_print_line, a line of 79 columns is not one that TeX broke, and the log does not say which it is.  A line break inside an engine message does not count, and nothing else about it is forgiven."""
    x, y = a[i][1], b[j][1]
    if not (x.startswith(_ENGINE_LINE) and y.startswith(_ENGINE_LINE)):
        return None
    k, m = i, j
    while x != y and (x.startswith(y) or y.startswith(x)):
        if len(x) < len(y) and k + 1 < len(a):
            k += 1
            x += a[k][1]
        elif len(y) < len(x) and m + 1 < len(b):
            m += 1
            y += b[m][1]
        else:
            return None
    return (k + 1, m + 1) if x == y else None


def first_log_divergence(a, b):
    """First differing normalized line; shipout counters are taken from A."""
    shipouts, label = 0, None
    i = j = 0
    while i < len(a) and j < len(b):
        if a[i][1] != b[j][1]:
            same = same_message_rewrapped(a, b, i, j)
            if same is None:
                break
            i, j = same
            continue
        m = re.match(rb'Completed box being shipped out \[([^\]]*)\]', a[i][1])
        if m:
            shipouts += 1
            label = m.group(1).decode('latin-1')
        i += 1
        j += 1
    else:
        if i == len(a) and j == len(b):
            return None
    if i < len(a):
        m = re.match(rb'Completed box being shipped out \[([^\]]*)\]', a[i][1])
        if m:
            shipouts += 1
            label = m.group(1).decode('latin-1')
    return {'line_a': a[i][0] if i < len(a) else None, 'line_b': b[j][0] if j < len(b) else None,
            'a': clip(a[i][1], 400) if i < len(a) else '<end of log>',
            'b': clip(b[j][1], 400) if j < len(b) else '<end of log>',
            'shipout': shipouts, 'page_label': label,
            'context': [clip(x[1], 200) for x in a[max(0, i - 3):i]]}


def log_stats(path, workdirs=()):
    st = {'errors': 0, 'first_error': None, 'pages': None, 'no_pages': False, 'fatal': False,
          'overfull': 0, 'underfull': 0, 'bytes': 0}
    try:
        with open(path, 'rb') as f:
            data = f.read()
    except OSError:
        st['missing'] = True
        return st
    st['bytes'] = len(data)
    in_box_warning = False              # what follows Overfull or Underfull, up to a blank line, is the typeset text, and a line of it can begin with "! "
    for line in data.split(b'\n'):
        if in_box_warning:
            in_box_warning = bool(line)
        elif line.startswith(b'! '):
            st['errors'] += 1
            st['first_error'] = st['first_error'] or clip(line, 200)
        elif line.startswith(b'Overfull '):
            st['overfull'] += 1
            in_box_warning = True
        elif line.startswith(b'Underfull '):
            st['underfull'] += 1
            in_box_warning = True
    # A long output name wraps this line anywhere, the page count included.
    k = data.rfind(b'Output written on ')
    m = re.search(rb'\((\d+) pages?, \d+ bytes\)', data[k:k + 16384].replace(b'\n', b'')) if k >= 0 else None
    if m:
        st['pages'] = int(m.group(1))
    st['no_pages'] = b'No pages of output.' in data
    st['fatal'] = b'Fatal error occurred' in data or b'Emergency stop' in data or b'capacity exceeded' in data
    return st


def unwritable_dirs(log_path, outdir):
    """Directories the document needed in the output directory (\\include into a subdirectory)."""
    try:
        with open(log_path, 'rb') as f:
            data = f.read()[-200000:].replace(b'\n', b'')
    except OSError:
        return []
    res = []
    for m in re.finditer(rb"I can't write on file `([^']*)'", data):
        name = os.fsdecode(m.group(1))
        d = os.path.dirname(name)
        if d and not os.path.isabs(d) and '..' not in d.split('/') and not os.path.isdir(os.path.join(outdir, d)):
            res.append(d)
    return res


# ------------------------------------------------------------------------------------------------ oracle stage
_oracle_version = None


def oracle_version():
    global _oracle_version
    if _oracle_version is None:
        exe = shutil.which('pdftex')
        if not exe:
            sys.exit('pdftex not found in PATH')
        banner = subprocess.run([exe, '--version'], capture_output=True, text=True).stdout.splitlines()[0]
        _oracle_version = {'banner': banner, 'path': exe, 'sha256': hash_file(os.path.realpath(exe))}
    return _oracle_version


def config_hash(e):
    keys = ('mode', 'fmt', 'main_rel', 'cwd', 'first_line', 'prelude', 'jobname', 'texinputs', 'env', 'args', 'bib',
            'index', 'seed_files', 'passes', 'max_passes', 'trusted', 'compare', 'mirror_dirs')
    payload = {k: e.get(k) for k in keys}
    if e.get('judge_failure'):                # only when set, so that existing oracle caches stay valid
        payload['judge_failure'] = True
    for k in ('log_normalize', 'index_style'):
        if e.get(k) not in (None, 'default'):
            payload[k] = e[k]
    payload.update(schema=SCHEMA, determinism=DETERMINISM, oracle=oracle_version())
    text = json.dumps(payload, sort_keys=True).replace(REPO, MAIN_ROOT)
    # What this checkout's own files say is the same in any checkout, but a directory of its own on the search path is printed in the log by its path: another checkout's oracle has another path there, at
    # another length.  refs/ and target/ are the main tree's for all of them, by resolve_path, so this is only a directory the checkout really holds, such as tests/ladder.
    linked = tuple(os.path.join(REPO, d) + os.sep for d in ('target', 'refs'))
    if REPO != MAIN_ROOT and any((t == REPO or t.startswith(REPO + os.sep)) and not t.startswith(linked) for t in e['texinputs']):
        text += REPO
    return sha_text(text)[:12]


def names_of(d):
    try:
        return {n for n in os.listdir(d) if not n.startswith('.')}
    except OSError:
        return set()


def cwd_listing(cwd):
    """Names only, without dotfiles: a new local file can shadow a package lookup that .fls never recorded as failed."""
    try:
        return sha_text('\n'.join(sorted(n for n in os.listdir(cwd) if not n.startswith('.'))))
    except OSError:
        return 'missing'


def closure_digest(inputs, listing):
    h = hashlib.sha256()
    for path in sorted(inputs):
        h.update(os.fsencode(path) + b'\0' + inputs[path].encode() + b'\n')
    h.update(listing.encode())
    return h.hexdigest()[:12]


def oracle_base(e):
    return os.path.join(ORACLE_ROOT, slug(e['id']))


def in_this_checkout(p, meta):
    """An input an oracle recorded under the checkout that made it, as this checkout has it.  config_hash counts every checkout as the main tree, so that they share one oracle of an entry.  That sharing
    is only sound if each asks of its own files: a test document edited here has to invalidate an oracle made from another checkout's copy, which that checkout still holds unchanged.  What this
    checkout does not have stays where it was recorded: a file git does not track, that a manifest names in the main tree."""
    root = meta.get('checkout')
    if not root:                                          # an oracle from before this was recorded: the main tree or one of its worktrees
        if not p.startswith(MAIN_ROOT + os.sep):
            return p
        m = re.match(r'\.claude/worktrees/[^/]+/', p[len(MAIN_ROOT) + 1:])
        root = MAIN_ROOT + (os.sep + m.group().rstrip('/') if m else '')
    if not p.startswith(root + os.sep):
        return p
    here = os.path.join(REPO, p[len(root) + 1:])
    return here if os.path.lexists(here) else p


def printed_by_another_path(e, meta):
    """An input found on a search path is printed in the log with the path it was found by.  So an oracle that found a directory of e's texinputs by way of a link, another checkout's target/, has that
    checkout's path in its log, and it wraps by that path's length.  Its inputs hash the same and it would be taken for this run's, and every candidate would differ from it in that line."""
    for t in e['texinputs']:
        real = os.path.realpath(t) + os.sep
        for p in meta['inputs']:
            if not p.startswith(t + os.sep) and os.path.realpath(p).startswith(real):
                return True
    return False


def load_valid_oracle(e):
    """The cached oracle for this entry if its configuration and every recorded input still hash the same, read as this checkout has them."""
    base = oracle_base(e)
    prefix = config_hash(e) + '-'
    try:
        names = os.listdir(base)
    except OSError:
        return None
    for name in names:
        if not name.startswith(prefix):
            continue
        d = os.path.join(base, name)
        try:
            meta = read_json(os.path.join(d, 'meta.json'))
        except (OSError, ValueError):
            continue
        # A run killed from outside says nothing about the document, and it read only part of what the document reads: its closure stays valid when a whole oracle's no longer is.
        if re.search(r'killed by signal (9|15)\b', meta.get('class_reason') or ''):
            continue
        current = {p: hash_file(in_this_checkout(p, meta)) for p in meta['inputs']}
        if closure_digest(current, cwd_listing(e['cwd'])) == name[len(prefix):] and not printed_by_another_path(e, meta):
            meta['dir'] = d
            return meta
    return None


def effective_class(meta, e):
    """expected_class only ever widens: a human 'nondeterministic' label survives a lucky pair of equal runs."""
    observed = meta['observed_class']
    if observed == 'normal' and e['expected_class'] == 'nondeterministic':
        return 'nondeterministic'
    return observed


def seed_workdir(e, out, seed_dir=None):
    if seed_dir:
        overlay(seed_dir, out)
        return
    if isinstance(e['bib'], dict):
        shutil.copy2(e['bib']['bbl'], os.path.join(out, e['jobname'] + '.bbl'))
    for rel, src in e['seed_files'].items():
        d = os.path.join(out, rel)
        os.makedirs(os.path.dirname(d), exist_ok=True)
        shutil.copy2(src, d)


def run_side_tools(e, out, env, job):
    """Bibliography and index programs.  They run here, in oracle preparation, and nowhere else: a candidate gets
    their output files replayed at the same point."""
    steps = []
    aux = os.path.join(out, job + '.aux')
    want_bib = e['bib'] == 'bibtex'
    if e['bib'] == 'auto' and os.path.isfile(aux):
        with open(aux, 'rb') as f:
            has_bibdata = b'\\bibdata' in f.read()
        shipped = os.path.isfile(os.path.join(e['cwd'], job + '.bbl'))
        want_bib = has_bibdata and not shipped
        if has_bibdata and shipped:
            steps.append({'step': 'bibtex', 'skipped': 'shipped .bbl in cwd'})
    if want_bib:
        tool_env = dict(env)
        tool_env['BIBINPUTS'] = e['cwd'] + ':'
        tool_env['BSTINPUTS'] = e['cwd'] + ':'
        tool_env['openin_any'] = 'a' if e['trusted'] else 'r'  # 'p' would refuse the absolute path of the .bib
        r = run_proc(['bibtex', job], out, tool_env, 120, os.path.join(out, '..', 'bibtex.stdout'))
        steps.append({'step': 'bibtex', 'rc': r['rc'], 'ms': r['ms'], 'timed_out': r['timed_out']})
    if e['index'] == 'makeindex' and os.path.isfile(os.path.join(out, job + '.idx')):
        r = run_proc(['makeindex', '-q'] + (['-s', e['index_style']] if e.get('index_style') else []) + [job], out, env, 120)
        steps.append({'step': 'makeindex', 'rc': r['rc'], 'ms': r['ms'], 'timed_out': r['timed_out']})
    return steps


def run_clean_oracle(e, timeout):
    """One complete from-clean oracle compile.  Returns a record; the work directory stays for the caller."""
    job = e['jobname']
    env = entry_env(e)
    engine = oracle_engine(e)
    mirrored = list(e['mirror_dirs'])
    if e.get('main') and not os.path.isfile(e['main']):
        return {'failed': f'main file missing: {e["main"]}', 'work': None, 'mirrored': mirrored, 'steps': [], 'inputs': {}}
    for src in list(e['seed_files'].values()) + ([e['bib']['bbl']] if isinstance(e['bib'], dict) else []):
        if not os.path.isfile(src):
            return {'failed': f'seed file missing: {src}', 'work': None, 'mirrored': mirrored, 'steps': [], 'inputs': {}}
    for _ in range(64):
        w = new_workdir()
        out = os.path.join(w, 'o')
        for d in mirrored:
            os.makedirs(os.path.join(out, d), exist_ok=True)
        seed_workdir(e, out)
        os.makedirs(os.path.join(w, 'seed'))
        seed_workdir(e, os.path.join(w, 'seed'))
        rec = drive_oracle_passes(e, w, out, job, env, engine, timeout)
        rec['mirrored'] = mirrored
        need = unwritable_dirs(os.path.join(out, job + '.log'), out)
        if need and (rec.get('failed') or rec.get('judged_failure')):
            mirrored += [d for d in need if d not in mirrored]
            shutil.rmtree(w, ignore_errors=True)
            continue
        return rec
    rec['failed'] = 'gave up after 64 attempts at mirroring \\include subdirectories'
    return rec


def drive_oracle_passes(e, w, out, job, env, engine, timeout):
    rec = {'work': w, 'workdir_path': out, 'steps': [], 'pass_ms': [], 'pass_cpu_ms': [], 'gen_after_pass': {},
           'inputs': {}, 'failed': None, 'converged': None, 'last_rc': None}
    limit = e['passes'] if isinstance(e['passes'], int) else e['max_passes']
    before = snapshot_side(out, job)
    pre = os.path.join(w, 'pre')
    fls = os.path.join(out, job + '.fls')
    for i in range(1, limit + 1):
        shutil.rmtree(pre, ignore_errors=True)
        copy_side(out, job, pre)
        remove_quietly(os.path.join(out, job + '.pdf'))   # so that a fatal later pass cannot look like it made one
        r = run_proc(tex_argv(e, engine, [], out, recorder=True), e['cwd'], env, timeout,
                     os.path.join(w, f'stdout.{i}'))
        rec['steps'].append({'step': 'tex', 'pass': i, 'rc': r['rc'], 'ms': r['ms'], 'timed_out': r['timed_out']})
        rec['pass_ms'].append(r['ms'])
        rec['pass_cpu_ms'].append(r['cpu_ms'])
        rec['last_rc'] = r['rc']
        _, inputs, _ = parse_fls(fls)
        rec['inputs'].update(inputs)
        if r['timed_out']:
            rec['failed'] = f'oracle timeout after {timeout} s in pass {i}'
            break
        if r['signal']:
            rec['failed'] = f'oracle killed by signal {r["signal"]} in pass {i}'
            break
        stats = log_stats(os.path.join(out, job + '.log'))
        has_pdf = os.path.isfile(os.path.join(out, job + '.pdf'))
        rec.pop('judged_failure', None)
        if r['rc'] != 0 and not has_pdf and (stats['fatal'] or stats.get('missing') or (e['compare'] == 'pdf' and not stats['no_pages'])):
            reason = f'no PDF, exit {r["rc"]}' + (f': {stats["first_error"]}' if stats['first_error'] else '')
            if not e['judge_failure'] or stats.get('missing'):
                rec['failed'] = reason
                break
            # The failure is the behaviour under test: the candidate must fail the same way, judged on log and exit status.
            rec['judged_failure'] = reason
        after = snapshot_side(out, job)
        if i == 1:
            tools = run_side_tools(e, out, env, job)
            rec['steps'] += tools
            current = snapshot_side(out, job)
            changed = [p for p in current if current[p] != after.get(p)]
            if changed:
                gen = os.path.join(w, 'gen', '1')
                for p in changed:
                    d = os.path.join(gen, p)
                    os.makedirs(os.path.dirname(d), exist_ok=True)
                    shutil.copy2(os.path.join(out, p), d)
                rec['gen_after_pass']['1'] = sorted(changed)
            after = current
        rec['converged'] = after == before
        if e['passes'] == 'auto' and rec['converged']:
            break
        before = after
    rec['passes_run'] = len(rec['pass_ms'])
    rec['final_side'] = snapshot_side(out, job) if os.path.isdir(out) else {}
    return rec


_ID_RE = re.compile(rb'/ID \[<[0-9A-Fa-f]*> ?<[0-9A-Fa-f]*>\]')


def pdf_equal_except_id(a, b):
    try:
        with open(a, 'rb') as f:
            x = f.read()
        with open(b, 'rb') as f:
            y = f.read()
    except OSError:
        return False
    return x == y or _ID_RE.sub(b'', x) == _ID_RE.sub(b'', y)


def classify_oracle(e, a, b):
    """Compare two clean runs made in different directories."""
    job = e['jobname']
    det = {}
    if a['failed'] or b['failed']:
        if a['failed'] and b['failed']:
            return 'oracle-fails', a['failed'], det
        return 'nondeterministic', f'run A: {a["failed"] or "ok"}; run B: {b["failed"] or "ok"}', det
    a_out, b_out = a['workdir_path'], b['workdir_path']
    pa, pb = os.path.join(a_out, job + '.pdf'), os.path.join(b_out, job + '.pdf')
    has_a, has_b = os.path.isfile(pa), os.path.isfile(pb)
    det['passes'] = [a['passes_run'], b['passes_run']]
    det['side_files_equal'] = a['final_side'] == b['final_side']
    det['side_files_differing'] = sorted(k for k in set(a['final_side']) | set(b['final_side'])
                                         if a['final_side'].get(k) != b['final_side'].get(k))
    raw_a, raw_b = open_bytes(os.path.join(a_out, job + '.log')), open_bytes(os.path.join(b_out, job + '.log'))
    la = judged_log(e, raw_a, [a_out])
    lb = judged_log(e, raw_b, [b_out])
    log_diff = first_log_divergence(la, lb)
    det['log_equal'] = log_diff is None
    if e['log_normalize'] != 'default':
        det['log_judged_lines'] = len(la)
        det['log_strict_equal'] = first_log_divergence(normalize_log(raw_a, [a_out]), normalize_log(raw_b, [b_out])) is None
    if log_diff:
        det['log_first_diff'] = log_diff
    reasons = []
    if a['passes_run'] != b['passes_run']:
        reasons.append(f'pass count {a["passes_run"]} vs {b["passes_run"]}')
    if not det['side_files_equal']:
        reasons.append('side files differ: ' + ', '.join(det['side_files_differing'][:5]))
    if has_a != has_b:
        reasons.append('PDF present in one run only')
    elif has_a:
        det['pdf_bytes_equal_except_id'] = pdf_equal_except_id(pa, pb)
        if not det['pdf_bytes_equal_except_id']:
            c = run_compare_pdf(pa, pb)
            det['compare_pdf'] = {k: c.get(k) for k in ('status', 'text')}
            if c['status'] != 'identical':
                reasons.append('PDF differs between clean runs: ' + c.get('text', c.get('error', '')))
    if not has_a or e['gate_log'] or e['compare'] == 'log':
        if log_diff:
            reasons.append(f'log differs between clean runs at line {log_diff["line_a"]}')
    if e['mode'] == 'ini-format' and not (os.path.isfile(os.path.join(a_out, job + '.fmt'))
                                          and os.path.isfile(os.path.join(b_out, job + '.fmt'))):
        return 'oracle-fails', 'ini-format run produced no .fmt', det
    if reasons:
        return 'nondeterministic', '; '.join(reasons), det
    return 'normal', None, det


def remove_quietly(path):
    try:
        os.unlink(path)
    except OSError:
        pass


def open_bytes(path):
    try:
        with open(path, 'rb') as f:
            return f.read()
    except OSError:
        return b''


def prepare_entry(e, force=False, retry_fails=False):
    """Returns (meta, reused)."""
    if not force:
        meta = load_valid_oracle(e)
        if meta and not (retry_fails and meta['observed_class'] == 'oracle-fails'):
            return meta, True
    timeout = e['timeout']
    a = run_clean_oracle(e, timeout)
    b = run_clean_oracle(e, timeout)
    cls, reason, det = classify_oracle(e, a, b)
    # A SIGKILL from outside (a cleanup in another session, the kernel out of memory) is not a property of the document, and this verdict is cached:
    # it would take the entry out of the denominator for good.  So a run killed by a signal gets one more try.
    if cls == 'nondeterministic' and 'killed by signal' in (reason or '') and not _stop.is_set():
        for r in (a, b):
            if r.get('work'):
                shutil.rmtree(r['work'], ignore_errors=True)
        a = run_clean_oracle(e, timeout)
        b = run_clean_oracle(e, timeout)
        cls, reason, det = classify_oracle(e, a, b)
    job = e['jobname']
    inputs = {}
    workdirs = [p for p in (a.get('workdir_path'), b.get('workdir_path')) if p]
    stale = []
    for path in a['inputs']:
        if any(path.startswith(w + '/') or path == w for w in workdirs):
            continue
        inputs[path] = hash_file(path)
        if os.path.splitext(path)[1] in SIDE_EXT and not path.startswith(TDS_PREFIXES):
            stale.append(path)
    for extra in [e.get('main')] + list(e['seed_files'].values()) + ([e['bib']['bbl']] if isinstance(e['bib'], dict) else []):
        if extra:
            inputs[extra] = hash_file(extra)
    if e['bib'] in ('bibtex', 'auto'):
        for name in sorted(os.listdir(e['cwd'])) if os.path.isdir(e['cwd']) else []:
            if name.endswith(('.bib', '.bst')):
                inputs[os.path.join(e['cwd'], name)] = hash_file(os.path.join(e['cwd'], name))
    key = f'{config_hash(e)}-{closure_digest(inputs, cwd_listing(e["cwd"]))}'
    base = oracle_base(e)
    os.makedirs(base, exist_ok=True)
    tmp = os.path.join(base, f'.tmp-{secrets.token_hex(4)}')
    os.makedirs(tmp)
    final = os.path.join(tmp, 'final')
    meta = {'schema': SCHEMA, 'id': e['id'], 'key': key, 'oracle': oracle_version(), 'created': now(), 'checkout': REPO,
            'observed_class': cls, 'class_reason': reason, 'determinism': det, 'jobname': job,
            'inputs': inputs, 'stale_side_inputs': sorted(stale), 'mirrored': a.get('mirrored', []),
            'steps': a.get('steps', []), 'pass_ms': a.get('pass_ms', []), 'pass_cpu_ms': a.get('pass_cpu_ms', []),
            'passes_run': a.get('passes_run', 0), 'converged': a.get('converged'), 'last_rc': a.get('last_rc'),
            'gen_after_pass': a.get('gen_after_pass', {}), 'workdir_path': a.get('workdir_path'),
            'side_files': a.get('final_side', {}), 'pdf': None, 'log': {}, 'judged_failure': a.get('judged_failure')}
    if a.get('work'):
        out = a['workdir_path']
        shutil.move(out, final)
        pre = os.path.join(a['work'], 'pre')
        if os.path.isdir(pre):
            shutil.move(pre, os.path.join(tmp, 'pre_last'))
        else:
            os.makedirs(os.path.join(tmp, 'pre_last'))
        for sub in ('gen', 'seed'):
            p = os.path.join(a['work'], sub)
            if os.path.isdir(p):
                shutil.move(p, os.path.join(tmp, sub))
        pdf = os.path.join(final, job + '.pdf')
        if os.path.isfile(pdf):
            meta['pdf'] = {'sha256': hash_contents(pdf), 'bytes': os.path.getsize(pdf)}
        meta['log'] = log_stats(os.path.join(final, job + '.log'))
    if cls == 'nondeterministic' and b.get('work') and os.path.isfile(os.path.join(b['workdir_path'], job + '.pdf')):
        shutil.copy2(os.path.join(b['workdir_path'], job + '.pdf'), os.path.join(tmp, 'run_b.pdf'))
    for r in (a, b):
        if r.get('work'):
            shutil.rmtree(r['work'], ignore_errors=True)
    write_json(os.path.join(tmp, 'meta.json'), meta)
    dest = os.path.join(base, key)
    if not force and not retry_fails and os.path.isfile(os.path.join(dest, 'meta.json')):
        # Another process compiled the same entry at the same time.  Its oracle stays: replacing it would change the work directory in the logs under whoever is reading them.
        shutil.rmtree(tmp, ignore_errors=True)
        meta = read_json(os.path.join(dest, 'meta.json'))
        meta['dir'] = dest
        return meta, True
    if os.path.isdir(dest):
        shutil.rmtree(dest, ignore_errors=True)
    os.rename(tmp, dest)
    # Old oracles of an entry go, but not young ones: a run in progress in another checkout, or under another run.py, may be reading one.
    for name in os.listdir(base):
        path = os.path.join(base, name)
        if name != key and not name.startswith('.tmp-') and time.time() - os.path.getmtime(path) > 86400:
            shutil.rmtree(path, ignore_errors=True)
    meta['dir'] = dest
    return meta, False


def oracle_record(e, meta, reused):
    cls = effective_class(meta, e)
    return {'id': e['id'], 'source': e['source'], 'tier': e['tier'], 'class': cls, 'observed_class': meta['observed_class'],
            'expected_class': e['expected_class'], 'class_mismatch': e['expected_class'] != meta['observed_class'],
            'reason': meta.get('class_reason'), 'reused': reused, 'key': meta['key'], 'passes_run': meta['passes_run'],
            'converged': meta['converged'], 'pass_ms': meta['pass_ms'], 'pages': meta['log'].get('pages'),
            'errors': meta['log'].get('errors'), 'first_error': meta['log'].get('first_error'),
            'overfull': meta['log'].get('overfull'), 'underfull': meta['log'].get('underfull'),
            'stale_side_inputs': meta.get('stale_side_inputs', []), 'dir': meta['dir'], 'steps': meta['steps'],
            'judged_failure': meta.get('judged_failure'), 'last_rc': meta.get('last_rc'), 'fatal': meta['log'].get('fatal'),
            'has_pdf': bool(meta.get('pdf')), 'units_total': len(e.get('units', ()))}


# ------------------------------------------------------------------------------------------------ comparators
def pdf_differences(path, alone=None):
    """What compare_pdf.py wrote to --differences, as lines: {'L4': [(where, oracle's, candidate's)], 'L5': [...]}."""
    if alone:
        subprocess.run([sys.executable, '-I', COMPARE_PDF, alone, alone, '--alone', '--pinned-clock', '--differences', path], capture_output=True, timeout=3600)
    try:
        d = read_json(path)
    except (OSError, ValueError):
        return {'L4': [], 'L5': [('the differences of the two PDFs could not be read', [], [b'?'])]}
    enc = lambda lines: [x.encode('utf-8', 'surrogateescape') for x in lines]
    return {k: [(w, enc(a), enc(b)) for w, a, b in d.get(k, [])] for k in ('L4', 'L5')}


def run_compare_pdf(a, b, differences=None):
    """L4 and L5 through tests/harness/compare_pdf.py; A is the oracle.  Its text output is parsed, and a missing
    RESULT line means the tool itself failed: exit status 1 alone cannot tell that from a difference."""
    try:
        p = subprocess.run([sys.executable, '-I', COMPARE_PDF, a, b, '--no-raster', '--pinned-clock', '--jobs', '1', '--context', '3'] + (['--differences', differences] if differences else []),
                           capture_output=True, timeout=3600)
    except subprocess.TimeoutExpired:
        return {'status': 'tool-error', 'error': 'compare_pdf.py timed out after 3600 s'}
    out = p.stdout.decode('utf-8', 'replace')
    err = p.stderr.decode('utf-8', 'replace')
    if 'RESULT:' not in out:
        return {'status': 'unreadable-pdf' if 'qpdf failed on' in err else 'tool-error', 'error': clip(err or out, 600)}
    res = {'status': 'identical' if 'RESULT: IDENTICAL' in out else 'different'}
    m = re.search(r'^pages: (\d+) vs (\d+); content-equal (\d+)/(\d+); meta-equal (\d+)/(\d+)', out, re.M)
    res['pages_a'], res['pages_b'] = int(m.group(1)), int(m.group(2))
    res['content_equal'], res['pages_compared'], res['meta_equal'] = int(m.group(3)), int(m.group(4)), int(m.group(5))
    res['diff_pages'] = [int(x.group(1)) for x in re.finditer(r'^\s*(\d+) DIFF\s+\d+ ', out, re.M)]
    m = re.search(r'^FIRST CONTENT DIVERGENCE: page (\d+), operator #(\d+) of (\d+)/(\d+)\n((?:    .*\n)*)', out, re.M)
    if m:
        ctx, opa, opb = [], None, None
        for line in m.group(5).splitlines():
            tag, body = line[4:5], line[6:]
            if tag == '=':
                ctx.append(body)
            elif tag == 'A':
                opa = body
            elif tag == 'B':
                opb = body
        res['first_content'] = {'page': int(m.group(1)), 'operator': int(m.group(2)), 'ops_a': int(m.group(3)),
                                'ops_b': int(m.group(4)), 'a': opa, 'b': opb, 'context': ctx}
    m = re.search(r'^FIRST META DIVERGENCE: page (\d+): (.*)$', out, re.M)
    if m:
        res['first_meta'] = {'page': int(m.group(1)), 'paths': m.group(2)}
    m = re.search(r'^document structure: (EQUAL|(\d+) DIFFERENCES) \((\d+) vs (\d+) fonts\)\n((?:     .*\n)*)', out, re.M)
    if m:
        res['structure_equal'] = m.group(1) == 'EQUAL'
        res['structure_diffs'] = int(m.group(2) or 0)
        res['structure_paths'] = [s.strip() for s in m.group(5).splitlines()]
        res['fonts_a'], res['fonts_b'] = int(m.group(3)), int(m.group(4))
    res['text'] = pdf_diff_text(res)
    return res


def pdf_diff_text(c):
    if c.get('first_content'):
        f = c['first_content']
        return f'L4 page {f["page"]}, operator #{f["operator"]} of {f["ops_a"]}/{f["ops_b"]}: oracle `{clip(f["a"])}` | candidate `{clip(f["b"])}`'
    if c.get('pages_a') != c.get('pages_b'):
        return f'L4 page count: oracle {c.get("pages_a")}, candidate {c.get("pages_b")}'
    if c.get('first_meta'):
        return f'L5 page {c["first_meta"]["page"]} metadata: {clip(c["first_meta"]["paths"])}'
    if c.get('structure_diffs'):
        return f'L5 document structure, {c["structure_diffs"]} differences, first: {clip(c["structure_paths"][0] if c["structure_paths"] else "?")}'
    return 'identical'


def page_hashes(pdf, dpi):
    """Per-page hash of the raster.  One pdftoppm process for the whole file, streaming PPM from its stdout, so
    memory stays at one page and a long PDF is not re-parsed per page."""
    p = subprocess.Popen(['pdftoppm', '-r', str(dpi), pdf], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    res = []
    f = p.stdout
    try:
        while True:
            head = f.read(2)
            if not head:
                break
            if head != b'P6':
                raise ValueError('pdftoppm output is not P6')
            fields, cur = [], b''
            while len(fields) < 3:
                c = f.read(1)
                if not c:
                    raise ValueError('truncated PPM header')
                if c.isspace():
                    if cur:
                        fields.append(int(cur))
                        cur = b''
                else:
                    cur += c
            n = fields[0] * fields[1] * 3
            h = hashlib.sha1()
            h.update(b'%d %d ' % (fields[0], fields[1]))
            while n:
                chunk = f.read(min(n, 1 << 20))
                if not chunk:
                    raise ValueError('truncated PPM data')
                h.update(chunk)
                n -= len(chunk)
            res.append(h.hexdigest())
    finally:
        f.close()
        p.wait()
    return res


def render_page(pdf, page, dpi):
    out = subprocess.run(['pdftoppm', '-r', str(dpi), '-f', str(page), '-l', str(page), '-singlefile', pdf],
                         capture_output=True).stdout
    m = re.match(rb'P6\s+(\d+)\s+(\d+)\s+(\d+)\s', out)
    return int(m.group(1)), int(m.group(2)), out[m.end():]


def raster_percent(a, b, page, dpi):
    wa, ha, x = render_page(a, page, dpi)
    wb, hb, y = render_page(b, page, dpi)
    if (wa, ha) != (wb, hb):
        return 100.0
    if x == y:
        return 0.0
    return 100.0 * sum(x[i:i + 3] != y[i:i + 3] for i in range(0, len(x), 3)) / (wa * ha)


def compare_raster(oracle_dir, oracle_pdf, cand_pdf, dpi):
    """L7.  Oracle hashes are cached next to the oracle.  Differing pixels are computed for the first 5 differing
    pages only, and the result says so; the list of differing pages is always complete."""
    cache = os.path.join(oracle_dir, f'raster-{dpi}.json')
    try:
        ha = read_json(cache)
    except (OSError, ValueError):
        ha = page_hashes(oracle_pdf, dpi)
        if not ha:                                # the PDF went away or will not render: say so, and do not cache that as "no pages"
            raise ValueError(f'pdftoppm rendered nothing from {oracle_pdf}')
        write_json(cache, ha)
    hb = page_hashes(cand_pdf, dpi)
    diff = [i + 1 for i in range(min(len(ha), len(hb))) if ha[i] != hb[i]]
    res = {'dpi': dpi, 'pages_a': len(ha), 'pages_b': len(hb), 'diff_pages': diff, 'percent': {}}
    for p in diff[:5]:
        res['percent'][str(p)] = round(raster_percent(oracle_pdf, cand_pdf, p, dpi), 4)
    if len(diff) > 5:
        res['percent_note'] = f'pixel percentage computed for the first 5 of {len(diff)} differing pages'
    return res


def side_diff(oracle_final, cand_out, job, oracle_side):
    """L0: bytes of every side file."""
    cand_side = snapshot_side(cand_out, job)
    missing = sorted(set(oracle_side) - set(cand_side))
    extra = sorted(set(cand_side) - set(oracle_side))
    differing = sorted(k for k in set(oracle_side) & set(cand_side) if oracle_side[k] != cand_side[k])
    res = {'files': len(oracle_side), 'missing_in_candidate': missing, 'extra_in_candidate': extra, 'differing': differing}
    if differing:
        name = differing[0]
        la = open_bytes(os.path.join(oracle_final, name)).split(b'\n')
        lb = open_bytes(os.path.join(cand_out, name)).split(b'\n')
        for i in range(max(len(la), len(lb))):
            x = la[i] if i < len(la) else b'<end of file>'
            y = lb[i] if i < len(lb) else b'<end of file>'
            if x != y:
                res['first'] = {'file': name, 'line': i + 1, 'a': clip(x), 'b': clip(y)}
                break
    return res


def side_text(d):
    if d.get('first'):
        f = d['first']
        return f'L0 {f["file"]} line {f["line"]}: oracle `{f["a"]}` | candidate `{f["b"]}`'
    if d['missing_in_candidate']:
        return f'L0 side file missing in candidate: {d["missing_in_candidate"][0]}'
    if d['extra_in_candidate']:
        return f'L0 extra side file in candidate: {d["extra_in_candidate"][0]}'
    return None


def first_byte_diff(x, y):
    lo, hi = 0, min(len(x), len(y))
    if x[:hi] == y[:hi]:
        return hi
    while lo < hi:
        mid = (lo + hi) // 2
        if x[lo:mid + 1] == y[lo:mid + 1]:
            lo = mid + 1
        else:
            hi = mid
    return lo


# The version string in the banner, the oracle's and an engine's own.  /PTEX.Fullbanner holds it in parentheses of its own, and the two differ in length.
ORACLE_VERSION = b'(TeX Live 2026/Arch Linux)'
_L6_BANNER = re.compile(rb'(/PTEX[._]Fullbanner \(This is pdfTeX, Version [^()\\]* )\(rstex\)( kpathsea version [^()\\]*\))')
_PDF_STRING = rb'\((?:[^()\\]|\\.|\((?:[^()\\]|\\.)*\))*\)'
_L6_MASKS = [(re.compile(rb'/Producer ' + _PDF_STRING), b'/Producer ()'),
             (re.compile(rb'(/Params\s*<<[^>]*?/ModDate\s*\()D:[^)]*\)'), rb'\1)'),
             (re.compile(rb'\nxref\n.*?\ntrailer', re.S), b'\nxref\ntrailer'),
             (re.compile(rb'startxref\n\d+'), b'startxref\n0')]


def oracle_version_string():
    """ORACLE_VERSION, or another oracle's, from its banner."""
    if not SECOND_ORACLE:
        return ORACLE_VERSION
    return re.search(r'\([^()]*\)', oracle_version()['banner']).group().encode()


def l6_mask(d):
    """What L6 looks at for equal-masked: the engine's version string put back to the oracle's, the offsets it moved hidden, and the date of an embedded
    file, which is the time of the run if the run wrote it.  The oracle's cached fingerprints hold this hash, so a change here takes a new name in oracle_inst."""
    d = _L6_BANNER.sub(lambda m: m.group(1) + oracle_version_string() + m.group(2), d, count=1)
    for rx, rep in _L6_MASKS:
        d = rx.sub(rep, d)
    return d


_STREAM_START = re.compile(rb'>>\s*stream\r?\n')


def inflate_streams(data):
    """The file with every /FlateDecode stream as its plain bytes and its /Length masked, and the body of a cross-reference stream and the index of an object stream gone,
    since those hold the offsets that a different deflate or an engine's own banner moves.  L6 sets \\pdfcompresslevel=0 first, but 11 packages (pdfx, hyperxmp, ...) set it back, and then the bytes L6 sees are
    deflate output, which DESIGN section 9 says is not to be compared.  Everything else stays byte for byte."""
    out, pos = [], 0
    while True:
        m = _STREAM_START.search(data, pos)
        if not m:
            out.append(data[pos:])
            return b''.join(out)
        start = data.rfind(b' obj', pos, m.start())
        head = data[start if start >= 0 else pos:m.start()]
        if b'/FlateDecode' not in head and b'/XRef' not in head and b'/ObjStm' not in head:
            out.append(data[pos:m.end()])
            pos = m.end()
            continue
        if b'/FlateDecode' in head:
            d = zlib.decompressobj()
            try:
                plain = d.decompress(data[m.end():])
            except zlib.error:                    # not what it says it is: compared as it stands, so a damaged stream cannot pass
                out.append(data[pos:m.end()])
                pos = m.end()
                continue
            end = len(data) - len(d.unused_data)
        else:                                     # not deflated, but a cross-reference stream or the index of an object stream holds offsets all the same
            end = data.find(b'endstream', m.end())
            end, plain = (end, data[m.end():end]) if end >= 0 else (m.end(), b'')
        first = re.search(rb'/First\s+(\d+)', head)
        if b'/ObjStm' in head and first:          # the index in front, numbers and offsets into a stream that can hold the banner under \\pdfobjcompresslevel=3; the numbers stay in the bytes that follow
            plain = plain[int(first.group(1)):]
        out.append(re.sub(rb'/First\s+\d+', b'/First ', re.sub(rb'/Length\s+\d+\s*', b'/Length ', data[pos:m.start()])))
        out.append(data[m.start():m.end()])
        out.append(b'' if b'/XRef' in head else plain)
        pos = end


def compare_bytes(a, b):
    """L6.  'equal' is the real result.  'equal-masked' means only the version string in the banner and the xref offsets its
    length moves were different, which an engine with its own banner cannot avoid.  'equal-inflated' means the document turned compression back on,
    and the files agree once its streams are decompressed."""
    x, y = open_bytes(a), open_bytes(b)
    res = {'bytes_a': len(x), 'bytes_b': len(y)}
    if not x or not y:
        res['status'] = 'missing'
        return res
    if x == y:
        res['status'] = 'equal'
        return res
    if l6_mask(x) == l6_mask(y):
        # The mask hides the xref offsets, which a longer banner moves.  So an offset the mask hid has to be right on its own.
        problems = new_pdf_problems(a, b)
        if problems:
            res.update(status='differs', offset=None, a='a valid cross-reference table', b=clip(problems[0]))
            return res
        res['status'] = 'equal-masked'
        return res
    # Where they differ once the banner is masked: an engine's own banner is always the first raw difference, and says nothing.
    mx, my = l6_mask(x), l6_mask(y)
    if any(k in x for k in (b'/FlateDecode', b'/XRef', b'/ObjStm')) and any(k in y for k in (b'/FlateDecode', b'/XRef', b'/ObjStm')):
        ix, iy = l6_mask(inflate_streams(x)), l6_mask(inflate_streams(y))
        if ix == iy:
            problems = new_pdf_problems(a, b)
            if problems:
                res.update(status='differs', offset=None, a='a valid file', b=clip(problems[0]))
            else:
                res['status'] = 'equal-inflated'
            return res
    k = first_byte_diff(mx, my)
    res.update(status='differs', offset=k, a=clip(mx[max(0, k - 40):k + 60]), b=clip(my[max(0, k - 40):k + 60]))
    return res


def l6_token(a, b, masks, key, show):
    """The token of L6: the two files in the form that compare_bytes last compared them in, a line at a time, with what the machine puts there out of the way.  So this does not depend on a banner, on
    deflate or on the offsets they move.  A file that agrees and that qpdf complains about has the complaint for its difference."""
    x, y = open_bytes(a), open_bytes(b)
    mx, my = l6_mask(x), l6_mask(y)
    if any(k in x for k in (b'/FlateDecode', b'/XRef', b'/ObjStm')) and any(k in y for k in (b'/FlateDecode', b'/XRef', b'/ObjStm')):
        mx, my = l6_mask(inflate_streams(x)), l6_mask(inflate_streams(y))
    if mx == my:
        return difftoken.token_of([('qpdf', [], [p.encode() for p in new_pdf_problems(a, b)])], key, show)
    mx, my = without_res_prefix(mx), without_res_prefix(my)
    return difftoken.lines_token('L6', [masks[0].apply(k) for k in mx.split(b'\n')], [masks[1].apply(k) for k in my.split(b'\n')], key, show)


_RES_PREFIX = re.compile(rb'/Font <<\s*/[A-Za-z]+\d+([A-Za-z0-9]{6}) \d+ 0 R')


def without_res_prefix(data):
    r"""\pdfuniqueresname puts the CRC of the job id on every resource name, and the job id holds the date of the format, which is the day and the machine that built it: that goes out of a token."""
    m = _RES_PREFIX.search(data)
    return data.replace(m.group(1), b'<RES>') if m else data


def pdf_problems(path):
    """What qpdf --check complains about: a damaged file that readers would repair in silence."""
    p = subprocess.run(['qpdf', '--check', path], capture_output=True)
    text = (p.stdout + p.stderr).decode('utf-8', 'replace').replace(path, '<file>')
    return [re.sub(r'\boffset \d+', 'offset N', line) for line in text.splitlines() if line.startswith(('WARNING', 'ERROR'))]


def new_pdf_problems(oracle, candidate):
    """What qpdf --check says of the candidate and not of the oracle.  An included PDF can bring its own faults, such as a duplicated dictionary key, and the oracle copies them too."""
    known = pdf_problems(oracle)
    found = pdf_problems(candidate)
    return [line for line in found if line not in known]


# 2000-01-01, the marker that not_rewritten looks for.  FORCE_SOURCE_DATE does not pin \pdffilemoddate (tested on 1.40.29), so a document that reads the
# date of its own side file sees this; the oracle sees a time that moves, which prepare has already classified as nondeterministic.
RESTORED_MTIME_NS = 946684800 * 10**9


def restore_state(meta, out, stamp=False):
    overlay(os.path.join(meta['dir'], 'pre_last'), out)
    if stamp:
        for root, _, names in os.walk(out):
            for n in names:
                os.utime(os.path.join(root, n), ns=(RESTORED_MTIME_NS, RESTORED_MTIME_NS))
    for d in meta.get('mirrored', []):
        os.makedirs(os.path.join(out, d), exist_ok=True)


def not_rewritten(meta, out):
    """Side files the oracle wrote in its last pass and this candidate left alone.  On a converged document the
    restored copy has the right bytes, so L0 alone would pass a candidate that never writes them."""
    _, _, outputs = parse_fls(os.path.join(meta['dir'], 'final', meta['jobname'] + '.fls'))
    wd = (meta.get('workdir_path') or '') + '/'
    res = []
    for path in outputs:
        rel = path[len(wd):] if path.startswith(wd) else None
        if rel and rel in meta['side_files']:
            try:
                if os.stat(os.path.join(out, rel)).st_mtime_ns == RESTORED_MTIME_NS:
                    res.append(rel)
            except OSError:
                pass
    return sorted(res)


def cached_oracle_run(e, meta, name, prelude):
    """A derived oracle compile from the last-pass state (L2 box log, L6 PDF), cached in the oracle directory."""
    d = os.path.join(meta['dir'], name)
    done = os.path.join(d, 'done.json')
    if os.path.isfile(done):
        return read_json(done), d
    w = new_workdir(meta.get('workdir_path'))
    out = os.path.join(w, 'o')
    restore_state(meta, out)
    r = run_proc(tex_argv(e, oracle_engine(e), [], out, prelude=prelude), e['cwd'], entry_env(e), e['timeout'],
                 os.path.join(w, 'stdout'))
    tmp = d + f'.tmp{secrets.token_hex(3)}'
    os.makedirs(tmp)
    for ext in ('.pdf', '.log'):
        p = os.path.join(out, e['jobname'] + ext)
        if os.path.isfile(p):
            shutil.move(p, tmp)
    info = {'rc': r['rc'], 'ms': r['ms'], 'workdir_path': out, 'timed_out': r['timed_out']}
    write_json(os.path.join(tmp, 'done.json'), info)
    shutil.rmtree(d, ignore_errors=True)
    os.rename(tmp, d)
    shutil.rmtree(w, ignore_errors=True)
    return info, d


def run_l2(e, meta, engine, extra, timeout):
    """L2: box dumps under \\tracingoutput with depth and breadth maximised, from the same last-pass state."""
    info, d = cached_oracle_run(e, meta, 'l2', BOX_PRELUDE)
    if info['rc'] != meta['last_rc']:
        return {'status': 'oracle-run-failed', 'error': f'oracle exit {info["rc"]} with the L2 prelude, {meta["last_rc"]} without it'}
    w = new_workdir(meta.get('workdir_path'))
    out = os.path.join(w, 'o')
    restore_state(meta, out)
    r = run_proc(tex_argv(e, engine, extra, out, prelude=BOX_PRELUDE), e['cwd'], candidate_env(e, out), timeout,
                 os.path.join(w, 'stdout'))
    job = e['jobname']
    la = box_lines(normalize_log(open_bytes(os.path.join(d, job + '.log')), [info['workdir_path']]))
    lb = box_lines(normalize_log(open_bytes(os.path.join(out, job + '.log')), [out]))
    div = first_log_divergence(la, lb)
    shutil.rmtree(w, ignore_errors=True)
    if meta['log'].get('pages') and not la:
        return {'status': 'oracle-run-failed', 'error': f'the oracle shipped {meta["log"]["pages"]} pages but its log has no box dump'}
    return {'status': 'equal' if div is None else 'differs', 'box_lines_a': len(la), 'box_lines_b': len(lb),
            'candidate_rc': r['rc'], 'candidate_timed_out': r['timed_out'], 'first': div,
            'oracle_log': os.path.join(d, job + '.log')}


def l2_text(l2):
    f = l2.get('first')
    if not f:
        return None
    where = f'shipout #{f["shipout"]}' + (f' [{f["page_label"]}]' if f['page_label'] else '') if f['shipout'] else 'before the first shipout'
    return f'L2 box-log line {f["line_a"]}/{f["line_b"]}, in {where}: oracle `{clip(f["a"])}` | candidate `{clip(f["b"])}`'


def run_l6(e, meta, engine, extra, timeout):
    info, d = cached_oracle_run(e, meta, 'l6', L6_PRELUDE)
    if info['rc'] != meta['last_rc'] or not os.path.isfile(os.path.join(d, e['jobname'] + '.pdf')):
        return {'status': 'oracle-run-failed', 'error': f'oracle exit {info["rc"]} with the L6 prelude, {meta["last_rc"]} without it, or no PDF'}
    w = new_workdir(meta.get('workdir_path'))
    out = os.path.join(w, 'o')
    restore_state(meta, out)
    r = run_proc(tex_argv(e, engine, extra, out, prelude=L6_PRELUDE), e['cwd'], candidate_env(e, out), timeout,
                 os.path.join(w, 'stdout'))
    res = compare_bytes(os.path.join(d, e['jobname'] + '.pdf'), os.path.join(out, e['jobname'] + '.pdf'))
    res['candidate_rc'] = r['rc']
    shutil.rmtree(w, ignore_errors=True)
    return res


INST_ENV = {'max_print_line': '1000000'}          # no wrapped lines, so a log of hundreds of MB can be read a line at a time
normalize_stream = lognorm.normalize_stream


split_box_lines = lognorm.split_box_lines


def stream_divergence(a, b):
    """first_log_divergence over two iterables of (no, line), without holding them."""
    shipouts, label, context = 0, None, []
    sentinel = (None, None)
    while True:
        x, y = next(a, sentinel), next(b, sentinel)
        if x is sentinel and y is sentinel:
            return None
        if x[1] != y[1]:
            if x[1] is not None:
                m = re.match(rb'Completed box being shipped out \[([^\]]*)\]', x[1])
                if m:
                    shipouts += 1
                    label = m.group(1).decode('latin-1')
            return {'line_a': x[0], 'line_b': y[0], 'a': clip(x[1], 400) if x[1] is not None else '<end of log>',
                    'b': clip(y[1], 400) if y[1] is not None else '<end of log>', 'shipout': shipouts, 'page_label': label,
                    'context': [clip(c, 200) for c in context]}
        m = re.match(rb'Completed box being shipped out \[([^\]]*)\]', x[1])
        if m:
            shipouts += 1
            label = m.group(1).decode('latin-1')
        context = context[-2:] + [x[1]]


def log_fingerprint(path, workdir):
    """(normal lines, hash per block).  Over a megabyte this runs in a process of its own, see lognorm.py."""
    try:
        size = os.path.getsize(path)
    except OSError:
        return 0, ['missing']
    if size < 1 << 20:
        return lognorm.fingerprint(path, workdir)
    p = subprocess.run([sys.executable, '-I', os.path.join(os.path.dirname(os.path.abspath(__file__)), 'lognorm.py'), path, workdir], capture_output=True, timeout=3600)
    r = json.loads(p.stdout)
    return r['lines'], r['blocks']


def inst_prelude(want):
    return (L6_PRELUDE if '6' in want else '') + (BOX_PRELUDE if '2' in want else '') + (L1_PRELUDE if '1' in want else '')


def inst_env(env):
    env = dict(env)
    env.update(INST_ENV)
    return env


def inst_fingerprint(e, out, r, want):
    job = e['jobname']
    n, blocks = log_fingerprint(os.path.join(out, job + '.log'), out)
    res = {'rc': r['rc'], 'signal': r['signal'], 'timed_out': r['timed_out'], 'lines': n, 'blocks': blocks}
    pdf = os.path.join(out, job + '.pdf')
    if '6' in want and os.path.isfile(pdf):
        data = open_bytes(pdf)
        res.update(pdf_sha=hashlib.sha256(data).hexdigest(), pdf_masked_sha=hashlib.sha256(l6_mask(data)).hexdigest(), pdf_bytes=len(data))
    return res


def oracle_inst(e, meta, want):
    """The oracle's instrumented pass, twice: a trace can move between clean runs where the page does not (a random seed nothing prints),
    and then it cannot judge anyone.  Only fingerprints are cached; the log itself comes back from a new run if a candidate differs."""
    name = 'inst4-' + ''.join(sorted(want))
    d = os.path.join(meta['dir'], name)
    done = os.path.join(d, 'done.json')
    if os.path.isfile(done):
        return read_json(done)
    runs = []
    for _ in range(2):
        w = new_workdir(meta.get('workdir_path'))
        out = os.path.join(w, 'o')
        restore_state(meta, out)
        r = run_proc(tex_argv(e, oracle_engine(e), [], out, recorder=reads_own_fls(meta), prelude=inst_prelude(want)), e['cwd'],
                     inst_env(entry_env(e)), e['timeout'] * 5, os.path.join(w, 'stdout'))
        runs.append(inst_fingerprint(e, out, r, want))
        shutil.rmtree(w, ignore_errors=True)
    info = runs[0]
    info['log_stable'] = runs[0]['blocks'] == runs[1]['blocks'] and runs[0]['lines'] == runs[1]['lines']
    info['pdf_stable'] = runs[0].get('pdf_sha') == runs[1].get('pdf_sha')
    info['want'] = want
    # This stays for every later run, on every checkout.  A kill from outside says nothing about the document: an interrupted compare kills what it runs, and that left an entry unjudged for good.
    if _stop.is_set() or any(r['signal'] in (signal.SIGKILL, signal.SIGTERM) for r in runs):
        return info
    tmp = d + f'.tmp{secrets.token_hex(3)}'
    os.makedirs(tmp)
    write_json(os.path.join(tmp, 'done.json'), info)
    shutil.rmtree(d, ignore_errors=True)
    os.rename(tmp, d)
    return info


def inst_tokens(pa, oo, pb, out, e, opts, shown):
    """The tokens of L1 and L2, from the logs of the two instrumented runs.  Over a megabyte in a process of its own, as in log_fingerprint."""
    spec = {'key': private_key(e), 'paths': [(p, s.decode()) for p, s in machine_paths(e) if p], 'candidate': [(d, '<ENGINE>') for d in engine_dirs(opts)]}
    show = shown.dir if shown else None
    if max(os.path.getsize(pa), os.path.getsize(pb)) < 1 << 20:
        return difftoken.inst_tokens(pa, oo, pb, out, spec, show)
    tool = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'difftoken.py')
    p = subprocess.run([sys.executable, '-I', tool, 'inst', pa, oo, pb, out, json.dumps(spec)] + ([show] if show else []), capture_output=True, timeout=3600)
    return json.loads(p.stdout)


def run_instrumented(e, meta, engine, extra, timeout, want, opts=None, shown=None):
    """One extra pass on each side from the last-pass state, doing the work of L1, L2 and L6 at once: assignments and restores, box dumps
    and uncompressed bytes.  Returns {level: result}.  A level the oracle cannot judge says why, and does not pass by silence.  With opts, for an entry whose tokens are wanted,
    res['tokens'] has them for the levels that differ."""
    ora = oracle_inst(e, meta, want)
    unavailable = None
    if ora['timed_out'] or ora['signal'] or ora['rc'] != meta['last_rc']:
        unavailable = f'oracle exit {ora["rc"]} signal {ora["signal"]} timed_out {ora["timed_out"]} with the instrumentation prelude, exit {meta["last_rc"]} without it'
    res = {}
    if unavailable:
        return {lv: {'status': 'unavailable', 'why': unavailable} for lv in ('L1', 'L2', 'L6') if lv[1] in want}
    w = new_workdir(meta.get('workdir_path'))
    out = os.path.join(w, 'o')
    job = e['jobname']
    try:
        restore_state(meta, out)
        prelude = inst_prelude(want)
        r = run_proc(tex_argv(e, engine, extra, out, recorder=reads_own_fls(meta), prelude=prelude), e['cwd'],
                     inst_env(candidate_env(e, out)), timeout, os.path.join(w, 'stdout'))
        cand = inst_fingerprint(e, out, r, want)
        log_same = cand['lines'] == ora['lines'] and cand['blocks'] == ora['blocks']
        if '6' in want:
            if ora.get('pdf_sha') is None:
                res['L6'] = {'status': 'unavailable', 'why': 'the oracle wrote no PDF with the prelude'}
            elif not ora['pdf_stable']:
                res['L6'] = {'status': 'unavailable', 'why': 'the oracle\'s uncompressed bytes differ between two clean runs'}
            elif cand.get('pdf_sha') == ora['pdf_sha']:
                res['L6'] = {'status': 'equal', 'bytes_a': ora['pdf_bytes'], 'bytes_b': cand['pdf_bytes']}
            elif cand.get('pdf_masked_sha') == ora['pdf_masked_sha'] and not pdf_problems(os.path.join(out, job + '.pdf')):
                res['L6'] = {'status': 'equal-masked', 'bytes_a': ora['pdf_bytes'], 'bytes_b': cand['pdf_bytes']}
            # Any other case goes on below against a new run of the oracle.  That includes a complaint from qpdf, which may be about what the oracle's file has too.
        need_rerun = (not log_same and ora['log_stable']) or ('L6' not in res and '6' in want)
        wo = None
        if need_rerun:
            wo = new_workdir(meta.get('workdir_path'))
            oo = os.path.join(wo, 'o')
            restore_state(meta, oo)
            run_proc(tex_argv(e, oracle_engine(e), [], oo, recorder=reads_own_fls(meta), prelude=prelude), e['cwd'], inst_env(entry_env(e)),
                     e['timeout'] * 5, os.path.join(wo, 'stdout'))
        tokens = {}
        if '6' in want and 'L6' not in res:
            res['L6'] = compare_bytes(os.path.join(oo, job + '.pdf'), os.path.join(out, job + '.pdf'))
            if opts and res['L6']['status'] in ('differs', 'missing'):
                show = shown.sink() if shown else None
                tokens['L6'] = l6_token(os.path.join(oo, job + '.pdf'), os.path.join(out, job + '.pdf'), token_masks(e, opts, oo, out), private_key(e), show) or NOTHING_LEFT
                if shown:
                    shown.write('L6', show)
        pa, pb = (os.path.join(oo, job + '.log'), os.path.join(out, job + '.log')) if need_rerun else (None, None)
        # No log at all is an answer about the candidate (it stopped before opening one), and about the run if it is the oracle's.
        oracle_lost = need_rerun and not log_same and not os.path.isfile(pa)
        no_log = {'line_a': 1, 'line_b': None, 'a': 'a log', 'b': '<no log file>', 'shipout': 0, 'page_label': None, 'context': []}
        cand_lost = need_rerun and not log_same and not oracle_lost and not os.path.isfile(pb)
        if '2' in want:
            if not ora['log_stable']:
                res['L2'] = {'status': 'unavailable', 'why': 'the oracle\'s trace differs between two clean runs'}
            elif oracle_lost:
                res['L2'] = {'status': 'unavailable', 'why': 'the oracle left no log when run again'}
            elif cand_lost:
                res['L2'] = {'status': 'differs', 'first': no_log, 'candidate_rc': r['rc'], 'candidate_timed_out': r['timed_out']}
            elif log_same:
                res['L2'] = {'status': 'equal', 'first': None}
            else:
                only_box = lambda p, wd: (x for inside, x in split_box_lines(normalize_stream(p, wd)) if inside)
                div = stream_divergence(only_box(pa, oo), only_box(pb, out))
                res['L2'] = {'status': 'equal' if div is None else 'differs', 'first': div, 'candidate_rc': r['rc'], 'candidate_timed_out': r['timed_out']}
        if '1' in want:
            if not ora['log_stable']:
                res['L1'] = {'status': 'unavailable', 'why': 'the oracle\'s trace differs between two clean runs'}
            elif oracle_lost:
                res['L1'] = {'status': 'unavailable', 'why': 'the oracle left no log when run again'}
            elif cand_lost:
                res['L1'] = {'status': 'differs', 'first': no_log, 'lines': ora['lines']}
            elif log_same:
                res['L1'] = {'status': 'equal', 'first': None, 'lines': ora['lines']}
            else:
                not_box = lambda p, wd: (x for inside, x in split_box_lines(normalize_stream(p, wd)) if not inside)
                div = stream_divergence(not_box(pa, oo), not_box(pb, out))
                res['L1'] = {'status': 'equal' if div is None else 'differs', 'first': div, 'lines': ora['lines']}
        if opts and need_rerun and not log_same and ora['log_stable'] and not oracle_lost:
            if cand_lost:
                tokens.update({lv: '0/1' for lv in ('L1', 'L2') if lv in res})
            else:
                got = inst_tokens(pa, oo, pb, out, e, opts, shown)
                tokens.update({lv: got.get(lv) or NOTHING_LEFT for lv in ('L1', 'L2') if res.get(lv, {}).get('status') == 'differs'})
        if tokens:
            res['tokens'] = tokens
        if wo:
            shutil.rmtree(wo, ignore_errors=True)
        return res
    finally:
        shutil.rmtree(w, ignore_errors=True)


def normalized_fls(path, workdir):
    lines = []
    for line in open_bytes(path).split(b'\n'):
        if line.startswith((b'PWD ',)) or b'texmf.cnf' in line:
            continue
        lines.append(line.replace(os.fsencode(workdir), b'#'))
    return lines


# ------------------------------------------------------------------------------------------------ compare stage
def reads_own_fls(meta):
    """The oracle always runs with -recorder.  A document that reads its own .fls (the currfile package does) behaves differently
    without one, so such a document needs -recorder on the candidate too."""
    return re.search(rb'^INPUT .*' + re.escape(os.fsencode(meta['jobname'] + '.fls')) + rb'$',
                     open_bytes(os.path.join(meta['dir'], 'final', meta['jobname'] + '.fls')), re.M) is not None


def candidate_timeout(e, opts):
    t = e['timeout'] * opts.timeout_scale
    cap = getattr(opts, 'timeout_cap', None)
    return min(t, cap) if cap else t


def run_candidate(e, meta, opts, mode):
    """Returns (work, out, last result, wall ms of all passes)."""
    w = new_workdir(meta.get('workdir_path'))
    out = os.path.join(w, 'o')
    env = candidate_env(e, out)
    timeout = candidate_timeout(e, opts)
    argv = tex_argv(e, opts.engine, opts.engine_arg, out, recorder=opts.fls or reads_own_fls(meta))
    if mode == 'last-pass':
        restore_state(meta, out, stamp=True)
        r = run_proc(argv, e['cwd'], env, timeout, os.path.join(w, 'stdout'))
        return w, out, r, r['ms']
    overlay(os.path.join(meta['dir'], 'seed'), out)
    for d in meta.get('mirrored', []):
        os.makedirs(os.path.join(out, d), exist_ok=True)
    total = 0.0
    r = None
    env = dict(env, RSTEX_NO_SNAPSHOT='1')      # pass 2 in one directory with one cache would be a restore, and this judges cold runs
    for i in range(1, meta['passes_run'] + 1):
        remove_quietly(os.path.join(out, e['jobname'] + '.pdf'))
        r = run_proc(argv, e['cwd'], env, timeout, os.path.join(w, 'stdout'))
        total += r['ms']
        if r['timed_out'] or r['signal'] or r['rc'] >= 126:
            return w, out, r, total
        overlay(os.path.join(meta['dir'], 'gen', str(i)), out)
    return w, out, r, total


def crash_reason(r, stdout_path, log_path):
    bits = []
    if r.get('exec_error'):
        bits.append(f'could not execute: {r["exec_error"]}')
    if r['signal']:
        bits.append(f'killed by signal {r["signal"]}')
    elif r['rc'] is not None and r['rc'] >= 100:
        bits.append(f'exit {r["rc"]}')
    return '; '.join(bits)


def tail_text(path, n=400):
    d = open_bytes(path)
    return clip(d[-n:], n) if d else ''


def check_failed_oracle(e, meta, opts):
    """An entry the oracle cannot finish: a candidate must not crash or hang on it, and it must stop in the same way, with the same log and
    exit status.  Returns (a crash or hang, the log's result, exit statuses that differ).  Not run when the oracle itself timed out or died:
    that would only spend the timeout again, and leaves no log to compare."""
    reason = meta.get('class_reason') or ''
    if 'timeout' in reason or 'signal' in reason or (e.get('main') and not os.path.isfile(e['main'])):
        return None, None, None
    w, out, r, _ = run_candidate(e, meta, opts, 'last-pass')
    tokens_on = e['id'] in getattr(opts, 'known', {}) or getattr(opts, 'tokens', False)
    try:
        if r['timed_out']:
            return f'timed out after {candidate_timeout(e, opts):g} s', None, None
        crash = crash_reason(r, None, None)
        if crash:
            return crash, None, None
        job = meta['jobname']
        oracle_log = os.path.join(meta['dir'], 'final', job + '.log')
        log = None
        if os.path.isfile(oracle_log) and (meta.get('determinism') or {}).get('log_equal', True):
            ld = first_log_divergence(judged_log(e, open_bytes(oracle_log), [meta.get('workdir_path')]),
                                      judged_log(e, open_bytes(os.path.join(out, job + '.log')), [out]))
            log = {'status': 'equal' if ld is None else 'differs', 'first': ld, 'form': e['log_normalize']}
            if tokens_on:
                raw = open_bytes(oracle_log), open_bytes(os.path.join(out, job + '.log'))
                log['tokens'], log['judged'] = log_tokens(e, opts, meta, out, raw[0], raw[1], Shown(e, opts, 'last-pass'))
                log['judged'].add('exit')
                if meta.get('last_rc') != r['rc']:
                    log['tokens']['exit'] = f'{meta.get("last_rc")}/{r["rc"]}'
        return None, log, None if meta.get('last_rc') == r['rc'] else f'exit status: oracle {meta.get("last_rc")}, candidate {r["rc"]}'
    finally:
        shutil.rmtree(w, ignore_errors=True)


def log_tokens(e, opts, meta, out, raw_a, raw_b, shown):
    """The tokens of the log, in the form it is judged in, and for a regression test's own form of it in the whole log too; and which of these were judged.  A log that moves between two clean
    runs of the oracle judges no one."""
    tokens, judged = {}, set()
    masks = token_masks(e, opts, meta.get('workdir_path'), out)
    max_line = int(e['env'].get('max_print_line', 79))
    forms = [('log', 'log_equal', lambda data, w: [x for _, x in judged_log(e, data, [w])])]
    if e['log_normalize'] != 'default':
        forms.append(('log_strict', 'log_strict_equal', lambda data, w: [x for _, x in normalize_log(data, [w])]))
    for level, stable, form in forms:
        if not (meta.get('determinism') or {}).get(stable, True):
            continue
        judged.add(level)
        show = shown.sink()
        t = difftoken.lines_token(level, difftoken.logical_lines(form(raw_a, meta.get('workdir_path')), max_line, masks[0]),
                                  difftoken.logical_lines(form(raw_b, out), max_line, masks[1]), private_key(e), show)
        if t:
            tokens[level] = t
            shown.write(level, show)
    return tokens, judged


def side_token(e, opts, meta, out, final, d0, shown):
    """The token of L0: what is missing, extra, not written, and each file that differs, a line at a time."""
    masks = token_masks(e, opts, meta.get('workdir_path'), out)
    parts = [(f'missing {n}', [n.encode()], []) for n in d0['missing_in_candidate']] + [(f'extra {n}', [], [n.encode()]) for n in d0['extra_in_candidate']]
    for n in d0['differing']:
        a = [masks[0].apply(x) for x in open_bytes(os.path.join(final, n)).split(b'\n')]
        b = [masks[1].apply(x) for x in open_bytes(os.path.join(out, n)).split(b'\n')]
        d = difftoken.between(a, b)
        if d:
            parts.append((n, d[0], d[1]))
    parts += [(f'not rewritten {n}', [], [n.encode()]) for n in d0.get('not_rewritten', ())]
    show = shown.sink()
    t = difftoken.token_of(parts, private_key(e), show)
    if t:
        shown.write('L0', show)
    return t


def compare_entry(e, opts, mode):
    base = {'id': e['id'], 'source': e['source'], 'tier': e['tier'], 'mode': mode, 'entry_mode': e['mode'],
            'manifest': e['manifest'], 'notes': e.get('notes'), 'levels': {}}
    if e.get('excluded'):
        return {**base, 'outcome': 'excluded', 'first_divergence': {'text': f'excluded by manifest: {e["excluded"]}'}}
    meta, reused = prepare_entry(e, retry_fails=getattr(opts, 'retry_oracle_fails', False))
    cls = effective_class(meta, e)
    base.update(oracle_key=meta['key'], oracle_class=cls, oracle_reused=reused,
                oracle_ms=meta['pass_ms'][-1] if mode == 'last-pass' and meta['pass_ms'] else sum(meta['pass_ms']),
                oracle_errors=meta['log'].get('errors'), pages=meta['log'].get('pages'), oracle_passes=meta.get('passes_run'),
                class_mismatch=e['expected_class'] != meta['observed_class'],
                oracle_judged_failure=meta.get('judged_failure'), units_total=len(e.get('units', ())))
    if cls == 'oracle-fails':
        crash, log, rc = check_failed_oracle(e, meta, opts) if mode == 'last-pass' else (None, None, None)
        if crash:
            return {**base, 'outcome': 'candidate-crash', 'first_divergence': {'text': f'the oracle fails cleanly here ({clip(meta.get("class_reason") or "", 120)}); the candidate: {crash}'}}
        tokens, judged = (log.pop('tokens', {}), log.pop('judged', set())) if log else ({}, set())
        if log:
            base['levels']['log'] = log
        ld = log['first'] if log else None
        if ld or rc:
            text = f'the oracle fails here too ({clip(meta.get("class_reason") or "", 120)}), differently: ' + \
                (f'log line {ld["line_a"]}/{ld["line_b"]}: oracle `{clip(ld["a"])}` | candidate `{clip(ld["b"])}`' if ld else rc)
            res = {**base, 'outcome': 'differs-log-only', 'first_divergence': {'text': text}}
            list_verdict(res, e, opts, mode, tokens, judged)
            return res
        res = {**base, 'outcome': 'oracle-fails', 'first_divergence': {'text': meta.get('class_reason') or 'oracle failed'}}
        list_verdict(res, e, opts, mode, tokens, judged)
        return res
    job = meta['jobname']
    cwd_before = names_of(e['cwd'])
    w, out, r, ms = run_candidate(e, meta, opts, mode)
    # An entry on the list says what differs, level by level, so here each level says all of it.  And it says it on any machine: L1 and L2 run on such an entry whatever the oracle takes.
    tokens_on = e['id'] in getattr(opts, 'known', {}) or getattr(opts, 'tokens', False)
    tokens, judged, shown = {}, set(), Shown(e, opts, mode)
    try:
        # Links of its own to the oracle's final files: on a machine that runs several of these, another may replace an oracle during a compare.
        final = os.path.join(w, 'oracle-final')
        shutil.copytree(os.path.join(meta['dir'], 'final'), final, copy_function=os.link)
        oracle_pdf = os.path.join(final, job + '.pdf')
        has_oracle_pdf = os.path.isfile(oracle_pdf) and os.path.getsize(oracle_pdf) > 0     # a package can leave an empty file
        res = dict(base)
        res.update(candidate_ms=ms, candidate_rc=r['rc'], candidate_cpu_ms=r['cpu_ms'], candidate_signal=r['signal'],
                   speedup=round(base['oracle_ms'] / ms, 3) if ms and base['oracle_ms'] else None)
        stdout_path = os.path.join(w, 'stdout')
        log_path = os.path.join(out, job + '.log')
        cand_pdf = os.path.join(out, job + '.pdf')
        if r['timed_out']:
            res.update(outcome='candidate-timeout',
                       first_divergence={'text': f'candidate timed out after {candidate_timeout(e, opts):g} s'})
            return finish_entry(res, e, w, out, job, opts)
        reason = crash_reason(r, stdout_path, log_path)
        if reason:
            res.update(outcome='candidate-crash',
                       first_divergence={'text': f'{reason}; output tail: {tail_text(stdout_path)}'})
            return finish_entry(res, e, w, out, job, opts)
        has_cand_pdf = os.path.isfile(cand_pdf) and os.path.getsize(cand_pdf) > 0
        if has_oracle_pdf and not has_cand_pdf:
            res.update(outcome='candidate-crash',
                       first_divergence={'text': f'exit {r["rc"]}, no PDF produced; log tail: {tail_text(log_path, 200)}'})
            return finish_entry(res, e, w, out, job, opts)
        if cls == 'nondeterministic':
            res.update(outcome='nondeterministic',
                       first_divergence={'text': meta.get('class_reason') or 'labelled nondeterministic in the manifest'})
            return finish_entry(res, e, w, out, job, opts, keep=False)

        divs = []                                 # (severity, text): 0 content, 1 structure
        levels = res['levels']

        # L4/L5
        pdf_parts = None
        if has_oracle_pdf and has_cand_pdf and e['compare'] == 'pdf':
            differences = os.path.join(w, 'differences.json') if tokens_on else None
            c = run_compare_pdf(oracle_pdf, cand_pdf, differences)
            levels['L4_L5'] = c
            if c['status'] == 'unreadable-pdf':
                res.update(outcome='candidate-crash', first_divergence={'text': f'candidate PDF unreadable: {c["error"]}'})
                return finish_entry(res, e, w, out, job, opts)
            if c['status'] == 'tool-error':
                res.update(outcome='harness-error', first_divergence={'text': f'compare_pdf.py failed: {c["error"]}'})
                return finish_entry(res, e, w, out, job, opts)
            if c.get('first_content') or c['pages_a'] != c['pages_b']:
                divs.append((0, c['text']))
            elif c['status'] != 'identical':
                divs.append((1, c['text']))
            if tokens_on:
                judged |= {'L4', 'L5'}
                if c['status'] != 'identical':
                    pdf_parts = pdf_differences(differences)
            # L7
            if opts.raster == 'all':
                try:
                    ra = compare_raster(meta['dir'], oracle_pdf, cand_pdf, opts.dpi)
                    levels['L7'] = ra
                    if ra['diff_pages']:
                        p = ra['diff_pages'][0]
                        divs.append((0, f'L7 raster differs on {len(ra["diff_pages"])} pages, first page {p} '
                                        f'({ra["percent"].get(str(p), "?")}% of pixels)'))
                    if ra['pages_a'] != ra['pages_b'] and c['pages_a'] == c['pages_b']:
                        divs.append((0, f'L7 rendered page count: oracle {ra["pages_a"]}, candidate {ra["pages_b"]}'))
                except (ValueError, OSError) as ex:
                    levels['L7'] = {'error': str(ex)}
                    divs.append((0, f'L7 raster failed: {ex}'))
            else:
                levels['L7'] = {'skipped': 'by --raster none'}
        elif has_oracle_pdf != has_cand_pdf and e['compare'] == 'pdf':
            divs.append((0, 'candidate produced a PDF where the oracle had no pages of output'))
            if tokens_on:
                judged |= {'L4', 'L5'}
                pdf_parts = pdf_differences(os.path.join(w, 'differences.json'), alone=cand_pdf)

        if pdf_parts:
            masks = token_masks(e, opts, meta.get('workdir_path'), out)
            for level in ('L4', 'L5'):
                parts = [(where, [masks[0].apply(x) for x in a], [masks[1].apply(x) for x in b]) for where, a, b in pdf_parts[level] if not where.endswith('//Params//ModDate')]     # the time of the run
                show = shown.sink()
                t = difftoken.token_of(parts, private_key(e), show)
                if t:
                    tokens[level] = t
                    shown.write(level, show)

        # Log.  Always measured; it gates only where the log is the only output, or on request.
        raw_a, raw_b = open_bytes(os.path.join(final, job + '.log')), open_bytes(log_path)
        la = judged_log(e, raw_a, [meta.get('workdir_path')])
        lb = judged_log(e, raw_b, [out])
        ld = first_log_divergence(la, lb)
        levels['log'] = {'status': 'equal' if ld is None else 'differs', 'first': ld, 'form': e['log_normalize']}
        # The log is an output like any other: a wrong line number in an error message is a bug.  It is judged as the main output where the
        # log is what the entry exercises, and otherwise gives its own outcome, differs-log-only.  A log that moves between two clean
        # oracle runs cannot judge anyone, and is noted instead.
        log_is_output = e['gate_log'] or e['compare'] == 'log' or not has_oracle_pdf
        log_stable = (meta.get('determinism') or {}).get('log_equal', True)
        gate_log = log_is_output or (opts.gate_log and log_stable)
        if ld and gate_log:
            divs.append((0 if log_is_output else 2, f'log line {ld["line_a"]}/{ld["line_b"]}: oracle `{clip(ld["a"])}` | candidate `{clip(ld["b"])}`'))
        if ld and not gate_log:
            res['log_note'] = f'log line {ld["line_a"]}/{ld["line_b"]}: oracle `{clip(ld["a"], 120)}` | candidate `{clip(ld["b"], 120)}`'
            if not log_stable:
                res['log_note'] += ' (not gating: the oracle\'s own log differs between two clean runs)'
        if tokens_on:
            log_tok, log_judged = log_tokens(e, opts, meta, out, raw_a, raw_b, shown)
            tokens.update(log_tok)
            judged |= log_judged
        if e['log_normalize'] != 'default' and not ld:
            # The l3build form hides line numbers, versions and what comes before \\START, so the whole log is measured as well.
            # A difference there is a finding, not a pass: a note by default, a structure difference under --gate-log.
            sd = first_log_divergence(normalize_log(raw_a, [meta.get('workdir_path')]), normalize_log(raw_b, [out]))
            levels['log_strict'] = {'status': 'equal' if sd is None else 'differs', 'first': sd}
            if sd:
                res['log_note'] = f'whole log line {sd["line_a"]}/{sd["line_b"]}: oracle `{clip(sd["a"], 120)}` | candidate `{clip(sd["b"], 120)}`'
                if opts.gate_log and (meta.get('determinism') or {}).get('log_strict_equal', True):
                    divs.append((1, res['log_note']))

        # Format files must exist; their bytes cannot be compared across engines.
        if e['mode'] == 'ini-format' and not os.path.isfile(os.path.join(out, job + '.fmt')):
            divs.append((0, 'candidate did not produce a .fmt file'))
            tokens['fmt'] = 'none'

        # L0
        d0 = side_diff(final, out, job, meta['side_files'])
        levels['L0'] = d0
        if d0['missing_in_candidate'] or d0['extra_in_candidate'] or d0['differing']:
            divs.append((1, side_text(d0)))
        if mode == 'last-pass':
            d0['not_rewritten'] = not_rewritten(meta, out)
            if d0['not_rewritten']:
                divs.append((1, f'L0 candidate never wrote {d0["not_rewritten"][0]}, which the oracle writes; the file only holds the restored copy'))
        if tokens_on:
            judged.add('L0')
            t = side_token(e, opts, meta, out, final, d0, shown)
            if t:
                tokens['L0'] = t
        scribbled = sorted(names_of(e['cwd']) - cwd_before)
        if scribbled:
            levels['source_dir'] = {'new_files': scribbled}
            divs.append((1, f'candidate created files in the source directory {e["cwd"]}: {", ".join(scribbled[:3])}'))
            tokens['source'] = ','.join(scribbled)     # no list entry can name this: it fails
        if opts.fls:
            fa, fb = normalized_fls(os.path.join(final, job + '.fls'), meta.get('workdir_path')), normalized_fls(os.path.join(out, job + '.fls'), out)
            if fa != fb:
                k = next((i for i in range(min(len(fa), len(fb))) if fa[i] != fb[i]), min(len(fa), len(fb)))
                t = f'L0 .fls line {k + 1}: oracle `{clip(fa[k]) if k < len(fa) else "<end>"}` | candidate `{clip(fb[k]) if k < len(fb) else "<end>"}`'
                levels['L0_fls'] = {'status': 'differs', 'text': t}
                divs.append((1, t))
                tokens['fls'] = 'differs'
            else:
                levels['L0_fls'] = {'status': 'equal', 'lines': len(fa)}

        # exit status
        if tokens_on:
            judged.add('exit')
        if meta['last_rc'] != r['rc']:
            divs.append((1, f'exit status: oracle {meta["last_rc"]}, candidate {r["rc"]}'))
            tokens['exit'] = f'{meta["last_rc"]}/{r["rc"]}'

        # L1, L2, L6: one instrumented pass per side, from the last-pass state, once per entry however many modes run
        want = ''
        modes_here = getattr(opts, 'modes', [mode])
        if e['mode'] not in ('ini', 'ini-format') and (mode == 'last-pass' or len(modes_here) == 1):
            if (e['l6'] or opts.l6) and has_oracle_pdf and e['compare'] == 'pdf':
                want += '6'
            if opts.box_log == 'always' or (opts.box_log == 'failed' and (divs or tokens_on)):
                want += '2'
            # CPU time, not wall clock: which entries L1 judges must not depend on how busy the machine was when the oracle was prepared.  It does depend on how fast the machine is, so an entry
            # whose tokens are wanted has it in any case: two machines judge the same levels of it.
            last_ms = (meta.get('pass_cpu_ms') or meta.get('pass_ms') or [0])[-1]
            if opts.l1 and (last_ms <= opts.l1_max_ms or tokens_on):
                want += '1'
            elif opts.l1:
                levels['L1'] = {'status': 'skipped', 'why': f'the oracle takes {last_ms:.0f} ms of CPU a pass, over --l1-max-ms {opts.l1_max_ms:g}'}
        if want:
            inst = run_instrumented(e, meta, opts.engine, opts.engine_arg, candidate_timeout(e, opts) * 5, want, opts if tokens_on else None, shown)
            tokens.update(inst.pop('tokens', {}))
            judged |= {k for k, v in inst.items() if v['status'] not in ('unavailable', 'skipped')}
            levels.update(inst)
            d6 = inst.get('L6')
            if d6 and d6['status'] in ('differs', 'missing'):
                divs.append((1, f'L6 uncompressed bytes differ at offset {d6.get("offset")}: oracle `{d6.get("a")}` | candidate `{d6.get("b")}`'))
            d1 = inst.get('L1')
            if d1 and d1['status'] == 'differs':
                f = d1['first']
                divs.append((1, f'L1 trace line {f["line_a"]}/{f["line_b"]}: oracle `{clip(f["a"])}` | candidate `{clip(f["b"])}`'))
            d2 = inst.get('L2')
            if d2 and d2['status'] == 'differs':
                divs.append((1, l2_text(d2)))
            own_date = levels.get('L4_L5') or {}
            if own_date.get('structure_paths') and (1, own_date.get('text')) in divs and \
                    len(own_date['structure_paths']) == own_date['structure_diffs'] and all(p.endswith('//Params//ModDate') for p in own_date['structure_paths']):
                # An embedded file the run itself wrote has the date of that run: FORCE_SOURCE_DATE does not pin \pdffilemoddate.  The oracle's own
                # two runs differ there, as soon as they are a second apart, and L6 masks that date as well.
                divs.remove((1, own_date['text']))
                res['log_note'] = 'an embedded file has another /ModDate, which for one the run itself wrote is the time of the run, on both sides'
            missing = sorted(k for k, v in inst.items() if v['status'] == 'unavailable')
            if missing:
                res['inst_note'] = f'{", ".join(missing)} could not judge this entry: {inst[missing[0]].get("why")}'

        content = [t for s, t in divs if s == 0]
        struct = [t for s, t in divs if s == 1]
        logonly = [t for s, t in divs if s == 2]
        res['outcome'] = 'differs-content' if content else 'differs-structure-only' if struct else 'differs-log-only' if logonly else 'identical'
        if divs:
            fd = {'text': (content + struct + logonly)[0], 'all': content + struct + logonly}
            c = levels.get('L4_L5') or {}
            if c.get('first_content'):
                fd.update(page=c['first_content']['page'], operator=c['first_content']['operator'],
                          a=c['first_content']['a'], b=c['first_content']['b'], context=c['first_content']['context'])
            unit = divergence_unit(e, fd, ld, os.path.join(final, job + '.log'))
            if unit:
                fd['unit'] = unit
            res['first_divergence'] = fd
        if res.get('first_divergence') and levels.get('L2', {}).get('status') == 'differs':
            res['first_divergence']['box_log'] = l2_text(levels['L2'])
        if tokens_on:
            # Pixels depend on the machine's poppler, so L7 has no token.  A page that renders otherwise has to be one whose content, attributes or fonts are already another by L4 or L5.
            ra = levels.get('L7') or {}
            c = levels.get('L4_L5') or {}
            parts = (pdf_parts or {}).get('L5', ())
            pages = set(c.get('diff_pages', ())) | {int(w.split()[1]) for w, _, _ in parts if w.startswith('page ')}
            if ra.get('diff_pages') and not any(not w.startswith('page ') for w, _, _ in parts) and not set(ra['diff_pages']) <= pages:
                tokens['L7'] = 'pages ' + ' '.join(str(x) for x in sorted(set(ra['diff_pages']) - pages))
            list_verdict(res, e, opts, mode, tokens, judged)
        return finish_entry(res, e, w, out, job, opts)
    except Exception:
        return {**base, 'outcome': 'harness-error', 'first_divergence': {'text': clip(traceback.format_exc(), 1200)}}
    finally:
        shutil.rmtree(w, ignore_errors=True)


_UNIT_MARK = re.compile(rb'\[\[unit ([^\]\n]*)\]\]')


def divergence_unit(e, fd, ld, oracle_log):
    """Which test inside a batch entry diverged.  "units" lists what each page exercises, in page order; a log
    divergence is located by the last [[unit NAME]] line the oracle's log has before it."""
    units = e.get('units')
    if not units:
        return None
    if fd.get('page') and fd['page'] <= len(units):
        return units[fd['page'] - 1]
    if ld and ld.get('line_a'):
        lines = open_bytes(oracle_log).split(b'\n')[:ld['line_a']]
        for line in reversed(lines):
            m = _UNIT_MARK.search(line)
            if m:
                return m.group(1).decode('latin-1')
    return None


def finish_entry(res, e, w, out, job, opts, keep=True):
    """Keep the failing candidate's outputs in the run directory."""
    if keep and res['outcome'] != 'identical' and opts.run_dir:
        d = os.path.join(opts.run_dir, 'fail', slug(e['id']), res['mode'])
        try:
            os.makedirs(d, exist_ok=True)
            for root, _, names in os.walk(out):
                for n in names:
                    p = os.path.join(root, n)
                    t = os.path.join(d, os.path.relpath(p, out))
                    os.makedirs(os.path.dirname(t), exist_ok=True)
                    shutil.copy2(p, t)
            s = os.path.join(w, 'stdout')
            if os.path.isfile(s):
                shutil.copy2(s, os.path.join(d, 'candidate.stdout'))
            res['artifacts'] = d
        except OSError as ex:
            res['artifacts_error'] = str(ex)
    return res


def compare_entry_shared_cache(e, opts, mode):
    """An older run.py in another checkout deletes every oracle of an entry but its own, under a run in progress here, and then this run blames the candidate or
    itself.  An outcome other than identical, from an oracle that is gone by now, is therefore run once more, against whatever oracle is there."""
    r = compare_entry(e, opts, mode)
    key = r.get('oracle_key')
    if r['outcome'] not in ('identical', 'excluded') and key and not os.path.isfile(os.path.join(oracle_base(e), key, 'meta.json')):
        r = compare_entry(e, opts, mode)
        r['retried'] = f'the oracle {key} was deleted during the first try'
    return r


def runs_dir(args):
    return args.runs_dir or DEFAULT_RUNS


def list_runs(root, stage):
    res = []
    try:
        names = os.listdir(root)
    except OSError:
        return res
    for n in names:
        p = os.path.join(root, n, 'run.json')
        try:
            info = read_json(p)
        except (OSError, ValueError):
            continue
        if info.get('stage') == stage:
            res.append((info.get('started_ns', 0), os.path.join(root, n)))
    return [p for _, p in sorted(res)]


def latest_run(root, stage):
    runs = list_runs(root, stage)
    return runs[-1] if runs else None


def read_results(run_dir):
    out = []
    try:
        with open(os.path.join(run_dir, 'results.jsonl'), 'rb') as f:
            data = f.read()
    except OSError:
        return out
    for line in data.split(b'\n'):
        if not line.strip():
            continue
        try:
            out.append(json.loads(line))
        except ValueError:                        # a line cut by SIGKILL; that entry simply runs again
            continue
    return out


# A record of an entry from the owner's private manifests says where it differs and gives a hash, not what the document says: DESIGN section 13.  The record lives under target/, but
# it gets read, copied into reports and pasted.  RSTEX_CORPUS_QUOTE_PRIVATE=1 keeps the text, for someone debugging on their own machine; the self-check needs that too.
PRIVATE_IDS = ('user/', 'local/', 'cd-local/')
QUOTING = {'a', 'b', 'context', 'box_log', 'first_error', 'class_reason', 'unit', 'notes', 'differing', 'extra_in_candidate', 'missing_in_candidate', 'not_rewritten', 'files', 'why'}
LOCATING = {'text', 'all'}


def hide(v):
    data = json.dumps(v, sort_keys=True).encode()
    return f'<{len(data)} bytes, sha256 {hashlib.sha256(data).hexdigest()[:12]}>'


def hide_but_where(text):
    """A message's words up to its first colon say where it differs, and any file named there goes too."""
    return re.sub(r'^(L0) \S+', r'\1 <file>', text.split(': ', 1)[0]) + ': ' + hide(text)


def redact(x):
    """A copy of a record with whatever can quote a document replaced by a hash."""
    if isinstance(x, list):
        return [redact(v) for v in x]
    if not isinstance(x, dict):
        return x
    out = {}
    for k, v in x.items():
        if k in LOCATING:
            out[k] = hide_but_where(v) if isinstance(v, str) else [hide_but_where(t) for t in v] if isinstance(v, list) else v
        elif k in QUOTING and isinstance(v, (str, list, dict)):
            out[k] = hide(v)
        else:
            out[k] = redact(v)
    return out


class ResultLog:
    def __init__(self, path):
        self.fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o644)
        self.lock = threading.Lock()

    def add(self, rec):
        if str(rec.get('id', '')).startswith(PRIVATE_IDS) and not os.environ.get('RSTEX_CORPUS_QUOTE_PRIVATE'):
            rec.update(redact(rec))             # in place: the summary is made from the same record
        data = (json.dumps(rec, sort_keys=True) + '\n').encode()
        with self.lock:
            os.write(self.fd, data)


def engine_info(path, extra):
    resolved = shutil.which(path) or path
    if not (os.path.isfile(resolved) and os.access(resolved, os.X_OK)):
        sys.exit(f'--engine {path}: not an executable file')
    return {'path': os.path.abspath(resolved), 'args': list(extra), 'sha256': hash_contents(resolved), 'bytes': os.path.getsize(resolved)}


def open_run(args, stage, config):
    """Create or resume a run directory.  A resumed run must have the same configuration, the same engine included."""
    root = runs_dir(args)
    os.makedirs(root, exist_ok=True)
    run_dir = None
    if getattr(args, 'resume', None):
        if args.resume == 'latest':
            run_dir = latest_run(root, stage)
            if run_dir is None:
                sys.exit(f'--resume: no earlier {stage} run in {root}')
        else:
            run_dir = args.resume if os.path.isabs(args.resume) else os.path.join(root, args.resume)
        info = read_json(os.path.join(run_dir, 'run.json'))
        if info['config'] != config:
            sys.exit(f'--resume: {run_dir} has a different configuration:\n  then: {info["config"]}\n  now:  {config}')
        return run_dir, info, True
    name = args.run_id or time.strftime('%Y%m%d-%H%M%S-') + stage + '-' + secrets.token_hex(2)
    run_dir = os.path.join(root, name)
    if os.path.exists(run_dir):
        sys.exit(f'run directory exists: {run_dir} (use --resume {name})')
    os.makedirs(run_dir)
    info = {'stage': stage, 'started': now(), 'started_ns': time.time_ns(), 'config': config, 'argv': sys.argv[1:],
            'oracle': oracle_version()}
    write_json(os.path.join(run_dir, 'run.json'), info)
    return run_dir, info, False


def disk_usage_bytes(path=TARGET):
    p = subprocess.run(['du', '-sb', path], capture_output=True, text=True)
    try:
        return int(p.stdout.split()[0])
    except (IndexError, ValueError):
        return 0


def run_parallel(items, jobs, worker, on_result, disk_budget=None):
    """Bounded thread pool over subprocess-heavy work.  On SIGINT: kill children, leave results written so far."""
    purge_old_workdirs()
    pending = iter(items)
    futures = {}
    state = {'done': 0, 'over_budget': False}
    budget_bytes = disk_budget * (1 << 30) if disk_budget else None
    if budget_bytes and disk_usage_bytes() > budget_bytes:
        state['over_budget'] = True
    with cf.ThreadPoolExecutor(max_workers=jobs) as ex:
        try:
            while True:
                while len(futures) < jobs and not _stop.is_set():
                    item = next(pending, None)
                    if item is None:
                        break
                    futures[ex.submit(worker, item, state)] = item
                if not futures:
                    break
                done, _ = cf.wait(futures, timeout=1, return_when=cf.FIRST_COMPLETED)
                for f in done:
                    item = futures.pop(f)
                    state['done'] += 1
                    on_result(item, f)
                    if budget_bytes and state['done'] % 50 == 0 and disk_usage_bytes() > budget_bytes:
                        state['over_budget'] = True
        except KeyboardInterrupt:
            _stop.set()
            kill_all_active()
            raise


def empty_selection(args, entries):
    """Selecting nothing must not look like passing.  True when the caller should stop; sets the exit status."""
    if entries:
        return None
    if getattr(args, 'only_failed_last_run', False):
        print('no entries failed in the last run; nothing to do')
        return 0
    print('error: the selection is empty, nothing was run', file=sys.stderr)
    return 2


def compare_cmd(args):
    entries, exclusions, files = selected(args)
    if (rc := empty_selection(args, entries)) is not None:
        return rc
    if args.bringup:                     # Named, printed in the summary, and never the default: every shortcut here has hidden a real bug in tests/corpus/mutate
        args.mode, args.l6, args.l1, args.gate_log = 'last-pass', False, False, False
        if args.box_log == 'always':
            args.box_log = 'failed'
    modes = ['last-pass', 'from-scratch'] if args.mode == 'both' else [args.mode]
    args.modes = modes
    args.known = {} if args.no_list else known_divergences(args.known_list)
    if not args.manifest and not args.known_list:                # a line that names no entry is an error, so a renamed entry cannot slip off.  A private id is not: a checkout without that document has no such entry
        all_ids = {e['id'] for e in load_manifests(None)[0]}
        strays = sorted(i for i in set(args.known) - all_ids if not i.startswith(PRIVATE_IDS))
        if strays:
            raise ManifestError(f'{KNOWN_DIVERGENCES}: no entry {strays[0]}')
    eng = engine_info(args.engine, args.engine_arg)
    config = {'engine': eng, 'modes': modes, 'raster': args.raster, 'dpi': args.dpi, 'l6': args.l6, 'l1': args.l1, 'l1_max_ms': args.l1_max_ms, 'fls': args.fls,
              'box_log': args.box_log, 'gate_log': args.gate_log, 'timeout_scale': args.timeout_scale, 'bringup': args.bringup, 'list': hash_file(args.known_list or KNOWN_DIVERGENCES),
              'selection': {'tier': args.tier, 'tier_exact': args.tier_exact, 'source': args.source, 'filter': args.filter, 'only_failed_last_run': args.only_failed_last_run},
              'manifests': sorted(os.path.relpath(f, REPO) if f.startswith(REPO) else f for f in files)}
    run_dir, info, resumed = open_run(args, 'compare', config)
    args.run_dir = run_dir
    prior = {}
    if resumed:
        for r in read_results(run_dir):
            prior[(r['id'], r['mode'])] = r
    jobs_list = []
    for e in entries:
        todo = [m for m in modes if not result_still_valid(prior.get((e['id'], m)), e)]
        if todo:
            jobs_list.append((e, todo))
    log = ResultLog(os.path.join(run_dir, 'results.jsonl'))
    total = sum(len(t) for _, t in jobs_list)
    pending = {(e['id'], m) for e, t in jobs_list for m in t}
    ids = {e['id'] for e in entries}
    kept = {k: v for k, v in prior.items() if k not in pending and k[0] in ids and k[1] in modes}
    if resumed:
        log_progress(f'resuming {os.path.basename(run_dir)}: {len(kept)} results kept, {total} to run')
    results = list(kept.values())
    counter = {'n': 0}

    def worker(item, state):
        e, todo = item
        out = []
        for m in todo:
            if _stop.is_set():
                break
            if state['over_budget']:
                out.append({'id': e['id'], 'source': e['source'], 'tier': e['tier'], 'mode': m, 'levels': {},
                            'outcome': 'skipped', 'first_divergence': {'text': f'disk budget of {args.disk_budget_gb} GB exceeded'}})
                continue
            if m == 'from-scratch' and out and out[-1].get('mode') == 'last-pass' and out[-1].get('oracle_passes') == 1:
                # With one pass there is nothing before it: from scratch and from the last-pass state are the same run, so it is not run twice.
                out.append({**out[-1], 'mode': m, 'same_run_as_last_pass': True})
                continue
            try:
                r = compare_entry_shared_cache(e, args, m)
                for _ in range(3):
                    if r['outcome'] != 'harness-error':
                        break
                    # Another process can throw an oracle out from under a compare, when the machine runs several.  What stays is real.
                    r = compare_entry_shared_cache(e, args, m)
                out.append(r)
            except Exception:
                out.append({'id': e['id'], 'source': e['source'], 'tier': e['tier'], 'mode': m, 'levels': {},
                            'outcome': 'harness-error', 'first_divergence': {'text': clip(traceback.format_exc(), 1200)}})
        return out

    def on_result(item, fut):
        for r in fut.result():
            log.add(r)
            results.append(r)
            counter['n'] += 1
            log_progress(f'[{counter["n"]}/{total}] {r["outcome"]:<24} {r["id"]} ({r["mode"]})')

    interrupted = False
    try:
        run_parallel(jobs_list, args.jobs, worker, on_result, args.disk_budget_gb)
    except KeyboardInterrupt:
        interrupted = True
    finalize_compare(run_dir, info, entries, results, exclusions, modes, interrupted, quiet=getattr(args, 'quiet', False))
    if interrupted:
        print(f'interrupted; continue with: run.py compare ... --resume {os.path.basename(run_dir)}')
        return 130
    bad = [r for r in results if r['outcome'] not in ('identical', 'known-divergence', 'oracle-fails', 'nondeterministic', 'excluded')]
    if not any(r['outcome'] in ('identical', 'known-divergence') for r in results):      # a listed entry was compared, level by level
        print('no entry was actually compared and found identical: treated as a failure')
        return 1
    return 1 if bad else 0


def smoke_cmd(args):
    """One line for an agent bringing the engine up: the smoke tier, last pass, PDF and side files, a short timeout, a short report.  Not the gate, and it says so."""
    t0 = time.monotonic()
    args.tier, args.tier_exact, args.source, args.only_failed_last_run = 'smoke', False, None, False
    args.bringup, args.raster, args.box_log, args.quiet = True, 'none', 'never', True
    args.mode, args.dpi, args.l6, args.l1, args.l1_max_ms, args.fls, args.gate_log = 'last-pass', 100, False, False, 2000, False, False
    args.show_differences, args.no_list, args.tokens, args.known_list = None, False, False, None
    args.resume, args.run_id, args.retry_oracle_fails = None, None, False
    stderr = sys.stderr
    sys.stderr = open(os.devnull, 'w')          # the per-entry progress lines: the run has 30 entries and takes seconds
    try:
        rc = compare_cmd(args)
    finally:
        sys.stderr = stderr
    if rc == 2:
        print('smoke: the selection is empty', file=sys.stderr)
        return rc
    run_dir = args.run_dir
    results = sorted(read_results(run_dir), key=lambda r: r['id'])
    comparable = [r for r in results if r['outcome'] not in NOT_COMPARABLE]
    for r in comparable:
        if r['outcome'] != 'identical':
            print(f'{r["outcome"]:<24} {r["id"]}: {clip((r.get("first_divergence") or {}).get("text") or "", 200)}')
    ok = sum(r['outcome'] == 'identical' for r in comparable)
    print(f'SMOKE {ok} / {len(comparable)} identical in {time.monotonic() - t0:.1f} s, {args.jobs} jobs (results: {run_dir}/summary.txt). '
          'A bring-up check, last pass only: not the gate')
    bad = [r for r in comparable if r['outcome'] != 'identical']
    if bad:
        print(f'next: run.py boxlog {bad[0]["id"]} --engine {args.engine}')
    return 0 if comparable and not bad else 1


def result_still_valid(r, e):
    """A result can be kept on resume only if the oracle it was compared against is still the valid one."""
    if r is None:
        return False
    meta = load_valid_oracle(e)
    return meta is not None and meta['key'] == r.get('oracle_key') or r['outcome'] == 'excluded'


SUBSTANCE = os.path.join(TARGET, 'audit', 'substance.json')


def substance_line(results):
    """The denominator with vacuous entries and duplicates out.  Written by tests/corpus/audit.py; an entry whose oracle has changed since the audit
    counts as not audited, and the line says how many."""
    try:
        audit = read_json(SUBSTANCE)['entries']
    except (OSError, ValueError, KeyError):
        return 'substance not audited: run tests/corpus/audit.py to take vacuous entries and duplicates out of the denominator'
    counted = [r for r in results if r['outcome'] not in NOT_COMPARABLE]
    keep, vac, dup, unknown = [], 0, 0, 0
    for r in counted:
        a = audit.get(r['id'])
        if not a or a.get('oracle_key') != r.get('oracle_key'):
            unknown += 1
            keep.append(r)
        elif a['verdict'] == 'substantive':
            keep.append(r)
        elif a['verdict'] == 'duplicate':
            dup += 1
        else:
            vac += 1
    return (f'substantive: {sum(r["outcome"] == "identical" for r in keep)} / {len(keep)} identical, with {vac} vacuous entries and {dup} duplicates out of the denominator'
            + (f'; {unknown} entries not audited, counted as substantive' if unknown else ''))


def tally(results):
    rows = {}
    for r in results:
        scopes = [('tier', r['tier']), ('source', r['source'])]
        if r['id'].startswith(('coverage/', 'extra/')):      # a pass on 4,600 package loads says nothing about the 500 primitive probes beside it
            scopes.append(('category', '/'.join(r['id'].split('/')[:2]).split('=')[0]))
        for scope, name in scopes:
            row = rows.setdefault(scope, {}).setdefault((r.get('mode', ''), name), {o: 0 for o in OUTCOMES})
            row[r['outcome']] += 1
    return rows


def summary_lines(run_dir, info, entries, results, exclusions, modes, list_all=False, interrupted=False):
    L = []
    cfg = info['config']
    L.append(f'run {os.path.basename(run_dir)}  engine {cfg["engine"]["path"]} {" ".join(cfg["engine"]["args"])}'
             f'  sha256 {cfg["engine"]["sha256"][:16]}')
    L.append(f'oracle {info["oracle"]["banner"]}')
    want = len(entries) * len(modes)
    if interrupted or len(results) < want:
        L.append(f'INCOMPLETE: {len(results)} of {want} selected comparisons have results')
    if cfg.get('bringup'):
        L.append('BRING-UP RUN: last pass only, no L1, L6, or log gate. This is not the gate; a pass here says little.')
    short = {'identical': 'ident', 'differs-content': 'content', 'differs-structure-only': 'struct', 'differs-log-only': 'log',
             'candidate-crash': 'crash', 'candidate-timeout': 'timeout', 'known-divergence': 'known', 'oracle-fails': 'orac-f',
             'nondeterministic': 'nondet', 'harness-error': 'harn-e', 'excluded': 'excl', 'skipped': 'skip'}
    rows = tally(results)
    for scope in ('tier', 'source', 'category'):
        if scope not in rows:
            continue
        L.append('')
        L.append(f'per {scope}:')
        L.append(f'  {"mode":<13}{scope:<28}{"total":>6}' + ''.join(f'{short[o]:>8}' for o in OUTCOMES))
        order = list(TIERS) if scope == 'tier' else sorted({k[1] for k in rows.get(scope, {})})
        for mode in modes:
            for name in order:
                row = rows.get(scope, {}).get((mode, name))
                if row:
                    L.append(f'  {mode:<13}{name:<28}{sum(row.values()):>6}' + ''.join(f'{row[o]:>8}' for o in OUTCOMES))
    L.append('')
    for mode in modes:
        rs = [r for r in results if r['mode'] == mode]
        count = {o: sum(r['outcome'] == o for r in rs) for o in OUTCOMES}
        comparable = len(rs) - sum(count[o] for o in NOT_COMPARABLE)
        L.append(f'PASS {mode}: {count["identical"]} / {comparable} comparable entries identical'
                 f'  (selected {len(entries)}; not comparable: accepted {count["known-divergence"]}, oracle-fails {count["oracle-fails"]}, '
                 f'nondeterministic {count["nondeterministic"]}, excluded {count["excluded"]}, skipped {count["skipped"]})')
        moved = sum(bool(r.get('list_moved')) for r in rs)
        agrees = sum(bool(r.get('list_agrees')) for r in rs)
        if moved or agrees:
            L.append(f'  on known_divergences.txt: {moved} differ otherwise than the entry says, and fail; {agrees} no longer differ')
        l6 = collections.Counter(r.get('levels', {}).get('L6', {}).get('status') for r in rs)
        if l6.get('equal') or l6.get('equal-masked') or l6.get('equal-inflated') or l6.get('differs'):
            L.append(f'  L6: {l6.get("equal", 0)} equal, {l6.get("equal-masked", 0)} equal but for the version string in the banner, {l6.get("equal-inflated", 0)} equal once decompressed, {l6.get("differs", 0)} differ')
        sub = substance_line(rs)
        if sub:
            L.append('  ' + sub)
        no_l1 = [r for r in rs if r.get('levels', {}).get('L1', {}).get('status') == 'skipped']
        if no_l1:
            L.append(f'  L1 not run on {len(no_l1)} large entries (--l1-max-ms): those were judged at L0, L2, L4 to L7 and the log only')
        unjudged = [r for r in rs if r.get('inst_note')]
        if unjudged:
            L.append(f'  instrumented levels could not judge {len(unjudged)} entries, so those were not checked at L1, L2 or L6; first: {unjudged[0]["id"]}: {clip(unjudged[0]["inst_note"], 160)}')
        judged = [r for r in rs if r.get('oracle_judged_failure')]
        if judged:
            L.append(f'  of which the oracle itself fails and the log is what is judged: '
                     f'{sum(r["outcome"] == "identical" for r in judged)} / {len(judged)} identical')
        batched = [r for r in rs if r.get('units_total')]
        if batched:
            L.append(f'  batch entries: {sum(r["units_total"] for r in batched if r["outcome"] == "identical")} / '
                     f'{sum(r["units_total"] for r in batched)} units are in identical entries')
    failing = [r for r in results if r['outcome'] != 'identical']
    if failing:
        L.append('')
        L.append(f'not identical, by entry ({len(failing)}):')
        for r in sorted(failing, key=lambda x: (OUTCOMES.index(x['outcome']), x['id'], x['mode'])):
            L.append(f'  {r["outcome"]:<24} {r["id"]} [{r["mode"]}]')
            fd = r.get('first_divergence') or {}
            if r.get('known_reason'):
                L.append(f'      accepted: {r["known_reason"]}')
            if r.get('list_moved'):
                L.append(f'      on known_divergences.txt: the entry says {r["list_moved"]["said"]}; it is now {r["list_moved"]["now"]}')
            if fd.get('text'):
                L.append(f'      {"reason" if r["outcome"] in NOT_COMPARABLE and not r.get("known_reason") else "first divergence"}: {fd["text"]}')
            if fd.get('unit'):
                L.append(f'      in unit: {fd["unit"]}')
            if fd.get('box_log'):
                L.append(f'      {fd["box_log"]}')
            if r.get('artifacts'):
                L.append(f'      artifacts: {r["artifacts"]}')
    identical = sorted((r for r in results if r['outcome'] == 'identical'), key=lambda x: (x['id'], x['mode']))
    agreeing = sorted((r for r in results if r.get('list_agrees')), key=lambda x: (x['id'], x['mode']))
    if agreeing:
        L.append('')
        L.append(f'AGREES: {len(agreeing)} comparisons on known_divergences.txt show no difference now.  Take what they say off the list:')
    for r in agreeing:
        L.append(f'AGREES, on known_divergences.txt: {r["id"]} [{r["mode"]}], whose entry says {r["list_agrees"]}')
    if identical and (list_all or len(identical) <= 100):
        L.append('')
        L.append(f'identical, by entry ({len(identical)}; these timings are uncontrolled, the bench stage is the measurement):')
        for r in identical:
            sp = f'{r["speedup"]:.2f}x' if r.get('speedup') else '-'
            levels = r.get('levels', {})
            l6 = levels.get('L6', {}).get('status')
            l2 = levels.get('L2', {}).get('status')
            logs = levels.get('log', {}).get('status')
            extra = ''.join(f' {name}={val}' for name, val in (('L6', l6), ('L2', l2), ('log', logs)) if val)
            L.append(f'  {r["id"]:<46} [{r["mode"]}] oracle {r.get("oracle_ms") or 0:9.1f} ms  candidate '
                     f'{r.get("candidate_ms") or 0:9.1f} ms  {sp:>7}{extra}')
    elif identical:
        L.append('')
        L.append(f'{len(identical)} identical entries are not printed here; they are listed in {run_dir}/summary.txt and results.json')
    notes = [r for r in results if r.get('log_note')]
    if cfg.get('gate_log') is False and notes:
        L.append('')
        L.append('the log was not gating in this run (--no-gate-log or --bringup)')
    if notes:
        L.append('')
        L.append(f'log differs but does not gate on {len(notes)} entries (first 20 here, all in results.json):')
        for r in notes[:20]:
            L.append(f'  {r["id"]} [{r["mode"]}]: {r["log_note"]}')
    mism = sorted({r['id'] for r in results if r.get('class_mismatch')})
    if mism:
        L.append('')
        L.append(f'expected_class differs from what the oracle did on {len(mism)} entries (all in results.json; the oracle\'s verdict is what counts): ' + ', '.join(mism[:10]))
    if exclusions:
        L.append('')
        L.append('coverage bounds stated by the manifests:')
        for x in exclusions:
            L.append(f'  [{x["manifest"]}] {x.get("what", "?")}: {x.get("why", "?")}')
    return L


def finalize_compare(run_dir, info, entries, results, exclusions, modes, interrupted=False, quiet=False):
    results = sorted(results, key=lambda r: (r['id'], r['mode']))
    full = {'run': info, 'selected': len(entries), 'modes': modes, 'finished': now(), 'interrupted': interrupted,
            'tally': {scope: {f'{k[0]}/{k[1]}': v for k, v in rows.items()} for scope, rows in tally(results).items()},
            'exclusions': exclusions, 'results': results}
    write_json(os.path.join(run_dir, 'results.json'), full)
    lines = summary_lines(run_dir, info, entries, results, exclusions, modes, list_all=True, interrupted=interrupted)
    with open(os.path.join(run_dir, 'summary.txt'), 'w') as f:
        f.write('\n'.join(lines) + '\n')
    if quiet:
        return
    print('\n'.join(summary_lines(run_dir, info, entries, results, exclusions, modes, interrupted=interrupted)))
    print(f'\nresults: {run_dir}/results.json   target/corpus uses {disk_usage_bytes() / (1 << 30):.2f} GiB')


def report_cmd(args):
    root = runs_dir(args)
    run = args.run
    if run in (None, 'latest'):
        candidates = list_runs(root, 'compare')
        if not candidates:
            sys.exit('no compare run found in ' + root)
        run_dir = candidates[-1]
    else:
        run_dir = run if os.path.isabs(run) else os.path.join(root, run)
    path = os.path.join(run_dir, 'summary.txt')
    if not os.path.isfile(path):
        print(f'{run_dir} did not finish: {len(read_results(run_dir))} results so far; continue it with compare --resume {os.path.basename(run_dir)}')
        return 1
    print(open(path).read())
    return 0


def boxlog_cmd(args):
    entries, _, _ = load_manifests(args.manifest)
    match = [e for e in entries if e['id'] == args.id]
    if not match:
        sys.exit(f'no entry {args.id!r}')
    e = match[0]
    eng = engine_info(args.engine, args.engine_arg)
    meta, _ = prepare_entry(e)
    if effective_class(meta, e) == 'oracle-fails':
        print(f'{e["id"]}: oracle fails: {meta.get("class_reason")}')
        return 2
    l2 = run_l2(e, meta, eng['path'], args.engine_arg, e['timeout'] * args.timeout_scale)
    if l2['status'] == 'oracle-run-failed':
        print(f'{e["id"]}: {l2["error"]}')
        return 2
    print(f'{e["id"]}: {l2["box_lines_a"]} oracle box lines, {l2["box_lines_b"]} candidate box lines, candidate exit {l2["candidate_rc"]}')
    if l2['status'] == 'equal':
        print('L2 box logs are equal')
        return 0
    print(l2_text(l2))
    for c in l2['first']['context']:
        print(f'      = {c}')
    print(f'oracle log: {l2["oracle_log"]}')
    return 1


def list_cmd(args):
    entries, exclusions, files = selected(args)
    print(f'{len(entries)} entries from {len(files)} manifests')
    for e in entries:
        meta = load_valid_oracle(e)
        state = effective_class(meta, e) if meta else 'not prepared'
        print(f'  {e["id"]:<46} {e["tier"]:<9} {e["source"]:<16} {e["mode"]:<10} {state}'
              + (f'  excluded: {e["excluded"]}' if e.get('excluded') else ''))
    for x in exclusions:
        print(f'  bound [{x["manifest"]}] {x.get("what", "?")}: {x.get("why", "?")}')
    return 0


def prepare_cmd(args):
    entries, exclusions, files = selected(args)
    if (rc := empty_selection(args, entries)) is not None:
        return rc
    root = runs_dir(args)
    os.makedirs(root, exist_ok=True)
    run_dir = os.path.join(root, args.run_id or time.strftime('%Y%m%d-%H%M%S-prepare-') + secrets.token_hex(2))
    os.makedirs(run_dir, exist_ok=True)
    info = {'stage': 'prepare', 'started': now(), 'started_ns': time.time_ns(), 'config': {'force': args.force},
            'oracle': oracle_version(), 'argv': sys.argv[1:]}
    write_json(os.path.join(run_dir, 'run.json'), info)
    log = ResultLog(os.path.join(run_dir, 'results.jsonl'))
    results = []
    total = len(entries)

    def worker(e, state):
        if e.get('excluded'):
            return {'id': e['id'], 'source': e['source'], 'tier': e['tier'], 'class': 'excluded',
                    'reason': e['excluded'], 'reused': False, 'observed_class': 'excluded', 'class_mismatch': False}
        if state['over_budget']:
            return {'id': e['id'], 'source': e['source'], 'tier': e['tier'], 'class': 'skipped',
                    'reason': f'disk budget of {args.disk_budget_gb} GB exceeded', 'reused': False,
                    'observed_class': 'skipped', 'class_mismatch': False}
        try:
            meta, reused = prepare_entry(e, force=args.force, retry_fails=args.retry_oracle_fails)
            return oracle_record(e, meta, reused)
        except Exception:
            return {'id': e['id'], 'source': e['source'], 'tier': e['tier'], 'class': 'harness-error',
                    'reason': clip(traceback.format_exc(), 1200), 'reused': False, 'observed_class': 'harness-error',
                    'class_mismatch': False}

    def on_result(e, fut):
        r = fut.result()
        log.add(r)
        results.append(r)
        log_progress(f'[{len(results)}/{total}] {r["class"]:<18} {"reused  " if r["reused"] else "compiled"} {r["id"]}')

    interrupted = False
    try:
        run_parallel(entries, args.jobs, worker, on_result, args.disk_budget_gb)
    except KeyboardInterrupt:
        interrupted = True
    results.sort(key=lambda r: r['id'])
    classes = ('normal', 'oracle-fails', 'nondeterministic', 'excluded', 'skipped', 'harness-error')
    L = []
    if interrupted or len(results) < total:
        L.append(f'INCOMPLETE: {len(results)} of {total} entries; running prepare again continues (oracles are cached)')
    L.append(f'prepare: {len(results)} of {total} selected entries; compiled {sum(not r["reused"] for r in results)}, reused {sum(r["reused"] for r in results)}')
    for scope in ('tier', 'source'):
        L.append(f'per {scope}:')
        names = list(TIERS) if scope == 'tier' else sorted({r[scope] for r in results})
        L.append(f'  {scope:<28}{"total":>6}' + ''.join(f'{c[:12]:>14}' for c in classes))
        for name in names:
            rs = [r for r in results if r[scope] == name]
            if rs:
                L.append(f'  {name:<28}{len(rs):>6}' + ''.join(f'{sum(r["class"] == c for r in rs):>14}' for c in classes))
    odd = [r for r in results if r['class'] != 'normal']
    if odd:
        L.append(f'not normal ({len(odd)}):')
        for r in odd:
            L.append(f'  {r["class"]:<18} {r["id"]}: {clip(r.get("reason") or "", 400)}')
    mism = [r for r in results if r.get('class_mismatch')]
    if mism:
        L.append('expected_class differs from what the oracle did:')
        for r in mism:
            L.append(f'  {r["id"]}: expected {r["expected_class"]}, observed {r["observed_class"]}')
    judged = [r for r in results if r.get('judged_failure')]
    if judged:
        L.append(f'the oracle fails on {len(judged)} normal entries with judge_failure set; they are compared on log and exit status')
    units = sum(r.get('units_total') or 0 for r in results)
    if units:
        L.append(f'batch entries cover {units} units')
    errs = [r for r in results if r.get('errors') and r['class'] == 'normal']
    if errs:
        L.append(f'normal entries with TeX errors in the log ({len(errs)}): compared as they are')
        for r in errs[:20]:
            L.append(f'  {r["id"]}: {r["errors"]} errors, first: {r.get("first_error")}')
        if len(errs) > 20:
            L.append(f'  ... {len(errs) - 20} more in {run_dir}/results.json')
    stale = [r for r in results if r.get('stale_side_inputs')]
    if stale:
        L.append(f'side files read from the source tree, not produced by the run ({len(stale)} entries):')
        for r in stale[:20]:
            L.append(f'  {r["id"]}: {", ".join(r["stale_side_inputs"][:4])}')
        if len(stale) > 20:
            L.append(f'  ... {len(stale) - 20} more in {run_dir}/results.json')
    for x in exclusions:
        L.append(f'bound [{x["manifest"]}] {x.get("what", "?")}: {x.get("why", "?")}')
    write_json(os.path.join(run_dir, 'results.json'), {'run': info, 'results': results, 'interrupted': interrupted, 'exclusions': exclusions})
    with open(os.path.join(run_dir, 'summary.txt'), 'w') as f:
        f.write('\n'.join(L) + '\n')
    print('\n'.join(L))
    print(f'results: {run_dir}/results.json   target/corpus uses {disk_usage_bytes() / (1 << 30):.2f} GiB')
    return 130 if interrupted else 0


# ------------------------------------------------------------------------------------------------ bench
def spawn_floor():
    """What this runner's process start and reap cost: a measuring floor for sub-millisecond claims."""
    w = new_workdir()
    best = min(run_proc(['/bin/true'], w, {}, 10)['ms'] for _ in range(20))
    shutil.rmtree(w, ignore_errors=True)
    return best


def evict_from_page_cache(paths):
    """Drop these files from the page cache, without root: POSIX_FADV_DONTNEED drops clean pages of any file this user can open.  Returns how many
    bytes were asked for.  A page shared with another process stays, and dirty pages stay, hence the sync first."""
    total = 0
    for path in paths:
        try:
            fd = os.open(path, os.O_RDONLY)
        except OSError:
            continue
        try:
            total += os.fstat(fd).st_size
            os.posix_fadvise(fd, 0, 0, os.POSIX_FADV_DONTNEED)
        finally:
            os.close(fd)
    return total


def cold_closure(e, meta, exe):
    """What a compile reads: every file the oracle recorded, the format and the engine binary with its shared libraries."""
    paths = [p for p in meta['inputs']]
    paths.append(os.path.realpath(exe))
    return paths


def bench_once(e, meta, engine, extra, opts=None, candidate=False):
    """One timed compile.  A candidate's cache is empty unless --keep-engine-cache says otherwise, so a minimum of N runs is the cold-engine figure and not, by
    accident, the time of a snapshot restored from the run before."""
    w = new_workdir()
    out = os.path.join(w, 'o')
    keep_cache = opts is not None and getattr(opts, 'keep_engine_cache', False)
    env = entry_env(e)
    if candidate and not keep_cache:
        env = dict(candidate_env(e, out), RSTEX_NO_SNAPSHOT='1')    # the figure for a run, not for the first run, which also writes a checkpoint
    all_passes = opts is not None and getattr(opts, 'all_passes', False)
    if all_passes:
        overlay(os.path.join(meta['dir'], 'seed'), out)
        for d in meta.get('mirrored', []):
            os.makedirs(os.path.join(out, d), exist_ok=True)
    else:
        restore_state(meta, out)
    argv = tex_argv(e, engine, extra, out)
    timeout = e['timeout'] * 3
    if opts is not None and getattr(opts, 'cold', False):
        os.sync()
        evict_from_page_cache(cold_closure(e, meta, engine))
    if all_passes:
        total = {'rc': 0, 'ms': 0.0, 'cpu_ms': 0.0, 'timed_out': False, 'signal': None}
        for i in range(1, meta['passes_run'] + 1):
            remove_quietly(os.path.join(out, meta['jobname'] + '.pdf'))
            r = run_proc(argv, e['cwd'], env, timeout)
            total.update(rc=r['rc'], timed_out=r['timed_out'], signal=r['signal'], ms=total['ms'] + r['ms'], cpu_ms=total['cpu_ms'] + r['cpu_ms'])
            if r['timed_out'] or r['signal']:
                break
            overlay(os.path.join(meta['dir'], 'gen', str(i)), out)
        r = total
    else:
        r = run_proc(argv, e['cwd'], env, timeout)
    has_pdf = os.path.isfile(os.path.join(out, meta['jobname'] + '.pdf'))
    shutil.rmtree(w, ignore_errors=True)
    return r, has_pdf


def wait_for_load(limit, patience):
    if limit is None:
        return os.getloadavg()[0], 0.0
    t0 = time.monotonic()
    while os.getloadavg()[0] > limit and time.monotonic() - t0 < patience:
        time.sleep(2)
    return os.getloadavg()[0], time.monotonic() - t0


def percentile(sorted_values, p):
    """Nearest rank."""
    if not sorted_values:
        return None
    k = max(0, math.ceil(p / 100 * len(sorted_values)) - 1)
    return sorted_values[k]


def bench_cmd(args):
    entries, exclusions, files = selected(args)
    if (rc := empty_selection(args, entries)) is not None:
        return rc
    eng = engine_info(args.engine, args.engine_arg)
    selection = {'tier': args.tier, 'tier_exact': args.tier_exact, 'source': args.source, 'filter': args.filter, 'only_failed_last_run': args.only_failed_last_run}
    config = {'engine': eng, 'n': args.n, 'manifests': sorted(os.path.relpath(f, REPO) if f.startswith(REPO) else f for f in files),
              'cpu': args.cpu, 'max_load': args.max_load, 'selection': selection, 'cold': args.cold, 'all_passes': args.all_passes,
              'keep_engine_cache': args.keep_engine_cache, 'allow_unverified': args.allow_unverified}
    run_dir, info, resumed = open_run(args, 'bench', config)
    prior = {r['id']: r for r in read_results(run_dir)} if resumed else {}
    log = ResultLog(os.path.join(run_dir, 'results.jsonl'))
    if args.cpu is not None:
        os.sched_setaffinity(0, {args.cpu})
    # A speed-up on a wrong answer is not a speed-up: an engine that stops early is very fast.  So by default the statistics hold only entries the same
    # binary passed.  The check is by hash, since an older run proves nothing about this binary.
    identical_ids = None
    verified_by = None
    if not args.allow_unverified:
        for candidate in reversed(list_runs(runs_dir(args), 'compare')):
            cfg = read_json(os.path.join(candidate, 'run.json'))['config']
            if cfg['engine']['sha256'] == eng['sha256'] and 'last-pass' in cfg['modes'] and not cfg.get('bringup'):     # a smoke run holds 30 entries and judges little
                by_id = {}
                for r in read_results(candidate):
                    by_id.setdefault(r['id'], []).append(r['outcome'])
                identical_ids = {k for k, v in by_id.items() if all(o == 'identical' for o in v)}
                verified_by = os.path.basename(candidate)
                break
        if identical_ids is None:
            sys.exit('bench: no compare run of this very binary, so its answers are unverified. Run compare first, or pass --allow-unverified and say so when quoting the numbers')
    floor = spawn_floor()
    results = list(prior.values())
    n_entries = len(entries)
    try:
        for k, e in enumerate(entries):
            if e['id'] in prior:
                continue
            rec = {'id': e['id'], 'source': e['source'], 'tier': e['tier']}
            if e.get('excluded'):
                rec.update(status='excluded', reason=e['excluded'])
            else:
                meta, _ = prepare_entry(e)
                cls = effective_class(meta, e)
                if cls != 'normal':
                    rec.update(status='excluded', reason=f'oracle class {cls}: {meta.get("class_reason")}')
                elif identical_ids is not None and e['id'] not in identical_ids:
                    rec.update(status='unverified', reason=f'not identical in compare run {verified_by}')
                else:
                    load0, waited = wait_for_load(args.max_load, args.load_patience)
                    ora, cand, cand_cpu, ora_cpu = [], [], [], []
                    failed = None
                    for i in range(args.n):
                        pair = [('o', oracle_engine(e), []), ('c', eng['path'], args.engine_arg)]
                        if i % 2:                 # alternate the order so a warming cache favours neither side
                            pair.reverse()
                        for tag, exe, extra in pair:
                            r, has_pdf = bench_once(e, meta, exe, extra, args, candidate=tag == 'c')
                            if r['timed_out'] or r['signal'] or (tag == 'c' and meta['pdf'] and not has_pdf):
                                failed = f'{"oracle" if tag == "o" else "candidate"} rc={r["rc"]} signal={r["signal"]} timed_out={r["timed_out"]} pdf={has_pdf}'
                            (ora if tag == 'o' else cand).append(r['ms'])
                            (ora_cpu if tag == 'o' else cand_cpu).append(r['cpu_ms'])
                    rec.update(oracle_ms=min(ora), candidate_ms=min(cand), oracle_cpu_ms=min(ora_cpu), candidate_cpu_ms=min(cand_cpu),
                               oracle_all=ora, candidate_all=cand, load_before=load0, load_after=os.getloadavg()[0], load_wait_s=waited,
                               oracle_first_ms=ora[0], candidate_first_ms=cand[0], oracle_median_ms=statistics.median(ora), candidate_median_ms=statistics.median(cand))
                    if failed:
                        rec.update(status='candidate-failed', reason=failed)
                    else:
                        rec.update(status='timed', speedup=min(ora) / min(cand) if min(cand) else None)
            log.add(rec)
            results.append(rec)
            log_progress(f'[{k + 1}/{n_entries}] {rec["status"]:<16} {e["id"]}'
                         + (f'  oracle {rec["oracle_ms"]:.1f} ms  candidate {rec["candidate_ms"]:.1f} ms' if 'oracle_ms' in rec else ''))
    except KeyboardInterrupt:
        kill_all_active()
        print(f'interrupted; continue with: run.py bench ... --resume {os.path.basename(run_dir)}')
        return 130
    timed = [r for r in results if r['status'] == 'timed']
    sp = sorted(r['speedup'] for r in timed if r.get('speedup'))
    agg = {'entries_selected': n_entries, 'entries_timed': len(timed), 'n': args.n, 'spawn_floor_ms': floor,
           'cpu_pinned': args.cpu, 'max_load': args.max_load, 'selection': selection, 'verified_by': verified_by,
           'cold': args.cold, 'all_passes': args.all_passes, 'keep_engine_cache': args.keep_engine_cache,
           'unverified': [{'id': r['id'], 'reason': r.get('reason')} for r in results if r['status'] == 'unverified'],
           'excluded': [{'id': r['id'], 'reason': r.get('reason')} for r in results if r['status'] == 'excluded'],
           'candidate_failed': [{'id': r['id'], 'reason': r.get('reason')} for r in results if r['status'] == 'candidate-failed'],
           'load_max': max((max(r['load_before'], r['load_after']) for r in timed), default=None)}
    if sp:
        worst = min(timed, key=lambda r: r['speedup'])
        best = max(timed, key=lambda r: r['speedup'])
        total_o = sum(r['oracle_ms'] for r in timed)
        total_c = sum(r['candidate_ms'] for r in timed)
        agg.update(worst=sp[0], worst_id=worst['id'], p10=percentile(sp, 10), median=statistics.median(sp), p90=percentile(sp, 90),
                   best=sp[-1], best_id=best['id'], geomean=math.exp(sum(math.log(x) for x in sp) / len(sp)),
                   total_oracle_ms=total_o, total_candidate_ms=total_c, total_speedup=total_o / total_c if total_c else None,
                   cpu_median=statistics.median([r['oracle_cpu_ms'] / r['candidate_cpu_ms'] for r in timed if r['candidate_cpu_ms']] or [0.0]))
    write_json(os.path.join(run_dir, 'results.json'), {'run': info, 'aggregate': agg, 'results': sorted(results, key=lambda r: r['id'])})
    what = 'all passes from nothing' if args.all_passes else 'last pass from the oracle aux state'
    L = [f'bench {os.path.basename(run_dir)}: min of {args.n} runs per side, serial, {what}; '
         + ('page cache emptied of the compile\'s files before every run' if args.cold else 'page cache warm: this measures the engine, not the disk')
         + ('; the candidate keeps its cache between runs, so this is the WARM figure' if args.keep_engine_cache else '; the candidate\'s cache is empty on every run'),
         f'engine {eng["path"]} {" ".join(args.engine_arg)}',
         f'selection: {json.dumps({k: v for k, v in selection.items() if v})} from {len(files)} manifests',
         f'answers: ' + (f'only entries identical in compare run {verified_by} count, {len(agg["unverified"])} left out as not identical' if verified_by else 'UNVERIFIED (--allow-unverified): a wrong answer can be fast, so this is no speed-up claim'),
         f'entries: {len(timed)} timed of {n_entries} selected; {len(agg["excluded"])} excluded, {len(agg["candidate_failed"])} candidate failed, {len(agg["unverified"])} unverified',
         f'spawn floor {floor:.2f} ms, not subtracted; load average up to {agg["load_max"]}; '
         f'cpu pin {args.cpu if args.cpu is not None else "none"}; max-load {args.max_load if args.max_load is not None else "none"}']
    if sp:
        L += [f'speed-up (oracle_ms / candidate_ms, above 1 means the candidate is faster):',
              f'  worst  {agg["worst"]:8.3f}x  {agg["worst_id"]}',
              f'  p10    {agg["p10"]:8.3f}x  (90% of entries are at or above this)',
              f'  median {agg["median"]:8.3f}x',
              f'  p90    {agg["p90"]:8.3f}x  (90% of entries are at or below this)',
              f'  best   {agg["best"]:8.3f}x  {agg["best_id"]}',
              f'  geomean {agg["geomean"]:7.3f}x;  median by CPU time {agg["cpu_median"]:.3f}x',
              f'total: oracle {agg["total_oracle_ms"]:.1f} ms, candidate {agg["total_candidate_ms"]:.1f} ms, ratio {agg["total_speedup"]:.3f}x',
              'first run against the minimum, oracle and candidate: a large gap on one side is a cache at work, and the minimum hides it: '
              f'{statistics.median(r["oracle_first_ms"] / r["oracle_ms"] for r in timed if r.get("oracle_first_ms")):.3f}x and {statistics.median(r["candidate_first_ms"] / r["candidate_ms"] for r in timed if r.get("candidate_first_ms")):.3f}x, median over entries']
        # A median over thousands of one-line load tests measures process start and the format.  Each stratum is stated on its own.
        strata = {}
        for r in timed:
            strata.setdefault(r['source'], []).append(r)
        L.append('by source (median and total ratio; a median over a source of small tests mostly measures start-up):')
        for name, rs in sorted(strata.items()):
            L.append(f'  {name:<28} n={len(rs):<6} median {statistics.median(r["speedup"] for r in rs):8.3f}x   total {sum(r["oracle_ms"] for r in rs) / max(1e-9, sum(r["candidate_ms"] for r in rs)):8.3f}x')
        sizes = [('under 100 ms', 0, 100), ('100 ms to 1 s', 100, 1000), ('1 s to 10 s', 1000, 10000), ('over 10 s', 10000, float('inf'))]
        L.append('by size of the job on the oracle:')
        for name, lo, hi in sizes:
            rs = [r for r in timed if lo <= r['oracle_ms'] < hi]
            if rs:
                L.append(f'  {name:<28} n={len(rs):<6} median {statistics.median(r["speedup"] for r in rs):8.3f}x   total {sum(r["oracle_ms"] for r in rs) / max(1e-9, sum(r["candidate_ms"] for r in rs)):8.3f}x')
        L.append('per entry:')
        for r in sorted(timed, key=lambda r: r['id']):
            L.append(f'  {r["id"]:<46} oracle {r["oracle_ms"]:9.1f} ms  candidate {r["candidate_ms"]:9.1f} ms  {r["speedup"]:7.3f}x')
    if args.max_load is None:
        L.append(f'load was not controlled (no --max-load); it reached {agg["load_max"]} here, and these wall-clock figures carry whatever else the machine did')
    for key in ('excluded', 'candidate_failed', 'unverified'):
        for x in agg[key]:
            L.append(f'{key}: {x["id"]}: {clip(x["reason"] or "", 300)}')
    for x in exclusions:
        L.append(f'bound [{x["manifest"]}] {x.get("what", "?")}: {x.get("why", "?")}')
    with open(os.path.join(run_dir, 'summary.txt'), 'w') as f:
        f.write('\n'.join(L) + '\n')
    print('\n'.join(L))
    print(f'results: {run_dir}/results.json')
    return 0


# ------------------------------------------------------------------------------------------------ coverage
def load_primitives(path=None):
    """name -> class, from tests/primitives.json or the inventory of docs/research/verification-and-inventory.md.
    Accepts a list of names, a list of {name, class}, or {class: [names]}."""
    if path is None:
        p = os.path.join(REPO, 'tests', 'primitives.json')
        path = p if os.path.isfile(p) else INVENTORY_FALLBACK
    data = read_json(path)
    if isinstance(data, dict) and 'primitives' in data:
        data = data['primitives']
    res = {}
    if isinstance(data, dict):
        for cls, names in data.items():
            if isinstance(names, list):
                for n in names:
                    res[n] = cls
    else:
        for x in data:
            if isinstance(x, str):
                res[x] = 'unknown'
            else:
                res[x['name']] = x.get('class', 'unknown')
    return res, path


_MODE_PREFIX = re.compile(rb'^(?:vertical|horizontal|restricted horizontal|internal vertical|math|display math|no) mode: ')
_NON_PRIM = (b'the letter ', b'the character ', b'blank space', b'math shift character', b'begin-group character',
             b'end-group character', b'alignment tab character', b'macro parameter character', b'superscript character',
             b'subscript character', b'end of alignment template', b'select font', b'undefined', b'macro', b'long macro',
             b'outer macro', b'long outer macro', b'protected macro', b'protected long macro',
             b'case ')                    # conditionals report their outcome: {true}, {false}, {case 3}
_TRACE_LINE = re.compile(rb'^\{([^\n]*)\}$', re.M)
_NAME = re.compile(rb'^([^A-Za-z]?)([A-Za-z]+)(?:$|[0-9":])')
_UNLESS = re.compile(rb'^([^A-Za-z]?)unless[^A-Za-z]?(if[a-z]*)(?:$|:)')


def trace_names(line_body):
    """(primitive names, had escape character, understood) for the body of one {...} log line.

    Under \\escapechar=-1, which microtype sets, names arrive without a backslash; any other escape character is
    one non-letter byte.  \\unless prints glued to its conditional: {\\unless\\ifx}, or {unlessifx} without escape.
    pdfTeX's {/path/pdftex.map} and {/path/x.enc} notes also have this shape, and are not commands."""
    body = _MODE_PREFIX.sub(b'', line_body, count=1)
    if body.startswith(_NON_PRIM) or body in (b'true', b'false') or body.startswith(b'/'):
        return [], False, True
    m = _UNLESS.match(body)
    if m:
        return ['unless', m.group(2).decode('latin-1')], bool(m.group(1)), True
    m = _NAME.match(body)
    if m:
        return [m.group(2).decode('latin-1')], bool(m.group(1)), True
    if len(body) == 2 and body[1:] in (b' ', b'-', b'/'):
        return [body[1:].decode()], True, True
    if body in (b' ', b'-', b'/'):
        return [body.decode()], False, True
    return [], False, False


def trace_entry(e, meta):
    """Run the oracle from the last-pass state with \\tracingcommands=2, its log being a FIFO read live, so a log of
    several hundred MB never reaches the disk.  Counts are cached in the oracle directory."""
    cache = os.path.join(meta['dir'], 'trace.json')
    try:
        cached = read_json(cache)
        if 'bodies' in cached:
            return parse_trace(cached)
    except (OSError, ValueError):
        pass
    w = new_workdir()
    out = os.path.join(w, 'o')
    restore_state(meta, out)
    fifo = os.path.join(out, meta['jobname'] + '.log')
    os.mkfifo(fifo)
    env = entry_env(e)
    env.update(max_print_line='1000000', error_line='254', half_error_line='238')
    fd = os.open(fifo, os.O_RDWR | os.O_NONBLOCK)      # O_RDWR never blocks and keeps the FIFO open for TeX
    try:
        fcntl.fcntl(fd, fcntl.F_SETPIPE_SZ, 1 << 20)
    except OSError:
        pass
    counts = {}
    total = 0
    tail = b''
    timeout = e['timeout'] * 30

    def consume(chunk, final=False):
        nonlocal tail, total
        total += len(chunk)
        data = tail + chunk
        cut = data.rfind(b'\n') + 1 if not final else len(data)
        tail = data[cut:]
        for m in _TRACE_LINE.finditer(data, 0, cut):
            b = m.group(1)
            counts[b] = counts.get(b, 0) + 1

    res = {}
    t0 = time.monotonic()
    p = subprocess.Popen(tex_argv(e, oracle_engine(e), [], out, prelude=TRACE_PRELUDE), cwd=e['cwd'], env=env,
                         stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    with _active_lock:
        _active[p.pid] = True
    try:
        while True:
            r, _, _ = select.select([fd], [], [], 0.05)
            if r:
                try:
                    chunk = os.read(fd, 1 << 20)
                except BlockingIOError:
                    chunk = b''
                if chunk:
                    consume(chunk)
                    continue
            if p.poll() is not None and not r:
                break
            if time.monotonic() - t0 > timeout:
                kill_group(p.pid)
                res['timed_out'] = True
                p.wait()
                break
        consume(b'', final=True)
    finally:
        os.close(fd)
        with _active_lock:
            _active.pop(p.pid, None)
    res.update(rc=p.returncode, log_bytes=total, wall_s=round(time.monotonic() - t0, 2),
               bodies={k.decode('latin-1'): v for k, v in counts.items()})
    shutil.rmtree(w, ignore_errors=True)
    write_json(cache, res)
    return parse_trace(res)


def parse_trace(res):
    """The raw counts are what is cached, so a better parser does not need new traces."""
    prims, bare, escaped, others = {}, {}, {}, {}
    for body, c in res['bodies'].items():
        names, had_escape, understood = trace_names(body.encode('latin-1'))
        if not understood:
            others[clip(body, 60)] = others.get(clip(body, 60), 0) + c
        for name in names:
            prims[name] = prims.get(name, 0) + c
            bucket = escaped if had_escape else bare
            bucket[name] = bucket.get(name, 0) + c
    return {**{k: v for k, v in res.items() if k != 'bodies'}, 'names': prims, 'bare': bare, 'escaped': escaped, 'unparsed': others}


_CS = re.compile(rb'\\([A-Za-z]+)')
_TEXT_EXT = {'.tex', '.sty', '.cls', '.clo', '.def', '.cfg', '.ldf', '.fd', '.ltx', '.dtx', '.ini', '.bbl', '.aux'}
_mentions_cache = {}


def mentions(path):
    got = _mentions_cache.get(path)
    if got is not None:
        return got
    got = set()
    if os.path.splitext(path)[1] in _TEXT_EXT:
        got = {m.decode('latin-1') for m in _CS.findall(open_bytes(path))}
    _mentions_cache[path] = got
    return got


def tds_denominators():
    """How much of the installed tree exists, so 'packages exercised' has a stated denominator."""
    res = {'sty': 0, 'cls': 0, 'pfb': 0, 'tfm': 0, 'vf': 0, 'mapped_pfb': 0}
    ls_r = '/usr/share/texmf-dist/ls-R'
    for raw in open_bytes(ls_r).split(b'\n'):
        for ext in ('sty', 'cls', 'pfb', 'tfm', 'vf'):
            if raw.endswith(b'.' + ext.encode()):
                res[ext] += 1
    mapfile = '/var/lib/texmf/fonts/map/pdftex/updmap/pdftex.map'
    names = set(re.findall(rb'<<?\[?([A-Za-z0-9._+-]+\.pfb)', open_bytes(mapfile)))
    res['mapped_pfb'] = len(names)
    return res


def filetype(path):
    b = os.path.basename(path)
    m = re.search(r'\.(\d+)(pk|gf)$', b)
    if m:
        return m.group(2)
    if b.endswith('.code.tex'):
        return 'code.tex'
    ext = os.path.splitext(b)[1].lstrip('.').lower()
    return ext or '(none)'


def coverage_cmd(args):
    entries, exclusions, files = selected(args)
    if (rc := empty_selection(args, entries)) is not None:
        return rc
    excluded = [{'id': e['id'], 'why': e['excluded']} for e in entries if e.get('excluded')]
    entries = [e for e in entries if not e.get('excluded')]
    metas = []

    def worker(e, state):
        return e, prepare_entry(e)[0]

    def on_result(e, fut):
        metas.append(fut.result())

    run_parallel(entries, args.jobs, worker, on_result)
    metas.sort(key=lambda x: x[0]['id'])
    agg = {'classes': {}, 'packages': {}, 'fonts': {}, 'font_kinds': {}, 'filetypes': {}, 'encodings': {}, 'maps': {},
           'local_files': 0, 'tds_files': {}}
    doc_stats = {'entries': len(metas), 'oracle_fails': 0, 'nondeterministic': 0, 'normal': 0, 'pages': 0,
                 'with_errors': 0, 'overfull': 0, 'underfull': 0}
    per_source = {}
    all_paths = set()
    for e, meta in metas:
        cls = effective_class(meta, e)
        doc_stats['normal' if cls == 'normal' else 'oracle_fails' if cls == 'oracle-fails' else 'nondeterministic'] += 1
        log = meta.get('log') or {}
        doc_stats['pages'] += log.get('pages') or 0
        doc_stats['with_errors'] += bool(log.get('errors'))
        doc_stats['overfull'] += log.get('overfull') or 0
        doc_stats['underfull'] += log.get('underfull') or 0
        src = per_source.setdefault(e['source'], {'entries': 0, 'pages': 0, 'classes': set(), 'packages': set(), 'fonts': set()})
        src['entries'] += 1
        src['pages'] += log.get('pages') or 0
        for path in meta['inputs']:
            all_paths.add(path)
            base = os.path.basename(path)
            ft = filetype(path)
            agg['filetypes'][ft] = agg['filetypes'].get(ft, 0) + 1
            if path.startswith(TDS_PREFIXES):
                agg['tds_files'][path] = agg['tds_files'].get(path, 0) + 1
            else:
                agg['local_files'] += 1
            bucket = {'cls': 'classes', 'sty': 'packages', 'enc': 'encodings', 'map': 'maps'}.get(ft)
            if ft in ('tfm', 'pfb', 'vf', 'pk', 'gf', 'ttf', 'otf', 'afm', 'mf'):
                agg['fonts'][base] = agg['fonts'].get(base, 0) + 1
                agg['font_kinds'].setdefault(ft, set()).add(base)
            if bucket:
                agg[bucket][base] = agg[bucket].get(base, 0) + 1
            if ft == 'cls':
                src['classes'].add(base)
            elif ft == 'sty':
                src['packages'].add(base)
            elif ft in ('tfm', 'pfb', 'vf', 'pk'):
                src['fonts'].add(base)
    report = {'generated': now(), 'selection': {'tier': args.tier, 'source': args.source, 'filter': args.filter},
              'documents': doc_stats, 'installed': tds_denominators(), 'exclusions': exclusions, 'excluded_entries': excluded,
              'classes': agg['classes'], 'packages': agg['packages'], 'fonts': agg['fonts'],
              'font_kinds': {k: len(v) for k, v in agg['font_kinds'].items()},
              'encodings': agg['encodings'], 'maps': agg['maps'], 'filetypes': agg['filetypes'],
              'per_source': {k: {'entries': v['entries'], 'pages': v['pages'], 'classes': len(v['classes']),
                                 'packages': len(v['packages']), 'fonts': len(v['fonts'])} for k, v in sorted(per_source.items())},
              'files_read_distinct': len(all_paths)}
    inst = report['installed']
    L = [f'coverage over {doc_stats["entries"]} entries: {doc_stats["normal"]} normal, {doc_stats["nondeterministic"]} nondeterministic, '
         f'{doc_stats["oracle_fails"]} oracle-fails; {doc_stats["pages"]} pages; {doc_stats["with_errors"]} logs with TeX errors; '
         f'{doc_stats["overfull"]} overfull and {doc_stats["underfull"]} underfull box warnings',
         f'classes exercised   {len(agg["classes"]):>6} of {inst["cls"]} .cls files installed',
         f'packages exercised  {len(agg["packages"]):>6} of {inst["sty"]} .sty files installed',
         f'fonts exercised     {len(agg["fonts"]):>6} distinct font files; by kind {json.dumps(report["font_kinds"], sort_keys=True)}; '
         f'installed tfm {inst["tfm"]}, pfb {inst["pfb"]} ({inst["mapped_pfb"]} named in pdftex.map), vf {inst["vf"]}',
         f'encodings {len(agg["encodings"])}, map files {len(agg["maps"])}, distinct files read {len(all_paths)}',
         'file types (documents reading such a file, summed per file): ' + ', '.join(f'{k} {v}' for k, v in sorted(agg['filetypes'].items(), key=lambda x: -x[1])),
         'top classes: ' + ', '.join(f'{k}:{v}' for k, v in sorted(agg['classes'].items(), key=lambda x: (-x[1], x[0]))[:15]),
         'top packages: ' + ', '.join(f'{k}:{v}' for k, v in sorted(agg['packages'].items(), key=lambda x: (-x[1], x[0]))[:25]),
         'per source: ' + '; '.join(f'{k}: {v["entries"]} entries, {v["pages"]} pp, {v["classes"]} cls, {v["packages"]} sty, {v["fonts"]} fonts' for k, v in report['per_source'].items())]
    if not args.no_trace:
        prims, prim_path = load_primitives(args.primitives)
        candidates = [(e, m) for e, m in metas if effective_class(m, e) != 'oracle-fails']
        if args.trace_all or args.trace_sample >= len(candidates):
            sample = candidates
            how = f'all {len(candidates)} entries the oracle can compile'
        else:
            # Stratified by source so a large source cannot crowd out a small one.  The sample is deterministic.
            rng = random.Random(args.seed)
            by_source = {}
            for x in candidates:
                by_source.setdefault(x[0]['source'], []).append(x)
            quota = {s: max(1, round(args.trace_sample * len(v) / len(candidates))) for s, v in by_source.items()}
            sample = []
            for s in sorted(by_source):
                v = sorted(by_source[s], key=lambda x: x[0]['id'])
                sample += rng.sample(v, min(quota[s], len(v)))
            how = f'{len(sample)} of {len(candidates)} entries, stratified by source, seed {args.seed}'
        traces = {}

        def tworker(item, state):
            e, m = item
            return e['id'], trace_entry(e, m)

        def ton_result(item, fut):
            i, t = fut.result()
            traces[i] = t
            log_progress(f'[trace {len(traces)}/{len(sample)}] {i}: {t["log_bytes"] / 1e6:.1f} MB log, {t["wall_s"]} s')

        run_parallel(sample, args.jobs, tworker, ton_result)
        expected_rc = {e['id']: m['last_rc'] for e, m in sample}
        failed_traces = {i: f'exit {t.get("rc")}, the oracle without tracing exits {expected_rc[i]}' + (', timed out' if t.get('timed_out') else '')
                         for i, t in traces.items() if t.get('timed_out') or t.get('rc') != expected_rc[i]}
        executed, bare_seen, unparsed, unknown_names = {}, {}, {}, {}
        docs = {}
        for i, t in traces.items():
            for n, c in t['names'].items():
                if n in prims:
                    executed[n] = executed.get(n, 0) + c
                    docs[n] = docs.get(n, 0) + 1
                else:
                    unknown_names[n] = unknown_names.get(n, 0) + c
            for n, c in t['bare'].items():
                if n in prims:
                    bare_seen[n] = bare_seen.get(n, 0) + c
            for n, c in t['unparsed'].items():
                unparsed[n] = unparsed.get(n, 0) + c
        mentioned = set()
        for e, m in candidates:
            for path in m['inputs']:
                mentioned |= mentions(path)
        by_class = {}
        for n, cls in prims.items():
            row = by_class.setdefault(cls, {'total': 0, 'executed': 0, 'never_executed_but_mentioned': [], 'never_seen': []})
            row['total'] += 1
            if n in executed:
                row['executed'] += 1
            elif n in mentioned:
                row['never_executed_but_mentioned'].append(n)
            else:
                row['never_seen'].append(n)
        report['primitives'] = {
            'list': prim_path, 'sample': how, 'seed': args.seed, 'traced_entries': sorted(traces),
            'method': '\\tracingcommands=2 from the oracle last-pass state, log read live through a FIFO. Names printed without a '
                      'backslash (microtype sets \\escapechar=-1) are counted too. Blind spot: a primitive read as an internal '
                      'quantity, such as \\the\\pdflastobj or \\ifnum\\pdfshellescape, prints nothing; \\numexpr and the like likewise. '
                      'Commands run only while the format was built are not traced here.',
            'by_class': by_class, 'executed': executed, 'documents': docs, 'bare_escape_counts': bare_seen,
            'trace_names_not_in_list': unknown_names, 'trace_lines_not_understood': unparsed,
            'log_bytes': sum(t['log_bytes'] for t in traces.values()),
            'trace_failed': failed_traces}
        total = sum(r['total'] for r in by_class.values())
        ex = sum(r['executed'] for r in by_class.values())
        L.append(f'primitives executed: {ex} of {total} in {prim_path}, over a trace of {how} '
                 f'({report["primitives"]["log_bytes"] / 1e6:.0f} MB of log)')
        for cls, r in sorted(by_class.items()):
            L.append(f'  {cls:<10} {r["executed"]:>4} / {r["total"]:<4} executed; {len(r["never_executed_but_mentioned"])} not traced but mentioned '
                     f'in loaded files; {len(r["never_seen"])} neither')
        if failed_traces:
            L.append(f'  TRACE FAILED on {len(failed_traces)} entries, whose counts are incomplete: ' + '; '.join(f'{k}: {v}' for k, v in sorted(failed_traces.items())[:10]))
        L.append(f'  counted from names without a backslash: {len(bare_seen)} primitives (\\escapechar=-1 trap handled), e.g. '
                 + ', '.join(sorted(bare_seen, key=lambda k: -bare_seen[k])[:8]))
        L.append(f'  blind spot: {report["primitives"]["method"]}')
        if unknown_names:
            L.append('  trace names not in the primitive list: ' + ', '.join(f'{k}:{v}' for k, v in sorted(unknown_names.items(), key=lambda x: -x[1])[:15]))
        if unparsed:
            L.append('  trace lines not understood: ' + ', '.join(f'{k!r}:{v}' for k, v in sorted(unparsed.items(), key=lambda x: -x[1])[:10]))
    else:
        L.append('primitives: not traced (--no-trace)')
    for x in excluded:
        L.append(f'excluded by manifest: {x["id"]}: {x["why"]}')
    for x in exclusions:
        L.append(f'bound [{x["manifest"]}] {x.get("what", "?")}: {x.get("why", "?")}')
    out = args.out or os.path.join(TARGET, 'coverage', 'latest.json')
    os.makedirs(os.path.dirname(out), exist_ok=True)
    write_json(out, report)
    print('\n'.join(L))
    print(f'full report: {out}')
    return 0


# ------------------------------------------------------------------------------------------------ self-check
def selfcheck_cmd(args):
    return SelfCheck(args).run()


class SelfCheck:
    """A runner that cannot fail proves nothing: this drives the real CLI with candidates that are known to be right
    and known to be wrong, and checks each verdict against something the runner did not compute."""

    def __init__(self, args):
        self.args = args
        self.root = os.path.join(TARGET, 'selfcheck')
        self.runs = os.path.join(self.root, 'runs')
        self.wrappers = os.path.join(self.root, 'wrappers')
        self.fails = []
        self.passes = 0
        self.ladder = os.path.join(MANIFEST_DIR, 'ladder.json')
        self.local = os.path.join(MANIFEST_DIR, 'user.local.json')
        self.fixtures = os.path.join(FIXTURES, 'selfcheck.json')
        self.pdflatex = shutil.which('pdflatex')
        self.j = str(min(args.jobs, 8))
        os.environ['RSTEX_CORPUS_QUOTE_PRIVATE'] = '1'     # it checks the changed word in the first differing operator of a private entry too

    def check(self, name, cond, detail=''):
        print(f'  {"ok  " if cond else "FAIL"} {name}' + (f'  [{detail}]' if detail else ''), flush=True)
        if cond:
            self.passes += 1
        else:
            self.fails.append(name)
        return cond

    def cli(self, *argv, timeout=3600):
        return subprocess.run([sys.executable, '-I', os.path.abspath(__file__), *argv], capture_output=True, text=True,
                              errors='replace', timeout=timeout)

    def compare(self, name, *argv):
        """Run compare with its own run id and return {(id, mode): result}."""
        run_id = f'{name}-{secrets.token_hex(3)}'
        if '--mode' not in argv:              # one result per entry, where the mode is not what a case is about
            argv = ('--mode', 'last-pass') + argv
        p = self.cli('compare', '--runs-dir', self.runs, '--run-id', run_id, '-j', self.j, *argv)
        run_dir = os.path.join(self.runs, run_id)
        res = {(r['id'], r['mode']): r for r in read_results(run_dir)}
        return p, res, run_dir

    def wrapper(self, name, body):
        path = os.path.join(self.wrappers, name)
        with open(path, 'w') as f:
            f.write('#!/bin/bash\n' + body)
        os.chmod(path, 0o755)
        return path

    def mutate_word_wrapper(self, name, old, new):
        """Runs the real engine on a copy of the main file in which the first occurrence of one word has changed.
        Exit 99 if the word was not there, so a mutation that did not apply cannot pass as a clean run."""
        return self.wrapper(name, f'''
set -u
args=("$@"); last="${{args[-1]}}"; prelude=""; file="$last"
if [[ "$last" == \\\\* ]]; then
  prelude="${{last%\\\\input \\"*}}"; file="${{last##*\\\\input \\"}}"; file="${{file%\\"}}"
fi
job="$(basename "$file" .tex)"
tmp="$(mktemp -d -p '{self.root}/tmp')"; trap 'rm -rf "$tmp"' EXIT
sed '0,/\\b{old}\\b/s//{new}/' "$file" > "$tmp/$job.tex"
cmp -s "$file" "$tmp/$job.tex" && {{ echo "mutation did not apply" >&2; exit 99; }}
unset 'args[-1]'
export openin_any=a
if [ -n "$prelude" ]; then new="$prelude\\\\input \\"$tmp/$job.tex\\""; else new="$tmp/$job.tex"; fi
"{self.pdflatex}" "${{args[@]}}" "-jobname=$job" "$new"
''')

    def first_line_wrapper(self, name, tex):
        """Adds TeX at the start of the job, leaving the document itself alone."""
        return self.wrapper(name, f'''
set -u
args=("$@"); last="${{args[-1]}}"; unset 'args[-1]'
if [[ "$last" == \\\\* ]]; then new='{tex} '"$last"; else new='{tex} \\input "'"$last"'"'; fi
"{self.pdflatex}" "${{args[@]}}" "$new"
''')

    def user_tree_state(self):
        """Stat of the source directories of private entries, to prove the runner wrote nothing there."""
        state = {}
        if not os.path.isfile(self.local):
            return state
        entries, _, _ = load_manifests([self.local])
        for cwd in sorted({e['cwd'] for e in entries}):
            for root, dirs, names in os.walk(cwd):
                for n in names + dirs:
                    p = os.path.join(root, n)
                    try:
                        st = os.lstat(p)
                        state[p] = (st.st_size, st.st_mtime_ns)
                    except OSError:
                        pass
        return state

    def run(self):
        os.makedirs(self.wrappers, exist_ok=True)
        os.makedirs(os.path.join(self.root, 'tmp'), exist_ok=True)
        shutil.rmtree(self.runs, ignore_errors=True)
        os.makedirs(self.runs)
        t0 = time.monotonic()
        before = self.user_tree_state()
        identity_sets = ['--manifest', self.ladder] + (['--manifest', self.local] if os.path.isfile(self.local) else [])
        self.identity(identity_sets)
        self.mutations()
        self.failure_classes()
        self.fixture_classes()
        self.l3build_form()
        self.list_checks()
        self.features(identity_sets)
        self.smoke_checks()
        self.bench_checks()
        self.coverage_checks()
        if before:
            after = self.user_tree_state()
            changed = [p for p in set(before) | set(after) if before.get(p) != after.get(p)]
            said = json.dumps(redact({'first_divergence': {'text': 'L0 name.aux line 2: words', 'all': ['log line 3/3: words'], 'a': 'words', 'context': ['words']}, 'levels': {'L0': {'differing': ['name.aux'], 'first': {'b': 'words'}}}}))
            self.check('a record of a private entry says where and gives a hash, and quotes nothing', 'words' not in said and 'name' not in said and 'line 3/3' in said, said)
            self.check('nothing changed under the source directories of private entries', not changed,
                       f'{len(before)} paths stat-ed before and after' + (f'; changed: {changed[:5]}' if changed else ''))
        print(f'\nSELF-CHECK: {self.passes} of {self.passes + len(self.fails)} checks passed in {time.monotonic() - t0:.0f} s')
        for f in self.fails:
            print(f'  FAILED: {f}')
        return 1 if self.fails else 0

    def identity(self, sets):
        print('\n== system pdflatex as the candidate: must be identical everywhere ==', flush=True)
        p = self.cli('prepare', '--runs-dir', self.runs, '-j', self.j, *sets)
        print(p.stdout)
        self.check('prepare exits 0', p.returncode == 0, p.stderr[-300:] if p.returncode else '')
        p, res, run_dir = self.compare('identity', '--engine', self.pdflatex, '--mode', 'both', *sets)
        print(p.stdout)
        entries, _, _ = load_manifests(sets[1::2])
        want = len(entries) * 2
        bad = {k: v['outcome'] for k, v in res.items() if v['outcome'] != 'identical'}
        self.check(f'all {len(entries)} entries identical in both modes', len(res) == want and not bad,
                   f'{len(res)} of {want} results; not identical: {bad}' if bad or len(res) != want else f'{len(res)} of {want} results')
        self.check('compare exits 0 when everything is identical', p.returncode == 0)
        # The defaults are both modes and every level.  L1, L2 and L6 come from one extra pass per entry, recorded with the last-pass result.
        once = [v for k, v in res.items() if k[1] == 'last-pass']
        l1 = sum(v['levels'].get('L1', {}).get('status') == 'equal' for v in once)
        l1_skipped = sum(v['levels'].get('L1', {}).get('status') == 'skipped' for v in once)     # a document over --l1-max-ms
        l2 = sum(v['levels'].get('L2', {}).get('status') == 'equal' for v in once)
        l6 = sum(v['levels'].get('L6', {}).get('status') == 'equal' for v in once)
        l45 = sum(v['levels'].get('L4_L5', {}).get('status') == 'identical' for v in res.values())
        l7 = sum(not v['levels'].get('L7', {}).get('diff_pages', [1]) for v in res.values())
        logs = sum(v['levels'].get('log', {}).get('status') == 'equal' for v in res.values())
        self.check("every level actually ran and agreed", l1 + l1_skipped == len(once) and l1 > 0 and l2 == len(once) and l6 == len(once) and l45 == want and l7 == want and logs == want,
                   f'L1 {l1}, L2 {l2}, L6 {l6} of {len(once)}; L4/L5 {l45}, L7 {l7}, normalized log {logs}, of {want}')
        p, res, _ = self.compare('identity-fls', '--engine', self.pdflatex, '--fls', '--raster', 'none', '--manifest', self.ladder, '--tier', 'smoke')
        self.check('--fls: the .fls file lists agree', p.returncode == 0 and all(v['levels'].get('L0_fls', {}).get('status') == 'equal' for v in res.values()),
                   f'{len(res)} entries, e.g. {next(iter(res.values()))["levels"]["L0_fls"]}')
        p = self.cli('report', '--runs-dir', self.runs, os.path.basename(run_dir))
        self.check('report reprints a stored summary', p.returncode == 0 and 'PASS last-pass' in p.stdout)

    def mutation_case(self, entry, old, new, manifest, label):
        """Mutate one word in ENTRY.  Checks the verdict against pdftotext and a direct compare_pdf.py run."""
        w = self.mutate_word_wrapper(f'mutate-{label}', old, new)
        filt = '^' + re.escape(entry) + '$'
        p, res, _ = self.compare(f'mutate-{label}', '--engine', w, '--manifest', manifest, '--filter', filt, '--box-log', 'failed')
        r = res.get((entry, 'last-pass'), {})
        fd = r.get('first_divergence', {})
        print(f'  runner says: {r.get("outcome")}\n    {fd.get("text")}\n    {fd.get("box_log")}')
        self.check(f'{label}: outcome differs-content', r.get('outcome') == 'differs-content', str(r.get('outcome')))
        self.check(f'{label}: compare exits 1', p.returncode == 1)
        page, op = fd.get('page'), fd.get('operator')
        if not (page and r.get('artifacts')):
            self.check(f'{label}: runner gave a page and kept the candidate PDF', False)
            return r
        entries, _, _ = load_manifests([manifest])
        e = next(x for x in entries if x['id'] == entry)
        meta, _ = prepare_entry(e)
        oracle_pdf = os.path.join(meta['dir'], 'final', e['jobname'] + '.pdf')
        cand_pdf = os.path.join(r['artifacts'], e['jobname'] + '.pdf')

        def page_text(pdf, n):
            return subprocess.run(['pdftotext', '-layout', '-f', str(n), '-l', str(n), pdf, '-'], capture_output=True, text=True, errors='replace').stdout

        # Independent oracle for the page: where the plain text of the two PDFs first differs, found by pdftotext.
        info = subprocess.run(['pdfinfo', oracle_pdf], capture_output=True, text=True).stdout
        pages = int(re.search(r'Pages:\s+(\d+)', info).group(1))
        first_text = next((n for n in range(1, pages + 1) if page_text(oracle_pdf, n) != page_text(cand_pdf, n)), None)
        self.check(f'{label}: page agrees with pdftotext', first_text == page, f'runner page {page}, pdftotext first differs on page {first_text} of {pages}')
        self.check(f'{label}: oracle page has the word, candidate page has the replacement',
                   old in page_text(oracle_pdf, page) and new in page_text(cand_pdf, page), f'{old} -> {new}')
        strings = lambda s: ''.join(re.findall(r'\(((?:[^()\\]|\\.)*)\)', s or ''))
        a_ops, b_ops = strings(fd.get('a')), strings(fd.get('b'))
        self.check(f'{label}: the first differing operator contains the changed word', old in a_ops and new in b_ops,
                   f'operator #{op}: oracle {a_ops[:60]!r}, candidate {b_ops[:60]!r}')
        direct = subprocess.run([sys.executable, '-I', COMPARE_PDF, oracle_pdf, cand_pdf, '--no-raster'], capture_output=True, text=True)
        m = re.search(r'FIRST CONTENT DIVERGENCE: page (\d+), operator #(\d+)', direct.stdout)
        self.check(f'{label}: a direct compare_pdf.py run says the same', direct.returncode == 1 and m and int(m.group(1)) == page and int(m.group(2)) == op,
                   m.group(0) if m else direct.stdout[-200:])
        l7 = r['levels'].get('L7', {})
        self.check(f'{label}: L7 first differs on that page, with a nonzero share of pixels', l7.get('diff_pages', [None])[0] == page and l7['percent'].get(str(page), 0) > 0,
                   f'{len(l7.get("diff_pages", []))} pages differ, page {page}: {l7.get("percent", {}).get(str(page))}%')
        l2 = r['levels'].get('L2', {})
        f = l2.get('first') or {}
        self.check(f'{label}: L2 first box-log difference is in the same shipout', l2.get('status') == 'differs' and f.get('shipout') == page,
                   f'box-log line {f.get("line_a")}, in shipout #{f.get("shipout")}, page {page}: {f.get("a")} | {f.get("b")}')
        return r

    def mutations(self):
        print('\n== one changed word: the runner must fail, and say where ==', flush=True)
        self.mutation_case('ladder/05-latex-sample2e', 'mathematical', 'mathematicalx', self.ladder, 'sample2e')
        self.mutation_case('ladder/03-plain-texweb', 'interwoven', 'interwovex', self.ladder, 'texweb')
        if os.path.isfile(self.local):
            # The experiment in docs/research/verification-and-inventory.md section 3.1 used this document and found
            # page 17, operator #1154, (isomorphisms.) vs (isomorphisns.).
            self.mutation_case('user/LMB', 'isomorphisms', 'isomorphisns', self.local, 'lmb')
        w = self.mutate_word_wrapper('mutate-scratch', 'mathematical', 'mathematicalx')
        p, res, _ = self.compare('mutate-scratch', '--engine', w, '--manifest', self.ladder, '--filter', '^ladder/05-latex-sample2e$', '--mode', 'from-scratch')
        r = res.get(('ladder/05-latex-sample2e', 'from-scratch'), {})
        self.check('from-scratch mode reports the mutation too', r.get('outcome') == 'differs-content' and r['first_divergence'].get('page'),
                   clip(r.get('first_divergence', {}).get('text', ''), 120))

    def expect(self, label, wrapper, entry, outcome, needle=None, extra=(), manifest=None):
        filt = '^' + re.escape(entry) + '$'
        p, res, _ = self.compare(label, '--engine', wrapper, '--manifest', manifest or self.ladder, '--filter', filt, *extra)
        r = next(iter(res.values()), {})
        text = (r.get('first_divergence') or {}).get('text', '')
        ok = r.get('outcome') == outcome and (needle is None or needle in text)
        self.check(f'{label}: {outcome}', ok, f'{r.get("outcome")}: {clip(text, 170)}')
        return r

    def failure_classes(self):
        print('\n== each failure class must land in its own bucket ==', flush=True)
        rung = 'ladder/13-hyperref'
        self.expect('crash-signal', self.wrapper('crash', 'kill -KILL $$\n'), rung, 'candidate-crash', 'signal 9')
        self.expect('crash-panic', self.wrapper('panic', 'echo "thread main panicked" >&2; exit 101\n'), rung, 'candidate-crash', 'panicked')
        self.expect('no-exec', self.wrapper('noexec', 'exit 127\n'), rung, 'candidate-crash', 'exit 127')
        self.expect('hang', self.wrapper('hang', 'sleep 600\n'), rung, 'candidate-timeout', 'timed out', ['--timeout-scale', '0.01'])
        self.expect('no-pdf', self.wrapper('nopdf', 'exit 0\n'), rung, 'candidate-crash', 'no PDF')
        garbage = self.wrapper('garbage', f'''
out=""; for a in "$@"; do case "$a" in -output-directory=*) out="${{a#-output-directory=}}";; esac; done
"{self.pdflatex}" "$@" >/dev/null 2>&1
head -c 200 /dev/urandom > "$out/13-hyperref.pdf"
''')
        self.expect('garbage-pdf', garbage, rung, 'candidate-crash', 'unreadable')
        self.expect('page-added', self.first_line_wrapper('pageadd', '\\AddToHook{begindocument/end}{x\\newpage}'), rung, 'differs-content', 'L4 page')
        self.expect('info-only', self.first_line_wrapper('info', '\\pdfinfo{/Mutated (yes)}'), rung, 'differs-structure-only', 'L5')
        aux = self.wrapper('aux', f'''
out=""; for a in "$@"; do case "$a" in -output-directory=*) out="${{a#-output-directory=}}";; esac; done
"{self.pdflatex}" "$@"; rc=$?
echo '% one extra line' >> "$out/13-hyperref.aux"
exit $rc
''')
        r = self.expect('aux-only', aux, rung, 'differs-structure-only', 'L0 13-hyperref.aux')
        self.check('aux-only: L4/L5 said identical, so only L0 could have caught it', r.get('levels', {}).get('L4_L5', {}).get('status') == 'identical')
        r = self.expect('image-dictionary', self.mutate_word_wrapper('decode', '0 1 0 1 0 1', '1 0 1 0 1 0'), 'fx/image-attr', 'differs-content', 'L4', manifest=self.fixtures)
        self.check('image-dictionary: an image is now judged with its dictionary, and L7 agrees', r.get('levels', {}).get('L7', {}).get('diff_pages') == [1], str(r.get('levels', {}).get('L7', {}).get('diff_pages')))
        r = self.expect('page-attribute', self.first_line_wrapper('userunit', '\\AddToHook{begindocument/end}{\\pdfpageattr{/UserUnit 2}}'), rung, 'differs-structure-only', 'L5')
        l7 = r.get('levels', {}).get('L7', {})
        self.check('page-attribute: L5 sees every key of a page, though L7 sees nothing', not l7.get('diff_pages'), str(l7.get('diff_pages')))
        lazy = self.wrapper('lazy', f'''
out=""; for a in "$@"; do case "$a" in -output-directory=*) out="${{a#-output-directory=}}";; esac; done
cp -p "$out/13-hyperref.aux" "$out/../saved.aux"
"{self.pdflatex}" "$@"; rc=$?
cp -p "$out/../saved.aux" "$out/13-hyperref.aux"
exit $rc
''')
        r = self.expect('never-writes-aux', lazy, rung, 'differs-structure-only', 'never wrote 13-hyperref.aux')
        self.check('never-writes-aux: the bytes matched, so only the rewrite check could have caught it', not r.get('levels', {}).get('L0', {}).get('differing'))
        status = self.wrapper('status', f'''"{self.pdflatex}" "$@"; exit 1\n''')
        self.expect('exit-status', status, rung, 'differs-structure-only', 'exit status')
        # A candidate that leaves no log in the instrumented pass stopped early.  That is a difference to report, not a failure of the tool.
        nolog = self.wrapper('nolog', f'''"{self.pdflatex}" "$@"; rc=$?
for a in "$@"; do case "$a" in -output-directory=*) out="${{a#-output-directory=}}";; \\\\*tracingoutput*) rm -f "$out/13-hyperref.log";; esac; done
exit $rc
''')
        self.expect('no-log-in-the-instrumented-pass', nolog, rung, 'differs-structure-only', 'no log file', ['--no-l6'])
        orphan = self.first_line_wrapper('orphan', '\\AddToHook{begindocument/end}{\\immediate\\pdfobj{(not referenced)}}')
        r = self.expect('orphan-object', orphan, rung, 'differs-structure-only', 'L5')
        self.check('orphan-object: L5 counts what nothing refers to, and L6 sees the bytes', r.get('levels', {}).get('L6', {}).get('status') == 'differs',
                   f'L6 offset {r.get("levels", {}).get("L6", {}).get("offset")}')
        broken = self.wrapper('brokenxref', f'''
out=""; for a in "$@"; do case "$a" in -output-directory=*) out="${{a#-output-directory=}}";; esac; done
"{self.pdflatex}" "$@"; rc=$?
python3 -I -c "import re,sys; p=sys.argv[1]; d=open(p,'rb').read(); open(p,'wb').write(re.sub(rb'startxref\\n\\d', b'startxref\\n9', d, count=1))" "$out/13-hyperref.pdf"
exit $rc
''')
        p, res, _ = self.compare('damaged-xref', '--engine', broken, '--manifest', self.ladder, '--filter', '^' + re.escape(rung) + '$')
        r = next(iter(res.values()), {})
        self.check('damaged-xref: readers repair it in silence, so it took a check of the file itself; it must not pass', r.get('outcome') in ('differs-structure-only', 'candidate-crash'),
                   f'{r.get("outcome")}: {clip((r.get("first_divergence") or {}).get("text") or "", 110)}')
        logmsg = self.wrapper('logmsg', f'''
"{self.pdflatex}" "$@"; rc=$?
for a in "$@"; do case "$a" in -output-directory=*) out="${{a#-output-directory=}}";; esac; done
sed -i 's/^LaTeX2e /LaTeX2e! /' "$out/13-hyperref.log"; exit $rc
''')
        self.expect('log-gate is the default', logmsg, rung, 'differs-log-only', 'log line', ['--no-l1'])
        r = self.expect('log-gate turned off', logmsg, rung, 'identical', None, ['--no-gate-log', '--no-l1'])
        self.check('log-gate turned off: the difference is still reported', 'log line' in (r.get('log_note') or ''), clip(r.get('log_note') or '', 100))
        w = self.mutate_word_wrapper('mutate-boxlog', 'mathematical', 'mathematicalx')
        p = self.cli('boxlog', 'ladder/05-latex-sample2e', '--engine', w, '--manifest', self.ladder)
        self.check('boxlog on demand finds the difference and exits 1', p.returncode == 1 and 'L2 box-log line' in p.stdout, clip(p.stdout.splitlines()[1] if p.stdout else p.stderr, 170))
        p = self.cli('boxlog', 'ladder/05-latex-sample2e', '--engine', self.pdflatex, '--manifest', self.ladder)
        self.check('boxlog with the real engine says equal and exits 0', p.returncode == 0 and 'equal' in p.stdout, clip(p.stdout.splitlines()[0], 120))

    def cache_across_checkouts(self):
        """Checkouts share one oracle of an entry, and the key counts them all as the main tree.  So the key cannot tell what a checkout would see from what the one that made the oracle saw.  Here is
        a main tree with two worktrees that link target/, as tests/gate.sh's users have, made up under the self-check's own directory: REPO and MAIN_ROOT are set for them, in this process."""
        global REPO, MAIN_ROOT
        saved = REPO, MAIN_ROOT
        top = os.path.join(os.path.realpath(self.root), 'checkouts')
        shutil.rmtree(top, ignore_errors=True)
        main = os.path.join(top, 'main')
        other, mine = (os.path.join(main, '.claude', 'worktrees', n) for n in ('other', 'mine'))
        os.makedirs(os.path.join(main, 'target', 'shim'))
        with open(os.path.join(main, 'target', 'shim', 'fxshim.sty'), 'w') as f:
            f.write('\\ProvidesPackage{fxshim}\n')
        for c in (main, other, mine):
            os.makedirs(os.path.join(c, 'tests'))
            with open(os.path.join(c, 'tests', 'shim.tex'), 'w') as f:
                f.write('\\documentclass{article}\\usepackage{fxshim}\\begin{document}x\\end{document}\n')
            with open(os.path.join(c, 'tests', 'plain.tex'), 'w') as f:
                f.write('\\documentclass{article}\\begin{document}x\\end{document}\n')
            if c != main:
                os.symlink(os.path.join(main, 'target'), os.path.join(c, 'target'))

        def entries(c):
            man = os.path.join(top, 'man.json')
            write_json(man, {'source': 'selfcheck-checkouts', 'entries': [
                {'id': 'cache/checkout-shim', 'main': os.path.join(c, 'tests', 'shim.tex'), 'texinputs': [os.path.join(c, 'target', 'shim')]},
                {'id': 'cache/checkout-plain', 'main': os.path.join(c, 'tests', 'plain.tex')}]})
            return load_manifests([man])[0]

        def logged(meta):
            return open_bytes(os.path.join(meta['dir'], 'final', meta['jobname'] + '.log')).replace(b'\n', b'')

        try:
            REPO, MAIN_ROOT = mine, main
            shim, plain = entries(mine)
            self.check('checkouts: a texinputs under target/ is the main tree\'s, whichever asks', shim['texinputs'] == [os.path.join(main, 'target', 'shim')], str(shim['texinputs']))
            # What a run.py from before this did in a worktree: it finds the package through that worktree's link.
            REPO = other
            old = {**entries(other)[0], 'texinputs': [os.path.join(other, 'target', 'shim')]}
            m_old, _ = prepare_entry(old, force=True)
            self.check('checkouts: such an oracle has the other checkout\'s path in its log', b'worktrees/other/target/shim' in logged(m_old), m_old['key'])
            REPO = mine
            _hash_cache.clear()
            self.check('checkouts: its key is the one this checkout asks for', m_old['key'].split('-')[0] == config_hash(shim), m_old['key'])
            self.check('checkouts: and it is refused, since every candidate would differ from it in that line', load_valid_oracle(shim) is None, m_old['key'])
            m_new, reused = prepare_entry(shim)
            self.check('checkouts: the next one prints the main tree\'s path', not reused and b'worktrees' not in logged(m_new) and load_valid_oracle(shim) is not None, m_new['key'])
            # Without a search path, one oracle serves every checkout, as long as each has the same file.
            REPO = other
            p_other = entries(other)[1]
            m_p, _ = prepare_entry(p_other, force=True)
            REPO = mine
            _hash_cache.clear()
            self.check('checkouts: another checkout\'s oracle serves this one when its own copy of the file is the same', (load_valid_oracle(plain) or {}).get('key') == m_p['key'], m_p['key'])
            with open(os.path.join(mine, 'tests', 'plain.tex'), 'a') as f:
                f.write('% edited here\n')
            _hash_cache.clear()
            self.check('checkouts: and not when this checkout has edited it, though the other still holds the old one', load_valid_oracle(plain) is None, m_p['key'])
            with open(os.path.join(mine, 'tests', 'plain.tex'), 'w') as f:
                f.write('\\documentclass{article}\\begin{document}x\\end{document}\n')
            meta_path = os.path.join(m_p['dir'], 'meta.json')
            bare = read_json(meta_path)
            del bare['checkout']
            write_json(meta_path, bare)
            _hash_cache.clear()
            self.check('checkouts: an oracle that does not say which checkout made it is taken as the same', (load_valid_oracle(plain) or {}).get('key') == m_p['key'], m_p['key'])
            write_json(meta_path, {**bare, 'class_reason': 'run A: ok; run B: oracle killed by signal 15 in pass 1'})
            self.check('an oracle from a run killed from outside is not taken', load_valid_oracle(plain) is None, m_p['key'])
            write_json(meta_path, bare)
            with open(os.path.join(mine, 'tests', 'plain.tex'), 'a') as f:
                f.write('% edited here\n')
            _hash_cache.clear()
            self.check('checkouts: and stale in the same way', load_valid_oracle(plain) is None, m_p['key'])
            # A directory of a checkout's own on the search path, such as tests/ladder: the log prints it, so each checkout has its own oracle of such an entry.
            for c in (other, mine):
                os.makedirs(os.path.join(c, 'tests', 'own'), exist_ok=True)
                shutil.copy(os.path.join(main, 'target', 'shim', 'fxshim.sty'), os.path.join(c, 'tests', 'own', 'fxshim.sty'))

            def own(c):
                man = os.path.join(top, 'man-own.json')
                write_json(man, {'source': 'selfcheck-checkouts', 'entries': [{'id': 'cache/checkout-own', 'main': os.path.join(c, 'tests', 'shim.tex'), 'texinputs': [os.path.join(c, 'tests', 'own')]}]})
                return load_manifests([man])[0][0]

            REPO = other
            _hash_cache.clear()
            m_other, _ = prepare_entry(own(other), force=True)
            REPO = mine
            _hash_cache.clear()
            self.check('checkouts: a search path in the checkout itself: another checkout\'s oracle, with its path in the log, is not taken', load_valid_oracle(own(mine)) is None, m_other['key'])
            m_mine, reused = prepare_entry(own(mine))
            self.check('checkouts: this one\'s log prints its own path', not reused and b'worktrees/mine/tests/own' in logged(m_mine) and b'worktrees/other' not in logged(m_mine), m_mine['key'])
            self.check('checkouts: and each keeps its own oracle', os.path.isdir(m_other['dir']) and m_other['key'] != m_mine['key'], m_mine['key'])
        finally:
            REPO, MAIN_ROOT = saved
            _hash_cache.clear()
        # A candidate that really does find a package by another path is a difference, and the fix above must not have hidden that.
        man = os.path.join(top, 'man-link.json')
        write_json(man, {'source': 'selfcheck-checkouts', 'entries': [{'id': 'cache/checkout-link', 'main': os.path.join(main, 'tests', 'shim.tex'), 'texinputs': [os.path.join(main, 'target', 'shim')]}]})
        self.expect('checkouts: the real engine is identical on it', self.pdflatex, 'cache/checkout-link', 'identical', manifest=man, extra=['--no-l1', '--raster', 'none'])
        w = self.wrapper('other-path', f'''export TEXINPUTS="{os.path.join(other, 'target', 'shim')}:"\nexec "{self.pdflatex}" "$@"\n''')
        self.expect('checkouts: a candidate that finds the package through another checkout\'s link is not', w, 'cache/checkout-link', 'differs-log-only', 'log line', ['--no-l1', '--raster', 'none'], manifest=man)
        for e in ('cache/checkout-shim', 'cache/checkout-plain', 'cache/checkout-link', 'cache/checkout-own'):
            shutil.rmtree(oracle_base({'id': e}), ignore_errors=True)
        shutil.rmtree(top, ignore_errors=True)

    def check_subset_tag_model(self):
        """print_char and pdftex_warn as a model, over every length of name and of file name, with \\newlinechar 10 and without: one normal form, whatever the column.  And what is not a line
        break still shows.  A line less after the message is the one thing that cannot show where its text ends at column 78 or 79: under LaTeX one empty line is right there, and the log does not say which it is."""
        def tex(name, file, nl, msg=b'\nmake_subset_tag(): subset-tag collision, resolved in round 3.\n'):
            out, col = bytearray(b'before\n\n'), 0        # pdftex_warn's two printlns
            for c in b'pdfTeX warning: ' + name + (b' (file ' + file + b')' if file else b'') + b': ' + msg:
                if c == 10 and nl:
                    out.append(10)
                    col = 0
                else:
                    out.append(c)
                    col += 1
                    if col == 79:
                        out.append(10)
                        col = 0
            return bytes(out) + b'\n>\n'                  # and the one that closes it, then whatever comes next
        same, word, more, less = [], [], [], []
        for nl in (False, True):
            for flen in range(160):
                file = b'f' * flen
                ref = normalize_log(tex(b'/usr/bin/pdftex', file, nl), [])
                for n in range(1, 50):
                    raw = tex(b'n' * n, file, nl)
                    if first_log_divergence(ref, normalize_log(raw, [])):
                        same.append((nl, flen, n))
                    if not first_log_divergence(ref, normalize_log(tex(b'n' * n, file, nl, b'\nmake_subset_tag(): subset-tag collision, resolved in round 4.\n'), [])):
                        word.append((nl, flen, n))
                    if not first_log_divergence(ref, normalize_log(raw.replace(b'\n\n>', b'\n\n\n>', 1), [])) and not nl:
                        more.append((nl, flen, n))
                    head = 16 + n + (8 + flen if flen else 0) + 2
                    if not nl and (head % 79 + 1 + 61) not in (78, 79) and not first_log_divergence(ref, normalize_log(raw.replace(b'\n\n>', b'\n>', 1), [])):
                        less.append((nl, flen, n))
        self.check('make_subset_tag, the model: one normal form under every name and file name', not same, str(same[:3]))
        self.check('make_subset_tag, the model: another word always shows', not word, str(word[:3]))
        self.check('make_subset_tag, the model: a line more always shows under plain', not more, str(more[:3]))
        self.check('make_subset_tag, the model: a line less shows but at columns 78 and 79', not less, str(less[:3]))

    def fixture_classes(self):
        print('\n== oracle classification on purpose-built documents ==', flush=True)
        p = self.cli('prepare', '--runs-dir', self.runs, '-j', self.j, '--manifest', self.fixtures, '--force')
        print(p.stdout)
        entries, _, _ = load_manifests([self.fixtures])
        got = {}
        for e in entries:
            m = load_valid_oracle(e)
            got[e['id']] = (m, e)
        want = {'fx/random': 'nondeterministic', 'fx/missing-input': 'oracle-fails', 'fx/infinite-loop': 'oracle-fails',
                'fx/include-subdir': 'normal', 'fx/bibtex': 'normal', 'fx/bib-auto': 'normal', 'fx/bbl-precomputed': 'normal',
                'fx/makeindex': 'normal', 'fx/errors-with-pdf': 'normal', 'fx/no-pages': 'normal', 'fx/ini-format': 'normal',
                'fx/format-pdflatex': 'normal', 'fx/seeded-random': 'normal', 'fx/image-attr': 'normal', 'fx/own-compression': 'normal', 'fx/engine-message': 'normal',
                'fx/subset-tag': 'normal', 'fx/subset-tag-latex': 'normal', 'fx/excluded': None}
        for i, cls in want.items():
            m, e = got.get(i, (None, None))
            if cls is None:
                continue
            self.check(f'{i} classified {cls}', m is not None and m['observed_class'] == cls,
                       f'{m["observed_class"]}: {clip(m.get("class_reason") or "", 120)}' if m else 'no oracle recorded')
        m, _ = got['fx/include-subdir']
        self.check('\\include subdirectory was mirrored into the output directory', m and m.get('mirrored') == ['chapters'], str(m and m.get('mirrored')))
        for i in ('fx/bibtex', 'fx/bib-auto'):
            m, _ = got[i]
            self.check(f'{i}: bibtex ran in preparation and its output is replayed after pass 1', m and m['gen_after_pass'].get('1') and any(s.get('step') == 'bibtex' and s.get('rc') == 0 for s in m['steps']),
                       f'{m and m["gen_after_pass"]}, {m and m["passes_run"]} passes')
        m, _ = got['fx/bbl-precomputed']
        self.check('a precomputed .bbl is used and no bibtex runs', m and not any(s.get('step') == 'bibtex' for s in m['steps']) and os.path.isfile(os.path.join(m['dir'], 'seed', 'fx-seeded.bbl')),
                   f'{m and m["passes_run"]} passes')
        m, _ = got['fx/makeindex']
        self.check('makeindex ran in preparation and its output is replayed', m and 'fx-index.ind' in m['gen_after_pass'].get('1', []), str(m and m['gen_after_pass']))
        p, res, _ = self.compare('fixtures', '--engine', self.pdflatex, '--manifest', self.fixtures, '--mode', 'both')
        print(p.stdout)
        wanted = {i: {'normal': 'identical'}.get(c, c) if c else 'excluded' for i, c in want.items()}
        for mode in ('last-pass', 'from-scratch'):
            actual = {i: res.get((i, mode), {}).get('outcome') for i in wanted}
            self.check(f'fixtures compare as expected ({mode})', actual == wanted, str({k: v for k, v in actual.items() if v != wanted[k]}) or f'{len(wanted)} entries')
        self.check('oracle-fails and nondeterministic entries stay visible in the printed summary',
                   'fx/missing-input' in p.stdout and 'fx/random' in p.stdout and 'oracle-fails' in p.stdout)
        # A wrong candidate on a document that only writes to its log must still fail.
        w = self.wrapper('quiet', f'''"{self.pdflatex}" "$@" >/dev/null; rc=$?
for a in "$@"; do case "$a" in -output-directory=*) out="${{a#-output-directory=}}";; esac; done
echo 'an extra line' >> "$out/fx-no-pages.log"; exit $rc
''')
        self.expect('no-pages: the log is the only output, so it gates', w, 'fx/no-pages', 'differs-content', 'log line', manifest=self.fixtures)
        # The engine's name is in its warnings, so they wrap elsewhere under another name.  Every length is tried, so that a message ending exactly at column 79 comes up on both sides.
        lengths, bad = range(1, 33), []
        for n in lengths:
            name = 'n' * n                        # kpathsea insists that its argv[0] exists, so a link on the path
            remove_quietly(os.path.join(self.wrappers, name))
            os.symlink(shutil.which('pdftex'), os.path.join(self.wrappers, name))
            for prog, fx in (('pdftex', '^fx/(engine-message|subset-tag)$'), ('pdflatex', '^fx/subset-tag-latex$')):
                w = self.wrapper('named', f'''export PATH="{self.wrappers}:$PATH"\nexec {name} -progname={prog} "$@"\n''')
                p, res, _ = self.compare('engine-name', '--engine', w, '--manifest', self.fixtures, '--filter', fx, '--raster', 'none')
                for (i, _m), r in res.items():
                    if r.get('outcome') != 'identical':
                        bad.append(f'{i}, {n}: {r.get("outcome")}: {clip((r.get("first_divergence") or {}).get("text") or "", 120)}')
        self.check(f'an engine with a name of any length from {lengths[0]} to {lengths[-1]} is identical on documents whose warnings name it', not bad, '; '.join(bad[:2]))
        self.check_subset_tag_model()
        # Under the very name that once showed a false difference: a real one in that message, a word or a line, still fails.
        name = 'n' * 11
        for what, edit in (('a changed word', "s/resolved in round 3/resolved in round 4/"), ('a line more', r"/resolved in round 3\.$/a\\")):
            w = self.wrapper('subset-word', f'''export PATH="{self.wrappers}:$PATH"\n{name} -progname=pdftex "$@"; rc=$?
for a in "$@"; do case "$a" in -output-directory=*) out="${{a#-output-directory=}}";; esac; done
sed -i '{edit}' "$out/fx-subset-tag.log"; exit $rc
''')
            self.expect(f'{what} in make_subset_tag\'s message still fails', w, 'fx/subset-tag', 'differs-log-only', 'log line', ['--no-l1', '--raster', 'none'], manifest=self.fixtures)
        w = self.wrapper('warning-word', f'''"{self.pdflatex}" "$@"; rc=$?
for a in "$@"; do case "$a" in -output-directory=*) out="${{a#-output-directory=}}";; esac; done
sed -i 's/invalid parameter/invalid argument/' "$out/fx-engine-message.log"; exit $rc
''')
        self.expect('a changed word in such a warning still fails', w, 'fx/engine-message', 'differs-log-only', 'log line', ['--no-l1', '--raster', 'none'], manifest=self.fixtures)
        # Where the oracle stops cleanly, a candidate that crashes on the same document is held to it, and takes no part in the denominator otherwise.
        self.expect('crash-where-the-oracle-fails', self.wrapper('crash-ofail', 'kill -KILL $$\n'), 'fx/missing-input', 'candidate-crash', 'signal 9', manifest=self.fixtures)
        # And it has to stop as the oracle stops: a clean exit that leaves another log is a difference.  The real engine on the same entry is in the classes above.
        self.expect('clean-failure-where-the-oracle-fails', self.wrapper('fail-ofail', 'exit 1\n'), 'fx/missing-input', 'differs-log-only', 'log line', manifest=self.fixtures)
        # A document that turns compression back on: L6 then sees deflate output, which the design chooses for itself.  It must not count, and the content still must.
        level = self.first_line_wrapper('own-level', '\\def\\fxlevel{1}')
        r = self.expect('own-compression: another deflate of the same streams is identical', level, 'fx/own-compression', 'identical', manifest=self.fixtures, extra=['--no-gate-log', '--no-l1'])     # the wrapper's first line shows in the log
        self.check('own-compression: L6 got there by decompressing', r.get('levels', {}).get('L6', {}).get('status') == 'equal-inflated', str(r.get('levels', {}).get('L6', {}).get('status')))
        word = self.mutate_word_wrapper('own-word', 'Compressed', 'Compresses')
        r = self.expect('own-compression: a changed word is still found', word, 'fx/own-compression', 'differs-content', 'L4', manifest=self.fixtures)
        self.check('own-compression: and L6 finds it too, in the decompressed bytes', r.get('levels', {}).get('L6', {}).get('status') == 'differs', str(r.get('levels', {}).get('L6', {}).get('status')))

    def list_checks(self):
        """What known_divergences.txt allows, through the real compare: a made-up engine whose difference is on a list of the self-check's own, and engines that differ from it by a digit, by a line, by
        nothing, and on an entry the list does not name."""
        print('\n== the list says how an entry differs ==', flush=True)
        entry = 'fx/engine-message'
        edit = lambda rule: self.wrapper('list-' + secrets.token_hex(3), f'''"{self.pdflatex}" "$@"; rc=$?
for a in "$@"; do case "$a" in -output-directory=*) out="${{a#-output-directory=}}";; esac; done
for f in "$out"/*.log; do sed -i '{rule}' "$f"; done; exit $rc
''')
        base = edit('s/invalid parameter/invalid parameter 12/')
        args = ['--manifest', self.fixtures, '--filter', '^' + entry + '$', '--raster', 'none', '--box-log', 'always']
        p, res, _ = self.compare('list-tokens', '--engine', base, '--no-list', '--tokens', *args)
        tokens = res.get((entry, 'last-pass'), {}).get('tokens') or {}
        self.check('an engine that differs has a token at each level where it does', 'log' in tokens and all(re.fullmatch(_LIST_TOKEN, t) for t in tokens.values()), str(tokens))
        path = os.path.join(self.root, 'list.txt')

        def listed(*lines):
            with open(path, 'w') as f:
                f.write('# the self-check\'s\n' + ''.join(x + '\n' for x in lines))

        said = f'{entry} last-pass:{tokens_text(tokens)} the wrapper changes one word'
        listed(said)
        with_list = ['--known-list', path] + args
        r = self.compare('list-holds', '--engine', base, *with_list)[1].get((entry, 'last-pass'), {})
        self.check('the same difference is accepted', r.get('outcome') == 'known-divergence' and r.get('known_reason') == 'the wrapper changes one word', str(r.get('outcome')))
        for name, rule in (('one digit inside it', 's/invalid parameter/invalid parameter 13/'), ('a new line in the log, away from it', '1a\\a new line')):
            bad = edit(rule if name.startswith('one') else 's/invalid parameter/invalid parameter 12/; ' + rule)
            p, res, run_dir = self.compare('list-moves', '--engine', bad, *with_list)
            r = res.get((entry, 'last-pass'), {})
            now = (r.get('list_moved') or {}).get('now', '')
            self.check(f'{name} fails, and prints what it is now', p.returncode == 1 and r.get('outcome') not in ('known-divergence', 'identical') and now and now != tokens_text(tokens) and now in p.stdout, f'{r.get("outcome")}, now {now}')
        p, res, _ = self.compare('list-agrees', '--engine', self.pdflatex, *with_list)
        r = res.get((entry, 'last-pass'), {})
        self.check('an engine that agrees is identical, and is said to be, loudly', p.returncode == 0 and r.get('outcome') == 'identical' and r.get('list_agrees') and 'AGREES, on known_divergences.txt: ' + entry in p.stdout, str(r.get('outcome')))
        p2, res2, _ = self.compare('list-unlisted', '--engine', edit('1a\\a new line'), '--known-list', path, '--manifest', self.fixtures, '--filter', '^fx/errors-with-pdf$', '--raster', 'none')
        r2 = res2.get(('fx/errors-with-pdf', 'last-pass'), {})
        self.check('an entry the list does not name still fails', p2.returncode == 1 and r2.get('outcome') not in ('known-divergence', 'identical', None), str(r2.get('outcome')))
        # A level run elsewhere or not at all.  The list names the log and a level this run does not judge.
        listed(f'{entry} last-pass:{tokens_text(dict(tokens, L2="-1+1:0123456789abcdef"))} a level that is not run here')
        r = self.compare('list-unjudged', '--engine', base, '--known-list', path, '--manifest', self.fixtures, '--filter', '^' + entry + '$', '--raster', 'none', '--box-log', 'never')[1].get((entry, 'last-pass'), {})
        self.check('a level this run does not judge neither fails nor passes an entry', r.get('outcome') == 'known-divergence', str(r.get('outcome')))
        r = self.compare('list-judged', '--engine', base, '--known-list', path, '--manifest', self.fixtures, '--filter', '^' + entry + '$', '--raster', 'none', '--box-log', 'always')[1].get((entry, 'last-pass'), {})
        self.check('and that level once judged has to say what the entry says', r.get('outcome') not in ('known-divergence', 'identical') and r.get('list_moved'), str(r.get('outcome')))
        # The mode an entry names.
        listed(f'{entry} from-scratch:{tokens_text(tokens)} from scratch only')
        r = self.compare('list-mode', '--engine', base, '--known-list', path, '--manifest', self.fixtures, '--filter', '^' + entry + '$', '--raster', 'none')[1].get((entry, 'last-pass'), {})
        self.check('an entry for one mode does not accept the other', r.get('outcome') not in ('known-divergence', 'identical'), str(r.get('outcome')))
        for bad_line in (f'{entry} no tokens at all', f'{entry} last-pass:log=-1+1:0123456789abcdef', f'{entry} last-pass:log=nonsense a reason', f'{entry} last-pass:L3=-1+1:0123456789abcdef a reason'):
            listed(bad_line)
            p = self.cli('compare', '--runs-dir', self.runs, '--run-id', 'list-form-' + secrets.token_hex(3), '--engine', base, '--known-list', path, '--manifest', self.fixtures, '--filter', '^' + entry + '$')
            self.check(f'a line without the form is an error: {bad_line[len(entry) + 1:][:30]}', p.returncode == 2 and 'the header of the file has the form' in p.stderr, clip(p.stderr, 100))
        # What depends on the machine stays out of a token.
        masks = difftoken.Masks([('/a/tree', b'<TREE>'), ('/a/tree/w/o', b'<OUT>')])
        other = difftoken.Masks([('/some/much/longer/tree', b'<TREE>'), ('/some/much/longer/tree/w/o', b'<OUT>')])
        wrap = lambda text, n: [text[i:i + n] for i in range(0, len(text), n)] + ([b''] if len(text) % n == 0 else [])
        one = wrap(b'before (/a/tree/w/o/x.aux) and a line that goes on and on past the end of any column at all, and then some more words', 40)
        two = wrap(b'before (/some/much/longer/tree/w/o/x.aux) and a line that goes on and on past the end of any column at all, and then some more words', 40)
        self.check('a token does not see how long the path was, or where TeX cut the line', difftoken.logical_lines(one, 40, masks) == difftoken.logical_lines(two, 40, other), str(difftoken.logical_lines(one, 40, masks)))
        new_line_a = [b'text', b'', b'(/some/much/longer/tree/w/o/x.aux)', b'more']
        new_line_b = [b'text (/a/tree/w/o/x.aux)', b'more']
        self.check('nor whether section 537 started a new line for it', difftoken.logical_lines(new_line_a, 79, other) == difftoken.logical_lines(new_line_b, 79, masks), str(difftoken.logical_lines(new_line_a, 79, other)))
        short, cut = b'{into \\x=macro:->INPUT /a/tree/w/o/x.fls}', b'{into \\x=macro:->INPUT /some/much/longer/tr\\ETC.}'
        self.check('nor how much of it a macro\'s 32 tokens show', masks.trace(short) == other.trace(cut), str(masks.trace(short)))
        self.check('and a trace line with no path stays as it is', masks.trace(b'{into \\x=macro:->/usr/share/a.tex and words\\ETC.}') == b'{into \\x=macro:->/usr/share/a.tex and words\\ETC.}')
        key = 'ab' * 32
        self.check('a private entry\'s token depends on the document: it cannot be tried against a guess', difftoken.lines_token('log', [b'a'], [b'b'], key) != difftoken.lines_token('log', [b'a'], [b'b']) and difftoken.lines_token('log', [b'a'], [b'b'], key) != difftoken.lines_token('log', [b'a'], [b'b'], 'cd' * 32))

    def l3build_form(self):
        print('\n== the l3build log normal form ==', flush=True)
        man = os.path.join(FIXTURES, 'l3build.json')
        p = self.cli('prepare', '--runs-dir', self.runs, '-j', self.j, '--manifest', man, '--force')
        entries, _, _ = load_manifests([man])
        metas = {e['id']: load_valid_oracle(e) for e in entries}
        m = metas['fx-l3/test']
        raw_log = open_bytes(os.path.join(m['dir'], 'final', 'fx-l3build.log'))
        self.check('a test finds what it wrote through the @OUT@ search path', m['observed_class'] == 'normal' and b'not found' not in raw_log and b'read back from' in raw_log, str(m['log'].get('first_error')))
        body = l3build_normalize_log(open_bytes(os.path.join(m['dir'], 'final', 'fx-l3build.log')), [m['workdir_path']])
        text = b'\n'.join(x[1] for x in body)
        self.check('the form keeps the body and drops what l3build drops', b'inside the test' in text and b'read back from' in text and b'> \\box...=' in text
                   and b'before the start' not in text and b'inside omit' not in text and b'END-TEST-LOG' not in text, clip(text, 170))
        p, res, _ = self.compare('l3build-identity', '--engine', self.pdflatex, '--manifest', man, '--mode', 'both')
        bad = {k: v['outcome'] for k, v in res.items() if v['outcome'] != 'identical'}
        self.check('system pdflatex is identical on both fixtures, in both modes', len(res) == 4 and not bad, str(bad))

        def sed_log(name, expr):
            return self.wrapper(name, f'''"{self.pdflatex}" "$@"; rc=$?
for a in "$@"; do case "$a" in -output-directory=*) out="${{a#-output-directory=}}";; -jobname=*) ;; esac; done
sed -i '{expr}' "$out"/fx-l3build*.log; exit $rc
''')
        self.expect('l3build form: a changed word in the test body fails', sed_log('l3-body', 's/^inside the test/inside a test/'), 'fx-l3/test', 'differs-content', 'log line', manifest=man)
        hidden = sed_log('l3-omit', 's/^inside omit/inside a omit/')
        relaxed = ['--no-gate-log', '--no-l1']      # the sed also reaches the instrumented log, which L1 would see
        r = self.expect('l3build form: a change between OMIT and TIMO passes the form', hidden, 'fx-l3/test', 'identical', manifest=man, extra=relaxed)
        self.check('... but the whole log is measured and the note says so', 'whole log line' in (r.get('log_note') or ''), clip(r.get('log_note') or '', 120))
        self.expect('... and it gates by default', hidden, 'fx-l3/test', 'differs-structure-only', 'whole log line', manifest=man)
        early = sed_log('l3-early', 's/^before the start marker/before a start marker/')
        r = self.expect('l3build form: a change before START passes the form', early, 'fx-l3/test', 'identical', manifest=man, extra=relaxed)
        self.check('... and is in the note too', 'whole log line' in (r.get('log_note') or ''), clip(r.get('log_note') or '', 120))
        self.expect('no START-TEST-LOG: the whole log is judged, so this cannot pass as an empty test', sed_log('l3-nostart', 's/^a line in a log/a line of a log/'), 'fx-l3/no-start', 'differs-content', 'log line', manifest=man)

    def features(self, sets):
        print('\n== selection, caching, resume ==', flush=True)
        known_bad = {'ladder/05-latex-sample2e', 'ladder/13-hyperref'}
        w = self.wrapper('fails-on-two', f'''
case "$*" in *05-latex-sample2e*|*13-hyperref*) kill -KILL $$;; esac
exec "{self.pdflatex}" "$@"
''')
        p, res, _ = self.compare('feat-fail', '--engine', w, '--manifest', self.ladder, '--tier', 'smoke')
        failed = {k[0] for k, v in res.items() if v['outcome'] not in ('identical',) + NOT_COMPARABLE}
        self.check('--tier smoke selects a proper subset', 0 < len(res) < len(load_manifests([self.ladder])[0]), f'{len(res)} entries')
        self.check('the failing subset is exactly the two rigged entries', failed == known_bad, str(sorted(failed)))
        p2, res2, _ = self.compare('feat-onlyfailed', '--engine', self.pdflatex, '--manifest', self.ladder, '--only-failed-last-run')
        self.check('--only-failed-last-run reruns exactly the failed entries', {k[0] for k in res2} == known_bad, str(sorted(k[0] for k in res2)))
        self.check('those entries pass with the good engine', len(res2) == 2 and all(v['outcome'] == 'identical' for v in res2.values()))
        p3, res3, _ = self.compare('feat-filter', '--engine', self.pdflatex, '--manifest', self.ladder, '--filter', '17-class', '--raster', 'none')
        self.check('--filter selects by regex', {k[0] for k in res3} == {'ladder/17-class-amsart', 'ladder/17-class-scrartcl', 'ladder/17-class-scrbook'}, str(sorted(k[0] for k in res3)))
        p4, res4, _ = self.compare('feat-source', '--engine', self.pdflatex, '--manifest', self.fixtures, '--source', 'no-such-source')
        self.check('an empty selection is an error, not a pass', len(res4) == 0 and p4.returncode == 2, f'exit {p4.returncode}')

        # A slow wrapper gives time to SIGKILL the runner in the middle of a run.
        slow = self.wrapper('slow', f'sleep 0.5\nexec "{self.pdflatex}" "$@"\n')
        run_id = f'feat-resume-{secrets.token_hex(3)}'
        cmd = [sys.executable, '-I', os.path.abspath(__file__), 'compare', '--runs-dir', self.runs, '--run-id', run_id, '-j', '2', '--engine', slow,
               '--manifest', self.ladder, '--raster', 'none', '--mode', 'last-pass']
        run_dir = os.path.join(self.runs, run_id)
        proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        deadline = time.monotonic() + 300
        while time.monotonic() < deadline and len(read_results(run_dir)) < 5:
            time.sleep(0.1)
        os.killpg(proc.pid, signal.SIGKILL)
        proc.wait()
        got_before = read_results(run_dir)
        self.check('SIGKILL mid-run leaves a partial result file', 5 <= len(got_before) < len(load_manifests([self.ladder])[0]), f'{len(got_before)} results')
        before_ids = [r['id'] for r in got_before]
        t = time.time()
        pr = self.cli('compare', '--runs-dir', self.runs, '--resume', run_id, '-j', '2', '--engine', slow, '--manifest', self.ladder, '--raster', 'none', '--mode', 'last-pass')
        after = read_results(run_dir)
        ids = [r['id'] for r in after]
        self.check('--resume finishes the run without repeating finished entries', len(ids) == len(set(ids)) == len(load_manifests([self.ladder])[0]) and ids[:len(before_ids)] == before_ids,
                   f'{len(before_ids)} kept, {len(ids) - len(before_ids)} added, {len(ids)} total')
        self.check('resumed run is all identical', pr.returncode == 0 and all(r['outcome'] == 'identical' for r in after))
        other = self.cli('compare', '--runs-dir', self.runs, '--resume', run_id, '--engine', self.pdflatex, '--manifest', self.ladder, '--raster', 'none', '--mode', 'last-pass')
        self.check('--resume refuses a different engine', other.returncode != 0 and 'different configuration' in (other.stderr or ''))

        # SIGINT kills the running candidates; those kills must not be recorded as candidate crashes.
        run_id = f'feat-sigint-{secrets.token_hex(3)}'
        cmd[cmd.index('--run-id') + 1] = run_id
        run_dir = os.path.join(self.runs, run_id)
        proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        deadline = time.monotonic() + 300
        while time.monotonic() < deadline and len(read_results(run_dir)) < 3:
            time.sleep(0.1)
        proc.send_signal(signal.SIGINT)
        rc = proc.wait(timeout=120)
        got = read_results(run_dir)
        self.check('SIGINT stops the run with exit 130 and records no killed entry as a crash', rc == 130 and got and all(r['outcome'] == 'identical' for r in got),
                   f'exit {rc}, {len(got)} results, all identical')
        p = self.cli('report', '--runs-dir', self.runs, run_id)
        self.check('an interrupted run says so in its summary', 'INCOMPLETE' in p.stdout, clip(p.stdout.splitlines()[2], 100) if p.stdout else '')

        # The oracle cache is keyed by the content of what the document read.
        doc_dir = os.path.join(self.root, 'cache-doc')
        shutil.rmtree(doc_dir, ignore_errors=True)
        os.makedirs(doc_dir)
        with open(os.path.join(doc_dir, 'body.tex'), 'w') as f:
            f.write('First.\n')
        with open(os.path.join(doc_dir, 'doc.tex'), 'w') as f:
            f.write('\\documentclass{article}\\begin{document}\\input{body}\\end{document}\n')
        man = os.path.join(self.root, 'cache-doc.json')
        write_json(man, {'source': 'selfcheck-cache', 'entries': [{'id': 'cache/doc', 'main': os.path.join(doc_dir, 'doc.tex')}]})
        e = load_manifests([man])[0][0]
        m1, r1 = prepare_entry(e, force=True)
        _hash_cache.clear()
        m2, r2 = prepare_entry(e)
        self.check('second prepare reuses the oracle', r2 and m1['key'] == m2['key'], m2['key'])
        with open(os.path.join(doc_dir, 'body.tex'), 'w') as f:
            f.write('Second.\n')
        _hash_cache.clear()
        m3, r3 = prepare_entry(e)
        self.check('an edit to an \\input file invalidates it', m3['key'] != m1['key'], f'{m1["key"]} -> {m3["key"]}')    # an oracle of that very content may survive from an earlier self-check
        self.cache_across_checkouts()

        scribble = self.wrapper('scribble', f'''"{self.pdflatex}" "$@"; rc=$?; touch scribbled-here.txt; exit $rc\n''')
        r = self.expect('scribbles-in-source-dir', scribble, 'cache/doc', 'differs-structure-only', 'scribbled-here.txt', manifest=man)
        remove_quietly(os.path.join(doc_dir, 'scribbled-here.txt'))
        p, res, _ = self.compare('feat-budget', '--engine', self.pdflatex, '--manifest', self.ladder, '--tier', 'smoke', '--disk-budget-gb', '0.000001')
        self.check('over the disk budget entries are reported as skipped and the run does not pass', p.returncode == 1 and len(res) == 9 and all(v['outcome'] == 'skipped' for v in res.values()),
                   f'{len(res)} results, exit {p.returncode}')
        p5 = self.cli('compare', '--runs-dir', self.runs, '--engine', self.pdflatex, '--manifest', os.path.join(self.root, 'no-such.json'))
        self.check('a missing manifest is an error, not an empty pass', p5.returncode != 0, clip((p5.stderr or '').strip().splitlines()[-1] if p5.stderr else '', 100))
        bad = os.path.join(self.root, 'bad.json')
        write_json(bad, {'source': 's', 'entries': [{'id': 'x', 'main': 'a.tex', 'pases': 2}]})
        p6 = self.cli('prepare', '--runs-dir', self.runs, '--manifest', bad)
        self.check('a misspelt manifest field is an error', p6.returncode != 0 and 'pases' in (p6.stderr or ''))

    def smoke_checks(self):
        print('\n== the smoke target ==', flush=True)
        args = ['--runs-dir', self.runs, '--manifest', self.ladder]
        t0 = time.monotonic()
        p = self.cli('smoke', '--engine', self.pdflatex, *args)
        self.check('smoke passes the real engine, exits 0, in under a minute, and says it is not the gate', p.returncode == 0 and 'SMOKE 9 / 9 identical' in p.stdout and 'not the gate' in p.stdout and time.monotonic() - t0 < 60,
                   clip(p.stdout.strip().splitlines()[-1] if p.stdout else p.stderr, 110))
        t0 = time.monotonic()
        p = self.cli('smoke', '--engine', self.wrapper('smoke-hang', 'sleep 600\n'), '--timeout-cap', '2', *args)
        self.check('a hang on every entry fails and stays bounded by the timeout cap', p.returncode == 1 and 'SMOKE 0 / 9 identical' in p.stdout and p.stdout.count('candidate-timeout') == 9 and time.monotonic() - t0 < 60,
                   f'exit {p.returncode}, {time.monotonic() - t0:.1f} s')
        w = self.mutate_word_wrapper('smoke-word', 'mathematical', 'mathematicalx')
        p = self.cli('smoke', '--engine', w, *args, '--filter', 'sample2e')
        self.check('a wrong page fails, and it names the next command', p.returncode == 1 and 'differs-content' in p.stdout and 'run.py boxlog ladder/05-latex-sample2e' in p.stdout, clip(p.stdout, 150))

    def bench_checks(self):
        print('\n== bench ==', flush=True)
        filt = ['--manifest', self.ladder, '--filter', '04-latex-hello|05-latex-sample2e|13-hyperref']
        unverified = self.cli('bench', '--runs-dir', os.path.join(self.root, 'no-such-runs'), '--engine', self.pdflatex, '-n', '1', *filt)
        self.check('bench refuses an engine with no compare run: a wrong answer can be fast', unverified.returncode != 0 and 'unverified' in (unverified.stderr or ''), clip(unverified.stderr or '', 110))
        p = self.cli('bench', '--runs-dir', self.runs, '--engine', self.pdflatex, '-n', '5', '--allow-unverified', *filt)
        print(p.stdout)
        run = latest_run(self.runs, 'bench')
        agg = read_json(os.path.join(run, 'results.json'))['aggregate']
        self.check('pdflatex against itself is close to 1.0x', 0.8 < agg['worst'] and agg['best'] < 1.25 and agg['entries_timed'] == 3,
                   f'worst {agg["worst"]:.3f}, median {agg["median"]:.3f}, best {agg["best"]:.3f}')
        lines = len(read_results(run))
        p = self.cli('bench', '--runs-dir', self.runs, '--engine', self.pdflatex, '-n', '5', '--allow-unverified', '--resume', os.path.basename(run), *filt)
        self.check('bench --resume keeps finished entries', p.returncode == 0 and len(read_results(run)) == lines == 3, f'{len(read_results(run))} results')
        p = self.cli('bench', '--runs-dir', self.runs, '--engine', self.pdflatex, '-n', '2', '--cpu', '3', '--max-load', '64', '--allow-unverified', *filt)
        run = latest_run(self.runs, 'bench')
        agg1 = read_json(os.path.join(run, 'results.json'))['aggregate']
        self.check('--cpu and --max-load are accepted and recorded', p.returncode == 0 and agg1['cpu_pinned'] == 3 and agg1['max_load'] == 64.0, f'cpu {agg1["cpu_pinned"]}, max load {agg1["max_load"]}')
        slow = self.wrapper('slow200', f'sleep 0.2\nexec "{self.pdflatex}" "$@"\n')
        p = self.cli('bench', '--runs-dir', self.runs, '--engine', slow, '-n', '3', '--allow-unverified', *filt)
        run = latest_run(self.runs, 'bench')
        agg2 = read_json(os.path.join(run, 'results.json'))['aggregate']
        results = {r['id']: r for r in read_json(os.path.join(run, 'results.json'))['results'] if r['status'] == 'timed'}
        ok = bool(results) and all(abs(r['candidate_ms'] - r['oracle_ms'] - 200) < 40 for r in results.values())
        self.check('a candidate slowed by 200 ms measures about 200 ms slower, and speed-up falls below 1', ok and agg2['best'] < 1,
                   ', '.join(f'{r["candidate_ms"] - r["oracle_ms"]:.0f} ms' for r in results.values()) + f'; best {agg2["best"]:.3f}x')
        # An engine that remembers its results in its cache: min-of-N must not become the time of a cache hit.
        replay = self.wrapper('replay', f'''
out=""; for a in "$@"; do case "$a" in -output-directory=*) out="${{a#-output-directory=}}";; esac; done
c="${{XDG_CACHE_HOME:-$HOME/.cache}}/rstex-selfcheck-replay/$(pwd | sha1sum | cut -c1-12)-$(echo "${{@: -1}}" | sha1sum | cut -c1-12)"
if [ -d "$c" ]; then cp -a "$c"/. "$out"/; exit 0; fi
"{self.pdflatex}" "$@"; rc=$?
mkdir -p "$c"; cp -a "$out"/. "$c"/; exit $rc
''')
        p = self.cli('bench', '--runs-dir', self.runs, '--engine', replay, '-n', '3', '--allow-unverified', *filt)
        run = latest_run(self.runs, 'bench')
        agg3 = read_json(os.path.join(run, 'results.json'))['aggregate']
        self.check('an engine cache is empty on every run, so a replay from it is not counted as speed', agg3['best'] < 1.25, f'best {agg3["best"]:.3f}x')
        p = self.cli('bench', '--runs-dir', self.runs, '--engine', self.pdflatex, '-n', '2', '--cold', '--allow-unverified', *filt)
        run = latest_run(self.runs, 'bench')
        self.check('--cold runs and says so', p.returncode == 0 and read_json(os.path.join(run, 'results.json'))['aggregate']['cold'] and 'page cache emptied' in p.stdout)

    def coverage_checks(self):
        print('\n== coverage ==', flush=True)
        out = os.path.join(self.root, 'coverage.json')
        args = ['coverage', '--runs-dir', self.runs, '-j', self.j, '--manifest', self.ladder, '--trace-all', '--out', out]
        p = self.cli(*args, timeout=3600)
        print(p.stdout)
        if not self.check('coverage exits 0', p.returncode == 0, (p.stderr or '')[-300:] if p.returncode else ''):
            return
        c = read_json(out)
        pr = c['primitives']
        ex = pr['executed']
        self.check('classes, packages, fonts and image types are seen',
                   'article.cls' in c['classes'] and 'hyperref.sty' in c['packages'] and 'cmr10.tfm' in c['fonts']
                   and all(k in c['filetypes'] for k in ('jpg', 'png', 'pdf', 'pfb', 'enc', 'map')),
                   f'{len(c["classes"])} classes, {len(c["packages"])} packages, {len(c["fonts"])} fonts')
        self.check('microtype trap: lpcode and rpcode are counted though traced without a backslash',
                   ex.get('lpcode', 0) > 0 and ex.get('rpcode', 0) > 0 and pr['bare_escape_counts'].get('lpcode', 0) > 0,
                   f'lpcode {ex.get("lpcode")} of which bare {pr["bare_escape_counts"].get("lpcode")}, rpcode {ex.get("rpcode")}')
        self.check('ordinary primitives are counted', ex.get('def', 0) > 1000 and ex.get('expandafter', 0) > 1000 and ex.get('pdfliteral', 0) > 0,
                   f'def {ex.get("def")}, expandafter {ex.get("expandafter")}, pdfliteral {ex.get("pdfliteral")}')
        self.check('the trace parser leaves nothing unexplained', not pr['trace_lines_not_understood'], str(list(pr['trace_lines_not_understood'].items())[:8]))
        self.check('every traced name is a known primitive', not pr['trace_names_not_in_list'], str(list(pr['trace_names_not_in_list'].items())[:8]))
        self.check('every trace ran as the oracle runs without tracing', not pr['trace_failed'], str(pr['trace_failed']))


# ------------------------------------------------------------------------------------------------ CLI
def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest='cmd', required=True)

    def add_selection(p):
        p.add_argument('--manifest', action='append', help='manifest file or glob; repeatable. Default: tests/corpus/manifests/*.json')
        p.add_argument('--tier', choices=TIERS, help='this tier and the ones below it (smoke < standard < full)')
        p.add_argument('--tier-exact', action='store_true', help='only the tier named by --tier')
        p.add_argument('--source', action='append', help='only this source; repeatable')
        p.add_argument('--filter', metavar='REGEX', help='only ids matching this regular expression')
        p.add_argument('--only-failed-last-run', action='store_true', help='only entries that failed in the most recent compare run')
        p.add_argument('-j', '--jobs', type=int, default=DEFAULT_JOBS, help=f'concurrent entries (default {DEFAULT_JOBS})')
        p.add_argument('--runs-dir', help=f'where runs are stored (default {DEFAULT_RUNS})')
        p.add_argument('--disk-budget-gb', type=float, default=40, help='stop starting entries when target/corpus exceeds this; such entries are reported as skipped')

    def add_engine(p):
        p.add_argument('--engine', required=True, help='candidate, invoked exactly like pdflatex')
        p.add_argument('--engine-arg', action='append', default=[], help='extra argument placed first; repeatable')
        p.add_argument('--timeout-scale', type=float, default=1.0, help='multiply candidate timeouts (a debug build may need 10)')

    p = sub.add_parser('list', help='show the selection and its oracle state')
    add_selection(p)
    p.set_defaults(func=list_cmd)

    p = sub.add_parser('prepare', help='oracle stage: compile twice from clean, classify, cache')
    add_selection(p)
    p.add_argument('--force', action='store_true', help='ignore cached oracles')
    p.add_argument('--retry-oracle-fails', action='store_true', help='recompile cached oracle failures')
    p.add_argument('--run-id')
    p.set_defaults(func=prepare_cmd)

    p = sub.add_parser('compare', help='run a candidate and compare it with the oracle')
    add_selection(p)
    add_engine(p)
    p.add_argument('--mode', choices=('last-pass', 'from-scratch', 'both'), default='both',
                   help='both by default: a last pass alone starts from the oracle\'s converged .aux, so it cannot see a bug in how passes feed each other')
    p.add_argument('--raster', choices=('all', 'none'), default='all', help='L7: render all pages of both sides, or skip it')
    p.add_argument('--dpi', type=int, default=100)
    p.add_argument('--l6', action=argparse.BooleanOptionalAction, default=True, help='L6 uncompressed bytes, in the instrumented pass (default on)')
    p.add_argument('--l1', action=argparse.BooleanOptionalAction, default=True, help='L1 assignment and restore trace, in the instrumented pass (default on)')
    p.add_argument('--l1-max-ms', type=float, default=2000, help='no L1 on an entry whose oracle pass takes more CPU time: its trace can reach a gigabyte, and small documents reach the same code. Counted in the summary')
    p.add_argument('--fls', action='store_true', help='pass -recorder to the candidate and compare the file list (L0)')
    p.add_argument('--box-log', choices=('never', 'failed', 'always'), default='always', help='L2 box logs, in the instrumented pass (default always: a 1 sp bug shows nowhere else)')
    p.add_argument('--gate-log', action=argparse.BooleanOptionalAction, default=True, help='a normalized log difference fails the entry (default on)')
    p.add_argument('--bringup', action='store_true', help='last pass, no L1, L6 or log gate, box logs only on failure: quick, and not the gate')
    p.add_argument('--tokens', action='store_true', help='take the token of each level of every entry that differs, as known_divergences.txt holds them, and not only of those it lists: results.jsonl has them under "tokens"')
    p.add_argument('--show-differences', metavar='DIR', help='write what each token covers to DIR, to be read before a line goes on the list; never for a private entry')
    p.add_argument('--no-list', action='store_true', help='read no known_divergences.txt, as for building it: every difference shows')
    p.add_argument('--known-list', metavar='FILE', help='this list in place of known_divergences.txt')
    p.add_argument('--retry-oracle-fails', action='store_true')
    p.add_argument('--resume', nargs='?', const='latest', help='continue the latest run, or the named one')
    p.add_argument('--run-id')
    p.set_defaults(func=compare_cmd)

    p = sub.add_parser('smoke', help='bring-up check in under a minute: the smoke tier, last pass, no instrumented pass. Not the gate')
    add_selection(p)
    add_engine(p)
    p.add_argument('--timeout-cap', type=float, default=8, help='seconds per candidate run (default 8), so that hangs on every entry still end within the minute. A debug build may need 60')
    p.set_defaults(func=smoke_cmd, jobs=8)

    p = sub.add_parser('boxlog', help='L2 on demand for one entry')
    p.add_argument('id')
    add_engine(p)
    p.add_argument('--manifest', action='append')
    p.set_defaults(func=boxlog_cmd)

    p = sub.add_parser('bench', help='serial timing, min of N, oracle against candidate')
    add_selection(p)
    add_engine(p)
    p.add_argument('-n', type=int, default=5, help='runs per side per entry; the minimum is kept')
    p.add_argument('--max-load', type=float, help='wait until the 1-minute load average is below this before each entry')
    p.add_argument('--load-patience', type=float, default=600, help='seconds to wait for --max-load before measuring anyway (recorded)')
    p.add_argument('--cpu', type=int, help='pin oracle and candidate to this CPU')
    p.add_argument('--allow-unverified', action='store_true', help='count entries with no passing compare run of this binary. The default is not to: a wrong answer can be fast')
    p.add_argument('--cold', action='store_true', help='drop the files the compile reads from the page cache before every run, with posix_fadvise: the time after a reboot or a long pause')
    p.add_argument('--all-passes', action='store_true', help='time all passes from nothing, as latexmk would, instead of the last pass')
    p.add_argument('--keep-engine-cache', action='store_true', help='let the candidate keep its snapshot cache between runs: the warm figure, reported as such')
    p.add_argument('--resume', nargs='?', const='latest')
    p.add_argument('--run-id')
    p.set_defaults(func=bench_cmd)

    p = sub.add_parser('coverage', help='what the corpus exercises')
    add_selection(p)
    p.add_argument('--no-trace', action='store_true')
    p.add_argument('--trace-all', action='store_true')
    p.add_argument('--trace-sample', type=int, default=50, help='entries to trace, stratified by source (default 50)')
    p.add_argument('--seed', type=int, default=1)
    p.add_argument('--primitives', help='default: tests/primitives.json, else the inventory from the verification research')
    p.add_argument('--out', help='default: target/corpus/coverage/latest.json')
    p.set_defaults(func=coverage_cmd)

    p = sub.add_parser('report', help='print the summary of a compare run')
    p.add_argument('run', nargs='?')
    p.add_argument('--runs-dir')
    p.set_defaults(func=report_cmd)

    p = sub.add_parser('selfcheck', help='prove the runner can pass and can fail')
    p.add_argument('-j', '--jobs', type=int, default=6)
    p.set_defaults(func=selfcheck_cmd)

    args = ap.parse_args()
    # Paths on the command line mean what they meant where the user stood; after that the runner works from the repo.
    if getattr(args, 'manifest', None):
        args.manifest = [os.path.abspath(m) for m in args.manifest]
    if getattr(args, 'engine', None) and '/' in args.engine:
        args.engine = os.path.abspath(args.engine)
    for name in ('primitives', 'out', 'runs_dir', 'known_list', 'show_differences'):
        if getattr(args, name, None):
            setattr(args, name, os.path.abspath(getattr(args, name)))
    os.makedirs(WORK_ROOT, exist_ok=True)
    os.makedirs(EMPTY_TEXMF, exist_ok=True)
    os.chdir(REPO)
    install_limits()

    def on_term(*_):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, on_term)
    # A shell's background job starts with SIGINT ignored, and then Python raises nothing for it.
    signal.signal(signal.SIGINT, signal.default_int_handler)
    try:
        return args.func(args)
    except ManifestError as ex:
        print(f'manifest error: {ex}', file=sys.stderr)
        return 2
    except KeyboardInterrupt:
        kill_all_active()
        return 130


if __name__ == '__main__':
    sys.exit(main())
