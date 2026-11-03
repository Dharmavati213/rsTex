#!/usr/bin/env -S python3 -I
"""ktrace.py: what a run asks of the kernel, seen from outside: each system call, and each page fault by the region it lands in, by phase.

    ktrace.py --engine PATH --docs hello [--mode cold|write|warm] [--calls] [--json FILE]

This machine has no strace and no perf, and a program's own counters do not see its start and its end, so this is ptrace for the system calls and perf_event_open for
the faults, all from a Python parent.  It takes the argv, environment and side files of bench.py and warm.py, by proflib:

    cold    RSTEX_NO_SNAPSHOT=1
    write   an empty cache directory, and RSTEX_CHECKPOINT_POLICY=always: a run that writes a checkpoint
    warm    the cache of such a run: a restore

Give it a prof build, tools/prof/rstex_prof.py --build, and with the counters file that writes it shows phases, and names the sections of the image.  Any other binary gets the
regions of /proc/PID/maps.  Times under ptrace mean nothing, since each system call stops twice, so only counts are shown, and the time of a call is for telling a long read
from a short one.  Faults are user-mode ones: a page first touched inside a system call, such as a read into a new page, counts in the rusage line and has no address here.
Nothing of a document is printed unless it is not private: the paths of a user/ or local/ entry stay out, and only their kind and count are given.
"""
import argparse
import collections
import ctypes
import json
import mmap
import os
import shutil
import signal
import struct
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import proflib  # noqa: E402

PTRACE_TRACEME, PTRACE_CONT, PTRACE_GETREGS, PTRACE_SYSCALL, PTRACE_SETOPTIONS, PTRACE_GETEVENTMSG = 0, 7, 12, 24, 0x4200, 0x4201
O_TRACESYSGOOD, O_TRACECLONE, O_TRACEEXEC, O_TRACEEXIT, O_EXITKILL = 1, 8, 0x10, 0x40, 0x100000
EV_CLONE, EV_EXEC, EV_EXIT = 3, 4, 6
REGS = ['r15', 'r14', 'r13', 'r12', 'rbp', 'rbx', 'r11', 'r10', 'r9', 'r8', 'rax', 'rcx', 'rdx', 'rsi', 'rdi', 'orig_rax', 'rip', 'cs', 'eflags', 'rsp', 'ss', 'fs_base', 'gs_base', 'ds', 'es', 'fs', 'gs']

SYSCALLS = {
    0: 'read', 1: 'write', 2: 'open', 3: 'close', 4: 'stat', 5: 'fstat', 6: 'lstat', 7: 'poll', 8: 'lseek', 9: 'mmap', 10: 'mprotect', 11: 'munmap', 12: 'brk', 13: 'rt_sigaction',
    14: 'rt_sigprocmask', 16: 'ioctl', 17: 'pread64', 18: 'pwrite64', 20: 'writev', 21: 'access', 24: 'sched_yield', 25: 'mremap', 28: 'madvise', 35: 'nanosleep', 39: 'getpid', 56: 'clone', 59: 'execve', 60: 'exit',
    61: 'wait4', 62: 'kill', 63: 'uname', 72: 'fcntl', 76: 'truncate', 77: 'ftruncate', 78: 'getdents', 79: 'getcwd', 80: 'chdir', 82: 'rename', 83: 'mkdir', 84: 'rmdir', 87: 'unlink', 89: 'readlink', 96: 'gettimeofday',
    97: 'getrlimit', 98: 'getrusage', 102: 'getuid', 104: 'getgid', 107: 'geteuid', 108: 'getegid', 110: 'getppid', 131: 'sigaltstack', 157: 'prctl', 158: 'arch_prctl', 186: 'gettid', 201: 'time', 202: 'futex', 204: 'sched_getaffinity',
    217: 'getdents64', 218: 'set_tid_address', 228: 'clock_gettime', 229: 'clock_getres', 231: 'exit_group', 234: 'tgkill', 257: 'openat', 262: 'newfstatat', 263: 'unlinkat', 267: 'readlinkat', 269: 'faccessat', 273: 'set_robust_list',
    280: 'utimensat', 298: 'perf_event_open', 302: 'prlimit64', 316: 'renameat2', 318: 'getrandom', 325: 'mlock2', 332: 'statx', 334: 'rseq', 435: 'clone3', 439: 'faccessat2',
}
# Which argument is a path, by system call.
PATH_ARG = {2: 0, 4: 0, 6: 0, 21: 0, 59: 0, 76: 0, 80: 0, 82: 0, 83: 0, 84: 0, 87: 0, 89: 0, 257: 1, 262: 1, 263: 1, 267: 1, 269: 1, 280: 1, 316: 1, 332: 1, 439: 1}

