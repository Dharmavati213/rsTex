#!/usr/bin/env -S python3 -I
r"""Is a preamble snapshot sound?  Each case changes the outside world between runs and asserts that the next run equals a cold compile.

    python3 -I tests/corpus/snapshot/run_snapshot.py --engine ENGINE [--cold-engine ENGINE] [--filter REGEX] [-j N] [--pdf-only] [--out FILE] [--keep]
    python3 -I tests/corpus/snapshot/run_snapshot.py --check-mutants      # prove the group can fail, on engines that are wrong on purpose
    python3 -I tests/corpus/snapshot/run_snapshot.py --list

DESIGN section 7: "a checkpoint records an ordered log of every observation of the outside world and is valid only if each observation would
get the same answer now".  There is one group of cases per kind of observation.  Protocol for a case, all in one directory, with one clock
and one engine cache:

  1. warm    run the engine twice on world W1, so that a snapshot exists, and is restored at least once
  2. W1'     save the source tree and output directory as they now stand
  3. change  change the world into W2: edit a file, make a file appear, move the clock, change a flag or a variable
  4. W2'     save the source tree and output directory as they now stand
  5. check   run the engine on W2
  6. cold    put W2' back and run the cold engine, with no cache; put W1' back and do it again for the reference below

Pass: the check run equals the cold run on W2', in the PDF bytes, the normalized log, every side file and the exit status, and what the terminal saw if the cold engine is the same program.
A case also fails as VACUOUS if the cold run on W1' equals the one on W2' although the case says the change is visible: such a test cannot catch
a stale snapshot, and passes for nothing.  Cases that say the change is invisible (touching a file) assert only the equality.

The cold engine defaults to the system pdflatex.  For rstex: --cold-engine "target/release/rstex --no-snapshot".  Every path stays the same between
the check run and the cold run, so logs compare without masks.  With --engine pdflatex there is no cache at all, so all cases must pass, and a
failure is a broken test.  Everything runs under target/corpus/snapshot/.  Nothing here writes to the user's documents.

The cases with `custom` have a protocol of their own, for a cache that is hostile, damaged or raced.  They are for rstex alone, and say by what they return what went wrong.  A case that
says `must` has to be a restore, and one that says `no_restore` must not be.
"""
import argparse
import concurrent.futures as cf
import importlib.util
import json
import os
import re
import shlex
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
spec = importlib.util.spec_from_file_location('corpus_run', os.path.join(os.path.dirname(HERE), 'run.py'))
run_py = importlib.util.module_from_spec(spec)
spec.loader.exec_module(run_py)
ROOT = os.path.join(REPO, 'target', 'corpus', 'snapshot')
JPEG = os.path.join(REPO, 'tests', 'ladder', 'ladder-logo.jpg')
EPOCH = 1700000000      # 2023-11-14 22:13:20 UTC
HEAD = r'\pdfcompresslevel=0 \pdfobjcompresslevel=0 \pdftrailerid{}' + '\n'
ARTICLE = HEAD + r'\documentclass{article}' + '\n'
os.makedirs(run_py.EMPTY_TEXMF, exist_ok=True)


def doc(preamble='', body='Body text.\n'):
    return ARTICLE + preamble + '\n\\begin{document}\n' + body + '\\end{document}\n'


class World:
    """One test's outside world.  `src` is the current directory of the run unless a change moves it, `out` the output directory."""

    def __init__(self, base, case):
        self.base = base
        self.src = os.path.join(base, 'src')
        self.out = os.path.join(base, 'out')
        self.cache = os.path.join(base, 'cache')
        os.makedirs(self.src)
        os.makedirs(self.out)
        os.makedirs(self.cache)
        self.cwd = self.src
        self.main = case.get('main', 'main.tex')
        self.job = case.get('job') or os.path.splitext(os.path.basename(self.main))[0]
        self.args = list(case.get('args', []))
        self.first_line = None
        self.stdin = case.get('stdin')
        self.outdir = self.out
        self.no_outdir = case.get('no_outdir', False)
        if self.no_outdir:
            self.outdir = self.src
        self.fsize_kb = case.get('fsize_kb')
        self.drop_env = list(case.get('drop_env', []))
        self.env = {'SOURCE_DATE_EPOCH': str(EPOCH), 'FORCE_SOURCE_DATE': '1', 'TZ': 'UTC', 'LC_ALL': 'C', 'LANG': 'C'}
        self.env.update(case.get('env', {}))
        for name, content in case.get('files', {}).items():
            self.write(name, content)

    def path(self, name):
        return os.path.join(self.src, name)

    def write(self, name, content, keep_stat=False):
        p = self.path(name)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        st = os.stat(p) if keep_stat and os.path.exists(p) else None
        data = content if isinstance(content, bytes) else content.encode()
        if st is not None:             # same inode: no rename
            with open(p, 'r+b') as f:
                f.write(data)
                f.truncate()
            os.utime(p, ns=(st.st_atime_ns, st.st_mtime_ns))
        else:
            with open(p, 'wb') as f:
                f.write(data)

    def read(self, name):
        with open(self.path(name), 'rb') as f:
            return f.read()

    def edit(self, name, old, new, keep_stat=False):
        d = self.read(name)
        o, n = old.encode(), new.encode()
        if o not in d:
            raise RuntimeError(f'edit of {name}: {old!r} not found')
        self.write(name, d.replace(o, n, 1), keep_stat)

    def rm(self, name):
        os.unlink(self.path(name))

    def touch(self, name, delta=3600):
        p = self.path(name)
        st = os.stat(p)
        os.utime(p, ns=(st.st_atime_ns, st.st_mtime_ns + delta * 10**9))

    def write_out(self, name, content):
        p = os.path.join(self.outdir, name)
        with open(p, 'wb') as f:
            f.write(content if isinstance(content, bytes) else content.encode())

    def rm_out(self, name):
        os.unlink(os.path.join(self.outdir, name))

    def clock(self, seconds):
        self.env['SOURCE_DATE_EPOCH'] = str(int(self.env['SOURCE_DATE_EPOCH']) + seconds)

    def save(self, tag):
        d = os.path.join(self.base, tag)
        force_rmtree(d)
        os.makedirs(d)
        for name in ('src', 'out'):
            if os.path.isdir(os.path.join(self.base, name)):        # a case may have removed the output directory
                shutil.copytree(os.path.join(self.base, name), os.path.join(d, name), symlinks=True, copy_function=copy_even_if_unreadable)
        return d

    def restore(self, tag):
        d = os.path.join(self.base, tag)
        for name in ('src', 'out'):
            force_rmtree(os.path.join(self.base, name))
            if os.path.isdir(os.path.join(d, name)):
                shutil.copytree(os.path.join(d, name), os.path.join(self.base, name), symlinks=True, copy_function=copy_even_if_unreadable)

    def run_env(self, cache=None):
        env = {k: os.environ[k] for k in ('PATH', 'HOME') if k in os.environ}
        env.update(run_py.NO_MKTEX)
        env.update(run_py.HERMETIC)
        env.update(self.env)
        env = {k: v for k, v in env.items() if v is not None}
        env['XDG_CACHE_HOME'] = cache or self.cache
        env['RSTEX_CHECKPOINT_TRACE'] = '1'     # an engine with no such thing ignores it; rstex says on stderr what it did
        env['RSTEX_CHECKPOINT_POLICY'] = 'always'   # the second run restores, however small the preamble: policy.py tests what a run does by itself
        for k in self.drop_env:
            env.pop(k, None)
        return env

    def argv(self, engine):
        fmt = [a for a in self.args if a.startswith(('-fmt=', '-ini'))]
        interaction = [a for a in self.args if a.startswith('-interaction')]
        tail = self.first_line if self.first_line else self.main
        argv = list(engine)
        if not fmt:
            argv.append('-fmt=pdflatex')
        if not interaction:
            argv.append('-interaction=batchmode')
        return argv + ([] if self.no_outdir else [f'-output-directory={self.outdir}']) + self.args + [tail]

    def run(self, engine, cache=None, argv=None, timeout=300):
        env = self.run_env(cache)
        argv = argv or self.argv(engine)
        if self.fsize_kb:
            argv = ['sh', '-c', f'ulimit -f {self.fsize_kb}; exec "$@"', 'sh'] + argv
        p = subprocess.run(argv, cwd=self.cwd, env=env, input=(self.stdin or '').encode(), stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout)
        said = b''.join(line for line in p.stderr.splitlines(True) if not line.startswith(b'rstex: checkpoint:'))
        # What the terminal saw, for judging an engine against itself: its standard output, then what it said on standard error.
        return {'rc': p.returncode, 'stdout': p.stdout[-2000:].decode('latin-1'), 'restored': b'rstex: checkpoint: restored' in p.stderr, 'terminal': p.stdout + b'\0' + said, 'trace': p.stderr.decode('latin-1')}

    def outputs(self):
        """Everything the run left in its output directory."""
        res = {}
        for root, _, names in os.walk(self.outdir):
            for n in names:
                p = os.path.join(root, n)
                res[os.path.relpath(p, self.outdir)] = run_py.open_bytes(p)
        return res


def force_rmtree(path):
    """rmtree, also of what a case made read-only."""
    if not os.path.lexists(path):
        return
    for root, dirs, _ in os.walk(path):
        for d in dirs:
            q = os.path.join(root, d)
            if not os.path.islink(q):
                os.chmod(q, 0o755)
    if not os.path.islink(path):
        os.chmod(path, 0o755)
    shutil.rmtree(path, ignore_errors=True)


def copy_even_if_unreadable(src, dst):
    """copy2, also of a file a case made unreadable, which keeps its mode."""
    if os.path.islink(src):
        return shutil.copy2(src, dst, follow_symlinks=False)
    mode = os.stat(src).st_mode & 0o777
    if mode & 0o400:
        return shutil.copy2(src, dst)
    os.chmod(src, mode | 0o400)
    try:
        shutil.copy2(src, dst)
    finally:
        os.chmod(src, mode)
    os.chmod(dst, mode)
    return dst


def judge(a, b, job, workdirs, pdf_only=False):
    """First difference between two sets of outputs: candidate A, cold B."""
    pdf = job + '.pdf'
    if (pdf in a) != (pdf in b):
        return f'PDF present in {"check" if pdf in a else "cold"} run only'
    if pdf_only:
        # A dump by another route numbers its fonts and objects differently, so this judges what the reader gets: page content and structure.
        if pdf in a and a[pdf] != b[pdf]:
            d = tempfile.mkdtemp(dir=os.path.dirname(workdirs[0]))
            try:
                for name, data in (('a.pdf', a[pdf]), ('b.pdf', b[pdf])):
                    with open(os.path.join(d, name), 'wb') as f:
                        f.write(data)
                c = run_py.run_compare_pdf(os.path.join(d, 'a.pdf'), os.path.join(d, 'b.pdf'))
                if c['status'] != 'identical':
                    return c.get('text') or c.get('error') or c['status']
            finally:
                shutil.rmtree(d, ignore_errors=True)
        return None
    if pdf in a and a[pdf] != b[pdf]:
        x, y = a[pdf], b[pdf]
        if run_py.l6_mask(x) != run_py.l6_mask(y):
            k = run_py.first_byte_diff(x, y)
            return f'PDF bytes differ at offset {k}: check `{run_py.clip(x[max(0, k - 30):k + 40])}` | cold `{run_py.clip(y[max(0, k - 30):k + 40])}`'
    log = job + '.log'
    la = run_py.normalize_log(a.get(log, b''), workdirs)
    lb = run_py.normalize_log(b.get(log, b''), workdirs)
    d = run_py.first_log_divergence(la, lb)
    if d:
        return f'log line {d["line_a"]}/{d["line_b"]}: check `{d["a"][:140]}` | cold `{d["b"][:140]}`'
    for name in sorted(set(a) | set(b)):
        if name in (pdf, log):
            continue
        if name not in a or name not in b:
            return f'side file {name} is only in the {"check" if name in a else "cold"} run'
        if a[name] != b[name]:
            return f'side file {name} differs'
    return None


def tiny_png(width, height, shade, alpha=False):
    def chunk(t, d):
        return struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d))
    raw = b''.join(b'\0' + bytes([shade, 128] if alpha else [shade]) * width for _ in range(height))
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 4 if alpha else 0, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b'')


def mutated_tfm():
    """cmss10 with one kern changed, for the shadow test.  cmss10 because the LaTeX format preloads cmr10 and never opens its TFM."""
    d = os.path.join(ROOT, 'assets')
    os.makedirs(d, exist_ok=True)
    tfm = os.path.join(d, 'cmss10.tfm')
    if not os.path.isfile(tfm):
        src = subprocess.run(['kpsewhich', 'cmss10.tfm'], capture_output=True, text=True).stdout.strip()
        pl = subprocess.run(['tftopl', src], capture_output=True, text=True, errors='replace').stdout
        new = pl.replace('(KRN C V R -0.111112)', '(KRN C V R -0.222223)', 1)
        assert new != pl
        plf = os.path.join(d, 'cmss10.pl')
        with open(plf, 'w') as f:
            f.write(new)
        subprocess.run(['pltotf', plf, tfm], capture_output=True, check=True)
    with open(tfm, 'rb') as f:
        return f.read()


def jpeg_with_height_changed():
    with open(JPEG, 'rb') as f:
        d = bytearray(f.read())
    i = 2
    while i < len(d):
        if d[i] != 0xFF:
            raise RuntimeError('bad JPEG')
        m = d[i + 1]
        n = (d[i + 2] << 8) + d[i + 3]
        if m in (0xC0, 0xC1, 0xC2):
            h = (d[i + 5] << 8) + d[i + 6]
            d[i + 5:i + 7] = bytes([(h - 1) >> 8, (h - 1) & 255])
            return bytes(d)
        i += 2 + n
    raise RuntimeError('no SOF')


def jpeg_with_payload_changed():
    with open(JPEG, 'rb') as f:
        d = bytearray(f.read())
    k = d.index(b'\xff\xda') + 40
    d[k] ^= 1
    return bytes(d)


with open(JPEG, 'rb') as _f:
    JPEG_BYTES = _f.read()


def tiny_pdf(width):
    """One empty page, for \\pdfximage."""
    objs = [b'<< /Type /Catalog /Pages 2 0 R >>', b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
            b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 %d 20] /Resources << >> >>' % width]
    out = b'%PDF-1.4\n'
    offsets = []
    for k, o in enumerate(objs):
        offsets.append(len(out))
        out += b'%d 0 obj\n' % (k + 1) + o + b'\nendobj\n'
    x = len(out)
    out += b'xref\n0 4\n0000000000 65535 f \n' + b''.join(b'%010d 00000 n \n' % n for n in offsets)
    out += b'trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n' % x
    return out

SHOW_V = r'Value: \V.' + '\n'


def case(rule, cid, *, files=None, change=None, visible=True, why='', **kw):
    d = {'id': f'{rule}/{cid}', 'rule': rule, 'files': files or {}, 'change': change, 'visible': visible, 'why': why}
    d.update(kw)
    return d


def _appear_shadow(w):
    w.write('pk.sty', r'\def\V{shadow from the current directory}')


