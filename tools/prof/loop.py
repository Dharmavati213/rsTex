#!/usr/bin/env -S python3 -I
"""loop.py: what a save costs up to the pixels: the file written, the build, and Poppler opening the PDF and rendering a page, for rstex behind latexmk and for rstex by itself.

    loop.py --engine PATH [--base PATH] [--docs ID,...] [--rounds N] [--long-rounds N] [--scale S] [--no-aa] [--json FILE]

--docs takes manifest ids and the short names of proflib.py; the default is the owner's two long documents and SGA3.  --rounds is for each document of up to 100 pages and --long-rounds for longer ones: half
of the rounds are timed by the plain spawn and half run under user-mode counters.  In each round the source changes once, and every configuration builds that state in its own output directory, in an order
that goes round:

    latexmk          latexmk -norc -pdf -pdflatex='<engine> -progname=pdflatex -fmt=<format> %O %S' <Lemma's flags> -outdir=<dir> ./<main>
    direct           <engine> -progname=pdflatex -fmt=<format> --until-stable <Lemma's flags> -output-directory=<dir> ./<main>
    direct-aa        the same in another directory: the control, whose paired ratio says how far to trust the others
    plain            <engine> and one pass, with no flag: what the flag itself costs, where one pass is enough
    base-latexmk     --base behind latexmk, to see what the save costs on a binary with no flag

The environment is Lemma's (crates/lemma-build/src/request.rs) with a pinned clock.  A configuration has its own cache directory, kept between runs, so that after the first few runs this is what a user gets: the preamble
checkpoint with its seal, and the native diagram path.

The change: a document is mirrored under target/, each entry a symbolic link but the main file, which is copied.  A short neutral sentence goes in and comes out again at a blank line near the middle of the
main file's body, or a comment at its end if it has no body.  Nothing of a document is read but to find that place, and this tool prints ids and numbers only.

Poppler is the library Lemma uses and in the way it uses it (crates/lemma-pdf/src/worker.rs and render.rs), by ctypes, in this process, once per round on the PDF of `direct`, which is the same bytes as the others': the whole
file read, poppler_document_new_from_bytes, then the middle page into an ARGB32 surface at --scale device pixels per bp (default 2, 144 dpi).  A viewer showing two pages pays the second page again, and after that it
has its own redraw, which is not measured here.  Page sizes are taken as Lemma's reload takes them.  Counters around Poppler are this process's own.  The numbers mean what numbers on this machine mean: read instructions first, then
paired cycles, against the control.  A wall time is what it was under the load that is printed with it.
"""
import argparse
import ctypes
import json
import os
import re
import shlex
import shutil
import struct
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(HERE)), 'tests', 'until_stable'))
import common  # noqa: E402
import proflib  # noqa: E402

DEFAULT_DOCS = ['user/main', 'user/LMB', 'local/sga3/SGA3-English']
SENTENCE = b'Here are a few neutral words that an edit has put in.\n\n'
OUTPUTS = ('.aux', '.log', '.toc', '.lof', '.lot', '.out', '.fls', '.fdb_latexmk', '.synctex.gz', '.rstex-tools')


# ------------------------------------------------------------------------------------------------ the document
def mirror(entry, dst):
    """A tree that reads as the document's, with its main file a copy.  Its own earlier outputs stay out, and a shipped `.bbl` stays in, as it is part of what came."""
    shutil.rmtree(dst, ignore_errors=True)
    rel = os.path.relpath(entry['main'], entry['cwd'])
    os.makedirs(dst)
    # Each directory on the way to the main file is real.
    src, to = entry['cwd'], dst
    parts = rel.split(os.sep)
    for i, part in enumerate(parts):
        for name in os.listdir(src):
            if name == part or name.startswith(entry['jobname'] + '.') and name.endswith(OUTPUTS) or name == entry['jobname'] + '.pdf':
                continue
            os.symlink(os.path.join(src, name), os.path.join(to, name))
        if i + 1 < len(parts):
            src, to = os.path.join(src, part), os.path.join(to, part)
            os.makedirs(to)
    shutil.copy2(entry['main'], os.path.join(dst, rel))
    return os.path.join(dst, rel)


