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

Pass: the check run equals the cold run on W2', in the PDF bytes, the normalized log, every side file and the exit status.
A case also fails as VACUOUS if the cold run on W1' equals the one on W2' although the case says the change is visible: such a test cannot catch
a stale snapshot, and passes for nothing.  Cases that say the change is invisible (touching a file) assert only the equality.

The cold engine defaults to the system pdflatex.  For rstex: --cold-engine "target/release/rstex --no-snapshot".  Every path stays the same between
the check run and the cold run, so logs compare without masks.  With --engine pdflatex there is no cache at all, so all cases must pass, and a
failure is a broken test.  Everything runs under target/corpus/snapshot/.  Nothing here writes to the user's documents.
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
        shutil.rmtree(d, ignore_errors=True)
        os.makedirs(d)
        for name in ('src', 'out'):
            shutil.copytree(os.path.join(self.base, name), os.path.join(d, name), symlinks=True, copy_function=shutil.copy2)
        return d

    def restore(self, tag):
        d = os.path.join(self.base, tag)
        for name in ('src', 'out'):
            shutil.rmtree(os.path.join(self.base, name), ignore_errors=True)
            shutil.copytree(os.path.join(d, name), os.path.join(self.base, name), symlinks=True, copy_function=shutil.copy2)

    def run(self, engine, cache=None):
        env = {k: os.environ[k] for k in ('PATH', 'HOME') if k in os.environ}
        env.update(run_py.NO_MKTEX)
        env.update(run_py.HERMETIC)
        env.update(self.env)
        env = {k: v for k, v in env.items() if v is not None}
        env['XDG_CACHE_HOME'] = cache or self.cache
        env['RSTEX_CHECKPOINT_TRACE'] = '1'     # an engine with no such thing ignores it; rstex says on stderr what it did
        env['RSTEX_CHECKPOINT_POLICY'] = 'always'   # the second run restores, however small the preamble: policy.py tests what a run does by itself
        fmt = [a for a in self.args if a.startswith(('-fmt=', '-ini'))]
        interaction = [a for a in self.args if a.startswith('-interaction')]
        tail = self.first_line if self.first_line else self.main
        argv = list(engine)
        if not fmt:
            argv.append('-fmt=pdflatex')
        if not interaction:
            argv.append('-interaction=batchmode')
        argv += [f'-output-directory={self.outdir}'] + self.args + [tail]
        p = subprocess.run(argv, cwd=self.cwd, env=env, input=(self.stdin or '').encode(), stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=300)
        return {'rc': p.returncode, 'stdout': p.stdout[-2000:].decode('latin-1'), 'restored': b'rstex: checkpoint: restored' in p.stdout}

    def outputs(self):
        """Everything the run left in its output directory."""
        res = {}
        for root, _, names in os.walk(self.outdir):
            for n in names:
                p = os.path.join(root, n)
                res[os.path.relpath(p, self.outdir)] = run_py.open_bytes(p)
        return res


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


def tiny_png(width, height, shade):
    def chunk(t, d):
        return struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d))
    raw = b''.join(b'\0' + bytes([shade]) * width for _ in range(height))
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 0, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b'')


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
         env={'TEXMFHOME': '@HERE@/tree'}, change=lambda w: (w.edit('tree/tex/latex/pk.sty', 'old', 'newer'), w.touch('tree/tex/latex/pk.sty'))),
    case('read', 'tds-tree-same-stat', why='the same edit keeping size, mtime and inode.  The rule of section 7 stats TDS files by size, mtime and inode, so a stat-only key misses this; ctime does not', files={'main.tex': doc(r'\usepackage{pk}', SHOW_V), 'tree/ls-R': '% ls-R -- filename database for kpathsea; do not change this line.\n./:\ntex\n\n./tex:\nlatex\n\n./tex/latex:\npk.sty\n', 'tree/tex/latex/pk.sty': r'\def\V{old}'},
         env={'TEXMFHOME': '@HERE@/tree'}, change=lambda w: w.edit('tree/tex/latex/pk.sty', 'old', 'new', keep_stat=True)),
    case('read', 'tds-tree-new-file', why='ls-R rebuilt with a new file that shadows a later tree', files={'main.tex': doc(r'\IfFileExists{newpk.sty}{\usepackage{newpk}}{\def\V{absent}}', SHOW_V), 'tree/ls-R': '% ls-R -- filename database for kpathsea; do not change this line.\n./:\ntex\n\n./tex:\nlatex\n\n./tex/latex:\n'},
         env={'TEXMFHOME': '@HERE@/tree'}, change=lambda w: (w.write('tree/tex/latex/newpk.sty', r'\def\V{present}'), w.write('tree/ls-R', '% ls-R -- filename database for kpathsea; do not change this line.\n./:\ntex\n\n./tex:\nlatex\n\n./tex/latex:\nnewpk.sty\n'))),
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
    case('stack', 'tracingassigns-at-document', why='with \\tracingassigns on nothing new is armed, and the checkpoint is taken at \\document', files={'main.tex': doc(r'\tracingassigns=1 ', 'Old body.\n')}, change=lambda w: w.edit('main.tex', 'Old body.', 'New body.')),
    case('stack', 'hook-writes-the-log', why='a message from the hook is in the journal that a restore plays back', files={'main.tex': doc(r'\AddToHook{begindocument/before}{\typeout{Said by the hook}}', 'Old body.\n')}, change=lambda w: w.edit('main.tex', 'Old body.', 'New body.')),
    case('stack', 'crlf-line-endings', why='the offset counts the bytes on disk', files={'main.tex': ARTICLE.replace('\n', '\r\n') + r'\begin{document}' + '\r\n' + 'Old body.\r\n' + r'\end{document}' + '\r\n'}, change=lambda w: w.edit('main.tex', 'Old body.', 'New body.')),
    case('stack', 'no-final-newline', files={'main.tex': ARTICLE + r'\input{pre}' + '\n' + 'Old body.\n' + r'\end{document}' + '\n', 'pre.tex': r'\begin{document}'}, change=lambda w: w.edit('main.tex', 'Old body.', 'New body.')),
    case('stack', 'endinput-in-preamble', why='\\endinput leaves the rest of the file unread; an edit after it changes nothing', visible=False, files={'main.tex': ARTICLE + r'\input{pre}' + '\n' + r'\begin{document}Body.\end{document}' + '\n', 'pre.tex': r'\def\V{x}\endinput' + '\n' + 'unread text\n'}, change=lambda w: w.edit('pre.tex', 'unread text', 'other text')),
    # ------------------------------------------------------------------ a restore that has to bring back what the preamble set up, with the body edited
    case('restored', 'png-from-the-preamble', why='the image table is in the image, what the C keeps behind it is read again at the first need', files={'main.tex': doc(r'\pdfximage{img.png}\setbox0=\hbox{\pdfrefximage\pdflastximage}', r'Old body. \box0' + '\n'), 'img.png': tiny_png(8, 8, 200)}, change=lambda w: w.edit('main.tex', 'Old body.', 'New body.')),
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