CASES = [
    # ------------------------------------------------------------------ Config: flags, directories, path specs, environment
    case('config', 'jobname', why='\\jobname read in the preamble; the key must have the job name', files={'main.tex': doc(r'\edef\V{\jobname}', SHOW_V)}, args=['-jobname=first'], job='first',
         change=lambda w: (w.args.__setitem__(0, '-jobname=second'), setattr(w, 'job', 'second'))),
    case('config', 'output-directory', why='a file found in the output directory: \\IfFileExists sees another directory', files={'main.tex': doc(r'\IfFileExists{note.tex}{\input{note.tex}}{\def\V{no note}}', SHOW_V), 'other/note.tex': r'\def\V{note in the other output directory}'},
         change=lambda w: setattr(w, 'outdir', w.path('other'))),
    case('config', 'cwd', why='the same main file with another current directory and so another macros.tex', files={'main.tex': doc(r'\input{macros}', SHOW_V), 'macros.tex': r'\def\V{first directory}', 'b/main.tex': doc(r'\input{macros}', SHOW_V), 'b/macros.tex': r'\def\V{second directory}'},
         change=lambda w: setattr(w, 'cwd', w.path('b'))),
    case('config', 'texinputs', why='TEXINPUTS is in the key', files={'main.tex': doc(r'\usepackage{pk}', SHOW_V), 'a/pk.sty': r'\def\V{package a}', 'b/pk.sty': r'\def\V{package b}'},
         env={'TEXINPUTS': 'a:'}, change=lambda w: w.env.__setitem__('TEXINPUTS', 'b:')),
    case('config', 'source-date-epoch', why='a date read in the preamble, one day later', files={'main.tex': doc(r'\edef\V{\the\year-\the\month-\the\day}', SHOW_V)}, change=lambda w: w.clock(86400)),
    case('config', 'force-source-date', why='FORCE_SOURCE_DATE decides whether \\year is the real one',
         files={'main.tex': doc(r'\edef\V{\the\year}', SHOW_V)}, change=lambda w: w.env.pop('FORCE_SOURCE_DATE')),
    case('config', 'interaction', why='\\interactionmode read in the preamble', files={'main.tex': doc(r'\edef\V{\the\interactionmode}', SHOW_V)}, args=['-interaction=batchmode'],
         change=lambda w: w.args.__setitem__(0, '-interaction=nonstopmode')),
    case('config', 'shell-flag', why='\\pdfshellescape read in the preamble', files={'main.tex': doc(r'\edef\V{\the\pdfshellescape}', SHOW_V)}, args=['-no-shell-escape'],
         change=lambda w: w.args.__setitem__(0, '-shell-restricted')),
    case('config', 'first-line', why='TeX on the command line before the document', files={'main.tex': doc('', r'\V' + '\n')}, first_line=r'\def\V{from the first line}\input{main}',
         change=lambda w: setattr(w, 'first_line', r'\def\V{from another first line}\input{main}')),
    case('config', 'max-print-line', why='only the log wraps differently, and a restored log prefix must wrap the same', files={'main.tex': doc(r'\typeout{A message long enough that it has to wrap when the line width comes down from the default of seventy nine}')},
         change=lambda w: w.env.__setitem__('max_print_line', '200')),
    case('config', 'texmfcnf-path', why='a path spec comes from a changed texmf.cnf, not from a variable: the expanded spec has to be what is keyed', files={'main.tex': doc(r'\usepackage{pk}', SHOW_V), 'a/pk.sty': r'\def\V{package a}', 'b/pk.sty': r'\def\V{package b}', 'cnf/texmf.cnf': 'TEXINPUTS.pdflatex = .:@HERE@/a:$TEXMF/tex/{latex,generic,}//\n'},
         env={'TEXMFCNF': '@HERE@/cnf:'}, change=lambda w: w.write('cnf/texmf.cnf', f'TEXINPUTS.pdflatex = .:{w.src}/b:$TEXMF/tex/{{latex,generic,}}//\n')),
    case('config', 'format-rebuilt', why='a format of the same name with other content: the format hash is in the key', files={'main.tex': doc('', r'\FORMATMARK' + '\n')}, custom_format='first', change=lambda w: w.rebuild_format('second')),
    # ------------------------------------------------------------------ lookup: resolved path or NOT FOUND, shadow probes included
    case('lookup', 'new-file-shadows-package', why='the file was found elsewhere before; a new one in the current directory comes first', files={'main.tex': doc(r'\usepackage{pk}', SHOW_V), 'lib/pk.sty': r'\def\V{from lib}'}, env={'TEXINPUTS': '.:lib:'}, change=_appear_shadow),
    case('lookup', 'hyperref-cfg-appears', why='a failed probe in a real package: 25 such probes in the thesis preamble', files={'main.tex': HEAD + r'\documentclass{article}\usepackage{hyperref}' + '\n' + r'\begin{document}Body.\end{document}' + '\n'}, change=lambda w: w.write('hyperref.cfg', r'\hypersetup{pdfauthor={from a new hyperref.cfg}}')),
    case('lookup', 'iffileexists-appears', files={'main.tex': doc(r'\IfFileExists{opt.tex}{\input{opt}}{\def\V{absent}}', SHOW_V)}, change=lambda w: w.write('opt.tex', r'\def\V{present}')),
    case('lookup', 'iffileexists-removed', files={'main.tex': doc(r'\IfFileExists{opt.tex}{\input{opt}}{\def\V{absent}}', SHOW_V), 'opt.tex': r'\def\V{present}'}, change=lambda w: w.rm('opt.tex')),
    case('lookup', 'implied-extension-shadow', why='\\input foo.bar tries foo.bar.tex first: a new file wins over the old one without being its name', files={'main.tex': doc(r'\input foo.bar ', SHOW_V), 'foo.bar': r'\def\V{from foo.bar}'}, change=lambda w: w.write('foo.bar.tex', r'\def\V{from foo.bar.tex}')),
    case('lookup', 'class-shadow', why='a local copy of the class', files={'main.tex': doc('', 'Body.\n')}, change=lambda w: w.write('article.cls', r'\NeedsTeXFormat{LaTeX2e}\ProvidesClass{article}\LoadClass{report}\AtBeginDocument{Local class.}')),
    case('lookup', 'tfm-shadow', why='a font loaded in the preamble: its TFM gets shadowed', files={'main.tex': doc(r'\font\x=cmss10 \setbox0=\hbox{\x AV}\edef\V{\the\wd0}', SHOW_V)}, change=lambda w: w.write('cmss10.tfm', mutated_tfm())),
    case('lookup', 'map-file-appears', why='\\pdfmapfile in the preamble reads a project map file', files={'main.tex': doc(r'\pdfmapfile{=local.map}', r'\font\x=cmss10 {\x Hello}' + '\n'), 'local.map': 'cmss10 CMSS10 <cmss10.pfb\n'}, change=lambda w: w.write('local.map', 'cmss10 CMSS12 <cmss12.pfb\n')),
    case('lookup', 'input-moved', why='the same name resolves to another directory', files={'main.tex': doc(r'\usepackage{pk}', SHOW_V), 'lib1/pk.sty': r'\def\V{first}', 'lib2/x': 'x'}, env={'TEXINPUTS': 'lib1:lib2:'}, change=lambda w: (w.rm('lib1/pk.sty'), w.write('lib2/pk.sty', r'\def\V{moved}'))),
    # ------------------------------------------------------------------ reads: whole-file hash, prefix hash, stat for TDS files
    case('read', 'preamble-edit', files={'main.tex': doc(r'\def\V{old}', SHOW_V)}, change=lambda w: w.edit('main.tex', r'\def\V{old}', r'\def\V{new}')),
    case('read', 'body-edit', why='the one the snapshot is for: body changes, preamble does not', files={'main.tex': doc(r'\def\V{same}', 'Old body.\n')}, change=lambda w: w.edit('main.tex', 'Old body.', 'New body with\\newpage two pages.')),
    case('read', 'input-same-size-same-mtime', why='project files are hashed, not stat-ed: same size, same mtime, same inode', files={'main.tex': doc(r'\input{macros}', SHOW_V), 'macros.tex': r'\def\V{aaa}'}, change=lambda w: w.edit('macros.tex', 'aaa', 'bbb', keep_stat=True)),
    case('read', 'input-touch-only', visible=False, why='no change of content, so it must still equal the cold run (and may hit)', files={'main.tex': doc(r'\input{macros}', SHOW_V), 'macros.tex': r'\def\V{aaa}'}, change=lambda w: w.touch('macros.tex')),
    case('read', 'open-read-file-tail', why='\\openin left open across the checkpoint; only the prefix through the read position was observed', files={'main.tex': doc(r'\newread\r \openin\r=data.txt \read\r to\A', r'\A\ \read\r to\B \B' + '\n'), 'data.txt': 'first line\nsecond line\n'}, change=lambda w: w.edit('data.txt', 'second line', 'other line')),
    case('read', 'open-read-file-prefix', files={'main.tex': doc(r'\newread\r \openin\r=data.txt \read\r to\A', r'\A\ \read\r to\B \B' + '\n'), 'data.txt': 'first line\nsecond line\n'}, change=lambda w: w.edit('data.txt', 'first line', 'other line')),
    case('read', 'closed-read-file-later-line', why='a closed file is hashed whole, though only line 1 was read', visible=False, files={'main.tex': doc(r'\newread\r \openin\r=data.txt \read\r to\A \closein\r', r'\A' + '\n'), 'data.txt': 'first line\nsecond line\n'}, change=lambda w: w.edit('data.txt', 'second line', 'other line')),
    case('read', 'tds-tree-changed', why='a package in a TDS tree with an ls-R: the stat changes', files={'main.tex': doc(r'\usepackage{pk}', SHOW_V), 'tree/ls-R': '% ls-R -- filename database for kpathsea; do not change this line.\n./:\ntex\n\n./tex:\nlatex\n\n./tex/latex:\npk.sty\n', 'tree/tex/latex/pk.sty': r'\def\V{old}'},
         env={'TEXMFHOME': '@HERE@/tree', 'TEXMFDBS': '@HERE@/tree:'}, change=lambda w: (w.edit('tree/tex/latex/pk.sty', 'old', 'newer'), w.touch('tree/tex/latex/pk.sty'))),
    case('read', 'tds-tree-same-stat', why='the same edit keeping size, mtime and inode.  The rule of section 7 stats TDS files by size, mtime and inode, so a stat-only key misses this; ctime does not', files={'main.tex': doc(r'\usepackage{pk}', SHOW_V), 'tree/ls-R': '% ls-R -- filename database for kpathsea; do not change this line.\n./:\ntex\n\n./tex:\nlatex\n\n./tex/latex:\npk.sty\n', 'tree/tex/latex/pk.sty': r'\def\V{old}'},
         env={'TEXMFHOME': '@HERE@/tree', 'TEXMFDBS': '@HERE@/tree:'}, change=lambda w: w.edit('tree/tex/latex/pk.sty', 'old', 'new', keep_stat=True)),
    case('read', 'tds-tree-new-file', why='ls-R rebuilt with a new file that shadows a later tree', files={'main.tex': doc(r'\IfFileExists{newpk.sty}{\usepackage{newpk}}{\def\V{absent}}', SHOW_V), 'tree/ls-R': '% ls-R -- filename database for kpathsea; do not change this line.\n./:\ntex\n\n./tex:\nlatex\n\n./tex/latex:\n'},
         env={'TEXMFHOME': '@HERE@/tree', 'TEXMFDBS': '@HERE@/tree:'}, change=lambda w: (w.write('tree/tex/latex/newpk.sty', r'\def\V{present}'), w.write('tree/ls-R', '% ls-R -- filename database for kpathsea; do not change this line.\n./:\ntex\n\n./tex:\nlatex\n\n./tex/latex:\nnewpk.sty\n'))),
    # ------------------------------------------------------------------ the preamble depends on what earlier passes wrote
    case('aux', 'preamble-reads-aux', why='section 7 assumes .aux is read after the checkpoint.  A package can read it earlier', files={'main.tex': doc(r'\makeatletter\newcommand\Found{none}\IfFileExists{\jobname.aux}{\def\newlabel#1#2{\def\Found{#2}}\input{\jobname.aux}\let\newlabel\@undefined}{}\makeatother', r'\Found\section{Sec}\label{a}' + '\n')}, change=lambda w: w.write_out('main.aux', r'\relax \newlabel{a}{{changed}{1}}' + '\n')),
    case('aux', 'preamble-reads-own-file', visible=False, why='a file the run itself wrote at its end, read back in the preamble of the next run', files={'main.tex': doc(r'\IfFileExists{main.state}{\input{main.state}}{\def\V{0}}', SHOW_V + r'\newwrite\o \immediate\openout\o=main.state \immediate\write\o{\string\def\string\V{\the\numexpr\V+1\relax}}\immediate\closeout\o' + '\n')}),
    case('aux', 'aux-deleted', why='latexmk -c, or a user cleaning up; the run reads the .aux after the checkpoint and must see that it is gone', files={'main.tex': doc('', r'\section{One}\label{a}See \ref{a}.' + '\n')}, change=lambda w: w.rm_out('main.aux')),
    case('aux', 'aux-stale-after-edit', why='a label moved in the source: the .aux is from before the edit, on both sides', files={'main.tex': doc('', r'\section{One}\label{a}\section{Two}\label{b}See \ref{a} and \ref{b}.' + '\n')}, change=lambda w: w.edit('main.tex', r'\section{One}\label{a}\section{Two}\label{b}', r'\section{One}\label{b}\section{Two}\label{a}')),
    case('aux', 'toc-changed', files={'main.tex': doc('', r'\tableofcontents\section{One}' + '\n')}, change=lambda w: w.write_out('main.toc', r'\contentsline {section}{\numberline {1}Changed}{1}%' + '\n')),
    case('aux', 'bbl-appears', why='bibtex ran between the passes', files={'main.tex': doc('', r'\cite{k}\bibliography{refs}\bibliographystyle{plain}' + '\n')}, change=lambda w: w.write_out('main.bbl', r'\begin{thebibliography}{1}\bibitem{k} A book.\end{thebibliography}' + '\n')),
    case('aux', 'filecontents-already-there', why='filecontents* does not overwrite: what is on disk decides, and an edit there has to win', files={'main.tex': HEAD + r'\begin{filecontents*}[noheader]{gen.tex}' + '\n' + r'\def\V{as generated}' + '\n' + r'\end{filecontents*}' + '\n' + r'\documentclass{article}\input{gen.tex}' + '\n' + r'\begin{document}\V.\end{document}' + '\n'}, change=lambda w: w.write_out('gen.tex', r'\def\V{edited by hand}')),
    # ------------------------------------------------------------------ stat-like primitives
    case('stat', 'pdffilesize', files={'main.tex': doc(r'\edef\V{\pdffilesize{data.bin}}', SHOW_V), 'data.bin': 'xxxx'}, change=lambda w: w.write('data.bin', 'xxxxx')),
    case('stat', 'pdffilesize-appears', files={'main.tex': doc(r'\edef\V{[\pdffilesize{data.bin}]}', SHOW_V)}, change=lambda w: w.write('data.bin', 'xxxx')),
    case('stat', 'pdffilemoddate', why='only the mtime changes; needs the real date, so FORCE_SOURCE_DATE is off', files={'main.tex': doc(r'\edef\V{\pdffilemoddate{data.bin}}', SHOW_V), 'data.bin': 'xxxx'}, env={'FORCE_SOURCE_DATE': None}, change=lambda w: w.touch('data.bin')),
    case('stat', 'pdfmdfivesum-same-size', files={'main.tex': doc(r'\edef\V{\pdfmdfivesum file{data.bin}}', SHOW_V), 'data.bin': 'aaaa'}, change=lambda w: w.write('data.bin', 'bbbb', keep_stat=True)),
    case('stat', 'pdffiledump', files={'main.tex': doc(r'\edef\V{\pdffiledump offset 1 length 2{data.bin}}', SHOW_V), 'data.bin': 'aabb'}, change=lambda w: w.write('data.bin', 'accb', keep_stat=True)),
    case('stat', 'image-header', why='image size read at \\pdfximage in the preamble, written at shipout', files={'main.tex': doc(r'\pdfximage{img.png}\edef\V{\the\pdflastximage}\setbox0=\hbox{\pdfrefximage\pdflastximage}\edef\W{\the\wd0}', r'\W\box0' + '\n'), 'img.png': tiny_png(8, 8, 200)}, change=lambda w: w.write('img.png', tiny_png(16, 8, 200))),
    case('stat', 'image-pixels-same-header', why='header unchanged: only the bytes written at shipout, after the checkpoint, differ', files={'main.tex': doc(r'\pdfximage{img.png}', r'\leavevmode\pdfrefximage\pdflastximage' + '\n'), 'img.png': tiny_png(8, 8, 200)}, change=lambda w: w.write('img.png', tiny_png(8, 8, 90))),
    case('stat', 'jpeg-header', files={'main.tex': doc(r'\pdfximage{img.jpg}\setbox0=\hbox{\pdfrefximage\pdflastximage}\edef\W{\the\ht0}', r'\W\box0' + '\n'), 'img.jpg': JPEG_BYTES}, change=lambda w: w.write('img.jpg', jpeg_with_height_changed())),
    case('stat', 'jpeg-payload', files={'main.tex': doc(r'\pdfximage{img.jpg}', r'\leavevmode\pdfrefximage\pdflastximage' + '\n'), 'img.jpg': JPEG_BYTES}, change=lambda w: w.write('img.jpg', jpeg_with_payload_changed())),
    case('stat', 'graphicx-size-from-file', why='\\includegraphics in a saved box in the preamble', files={'main.tex': doc(r'\usepackage{graphicx}\newsavebox\pic\sbox\pic{\includegraphics{img.png}}', r'\usebox\pic' + '\n'), 'img.png': tiny_png(8, 8, 200)}, change=lambda w: w.write('img.png', tiny_png(24, 8, 200))),
    # ------------------------------------------------------------------ volatile reads
    case('volatile', 'date-read', files={'main.tex': doc(r'\edef\V{\the\day}', SHOW_V)}, change=lambda w: w.clock(86400)),
    case('volatile', 'date-not-read-then-used', why='the preamble reads no date: a hit is right, and the restored \\day must be the new one', files={'main.tex': doc('', r'\today' + '\n')}, change=lambda w: w.clock(86400)),
    case('volatile', 'today-edef', files={'main.tex': doc(r'\edef\V{\today}', SHOW_V)}, change=lambda w: w.clock(86400)),
    case('volatile', 'time-read', files={'main.tex': doc(r'\edef\V{\the\time}', SHOW_V)}, change=lambda w: w.clock(60)),
    case('volatile', 'time-not-read-then-used', why='\\time refreshed on restore', files={'main.tex': doc('', r'\the\time' + '\n')}, change=lambda w: w.clock(60)),
    case('volatile', 'year-assigned', visible=False, why='refresh \\year unless the preamble assigned it', files={'main.tex': doc(r'\year=2000 ', r'\the\year' + '\n')}, change=lambda w: w.clock(86400 * 400)),
    case('volatile', 'c-sys-patched', why='expl3 constants of the clock: whitelisted and patched on restore', files={'main.tex': doc(r'\ExplSyntaxOn \cs_new:Npn \showminute { \int_use:N \c_sys_minute_int } \ExplSyntaxOff', r'\showminute' + '\n')}, change=lambda w: w.clock(60)),
    case('volatile', 'c-sys-copied-in-preamble', why='TRAP.  The read of \\c_sys_minute_int is whitelisted, so it taints nothing; patching the constant does not reach a copy made in the preamble', files={'main.tex': doc(r'\ExplSyntaxOn \cs_gset:Npx \V { \int_use:N \c_sys_minute_int } \ExplSyntaxOff', SHOW_V)}, change=lambda w: w.clock(60)),
    case('volatile', 'pgfmath-seed-used-in-body', why='TRAP.  The pgfmath seed read of \\time*\\year is whitelisted, but section 7 lists no patch of it on restore, and pgfmath keeps it in \\pgfmath@rnd@z', files={'main.tex': doc(r'\usepackage{pgfmath}', r'\pgfmathrandominteger\r{1}{1000000}\r' + '\n')}, change=lambda w: w.clock(60)),
    case('volatile', 'pinned-seed-state', why='\\pdfsetrandomseed and draws in the preamble: the generator state has to come back with the image', visible=True, files={'main.tex': doc(r'\pdfsetrandomseed 42 \edef\V{\pdfuniformdeviate 1000}', SHOW_V + r'\pdfuniformdeviate 1000' + '\n')}, change=lambda w: w.edit('main.tex', r'\pdfsetrandomseed 42', r'\pdfsetrandomseed 43')),
    case('volatile', 'pdfcreationdate-read', files={'main.tex': doc(r'\edef\V{\pdfcreationdate}', SHOW_V)}, change=lambda w: w.clock(60)),
    case('volatile', 'creationdate-in-info', why='the PDF /CreationDate after a restore, and /ID, which is computed from the clock and the path', files={'main.tex': r'\pdfcompresslevel=0 \pdfobjcompresslevel=0 ' + '\n' + r'\documentclass{article}\begin{document}Body.\end{document}' + '\n'}, change=lambda w: w.clock(60)),
    # ------------------------------------------------------------------ hard taint
    case('taint', 'write18-in-preamble', why='shell output read back', files={'main.tex': doc(r'\immediate\write18{cat ext.txt > got.tex}\input{got}', SHOW_V), 'ext.txt': r'\def\V{old output}'}, args=['-shell-escape'], env={'TEXMF_OUTPUT_DIRECTORY': None}, change=lambda w: w.write('ext.txt', r'\def\V{new output}')),
    case('taint', 'pipe-input', files={'main.tex': doc(r'\input|"cat ext.txt"', SHOW_V), 'ext.txt': r'\def\V{old output}'}, args=['-shell-escape'], change=lambda w: w.write('ext.txt', r'\def\V{new output}')),
    case('taint', 'terminal-read', files={'main.tex': doc(r'\read16 to\V', SHOW_V)}, args=['-interaction=scrollmode'], stdin='old answer\n', change=lambda w: setattr(w, 'stdin', 'new answer\n')),
    case('taint', 'error-in-preamble', why='the error count and the exit status have to survive; no checkpoint past an error', files={'main.tex': doc(r'\undefinedcommandinthepreamble', 'Old body.\n')}, change=lambda w: w.edit('main.tex', 'Old body.', 'New body.')),
    case('taint', 'fatal-then-fixed', why='the first run dies in the preamble, then the missing file appears', files={'main.tex': doc(r'\input{missing}', SHOW_V)}, change=lambda w: w.write('missing.tex', r'\def\V{now present}')),
    # ------------------------------------------------------------------ the run's own writes
    case('own-writes', 'openout-closed-file-deleted', why='journalled, and re-materialised on restore', files={'main.tex': doc(r'\newwrite\o \immediate\openout\o=note.txt \immediate\write\o{from the preamble}\immediate\closeout\o', 'Old body.\n')}, change=lambda w: (w.rm_out('note.txt'), w.edit('main.tex', 'Old body.', 'New body.'))),
    case('own-writes', 'openout-closed-file-tampered', files={'main.tex': doc(r'\newwrite\o \immediate\openout\o=note.txt \immediate\write\o{from the preamble}\immediate\closeout\o', 'Old body.\n')}, change=lambda w: (w.write_out('note.txt', 'tampered\n'), w.edit('main.tex', 'Old body.', 'New body.'))),
    case('own-writes', 'openout-open-across-checkpoint', why='a stream open at the checkpoint: its bytes so far come back, and the body appends', files={'main.tex': doc(r'\newwrite\o \immediate\openout\o=note.txt \immediate\write\o{from the preamble}', r'\immediate\write\o{from the old body}\immediate\closeout\o Body.' + '\n')}, change=lambda w: w.edit('main.tex', 'from the old body', 'from the new body')),
    case('own-writes', 'read-back-own-file', why='written and read back in the preamble, then deleted', files={'main.tex': doc(r'\newwrite\o \immediate\openout\o=gen.tex \immediate\write\o{\string\def\string\V{generated}}\immediate\closeout\o \input{gen.tex}', SHOW_V)}, visible=False, change=lambda w: w.rm_out('gen.tex')),
    case('own-writes', 'recorder-file', why='the .fls of a restored run lists what the preamble read', files={'main.tex': doc(r'\input{macros}', 'Old body.\n'), 'macros.tex': r'\def\V{x}'}, args=['-recorder'], change=lambda w: w.edit('main.tex', 'Old body.', 'New body.')),
    case('own-writes', 'pdf-objects-in-preamble', why='\\pdfobj, \\pdfcatalog, \\pdfinfo and \\pdfmapline before the checkpoint; object numbers must continue from there', files={'main.tex': doc(r'\immediate\pdfobj{(a stream)}\pdfcatalog{/Mutated \the\pdflastobj\space 0 R}\pdfinfo{/Subject (from the preamble)}\pdfmapline{+cmss10 CMSS10 <cmss10.pfb}', 'Old body.\n')}, change=lambda w: w.edit('main.tex', 'Old body.', 'New body.')),
    case('own-writes', 'shipout-in-preamble', why='the PDF has started before the checkpoint', files={'main.tex': doc(r'\setbox0=\hbox{A page from the preamble}\shipout\box0 ', 'Old body.\n')}, change=lambda w: w.edit('main.tex', 'Old body.', 'New body.')),
    case('own-writes', 'font-used-in-preamble', why='\\pdfincludechars and a loaded font state before the checkpoint', files={'main.tex': doc(r'\font\x=cmss10 \pdfincludechars\x{Q}', r'{\x Old}' + '\n')}, change=lambda w: w.edit('main.tex', r'{\x Old}', r'{\x New}')),
    # ------------------------------------------------------------------ stacked files and the position in them
    case('stack', 'begin-document-in-input', why='path and offset of every stacked file', files={'main.tex': HEAD + r'\documentclass{article}\input{pre}' + '\n' + 'Old body.\n' + r'\end{document}' + '\n', 'pre.tex': r'\def\V{x}' + '\n' + r'\begin{document}' + '\n' + 'After in pre.\n'}, change=lambda w: w.edit('main.tex', 'Old body.', 'New body.')),
    case('stack', 'tail-of-stacked-file-edited', why='only the part after the read position changes', files={'main.tex': HEAD + r'\documentclass{article}\input{pre}' + '\n' + 'Body.\n' + r'\end{document}' + '\n', 'pre.tex': r'\def\V{x}' + '\n' + r'\begin{document}' + '\n' + 'Old tail.\n'}, change=lambda w: w.edit('pre.tex', 'Old tail.', 'New tail.')),
    case('stack', 'prefix-of-stacked-file-edited', files={'main.tex': HEAD + r'\documentclass{article}\input{pre}' + '\n' + r'\V' + '\n' + r'\end{document}' + '\n', 'pre.tex': r'\def\V{old}' + '\n' + r'\begin{document}' + '\n' + 'Tail.\n'}, change=lambda w: w.edit('pre.tex', r'\def\V{old}', r'\def\V{new}')),
    case('stack', 'rest-of-begin-document-line', why='the rest of the line is already in the buffer, so the hash has to cover the whole line', files={'main.tex': ARTICLE + r'\begin{document} Old rest of line.' + '\n' + r'\end{document}' + '\n'}, change=lambda w: w.edit('main.tex', 'Old rest of line.', 'New rest of line.')),
    case('stack', 'begin-document-in-macro', why='no line of its own: \\begin{document} comes out of a macro', files={'main.tex': ARTICLE + r'\def\go{\begin{document}}' + '\n' + r'\go Old on the same line.' + '\n' + r'\end{document}' + '\n'}, change=lambda w: w.edit('main.tex', 'Old on the same line.', 'New on the same line.')),
    case('stack', 'document-redefined', why='a class wraps \\document: its first macro call is not the one the checkpoint looks for in plain LaTeX', files={'main.tex': ARTICLE + r'\let\olddocument\document\def\document{\input{late}\olddocument}' + '\n' + r'\begin{document}\V.\end{document}' + '\n', 'late.tex': r'\def\V{old}'}, change=lambda w: w.edit('late.tex', 'old', 'new')),
    case('stack', 'document-redefined-body-edit', why='the same, with what is read by the wrapper unchanged: now that file is in the log, and the restore comes from after it', files={'main.tex': ARTICLE + r'\let\olddocument\document\def\document{\input{late}\olddocument}' + '\n' + r'\begin{document}' + '\n' + r'Old \V.\end{document}' + '\n', 'late.tex': r'\def\V{same}'}, change=lambda w: w.edit('main.tex', 'Old', 'New')),
    case('stack', 'document-long-with-a-parameter', why='\\document armed while macro_call scans its argument, not at the big switch: its \\long has to hold, or \\par is an error', files={'main.tex': ARTICLE + r'\let\realdocument\document\long\def\document#1{\def\W{#1}}\expandafter\relax\document{from an\par argument}\let\document\realdocument' + '\n' + r'\begin{document}Old \W.\end{document}' + '\n'}, change=lambda w: w.edit('main.tex', 'Old', 'New')),
    case('stack', 'scantokens-begin-document', why='in_open > 1 from a pseudo-file: not a safe point', files={'main.tex': ARTICLE + r'\def\V{old}\scantokens{\begin{document}}' + '\n' + r'\V' + '\n' + r'\end{document}' + '\n'}, change=lambda w: w.edit('main.tex', r'\def\V{old}', r'\def\V{new}')),
    case('stack', 'file-read-in-before-hook', why='the checkpoint is taken where \\document reads the .aux file, so what its first hook reads is in the log', files={'main.tex': doc(r'\AddToHook{begindocument/before}{\input{hooked}}', SHOW_V), 'hooked.tex': r'\def\V{old}'}, change=lambda w: w.write('hooked.tex', r'\def\V{new}')),
    case('stack', 'clock-read-in-before-hook', why='a read of the clock between \\document and its \\@input', files={'main.tex': doc(r'\AddToHook{begindocument/before}{\xdef\V{\the\time}}', SHOW_V)}, change=lambda w: w.clock(60)),
    case('stack', 'at-input-in-before-hook', why='a hook that uses \\@input itself: the moment comes there, and the rest of the hook and \\document run in every run', files={'main.tex': doc(r'\makeatletter\AddToHook{begindocument/before}{\@input{hooked}\xdef\W{\V\space and more}}\makeatother', r'\W' + '\n'), 'hooked.tex': r'\def\V{old}'}, change=lambda w: w.write('hooked.tex', r'\def\V{new}')),
    case('stack', 'at-input-wrapped-in-preamble', why='a macro of its own in place of \\@input: it is what \\document fetches', files={'main.tex': doc(r'\makeatletter\let\oldinput\@input\def\@input#1{\def\V{wrapped}\oldinput{#1}}\makeatother', 'Old body. ' + SHOW_V)}, change=lambda w: w.edit('main.tex', 'Old body.', 'New body.')),
    case('stack', 'at-input-unmade-in-before-hook', why='\\@input is no macro by the time \\document comes to it: no checkpoint', files={'main.tex': doc(r'\makeatletter\AddToHook{begindocument/before}{\let\@input\@gobble}\makeatother', 'Old body.\n')}, change=lambda w: w.edit('main.tex', 'Old body.', 'New body.')),
    case('stack', 'at-input-relax-in-before-hook', why='not even a macro: the trigger cannot come back', files={'main.tex': doc(r'\makeatletter\AddToHook{begindocument/before}{\let\@input\relax}\makeatother', 'Old body.\n')}, change=lambda w: w.edit('main.tex', 'Old body.', 'New body.')),
    case('stack', 'tracingassigns-at-at-input', why='with \\tracingassigns on \\@input is armed all the same, since eq_define sees through the bit', files={'main.tex': doc(r'\tracingassigns=1 ', 'Old body.\n')}, change=lambda w: w.edit('main.tex', 'Old body.', 'New body.')),
    case('stack', 'hook-writes-the-log', why='a message from the hook is in the journal that a restore plays back', files={'main.tex': doc(r'\AddToHook{begindocument/before}{\typeout{Said by the hook}}', 'Old body.\n')}, change=lambda w: w.edit('main.tex', 'Old body.', 'New body.')),
    case('stack', 'crlf-line-endings', why='the offset counts the bytes on disk', files={'main.tex': ARTICLE.replace('\n', '\r\n') + r'\begin{document}' + '\r\n' + 'Old body.\r\n' + r'\end{document}' + '\r\n'}, change=lambda w: w.edit('main.tex', 'Old body.', 'New body.')),
    case('stack', 'no-final-newline', files={'main.tex': ARTICLE + r'\input{pre}' + '\n' + 'Old body.\n' + r'\end{document}' + '\n', 'pre.tex': r'\begin{document}'}, change=lambda w: w.edit('main.tex', 'Old body.', 'New body.')),
    case('stack', 'endinput-in-preamble', why='\\endinput leaves the rest of the file unread; an edit after it changes nothing', visible=False, files={'main.tex': ARTICLE + r'\input{pre}' + '\n' + r'\begin{document}Body.\end{document}' + '\n', 'pre.tex': r'\def\V{x}\endinput' + '\n' + 'unread text\n'}, change=lambda w: w.edit('pre.tex', 'unread text', 'other text')),
    # ------------------------------------------------------------------ a restore that has to bring back what the preamble set up, with the body edited
    case('restored', 'png-from-the-preamble', why='the image table is in the image, what the C keeps behind it is read again', files={'main.tex': doc(r'\pdfximage{img.png}\setbox0=\hbox{\pdfrefximage\pdflastximage}', r'Old body. \box0' + '\n'), 'img.png': tiny_png(8, 8, 200)}, change=lambda w: w.edit('main.tex', 'Old body.', 'New body.')),
    case('restored', 'jpeg-from-the-preamble', files={'main.tex': doc(r'\pdfximage{img.jpg}\setbox0=\hbox{\pdfrefximage\pdflastximage}', r'Old body. \box0' + '\n'), 'img.jpg': JPEG_BYTES}, change=lambda w: w.edit('main.tex', 'Old body.', 'New body.')),
    case('restored', 'pdf-from-the-preamble', files={'main.tex': doc(r'\pdfximage{img.pdf}\setbox0=\hbox{\pdfrefximage\pdflastximage}', r'Old body. \box0' + '\n'), 'img.pdf': tiny_pdf(40)}, change=lambda w: w.edit('main.tex', 'Old body.', 'New body.')),
    case('restored', 'pdf-from-the-preamble-changed', files={'main.tex': doc(r'\pdfximage{img.pdf}\setbox0=\hbox{\pdfrefximage\pdflastximage}', r'Body. \box0' + '\n'), 'img.pdf': tiny_pdf(40)}, change=lambda w: w.write('img.pdf', tiny_pdf(50))),
    case('restored', 'tounicode-from-the-preamble', why='\\pdfglyphtounicode is kept as a log in the image and replayed into a new Host', files={'main.tex': doc(r'\pdfgentounicode=1 \pdfglyphtounicode{A}{0041}\pdfmapline{+cmss10 CMSS10 <cmss10.pfb}\font\x=cmss10', r'{\x Old A}' + '\n')}, change=lambda w: w.edit('main.tex', 'Old A', 'New A')),
    case('restored', 'big-streams-from-the-preamble', why='over 128 KiB of \\pdfobj streams starts the deflate workers: what they hold has to be in the checkpoint, and in the offsets it holds', files={'main.tex': doc(r'\pdfcompresslevel=6 \immediate\pdfobj stream file {big.dat}\edef\A{\the\pdflastobj}\immediate\pdfobj stream file {big.dat}\immediate\pdfobj stream file {big.dat}\pdfcatalog{/Big \A\space 0 R}', 'Old body.\n'), 'big.dat': bytes((i * i + (i >> 7)) & 255 for i in range(300000))}, change=lambda w: w.edit('main.tex', 'Old body.', 'New body.')),
    case('restored', 'dvi-output', why='no PDF at all', files={'main.tex': doc(r'\def\V{same}', 'Old body.\n')}, args=['-output-format=dvi'], job='main', change=lambda w: w.edit('main.tex', 'Old body.', 'New body.')),
    case('restored', 'draftmode', visible=False, why='no PDF, and a body edit that leaves the log as it was', files={'main.tex': doc(r'\def\V{same}', 'Old body.\n')}, args=['-draftmode'], change=lambda w: w.edit('main.tex', 'Old body.', 'New body.')),
    case('restored', 'recorder-after-a-failed-validation', why='a checkpoint that does not hold has made lookups the cold run then makes again: the .fls must not show it', files={'main.tex': doc(r'\font\x=cmss10 \input{macros}', SHOW_V), 'macros.tex': r'\def\V{old}'}, args=['-recorder'], change=lambda w: w.edit('macros.tex', 'old', 'new')),
    case('restored', 'group-open-at-document', why='\\begin{document} one group down, closed in the body', files={'main.tex': ARTICLE + r'\begingroup\def\V{in a group}' + '\n' + r'\begin{document}\endgroup Old \V.' + '\n' + r'\end{document}' + '\n'}, change=lambda w: w.edit('main.tex', 'Old', 'New')),
    # ------------------------------------------------------------------ reads of what the whitelisted sites made, by ways other than the plain one
    case('volatile', 'c-sys-let-in-preamble', why='a copy by \\let carries the value away from the constant that is patched', files={'main.tex': doc(r'\makeatletter\ExplSyntaxOn \cs_gset_eq:NN \Vm \c_sys_minute_int \ExplSyntaxOff', r'\the\Vm' + '\n')}, change=lambda w: w.clock(60)),
    case('volatile', 'c-sys-noexpand-in-preamble', why='what \\noexpand brings back goes round the test on outer, but its own fetch of the constant does not, and that counts as a read', files={'main.tex': doc(r'\ExplSyntaxOn \setbox0\hbox{$\expandafter\relax\noexpand\c_sys_minute_int$} \xdef\V{\the\wd0} \ExplSyntaxOff', SHOW_V)}, change=lambda w: w.clock(120)),
    case('volatile', 'c-sys-timestamp-edef', files={'main.tex': doc(r'\ExplSyntaxOn \cs_gset:Npx \V { \c_sys_timestamp_str } \ExplSyntaxOff', SHOW_V)}, change=lambda w: w.clock(60)),
    case('volatile', 'c-sys-year-read-next-day', why='a date read through the constant: good for the same day', files={'main.tex': doc(r'\ExplSyntaxOn \cs_gset:Npx \V { \int_use:N \c_sys_day_int } \ExplSyntaxOff', SHOW_V)}, change=lambda w: w.clock(86400)),
    case('volatile', 'c-sys-meaning-in-preamble', files={'main.tex': doc(r'\ExplSyntaxOn \cs_gset:Npx \V { \cs_meaning:N \c_sys_minute_int } \ExplSyntaxOff', SHOW_V)}, change=lambda w: w.clock(60)),
    case('volatile', 'c-sys-ifx-in-preamble', files={'main.tex': doc(r'\ExplSyntaxOn \tex_mathchardef:D \Zm = 0 ~ \tex_ifx:D \c_sys_minute_int \Zm \tl_gset:Nn \V {zero} \tex_else:D \tl_gset:Nn \V {other} \tex_fi:D \ExplSyntaxOff', SHOW_V)}, change=lambda w: w.clock(30 * 60 + 47 * 60)),
    case('volatile', 'pgfmath-seed-used-in-preamble', why='a random number drawn in the preamble comes from the seed of that run', files={'main.tex': doc(r'\usepackage{pgfmath}\pgfmathrandominteger\r{1}{1000000}\xdef\V{\r}', SHOW_V)}, change=lambda w: w.clock(60)),
    case('volatile', 'pgfmath-seed-read-in-preamble', why='no draw, so the seed is unchanged at the checkpoint, and only the trap can tell', files={'main.tex': doc(r'\usepackage{pgfmath}\makeatletter\edef\V{\pgfmath@rnd@z}\makeatother', SHOW_V)}, change=lambda w: w.clock(60)),
    case('volatile', 'pgfmath-seed-pinned', why='the user set the seed in the preamble: a patch would undo that', visible=False, files={'main.tex': doc(r'\usepackage{pgfmath}\pgfmathsetseed{42}', r'\pgfmathrandominteger\r{1}{1000000}\r' + '\n')}, change=lambda w: w.clock(60)),
    case('volatile', 'pgfmath-seed-digits-change', why='\\time*\\year passes a power of ten: the patched macro needs a body of another size', files={'main.tex': doc(r'\usepackage{pgfmath}', r'\pgfmathrandominteger\r{1}{1000000}\r' + '\n')}, env={'SOURCE_DATE_EPOCH': '1699920000'}, change=lambda w: w.clock(6 * 3600)),
    case('volatile', 'year-assigned-in-a-group', why='assigned and restored by the group: nothing at the checkpoint shows it was', files={'main.tex': doc(r'\begingroup\year=2000 \endgroup', r'\the\year' + '\n')}, change=lambda w: w.clock(86400 * 400)),
    case('volatile', 'year-assigned-its-own-value', why='nothing shows an assignment that left the value as it was, and next year it matters', files={'main.tex': doc(r'\year=2023 ', r'\the\year' + '\n')}, visible=False, change=lambda w: w.clock(86400 * 400)),
    case('volatile', 'everyjob-copies-the-clock', why='a format of its own: the reads in \\everyjob are not expl3\'s alone', files={'main.tex': doc('', r'\STAMP' + '\n')}, custom_format='first', format_extra=r'\everyjob{\xdef\STAMP{\the\time}}', change=lambda w: w.clock(60)),
    # ------------------------------------------------------------------ the cache itself: a damaged or foreign snapshot is a miss, never a wrong answer
    case('cache', 'corrupted-bytes', visible=False, why='every cache file gets bytes flipped', files={'main.tex': doc(r'\def\V{x}', SHOW_V)}, change=lambda w: w.damage_cache('flip')),
    case('cache', 'truncated', visible=False, why='a crash during a publish that was not atomic, or a full disk', files={'main.tex': doc(r'\def\V{x}', SHOW_V)}, change=lambda w: w.damage_cache('truncate')),
    case('cache', 'emptied', visible=False, files={'main.tex': doc(r'\def\V{x}', SHOW_V)}, change=lambda w: w.damage_cache('empty')),
    case('cache', 'read-only', visible=False, why='cannot publish: the run must still be right', files={'main.tex': doc(r'\def\V{x}', SHOW_V)}, change=lambda w: w.damage_cache('readonly')),
    case('cache', 'same-preamble-other-project', why='two projects with the same main file: their \\input files differ', files={'main.tex': doc(r'\input{macros}', SHOW_V), 'macros.tex': r'\def\V{project A}', 'b/main.tex': doc(r'\input{macros}', SHOW_V), 'b/macros.tex': r'\def\V{project B}'}, change=lambda w: setattr(w, 'cwd', w.path('b'))),
]



