"""What the tools of tests/until_stable and tools/prof/loop.py share: a document's command line as Lemma has it, for latexmk and for `rstex --until-stable`, and the programs that
rstex only names.  Everything a run writes stays under target/.  Nothing here prints a file name of a private document or anything from one: a caller gets statuses, counts and extensions.
"""
import os
import re
import shutil
import subprocess
import sys

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO, 'tools', 'prof'))
import proflib  # noqa: E402

# What crates/lemma-build/src/request.rs puts in the environment and on the command line.
LEMMA_ENV = {'max_print_line': '10000', 'error_line': '254', 'half_error_line': '238'}
LEMMA_FLAGS = ['-synctex=1', '-interaction=nonstopmode', '-file-line-error', '-recorder']
TOOL_LINE = re.compile(rb'^rstex: until-stable: tool=(\S+) reason=(\S+) file=(.*)$', re.M)
PASSES_LINE = re.compile(rb'^rstex: until-stable: passes=(\d+) result=(\w+)', re.M)
# Files that are not the engine's: the two drivers' own, and what the programs say on the way.
NOT_COMPARED = ('.fdb_latexmk', '.rstex-tools', '.blg', '.ilg')
STATUS_TOOL = 10


# The public documents of the corpus manifests.
PUBLIC = ('arxiv/', 'books/', 'docs/', 'manuals/', 'notes/')


def looks_like(entry, tool):
    """True if a public document's main file says that it uses the program."""
    try:
        with open(entry['main'], 'rb') as f:
            text = f.read()
    except OSError:
        return False
    text = re.sub(rb'(?m)(?<!\\)%.*$', b'', text)
    if tool == 'makeindex':
        return b'\\makeindex' in text and b'\\printindex' in text
    has_biblatex = re.search(rb'\\usepackage(\[[^]]*\])?\{biblatex\}', text) is not None
    if tool == 'biber':
        return has_biblatex
    return not has_biblatex and b'\\bibliography{' in text and any(f.endswith('.bib') for f in os.listdir(entry['cwd']))


def out_root():
    root = os.path.join(os.path.realpath(os.path.join(REPO, 'target')), 'until-stable')
    os.makedirs(root, exist_ok=True)
    return root


def document_env(entry, cache):
    """The environment of a document under the two drivers: what corpus runs pin, with Lemma's lines, and no paranoia about paths, which bibtex under latexmk would trip on."""
    env = proflib.corpus().entry_env(entry)
    env.update(LEMMA_ENV)
    env.pop('openin_any', None)
    env.pop('openout_any', None)
    env['XDG_CACHE_HOME'] = cache
    return env


def tool_env(env, cwd):
    """A program runs where the outputs are, and finds what lies beside the document as latexmk tells it to."""
    e = dict(env)
    e['BIBINPUTS'] = cwd + ':'
    e['BSTINPUTS'] = cwd + ':'
    return e


def run_tool(tool, file, out, cwd, env, log):
    """What the line says, in the output directory: bibtex and biber take the name without its directory, as latexmk gives it to them there.  True if it went well enough to go on:
    bibtex says 1 for warnings, and latexmk stops at more than that, as it does for the others at anything but 0."""
    name = os.path.relpath(os.path.join(cwd, os.fsdecode(file)), out)
    if tool == 'makeindex':
        argv = ['makeindex', '-q', name]
    elif tool == 'bibtex':
        argv = ['bibtex', name[:-4] if name.endswith('.aux') else name]
    else:
        argv = ['biber', '--quiet', name]
    rc = subprocess.run(argv, cwd=out, env=tool_env(env, cwd), stdin=subprocess.DEVNULL, stdout=log, stderr=log, check=False).returncode
    return rc <= (1 if tool == 'bibtex' else 0)


def outputs(out):
    """Relative path -> bytes of everything in the output directory that is the engine's."""
    res = {}
    for root, _, files in os.walk(out):
        for f in files:
            if f.endswith(NOT_COMPARED):
                continue
            p = os.path.join(root, f)
            with open(p, 'rb') as fh:
                res[os.path.relpath(p, out)] = fh.read()
    return res


def latexmk_only(latexmk, direct, job):
    """Takes out what latexmk leaves and rstex rightly does not: the `.aux` it writes before a first run, if the document never wrote one, as its set_trivial_aux_fdb does, and a `.ind` that
    makeindex made and no pass read, since latexmk runs it for any `.idx`.  Says how many of the second kind."""
    aux = job + '.aux'
    if aux not in direct and latexmk.get(aux) == b'\\relax \n\\gdef \\@abspage@last{1}\n':
        del latexmk[aux]
    unread = [p for p in latexmk if p.endswith('.ind') and p not in direct]
    for p in unread:
        del latexmk[p]
    return len(unread)


def differing(a, b):
    """The extensions of the files that differ or that only one side has, with a count of each: no names."""
    res = {}
    for p in sorted(set(a) | set(b)):
        if a.get(p) != b.get(p):
            ext = os.path.splitext(p)[1] or '(none)'
            res[ext] = res.get(ext, 0) + 1
    return res


def reset(out):
    shutil.rmtree(out, ignore_errors=True)
    os.makedirs(out)


def mirror_dirs(src, out):
    """A \\include'd file's directory under the output directory is the caller's to make: Lemma does, from the sources.  Through links, as far as a document's own tree can reasonably go."""
    for root, dirs, _ in os.walk(src, followlinks=True):
        rel = os.path.relpath(root, src)
        dirs[:] = [d for d in dirs if not d.startswith('.') and rel.count(os.sep) < 6]
        if rel != '.':
            os.makedirs(os.path.join(out, rel), exist_ok=True)
