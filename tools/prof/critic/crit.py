"""oc_adv.py's helpers: formats per binary through proflib, runs in fresh directories.  RSTEX_CRITIC_ENGINE is the binary judged."""
import os, subprocess, sys, secrets
sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
import proflib
WORK = os.path.join(proflib.out_root(), 'critic')
os.makedirs(WORK, exist_ok=True)

def engine(name):
    return proflib.Engine(name, os.environ['RSTEX_CRITIC_ENGINE'])

def base_env(cache):
    c = proflib.corpus()
    env = {k: os.environ[k] for k in ('PATH', 'HOME') if k in os.environ}
    env.update(c.DETERMINISM); env.update(c.NO_MKTEX); env.update(c.HERMETIC)
    env['openout_any'] = 'p'
    env['XDG_CACHE_HOME'] = cache
    return env

def fresh(tag):
    d = os.path.join(WORK, tag + '-' + secrets.token_hex(4))
    os.makedirs(d)
    return d

def snapshot(d):
    res = {}
    for root, _, names in os.walk(d):
        for n in names:
            p = os.path.join(root, n)
            with open(p, 'rb') as f:
                res[os.path.relpath(p, d)] = f.read()
    return res

def differs(a, b, skip=()):
    out = []
    for name in sorted(set(a) | set(b)):
        if name in skip: continue
        if a.get(name) != b.get(name):
            ext = name.rsplit('.', 1)[-1]
            if name not in a or name not in b:
                out.append(ext + ':absent')
            else:
                x, y = a[name], b[name]
                n = min(len(x), len(y))
                k = next((i for i in range(n) if x[i] != y[i]), n)
                out.append(f'{ext}:{len(x)}/{len(y)}@{k}')
    return out

def latex_argv(eng, main, out, extra=()):
    return [eng.path, '-progname=pdflatex', '-fmt=' + eng.format_path('pdflatex'), '-interaction=batchmode', '-no-shell-escape', '-output-directory=' + out] + list(extra) + [main]

def run(argv, cwd, env):
    p = subprocess.run(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return p.returncode, p.stdout, p.stderr.decode('latin-1')
