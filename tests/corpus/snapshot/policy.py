#!/usr/bin/env -S python3 -I
r"""When does a run write a checkpoint, and what does the cache come to?  DESIGN section 7, "whether to write one".  run_snapshot.py asks if a checkpoint is sound, with
RSTEX_CHECKPOINT_POLICY=always; this asks what the program does by itself, and that it never shows in the output.

    python3 -I tests/corpus/snapshot/policy.py --engine ENGINE

ENGINE is anything that takes -fmt=pdflatex, such as a wrapper that finds formats built by the same binary (tests/corpus/README.md, "How a candidate is invoked").  Every run of a case is compared with --no-snapshot on the same world: PDF, log and
exit status.  Everything runs under target/corpus/snapshot/policy.  Exit status 0 if all cases pass.
"""
import argparse
import glob
import os
import re
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
ROOT = os.path.join(REPO, 'target', 'corpus', 'snapshot', 'policy')
HEAD = r'\pdfcompresslevel=0 \pdfobjcompresslevel=0 \pdftrailerid{}' + '\n'
# Past the bound for writing at first sight, FIRST_SIGHT_BYTES of checkpoint.rs: a file to read in the preamble.
LONG_FILE = '% a comment that TeX has to read and throw away\n' * 6000
# Long by the clock, but it reads next to nothing, and the bound goes by what is read, so that a run does not decide by the load of the machine.
SLOW = r'\newcount\N \loop\advance\N by 1 \ifnum\N<300000 \repeat' + '\n'
# checkpoint.rs's RACY_NS, and a little.
RACY = 2.2


def doc(preamble='', body='Some text.'):
    return HEAD + r'\documentclass{article}' + '\n' + preamble + '\n' + r'\begin{document}' + '\n' + body + '\n' + r'\end{document}' + '\n'