# ==================================================================== The adversarial review: what the groups above leave out
def _body_edit(w):
    w.edit('main.tex', 'Old body.', 'New body.')


def tiny_pdf_two_pages(minor=4):
    """Two pages that share a resource object, which an inclusion copies once per document."""
    stream = b'0 0 m 10 10 l S'
    objs = [b'<< /Type /Catalog /Pages 2 0 R >>', b'<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>',
            b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 40 20] /Resources 5 0 R /Contents 6 0 R >>',
            b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 50 20] /Resources 5 0 R /Contents 6 0 R >>',
            b'<< /ExtGState << /G0 7 0 R >> >>', b'<< /Length %d >>\nstream\n%s\nendstream' % (len(stream), stream), b'<< /LW 2 >>']
    out = b'%%PDF-1.%d\n' % minor
    offsets = []
    for k, o in enumerate(objs):
        offsets.append(len(out))
        out += b'%d 0 obj\n' % (k + 1) + o + b'\nendobj\n'
    x = len(out)
    out += b'xref\n0 %d\n0000000000 65535 f \n' % (len(objs) + 1) + b''.join(b'%010d 00000 n \n' % n for n in offsets)
    out += b'trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n' % (len(objs) + 1, x)
    return out


def _zoneinfo(name):
    with open(os.path.join('/usr/share/zoneinfo', name), 'rb') as f:
        return f.read()


LSR_HEAD = '% ls-R -- filename database for kpathsea; do not change this line.\n'
TIMEZONE_TAIL = r'\ExplSyntaxOn \str_range:Nnn \c_sys_timestamp_str {17} {23} \ExplSyntaxOff' + '\n'
NO_DATES = r'\pdfinfoomitdate=1 '
GENERATED = r'\newwrite\o \immediate\openout\o=gen.txt \immediate\write\o{line one}\immediate\write\o{line two}\immediate\closeout\o \newread\r \openin\r=gen.txt \read\r to\A'