def places(text):
    """Where a sentence may go, best first: blank lines of the body, from the middle outwards."""
    begin = text.find(b'\\begin{document}')
    if begin < 0:
        return []
    end = text.find(b'\\end{document}', begin)
    end = len(text) if end < 0 else end
    found = [m.start() + 1 for m in re.finditer(rb'\n[ \t]*\n', text[begin:end])]
    found = [begin + p for p in found]
    middle = len(found) // 2
    return sorted(found, key=lambda p: abs(found.index(p) - middle))[:6]


def save(path, data):
    """As an editor does: a new file, then a rename."""
    t0 = time.monotonic_ns()
    tmp = path + '.saving'
    with open(tmp, 'wb') as f:
        f.write(data)
    os.replace(tmp, path)
    return (time.monotonic_ns() - t0) / 1e6


# ------------------------------------------------------------------------------------------------ Poppler
class Self:
    """User-mode counters on this process."""
    IOC_ENABLE, IOC_DISABLE, IOC_RESET = 0x2400, 0x2401, 0x2403

    def __init__(self):
        self.libc = ctypes.CDLL(None, use_errno=True)
        self.fds = {}
        for name, config in (('instructions', 1), ('cycles', 0)):
            flags = proflib.F_DISABLED | proflib.F_EXCLUDE_KERNEL | proflib.F_EXCLUDE_HV
            head = struct.pack('<IIQQQQQ', 0, proflib.ATTR_SIZE, config, 0, 0, 0, flags)
            buf = ctypes.create_string_buffer(head + bytes(proflib.ATTR_SIZE - len(head)), proflib.ATTR_SIZE)
            fd = self.libc.syscall(ctypes.c_long(proflib.SYS_PERF_EVENT_OPEN), buf, ctypes.c_int(0), ctypes.c_int(-1), ctypes.c_int(-1), ctypes.c_ulong(proflib.PERF_FLAG_FD_CLOEXEC))
            if fd >= 0:
                self.fds[name] = fd

    def measure(self, fn):
        """What `fn` took: wall, this thread's CPU and the counters."""
        for fd in self.fds.values():
            self.libc.ioctl(fd, self.IOC_RESET, 0)
            self.libc.ioctl(fd, self.IOC_ENABLE, 0)
        t0, c0 = time.monotonic_ns(), time.thread_time_ns()
        res = fn()
        t1, c1 = time.monotonic_ns(), time.thread_time_ns()
        out = {'wall_ms': (t1 - t0) / 1e6, 'cpu_ms': (c1 - c0) / 1e6}
        for name, fd in self.fds.items():
            self.libc.ioctl(fd, self.IOC_DISABLE, 0)
            out[name] = struct.unpack('<Q', os.read(fd, 8))[0]
        return res, out


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

    def open(self, data):
        """`data` must outlive the document, as Lemma's bytes do."""
        b = self.glib.g_bytes_new_static(data, len(data))
        doc = self.pop.poppler_document_new_from_bytes(b, None, None)
        self.glib.g_bytes_unref(b)
        if not doc:
            raise RuntimeError('poppler could not open the PDF')
        return doc, self.pop.poppler_document_get_n_pages(doc)

    def render(self, doc, index, scale):
        page = self.pop.poppler_document_get_page(doc, index)
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

    def close(self, doc):
        self.gobject.g_object_unref(doc)