class World:
    def __init__(self, engine, name):
        self.engine = engine
        self.base = os.path.join(ROOT, name)
        shutil.rmtree(self.base, ignore_errors=True)
        self.src = os.path.join(self.base, 'src')
        self.out = os.path.join(self.base, 'out')
        self.cache = os.path.join(self.base, 'cache')
        for d in (self.src, self.out, self.cache):
            os.makedirs(d)
        self.ck_dir = os.path.join(self.cache, 'rstex', 'checkpoints')
        self.failures = []

    def write(self, content, name='main.tex'):
        with open(os.path.join(self.src, name), 'w') as f:
            f.write(content)

    def checkpoints(self):
        return glob.glob(os.path.join(self.ck_dir, '*.ck'))

    def engine_run(self, extra, cache, policy=None):
        env = {k: os.environ[k] for k in ('PATH', 'HOME') if k in os.environ}
        env.update({'SOURCE_DATE_EPOCH': '1700000000', 'FORCE_SOURCE_DATE': '1', 'TZ': 'UTC', 'LC_ALL': 'C', 'LANG': 'C', 'XDG_CACHE_HOME': cache, 'RSTEX_CHECKPOINT_TRACE': '1'})
        if policy:
            env['RSTEX_CHECKPOINT_POLICY'] = policy
        argv = self.engine + extra + ['-fmt=pdflatex', '-interaction=batchmode', f'-output-directory={self.out}', 'main.tex']
        p = subprocess.run(argv, cwd=self.src, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        return p.returncode, p.stderr.decode('latin-1')

    def outputs(self):
        res = {}
        for name in ('main.pdf', 'main.log'):
            with open(os.path.join(self.out, name), 'rb') as f:
                res[name] = f.read()
        return res

    def run(self, policy=None):
        """A run, compared with a cold one from the same state of the output directory: what it said about the checkpoint, its first line."""
        state = {}
        for n in os.listdir(self.out):
            with open(os.path.join(self.out, n), 'rb') as f:
                state[n] = f.read()
        rc, err = self.engine_run([], self.cache, policy)
        got = self.outputs()
        for n in os.listdir(self.out):
            os.unlink(os.path.join(self.out, n))
        for n, d in state.items():
            with open(os.path.join(self.out, n), 'wb') as f:
                f.write(d)
        cold_rc, _ = self.engine_run(['--no-snapshot'], os.path.join(self.base, 'cold-cache'))
        cold = self.outputs()
        if (rc, got) != (cold_rc, cold):
            self.failures.append('a run differs from a cold one: ' + ', '.join(n for n in got if got[n] != cold[n]))
        return [line[len('rstex: checkpoint: '):] for line in err.splitlines() if line.startswith('rstex: checkpoint: ')]

    def expect(self, said, *wanted):
        """Each of `wanted` is part of a line the run said about the checkpoint."""
        for w in wanted:
            if not any(w in s for s in said):
                self.failures.append(f'expected {w!r}, said {said}')

    def check(self, ok, what):
        if not ok:
            self.failures.append(what)


def second_sight(w):
    """A small document: the first run leaves a stamp only, the second writes, the third restores, and in time the sum is no longer done."""
    w.write(doc())
    said = w.run()
    w.expect(said, 'none', 'first sight')
    w.check(not w.checkpoints(), 'the first run wrote a checkpoint')
    seen = os.path.join(w.ck_dir, 'seen')
    w.check(os.path.getsize(seen) == 65536, 'the table has its size')
    w.check(os.stat(seen).st_blocks * 512 <= 65536, 'the table is no larger on the disk')
    w.expect(w.run(), 'none', 'taken')
    w.check(len(w.checkpoints()) == 1, 'the second run wrote no checkpoint')
    w.expect(w.run(), 'restored', 'sum')
    time.sleep(RACY)
    w.expect(w.run(), 'restored', 'sum')
    said = w.run()
    w.expect(said, 'restored')
    w.check(not any('sum' in s for s in said), f'a sealed file had its sum done: {said}')


def body_edit_is_the_same_preamble(w):
    w.write(doc(body='First body.'))
    w.expect(w.run(), 'first sight')
    w.write(doc(body='Second body.'))
    w.expect(w.run(), 'taken')
    w.write(doc(body='Third body.'))
    w.expect(w.run(), 'restored')


def touch_is_the_same_preamble(w):
    w.write(doc())
    w.expect(w.run(), 'first sight')
    p = os.path.join(w.src, 'main.tex')
    st = os.stat(p)
    os.utime(p, ns=(st.st_atime_ns, st.st_mtime_ns + 3600 * 10**9))
    w.expect(w.run(), 'taken')


def edited_preamble_writes_nothing_until_it_stays(w):
    for k in range(3):
        w.write(doc(rf'\def\V{{value {k}}}'))
        w.expect(w.run(), 'first sight')
        w.check(not w.checkpoints(), f'a checkpoint for the preamble of run {k}')
    w.expect(w.run(), 'taken')
    w.expect(w.run(), 'restored')
    # What is on the disk then does not hold: a miss, and it stays a miss until the new preamble is seen twice.
    w.write(doc(r'\def\V{another value}'))
    w.expect(w.run(), 'does not hold', 'first sight')
    w.expect(w.run(), 'does not hold', 'taken')
    w.expect(w.run(), 'restored')


def long_preamble_writes_at_first_sight(w):
    w.write(LONG_FILE, 'long.tex')
    w.write(doc(r'\input{long}'))
    w.expect(w.run(), 'none', 'taken')
    w.check(len(w.checkpoints()) == 1, 'no checkpoint')
    w.expect(w.run(), 'restored')


def what_is_long_goes_by_what_is_read(w):
    """A long time spent on little that is read: one run later than it could have been, and never by what the machine happens to be doing."""
    w.write(doc(SLOW))
    w.expect(w.run(), 'first sight')
    w.expect(w.run(), 'taken')
    w.expect(w.run(), 'restored')
    # The rest of the main file is not read yet, and a long body is no reason.
    w.write(doc(SLOW, 'Body text.\n' * 40000))
    w.expect(w.run(), 'restored')


def forced_policies(w):
    w.write(doc())
    w.expect(w.run('always'), 'taken')
    w.expect(w.run('always'), 'restored')
    shutil.rmtree(w.ck_dir)
    w.write(LONG_FILE, 'long.tex')
    w.write(doc(r'\input{long}'))
    w.expect(w.run('second'), 'first sight')
    w.expect(w.run('second'), 'taken')


def sealed_file_changed(w):
    """A seal is for the file as a stat sees it.  A byte changed in a span that is mapped, and the times put back as far as they go, is a damaged image, and a cold run."""
    w.write(doc())
    w.expect(w.run('always'), 'taken')
    time.sleep(RACY)
    w.expect(w.run('always'), 'restored', 'sum')
    said = w.run('always')
    w.expect(said, 'restored')
    w.check(not any('sum' in s for s in said), f'not sealed: {said}')
    p = w.checkpoints()[0]
    st = os.stat(p)
    with open(p, 'r+b') as f:
        f.seek(st.st_size // 2)
        b = f.read(1)
        f.seek(st.st_size // 2)
        f.write(bytes([b[0] ^ 1]))
    os.utime(p, ns=(st.st_atime_ns, st.st_mtime_ns))
    said = w.run('always')
    w.expect(said, 'damaged image')


def old_files_go_when_one_is_published(w):
    """Nothing else looks at the directory: a run that publishes nothing leaves them."""
    os.makedirs(w.ck_dir)
    rstex = os.path.dirname(w.ck_dir)
    now = time.time()

    def old(path, days):
        with open(path, 'wb') as f:
            f.write(b'x' * 1000)
        os.utime(path, (now - days * 86400, now - days * 86400))
    old(os.path.join(w.ck_dir, 'aaaaaaaaaaaaaaaa.ck'), 40)
    old(os.path.join(w.ck_dir, 'bbbbbbbbbbbbbbbb.ck'), 20)
    old(os.path.join(w.ck_dir, 'cccccccccccccccc.ck.123.tmp'), 2)
    for k in range(12):
        old(os.path.join(rstex, f'kpse-index-{k:016x}'), 2 + k)
        old(os.path.join(rstex, f'map-{k:016x}'), 2 + k)
    w.write(doc())
    w.expect(w.run(), 'first sight')
    w.check(os.path.exists(os.path.join(w.ck_dir, 'aaaaaaaaaaaaaaaa.ck')), 'a run that published no checkpoint looked at the directory')
    w.expect(w.run(), 'taken')
    w.check(not os.path.exists(os.path.join(w.ck_dir, 'aaaaaaaaaaaaaaaa.ck')), 'a checkpoint not used for 40 days is still there')
    w.check(os.path.exists(os.path.join(w.ck_dir, 'bbbbbbbbbbbbbbbb.ck')), 'a checkpoint used 20 days ago is gone')
    w.check(not os.path.exists(os.path.join(w.ck_dir, 'cccccccccccccccc.ck.123.tmp')), 'a temporary file of a run that died is still there')
    w.check(len(glob.glob(os.path.join(rstex, 'kpse-index-*'))) <= 8, 'more than 8 indexes are kept')
    w.check(len(glob.glob(os.path.join(rstex, 'map-*'))) <= 8, 'more than 8 map tables are kept')
    w.check(os.path.exists(os.path.join(rstex, 'kpse-index-' + '0' * 16)), 'the newest of the old indexes is gone')


CASES = [second_sight, body_edit_is_the_same_preamble, touch_is_the_same_preamble, edited_preamble_writes_nothing_until_it_stays, long_preamble_writes_at_first_sight, what_is_long_goes_by_what_is_read,
         forced_policies, sealed_file_changed, old_files_go_when_one_is_published]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--engine', required=True)
    ap.add_argument('--filter')
    args = ap.parse_args()
    engine = [os.path.abspath(args.engine)]
    bad = 0
    cases = [c for c in CASES if not args.filter or re.search(args.filter, c.__name__)]
    for c in cases:
        w = World(engine, c.__name__)
        try:
            c(w)
        except (OSError, IndexError) as ex:
            w.failures.append(f'{type(ex).__name__}: {ex}')
        print(f'{"pass" if not w.failures else "FAIL"} {c.__name__}')
        for f in w.failures:
            print(f'     {f}')
        bad += bool(w.failures)
        if not w.failures:
            shutil.rmtree(w.base, ignore_errors=True)
    print(f'{"PASS" if not bad else "FAIL"} checkpoint policy: {len(cases) - bad} / {len(cases)}')
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