CASES += [
    # ------------------------------------------------------------------ a file open at the checkpoint: what its prefix does not say
    case('read', 'last-line-of-open-file-grows', why='the last line had no terminator, so the prefix ends inside what is now a longer line', files={'main.tex': doc(r'\newread\r \openin\r=data.txt \read\r to\A \read\r to\V', SHOW_V), 'data.txt': 'first\nsecond'}, change=lambda w: w.write('data.txt', 'first\nsecond and more\n')),
    case('read', 'open-file-cr-gets-its-lf', why='a CR at the end of the file: with an LF after it the next line starts one byte later', files={'main.tex': doc(r'\newread\r \openin\r=data.txt \read\r to\A', r'\read\r to\V Value: \V.' + '\n'), 'data.txt': 'first\r'}, change=lambda w: w.write('data.txt', 'first\r\nsecond\n')),
    case('stack', 'last-line-of-stacked-file-grows', why='the file holding \\begin{document} ends there with no terminator, and then the line grows: the rest of it is in the buffer in a cold run', files={'main.tex': ARTICLE + r'\makeatletter\let\primitiveinput\@@input\makeatother\primitiveinput pre ' + '\n' + 'Body.\n' + r'\end{document}' + '\n', 'pre.tex': r'\begin{document}'}, change=lambda w: w.write('pre.tex', r'\begin{document}Line \the\inputlineno.' + '\n')),
    case('own-writes', 'own-file-open-at-checkpoint-rewritten-by-the-body', visible=False, why='written, closed and opened in the preamble; the body of the last run wrote the file again.  A cold run writes it first, so it reads its own bytes', files={'main.tex': doc(GENERATED, r'\read\r to\V Value: \V. \immediate\openout\o=gen.txt \immediate\write\o{changed one, longer}\immediate\write\o{changed two}\immediate\closeout\o' + '\n')}),
    case('own-writes', 'own-file-open-at-checkpoint-tampered', visible=False, why='the same, changed from outside', files={'main.tex': doc(GENERATED, r'\read\r to\V Value: \V.' + '\n')}, change=lambda w: w.write_out('gen.txt', 'line one\nsomething else\n')),
    case('own-writes', 'own-input-open-at-checkpoint-tampered', why='\\begin{document} in a file the preamble itself wrote: its rest comes from what the run wrote', files={'main.tex': ARTICLE + r'\newwrite\o \immediate\openout\o=gen.tex \immediate\write\o{\string\begin{document}}\immediate\write\o{From the generated file.}\immediate\closeout\o \input{gen.tex}' + '\n' + 'Old body.\n' + r'\end{document}' + '\n'}, change=lambda w: (w.write_out('gen.tex', r'\begin{document}' + '\nTampered.\n'), _body_edit(w))),
    case('own-writes', 'own-file-deleted-while-open-at-checkpoint', visible=False, why='what the run wrote and has open is gone: it writes it again', files={'main.tex': doc(GENERATED, r'\read\r to\V Value: \V.' + '\n')}, change=lambda w: w.rm_out('gen.txt')),
    # ------------------------------------------------------------------ reads
    case('read', 'symlink-retargeted', why='the same name, the same stat of the link if it were lstat: the bytes decide', files={'main.tex': doc(r'\input{link}', SHOW_V), 'a.tex': r'\def\V{from a}', 'b.tex': r'\def\V{from b}'}, setup=lambda w: os.symlink('a.tex', w.path('link.tex')), change=lambda w: (os.unlink(w.path('link.tex')), os.symlink('b.tex', w.path('link.tex')))),
    case('read', 'becomes-readable', why='kpathsea looked and could not read it: \\IfFileExists said no', files={'main.tex': doc(r'\IfFileExists{opt.tex}{\input{opt}}{\def\V{absent}}', SHOW_V), 'opt.tex': r'\def\V{present}'}, setup=lambda w: os.chmod(w.path('opt.tex'), 0o000), change=lambda w: os.chmod(w.path('opt.tex'), 0o644)),
    case('read', 'becomes-unreadable', files={'main.tex': doc(r'\IfFileExists{opt.tex}{\input{opt}}{\def\V{absent}}', SHOW_V), 'opt.tex': r'\def\V{present}'}, change=lambda w: os.chmod(w.path('opt.tex'), 0o000)),
    case('read', 'directory-then-file', why='a directory of that name reads as nothing', files={'main.tex': doc(r'\IfFileExists{d.tex}{\input{d}}{\def\V{absent}}', SHOW_V)}, setup=lambda w: os.mkdir(w.path('d.tex')), change=lambda w: (os.rmdir(w.path('d.tex')), w.write('d.tex', r'\def\V{now a file}'))),
    case('read', 'file-then-directory', files={'main.tex': doc(r'\IfFileExists{d.tex}{\input{d}}{\def\V{absent}}', SHOW_V), 'd.tex': r'\def\V{a file}'}, change=lambda w: (w.rm('d.tex'), os.mkdir(w.path('d.tex')))),
    case('read', 'mtime-backwards-same-bytes', visible=False, must=True, files={'main.tex': doc(r'\input{macros}', SHOW_V + 'Old body.\n'), 'macros.tex': r'\def\V{aaa}'}, change=lambda w: (w.touch('macros.tex', delta=-86400), _body_edit(w))),
    case('read', 'mtime-backwards-other-bytes', files={'main.tex': doc(r'\input{macros}', SHOW_V), 'macros.tex': r'\def\V{aaa}'}, change=lambda w: (w.write('macros.tex', r'\def\V{bbb}'), w.touch('macros.tex', delta=-86400))),
    case('read', 'tds-mtime-backwards-other-bytes', files={'main.tex': doc(r'\usepackage{pk}', SHOW_V), 'tree/ls-R': LSR_HEAD + './:\ntex\n\n./tex:\nlatex\n\n./tex/latex:\npk.sty\n', 'tree/tex/latex/pk.sty': r'\def\V{old}'}, env={'TEXMFHOME': '@HERE@/tree', 'TEXMFDBS': '@HERE@/tree:'}, change=lambda w: (w.edit('tree/tex/latex/pk.sty', 'old', 'new'), w.touch('tree/tex/latex/pk.sty', delta=-86400))),
    case('read', 'tds-touched-same-bytes', visible=False, must=True, why='a new stat, the same bytes: no miss', files={'main.tex': doc(r'\usepackage{pk}', SHOW_V), 'tree/ls-R': LSR_HEAD + './:\ntex\n\n./tex:\nlatex\n\n./tex/latex:\npk.sty\n', 'tree/tex/latex/pk.sty': r'\def\V{old}'}, env={'TEXMFHOME': '@HERE@/tree', 'TEXMFDBS': '@HERE@/tree:'}, change=lambda w: w.touch('tree/tex/latex/pk.sty')),
    case('read', 'main-file-removed', files={'main.tex': doc('', 'Body.\n')}, change=lambda w: w.rm('main.tex')),
    case('read', 'first-line-one-byte', why='the first line of the file, by one byte', files={'main.tex': doc(r'\def\V{same}', SHOW_V)}, change=lambda w: w.edit('main.tex', r'\pdfcompresslevel=0', r'\pdfcompresslevel=1')),
    case('read', 'preamble-grows-at-the-end', files={'main.tex': doc(r'\def\V{same}', SHOW_V)}, change=lambda w: w.edit('main.tex', r'\begin{document}', r'\def\V{added}' + '\n' + r'\begin{document}')),
    case('read', 'body-empty-after-restore', must=True, why='no page at all: "No pages of output", and no PDF', files={'main.tex': doc(r'\def\V{same}', 'Old body.\n')}, change=lambda w: w.edit('main.tex', 'Old body.', '')),
    # ------------------------------------------------------------------ lookups in trees
    case('lookup', 'texmfhome-shadows-a-class', why='no ls-R there, so the disk is searched', files={'main.tex': doc('', 'Body.\n'), 'tree/x': 'x'}, env={'TEXMFHOME': '@HERE@/tree'}, change=lambda w: w.write('tree/tex/latex/article.cls', r'\NeedsTeXFormat{LaTeX2e}\ProvidesClass{article}\LoadClass{report}\AtBeginDocument{Local class.}')),
    case('lookup', 'new-ls-r-points-elsewhere', why='a new ls-R: its answer comes before the search of the disk', files={'main.tex': doc(r'\usepackage{pk}', SHOW_V), 'tree/tex/latex/pk.sty': r'\def\V{from the top}', 'tree/tex/latex/b/pk.sty': r'\def\V{from b}'}, env={'TEXMFHOME': '@HERE@/tree', 'TEXMFDBS': '@HERE@/tree:'}, change=lambda w: w.write('tree/ls-R', LSR_HEAD + './tex/latex/b:\npk.sty\n')),
    case('lookup', 'pdftex-map-shadowed', why='a font used in the preamble brought the default map file in: a new one in the current directory comes first', files={'main.tex': doc(r'\font\x=cmss10 \pdfincludechars\x{Q}\pdfmapline{+cmr12 CMR12 <cmr12.pfb}', r'{\x Old}' + '\n')}, change=lambda w: w.write('pdftex.map', 'cmss10 CMSS12 <cmss12.pfb\n')),
    case('lookup', 'map-from-texfontmaps', files={'main.tex': doc(r'\pdfmapfile{+local.map}\font\x=cmss10 \pdfincludechars\x{Q}', r'{\x Old}' + '\n'), 'maps/a/local.map': 'cmss10 CMSS10 <cmss10.pfb\n', 'maps/b/local.map': 'cmss10 CMSS12 <cmss12.pfb\n'}, env={'TEXFONTMAPS': '@HERE@/maps/a:@HERE@/maps/b:'}, change=lambda w: w.rm('maps/a/local.map')),
    # ------------------------------------------------------------------ stat-like primitives through other doors
    case('stat', 'pdfmdfivesum-appears', files={'main.tex': doc(r'\edef\V{[\pdfmdfivesum file{data.bin}]}', SHOW_V)}, change=lambda w: w.write('data.bin', 'xxxx')),
    case('stat', 'pdffiledump-removed', files={'main.tex': doc(r'\edef\V{[\pdffiledump length 2{data.bin}]}', SHOW_V), 'data.bin': 'aabb'}, change=lambda w: w.rm('data.bin')),
    case('stat', 'pdfstrcmp-of-the-sum', why='what depends on the read through another primitive', files={'main.tex': doc(r'\edef\V{\pdfstrcmp{\pdfmdfivesum file{data.bin}}{\pdfmdfivesum{aaaa}}}', SHOW_V), 'data.bin': 'aaaa'}, change=lambda w: w.write('data.bin', 'bbbb', keep_stat=True)),
    case('stat', 'pdffilemoddate-backwards', files={'main.tex': doc(r'\edef\V{\pdffilemoddate{data.bin}}', SHOW_V), 'data.bin': 'xxxx'}, env={'FORCE_SOURCE_DATE': None}, change=lambda w: w.touch('data.bin', delta=-86400)),
    case('stat', 'pdffilesize-same-size', visible=False, must=True, why='it looks at the size and no more', files={'main.tex': doc(r'\edef\V{\pdffilesize{data.bin}}', SHOW_V), 'data.bin': 'aaaa'}, change=lambda w: w.write('data.bin', 'bbbb')),
    case('stat', 'found-in-output-directory-first', why='\\pdffilesize looks in the output directory first', files={'main.tex': doc(r'\edef\V{\pdffilesize{data.bin}}', SHOW_V), 'data.bin': 'aaaa'}, change=lambda w: w.write_out('data.bin', 'aaaaaaaa')),
    case('stat', 'sum-found-in-output-directory-first', files={'main.tex': doc(r'\edef\V{\pdfmdfivesum file{data.bin}}', SHOW_V), 'data.bin': 'aaaa'}, change=lambda w: w.write_out('data.bin', 'bbbb')),
    case('stat', 'image-found-in-output-directory-first', files={'main.tex': doc(r'\pdfximage{img.png}\setbox0=\hbox{\pdfrefximage\pdflastximage}\edef\W{\the\wd0}', r'\W\box0' + '\n'), 'img.png': tiny_png(8, 8, 200)}, change=lambda w: w.write_out('img.png', tiny_png(16, 8, 200))),
    case('stat', 'size-of-own-closed-file', visible=False, must=True, why='the answer is the run\'s own', files={'main.tex': doc(r'\newwrite\o \immediate\openout\o=note.txt \immediate\write\o{some text}\immediate\closeout\o \edef\V{\pdffilesize{note.txt}}', SHOW_V)}, change=lambda w: w.write_out('note.txt', 'x')),
    # ------------------------------------------------------------------ the clock, by ways that are not a plain read
    case('volatile', 'advance-day', why='\\advance reads the parameter without going through scan_something_internal', files={'main.tex': doc(r'\advance\day by 1 ', r'\the\day' + '\n')}, change=lambda w: w.clock(86400)),
    case('volatile', 'advance-year-in-the-preamble', files={'main.tex': doc(r'\advance\year by -1 ', r'\the\year' + '\n')}, change=lambda w: w.clock(86400 * 400)),
    case('volatile', 'multiply-time', files={'main.tex': doc(r'\multiply\time by 2 ', r'\the\time' + '\n')}, change=lambda w: w.clock(60)),
    case('volatile', 'divide-month', files={'main.tex': doc(r'\divide\month by 2 ', r'\the\month' + '\n')}, change=lambda w: w.clock(86400 * 62)),
    case('volatile', 'traced-assignment-of-year', why='\\tracingassigns writes the old value in the log', files={'main.tex': doc(r'\tracingassigns=1 \year=2000 \tracingassigns=0 ', 'Body.\n')}, change=lambda w: w.clock(86400 * 400)),
    case('volatile', 'traced-restore-of-time', why='\\tracingrestores writes the value that comes back in the log', files={'main.tex': doc(r'\tracingrestores=1 \begingroup\time=5 \endgroup \tracingrestores=0 ', 'Body.\n')}, change=lambda w: w.clock(60)),
    case('volatile', 'traced-pgfmath-seed', why='\\tracingassigns writes what the patched macro held', files={'main.tex': doc(r'\tracingassigns=1 \usepackage{pgfmath}\tracingassigns=0 ', 'Body.\n')}, change=lambda w: w.clock(60)),
    case('volatile', 'traced-pgfmath-macros', why='\\tracingmacros during the seed', files={'main.tex': doc(r'\tracingmacros=2 \tracingcommands=2 \usepackage{pgfmath}\tracingmacros=0 \tracingcommands=0 ', 'Body.\n')}, change=lambda w: w.clock(60)),
    case('volatile', 'everyjob-traced-from-the-format', why='a format with \\tracingassigns on: expl3\'s constants are in the log', files={'main.tex': doc('', 'Body.\n')}, custom_format='first', format_extra=r'\tracingassigns=1 ', change=lambda w: w.clock(60)),
    case('volatile', 'message-of-the-day', files={'main.tex': doc(r'\message{\the\day}', 'Body.\n')}, change=lambda w: w.clock(86400)),
    case('volatile', 'numexpr-of-time', files={'main.tex': doc(r'\edef\V{\the\numexpr\time+1\relax}', SHOW_V)}, change=lambda w: w.clock(60)),
    case('volatile', 'ifnum-on-the-year', files={'main.tex': doc(r'\ifnum\year>2023 \def\V{later}\else\def\V{now}\fi', SHOW_V)}, change=lambda w: w.clock(86400 * 400)),
    case('volatile', 'day-through-a-let', files={'main.tex': doc(r'\let\today@day\day \edef\V{\the\today@day}'.replace('@', 'X'), SHOW_V)}, change=lambda w: w.clock(86400)),
    case('volatile', 'zone-changes-length-of-the-timestamp', why='Z and +09\'00\' differ in length: \\c_sys_timestamp_str has to take the new one, as a change to or from summer time does', files={'main.tex': doc(NO_DATES, TIMEZONE_TAIL), 'tz': _zoneinfo('UTC')}, env={'TZ': ':@HERE@/tz', 'SOURCE_DATE_EPOCH': None, 'FORCE_SOURCE_DATE': None}, change=lambda w: w.write('tz', _zoneinfo('Asia/Tokyo'))),
    case('volatile', 'force-source-date-invalid', visible=False, why='its warning comes once in a run, however many times the clock is asked', files={'main.tex': doc(NO_DATES, 'Old body.\n')}, env={'FORCE_SOURCE_DATE': '2'}, change=_body_edit),
    case('volatile', 'source-date-epoch-invalid', why='a fatal error on stderr, and exit 1', files={'main.tex': doc('', 'Body.\n')}, change=lambda w: w.env.__setitem__('SOURCE_DATE_EPOCH', 'not a number')),
    case('volatile', 'source-date-epoch-not-forced', why='only /CreationDate moves with it', files={'main.tex': doc(r'\pdfinfoomitdate=0 ', r'Body.' + '\n')}, env={'FORCE_SOURCE_DATE': None}, change=lambda w: w.clock(3600)),
    case('volatile', 'source-date-epoch-appears', why='the key has whether it is set', files={'main.tex': doc('', r'\edef\V{\pdfcreationdate}\V' + '\n')}, env={'FORCE_SOURCE_DATE': None, 'SOURCE_DATE_EPOCH': None}, change=lambda w: w.env.__setitem__('SOURCE_DATE_EPOCH', str(EPOCH))),
    case('volatile', 'seed-pinned-then-drawn-in-the-body', must=True, why='a pinned generator comes back with the image', files={'main.tex': doc(r'\pdfsetrandomseed 42 ', 'Old body. ' + r'\pdfuniformdeviate 1000' + '\n')}, change=_body_edit),
    case('volatile', 'normal-deviate-after-pinned-seed', files={'main.tex': doc(r'\pdfsetrandomseed 42 \edef\V{\pdfnormaldeviate}', SHOW_V + r'\pdfnormaldeviate' + '\n')}, change=lambda w: w.edit('main.tex', r'seed 42', r'seed 43')),
    case('volatile', 'pdfrandomseed-read', visible=False, no_restore=True, files={'main.tex': doc(r'\edef\V{\the\pdfrandomseed}', 'Old body.\n')}, change=_body_edit),
    case('volatile', 'uniformdeviate-read', visible=False, no_restore=True, files={'main.tex': doc(r'\edef\V{\pdfuniformdeviate 10}', 'Old body.\n')}, change=_body_edit),
    case('volatile', 'normaldeviate-read', visible=False, no_restore=True, files={'main.tex': doc(r'\edef\V{\pdfnormaldeviate}', 'Old body.\n')}, change=_body_edit),
    case('volatile', 'elapsedtime-read', visible=False, no_restore=True, files={'main.tex': doc(r'\pdfresettimer \edef\V{\the\pdfelapsedtime}', 'Old body.\n')}, change=_body_edit),
    case('volatile', 'pdfresettimer-only', visible=False, files={'main.tex': doc(r'\pdfresettimer', r'\ifnum\pdfelapsedtime<1000000000 fine\fi Old body.' + '\n')}, change=_body_edit),
    # ------------------------------------------------------------------ configuration, through the doors kpathsea has
    case('config', 'openin-any', visible=False, why='openin_any is in the key, though the oracle reads this name under either value', files={'main.tex': doc(r'\IfFileExists{../up.tex}{\def\V{readable}}{\def\V{refused}}', SHOW_V), '../up.tex': 'x'}, env={'openin_any': 'a'}, change=lambda w: w.env.__setitem__('openin_any', 'p')),
    case('config', 'openout-any', why='openout_any decides whether a name is written at all', files={'main.tex': doc(r'\newwrite\o \immediate\openout\o=.hidden.txt \immediate\write\o{text}\immediate\closeout\o', 'Body.\n')}, env={'openout_any': 'a'}, change=lambda w: w.env.__setitem__('openout_any', 'p')),
    case('config', 'texmfoutput-fallback-goes-away', why='the output directory could not be written, so everything went to TEXMFOUTPUT.  Whether it can be written is an observation', files={'main.tex': doc(r'\def\V{same}', SHOW_V), 'other/x': 'x'}, env={'TEXMFOUTPUT': '@HERE@/other'}, setup=lambda w: os.chmod(w.out, 0o555), change=lambda w: os.chmod(w.out, 0o755)),
    case('config', 'texinputs-progname-suffix', why='TEXINPUTS_pdflatex is no name the key knows; the expanded spec is compared', files={'main.tex': doc(r'\usepackage{pk}', SHOW_V), 'a/pk.sty': r'\def\V{package a}', 'b/pk.sty': r'\def\V{package b}'}, env={'TEXINPUTS_pdflatex': 'a:'}, change=lambda w: w.env.__setitem__('TEXINPUTS_pdflatex', 'b:')),
    case('config', 'texmfdbs-changes', why='another list of databases, with no other variable changed', files={'main.tex': doc(r'\usepackage{pk}', SHOW_V), 'tree/tex/latex/pk.sty': r'\def\V{from the top}', 'tree/tex/latex/b/pk.sty': r'\def\V{from b}', 'tree/ls-R': LSR_HEAD + './tex/latex/b:\npk.sty\n'}, env={'TEXMFHOME': '@HERE@/tree'}, change=lambda w: w.env.__setitem__('TEXMFDBS', w.path('tree') + ':')),
    case('config', 'tfmfonts-spec', why='a font search path', files={'main.tex': doc(r'\font\x=cmss10 \setbox0=\hbox{\x AV}\edef\V{\the\wd0}', SHOW_V), 'tfm/x': 'x'}, env={'TFMFONTS': '@HERE@/tfm:'}, change=lambda w: w.write('tfm/cmss10.tfm', mutated_tfm())),
    case('config', 'kpse-dot', why='what `.` is in a search path', files={'main.tex': doc(r'\input{macros}', SHOW_V), 'macros.tex': r'\def\V{here}', 'b/macros.tex': r'\def\V{in b}'}, change=lambda w: w.env.__setitem__('KPSE_DOT', w.path('b'))),
    case('config', 'cnf-line', why='-cnf-line sets a variable as texmf.cnf would', files={'main.tex': doc(r'\usepackage{pk}', SHOW_V), 'a/pk.sty': r'\def\V{package a}', 'b/pk.sty': r'\def\V{package b}'}, args=['-cnf-line=TEXINPUTS=a:'], change=lambda w: w.args.__setitem__(0, '-cnf-line=TEXINPUTS=b:')),
    case('config', 'texmf-cnf-appears', why='a cnf file that was not there', files={'main.tex': doc(r'\usepackage{pk}', SHOW_V), 'a/pk.sty': r'\def\V{package a}', 'cnf/x': 'x'}, env={'TEXMFCNF': '@HERE@/cnf:'}, change=lambda w: w.write('cnf/texmf.cnf', f'TEXINPUTS.pdflatex = .:{w.src}/a:$TEXMF/tex/{{latex,generic,}}//\n')),
    case('config', 'cnf-variable-in-the-file', why='max_print_line from a changed texmf.cnf, with no variable in the environment', files={'main.tex': doc(r'\typeout{A message long enough that it has to wrap when the line width comes down from the default of seventy nine}'), 'cnf/texmf.cnf': 'max_print_line = 60\n'}, env={'TEXMFCNF': '@HERE@/cnf:'}, change=lambda w: w.write('cnf/texmf.cnf', 'max_print_line = 200\n')),
    case('config', 'shell-escape-from-the-environment', files={'main.tex': doc(r'\edef\V{\the\pdfshellescape}', SHOW_V)}, env={'shell_escape': 'f'}, change=lambda w: w.env.__setitem__('shell_escape', 't')),
    case('config', 'halt-on-error', why='an error in the body, and a flag that says what it does', files={'main.tex': doc(r'\def\V{same}', r'\undefinedinthebody Then more.' + '\n')}, args=['-interaction=nonstopmode'], change=lambda w: w.args.append('-halt-on-error')),
    case('config', 'file-line-error', files={'main.tex': doc(r'\def\V{same}', r'\undefinedinthebody' + '\n')}, args=['-interaction=nonstopmode'], change=lambda w: w.args.append('-file-line-error')),
    case('config', 'eight-bit', why='-8bit decides what the log prints for a high character, in the preamble too', files={'main.tex': doc(r'\typeout{\string^^01}', 'Body.\n')}, change=lambda w: w.args.append('-8bit')),
    case('config', 'output-format-flag', files={'main.tex': doc(r'\edef\V{\the\pdfoutput}', SHOW_V)}, args=['-output-format=pdf'], change=lambda w: (w.args.__setitem__(0, '-output-format=dvi'))),
    case('config', 'pdfoutput-on-the-first-line', files={'main.tex': doc(r'\edef\V{\the\pdfoutput}', SHOW_V)}, first_line=r'\pdfoutput=1 \input{main}', change=lambda w: setattr(w, 'first_line', r'\pdfoutput=0 \input{main}')),
    case('config', 'jobname-on-the-first-line', files={'main.tex': doc(r'\edef\V{\jobname}', SHOW_V)}, first_line=r'\input{main}', job='main', change=lambda w: (w.args.append('-jobname=other'), setattr(w, 'job', 'other'))),
    case('config', 'draftmode-flag', why='no PDF, then a PDF', files={'main.tex': doc(r'\edef\V{\the\pdfdraftmode}', SHOW_V)}, args=['-draftmode'], change=lambda w: w.args.remove('-draftmode')),
    case('config', 'error-line', files={'main.tex': doc(r'\def\V{same}', r'\undefinedinthebody with a line long enough to be cut at some place or other, if the width says so' + '\n')}, args=['-interaction=nonstopmode'], env={'error_line': '79'}, change=lambda w: w.env.__setitem__('error_line', '70')),
    case('config', 'format-moved-earlier-on-the-path', why='another format of that name, earlier on the path', files={'main.tex': doc('', r'\FORMATMARK' + '\n')}, custom_format='first', change=lambda w: w.rebuild_format('second', where='fmts-first')),
    case('config', 'plain-format', must=True, why='the plain format has no \\document, so there is never a checkpoint', visible=False, no_restore=True, args=['-fmt=pdftex'], files={'main.tex': 'Old body.\n\\bye\n'}, change=_body_edit),
    case('config', 'path-with-spaces', must=True, why='the name of the directory and of the file', files={'a dir/main file.tex': doc(r'\input{"sub dir/macros"}', SHOW_V + 'Old body.\n'), 'a dir/sub dir/macros.tex': r'\def\V{in a directory}'}, cwd='a dir', main='"main file.tex"', job='main file', change=lambda w: w.edit('a dir/main file.tex', 'Old body.', 'New body.')),
    case('config', 'path-not-ascii', must=True, files={'d\u00e9j\u00e0/m\u00e9moire.tex': doc(r'\input{macros}', SHOW_V + 'Old body.\n'), 'd\u00e9j\u00e0/macros.tex': r'\def\V{here}'}, cwd='d\u00e9j\u00e0', main='m\u00e9moire.tex', job='m\u00e9moire', change=lambda w: w.edit('d\u00e9j\u00e0/m\u00e9moire.tex', 'Old body.', 'New body.')),
    case('config', 'very-long-path', must=True, why='two thousand bytes of directories above the current one', files={('long-directory-name-' * 5 + '/') * 20 + 'main.tex': doc(r'\input{macros}', SHOW_V + 'Old body.\n'), ('long-directory-name-' * 5 + '/') * 20 + 'macros.tex': r'\def\V{here}'}, cwd=('long-directory-name-' * 5 + '/') * 20, change=lambda w: w.edit(('long-directory-name-' * 5 + '/') * 20 + 'main.tex', 'Old body.', 'New body.')),
    # ------------------------------------------------------------------ what the run shows on the way: the terminal and the log's early content
    case('own-writes', 'terminal-text-of-the-preamble', must=True, why='nonstopmode: everything the preamble said to the terminal comes first, and the line it left open goes on', files={'main.tex': doc(r'\message{from the preamble}\typeout{a whole line}\message{left open}', r'\message{from the body}Old body.' + '\n')}, args=['-interaction=nonstopmode'], change=_body_edit),
    case('own-writes', 'long-lines-in-the-log', must=True, why='a wrapped line, and a column left in the middle of one', files={'main.tex': doc(r'\wlog{' + 'A long line that wraps in the log. ' * 8 + r'}\immediate\write-1{Log only}\message{' + 'Terminal too. ' * 12 + '}', 'Old body.\n')}, args=['-interaction=nonstopmode'], change=_body_edit),
    case('own-writes', 'overfull-box-in-the-preamble', must=True, why='a warning: history carries over and the end says to see the transcript', files={'main.tex': doc(r'\setbox0=\hbox to 1pt{Overfull text}', 'Old body.\n')}, args=['-interaction=nonstopmode'], change=_body_edit),
    case('own-writes', 'showbox-in-the-preamble-sets-history', why='\\showbox counts as an error', files={'main.tex': doc(r'\setbox0=\hbox{x}\showbox0 ', 'Old body.\n')}, args=['-interaction=nonstopmode'], visible=True, change=_body_edit),
    case('own-writes', 'error-in-the-body-after-a-restore', must=True, why='exit status 1, and "see the transcript file"', files={'main.tex': doc('', r'\undefinedinthebody Old body.' + '\n')}, args=['-interaction=nonstopmode'], change=_body_edit),
    case('own-writes', 'fatal-in-the-body-after-a-restore', must=True, visible=False, why='a fatal error with a PDF begun in the preamble', files={'main.tex': doc(r'\immediate\pdfobj{(an object)}', r'Old body.\newpage More. \input{not-a-file-here}' + '\n')}, args=['-interaction=nonstopmode', '-halt-on-error'], change=_body_edit),
    case('own-writes', 'end-of-file-with-no-end', must=True, visible=False, why='the body ends with no \\end{document}: the terminal is asked, and there is nothing there', files={'main.tex': doc('', 'Old body.\n').replace(r'\end{document}', '')}, change=_body_edit),
    case('own-writes', 'output-directory-removed', why='the restore has written nothing yet when it finds it cannot, or the terminal gets its text twice', files={'main.tex': doc(r'\newwrite\o \immediate\openout\o=note.txt \immediate\write\o{text}\immediate\closeout\o', 'Body.\n')}, args=['-interaction=nonstopmode'], change=lambda w: force_rmtree(w.out)),
    case('own-writes', 'closed-file-now-a-directory', why='the same, for a file the preamble wrote', files={'main.tex': doc(r'\newwrite\o \immediate\openout\o=note.txt \immediate\write\o{text}\immediate\closeout\o', 'Body.\n')}, args=['-interaction=nonstopmode'], change=lambda w: (w.rm_out('note.txt'), os.mkdir(os.path.join(w.out, 'note.txt')))),
    case('own-writes', 'log-now-read-only', why='the log cannot be opened', visible=True, files={'main.tex': doc(r'\def\V{same}', 'Body.\n')}, args=['-interaction=nonstopmode'], change=lambda w: os.chmod(os.path.join(w.out, 'main.log'), 0o444)),
    case('own-writes', 'read-of-a-file-open-for-writing', visible=False, why='not closed, so the read sees a part of it', no_restore=True, files={'main.tex': doc(r'\newwrite\o \immediate\openout\o=open.txt \immediate\write\o{text}\newread\r \openin\r=open.txt \read\r to\A \closein\r', 'Old body.\n')}, change=_body_edit),
    case('own-writes', 'two-closed-versions-of-one-file', must=True, why='written twice in the preamble: the journal has both, in order', files={'main.tex': doc(r'\newwrite\o \immediate\openout\o=note.txt \immediate\write\o{first}\immediate\closeout\o \input{note.txt}\relax \immediate\openout\o=note.txt \immediate\write\o{second}\immediate\closeout\o', 'Old body.\n').replace(r'{first}', r'{\string\relax}')}, change=_body_edit),
    case('own-writes', 'openout-from-the-body-of-a-whatsit', must=True, why='\\openout with no \\immediate, in a box of the preamble: it runs at shipout', files={'main.tex': doc(r'\newwrite\o \setbox0=\hbox{\openout\o=late.txt \write\o{in a box}\closeout\o}', r'Old body.\leavevmode\box0' + '\n')}, change=_body_edit),
    case('own-writes', 'recorder-with-a-kept-index', must=True, why='the .fls lists the format and the cnf files too', files={'main.tex': doc(r'\usepackage{pk}', 'Old body.\n'), 'pk.sty': r'\def\V{x}'}, args=['-recorder'], change=_body_edit),
    # ------------------------------------------------------------------ what has to come back with the image, or in Host
    case('restored', 'immediate-png-then-again', must=True, why='\\immediate writes the image in the preamble, and the table of what Host has read is empty after a restore', files={'main.tex': doc(r'\immediate\pdfximage{img.png}\edef\A{\the\pdflastximage}', r'Old body.\leavevmode\pdfrefximage\A\immediate\pdfximage{img.png}\pdfrefximage\pdflastximage' + '\n'), 'img.png': tiny_png(8, 8, 200)}, change=_body_edit),
    case('restored', 'immediate-jpeg', must=True, files={'main.tex': doc(r'\immediate\pdfximage{img.jpg}\edef\A{\the\pdflastximage}', r'Old body.\leavevmode\pdfrefximage\A' + '\n'), 'img.jpg': JPEG_BYTES}, change=_body_edit),
    case('restored', 'immediate-pdf-then-its-other-page', why='a page of a PDF written in the preamble: another page of it shares an object, which an inclusion copies once per document', files={'main.tex': doc(r'\immediate\pdfximage page 1 {two.pdf}\edef\A{\the\pdflastximage}', r'Old body.\leavevmode\pdfrefximage\A\immediate\pdfximage page 2 {two.pdf}\pdfrefximage\pdflastximage' + '\n'), 'two.pdf': tiny_pdf_two_pages()}, change=_body_edit),
    case('restored', 'pdf-page-from-the-preamble-and-its-other-page', must=True, files={'main.tex': doc(r'\pdfximage page 1 {two.pdf}\edef\A{\the\pdflastximage}', r'Old body.\leavevmode\pdfrefximage\A\pdfximage page 2 {two.pdf}\pdfrefximage\pdflastximage' + '\n'), 'two.pdf': tiny_pdf_two_pages()}, change=_body_edit),
    case('restored', 'immediate-form-with-text', why='\\immediate\\pdfxform writes a form in the preamble, with a font: is it a page in all but name?', files={'main.tex': doc(r'\setbox0=\hbox{Text in a form}\immediate\pdfxform0 \edef\A{\the\pdflastxform}', r'Old body. Text\leavevmode\pdfrefxform\A' + '\n')}, change=_body_edit),
    case('restored', 'form-with-image-immediate', files={'main.tex': doc(r'\immediate\pdfximage{img.png}\setbox0=\hbox{Text\pdfrefximage\pdflastximage}\immediate\pdfxform0 \edef\A{\the\pdflastxform}', r'Old body.\leavevmode\pdfrefxform\A' + '\n'), 'img.png': tiny_png(8, 8, 200)}, change=_body_edit),
    case('restored', 'form-not-immediate', must=True, files={'main.tex': doc(r'\setbox0=\hbox{Text in a form}\pdfxform0 \edef\A{\the\pdflastxform}', r'Old body. Text\leavevmode\pdfrefxform\A' + '\n')}, change=_body_edit),
    case('restored', 'object-stream-open-at-the-checkpoint', must=True, why='object compression on: objects of the preamble wait in a stream that is not finished', files={'main.tex': r'\pdftrailerid{}\pdfcompresslevel=9 \pdfobjcompresslevel=2 ' + '\n' + r'\documentclass{article}\immediate\pdfobj{[1 2 3]}\immediate\pdfobj{(a string)}\pdfcatalog{/Mutated \the\pdflastobj\space 0 R}' + '\n' + r'\begin{document}' + '\nOld body.\n' + r'\end{document}' + '\n'}, change=_body_edit),
    case('restored', 'reserved-object-filled-in-the-body', must=True, files={'main.tex': doc(r'\pdfobj reserveobjnum \edef\A{\the\pdflastobj}\pdfcatalog{/Later \A\space 0 R}', r'Old body.\immediate\pdfobj useobjnum \A {(filled in)}' + '\n')}, change=_body_edit),
    case('restored', 'color-stack-from-the-preamble', must=True, files={'main.tex': doc(r'\edef\A{\pdfcolorstackinit page direct{0 g}}', r'Old body.\leavevmode\pdfcolorstack\A push {1 g}Coloured\pdfcolorstack\A pop' + '\n')}, change=_body_edit),
    case('restored', 'attributes-from-the-preamble', must=True, files={'main.tex': doc(r'\pdfpageattr{/Rotate 90}\pdfpagesattr{/Mutated 1}\pdfpageresources{/Extra 2}\pdfinfo{/Title (From the preamble)}\pdfnames{/Empty []}', 'Old body.\n')}, change=_body_edit),
    case('restored', 'map-line-forms', must=True, why='\\pdfmapline replacing and deleting entries of the default map, once it is in', files={'main.tex': doc(r'\font\x=cmss10 \font\y=cmss12 \pdfincludechars\x{Q}\pdfmapline{=cmss10 CMSS12 <cmss12.pfb}\pdfmapline{-cmss17}', r'{\x Old}{\y Old}Old body.' + '\n')}, change=_body_edit),
    case('restored', 'map-file-emptied', must=True, files={'main.tex': doc(r'\pdfmapfile{}\pdfmapline{+cmss10 CMSS10 <cmss10.pfb}\font\x=cmss10 ', r'{\x Old} body.' + '\n').replace('Old} body.', 'Old} Old body.')}, change=_body_edit),
    case('restored', 'saved-box-with-its-fonts', must=True, why='a font used only in a box of the preamble', files={'main.tex': doc(r'\font\x=cmss10 \setbox0=\hbox{\x In a box}\newsavebox\kept\sbox\kept{\x A kept box}', r'Old body.\leavevmode\box0 \usebox\kept' + '\n')}, change=_body_edit),
    case('restored', 'paragraph-in-the-preamble', must=True, why='line breaking and hyphenation before the first page', files={'main.tex': doc(r'\hyphenation{pre-am-ble}\setbox0=\vbox{\everypar{}\hsize=4cm Paragraph typeset in the preamble, long enough that some of its words get hyphenated by the patterns of the format\par}', r'Old body.\par\box0' + '\n')}, change=_body_edit),
    case('restored', 'hyphenation-exceptions-from-the-preamble', must=True, files={'main.tex': doc(r'\hyphenation{dis-cern-ing}', r'\hsize=1cm Old body. discerning discerning\par' + '\n')}, change=_body_edit),
    case('restored', 'patterns-too-late', why='\\patterns in a run from a format is an error, and so no checkpoint', files={'main.tex': doc(r'\patterns{a1b}', 'Old body.\n')}, args=['-interaction=nonstopmode'], change=_body_edit),
    case('restored', 'font-expansion-in-a-box-of-the-preamble', must=True, why='expanded fonts made while the preamble set a paragraph', files={'main.tex': doc(r'\usepackage{microtype}\newsavebox\kept\savebox\kept{\parbox{4cm}{Paragraph typeset in the preamble with font expansion, long enough that it needs some of the stretch}}', r'Old body. \usebox\kept' + '\n')}, change=_body_edit),
    case('restored', 'letterspace-and-copy-of-a-font', must=True, files={'main.tex': doc(r'\font\x=cmss10 \letterspacefont\y\x 100 \pdfcopyfont\z\x \pdffontattr\x{/Mutated 1}', r'{\y Old} {\z Old} {\x Old} Old body.' + '\n')}, change=_body_edit),
    case('restored', 'char-codes-of-a-font', must=True, files={'main.tex': doc(r'\font\x=cmss10 \rpcode\x`A=50 \lpcode\x`A=50 \efcode\x`A=500 \tagcode\x`A=2 \knbscode\x`A=10 \stbscode\x`A=10 \shbscode\x`A=10 \knbccode\x`A=10 \knaccode\x`A=10 ', r'{\pdfprotrudechars=2 \x AAA} Old body.' + '\n')}, change=_body_edit),
    case('restored', 'pseudo-file-open-at-the-checkpoint', why='\\scantokens on the input stack, with more lines to come after it', files={'main.tex': ARTICLE + r'\scantokens{\def\V{x}\begin{document} Old in the pseudo-file.' + '\n' + r'Second line of it.}' + '\n' + r'\end{document}' + '\n'}, change=lambda w: w.edit('main.tex', 'Old in', 'New in')),
    case('restored', 'readline-open-at-the-checkpoint', must=True, files={'main.tex': doc(r'\newread\r \openin\r=data.txt \readline\r to\A', r'\readline\r to\V Value: \V. Old body.' + '\n'), 'data.txt': 'first % line\nsecond % line\n'}, change=_body_edit),
    case('restored', 'many-reads-of-one-file', must=True, why='one file open on two streams and as the input, at different places', files={'main.tex': doc(r'\newread\r \newread\s \openin\r=data.txt \openin\s=data.txt \read\r to\A \read\s to\B \read\s to\B', r'\read\r to\V \read\s to\W Value: \V, \W. Old body.' + '\n'), 'data.txt': 'first\nsecond\nthird\n'}, change=_body_edit),
    case('restored', 'tracing-at-the-trap', must=True, why='\\tracingcommands and \\tracingmacros on when \\document is fetched', files={'main.tex': doc(r'\tracingcommands=2 \tracingmacros=2 \tracingonline=1 ', r'\tracingcommands=0 \tracingmacros=0 Old body.' + '\n')}, args=['-interaction=nonstopmode'], change=_body_edit),
    # ------------------------------------------------------------------ ways to \document
    case('stack', 'document-outer', must=True, why='\\outer on what is armed: check_outer_validity has to go on after the trap', files={'main.tex': ARTICLE + r'\let\realdocument\document \outer\def\document{\realdocument}' + '\n' + r'\begin{document}' + '\nOld body.\n' + r'\end{document}' + '\n'}, visible=True, args=['-interaction=nonstopmode'], change=_body_edit),
    case('stack', 'document-by-csname', must=True, files={'main.tex': ARTICLE + r'\csname document\endcsname' + '\nOld body.\n' + r'\end{document}' + '\n'}, change=_body_edit),
    case('stack', 'document-after-expandafter', why='not at the big switch', files={'main.tex': ARTICLE + r'\expandafter\document\relax Old body.\end{document}' + '\n'}, change=_body_edit),
    case('stack', 'document-as-a-number', why='in a scan, not at the big switch: an error that goes on', files={'main.tex': ARTICLE + r'\count0=\document Old body.\end{document}' + '\n'}, args=['-interaction=nonstopmode'], change=_body_edit),
    case('stack', 'document-copied-by-let', why='the copy is no trap, and the run goes on with its log open', files={'main.tex': ARTICLE + r'\let\again\document' + '\n' + r'\again Old body.\end{document}' + '\n'}, change=_body_edit),
    case('stack', 'self-let-of-document-traced', visible=False, why='with \\tracingassigns on: "reassigning", as with no snapshot at all, and not "changing"', files={'main.tex': ARTICLE + r'\tracingassigns=1 \let\document\document \tracingassigns=0 ' + '\n' + r'\begin{document}Old body.\end{document}' + '\n'}, change=_body_edit),
    case('stack', 'self-let-of-document-in-a-group-traced', visible=False, why='in a group, with \\tracingrestores: a self-assignment saves nothing, so nothing is restored', files={'main.tex': ARTICLE + r'\tracingrestores=1 \begingroup\let\document\document\endgroup \tracingrestores=0 ' + '\n' + r'\begin{document}' + '\nOld body.\n' + r'\end{document}' + '\n'}, change=_body_edit),
    case('stack', 'tracing-assigns-on-at-begin-document', must=True, why='LaTeX\'s \\begin tests \\document with \\ifx on its way: that is no redefinition, and the run still takes its checkpoint', files={'main.tex': ARTICLE + r'\tracingassigns=1 \tracingrestores=1 ' + '\n' + r'\begin{document}' + '\nOld body.\n' + r'\end{document}' + '\n'}, change=_body_edit),
    case('stack', 'document-redefined-with-tracing-on', must=True, why='armed again, after a change the log shows', files={'main.tex': ARTICLE + r'\let\realdocument\document \tracingassigns=1 \def\document{\realdocument}\let\document\document ' + '\n' + r'\begin{document}' + '\nOld body.\n' + r'\end{document}' + '\n'}, change=_body_edit),
    case('stack', 'self-let-of-document-untraced-then-traced', must=True, visible=True, why='a self-assignment of what is armed, seen by a later trace of the value', files={'main.tex': ARTICLE + r'\let\document\document \tracingassigns=1 \def\other{x}\tracingassigns=0 ' + '\n' + r'\begin{document}' + '\nOld body.\n' + r'\end{document}' + '\n'}, change=_body_edit),
    case('stack', 'document-redefined-in-a-group', must=True, why='the armed word on the save stack comes back', files={'main.tex': ARTICLE + r'\begingroup\def\document{\relax}\endgroup' + '\n' + r'\begin{document}' + '\nOld body.\n' + r'\end{document}' + '\n'}, change=_body_edit),
    case('stack', 'document-in-a-box', why='at the big switch, in a box, in horizontal mode', files={'main.tex': ARTICLE + r'\setbox0=\hbox\bgroup\begin{document}Old body.\egroup\box0 \end{document}' + '\n'}, args=['-interaction=nonstopmode'], change=_body_edit),
    case('stack', 'document-from-everypar', why='\\everypar on the input stack', files={'main.tex': ARTICLE + r'\everypar{\everypar{}\begin{document}}\leavevmode Old body.\end{document}' + '\n'}, args=['-interaction=nonstopmode'], change=_body_edit),
    case('stack', 'document-in-an-alignment', why='in the alignment\'s group, with v and u templates on the stack', files={'main.tex': ARTICLE + r'\halign{#\cr\begin{document}Old body.\cr}\end{document}' + '\n'}, args=['-interaction=nonstopmode'], change=_body_edit),
    case('stack', 'document-in-math', files={'main.tex': ARTICLE + r'$\begin{document}$Old body.\end{document}' + '\n'}, args=['-interaction=nonstopmode'], change=_body_edit),
    case('stack', 'document-in-a-write', why='\\document in an \\edef, not run', files={'main.tex': ARTICLE + r'\edef\V{\noexpand\document}\begin{document}Old body.\end{document}' + '\n'}, change=_body_edit),
    case('stack', 'at-begin-document-reads-a-file', must=True, why='after the checkpoint: what it reads is the current file', files={'main.tex': doc(r'\AtBeginDocument{\input{late}}', SHOW_V + 'Old body.\n'), 'late.tex': r'\def\V{old}'}, change=lambda w: (w.edit('late.tex', 'old', 'new'), _body_edit(w))),
    case('stack', 'at-end-preamble-hook', why='a hook that runs in \\document, before the checkpoint: what it reads is in the log', files={'main.tex': doc(r'\usepackage{etoolbox}\AtEndPreamble{\input{late}}', SHOW_V + 'Old body.\n'), 'late.tex': r'\def\V{old}'}, change=lambda w: (w.edit('late.tex', 'old', 'new'), _body_edit(w))),
    case('stack', 'at-end-preamble-hook-body-edit', must=True, why='the same, with what it read as it was', files={'main.tex': doc(r'\usepackage{etoolbox}\AtEndPreamble{\input{late}}', SHOW_V + 'Old body.\n'), 'late.tex': r'\def\V{old}'}, change=_body_edit),
    case('stack', 'hook-of-begindocument-before', files={'main.tex': doc(r'\AddToHook{begindocument/before}{\input{late}}', SHOW_V + 'Old body.\n'), 'late.tex': r'\def\V{old}'}, change=lambda w: (w.edit('late.tex', 'old', 'new'), _body_edit(w))),
    case('stack', 'hook-of-begindocument-before-body-edit', must=True, why='the same, with what it read as it was', files={'main.tex': doc(r'\AddToHook{begindocument/before}{\input{late}}', SHOW_V + 'Old body.\n'), 'late.tex': r'\def\V{old}'}, change=_body_edit),
    case('stack', 'everyjob-from-the-format', why='a format\'s \\everyjob runs once, and not again on a restore', files={'main.tex': doc('', r'\the\count200 Old body.' + '\n')}, custom_format='first', format_extra=r'\everyjob{\global\advance\count200 by 1 }', change=_body_edit),
    case('stack', 'everyjob-set-in-the-preamble', must=True, why='too late for this run, and a restore must not run it', files={'main.tex': doc(r'\everyjob{\global\advance\count200 by 1 }', r'\the\count200 Old body.' + '\n')}, change=_body_edit),
    # ------------------------------------------------------------------ hard taint
    case('taint', 'write18-off-is-no-taint', must=True, why='disabled: it says so in the log, and nothing runs', files={'main.tex': doc(r'\immediate\write18{touch made-by-shell}', 'Old body.\n')}, args=['-no-shell-escape'], change=_body_edit),
    case('taint', 'write18-restricted-allowed', no_restore=True, visible=True, why='restricted, and on the list: it runs', files={'main.tex': doc(r'\immediate\write18{kpsewhich --version > /dev/null}\edef\V{\the\pdfshellescape}', SHOW_V + 'Old body.\n')}, args=['-shell-restricted'], env={'TEXMF_OUTPUT_DIRECTORY': None}, change=_body_edit),
    case('taint', 'openout-to-a-pipe', no_restore=True, files={'main.tex': doc(r'\newwrite\o \immediate\openout\o=|"cat > piped.txt" \immediate\write\o{text}\immediate\closeout\o', 'Old body.\n')}, args=['-shell-escape'], change=_body_edit),
    case('taint', 'input-from-a-pipe-no-shell', why='shell off, so `|` is refused and says so', files={'main.tex': doc(r'\newread\r \openin\r=|"echo x" \ifeof\r\def\V{refused}\else\def\V{ran}\fi', SHOW_V + 'Old body.\n')}, args=['-no-shell-escape'], change=_body_edit),
    case('taint', 'pausing-reads-the-terminal', no_restore=True, why='\\pausing in the preamble: each line waits for one from the terminal', files={'main.tex': doc('\\pausing=1 \n\\def\\V{same}\n\\pausing=0 ', 'Old body.\n')}, args=['-interaction=errorstopmode'], stdin='\n\n\n\n', change=_body_edit),
    case('taint', 'read-from-the-terminal-in-nonstop', visible=False, why='a fatal error: there is no terminal to ask', files={'main.tex': doc(r'\read16 to\V', SHOW_V)}, args=['-interaction=nonstopmode'], change=lambda w: w.edit('main.tex', 'Value', 'Other')),
    case('taint', 'read-of-a-stream-not-open', why='\\read from a closed stream is the terminal too', files={'main.tex': doc(r'\read5 to\V', SHOW_V)}, args=['-interaction=scrollmode'], stdin='old answer\n', change=lambda w: setattr(w, 'stdin', 'new answer\n')),
    # ------------------------------------------------------------------ the cache itself: the rest are in SECURITY below
]

