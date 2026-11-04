#!/usr/bin/env -S python3 -I
"""If checkpoints were made with the format for a few fixed preamble prefixes: how many public documents begin with one, and how much of the preamble that is."""
import collections, os, re, sys
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import proflib
entries,_,_ = proflib.corpus().load_manifests(None)
docs = []
for e in entries:
    cat = e['id'].split('/')[0]
    if e.get('mode') != 'latex' or cat in ('coverage', 'l3build', 'cd', 'extra', 'user', 'local', 'cd-local', 'ladder'):
        continue
    try:
        t = open(e['main'], 'rb').read().decode('latin-1')
    except OSError:
        continue
    m = re.search(r'\\begin\{document\}', t)
    if not m or '\\documentclass' not in t[:m.start()]:
        continue
    docs.append(t[:m.start()])
print(len(docs), 'public documents with \\documentclass and \\begin{document} in the main file')
# by the line: a prefix must be the same bytes, to the end of a line
counts = collections.Counter()
for d in docs:
    lines = d.split('\n')
    pref = ''
    for k, l in enumerate(lines):
        pref += l + '\n'
        if '\\documentclass' in pref and l.strip() and not l.lstrip().startswith('%'):
            counts[pref] += 1
top = counts.most_common(4000)
# greedy: the N prefixes that cover most documents, each document taking its longest covered prefix
for n in (1, 5, 20, 100):
    chosen = []
    covered = set()
    rest = dict(counts)
    per = {}
    for i, d in enumerate(docs):
        pass
    remaining = list(range(len(docs)))
    for _ in range(n):
        c = collections.Counter()
        for i in remaining:
            lines = docs[i].split('\n'); pref = ''
            seen_first = False
            for l in lines:
                pref += l + '\n'
                if '\\documentclass' in pref and l.strip() and not l.lstrip().startswith('%'):
                    c[pref] += 1
        if not c:
            break
        p, v = c.most_common(1)[0]
        chosen.append(p)
        remaining = [i for i in remaining if not docs[i].startswith(p)]
    cov = len(docs) - len(remaining)
    print(f'{n:>4} prefixes cover {cov} of {len(docs)} documents, {100*cov/len(docs):.1f}%')
# how far beyond the class line a shared prefix goes: of those that share, how many \usepackage lines are in it
single = collections.Counter()
for p, v in counts.items():
    pk = p.count('\\usepackage')
    single[pk] = max(single[pk], v)
print('largest number of documents sharing a prefix with k \\usepackage in it:', sorted(single.items())[:8])