# ------------------------------------------------------------------------------------------------ a configuration
class Config:
    """How one way of building runs on one document."""

    def __init__(self, label, engine, doc, src, main_rel, root, latexmk):
        self.label, self.engine, self.doc, self.src, self.main_rel = label, engine, doc, src, main_rel
        self.latexmk = latexmk
        self.out = '.build-' + label
        self.cache = os.path.join(root, 'cache-' + engine.sha[:16])
        os.makedirs(self.cache, exist_ok=True)
        self.env = common.document_env(doc.entry, self.cache)
        fmt = engine.format_path(doc.format)
        self.flag = label.startswith('direct')
        if latexmk:
            cmd = f'{shlex.quote(engine.path)} -progname={doc.format} -fmt={shlex.quote(fmt)} %O %S'
            self.argv = ['latexmk', '-norc', '-pdf', '-pdflatex=' + cmd] + common.LEMMA_FLAGS + ['-outdir=' + self.out, './' + main_rel]
        else:
            self.argv = [engine.path, '-progname=' + doc.format, '-fmt=' + fmt] + (['--until-stable'] if self.flag else []) + common.LEMMA_FLAGS + ['-output-directory=' + self.out, './' + main_rel]
        self.pdf = os.path.join(src, self.out, doc.entry['jobname'] + '.pdf')
        self.tool_runs = 0
        self.failures = 0
        self.timed, self.counted = [], []

    def prepare(self):
        """From nothing to where a save starts."""
        out = os.path.join(self.src, self.out)
        common.reset(out)
        common.mirror_dirs(self.src, out)
        self.build_settled()

    def build_settled(self):
        """A build with what the caller of `direct` does when a program is named.  Returns the last run's status."""
        for _ in range(6):
            r = subprocess.run(self.argv, cwd=self.src, env=self.env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, check=False)
            if self.latexmk or r.returncode != common.STATUS_TOOL:
                return r.returncode
            self.follow_tools(r.stderr)
        return r.returncode

    def follow_tools(self, stderr):
        for tool, _, file in common.TOOL_LINE.findall(stderr):
            self.tool_runs += 1
            with open(os.devnull, 'wb') as null:
                common.run_tool(tool.decode(), file, os.path.join(self.src, self.out), self.src, self.env, null)

    def passes_of(self, stderr):
        m = common.PASSES_LINE.search(stderr)
        return int(m.group(1)) if m else None

    def one(self, counted):
        """One build of the state the source is in.  What it took, whatever it took."""
        if counted:
            r = proflib.run_counted(self.argv, self.src, self.env)
            c = r['counters']
            rec = {'instructions': c.get('instructions', {}).get('value'), 'cycles': c.get('cycles', {}).get('value')}
        else:
            r = proflib.run_timed(self.argv, self.src, self.env)
            rec = {}
        rec.update(wall_ms=r['wall_ms'], cpu_ms=r['cpu_ms'], sys_ms=r['sys_ms'], minflt=r['minflt'])
        if r['rc'] == common.STATUS_TOOL and not self.latexmk:
            self.build_settled()
            self.tool_runs += 1
        elif r['rc'] != 0:
            self.failures += 1
        (self.counted if counted else self.timed).append(rec)


def median(rows, key):
    vals = [r[key] for r in rows if r.get(key) is not None]
    return proflib.median(vals)


def paired_ratio(a, b, key):
    vals = [x[key] / y[key] for x, y in zip(a, b) if x.get(key) and y.get(key)]
    return proflib.median(vals)