GENERATED_IN_CWD = r'\newwrite\o \immediate\openout\o=gen.txt \immediate\write\o{line one}\immediate\write\o{line two}\immediate\closeout\o \newread\r \openin\r=gen.txt \read\r to\A'
# The log past its first buffer at the checkpoint, and a body that asks after each of its own lines how much of it is on the disk: a whole number of buffers, as stdio leaves it.
LOG_PAST_A_BUFFER = r'\newcount\lines \loop\wlog{A line of the log, \the\lines, that adds up after some of them to more than a buffer holds.}\advance\lines by 1 \ifnum\lines<60 \repeat'
LOG_SIZES = r'\lines=0 \loop\wlog{And one of the body, \the\lines, that moves the end of the log on.}\message{[\pdffilesize{\jobname.log}]}\advance\lines by 1 \ifnum\lines<120 \repeat '

CASES += [
    # ------------------------------------------------------------------ second round.  With the current directory as the output directory: paths have no directory in front
    case('nooutdir', 'body-edit', must=True, no_outdir=True, files={'main.tex': doc(r'\input{macros}', SHOW_V + 'Old body.\n'), 'macros.tex': r'\def\V{here}'}, change=_body_edit),
    case('nooutdir', 'aux-cycle', must=True, no_outdir=True, why='the .aux of the last run, read after the checkpoint', files={'main.tex': doc('', r'\section{One}\label{a}See \ref{a}. Old body.' + '\n')}, change=_body_edit),
    case('nooutdir', 'read-back-own-file', visible=False, must=True, no_outdir=True, files={'main.tex': doc(r'\newwrite\o \immediate\openout\o=gen.tex \immediate\write\o{\string\def\string\V{generated}}\immediate\closeout\o \input{gen.tex}', SHOW_V)}),
    case('nooutdir', 'read-back-own-file-removed', visible=False, no_outdir=True, files={'main.tex': doc(r'\newwrite\o \immediate\openout\o=gen.tex \immediate\write\o{\string\def\string\V{generated}}\immediate\closeout\o \input{gen.tex}', SHOW_V)}, change=lambda w: w.rm('gen.tex')),
    case('nooutdir', 'own-file-open-at-checkpoint-tampered', visible=False, no_outdir=True, files={'main.tex': doc(GENERATED_IN_CWD, r'\read\r to\V Value: \V.' + '\n')}, change=lambda w: w.write('gen.txt', 'line one\nsomething else\n')),
    case('nooutdir', 'recorder', must=True, no_outdir=True, files={'main.tex': doc(r'\input{macros}', 'Old body.\n'), 'macros.tex': r'\def\V{x}'}, args=['-recorder'], change=_body_edit),
    case('nooutdir', 'texmf-output-directory-variable', why='the option\'s other name', no_outdir=True, files={'main.tex': doc(r'\IfFileExists{note.tex}{\input{note.tex}}{\def\V{no note}}', SHOW_V), 'other/note.tex': r'\def\V{note in the other directory}', 'other/x': 'x'}, env={'TEXMF_OUTPUT_DIRECTORY': None}, change=lambda w: (w.env.__setitem__('TEXMF_OUTPUT_DIRECTORY', w.path('other')), setattr(w, 'outdir', w.path('other')))),
    # ------------------------------------------------------------------ more of what a file open at the checkpoint can do
    case('read', 'open-file-nothing-read-yet', why='no line read: nothing of it is known, and it may change', files={'main.tex': doc(r'\newread\r \openin\r=data.txt ', r'\read\r to\V Value: \V.' + '\n'), 'data.txt': 'first\n'}, must=True, change=lambda w: w.write('data.txt', 'other\n')),
    case('read', 'open-file-grows-after-a-line', must=True, why='the line read had its terminator: more after it is what a cold run sees too', files={'main.tex': doc(r'\newread\r \openin\r=data.txt \read\r to\A', r'\read\r to\V Value: \V.' + '\n'), 'data.txt': 'first\n'}, change=lambda w: w.write('data.txt', 'first\nsecond\n')),
    case('read', 'from-standard-input', why='what stdin holds can be read once: a check of it would leave nothing for the cold run', files={'main.tex': doc(r'\newread\r \openin\r=/dev/stdin \read\r to\V', SHOW_V)}, stdin='old answer\n', change=lambda w: setattr(w, 'stdin', 'new answer\n')),
    case('stack', 'endinput-on-the-line-of-begin-document', must=True, why='\\endinput then \\begin{document}: the file is at its end for the run, with more in it', files={'main.tex': ARTICLE + r'\makeatletter\let\primitiveinput\@@input\makeatother\primitiveinput pre ' + '\n' + 'Old body.\n' + r'\end{document}' + '\n', 'pre.tex': r'\endinput\begin{document}' + '\nunread text\n'}, change=_body_edit),
    case('stack', 'everything-on-the-first-line', why='no file at all: the log opens at the class', files={'main.tex': 'unused\n'}, first_line=r'\pdfcompresslevel=0 \pdfobjcompresslevel=0 \pdftrailerid{}\documentclass{article}\begin{document}Old.\end{document}', job='article', change=lambda w: setattr(w, 'first_line', r'\pdfcompresslevel=0 \pdfobjcompresslevel=0 \pdftrailerid{}\documentclass{article}\begin{document}New.\end{document}')),
    case('stack', 'outer-document-in-an-argument', why='\\outer on what is armed, in the argument of a macro: "Forbidden control sequence", and the capture loops see it', files={'main.tex': ARTICLE + r'\let\realdocument\document \outer\def\document{\realdocument}\def\foo#1{\def\W{#1}}\foo{\document}' + '\n' + r'\let\document\realdocument \begin{document}' + '\nOld body.\n' + r'\end{document}' + '\n'}, args=['-interaction=nonstopmode'], change=_body_edit),
    case('stack', 'document-as-a-macro-argument', must=True, why='a mention, and then the real thing', files={'main.tex': ARTICLE + r'\def\foo#1{\def\W{#1}}\foo{\document}\W' + '\nOld body.\n' + r'\end{document}' + '\n'}, change=_body_edit),
    # ------------------------------------------------------------------ what comes back on the save stack
    case('volatile', 'day-assigned-in-a-group-still-open', why='the old value of \\day waits on the save stack, and the end of the group brings it back', files={'main.tex': ARTICLE + r'\begingroup\day=5 ' + '\n' + r'\begin{document}' + '\n' + r'\endgroup\the\day' + '\n' + r'\end{document}' + '\n'}, change=lambda w: w.clock(86400)),
    case('volatile', 'time-assigned-in-a-group-still-open', files={'main.tex': ARTICLE + r'\begingroup\time=5 ' + '\n' + r'\begin{document}' + '\n' + r'\endgroup\the\time' + '\n' + r'\end{document}' + '\n'}, change=lambda w: w.clock(60)),
    case('volatile', 'year-assigned-outside-the-open-group', must=True, why='assigned before \\begin opens its group: nothing on the save stack', files={'main.tex': doc(r'\year=2000 ', r'\the\year Old body.' + '\n')}, change=_body_edit),
    # ------------------------------------------------------------------ a file the run looks for at its very start
    case('config', 'tcx-found-elsewhere', why='the name found is in the log', files={'main.tex': doc('', 'Body.\n'), 'tree/x': 'x'}, args=['-translate-file=cp227.tcx'], env={'TEXMFHOME': '@HERE@/tree'}, change=lambda w: w.write('tree/web2c/cp227.tcx', _zoneinfo_free_tcx())),
    # ------------------------------------------------------------------ inclusions
    case('restored', 'two-pages-of-a-pdf-from-the-preamble', must=True, why='neither written yet: the object they share is copied once', files={'main.tex': doc(r'\pdfximage page 1 {two.pdf}\edef\A{\the\pdflastximage}\pdfximage page 2 {two.pdf}\edef\B{\the\pdflastximage}', r'Old body.\leavevmode\pdfrefximage\A\pdfrefximage\B' + '\n'), 'two.pdf': tiny_pdf_two_pages()}, change=_body_edit),
    case('restored', 'immediate-pdf-with-a-sibling-waiting', why='one page written in the preamble, another of the file only named: the document stays open in Host, with what it has copied', files={'main.tex': doc(r'\pdfximage page 2 {two.pdf}\edef\B{\the\pdflastximage}\immediate\pdfximage page 1 {two.pdf}\edef\A{\the\pdflastximage}', r'Old body.\leavevmode\pdfrefximage\A\pdfrefximage\B' + '\n'), 'two.pdf': tiny_pdf_two_pages()}, change=_body_edit),
    case('restored', 'pdf-page-in-the-preamble-immediate-sibling-in-the-body', must=True, why='a page named in the preamble keeps the document open while a sibling is written with \\immediate in the body', files={'main.tex': doc(r'\pdfximage page 1 {two.pdf}\edef\A{\the\pdflastximage}', r'Old body.\immediate\pdfximage page 2 {two.pdf}\leavevmode\pdfrefximage\A\pdfrefximage\pdflastximage' + '\n'), 'two.pdf': tiny_pdf_two_pages()}, change=_body_edit),
    case('restored', 'pdf-newer-than-allowed-warned-once', must=True, why='the warning of \\pdfximage is in the log of the preamble, and the file is read again', files={'main.tex': doc(r'\pdfminorversion=4 \pdfinclusionerrorlevel=0 \pdfximage{new.pdf}\edef\A{\the\pdflastximage}', r'Old body.\leavevmode\pdfrefximage\A' + '\n'), 'new.pdf': tiny_pdf_two_pages(minor=7)}, args=['-interaction=nonstopmode'], change=_body_edit),
    case('restored', 'pdf-newer-than-allowed-level-raised-after', must=True, why='no error when it was named: the level then is what counts', files={'main.tex': doc(r'\pdfminorversion=4 \pdfinclusionerrorlevel=-1 \pdfximage{new.pdf}\edef\A{\the\pdflastximage}\pdfinclusionerrorlevel=1 ', r'Old body.\leavevmode\pdfrefximage\A' + '\n'), 'new.pdf': tiny_pdf_two_pages(minor=7)}, args=['-interaction=nonstopmode'], change=_body_edit),
    case('restored', 'images-with-the-recorder', must=True, why='the .fls lists each image once', files={'main.tex': doc(r'\pdfximage{img.png}\edef\A{\the\pdflastximage}\pdfximage{img.jpg}\edef\B{\the\pdflastximage}\pdfximage page 1 {two.pdf}\edef\C{\the\pdflastximage}', r'Old body.\leavevmode\pdfrefximage\A\pdfrefximage\B\pdfrefximage\C' + '\n'), 'img.png': tiny_png(8, 8, 200), 'img.jpg': JPEG_BYTES, 'two.pdf': tiny_pdf_two_pages()}, args=['-recorder'], change=_body_edit),
    case('restored', 'png-with-alpha-used-on-a-later-page', must=True, why='a transparent PNG gives a page group, from what it was named until the next page is out', files={'main.tex': doc(r'\pdfximage{alpha.png}\edef\A{\the\pdflastximage}', r'Old body.\newpage\leavevmode\pdfrefximage\A\newpage Third.' + '\n'), 'alpha.png': tiny_png(8, 8, 200, alpha=True)}, change=_body_edit),
    case('restored', 'png-with-alpha-named-before-the-pdf-opens', must=True, why='the version it was read under is not fixed yet, and by the checkpoint it is: reading it again must not change its mind about a page group', files={'main.tex': doc(r'\pdfximage{alpha.png}\edef\A{\the\pdflastximage}\immediate\pdfobj{(opens the file)}', r'Old body.\leavevmode\pdfrefximage\A' + '\n'), 'alpha.png': tiny_png(8, 8, 200, alpha=True)}, change=_body_edit),
    case('restored', 'pngs-with-alpha-named-before-and-after-the-pdf-opens', must=True, files={'main.tex': doc(r'\pdfximage{alpha.png}\edef\A{\the\pdflastximage}\immediate\pdfobj{(opens the file)}\pdfximage{alpha.png}\edef\B{\the\pdflastximage}', r'Old body.\leavevmode\pdfrefximage\A\newpage\pdfrefximage\B\newpage Third.' + '\n'), 'alpha.png': tiny_png(8, 8, 200, alpha=True)}, change=_body_edit),
    case('restored', 'jpeg-named-before-the-pdf-opens', must=True, files={'main.tex': doc(r'\pdfximage{img.jpg}\edef\A{\the\pdflastximage}\immediate\pdfobj{(opens the file)}', r'Old body.\leavevmode\pdfrefximage\A' + '\n'), 'img.jpg': JPEG_BYTES}, change=_body_edit),
    case('restored', 'minor-version-low-with-a-png', must=True, files={'main.tex': doc(r'\pdfminorversion=3 \pdfximage{alpha.png}\edef\A{\the\pdflastximage}', r'Old body.\leavevmode\pdfrefximage\A' + '\n'), 'alpha.png': tiny_png(8, 8, 200, alpha=True)}, change=_body_edit),
    case('restored', 'image-named-in-the-preamble-never-used', must=True, why='nothing of it reaches the PDF, in a cold run either', files={'main.tex': doc(r'\pdfximage{img.png}\pdfximage{alpha.png}\pdfximage page 1 {two.pdf}', 'Old body.\n'), 'img.png': tiny_png(8, 8, 200), 'alpha.png': tiny_png(8, 8, 200, alpha=True), 'two.pdf': tiny_pdf_two_pages()}, change=_body_edit),
    case('restored', 'pdfobj-file-changes', why='\\immediate\\pdfobj file reads in the preamble', files={'main.tex': doc(r'\immediate\pdfobj file {data.txt}\pdfcatalog{/Mutated \the\pdflastobj\space 0 R}', 'Body.\n'), 'data.txt': '(old)'}, change=lambda w: w.write('data.txt', '(new)')),
    case('restored', 'pdfobj-file-not-immediate', must=True, why='read when it is written, in the body: the file as it is then', files={'main.tex': doc(r'\pdfobj file {data.txt}\pdfrefobj\pdflastobj', 'Old body.\n').replace(r'\pdfrefobj\pdflastobj', r'\edef\A{\the\pdflastobj}').replace('Old body.', r'Old body.\leavevmode\pdfrefobj\A\relax'), 'data.txt': '(old)'}, change=lambda w: (w.write('data.txt', '(new)'), _body_edit(w))),
    case('stat', 'image-removed', why='a fatal error in a cold run', files={'main.tex': doc(r'\pdfximage{img.png}', 'Body.\n'), 'img.png': tiny_png(8, 8, 200)}, args=['-interaction=nonstopmode'], change=lambda w: w.rm('img.png')),
    case('stat', 'input-found-in-output-directory-first', files={'main.tex': doc(r'\input{macros}', SHOW_V), 'macros.tex': r'\def\V{in the source directory}'}, change=lambda w: w.write_out('macros.tex', r'\def\V{in the output directory}')),
    case('restored', 'synctex-flag', must=True, files={'main.tex': doc(r'\def\V{same}', 'Old body.\n')}, args=['-synctex=1'], change=_body_edit),
    # ------------------------------------------------------------------ shell in the body of a restored run: it has what a child gets
    case('taint', 'shell-in-the-body-after-a-restore', must=True, why='what the child inherits: the output directory, the program name, and so what kpsewhich finds', files={'main.tex': doc('', r'\immediate\write18{env | grep -E "^(TEXMF_OUTPUT_DIRECTORY|progname|engine|SELFAUTO[A-Z]*)=" | sort > \jobname.childenv; kpsewhich article.cls >> \jobname.childenv}Old body.' + '\n')}, args=['-shell-escape'], env={'TEXMF_OUTPUT_DIRECTORY': None}, no_outdir=True, change=_body_edit),
]