libc = ctypes.CDLL(None, use_errno=True)
libc.ptrace.argtypes = [ctypes.c_long, ctypes.c_long, ctypes.c_void_p, ctypes.c_void_p]
libc.ptrace.restype = ctypes.c_long


def ptrace(req, pid, addr=None, data=None):
    r = libc.ptrace(req, pid, addr, data)
    if r < 0:
        raise OSError(ctypes.get_errno(), f'ptrace {req}')
    return r


def getregs(pid):
    buf = (ctypes.c_ulonglong * len(REGS))()
    ptrace(PTRACE_GETREGS, pid, None, ctypes.addressof(buf))
    return dict(zip(REGS, buf))


def read_cstring(mem, addr):
    """A NUL-terminated string of the stopped process."""
    try:
        data = os.pread(mem, 4096, addr)
    except OSError:
        return None
    return data.split(b'\0', 1)[0].decode('latin-1')


class Faults:
    """Every user-mode page fault of one thread, from its next exec or from now, with where it landed.  The kernel does not map an event that follows new threads, so each gets its own."""
    SAMPLE_IP, SAMPLE_TID, SAMPLE_TIME, SAMPLE_ADDR = 1, 2, 4, 8
    PAGES = 1 << 6

    def __init__(self, pid, at_exec):
        flags = proflib.F_EXCLUDE_KERNEL | proflib.F_EXCLUDE_HV | (1 << 25)     # use_clockid
        if at_exec:
            flags |= proflib.F_DISABLED | proflib.F_ENABLE_ON_EXEC
        sample_type = self.SAMPLE_IP | self.SAMPLE_TID | self.SAMPLE_TIME | self.SAMPLE_ADDR
        head = struct.pack('<IIQQQQQ', 1, proflib.ATTR_SIZE, 2, 1, sample_type, 0, flags)
        buf = bytearray(head + bytes(proflib.ATTR_SIZE - len(head)))
        struct.pack_into('<i', buf, 92, time.CLOCK_MONOTONIC)
        cbuf = ctypes.create_string_buffer(bytes(buf), proflib.ATTR_SIZE)
        self.fd = libc.syscall(ctypes.c_long(proflib.SYS_PERF_EVENT_OPEN), cbuf, ctypes.c_int(pid), ctypes.c_int(-1), ctypes.c_int(-1), ctypes.c_ulong(proflib.PERF_FLAG_FD_CLOEXEC))
        if self.fd < 0:
            raise OSError(ctypes.get_errno(), 'perf_event_open')
        self.map = mmap.mmap(self.fd, (self.PAGES + 1) * 4096, mmap.MAP_SHARED, mmap.PROT_READ | mmap.PROT_WRITE)

    def samples(self):
        data_head, data_tail = struct.unpack_from('<QQ', self.map, 1024)
        data_offset, data_size = struct.unpack_from('<QQ', self.map, 1040)
        ring = self.map[data_offset:data_offset + data_size]
        out = []
        at = data_tail
        while at < data_head:
            rtype, _misc, size = struct.unpack_from('<IHH', (ring + ring)[at % data_size:at % data_size + 8])
            body = (ring + ring)[at % data_size:at % data_size + size]
            if rtype == 9:      # PERF_RECORD_SAMPLE
                ip, _pid, tid, t, addr = struct.unpack_from('<QIIQQ', body, 8)
                out.append((t, tid, ip, addr))
            at += size
        return sorted(out)


