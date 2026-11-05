#!/usr/bin/env -S python3 -I
"""synctex_cost.py: what -synctex=1 costs a run, on the engine's own thread and in all.

    synctex_cost.py --engine PATH [--base PATH] [--docs hello,main,LMB,sga3] [--no-warm] [--no-cold] [--counters 3] [--rounds 0] [--option=-synctex=1] [--native-off] [--json FILE]

Each document compiles the way warm.py has it, in a directory that keeps its path, with the oracle's last-pass side files put back before every run.  Two kinds of run:

    cold    RSTEX_NO_SNAPSHOT=1
    warm    a checkpoint written for this option, and sealed: a restore, as a user has it from the third run on, proved by RSTEX_CHECKPOINT_TRACE

and for each the option off and on.  --base adds the same with SyncTeX off for another binary, the base a change is judged against.

The counters are user-mode instructions and cycles from exec to exit, twice over: of the first thread alone, which with glibc is the engine's (DESIGN section 8), and of it and the threads it starts, as bench.py counts.  So what the
deflate workers do shows in the second and not in the first.  Instructions first: tools/prof/bench.py says why.  --rounds N adds N rounds of paired runs in turn, the order changing each round, of off, on and
off again, whose medians of paired ratios are the cycles to believe, with off against off as the control: N of 40 or more, as PLAN section 8 says.

Only ids and numbers are printed, and everything a run writes stays under target/.
"""
import argparse
import ctypes
import gzip
import json
import os
import shutil
import statistics
import struct
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import proflib  # noqa: E402

COUNTERS = [('instr_engine', 0, 1, False), ('cycles_engine', 0, 0, False), ('instr_all', 0, 1, True), ('cycles_all', 0, 0, True), ('faults', 1, 2, True)]


def perf_open(pid, type_, config, inherit):
    if proflib._libc is None:
        proflib._libc = ctypes.CDLL(None, use_errno=True)
    flags = proflib.F_DISABLED | proflib.F_EXCLUDE_KERNEL | proflib.F_EXCLUDE_HV | proflib.F_ENABLE_ON_EXEC
    if inherit:
        flags |= proflib.F_INHERIT
    head = struct.pack('<IIQQQQQ', type_, proflib.ATTR_SIZE, config, 0, 0, proflib.READ_TIMES, flags)
    buf = ctypes.create_string_buffer(head + bytes(proflib.ATTR_SIZE - len(head)), proflib.ATTR_SIZE)
    return proflib._libc.syscall(ctypes.c_long(proflib.SYS_PERF_EVENT_OPEN), buf, ctypes.c_int(pid), ctypes.c_int(-1), ctypes.c_int(-1), ctypes.c_ulong(proflib.PERF_FLAG_FD_CLOEXEC))


def counted(argv, cwd, env):
    """As proflib.run_counted, with these counters."""
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
    fds = [(name, perf_open(pid, t, c, inh)) for name, t, c, inh in COUNTERS]
    t0 = time.monotonic_ns()
    os.write(w, b'x')
    os.close(w)
    _, status, ru = os.wait4(pid, 0)
    t1 = time.monotonic_ns()
    res = {'rc': os.waitstatus_to_exitcode(status), 'wall_ms': (t1 - t0) / 1e6, 'cpu_ms': (ru.ru_utime + ru.ru_stime) * 1e3, 'minflt': ru.ru_minflt}
    for name, fd in fds:
        if fd >= 0:
            res[name] = struct.unpack('<QQQ', os.read(fd, 24))[0]
            os.close(fd)
    return res


class NoRestore(Exception):
    """A document that takes no checkpoint, such as one in plain TeX."""