CASES += [
    # ------------------------------------------------------------------ third round
    case('config', 'texmfvar-tree', why='in the key, and in the search path', files={'main.tex': doc(r'\usepackage{pk}', SHOW_V), 'one/tex/latex/pk.sty': r'\def\V{in one}', 'two/tex/latex/pk.sty': r'\def\V{in two}'}, env={'TEXMFVAR': '@HERE@/one'}, change=lambda w: w.env.__setitem__('TEXMFVAR', w.path('two'))),
    case('own-writes', 'stream-still-open-read-in-the-body', must=True, why='written in the preamble, not closed, read in the body: what is on the disk by then is no more than in a run with no checkpoint', files={'main.tex': doc(r'\newwrite\o \immediate\openout\o=open.txt \immediate\write\o{some text}', r'\newread\r \openin\r=open.txt \ifeof\r Nothing.\else\read\r to\V Value: \V.\fi Old body.' + '\n')}, change=_body_edit),
    case('own-writes', 'stream-still-open-read-in-the-body-of-a-run-that-writes-a-checkpoint', why='the same with an edit of the preamble: this run takes the checkpoint, and that must not put the bytes on the disk', files={'main.tex': doc(r'\def\W{old}\newwrite\o \immediate\openout\o=open.txt \immediate\write\o{some text}', r'\newread\r \openin\r=open.txt \ifeof\r Nothing.\else\read\r to\V Value: \V.\fi \W.' + '\n')}, change=lambda w: w.edit('main.tex', r'\def\W{old}', r'\def\W{new}')),
    case('own-writes', 'long-stream-still-open', why='past its buffer: part of it on the disk', files={'main.tex': doc(r'\newwrite\o \immediate\openout\o=open.txt \newcount\c \loop\immediate\write\o{A line of text that adds up, after a good many of them, to more than a buffer holds.}\advance\c by 1 \ifnum\c<1000 \repeat', r'\pdfmdfivesum file {open.txt} Old body.' + '\n')}, change=_body_edit),
    case('own-writes', 'log-past-its-buffer-read-in-the-body', must=True, why='a restore put the whole log of the preamble on the disk, and the buffers after it went out from there', files={'main.tex': doc(LOG_PAST_A_BUFFER, LOG_SIZES + 'Old body.\n')}, change=_body_edit),
    case('own-writes', 'log-read-in-the-body-on-a-narrower-day', must=True, why='the 4th of the next month: a byte less of date in the first line, and the buffers still go out every 4 KiB from the start', files={'main.tex': doc(LOG_PAST_A_BUFFER, LOG_SIZES + 'Old body.\n')}, change=lambda w: (w.clock(20 * 86400), _body_edit(w))),
    case('config', 'bad-value-of-a-bound-variable', visible=False, why='a message on stderr, from what main_body does first, which a restore skips', files={'main.tex': doc(r'\def\V{same}', 'Old body.\n')}, env={'error_line': '0'}, change=_body_edit),
    case('config', 'progname-chooses-a-path', why='-progname picks TEXINPUTS.name in a texmf.cnf', files={'main.tex': doc(r'\usepackage{pk}', SHOW_V), 'a/pk.sty': r'\def\V{package a}', 'b/pk.sty': r'\def\V{package b}', 'cnf/texmf.cnf': 'TEXINPUTS.one = .:@HERE@/a:$TEXMF/tex/{latex,generic,}//\nTEXINPUTS.two = .:@HERE@/b:$TEXMF/tex/{latex,generic,}//\n'}, env={'TEXMFCNF': '@HERE@/cnf:'}, args=['-progname=one'], change=lambda w: w.args.__setitem__(0, '-progname=two')),
    case('restored', 'virtual-font-in-a-box-of-the-preamble', must=True, why='a font with a .vf: read when a box of the preamble first uses a character, its packets kept in the image', files={'main.tex': doc(r'\font\x=ptmr7t \setbox0=\hbox{\x In a box}', r'Old body.\leavevmode\box0 {\x And after}' + '\n')}, change=_body_edit),
    case('own-writes', 'interaction-set-in-the-preamble', must=True, why='the mode the run is in comes back, and so where the terminal text goes from there', files={'main.tex': doc(r'\scrollmode\typeout{now on the terminal}', r'\message{from the body}Old body.' + '\n')}, change=_body_edit),
    case('own-writes', 'batchmode-set-in-the-preamble', must=True, files={'main.tex': doc(r'\typeout{on the terminal}\batchmode', r'\message{only in the log}Old body.' + '\n')}, args=['-interaction=nonstopmode'], change=_body_edit),
    case('restored', 'annotation-in-a-box-of-the-preamble', must=True, files={'main.tex': doc(r'\setbox0=\hbox{\pdfannot width 10pt height 10pt depth 0pt {/Subtype /Text /Contents (a note)}x}', r'Old body.\leavevmode\box0' + '\n')}, change=_body_edit),
    case('restored', 'outlines-and-destinations-in-the-preamble', must=True, why='\\pdfoutline is kept in a list, \\pdfdest in a box', files={'main.tex': doc(r'\pdfoutline goto name {far} {First}\setbox0=\hbox{\pdfdest name {far} xyz x}', r'Old body.\leavevmode\box0' + '\n')}, change=_body_edit),
    case('restored', 'link-in-a-box-of-the-preamble', must=True, files={'main.tex': doc(r'\setbox0=\hbox{\pdfstartlink height 8pt depth 0pt goto name {far}x\pdfendlink}', r'Old body.\leavevmode\box0\pdfdest name {far} fit \relax' + '\n')}, change=_body_edit),
    case('restored', 'write-and-literal-in-a-box-of-the-preamble', must=True, files={'main.tex': doc(r'\newwrite\o \immediate\openout\o=seen.txt \setbox0=\hbox{\write\o{\the\inputlineno}\pdfliteral{0 g}\pdfsave\pdfrestore\special{x}\pdfsavepos x}', r'Old body.\leavevmode\box0 \the\pdflastxpos' + '\n')}, change=_body_edit),
    case('restored', 'output-routine-and-marks-from-the-preamble', must=True, files={'main.tex': doc(r'\output={\global\setbox1=\box255 \deadcycles=0 \shipout\box1 }\mark{from the preamble}', r'Old body.\mark{from the body}\newpage \topmark' + '\n')}, change=_body_edit),
    case('restored', 'many-registers-and-sparse-arrays', must=True, why='e-TeX\'s sparse arrays are in mem', files={'main.tex': doc(r'\count3000=7 \dimen3000=1pt \skip3000=1pt plus 1fil \toks3000={kept}\setbox3000=\hbox{x}\muskip3000=1mu \marks3000{from the preamble}\interlinepenalties=2 1 2 \parshape 1 0pt 10cm ', r'\the\count3000 \the\dimen3000 \the\skip3000 \the\toks3000 \box3000 \the\muskip3000 Old body.' + '\n')}, change=_body_edit),
    case('restored', 'many-new-control-sequences', must=True, why='the hash table grows in the preamble past what the format left room for', files={'main.tex': doc(r'\newcount\c \c=0 \loop\expandafter\def\csname name\the\c\endcsname{\the\c}\advance\c by 1 \ifnum\c<5000 \repeat', r'\csname name4999\endcsname Old body.' + '\n')}, change=_body_edit),
    case('restored', 'big-token-lists', must=True, files={'main.tex': doc(r'\newtoks\t \t={x}\newcount\c \c=0 \loop\t=\expandafter{\the\t\the\t}\advance\c by 1 \ifnum\c<14 \repeat \edef\V{\the\numexpr\c}', SHOW_V + 'Old body.\n')}, change=_body_edit),
    case('restored', 'strings-made-in-the-preamble', must=True, why='\\pdfstrcmp, \\jobname, font names, \\pdfmdfivesum make strings', files={'main.tex': doc(r'\edef\V{\pdfmdfivesum{abc}\jobname\pdfstrcmp{a}{b}}\font\x=cmss10 at 11pt \edef\W{\fontname\x}', SHOW_V + r'\W Old body.' + '\n')}, change=_body_edit),
    case('stack', 'input-stack-deep-at-the-checkpoint', must=True, why='three files on the stack and a macro on top', files={'main.tex': ARTICLE + r'\input{one}' + '\nOld body.\n' + r'\end{document}' + '\n', 'one.tex': r'\input{two}' + '\nAfter two.\n', 'two.tex': r'\def\go{\begin{document}}' + '\n' + r'\go' + '\nAfter go.\n'}, change=_body_edit),
    case('stack', 'conditional-open-at-the-checkpoint', must=True, why='\\begin{document} inside \\iftrue: the \\fi is still to come', files={'main.tex': ARTICLE + r'\iftrue\begin{document}' + '\nOld body.\n' + r'\fi\end{document}' + '\n'}, change=_body_edit),
    case('stack', 'endlinechar-and-catcodes-changed', must=True, files={'main.tex': ARTICLE + r'\endlinechar=-1 \catcode`\Q=13 \defQ{active letter}\begin{document}' + '\nOld body. Q\n' + r'\endlinechar=13 \end{document}' + '\n'}, change=_body_edit),
    case('volatile', 'real-clock-not-pinned', must=True, why='no SOURCE_DATE_EPOCH: what the body reads of the clock comes from this run.  The date alone, or the two would differ', files={'main.tex': doc(NO_DATES, r'\the\year-\the\month-\the\day Old body.' + '\n')}, env={'SOURCE_DATE_EPOCH': None, 'FORCE_SOURCE_DATE': None}, change=_body_edit),
]


# ==================================================================== Hostile or damaged caches, with a protocol of their own.  Each returns what went wrong, or None
def xxh64(data, seed=0):
    P1, P2, P3, P4, P5 = 0x9E3779B185EBCA87, 0xC2B2AE3D27D4EB4F, 0x165667B19E3779F9, 0x85EBCA77C2B2AE63, 0x27D4EB2F165667C5
    M = (1 << 64) - 1
    rol = lambda x, r: ((x << r) | (x >> (64 - r))) & M
    rnd = lambda acc, w: (rol((acc + w * P2) & M, 31) * P1) & M
    merge = lambda h, v: (((h ^ rnd(0, v)) * P1) + P4) & M
    n, p = len(data), 0
    if n >= 32:
        a, b, c, d = (seed + P1 + P2) & M, (seed + P2) & M, seed, (seed - P1) & M
        while p + 32 <= n:
            w = struct.unpack_from('<4Q', data, p)
            a, b, c, d = rnd(a, w[0]), rnd(b, w[1]), rnd(c, w[2]), rnd(d, w[3])
            p += 32
        h = (rol(a, 1) + rol(b, 7) + rol(c, 12) + rol(d, 18)) & M
        for v in (a, b, c, d):
            h = merge(h, v)
    else:
        h = (seed + P5) & M
    h = (h + n) & M
    while p + 8 <= n:
        h = ((rol(h ^ rnd(0, struct.unpack_from('<Q', data, p)[0]), 27) * P1) + P4) & M
        p += 8
    if p + 4 <= n:
        h = ((rol(h ^ ((struct.unpack_from('<I', data, p)[0] * P1) & M), 23) * P2) + P3) & M
        p += 4
    while p < n:
        h = (rol(h ^ ((data[p] * P5) & M), 11) * P1) & M
        p += 1
    h ^= h >> 33
    h = (h * P2) & M
    h ^= h >> 29
    h = (h * P3) & M
    return h ^ (h >> 32)


def ck_files(w):
    d = os.path.join(w.cache, 'rstex', 'checkpoints')
    return [os.path.join(d, n) for n in sorted(os.listdir(d)) if n.endswith('.ck')] if os.path.isdir(d) else []


def against_cold(w, cold, run):
    """Runs `run`, from the world as it is, and the cold engine from the same state.  What differs, or None."""
    w.save('before-check')
    r = run()
    out = w.outputs()
    w.restore('before-check')
    c = w.run(cold, cache=os.path.join(w.base, 'cold-cache'))
    return r, (f'exit status: check {r["rc"]}, cold {c["rc"]}' if r['rc'] != c['rc'] else judge(out, w.outputs(), w.job, [w.base]))


def warm_up(w, engine):
    w.run(engine)
    r = w.run(engine)
    return r, ck_files(w)


def sec_dir_is_a_symlink(w, engine, cold):
    elsewhere = os.path.join(w.base, 'elsewhere')
    os.makedirs(elsewhere)
    os.symlink(elsewhere, os.path.join(w.cache, 'rstex'))
    w.run(engine)
    r, why = against_cold(w, cold, lambda: w.run(engine))
    if r['restored']:
        return 'restored through a symbolic link'
    if os.listdir(elsewhere):
        return f'wrote through a symbolic link: {os.listdir(elsewhere)}'
    return why


def sec_dir_writable_by_others(w, engine, cold):
    _, cks = warm_up(w, engine)
    if not cks:
        return 'no checkpoint to begin with'
    os.chmod(os.path.join(w.cache, 'rstex'), 0o777)
    r, why = against_cold(w, cold, lambda: w.run(engine))
    return 'restored from a directory that others can write' if r['restored'] else why


def sec_dir_is_private(w, engine, cold):
    w.run(engine)
    mode = os.stat(os.path.join(w.cache, 'rstex')).st_mode & 0o777
    return None if mode == 0o700 else f'the cache directory has mode {mode:o}'


def sec_old_dir_made_private(w, engine, cold):
    """A directory from before, that only its owner could write to: it goes on being used, and others no longer read it."""
    os.mkdir(os.path.join(w.cache, 'rstex'), 0o755)
    os.chmod(os.path.join(w.cache, 'rstex'), 0o755)
    r, cks = warm_up(w, engine)
    mode = os.stat(os.path.join(w.cache, 'rstex')).st_mode & 0o777
    if mode != 0o700:
        return f'the cache directory has mode {mode:o}'
    return None if r['restored'] else 'no restore'


def sec_ck_writable_by_others(w, engine, cold):
    _, cks = warm_up(w, engine)
    os.chmod(cks[0], 0o666)
    r, why = against_cold(w, cold, lambda: w.run(engine))
    return 'restored from a file that others can write' if r['restored'] else why


def sec_ck_is_a_symlink(w, engine, cold):
    _, cks = warm_up(w, engine)
    kept = os.path.join(w.base, 'kept.ck')
    shutil.move(cks[0], kept)
    os.symlink(kept, cks[0])
    r, why = against_cold(w, cold, lambda: w.run(engine))
    return 'restored through a symbolic link' if r['restored'] else why


def sec_ck_is_a_fifo(w, engine, cold):
    _, cks = warm_up(w, engine)
    os.unlink(cks[0])
    os.mkfifo(cks[0])
    try:
        r, why = against_cold(w, cold, lambda: w.run(engine, timeout=60))
    except subprocess.TimeoutExpired:
        return 'hangs on a FIFO in place of the file'
    return why


def sec_ck_is_a_directory(w, engine, cold):
    _, cks = warm_up(w, engine)
    os.unlink(cks[0])
    os.mkdir(cks[0])
    r, why = against_cold(w, cold, lambda: w.run(engine))
    return why


def sec_foreign_config(w, engine, cold):
    """A sound checkpoint of another command line, under this one's name."""
    w.args = ['-jobname=first']
    w.job = 'first'
    _, cks = warm_up(w, engine)
    w.args = ['-jobname=second']
    w.job = 'second'
    _, cks2 = warm_up(w, engine)
    other = [c for c in cks2 if c not in cks]
    if not other:
        return 'no checkpoint for the second name'
    shutil.copyfile(cks[0], other[0])
    r, why = against_cold(w, cold, lambda: w.run(engine))
    return 'restored another configuration' if r['restored'] else why


def _forge(path, meta_len=None, image_len=None, meta=None):
    """Rewrites a checkpoint's header and meta part, with hashes that hold: what an accident cannot do."""
    with open(path, 'rb') as f:
        d = f.read()
    magic, ml, il, mh, ih, size, _ = struct.unpack_from('<7Q', d)
    m = d[56:56 + ml]
    rest = d[56 + ml:]
    if meta is not None:
        m = meta(m)
        mh = xxh64(m)
    ml2 = len(m) if meta_len is None else meta_len
    il2 = il if image_len is None else image_len
    h = struct.pack('<6Q', magic, ml2, il2, mh, ih, size)
    with open(path, 'wb') as f:
        f.write(h + struct.pack('<Q', xxh64(h)) + m + rest)


def sec_header_lengths_wrap(w, engine, cold):
    """Lengths that add up to the file's length only modulo 2^64: no huge allocation, no panic."""
    _, cks = warm_up(w, engine)
    size = os.path.getsize(cks[0])
    _forge(cks[0], meta_len=(1 << 63) + 8, image_len=(1 << 63) - 8 + size - 56)
    r, why = against_cold(w, cold, lambda: w.run(engine))
    return why


def sec_spans_out_of_the_image(w, engine, cold):
    """The last span of the meta part reaches past the POD part of the image."""
    def edit(m):
        return m[:-8] + struct.pack('<Q', 1 << 60)
    _, cks = warm_up(w, engine)
    _forge(cks[0], meta=edit)
    r, why = against_cold(w, cold, lambda: w.run(engine))
    return why