def trace(argv, cwd, env):
    """Runs the command under both: (system calls, faults, /proc/PID/maps at the end, rusage)."""
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
            ptrace(PTRACE_TRACEME, 0)
            os.execve(exe, argv, env)
        finally:
            os._exit(status)
    os.close(r)
    faults = [Faults(pid, True)]
    t0 = time.monotonic_ns()
    os.write(w, b'x')
    os.close(w)
    _, status = os.waitpid(pid, 0)
    options = O_TRACESYSGOOD | O_TRACECLONE | O_TRACEEXEC | O_TRACEEXIT | O_EXITKILL
    ptrace(PTRACE_SETOPTIONS, pid, None, options)
    calls, open_call = [], {}
    maps = None
    mems = {}
    ptrace(PTRACE_SYSCALL, pid)
    ru = None
    live = {pid}
    while live:
        tid, status, usage = os.wait4(-1, os.WUNTRACED | 0x40000000)     # __WALL
        if os.WIFEXITED(status) or os.WIFSIGNALED(status):
            live.discard(tid)
            if tid == pid:
                ru = usage
            continue
        if not os.WIFSTOPPED(status):
            continue
        sig = os.WSTOPSIG(status)
        deliver = None
        if sig == signal.SIGTRAP | 0x80:
            regs = getregs(tid)
            if tid in open_call:
                c = open_call.pop(tid)
                c['ret'] = regs['rax'] - (1 << 64) if regs['rax'] >= 1 << 63 else regs['rax']
                c['t1'] = time.monotonic_ns()
                calls.append(c)
            else:
                nr = regs['orig_rax']
                args = [regs['rdi'], regs['rsi'], regs['rdx'], regs['r10'], regs['r8'], regs['r9']]
                c = {'t0': time.monotonic_ns(), 'tid': tid, 'nr': nr, 'name': SYSCALLS.get(nr, str(nr)), 'args': args}
                if nr in PATH_ARG:
                    if tid not in mems:
                        mems[tid] = os.open(f'/proc/{tid}/mem', os.O_RDONLY)
                    c['path'] = read_cstring(mems[tid], args[PATH_ARG[nr]])
                open_call[tid] = c
        elif sig == signal.SIGTRAP and status >> 16:
            event = status >> 16
            if event == EV_CLONE:
                msg = ctypes.c_ulong()
                ptrace(PTRACE_GETEVENTMSG, tid, None, ctypes.addressof(msg))
                live.add(msg.value)
                faults.append(Faults(msg.value, False))
            elif event == EV_EXIT and tid == pid:
                with open(f'/proc/{pid}/maps') as f:
                    maps = f.read()
        elif sig == signal.SIGSTOP and tid not in open_call and tid != pid:
            pass        # a new thread's first stop
        else:
            deliver = sig
        ptrace(PTRACE_SYSCALL, tid, None, deliver)
    for fd in mems.values():
        os.close(fd)
    return {'calls': calls, 'faults': sorted(x for f in faults for x in f.samples()), 'maps': maps or '', 't0': t0, 'rusage': ru}


def parse_maps(text):
    """[(start, end, name)] of /proc/PID/maps."""
    out = []
    for line in text.splitlines():
        p = line.split(None, 5)
        lo, hi = p[0].split('-')
        out.append((int(lo, 16), int(hi, 16), p[1], p[5] if len(p) > 5 else ''))
    return out


class Regions:
    """Names an address: a section of the image if the prof build said where they are, otherwise what the maps say."""

    def __init__(self, maps, image):
        self.maps = maps
        self.sections = []
        if image:
            self.sections = sorted((image['base'] + s['offset'], image['base'] + s['offset'] + s['bytes'], s['name']) for s in image['sections'])

    def name(self, addr):
        for a, b, n in self.sections:
            if a <= addr < b:
                return 'image: ' + n
        for lo, hi, perms, name in self.maps:
            if lo <= addr < hi:
                base = os.path.basename(name) if name.startswith('/') else name
                if name.startswith('/'):
                    kind = 'text' if 'x' in perms else ('data' if 'w' in perms else 'rodata')
                    return f'{kind}: {base}'
                return name or f'anon {perms}'
        return 'unmapped now'


def phases_of(counters):
    if not counters:
        return []
    main = counters['clock'].get('main_abs_ns')
    zero = counters['clock'].get('t0_abs_ns', main)
    return [(zero + p['t_ns'], p['name']) for p in counters['phases']]


def phase_at(phases, t):
    name = 'before main'
    for at, n in phases:
        if t <= at:
            return n
        name = n
    return 'after the last mark'