class Setup:
    """One binary, one document, one option, one kind of run: all it needs to run again from the same state."""

    def __init__(self, engine, doc, option, warm, root, name):
        self.doc, self.option, self.warm = doc, option, warm
        self.root = os.path.join(root, name)
        shutil.rmtree(self.root, ignore_errors=True)
        self.work = os.path.join(self.root, 'o')
        self.cache = os.path.join(self.root, 'c')
        os.makedirs(self.cache)
        self.reset()
        argv, self.cwd, env = engine.command(doc, self.work)
        self.argv = [argv[0]] + ([option] if option else []) + argv[1:]
        self.env = dict(env)
        if warm:
            self.env.pop('RSTEX_NO_SNAPSHOT', None)
            self.env['RSTEX_CHECKPOINT_POLICY'] = 'always'
            # The preamble's checkpoint and no more: a chain would have a run start at its last record, and there is a chain on both sides now.
            self.env['RSTEX_NO_PAGES'] = '1'
            self.env['XDG_CACHE_HOME'] = self.cache
            subprocess.run(self.argv, cwd=self.cwd, env=self.env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            proflib.seal_checkpoint(self.reset, self.argv, self.cwd, self.env)
            self.reset()
            e = dict(self.env, RSTEX_CHECKPOINT_TRACE='1')
            p = subprocess.run(self.argv, cwd=self.cwd, env=e, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
            said = [s for s in p.stderr.decode('latin-1').splitlines() if s.startswith('rstex: checkpoint: ')]
            if 'rstex: checkpoint: restored' not in said:
                raise NoRestore()

    def reset(self):
        shutil.rmtree(self.work, ignore_errors=True)
        os.makedirs(self.work)
        proflib.corpus().restore_state(self.doc.prepare(), self.work)

    def run(self):
        self.reset()
        return counted(self.argv, self.cwd, self.env)

    def file_sizes(self):
        """The size of the file, and of its text."""
        job = self.doc.entry['jobname']
        for name, gz in ((job + '.synctex.gz', True), (job + '.synctex', False)):
            p = os.path.join(self.work, name)
            if os.path.exists(p):
                with open(p, 'rb') as f:
                    data = f.read()
                if gz:
                    return len(data), len(gzip.decompress(data))
                return len(data), len(data)
        return None, None


def median(rs, key):
    return statistics.median(r[key] for r in rs)


def paired(a, b, rounds):
    """Runs of a and b in turn, the order changing each round: the median of the ratios b/a, for each counter."""
    ratios = {}
    for k in range(rounds):
        ra = a.run() if k % 2 == 0 else None
        rb = b.run()
        if ra is None:
            ra = a.run()
        for key in ('instr_engine', 'cycles_engine', 'instr_all', 'cycles_all', 'cpu_ms', 'wall_ms'):
            ratios.setdefault(key, []).append(rb[key] / ra[key])
    return {key: statistics.median(v) for key, v in ratios.items()}


def show(label, off, on):
    print(f'  {label:14s} Minstr engine {off["instr_engine"] / 1e6:10.3f} -> {on["instr_engine"] / 1e6:10.3f}  {on["instr_engine"] / off["instr_engine"]:.4f}   all {off["instr_all"] / 1e6:10.3f} -> {on["instr_all"] / 1e6:10.3f}  {on["instr_all"] / off["instr_all"]:.4f}   faults {off["faults"]:.0f} -> {on["faults"]:.0f}')


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--engine', required=True)
    ap.add_argument('--base')
    ap.add_argument('--docs', default='hello,main,LMB,sga3')
    ap.add_argument('--no-warm', action='store_true')
    ap.add_argument('--no-cold', action='store_true')
    ap.add_argument('--native-off', action='store_true')
    ap.add_argument('--counters', type=int, default=3)
    ap.add_argument('--rounds', type=int, default=0)
    ap.add_argument('--option', default='-synctex=1')
    ap.add_argument('--json')
    args = ap.parse_args()
    engine = proflib.Engine('new', args.engine, no_commute=args.native_off)
    base = proflib.Engine('base', args.base, no_commute=args.native_off) if args.base else None
    docs = proflib.resolve_docs(args.docs.split(','))
    out = []
    kinds = ([('cold', False)] if not args.no_cold else []) + ([('warm', True)] if not args.no_warm else [])
    for doc in docs:
        doc.prepare()
        root = os.path.join(proflib.out_root(), 'synctex-cost', doc.name.replace('/', '_'))
        print(doc.id)
        rec = {'id': doc.id}
        for kind, warm in kinds:
            try:
                off = Setup(engine, doc, None, warm, root, kind + '-off')
                on = Setup(engine, doc, args.option, warm, root, kind + '-on_')
            except NoRestore:
                print(f'  {kind}: no restore')
                continue
            roff = [off.run() for _ in range(args.counters)]
            ron = [on.run() for _ in range(args.counters)]
            gz, text = on.file_sizes()
            res = {'off': {k: median(roff, k) for k in roff[0]}, 'on': {k: median(ron, k) for k in ron[0]}, 'gz': gz, 'text': text}
            show(kind, res['off'], res['on'])
            print(f'  {"":14s} the file {gz} bytes of {text} of text, {len(ron)} runs each; CPU ms {res["off"]["cpu_ms"]:.1f} -> {res["on"]["cpu_ms"]:.1f} on a machine whose load says little')
            if base:
                b = Setup(base, doc, None, warm, root, kind + '-bas')
                rb = [b.run() for _ in range(args.counters)]
                res['base'] = {k: median(rb, k) for k in rb[0]}
                show(kind + ' base, off', res['base'], res['off'])
            if args.rounds:
                off2 = Setup(engine, doc, None, warm, root, kind + '-of2')
                res['paired'] = paired(off, on, args.rounds)
                res['control'] = paired(off, off2, args.rounds)
                print(f'  {"":14s} in turn, {args.rounds} rounds, median of ratios: cycles engine {res["paired"]["cycles_engine"]:.4f} (A/A {res["control"]["cycles_engine"]:.4f}), cycles all {res["paired"]["cycles_all"]:.4f} (A/A {res["control"]["cycles_all"]:.4f}), CPU {res["paired"]["cpu_ms"]:.4f} (A/A {res["control"]["cpu_ms"]:.4f}), wall {res["paired"]["wall_ms"]:.4f} (A/A {res["control"]["wall_ms"]:.4f})')
            rec[kind] = res
        out.append(rec)
    if args.json:
        with open(args.json, 'w') as f:
            json.dump(out, f, indent=1)
    return 0


if __name__ == '__main__':
    sys.exit(main())