def sec_spans_reversed(w, engine, cold):
    def edit(m):
        n = struct.unpack_from('<Q', m, len(m) - 16)[0]
        return m[:-16] + struct.pack('<QQ', n + 8, n)
    _, cks = warm_up(w, engine)
    _forge(cks[0], meta=edit)
    r, why = against_cold(w, cold, lambda: w.run(engine))
    return why


def sec_bit_flips(w, engine, cold):
    """One flipped bit at a time, over the header, the meta part and the image.  Never a crash, and never other output."""
    import random
    _, cks = warm_up(w, engine)
    path = cks[0]
    with open(path, 'rb') as f:
        good = f.read()
    ml = struct.unpack_from('<Q', good, 8)[0]
    rng = random.Random(7)
    spots = list(range(0, 56, 7)) + [56 + rng.randrange(ml) for _ in range(10)] + [56 + ml + rng.randrange(len(good) - 56 - ml) for _ in range(16)] + [len(good) - 1]
    w.save('flip-base')
    ref = w.run(cold, cache=os.path.join(w.base, 'cold-cache'))
    ref_out = w.outputs()
    for at in spots:
        w.restore('flip-base')
        d = bytearray(good)
        d[at] ^= 1 << rng.randrange(8)
        with open(path, 'wb') as f:
            f.write(d)
        r = w.run(engine)
        why = f'exit status {r["rc"]}' if r['rc'] != ref['rc'] else judge(w.outputs(), ref_out, w.job, [w.base])
        if why:
            return f'a bit flipped at {at} of {len(good)} (meta from 56, {ml} bytes): {why}'
    return None