def parse_prof_image(path):
    """The section table a prof build wrote."""
    image = {'base': None, 'sections': []}
    with open(path) as f:
        for line in f:
            p = line.rstrip('\n').split('\t')
            if p[0] == 'image':
                image['base'] = int(p[1])
            elif p[0] == 'section':
                image['sections'].append({'name': p[1], 'offset': int(p[2]), 'bytes': int(p[3])})
    return image if image['base'] else None


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--engine', required=True)
    ap.add_argument('--docs', default='hello')
    ap.add_argument('--mode', default='cold', choices=['cold', 'write', 'warm'])
    ap.add_argument('--calls', action='store_true', help='each system call, in order')
    ap.add_argument('--json')
    args = ap.parse_args()
    engine = proflib.Engine('e', args.engine)
    for doc in proflib.resolve_docs(args.docs.split(',')):
        doc.prepare()
        private = doc.id.startswith(('user/', 'local/', 'cd-local/'))
        root = os.path.join(proflib.out_root(), 'ktrace', doc.name.replace('/', '_'))
        shutil.rmtree(root, ignore_errors=True)
        work = os.path.join(root, 'o')
        cache = os.path.join(root, 'cache')
        os.makedirs(work)
        os.makedirs(cache)
        counters = os.path.join(root, 'counters.txt')

        def command():
            shutil.rmtree(work, ignore_errors=True)
            os.makedirs(work)
            proflib.corpus().restore_state(doc.prepare(), work)
            argv, cwd, env = engine.command(doc, work)
            env = dict(env, XDG_CACHE_HOME=cache, RSTEX_CHECKPOINT_POLICY='always')
            del env['RSTEX_NO_SNAPSHOT']
            return argv, cwd, env
        argv, cwd, env = command()
        if args.mode == 'cold':
            env['RSTEX_NO_SNAPSHOT'] = '1'
        if args.mode == 'warm':
            subprocess.run(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
            argv, cwd, env = command()
        env['RSTEX_PROF_COUNTERS'] = counters
        res = trace(argv, cwd, env)
        parsed = proflib.parse_counters(counters) if os.path.exists(counters) else None
        phases = phases_of(parsed)
        image = parse_prof_image(counters) if parsed else None
        regions = Regions(parse_maps(res['maps']), image)

        print(f'== {doc.name}, {args.mode}: {len(res["calls"])} system calls, {len(res["faults"])} user-mode faults; rusage minflt {res["rusage"].ru_minflt}')
        by_name = collections.Counter(c['name'] for c in res['calls'])
        print('system calls: ' + ', '.join(f'{k} {v}' for k, v in by_name.most_common()))
        table = collections.defaultdict(collections.Counter)
        order = []
        for t, _tid, _ip, addr in res['faults']:
            ph = phase_at(phases, t)
            if ph not in order:
                order.append(ph)
            table[ph][regions.name(addr)] += 1
        totals = collections.Counter()
        for ph in order:
            totals.update(table[ph])
        print('faults by region:')
        for name, n in totals.most_common():
            print(f'  {n:>6}  {name}')
        if phases:
            print('faults by phase:')
            for ph in order:
                top = ', '.join(f'{k} {v}' for k, v in table[ph].most_common(6))
                print(f'  {sum(table[ph].values()):>6}  {ph}: {top}')
            print('system calls by phase:')
            per = collections.defaultdict(collections.Counter)
            seen = []
            for c in res['calls']:
                ph = phase_at(phases, c['t0'])
                if ph not in seen:
                    seen.append(ph)
                per[ph][c['name']] += 1
            for ph in seen:
                print(f'  {sum(per[ph].values()):>6}  {ph}: ' + ', '.join(f'{k} {v}' for k, v in per[ph].most_common()))
        if args.calls:
            print('system calls in order:')
            for c in res['calls']:
                path = c.get('path')
                if private and path:
                    path = '(a path)'
                sizes = f' {c["args"][2]}' if c['name'] in ('read', 'write', 'pread64', 'pwrite64') else ''
                if c['name'] in ('mmap', 'munmap', 'madvise', 'mprotect'):
                    sizes = f' {c["args"][1]}'
                print(f'  {(c["t0"] - res["t0"]) / 1e6:>9.3f} {(c["t1"] - c["t0"]) / 1e3:>8.1f}us {c["name"]:<14}{sizes} {path or ""} = {c["ret"]}')
        if args.json:
            with open(args.json, 'w') as f:
                json.dump({'calls': res['calls'], 'faults': res['faults']}, f)


if __name__ == '__main__':
    main()