def run_doc(doc, args, engine, base, root, pop, me):
    slug = re.sub(r'[^A-Za-z0-9_.-]', '_', doc.id)
    work = os.path.join(root, 'loop', slug)
    src = os.path.join(work, 'src')
    main = mirror(doc.entry, src)
    main_rel = os.path.relpath(main, src)
    configs = [Config('direct', engine, doc, src, main_rel, work, False), Config('latexmk', engine, doc, src, main_rel, work, True), Config('plain', engine, doc, src, main_rel, work, False)]
    if not args.no_aa:
        configs.append(Config('direct-aa', engine, doc, src, main_rel, work, False))
    if base is not None:
        configs.append(Config('base-latexmk', base, doc, src, main_rel, work, True))
    for c in configs:
        c.prepare()
    with open(main, 'rb') as f:
        original = f.read()
    # A place where the sentence does no harm: the first that compiles.
    edited, where = None, 'comment'
    for p in places(original):
        candidate = original[:p] + SENTENCE + original[p:]
        save(main, candidate)
        if configs[0].build_settled() == 0:
            edited, where = candidate, 'body'
            break
    if edited is None:
        edited = original + b'\n% an edit\n'
    for c in configs:
        save(main, original)
        c.build_settled()
    # Runs that leave the checkpoint and its seal, as the policy has them: a user has those from the second or third save on.
    for state in (edited, original, edited, original):
        save(main, state)
        for c in configs:
            c.build_settled()
    time.sleep(2.2)
    for state in (edited, original):
        save(main, state)
        for c in configs:
            c.build_settled()
    with open(configs[0].pdf, 'rb') as f:
        data = f.read()
    probe, pages = pop.open(data)
    pop.close(probe)
    rounds = args.long_rounds if pages > 100 else args.rounds
    saves, opens, renders, reads, order = [], [], [], [], 0
    state = 0
    for k in range(rounds):
        counted = k % 2 == 1
        state ^= 1
        saves.append(save(main, edited if state else original))
        order = (order + 1) % len(configs)
        for c in configs[order:] + configs[:order]:
            c.one(counted)
        t0 = time.monotonic_ns()
        with open(configs[0].pdf, 'rb') as f:
            data = f.read()
        reads.append((time.monotonic_ns() - t0) / 1e6)
        (document, n), o = me.measure(lambda d=data: pop.open(d))
        opens.append(o)
        _, r = me.measure(lambda: pop.render(document, n // 2, args.scale))
        renders.append(r)
        pop.close(document)
    save(main, original)
    return summarize(doc, configs, saves, reads, opens, renders, pages, len(data), where, rounds)


def summarize(doc, configs, saves, reads, opens, renders, pages, pdf_size, where, rounds):
    by = {c.label: c for c in configs}
    res = {'id': doc.id, 'pages': pages, 'pdf_mb': round(pdf_size / 1e6, 2), 'edit': where, 'rounds': rounds, 'load': round(proflib.load_now(), 1),
           'save_ms': median([{'x': s} for s in saves], 'x'), 'read_ms': median([{'x': s} for s in reads], 'x')}
    for name, rows in (('open', opens), ('render', renders)):
        res[name] = {k: median(rows, k) for k in ('wall_ms', 'cpu_ms', 'instructions', 'cycles')}
    for c in configs:
        res[c.label] = {'wall_ms': median(c.timed, 'wall_ms'), 'cpu_ms': median(c.timed, 'cpu_ms'), 'sys_ms': median(c.timed, 'sys_ms'), 'minflt': median(c.timed, 'minflt'),
                        'instructions': median(c.counted, 'instructions'), 'cycles': median(c.counted, 'cycles'), 'failures': c.failures, 'tool_runs': c.tool_runs}
    d, l = by['direct'], by['latexmk']
    res['latexmk_over_direct'] = {k: paired_ratio(l.timed if k.endswith('ms') else l.counted, d.timed if k.endswith('ms') else d.counted, k) for k in ('instructions', 'cycles', 'cpu_ms', 'wall_ms')}
    res['latexmk_minus_direct'] = {k: proflib.median([x[k] - y[k] for x, y in zip(l.timed if k.endswith('ms') else l.counted, d.timed if k.endswith('ms') else d.counted) if x.get(k) is not None and y.get(k) is not None]) for k in ('instructions', 'cycles', 'cpu_ms', 'wall_ms')}
    def against(x, y):
        return {k: paired_ratio(x.timed if k.endswith('ms') else x.counted, y.timed if k.endswith('ms') else y.counted, k) for k in ('instructions', 'cycles', 'cpu_ms', 'wall_ms')}
    res['direct_over_plain'] = against(d, by['plain'])
    if 'direct-aa' in by:
        res['aa'] = against(d, by['direct-aa'])
    if 'base-latexmk' in by:
        res['latexmk_over_base_latexmk'] = against(l, by['base-latexmk'])
    # The shares of a save, in wall and in CPU, down each path.
    def shares(build, key):
        parts = {'save': res['save_ms'], 'read PDF': res['read_ms'], 'build': build[key], 'open': res['open'][key], 'render': res['render'][key]}
        total = sum(parts.values())
        return {'total_ms': round(total, 2), **{k: round(100 * v / total, 1) for k, v in parts.items()}}
    res['shares'] = {c.label: {'wall': shares(res[c.label], 'wall_ms'), 'cpu': shares(res[c.label], 'cpu_ms')} for c in configs}
    return res


def show(r):
    print(f"\n{r['id']}: {r['pages']} pages, PDF {r['pdf_mb']} MB, edit in the {r['edit']}, {r['rounds']} rounds, load {r['load']} at the end")
    print(f"  save {r['save_ms']:.2f} ms, read PDF {r['read_ms']:.2f} ms")
    for name in ('open', 'render'):
        p = r[name]
        print(f"  Poppler {name}: wall {p['wall_ms']:.1f} ms, CPU {p['cpu_ms']:.1f} ms, {(p.get('instructions') or 0) / 1e6:.1f} Minstr, {(p.get('cycles') or 0) / 1e6:.1f} Mcycles")
    for label in ('direct', 'direct-aa', 'plain', 'latexmk', 'base-latexmk'):
        if label in r:
            b = r[label]
            print(f"  {label:13s} wall {b['wall_ms']:.1f} ms, CPU {b['cpu_ms']:.1f} ms, sys {b['sys_ms']:.1f} ms, {b['instructions'] / 1e6:.1f} Minstr, {b['cycles'] / 1e6:.1f} Mcycles, {b['minflt']:.0f} faults, failures {b['failures']}, tools {b['tool_runs']}")
    print('  latexmk minus direct, paired:', {k: round(v / (1e6 if not k.endswith('ms') else 1), 1) for k, v in r['latexmk_minus_direct'].items()}, '(M for counters)')
    print('  latexmk over direct, paired:', {k: round(v, 3) for k, v in r['latexmk_over_direct'].items()})
    print('  direct over plain, paired:', {k: round(v, 4) for k, v in r['direct_over_plain'].items()})
    if 'aa' in r:
        print('  control, direct over direct-aa:', {k: round(v, 3) for k, v in r['aa'].items()})
    if 'latexmk_over_base_latexmk' in r:
        print('  latexmk over base-latexmk:', {k: round(v, 3) for k, v in r['latexmk_over_base_latexmk'].items()})
    for label, s in r['shares'].items():
        for kind in ('wall', 'cpu'):
            x = s[kind]
            print(f"  {label:13s} {kind:4s} total {x['total_ms']:9.1f} ms: " + ', '.join(f'{k} {x[k]:.1f}%' for k in ('save', 'build', 'read PDF', 'open', 'render')))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--engine', required=True)
    ap.add_argument('--base')
    ap.add_argument('--docs', default=','.join(DEFAULT_DOCS))
    ap.add_argument('--rounds', type=int, default=40)
    ap.add_argument('--long-rounds', type=int, default=6)
    ap.add_argument('--scale', type=float, default=2.0)
    ap.add_argument('--no-aa', action='store_true')
    ap.add_argument('--json')
    args = ap.parse_args()
    root = common.out_root()
    engine = proflib.Engine('engine', args.engine)
    base = proflib.Engine('base', args.base) if args.base else None
    pop, me = Poppler(), Self()
    results = []
    for doc in proflib.resolve_docs(args.docs.split(',')):
        r = run_doc(doc, args, engine, base, root, pop, me)
        show(r)
        results.append(r)
        sys.stdout.flush()
    if args.json:
        with open(args.json, 'w') as f:
            json.dump(results, f, indent=1, sort_keys=True)


if __name__ == '__main__':
    main()