def sec_cuts(w, engine, cold):
    """The file cut short at each kind of place."""
    _, cks = warm_up(w, engine)
    path = cks[0]
    with open(path, 'rb') as f:
        good = f.read()
    ml = struct.unpack_from('<Q', good, 8)[0]
    w.save('cut-base')
    ref = w.run(cold, cache=os.path.join(w.base, 'cold-cache'))
    ref_out = w.outputs()
    for n in (0, 8, 55, 56, 57, 56 + ml // 2, 56 + ml, 56 + ml + 1, (len(good) + 56 + ml) // 2, len(good) - 4096, len(good) - 1):
        w.restore('cut-base')
        with open(path, 'wb') as f:
            f.write(good[:n])
        r = w.run(engine)
        why = f'exit status {r["rc"]}' if r['rc'] != ref['rc'] else judge(w.outputs(), ref_out, w.job, [w.base])
        if why:
            return f'cut at {n} of {len(good)}: {why}'
    return None


def sec_stale_tmp_is_not_used(w, engine, cold):
    """What a run killed in mid-write leaves: a temporary with half a file.  It is not looked at, and it does not stop the next publish."""
    w.run(engine)
    cks = ck_files(w)
    with open(cks[0], 'rb') as f:
        good = f.read()
    with open(cks[0] + '.99999.tmp', 'wb') as f:
        f.write(good[:len(good) // 2])
    os.unlink(cks[0])
    r, why = against_cold(w, cold, lambda: w.run(engine))
    if why:
        return why
    return None if w.run(engine)['restored'] else 'no checkpoint was published next to a stale temporary'


def sec_killed_in_mid_run(w, engine, cold):
    """The real thing: SIGKILL after a while, at many points of a run that may be writing a checkpoint."""
    import signal
    env = w.run_env()
    argv = w.argv(engine)
    w.save('kill-base')
    ref = w.run(cold, cache=os.path.join(w.base, 'cold-cache'))
    ref_out = w.outputs()
    for delay in range(0, 60, 4):
        w.restore('kill-base')
        for c in ck_files(w):
            os.unlink(c)
        p = subprocess.Popen(argv, cwd=w.cwd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        time.sleep(delay / 1000)
        try:
            os.killpg(p.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        p.wait()
        for c in ck_files(w):
            with open(c, 'rb') as f:
                d = f.read()
            ml, il = struct.unpack_from('<2Q', d, 8)
            if len(d) != 56 + ml + il:
                return f'a published file that is not whole, killed after {delay} ms'
        w.restore('kill-base')
        r = w.run(engine)
        why = f'exit status {r["rc"]}' if r['rc'] != ref['rc'] else judge(w.outputs(), ref_out, w.job, [w.base])
        if why:
            return f'after a kill at {delay} ms: {why}'
    return None


def sec_racing(w, engine, cold):
    """Eight runs at once on one document, four times, starting with no checkpoint: whichever reads whichever, the result is the same."""
    env = w.run_env()
    argv = w.argv(engine)
    for _ in range(4):
        ps = [subprocess.Popen(argv, cwd=w.cwd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE) for _ in range(8)]
        for p in ps:
            _, err = p.communicate(timeout=300)
            if p.returncode != 0:
                return f'exit status {p.returncode}: {err[-200:]!r}'
    r, why = against_cold(w, cold, lambda: w.run(engine))
    if why:
        return why
    return None if r['restored'] else 'no restore after all that'


def _changed_before_it_was_read(w, engine, cold, changed_to):
    """A preamble writes a file and closes it, something beside the run changes it, and the preamble goes on to look at it.  A restore gives the journal's bytes back, so a cold run reads those, and the image has what came of the others."""
    import threading
    gate = os.path.join(w.outdir, 'gate.txt')
    for attempt in range(5):
        done = threading.Event()

        def meddle():
            while not done.is_set():
                if os.path.exists(gate):
                    with open(os.path.join(w.outdir, 'gen.tex'), 'w') as f:
                        f.write(changed_to)
                    return
                time.sleep(0.0001)
        if os.path.exists(gate):
            os.unlink(gate)
        t = threading.Thread(target=meddle)
        t.start()
        first = w.run(engine)
        done.set()
        t.join()
        with open(os.path.join(w.outdir, w.job + '.log'), 'rb') as f:
            if b'SAW another' in f.read():
                break
        for n in os.listdir(os.path.join(w.cache, 'rstex', 'checkpoints')):
            if n.endswith('.ck'):
                os.unlink(os.path.join(w.cache, 'rstex', 'checkpoints', n))
    else:
        return 'the change came too late each time, so this says nothing'
    if first['rc'] != 0:
        return f'exit status {first["rc"]}'
    r, why = against_cold(w, cold, lambda: w.run(engine))
    if why:
        return why
    if r['restored']:
        return 'a restore of what came of another file'
    # The run that went cold wrote a checkpoint of its own, from what it wrote itself, and that one holds.
    r, why = against_cold(w, cold, lambda: w.run(engine))
    if why:
        return why
    return None if r['restored'] else 'no restore after all that'


def sec_own_file_changed_before_it_was_read(w, engine, cold):
    return _changed_before_it_was_read(w, engine, cold, '\\def\\fromgen{another}\n')


def sec_own_file_changed_before_its_size_was_read(w, engine, cold):
    return _changed_before_it_was_read(w, engine, cold, 'something longer than what the run wrote\n')


def sec_two_directories_at_once(w, engine, cold):
    """The very same file in two directories, at once, with one cache."""
    other = World(os.path.join(w.base, 'second'), {'files': {}})
    other.cache = w.cache
    other.write('main.tex', w.read('main.tex'))
    a = w.argv(engine)
    b = other.argv(engine)
    for _ in range(3):
        ps = [subprocess.Popen(x, cwd=y.cwd, env=y.run_env(), stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL) for x, y in [(a, w), (b, other)] * 3]
        for p in ps:
            p.wait(timeout=300)
    if other.run(engine)['rc'] != 0:
        return 'the second directory failed'
    r, why = against_cold(w, cold, lambda: w.run(engine))
    return why


def sec_truncated_while_mapped(w, engine, cold):
    """A checkpoint cut short from outside while a run has it mapped: SIGBUS at the next page it touches.  An exit with a message, not a bare signal."""
    w.env['CK_TO_CUT'] = '/nonexistent/none'
    w.run(engine)
    cks = ck_files(w)
    if not cks:
        return 'no checkpoint'
    w.env['CK_TO_CUT'] = cks[0]
    r = w.run(engine)
    if r['rc'] < 0:
        return f'killed by signal {-r["rc"]}'
    if not r['restored']:
        return 'no restore'
    if r['rc'] != 0 and b'cut short' not in r['terminal']:
        return f'exit status {r["rc"]} with no word of why: {r["terminal"][-200:]!r}'
    return None


def _zoneinfo_free_tcx():
    src = subprocess.run(['kpsewhich', '-format=web2c files', 'cp227.tcx'], capture_output=True, text=True).stdout.strip()
    with open(src, 'rb') as f:
        return f.read()


def sec_map_tables_damaged(w, engine, cold):
    '''What a restored image needs of the kept tables of the default map file is gone: the entry is parsed for again, with nothing said.'''
    w.write('main.tex', doc(r'\font\x=cmss10 \pdfincludechars\x{Q}', r'{\x Old} \font\y=cmss12 {\y Another font}' + '\n'))
    w.run(engine)
    time_left = [n for n in os.listdir(os.path.join(w.cache, 'rstex')) if n.startswith('map')]
    if not time_left:
        return 'no tables were kept'
    tables = os.path.join(w.cache, 'rstex', time_left[0])
    r, why = against_cold(w, cold, lambda: w.run(engine))
    if why or not r['restored']:
        return f'as they are: {why or "no restore"}'
    with open(tables, 'r+b') as f:
        f.seek(os.path.getsize(tables) // 2)
        b = f.read(1)
        f.seek(-1, 1)
        f.write(bytes([b[0] ^ 1]))
    r, why = against_cold(w, cold, lambda: w.run(engine))
    if why or not r['restored']:
        return f'flipped: {why or "no restore"}'
    os.unlink(tables)
    r, why = against_cold(w, cold, lambda: w.run(engine))
    if why or not r['restored']:
        return f'removed: {why or "no restore"}'
    return None


def sec_date_gets_wider(w, engine, cold):
    '''The first line of the log has just room for the date on the 9th, and not on the 10th, when TeX wraps it.'''
    w.env['SOURCE_DATE_EPOCH'] = '1699488000'    # 9 November 2023, 00:00
    name = 'format-with-a-rather-long-name'
    w.format_extra = ''
    w.rebuild_format('first', engine=cold, name=name)
    w.env['TEXFORMATS'] = w.path('fmts') + ':'
    w.args = [f'-fmt={name}']
    c = w.run(cold, cache=os.path.join(w.base, 'cold-cache'))
    with open(os.path.join(w.outdir, 'main.log'), 'rb') as f:
        first = f.readline().rstrip(b'\n')
    width = len(first) - first.index(b' (preloaded') - 3     # TeX does not count the month, which goes out by wlog
    if width < 60:
        return f'the test cannot set max_print_line to {width}'
    # One column to spare today: tomorrow it reaches the end, and TeX breaks the line there.
    w.env['max_print_line'] = str(width + 1)
    w.run(engine)
    w.clock(86400)
    r, why = against_cold(w, cold, lambda: w.run(engine))
    return why


def sec_file_size_limit(w, engine, cold):
    '''A limit on file size, such as a sandbox has, under a cache file's size and over all the document writes.  No cache file is worth SIGXFSZ, which takes the whole run with it.'''
    def limited():
        w.fsize_kb = 2048       # of sh's 512-byte blocks
        try:
            return w.run(engine)
        finally:
            w.fsize_kb = None
    for k in (1, 2):
        r, why = against_cold(w, cold, limited)
        if why:
            return f'run {k}: {why}'
    return None


def sec_format_with_an_ampersand(w, engine, cold):
    '''&name as the first argument: the format is chosen in the middle of the run, with the program name that the option's absence leaves.'''
    w.format_extra = ''
    w.rebuild_format('first', engine=cold, name='one')
    w.env['TEXFORMATS'] = w.path('fmts') + ':'
    w.write('main.tex', doc('', r'\FORMATMARK. Old body.' + '\n'))
    argv = lambda e: list(e) + ['-interaction=batchmode', f'-output-directory={w.outdir}', '&one main.tex']
    w.run(engine, argv=argv(engine))
    r = w.run(engine, argv=argv(engine))
    if not r['restored']:
        return 'no restore'
    _body_edit(w)
    w.save('before-check')
    r = w.run(engine, argv=argv(engine))
    out = w.outputs()
    w.restore('before-check')
    c = w.run(cold, cache=os.path.join(w.base, 'cold-cache'), argv=argv(cold))
    return f'exit status {r["rc"]}, cold {c["rc"]}' if r['rc'] != c['rc'] else judge(out, w.outputs(), w.job, [w.base])


def sec_cache_name_is_a_file(w, engine, cold):
    '''Where the cache directory would go there is a file: quietly no cache.'''
    with open(os.path.join(w.cache, 'rstex'), 'w') as f:
        f.write('in the way')
    w.run(engine)
    r, why = against_cold(w, cold, lambda: w.run(engine))
    return 'a restore with no cache' if r['restored'] else why


def sec_parents_of_the_cache_are_made(w, engine, cold):
    '''XDG_CACHE_HOME that does not exist yet.'''
    w.cache = os.path.join(w.base, 'not', 'yet', 'here')
    r, cks = warm_up(w, engine)
    if not r['restored'] or not cks:
        return 'no restore'
    mode = os.stat(os.path.join(w.cache, 'rstex')).st_mode & 0o777
    return None if mode == 0o700 else f'mode {mode:o}'


def sec_kept_tables_are_private_too(w, engine, cold):
    '''The index of the tree, the tree's name and the tables of the default map: a file of them that others can write, or that is a symbolic link, is not taken, and the next run has made its own.'''
    w.write('main.tex', doc(r'\font\x=cmss10 \pdfincludechars\x{Q}', r'{\x Old}' + '\n'))
    w.run(engine)
    root = os.path.join(w.cache, 'rstex')
    kept = [n for n in os.listdir(root) if n.startswith(('kpse-index', 'kpse-tree', 'map')) and os.path.isfile(os.path.join(root, n))]
    if len(kept) < 3:
        return f'only these were kept: {kept}'
    for how in ('writable', 'symlink'):
        for n in kept:
            q = os.path.join(root, n)
            if how == 'writable':
                os.chmod(q, 0o666)
            else:
                shutil.move(q, q + '.moved')
                os.symlink(q + '.moved', q)
        # A cold run: a restored one finds the map's tables again by parsing, and has no reason to publish them.
        for c in ck_files(w):
            os.unlink(c)
        r, why = against_cold(w, cold, lambda: w.run(engine))
        if why:
            return f'{how}: {why}'
        for n in kept:
            st = os.lstat(os.path.join(root, n))
            if st.st_mode & 0o777 != 0o600 or not os.path.isfile(os.path.join(root, n)) or os.path.islink(os.path.join(root, n)):
                return f'{how}: {n} has stayed, with mode {st.st_mode & 0o777:o}'
    return None


def sec_seen_table_is_private_too(w, engine, cold):
    '''The table that says a checkpoint need not be summed: one that others can write is not believed, and a symbolic link is neither believed nor written through.'''
    r, cks = warm_up(w, engine)
    if not r['restored'] or not cks:
        return 'no restore'
    seen = os.path.join(w.cache, 'rstex', 'checkpoints', 'seen')
    # The checkpoint has to be old enough for a stat to show any change: then one restore seals it, and the next does no sum.
    time.sleep(2.2)
    w.run(engine)
    if 'passed its sum' in w.run(engine)['trace']:
        return 'no seal, so this case tests nothing'
    if os.stat(seen).st_mode & 0o777 != 0o600:
        return f'the table has mode {os.stat(seen).st_mode & 0o777:o}'
    os.chmod(seen, 0o666)
    r, why = against_cold(w, cold, lambda: w.run(engine))
    if why:
        return f'writable: {why}'
    if 'passed its sum' not in r['trace']:
        return 'a table that others can write was believed'
    os.chmod(seen, 0o600)
    elsewhere = os.path.join(w.base, 'seen-elsewhere')
    shutil.move(seen, elsewhere)
    with open(elsewhere, 'rb') as f:
        before = f.read()
    os.symlink(elsewhere, seen)
    r, why = against_cold(w, cold, lambda: w.run(engine))
    if why:
        return f'symbolic link: {why}'
    if 'passed its sum' not in r['trace']:
        return 'a table behind a symbolic link was believed'
    with open(elsewhere, 'rb') as f:
        return None if f.read() == before and os.path.islink(seen) else 'written through a symbolic link'


def sec_deps_of_a_format_writable_by_others(w, engine, cold):
    '''The list of what a format was built from, that others can write: it does not count, and the format is built again.'''
    w.run(engine, argv=_raw_argv(w, engine))
    deps = [os.path.join(r_, n) for r_, _, ns in os.walk(w.cache) for n in ns if n.endswith('.deps')]
    if not deps:
        return 'no list was kept'
    os.chmod(deps[0], 0o666)
    r, why = against_cold(w, cold, lambda: w.run(engine, argv=_raw_argv(w, engine)))
    if why:
        return why
    return None if os.stat(deps[0]).st_mode & 0o777 == 0o600 else f'the list has stayed, with mode {os.stat(deps[0]).st_mode & 0o777:o}'


def sec_error_leaves_no_checkpoint(w, engine, cold):
    '''An error in the preamble: no file comes of it.'''
    w.write('main.tex', doc(r'\undefinedinthepreamble', 'Body.\n'))
    w.args = ['-interaction=nonstopmode']
    w.run(engine)
    r, why = against_cold(w, cold, lambda: w.run(engine))
    if ck_files(w):
        return f'a checkpoint: {ck_files(w)}'
    return why or (f'exit status {r["rc"]}, where an error gives 1' if r['rc'] != 1 else None)


def sec_invocation_name(w, engine, cold):
    '''The program by another name: a name in the messages, so another key.'''
    fmt = os.path.join(os.path.dirname(engine[0]), 'fmt', 'pdflatex')
    argv = lambda name: [os.path.join(w.base, name), f'-fmt={fmt}', '-progname=pdflatex', '-interaction=batchmode', f'-output-directory={w.outdir}', w.main]
    for name in ('first-name', 'second-name'):
        os.symlink(_raw_engine(engine), os.path.join(w.base, name))
    w.run(engine, argv=argv('first-name'))
    if not w.run(engine, argv=argv('first-name'))['restored']:
        return 'no restore to begin with'
    r = w.run(engine, argv=argv('second-name'))
    return 'restored under another name' if r['restored'] else None


def sec_first_line_names_the_format(w, engine, cold):
    '''%&name on the first line, with -parse-first-line: another name in the first line is another format.'''
    w.format_extra = ''
    w.rebuild_format('first', engine=cold, name='one')
    w.rebuild_format('second', engine=cold, name='two')
    w.env['TEXFORMATS'] = w.path('fmts') + ':'
    w.write('main.tex', '%&one\n' + doc('', r'\FORMATMARK' + '\n'))
    argv = lambda e: list(e) + ['-parse-first-line', '-interaction=batchmode', f'-output-directory={w.outdir}', w.main]
    w.run(engine, argv=argv(engine))
    r = w.run(engine, argv=argv(engine))
    if not r['restored']:
        return 'no restore to begin with'
    w.write('main.tex', '%&two\n' + doc('', r'\FORMATMARK' + '\n'))
    w.save('before-check')
    r = w.run(engine, argv=argv(engine))
    out = w.outputs()
    w.restore('before-check')
    c = w.run(cold, cache=os.path.join(w.base, 'cold-cache'), argv=argv(cold))
    return 'restored another format' if r['restored'] else judge(out, w.outputs(), w.job, [w.base])


def sec_ini_takes_no_checkpoint(w, engine, cold):
    '''-ini: never a checkpoint, whatever the file does.'''
    w.write('main.tex', r'\input latex.ltx ' + '\n' + r'\documentclass{article}\begin{document}Body.\end{document}' + '\n')
    p = w.run(engine, argv=[_raw_engine(engine), '-ini', '-interaction=batchmode', f'-output-directory={w.outdir}', w.main])
    return f'a checkpoint from -ini: {ck_files(w)}' if ck_files(w) or p['restored'] else None


def sec_snapshot_switched_off(w, engine, cold):
    '''RSTEX_NO_SNAPSHOT and --no-snapshot: a checkpoint that is there is not used, and none is written.'''
    _, cks = warm_up(w, engine)
    if not cks:
        return 'no checkpoint to begin with'
    when = os.stat(cks[0]).st_mtime_ns
    w.env['RSTEX_NO_SNAPSHOT'] = '1'
    r, why = against_cold(w, cold, lambda: w.run(engine))
    w.env.pop('RSTEX_NO_SNAPSHOT')
    if why or r['restored']:
        return f'with the variable: {why or "restored"}'
    r, why = against_cold(w, cold, lambda: w.run(list(engine) + ['--no-snapshot']))
    if why or r['restored']:
        return f'with the option: {why or "restored"}'
    w.write('main.tex', w.read('main.tex') + b'% Another preamble hash, for the next run\n')
    os.unlink(cks[0])
    w.env['RSTEX_NO_SNAPSHOT'] = '1'
    w.run(engine)
    w.env.pop('RSTEX_NO_SNAPSHOT')
    return f'a checkpoint with the snapshot off: {ck_files(w)}' if ck_files(w) else None


def sec_program_replaced(w, engine, cold):
    '''The program file is replaced, by the same bytes even: another stat, so another key.'''
    prog = os.path.join(w.base, 'rstex')
    shutil.copy2(_raw_engine(engine), prog)
    fmt = os.path.join(os.path.dirname(engine[0]), 'fmt', 'pdflatex')
    argv = [prog, f'-fmt={fmt}', '-progname=pdflatex', '-interaction=batchmode', f'-output-directory={w.outdir}', w.main]
    w.run(engine, argv=argv)
    if not w.run(engine, argv=argv)['restored']:
        return 'no restore to begin with'
    os.unlink(prog)
    shutil.copy2(_raw_engine(engine), prog)
    r, why = against_cold(w, cold, lambda: w.run(engine, argv=argv))
    return 'restored for a program that is another file' if r['restored'] else why


def sec_map_file_edited_with_tables_kept(w, engine, cold):
    '''The default map file in a directory of its own, its tables kept in the cache.  An edit that leaves its size and modification date: the stamp sees the change time.'''
    w.env['TEXFONTMAPS'] = w.path('maps') + ':'
    w.write('maps/pdftex.map', 'cmss10 CMSS10 <cmss10.pfb\ncmr12 CMR12 <cmr12.pfb\n')
    w.write('main.tex', doc(r'\font\x=cmss10 \pdfincludechars\x{Q}', r'{\x Old}' + '\n'))
    time.sleep(2.2)     # a map file younger than that does not get its tables kept
    w.run(engine)
    if not [n for n in os.listdir(os.path.join(w.cache, 'rstex')) if n.startswith('map')]:
        return 'no tables were kept'
    r, why = against_cold(w, cold, lambda: w.run(engine))
    if why or not r['restored']:
        return f'with the file as it was: {why or "no restore"}'
    w.write('maps/pdftex.map', 'cmss10 CMSS12 <cmss12.pfb\ncmr12 CMR12 <cmr12.pfb\n', keep_stat=True)
    r, why = against_cold(w, cold, lambda: w.run(engine))
    return 'restored past an edit of the map' if r['restored'] else why


def sec_no_space_left(w, engine, cold):
    '''A cache directory with no room: a small tmpfs in a namespace of our own, where that can be had.  Nothing comes of it but a cold run.'''
    if not shutil.which('unshare') or subprocess.run(['unshare', '-Urm', 'true'], capture_output=True).returncode != 0:
        return None
    mnt = os.path.join(w.base, 'small')
    os.mkdir(mnt)
    sh = f'mount -t tmpfs -o size=256k none {shlex.quote(mnt)} || exit 77; for i in 1 2 3; do ' + ' '.join(shlex.quote(a) for a in w.argv(engine)) + ' || exit $?; done; find {0} -name "*.tmp*" | grep . && exit 78; exit 0'.format(shlex.quote(mnt))
    env = w.run_env()
    env['XDG_CACHE_HOME'] = mnt
    p = subprocess.run(['unshare', '-Urm', 'sh', '-c', sh], cwd=w.cwd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=300)
    if p.returncode == 77:
        return None
    if p.returncode != 0:
        return f'exit status {p.returncode}: {p.stderr[-300:]!r}'
    out = w.outputs()
    w.save('before-check')
    c = w.run(cold, cache=os.path.join(w.base, 'cold-cache'))
    return judge(out, w.outputs(), w.job, [w.base])


def _raw_engine(engine):
    """The program itself, which finds or builds its own format as `rstex doc.tex` does."""
    p = os.path.join(os.path.dirname(engine[0]), 'bin', 'rstex')
    return p if os.path.exists(p) else engine[0]


def _raw_argv(w, engine):
    return [_raw_engine(engine), '-progname=pdflatex', '-interaction=batchmode', f'-output-directory={w.outdir}'] + w.args + [w.main]


def sec_autofmt_read_only_cache(w, engine, cold):
    """No format anywhere, and a cache that cannot be written: the document still compiles, as it does when the cache can."""
    os.chmod(w.cache, 0o555)
    try:
        r, why = against_cold(w, cold, lambda: w.run(engine, argv=_raw_argv(w, engine)))
    finally:
        os.chmod(w.cache, 0o755)
    left = [n for n in os.listdir(w.cwd) if n != 'main.tex'] + [n for n in os.listdir(w.outdir) if n.startswith('.')]
    return why or (f'left behind: {left}' if left else None)


def sec_autofmt_no_home(w, engine, cold):
    w.drop_env = ['HOME', 'XDG_CACHE_HOME']
    r, why = against_cold(w, cold, lambda: w.run(engine, argv=_raw_argv(w, engine)))
    left = [n for n in os.listdir(w.cwd) if n != 'main.tex'] + [n for n in os.listdir(w.outdir) if n.startswith('.')]
    return why or (f'left behind: {left}' if left else None)


def sec_autofmt_first_runs_race(w, engine, cold):
    """Four first runs at once in four directories: one builds and the rest wait, or build too."""
    worlds = []
    for k in range(4):
        x = World(os.path.join(w.base, f'd{k}'), {'files': {}})
        x.cache = w.cache
        x.write('main.tex', w.read('main.tex'))
        worlds.append(x)
    ps = [subprocess.Popen(_raw_argv(x, engine), cwd=x.cwd, env=x.run_env(), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT) for x in worlds]
    for p in ps:
        out, _ = p.communicate(timeout=600)
        if p.returncode != 0:
            return f'exit status {p.returncode}: {out[-300:]!r}'
    left = [n for r_, ds, _ in os.walk(w.cache) for n in ds if n.startswith('tmp-')]
    if left:
        return f'left behind: {left}'
    r, why = against_cold(w, cold, lambda: w.run(engine, argv=_raw_argv(w, engine)))
    return why


def sec_autofmt_ignores_user_trees(w, engine, cold):
    """A format is what the installation would have built: a user's tree does not get into it, or the next run's output would hang on who came first."""
    w.write('main.tex', doc('', r'\ifdefined\FMTLEAK Leak.\else Clean.\fi' + '\n'))
    w.write('tree/tex/latex/texsys.cfg', r'\gdef\FMTLEAK{yes}')
    time.sleep(2.2)     # a stamp too young to trust keeps a format from being kept
    w.env['TEXMFHOME'] = w.path('tree')
    w.run(engine, argv=_raw_argv(w, engine))
    w.env.pop('TEXMFHOME')
    r, why = against_cold(w, cold, lambda: w.run(engine, argv=_raw_argv(w, engine)))
    return why


def sec_autofmt_damaged_format(w, engine, cold):
    """A format in the cache changed in place, then cut short: seen by its stamp, and built again."""
    w.run(engine, argv=_raw_argv(w, engine))
    fmts = [os.path.join(r_, n) for r_, _, ns in os.walk(w.cache) for n in ns if n.endswith('.fmt')]
    if not fmts:
        return 'no format in the cache'
    with open(fmts[0], 'r+b') as f:
        f.seek(os.path.getsize(fmts[0]) // 2)
        b = f.read(1)
        f.seek(-1, 1)
        f.write(bytes([b[0] ^ 1]))
    r, why = against_cold(w, cold, lambda: w.run(engine, argv=_raw_argv(w, engine)))
    if why:
        return 'flipped: ' + why
    os.truncate(fmts[0], os.path.getsize(fmts[0]) // 2)
    r, why = against_cold(w, cold, lambda: w.run(engine, argv=_raw_argv(w, engine)))
    return 'cut: ' + why if why else None


def sec_relative_cache_names(w, engine, cold):
    """XDG_CACHE_HOME and HOME that are not absolute: the XDG specification says to ignore them, and a cache under the current directory would be whatever the document's directory holds."""
    for name, values in (('XDG_CACHE_HOME', {'XDG_CACHE_HOME': 'rel/cache'}), ('HOME', {'HOME': 'relhome'}), ('empty', {'HOME': ''})):
        env = w.run_env()
        env.pop('XDG_CACHE_HOME')
        env['HOME'] = os.path.join(w.base, 'home')
        env.update(values)
        p = subprocess.run(w.argv(engine), cwd=w.cwd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, timeout=300)
        made = [n for n in os.listdir(w.cwd) if n != 'main.tex']
        if made:
            return f'{name}: a cache under the current directory: {made}'
        if p.returncode != 0:
            return f'{name}: exit status {p.returncode}'
    return None


def sec_no_cache_at_all(w, engine, cold):
    w.drop_env = ['HOME', 'XDG_CACHE_HOME']
    w.run(engine)
    r, why = against_cold(w, cold, lambda: w.run(engine))
    return 'a restore with no cache at all' if r['restored'] else why


SECURITY_DOC = {'main.tex': doc(r'\usepackage{array}\def\V{x}', SHOW_V)}
CUT_DOC = {'main.tex': doc(r'\usepackage{array}', r'\immediate\write18{: > "$CK_TO_CUT"}\font\later=cmr17 \fontdimen7\later=1pt \expandafter\let\csname a name not seen yet\endcsname\relax {\later A paragraph long enough to touch the hyphenation trie, in a new font}\par' + '\n')}
SECURITY = [
    ('kept-tables-are-private-too', sec_kept_tables_are_private_too), ('seen-table-is-private-too', sec_seen_table_is_private_too), ('deps-of-a-format-writable-by-others', sec_deps_of_a_format_writable_by_others), ('error-leaves-no-checkpoint', sec_error_leaves_no_checkpoint), ('invocation-name', sec_invocation_name),
    ('format-with-an-ampersand', sec_format_with_an_ampersand), ('cache-name-is-a-file', sec_cache_name_is_a_file), ('parents-of-the-cache-are-made', sec_parents_of_the_cache_are_made),
    ('file-size-limit', sec_file_size_limit),
    ('first-line-names-the-format', sec_first_line_names_the_format), ('ini-takes-no-checkpoint', sec_ini_takes_no_checkpoint), ('snapshot-switched-off', sec_snapshot_switched_off),
    ('program-replaced', sec_program_replaced), ('map-file-edited-with-tables-kept', sec_map_file_edited_with_tables_kept), ('no-space-left', sec_no_space_left),
    ('map-tables-damaged', sec_map_tables_damaged), ('date-gets-wider', sec_date_gets_wider),
    ('dir-is-a-symlink', sec_dir_is_a_symlink), ('dir-writable-by-others', sec_dir_writable_by_others), ('dir-is-private', sec_dir_is_private), ('old-dir-made-private', sec_old_dir_made_private),
    ('file-writable-by-others', sec_ck_writable_by_others), ('file-is-a-symlink', sec_ck_is_a_symlink), ('file-is-a-fifo', sec_ck_is_a_fifo), ('file-is-a-directory', sec_ck_is_a_directory),
    ('foreign-config', sec_foreign_config), ('header-lengths-wrap', sec_header_lengths_wrap), ('spans-out-of-the-image', sec_spans_out_of_the_image), ('spans-reversed', sec_spans_reversed),
    ('bit-flips', sec_bit_flips), ('cuts', sec_cuts), ('stale-tmp-is-not-used', sec_stale_tmp_is_not_used), ('killed-in-mid-run', sec_killed_in_mid_run),
    ('racing', sec_racing), ('two-directories-at-once', sec_two_directories_at_once), ('relative-cache-names', sec_relative_cache_names), ('no-cache-at-all', sec_no_cache_at_all),
    ('autofmt-read-only-cache', sec_autofmt_read_only_cache), ('autofmt-no-home', sec_autofmt_no_home), ('autofmt-first-runs-race', sec_autofmt_first_runs_race),
    ('autofmt-ignores-user-trees', sec_autofmt_ignores_user_trees), ('autofmt-damaged-format', sec_autofmt_damaged_format),
]
CASES += [case('cache', cid, why=(fn.__doc__ or '').strip().split('\n')[0], visible=False, rstex_only=True, custom=fn, files=SECURITY_DOC) for cid, fn in SECURITY]
CASES.append(case('cache', 'truncated-while-mapped', why=sec_truncated_while_mapped.__doc__, visible=False, rstex_only=True, custom=sec_truncated_while_mapped, files=CUT_DOC, args=['-shell-escape']))
OWN_THEN_WAIT = r'''\newwrite\own \newcount\n
\immediate\openout\own=gen.tex \immediate\write\own{\string\def\string\fromgen{ours}}\immediate\closeout\own
\immediate\openout\own=gate.txt \immediate\closeout\own
\n=0 \loop\ifnum\n<400000 \advance\n1 \repeat
'''
CASES.append(case('own-writes', 'read-back-what-another-wrote', why=_changed_before_it_was_read.__doc__, visible=False, rstex_only=True, custom=sec_own_file_changed_before_it_was_read,
                  files={'main.tex': doc(OWN_THEN_WAIT + r'\input{gen.tex}\typeout{SAW \fromgen}', 'What it read: \\fromgen.\n')}, no_outdir=True))
CASES.append(case('own-writes', 'size-of-what-another-wrote', why=_changed_before_it_was_read.__doc__, visible=False, rstex_only=True, custom=sec_own_file_changed_before_its_size_was_read,
                  files={'main.tex': doc(OWN_THEN_WAIT + r'\edef\thesize{\pdffilesize{gen.tex}}\typeout{SAW \ifnum\thesize>30 another\else ours\fi}', 'Its size: \\thesize.\n')}, no_outdir=True))


# The check run of these has to be a restore, whatever else the engine does: an edit of the body, and the clock moved, with no read of it in the preamble.
MUST_RESTORE = ['stack/at-input-in-before-hook', 'stack/at-input-wrapped-in-preamble', 'stack/hook-writes-the-log', 'stack/tracingassigns-at-at-input', 'read/body-edit', 'read/open-read-file-tail', 'aux/aux-stale-after-edit', 'volatile/date-not-read-then-used', 'volatile/time-not-read-then-used', 'volatile/year-assigned',
                'volatile/c-sys-patched', 'volatile/pgfmath-seed-used-in-body', 'volatile/creationdate-in-info', 'own-writes/openout-open-across-checkpoint', 'own-writes/recorder-file',
                'own-writes/read-back-own-file', 'stack/begin-document-in-input', 'stack/document-redefined-body-edit', 'restored/png-from-the-preamble', 'restored/jpeg-from-the-preamble', 'restored/pdf-from-the-preamble',
                'restored/tounicode-from-the-preamble', 'restored/dvi-output', 'restored/big-streams-from-the-preamble'] + [c['id'] for c in CASES if c.get('must') and not c.get('no_restore')]


def _damage_cache(self, how):
    for root, _, names in os.walk(self.cache):
        for n in names:
            p = os.path.join(root, n)
            if how == 'flip':
                d = bytearray(run_py.open_bytes(p))
                for k in range(0, len(d), 4096):
                    d[k] ^= 0x55
                with open(p, 'wb') as f:
                    f.write(d)
            elif how == 'truncate':
                os.truncate(p, os.path.getsize(p) // 2)
            elif how == 'empty':
                os.truncate(p, 0)
    if how == 'readonly':
        os.chmod(self.cache, 0o555)


def _rebuild_format(self, mark, engine=None, where='fmts', name='custom'):
    engine = engine or self.format_engine
    d = self.path(where)
    os.makedirs(d, exist_ok=True)
    env = {k: os.environ[k] for k in ('PATH', 'HOME') if k in os.environ}
    env.update(run_py.NO_MKTEX)
    env.update(run_py.HERMETIC)
    env.update(self.env)
    env = {k: v for k, v in env.items() if v is not None}
    # INITEX has no brace catcodes yet, and latex.ltx ends in \\dump, so the mark has to go first and bring its own braces.
    with open(os.path.join(d, 'mark.ini'), 'w') as f:
        f.write(f'\\catcode`\\{{=1 \\catcode`\\}}=2 \\def\\FORMATMARK{{{mark}}}{getattr(self, "format_extra", "")}\n\\input pdflatex.ini\n')
    p = subprocess.run(list(engine) + ['-ini', f'-jobname={name}', '-interaction=batchmode', f'-output-directory={d}', '*mark.ini'],
                       cwd=d, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    if not os.path.isfile(os.path.join(d, name + '.fmt')):
        raise RuntimeError(f'format build failed: {p.stdout[-400:]!r}')


World.damage_cache = _damage_cache
World.rebuild_format = _rebuild_format


def substitute(value, here):
    return value.replace('@HERE@', here) if isinstance(value, str) else value


def run_case(c, engine, cold, args):
    base = os.path.join(args.root, re.sub(r'[^A-Za-z0-9._-]+', '_', c['id']))
    shutil.rmtree(base, ignore_errors=True)
    os.makedirs(base)
    res = {'id': c['id'], 'rule': c['rule'], 'visible': c['visible'], 'why': c['why']}
    w = World(base, {**c, 'files': {}})
    for name, content in c['files'].items():
        w.write(name, content.replace('@HERE@', w.src) if isinstance(content, str) else content)
    w.env = {k: substitute(v, w.src) for k, v in w.env.items()}
    w.first_line = c.get('first_line')
    w.format_engine = cold
    if c.get('cwd'):
        w.cwd = w.path(c['cwd'])
    if c.get('setup'):
        c['setup'](w)
    if c.get('custom'):
        try:
            why = c['custom'](w, engine, cold)
            res.update(status='FAIL', detail=why) if why else res.update(status='pass', detail=None)
        except Exception as ex:
            res.update(status='ERROR', detail=f'{type(ex).__name__}: {ex}')
        force_rmtree(base)
        return res
    if c.get('custom_format'):
        w.format_extra = c.get('format_extra', '')
        w.rebuild_format(c['custom_format'], engine=cold)
        w.env['TEXFORMATS'] = w.path('fmts-first') + ':' + w.path('fmts') + ':'
        w.args.append('-fmt=custom')
    try:
        w.run(engine)
        second = w.run(engine)
        res['warm_rc'] = second['rc']
        res['second_restored'] = second['restored']
        w.save('w1')
        if c['change']:
            c['change'](w)
        w.save('w2')
        check = w.run(engine)
        res['check_restored'] = check['restored']
        res['check_trace'] = check['trace'][-400:]
        res['second_trace'] = second['trace'][-400:]
        check_out = w.outputs()
        w.restore('w2')
        cold_after = w.run(cold, cache=os.path.join(base, 'cold-cache'))
        cold_out = w.outputs()
        workdirs = [base]
        diff = None
        if check['rc'] != cold_after['rc']:
            diff = f'exit status: check {check["rc"]}, cold {cold_after["rc"]}'
        diff = diff or judge(check_out, cold_out, w.job, workdirs, args.pdf_only)
        # An engine judged against itself shows the terminal the same, too.
        if not diff and engine[0] == cold[0] and check['terminal'] != cold_after['terminal']:
            diff = f'terminal: check `{run_py.clip(check["terminal"][-300:])}` | cold `{run_py.clip(cold_after["terminal"][-300:])}`'
        if not diff and c.get('no_restore') and (second['restored'] or check['restored']):
            diff = 'restored, and this case says no checkpoint may be taken'
        # The reference: the same cold engine on the world before the change.
        w.restore('w1')
        fresh = World.__new__(World)
        fresh.__dict__.update(w.__dict__)
        fresh.env = {'SOURCE_DATE_EPOCH': str(EPOCH), 'FORCE_SOURCE_DATE': '1', 'TZ': 'UTC', 'LC_ALL': 'C', 'LANG': 'C', **{k: substitute(v, w.src) for k, v in c.get('env', {}).items()}}
        if c.get('custom_format'):
            fresh.env['TEXFORMATS'] = w.path('fmts-first') + ':' + w.path('fmts') + ':'
        fresh.job = c.get('job') or os.path.splitext(os.path.basename(c.get('main', 'main.tex')))[0]
        fresh.args = list(c.get('args', [])) + (['-fmt=custom'] if c.get('custom_format') else [])
        fresh.outdir = w.src if c.get('no_outdir') else w.out
        fresh.cwd = w.path(c['cwd']) if c.get('cwd') else w.src
        fresh.first_line = c.get('first_line')
        fresh.stdin = c.get('stdin')
        cold_before = fresh.run(cold, cache=os.path.join(base, 'cold-cache'))
        before_out = fresh.outputs()
        changed = None
        if cold_before['rc'] != cold_after['rc']:
            changed = f'exit status {cold_before["rc"]} -> {cold_after["rc"]}'
        changed = changed or judge(before_out, cold_out, fresh.job if fresh.job == w.job else w.job, workdirs, False) or (None if fresh.job == w.job else f'job name {fresh.job} -> {w.job}')
        res['cold_changed'] = changed
        res['cold_rc'] = cold_after['rc']
        if diff:
            res.update(status='FAIL', detail=diff)
        elif c['visible'] and not changed and c['rule'] != 'cache':
            res.update(status='VACUOUS', detail='the change is not visible to a cold compile, so this cannot catch a stale snapshot')
        else:
            res.update(status='pass', detail=None)
    except Exception as ex:
        res.update(status='ERROR', detail=f'{type(ex).__name__}: {ex}')
    finally:
        try:
            os.chmod(w.cache, 0o755)
        except OSError:
            pass
        if not args.keep:
            force_rmtree(base)
    return res


def run_group(engine, cold, args, label):
    rstex = os.path.basename(engine[0]).startswith('rstex')
    cases = [c for c in CASES if (not args.filter or re.search(args.filter, c['id'])) and (rstex or not c.get('rstex_only'))]
    # Of this process: tests/gate.sh runs this, and two gates at once must not share a case's directory.
    args.root = os.path.join(ROOT, 'w', f'{label}-{os.getpid()}')
    os.makedirs(args.root, exist_ok=True)
    mutated_tfm()
    with cf.ThreadPoolExecutor(args.jobs) as ex:
        results = list(ex.map(lambda c: run_case(c, engine, cold, args), cases))
    if not args.keep:
        force_rmtree(args.root)
    return results


def print_results(results, label, verbose=True):
    by = {}
    for r in results:
        by.setdefault(r['status'], []).append(r)
    print(f'{label}: ' + ', '.join(f'{k} {len(v)}' for k, v in sorted(by.items())) + f', of {len(results)} cases')
    # A case only tests a restore if there was one: how many second runs and check runs really restored, which an engine with no cache never does.
    print(f'  restored: second run {sum(1 for r in results if r.get("second_restored"))}, check run {sum(1 for r in results if r.get("check_restored"))}')
    per_rule = {}
    for r in results:
        per_rule.setdefault(r['rule'], {}).setdefault(r['status'], 0)
        per_rule[r['rule']][r['status']] += 1
    for rule, row in per_rule.items():
        print(f'  {rule:<12} ' + ', '.join(f'{k} {v}' for k, v in sorted(row.items())))
    if verbose:
        for r in results:
            if r['status'] != 'pass':
                print(f'  {r["status"]:<8} {r["id"]}: {(r["detail"] or "")[:230]}')


def mutant_wrapper(kind):
    d = os.path.join(ROOT, 'wrappers')
    os.makedirs(d, exist_ok=True)
    p = os.path.join(d, kind)
    with open(p, 'w') as f:
        f.write(f'#!/bin/bash\nexec python3 -I {json.dumps(os.path.join(HERE, "mutants.py"))} {kind} "$@"\n')
    os.chmod(p, 0o755)
    return p


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--engine', help='engine under test; a string split like a shell line')
    ap.add_argument('--cold-engine', default='pdflatex')
    ap.add_argument('--filter')
    ap.add_argument('-j', '--jobs', type=int, default=6)
    ap.add_argument('--pdf-only', action='store_true', help='judge the PDF and exit status only')
    ap.add_argument('--out', default=os.path.join(ROOT, 'results.json'))
    ap.add_argument('--keep', action='store_true')
    ap.add_argument('--list', action='store_true')
    ap.add_argument('--check-mutants', action='store_true')
    args = ap.parse_args()
    if args.list:
        for c in CASES:
            print(f'{c["id"]:<46} {"visible" if c["visible"] else "invisible":<10} {c["why"]}')
        print(f'{len(CASES)} cases')
        return 0
    cold = shlex.split(args.cold_engine)
    os.makedirs(ROOT, exist_ok=True)
    if args.check_mutants:
        results = {}
        results['pdflatex'] = run_group(['pdflatex'], cold, args, 'pdflatex')
        print_results(results['pdflatex'], 'pdflatex (no cache at all): every case must pass, and none may be vacuous')
        args.pdf_only = False
        results['stale'] = run_group([mutant_wrapper('stale')], cold, args, 'stale')
        print_results(results['stale'], 'stale: replays its first run for ever. Every case with a visible change must fail', verbose=False)
        # Replaying the first pass is right by accident when the world goes back to how it was before the first pass.
        # And its copy of the first run's log cannot go over a log made read-only, which comes to what a cold run does.
        by_accident = {'aux/aux-deleted', 'own-writes/log-now-read-only'}
        missed = [r['id'] for r in results['stale'] if r['visible'] and r['status'] == 'pass' and r['rule'] != 'cache' and r['id'] not in by_accident]
        print(f'  visible-change cases the stale engine passes: {len(missed)}: {missed}')
        args.pdf_only = True
        results['naive'] = run_group([mutant_wrapper('naive')], cold, args, 'naive')
        print_results(results['naive'], 'naive: a real preamble dump by mylatexformat, keyed on the text of the preamble only, judged on the PDF and exit status')
        with open(args.out, 'w') as f:
            json.dump(results, f, indent=1)
        bad = [r['id'] for r in results['pdflatex'] if r['status'] != 'pass']
        print(f'{"OK" if not bad and not missed else "NOT OK"}: pdflatex not passing: {len(bad)}; visible changes the stale engine survives: {len(missed)}')
        return 1 if bad or missed else 0
    if not args.engine:
        ap.error('--engine is required')
    results = run_group(shlex.split(args.engine), cold, args, 'engine')
    print_results(results, f'snapshot soundness, engine {args.engine}')
    with open(args.out, 'w') as f:
        json.dump(results, f, indent=1)
    # An engine that never restores equals a cold compile in every case, and passes for nothing.  These are the ones a snapshot exists for.
    by_id = {r['id']: r for r in results}
    must = [i for i in MUST_RESTORE if i in by_id] if 'pdflatex' not in args.engine else []
    unused = [i for i in must if not by_id[i].get('check_restored')]
    for i in unused:
        print(f'  NOT RESTORED {i}: the check run was a cold one, so this case tested nothing')
    ok = all(r['status'] == 'pass' for r in results) and not unused
    print(f'{"PASS" if ok else "FAIL"} snapshot: {sum(r["status"] == "pass" for r in results)} / {len(results)}, and {len(must) - len(unused)} of {len(must)} that must restore did; details: {args.out}')
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main())
