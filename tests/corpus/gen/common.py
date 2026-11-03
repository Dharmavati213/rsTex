"""Shared helpers of the coverage generators.  Standard library only.

Everything generated goes under target/corpus/gen/ and is not committed.  Manifest paths are repo-relative, so the
manifest stays free of machine-specific paths.  Each entry gets its own directory: run.py hashes the names in an
entry's cwd, and a document in a shared directory could also be picked up by another document's \\input.
"""
import hashlib
import json
import os
import re
import shutil
import subprocess

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
GEN_ROOT = os.path.join(REPO, 'target', 'corpus', 'gen')
MANIFEST = os.path.join(REPO, 'tests', 'corpus', 'manifests', 'coverage.json')
TEXMF_DIST = '/usr/share/texmf-dist'
ORACLE_ENV = {'SOURCE_DATE_EPOCH': '1700000000', 'FORCE_SOURCE_DATE': '1', 'TZ': 'UTC', 'LC_ALL': 'C', 'LANG': 'C',
              'openout_any': 'p', 'openin_any': 'p', 'MKTEXTFM': '0', 'MKTEXPK': '0', 'MKTEXMF': '0', 'MKTEXTEX': '0'}


def rel(path):
    return os.path.relpath(path, REPO)


def safe_name(name):
    """A directory name that keeps the original readable and stays unique when characters had to be replaced."""
    clean = re.sub(r'[^A-Za-z0-9._+-]', '_', name)
    if clean == name and name not in ('.', '..') and not name.startswith('-'):
        return name
    return f'x{hashlib.sha1(name.encode("latin-1")).hexdigest()[:8]}-{clean.lstrip("-.")}'


def write_doc(category, name, text, filename='doc.tex'):
    """Write target/corpus/gen/<category>/<name>/<filename> unless it is already there with these bytes, so that the
    mtime and with it run.py's hash cache survive a regeneration.  Returns the path."""
    d = os.path.join(GEN_ROOT, category, safe_name(name))
    os.makedirs(d, exist_ok=True)
    p = os.path.join(d, filename)
    data = text.encode('latin-1') if isinstance(text, str) else text
    try:
        with open(p, 'rb') as f:
            if f.read() == data:
                return p
    except OSError:
        pass
    tmp = p + '.tmp'
    with open(tmp, 'wb') as f:
        f.write(data)
    os.replace(tmp, p)
    return p


def clean_category(category, keep):
    """Remove generated directories of a category that the current generation did not produce."""
    root = os.path.join(GEN_ROOT, category)
    keep = {safe_name(k) for k in keep}
    try:
        names = os.listdir(root)
    except OSError:
        return
    for n in names:
        p = os.path.join(root, n)
        if n not in keep and os.path.isdir(p):
            shutil.rmtree(p, ignore_errors=True)


def oracle_env():
    env = {k: os.environ[k] for k in ('PATH', 'HOME') if k in os.environ}
    env.update(ORACLE_ENV)
    return env


def run_pdftex(doc, outdir, fmt='pdftex', timeout=120, prelude=None):
    """One oracle compile with the same settings as run.py.  Returns (exit status or None on timeout, log bytes)."""
    os.makedirs(outdir, exist_ok=True)
    exe = 'pdflatex' if fmt == 'pdflatex' else 'pdftex'
    target = os.path.basename(doc)
    argv = [exe, '-fmt=' + fmt, '-interaction=batchmode', '-no-shell-escape', '-output-directory=' + outdir,
            (prelude + '\\input "' + target + '"') if prelude else target]
    try:
        p = subprocess.run(argv, cwd=os.path.dirname(doc), env=oracle_env(), stdin=subprocess.DEVNULL,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=timeout)
        rc = p.returncode
    except subprocess.TimeoutExpired:
        rc = None
    job = os.path.splitext(target)[0]
    try:
        with open(os.path.join(outdir, job + '.log'), 'rb') as f:
            return rc, f.read()
    except OSError:
        return rc, b''


def entry(eid, main, **fields):
    e = {'id': eid, 'main': rel(main)}
    e.update(fields)
    return e


def stride_sample(items, n):
    """n items spread evenly over a sorted list, deterministic: the choice is not tuned to anything."""
    items = list(items)
    if n >= len(items):
        return items
    return [items[(i * len(items)) // n] for i in range(n)]


def write_json(path, obj, indent=None):
    tmp = path + '.tmp'
    with open(tmp, 'w') as f:
        json.dump(obj, f, indent=indent, sort_keys=False)
        f.write('\n')
    os.replace(tmp, path)
