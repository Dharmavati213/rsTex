#!/usr/bin/env -S python3 -I
r"""commute_cost.py: what the commute command costs per diagram, DESIGN.md section 10.

    commute_cost.py --commute PATH [-n 5] [--timed 21] [--json FILE]

One diagram with an empty cache, so that the formats are built; one with the format cached; a file of 20 and a file of 200; and under a preamble, the run that writes a checkpoint and the run that restores it.
In user-mode instructions and cycles first, as for everything on this machine (bench.py says why), then wall, user and system time per diagram: the best and the median of --timed runs.  The cache is under target/.
The diagrams are a small square and four others with more in them, in turn.  None of it comes from a document.
"""
import argparse
import json
import os
import shutil
import statistics
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import proflib  # noqa: E402

BODIES = [
    r'''A \arrow[r, "f"] \arrow[d, "g"'] & B \arrow[d, "h"] \\ C \arrow[r, "k"'] & D''',
    r'''X \arrow[r, hook, "i"] \arrow[dr, "j"'] & Y \arrow[d, two heads, "p"] \\ & \mathbb{Z}''',
    r'''0 \arrow[r] & K \arrow[r] \arrow[d, "\alpha"] & M \arrow[r] \arrow[d, "\beta"] & N \arrow[r] \arrow[d, "\gamma"] & 0 \\ 0 \arrow[r] & K' \arrow[r] & M' \arrow[r] & N' \arrow[r] & 0''',
    r'''\mathcal{F} \arrow[r, bend left, "\phi"] \arrow[r, bend right, "\psi"'] & \mathcal{G} \arrow[r, dashed, "\otimes"] & \mathrm{Hom}(U, V)''',
    r'''E \arrow[d, Rightarrow] \arrow[r, "\pi"] & F \arrow[dl, equal] \\ \operatorname{Spec} R''',
]


def diagrams(n):
    return ''.join('\\begin{tikzcd}\n' + BODIES[k % len(BODIES)] + '\n\\end{tikzcd}\n' for k in range(n))


def row(name, r, n, runs):
    c = r['counters']
    wall = [x['wall_ms'] / n for x in runs]
    print(f'{name:<30} {n:>4} {c["instructions"]["value"] / n / 1e6:>9.3f} {c["cycles"]["value"] / n / 1e6:>9.3f} {r["minflt"] / n:>7.0f} {min(wall):>8.2f} {statistics.median(wall):>8.2f} '
          f'{statistics.median(x["user_ms"] / n for x in runs):>8.2f} {statistics.median(x["sys_ms"] / n for x in runs):>8.2f}')
    return {'name': name, 'diagrams': n, 'Minstr': c['instructions']['value'] / n / 1e6, 'Mcycles': c['cycles']['value'] / n / 1e6, 'faults': r['minflt'] / n, 'wall_min_ms': min(wall), 'wall_median_ms': statistics.median(wall)}


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('--commute', required=True)
    p.add_argument('-n', type=int, default=5)
    p.add_argument('--timed', type=int, default=21)
    p.add_argument('--json')
    a = p.parse_args()
    root = os.path.join(proflib.target_root(), 'commute-cost')
    shutil.rmtree(root, ignore_errors=True)
    os.makedirs(root)
    cache = os.path.join(root, 'cache')
    os.makedirs(cache)
    env = {k: v for k, v in os.environ.items() if not k.startswith('RSTEX_')}
    env['XDG_CACHE_HOME'] = cache
    exe = os.path.abspath(a.commute)

    def write(name, text):
        path = os.path.join(root, name)
        with open(path, 'w', encoding='utf-8') as f:
            f.write(text)
        return name

    results = []
    print(f'{"":<30} {"n":>4} {"Minstr":>9} {"Mcycles":>9} {"faults":>7} {"wall min":>8} {"wall med":>8} {"user":>8} {"sys":>8}   (per diagram, ms)')
    one = write('one.tex', diagrams(1))
    # Cold: each timed run has an empty cache, for three runs: it takes seconds.
    cold = []
    r = None
    for k in range(3):
        shutil.rmtree(cache, ignore_errors=True)
        os.makedirs(cache)
        if k == 0:
            r = proflib.run_counted([exe, one], root, env)
            continue
        cold.append(proflib.run_timed([exe, one], root, env))
    results.append(row('one diagram, cold format', r, 1, cold))
    for name, n in (('one diagram, format cached', 1), ('a file of 20', 20), ('a file of 200', 200)):
        f = write(f'n{n}.tex', diagrams(n))
        best = None
        for _ in range(a.n):
            c = proflib.run_counted([exe, f], root, env)
            if c['rc'] != 0:
                proflib.die(f'{name}: status {c["rc"]}')
            if best is None or c['counters']['instructions']['value'] < best['counters']['instructions']['value']:
                best = c
        runs = [proflib.run_timed([exe, f], root, env) for _ in range(a.timed if n < 200 else max(5, a.timed // 3))]
        results.append(row(name, best, n, runs))
    write('pre.tex', '\\newcommand{\\Mine}{\\mathbb{M}}\n')
    f = write('p.tex', '\\begin{tikzcd}\n\\Mine \\arrow[r] & B\n\\end{tikzcd}\n')
    argv = [exe, '--preamble', 'pre.tex', f]
    first = proflib.run_counted(argv, root, env)
    results.append(row('preamble, first run', first, 1, [first]))
    best = min((proflib.run_counted(argv, root, env) for _ in range(a.n)), key=lambda c: c['counters']['instructions']['value'])
    results.append(row('preamble, from the checkpoint', best, 1, [proflib.run_timed(argv, root, env) for _ in range(a.timed)]))
    if a.json:
        with open(a.json, 'w', encoding='utf-8') as f:
            json.dump(results, f, indent=1)
    shutil.rmtree(root, ignore_errors=True)


if __name__ == '__main__':
    sys.exit(main())
