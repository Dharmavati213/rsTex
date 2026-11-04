#!/usr/bin/env -S python3 -I -S
"""benchrun.py: the small process that tools/prof/benchset.py spawns its measured runs from.  It reads requests on standard input and answers on standard output, in marshal.

A child's ru_maxrss carries its parent's size across the exec: exec keeps the old address space's peak in the signal struct, whether by fork or by vfork, and a worker holding the manifests is over 100 MB, more
than a small run of rstex or pdflatex ever touches.  So a peak measured from such a worker is that worker's size.  This process is started with -S and holds next to nothing, so what it reports is the run's peak, down to
the floor it reports by running /bin/true.  Forking from something this small also costs less than from a large interpreter, so a run spawned from here can be timed as well as counted.

The counters are tools/prof/proflib.py's run_counted, whose constants the caller passes as the attribute blocks to open: user mode, from the exec on, following threads and children.  A run has a limit in
seconds, and past it its session is killed, with what the counters hold kept.  Nothing here looks at what a run prints: its stdout goes nowhere, and its stderr goes to a file if the caller names one.
"""
import ctypes
import marshal
import os
import signal
import struct
import sys
import time

libc = ctypes.CDLL(None, use_errno=True)
PR_SET_PDEATHSIG = 1
PERF_FLAG_FD_CLOEXEC = 8


def perf_open(pid, attr, nr):
    buf = ctypes.create_string_buffer(attr, len(attr))
    return libc.syscall(ctypes.c_long(nr), buf, ctypes.c_int(pid), ctypes.c_int(-1), ctypes.c_int(-1), ctypes.c_ulong(PERF_FLAG_FD_CLOEXEC))


def run(req):
    r, w = os.pipe()
    pid = os.fork()
    if pid == 0:
        status = 127
        try:
            libc.prctl(PR_SET_PDEATHSIG, signal.SIGKILL)
            os.setsid()
            os.close(w)
            os.chdir(req['cwd'])
            null = os.open(os.devnull, os.O_RDWR)
            for fd in (0, 1, 2):
                os.dup2(null, fd)
            if req.get('stderr'):
                os.dup2(os.open(req['stderr'], os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), 2)
                if req.get('merge'):
                    os.dup2(2, 1)
            os.read(r, 1)
            os.close(r)
            os.execve(req['exe'], req['argv'], req['env'])
        finally:
            os._exit(status)
    os.close(r)
    fds = [(name, perf_open(pid, attr, req['perf_nr'])) for name, attr in req['events']]
    timed_out = [False]

    def expired(*_):
        timed_out[0] = True
        try:
            os.killpg(pid, signal.SIGKILL)
        except OSError:
            pass
    if req.get('limit'):
        signal.signal(signal.SIGALRM, expired)
        signal.setitimer(signal.ITIMER_REAL, req['limit'])
    t0 = time.monotonic_ns()
    os.write(w, b'x')
    os.close(w)
    _, status, ru = os.wait4(pid, 0)
    t1 = time.monotonic_ns()
    signal.setitimer(signal.ITIMER_REAL, 0)
    counters = {}
    for name, fd in fds:
        if fd < 0:
            continue
        v = struct.unpack('<QQQ', os.read(fd, 24))
        os.close(fd)
        counters[name] = (v[0], v[1], v[2])
    return {'rc': os.waitstatus_to_exitcode(status), 'wall_ms': (t1 - t0) / 1e6, 'user_ms': ru.ru_utime * 1e3, 'sys_ms': ru.ru_stime * 1e3, 'minflt': ru.ru_minflt, 'majflt': ru.ru_majflt,
            'maxrss_kb': ru.ru_maxrss, 'counters': counters, 'timed_out': timed_out[0]}


def main():
    libc.prctl(PR_SET_PDEATHSIG, signal.SIGKILL)
    while True:
        try:
            req = marshal.load(sys.stdin.buffer)
        except (EOFError, ValueError):
            return
        marshal.dump(run(req), sys.stdout.buffer)
        sys.stdout.buffer.flush()


if __name__ == '__main__':
    main()