# The check run of these has to be a restore, whatever else the engine does: an edit of the body, and the clock moved, with no read of it in the preamble.
MUST_RESTORE = ['stack/at-input-in-before-hook', 'stack/at-input-wrapped-in-preamble', 'stack/hook-writes-the-log', 'stack/tracingassigns-at-document', 'read/body-edit', 'read/open-read-file-tail', 'aux/aux-stale-after-edit', 'volatile/date-not-read-then-used', 'volatile/time-not-read-then-used', 'volatile/year-assigned',
                'volatile/c-sys-patched', 'volatile/pgfmath-seed-used-in-body', 'volatile/creationdate-in-info', 'own-writes/openout-open-across-checkpoint', 'own-writes/recorder-file',
                'own-writes/read-back-own-file', 'stack/begin-document-in-input', 'stack/document-redefined-body-edit', 'restored/png-from-the-preamble', 'restored/jpeg-from-the-preamble', 'restored/pdf-from-the-preamble',
                'restored/tounicode-from-the-preamble', 'restored/dvi-output', 'restored/big-streams-from-the-preamble']


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


def _rebuild_format(self, mark, engine=None):
    engine = engine or self.format_engine
    d = self.path('fmts')
    os.makedirs(d, exist_ok=True)
    env = {k: os.environ[k] for k in ('PATH', 'HOME') if k in os.environ}
    env.update(run_py.NO_MKTEX)
    env.update(run_py.HERMETIC)
    env.update(self.env)
    env = {k: v for k, v in env.items() if v is not None}
    # INITEX has no brace catcodes yet, and latex.ltx ends in \\dump, so the mark has to go first and bring its own braces.
    with open(os.path.join(d, 'mark.ini'), 'w') as f:
        f.write(f'\\catcode`\\{{=1 \\catcode`\\}}=2 \\def\\FORMATMARK{{{mark}}}{getattr(self, "format_extra", "")}\n\\input pdflatex.ini\n')
    p = subprocess.run(list(engine) + ['-ini', '-jobname=custom', '-interaction=batchmode', f'-output-directory={d}', '*mark.ini'],
                       cwd=d, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    if not os.path.isfile(os.path.join(d, 'custom.fmt')):
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
    if c.get('custom_format'):
        w.format_extra = c.get('format_extra', '')
        w.rebuild_format(c['custom_format'], engine=cold)
        w.env['TEXFORMATS'] = w.path('fmts') + ':'
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
        check_out = w.outputs()
        w.restore('w2')
        cold_after = w.run(cold, cache=os.path.join(base, 'cold-cache'))
        cold_out = w.outputs()
        workdirs = [base]
        diff = None
        if check['rc'] != cold_after['rc']:
            diff = f'exit status: check {check["rc"]}, cold {cold_after["rc"]}'
        diff = diff or judge(check_out, cold_out, w.job, workdirs, args.pdf_only)
        # The reference: the same cold engine on the world before the change.
        w.restore('w1')
        fresh = World.__new__(World)
        fresh.__dict__.update(w.__dict__)
        fresh.env = {'SOURCE_DATE_EPOCH': str(EPOCH), 'FORCE_SOURCE_DATE': '1', 'TZ': 'UTC', 'LC_ALL': 'C', 'LANG': 'C', **{k: substitute(v, w.src) for k, v in c.get('env', {}).items()}}
        if c.get('custom_format'):
            fresh.env['TEXFORMATS'] = w.path('fmts') + ':'
        fresh.job = c.get('job') or os.path.splitext(os.path.basename(c.get('main', 'main.tex')))[0]
        fresh.args = list(c.get('args', [])) + (['-fmt=custom'] if c.get('custom_format') else [])
        fresh.outdir = w.out
        fresh.cwd = w.src
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
            shutil.rmtree(base, ignore_errors=True)
    return res


def run_group(engine, cold, args, label):
    cases = [c for c in CASES if not args.filter or re.search(args.filter, c['id'])]
    args.root = os.path.join(ROOT, 'w', label)
    os.makedirs(args.root, exist_ok=True)
    mutated_tfm()
    with cf.ThreadPoolExecutor(args.jobs) as ex:
        results = list(ex.map(lambda c: run_case(c, engine, cold, args), cases))
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
        by_accident = {'aux/aux-deleted'}
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
