#!/usr/bin/env -S python3
"""resume_state.py: could output stop early after an edit?  The whole state at a page boundary, after an edit, against the old run's at the same page.  A prototype's tool.

    resume_state.py --engine PATH --docs main,... [--at 0.5] [--types letter,word] [--pages 12]

As resume_edit.py, the document's directory is seen through links and never written to.  Two runs with RSTEX_PAGE_DUMP, one on the document and one on the edited document, each leave the image at
each page from the edit on; numpy says how many bytes differ in each section, which is what a test of equality at a page boundary would have to see as none.  Then the page streams, for how
many pages after the edit come out the same.  Only numbers are printed.
"""
import argparse
import os
import shutil
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import proflib  # noqa: E402
import resume as resume_tool  # noqa: E402
import resume_edit as E  # noqa: E402


def load(d, page):
    idx = {}
    p = os.path.join(d, f'p{page}.idx')
    if not os.path.exists(p):
        return None, None
    with open(p) as f:
        for line in f:
            if line.startswith('field '):
                continue
            name, off, n, at = line.split()
            idx[name] = (int(off), int(n), int(at))
    img = np.memmap(os.path.join(d, f'p{page}.img'), dtype=np.uint8, mode='r')
    return idx, img


def compare(a, b, page):
    ia, ma = load(a, page)
    ib, mb = load(b, page)
    if ia is None or ib is None:
        return None
    res = {}
    for name in sorted(set(ia) | set(ib)):
        if name not in ia or name not in ib:
            res[name] = ('only one', 0)
            continue
        (oa, na, ata), (ob, nb, atb) = ia[name], ib[name]
        # Over the same image offsets.
        lo, hi = max(oa, ob), min(oa + na, ob + nb)
        d = 0
        if hi > lo:
            xa = ma[ata + lo - oa:ata + hi - oa]
            xb = mb[atb + lo - ob:atb + hi - ob]
            d = int(np.count_nonzero(xa != xb))
        extra = abs(na - nb)
        if d or extra:
            res[name] = (d, extra)
    return res


def one(engine, doc, args):
    doc.prepare()
    root = os.path.join(proflib.out_root(), 'resume-state', doc.name.replace('/', '_'))
    shutil.rmtree(root, ignore_errors=True)
    src = os.path.join(root, 'src')
    E.farm(doc.entry['cwd'], src)
    e2 = dict(doc.entry, cwd=src)
    d2 = E.Doc2(doc.name, e2)
    d2.meta = doc.meta
    R = resume_tool.Runner(engine, d2, root)
    print(doc.name)
    # Where the records are.
    t = R.run({'RSTEX_PAGE_CK': '1', 'RSTEX_CHECKPOINT_TRACE': '1', 'RSTEX_PAGE_INPUTS': '1'}, counted=False)
    where = []
    import re
    for line in t['trace']:
        m = re.match(r'pages: rec (\d+) page (\d+) at (\d+) (.*)$', line)
        if m:
            where.append((int(m.group(1)), int(m.group(2)), int(m.group(3)), m.group(4)))
    n = len(where)
    for frac in [float(x) for x in args.at.split(',')]:
        for kind in args.types.split(','):
            found = None
            for k in range(max(0, min(n - 1, int(frac * n))), n):
                _, page, pos, path = where[k]
                full = os.path.join(src, path) if not os.path.isabs(path) else path
                if not os.path.normpath(full).startswith(src):
                    continue
                with open(full, 'rb') as f:
                    data = f.read()
                got = E.edit_bytes(data, pos, kind)
                if got:
                    found = (page, full, got[0])
                    break
            if not found:
                print(f'  {kind}@{frac}: no prose line')
                continue
            page, full, edited = found
            first, last = page + 1, page + args.pages
            da = os.path.join(root, 'dump-a')
            db = os.path.join(root, 'dump-b')
            for d in (da, db):
                shutil.rmtree(d, ignore_errors=True)
                os.makedirs(d)
            R.drop_chain()
            R.run({'RSTEX_PAGE_CK': '1', 'RSTEX_PAGE_DUMP': f'{da}:{first}:{last}'}, counted=False)
            sa = resume_tool.state(R.work)
            os.unlink(full)
            with open(full, 'wb') as f:
                f.write(edited)
            try:
                R.drop_chain()
                R.run({'RSTEX_PAGE_CK': '1', 'RSTEX_PAGE_DUMP': f'{db}:{first}:{last}'}, counted=False)
                sb = resume_tool.state(R.work)
            finally:
                os.unlink(full)
                os.symlink(os.path.join(doc.entry['cwd'], os.path.relpath(full, src)), full)
            pa = E.page_streams(next((v for k_, v in sa.items() if k_.endswith('.pdf')), b''))
            pb = E.page_streams(next((v for k_, v in sb.items() if k_.endswith('.pdf')), b''))
            fd, left, tail = E.stream_diff(pa, pb)
            aux = any(sb.get(x) != sa.get(x) for x in sb if x.endswith(('.aux', '.toc', '.out', '.lof', '.lot')))
            print(f'  {kind}@{frac}: edit after page {page}; page streams {len(pa)}->{len(pb)}, first that differs {fd}, differ {left - tail}, same to the end {tail}; side files changed: {aux}')
            for q in range(first, last + 1):
                r = compare(da, db, q)
                if r is None:
                    continue
                total = sum(x[0] if isinstance(x[0], int) else 0 for x in r.values())
                parts = ' '.join(f'{k}={v[0]}' + (f'+{v[1]}' if v[1] else '') for k, v in sorted(r.items(), key=lambda kv: -(kv[1][0] if isinstance(kv[1][0], int) else 0))[:8])
                print(f'    page {q}: {total} bytes differ: {parts}')
            shutil.rmtree(da, ignore_errors=True)
            shutil.rmtree(db, ignore_errors=True)
            sys.stdout.flush()
    if not args.keep:
        shutil.rmtree(root, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--engine', required=True)
    ap.add_argument('--docs', required=True)
    ap.add_argument('--at', default='0.5')
    ap.add_argument('--types', default='letter,word')
    ap.add_argument('--pages', type=int, default=12)
    ap.add_argument('--keep', action='store_true')
    args = ap.parse_args()
    engine = proflib.Engine('new', args.engine)
    for doc in proflib.resolve_docs(args.docs.split(',')):
        one(engine, doc, args)


if __name__ == '__main__':
    main()
