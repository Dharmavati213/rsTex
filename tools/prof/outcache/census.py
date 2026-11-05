#!/usr/bin/env -S python3 -I
"""What the first lines of public documents look like: how many a checkpoint made with the format could serve.  Public entries only."""
import collections, os, re, sys
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import proflib
entries,_,_ = proflib.corpus().load_manifests(None)
cats = collections.Counter(); first = collections.Counter(); exact = collections.Counter(); prefix = collections.Counter(); pre = collections.Counter()
for e in entries:
    cat = e['id'].split('/')[0]
    if e.get('mode') != 'latex' or cat in ('coverage', 'l3build', 'cd', 'extra', 'user', 'local', 'cd-local', 'ladder'):
        continue
    try:
        t = open(e['main'], 'rb').read().decode('latin-1')
    except OSError:
        continue
    cats[cat] += 1
    lines = t.split('\n')
    i = 0
    while i < len(lines) and (not lines[i].strip() or lines[i].lstrip().startswith('%')):
        i += 1
    head = lines[i].strip() if i < len(lines) else ''
    m = re.match(r'\\documentclass(\[[^\]]*\])?\{([^}]*)\}(.*)$', head)
    if not m:
        first['other first line'] += 1
        continue
    first['class on line 1' if i == 0 else 'class after comments or blank lines'] += 1
    exact[(m.group(1) or '') + '{' + m.group(2) + '}'] += 1
    m2 = re.search(r'\\begin\{document\}', t)
    body = t[:m2.start()] if m2 else t
    pre['no package' if '\\usepackage' not in body and '\\input' not in body and '\\RequirePackage' not in body else 'has packages'] += 1
print(sum(cats.values()), 'documents', dict(cats))
print(first.most_common())
print(pre.most_common())
tot = sum(exact.values())
for k, v in exact.most_common(12):
    print(f'{100*v/tot:6.1f}%  {v:5d}  {k}')
