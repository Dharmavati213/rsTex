#!/usr/bin/env python3
"""analyze.py <dump files...>: aggregate prof.so dumps into phase table + function profile."""
import sys, os, re, bisect, collections, subprocess, json

S = os.path.dirname(os.path.abspath(__file__))
DBG = {
    'pdftex': f'{S}/dbg/pdftex.debug',
    'libz.so': f'{S}/dbg/libz.so.1/debug',
    'libkpathsea': f'{S}/dbg/libkpathsea.so.6/debug',
    'libc.so': f'{S}/dbg/libc.so.6/debug',
}

_symcache = {}
def symbols(path):
    if path in _symcache:
        return _symcache[path]
    out = subprocess.run(['nm', '-n', path], capture_output=True, text=True).stdout
    addrs, names = [], []
    for line in out.splitlines():
        p = line.split()
        if len(p) == 3 and p[1] in 'tTwWiI':
            addrs.append(int(p[0], 16)); names.append(p[2].split('.')[0] if '.cold' not in p[2] and '.part' not in p[2] else p[2].split('.')[0])
    _symcache[path] = (addrs, names)
    return addrs, names

class Run:
    pass

def load(fn):
    r = Run(); r.samples = []; r.events = []; r.c = {}; r.maps = []
    mode = None
    with open(fn, errors='replace') as f:
        for line in f:
            if line.startswith('MAPS'): mode = 'm'; continue
            if line.startswith('ENDMAPS'): mode = None; continue
            if mode == 'm':
                p = line.split()
                lo, hi = [int(x, 16) for x in p[0].split('-')]
                r.maps.append((lo, hi, p[1], int(p[2], 16), p[5] if len(p) > 5 else ''))
                continue
            if line.startswith('H '):
                p = line.split()
                r.h = {p[i]: int(p[i + 1]) for i in range(1, len(p) - 1, 2)}
            elif line.startswith('C '):
                p = line.split()
                r.c[p[1]] = (int(p[2]), int(p[3]), int(p[4]))
            elif line.startswith('E '):
                p = line.rstrip('\n').split(' ', 5)
                r.events.append((int(p[1]), int(p[2]), int(p[3]), int(p[4]), p[5]))
            elif line.startswith('S '):
                p = line.split()
                r.samples.append((int(p[1]), [int(x, 16) for x in p[2:]]))
    # base per object
    r.bases = {}
    for lo, hi, perm, off, path in r.maps:
        if path and path not in r.bases and off == 0:
            r.bases[path] = lo
    r.lo = [m[0] for m in r.maps]
    return r

def resolve(r, addr, cache):
    v = cache.get(addr)
    if v: return v
    i = bisect.bisect_right(r.lo, addr) - 1
    name = '?'
    if i >= 0 and addr < r.maps[i][1]:
        path = r.maps[i][4]
        base = r.bases.get(path, r.maps[i][0])
        off = addr - base
        short = os.path.basename(path)
        dbg = None
        for k, d in DBG.items():
            if k in short: dbg = d
        if dbg is None and path.startswith('/'):
            dbg = path if os.path.exists(path) else None
        name = short.split('.')[0] + '!?'
        if dbg:
            addrs, names = symbols(dbg)
            j = bisect.bisect_right(addrs, off) - 1
            if j >= 0:
                name = (short.split('.')[0] + '!' if 'pdftex' not in short else '') + names[j]
    cache[addr] = name
    return name

def first_ev(r, pred, after=0):
    for e in r.events:
        if e[0] >= after and pred(e):
            return e[0]
    return None

def segment(r, job):
    """return dict of phase boundaries (absolute ns)."""
    ev = r.events
    t = {}
    t['t0'] = r.h['t0']; t['ctor'] = r.h['ctor']; t['main'] = r.h['main']; t['end'] = r.h['end']
    t['cnf'] = first_ev(r, lambda e: e[1] == 10 and e[4].endswith('texmf.cnf'))
    t['lsr_first'] = first_ev(r, lambda e: e[1] == 10 and e[4].endswith('ls-R'))
    lsrs = [e[0] for e in ev if e[1] == 10 and (e[4].endswith('ls-R') or e[4].endswith('aliases'))]
    t['fmt'] = first_ev(r, lambda e: e[1] == 10 and e[4].endswith('.fmt') and e[2] == 1)
    t['fmt_end'] = first_ev(r, lambda e: e[1] == 21)
    auxr = [e[0] for e in ev if e[1] == 10 and e[4].endswith(job + '.aux') and e[2] == 1]
    t['begindoc'] = auxr[0] if auxr else None
    t['enddoc'] = auxr[-1] if len(auxr) >= 2 else None
    t['map'] = first_ev(r, lambda e: e[1] == 10 and e[4].endswith('pdftex.map') and e[2] == 1)
    t['firstpfb'] = first_ev(r, lambda e: e[1] == 10 and e[4].endswith('.pfb') and e[2] == 1)
    t['nlsr'] = len(lsrs)
    return t

def analyze(files, job, interval_us=int(os.environ.get('INTERVAL','500')), topn=45):
    runs = [load(f) for f in files]
    phases = collections.Counter(); bound = collections.defaultdict(list)
    self_prof = collections.Counter(); incl = collections.Counter()
    ph_self = collections.defaultdict(collections.Counter)
    ph_incl = collections.defaultdict(collections.Counter)
    nr = len(runs)
    tot_samp = 0
    cats = collections.defaultdict(collections.Counter)
    cnt_c = collections.defaultdict(list)
    for r in runs:
        cache = {}
        t = segment(r, job)
        for k, v in t.items():
            if v is not None: bound[k].append(v - t['t0'] if k != 'nlsr' else v)
        for k, v in r.c.items():
            cnt_c[k].append(v)
        tb = [('main_to_cnf', t['main']), ('kpse_cnf_lsr', t['cnf'] or t['main']), ('fmt_undump', t['fmt']), ('preamble', t['fmt_end']), ('body', t['begindoc']), ('enddoc_after_lastpage', t['enddoc'] or t['end'])]
        for ts, stack in r.samples:
            tot_samp += 1
            ph = 'end'
            for name, tt in reversed(tb):
                if tt is not None and ts >= tt:
                    ph = name; break
            names = [resolve(r, a, cache) for a in stack]
            # dedupe consecutive
            leaf = names[0]
            self_prof[leaf] += 1; ph_self[ph][leaf] += 1
            for n in set(names):
                incl[n] += 1; ph_incl[ph][n] += 1
            cats[ph][category(names)] += 1
    w = interval_us / 1000.0 / nr
    res = {}
    print(f"# {job}: runs={nr} samples={tot_samp} interval={interval_us}us")
    b = {k: sorted(v)[len(v) // 2] / 1e6 for k, v in bound.items() if k != 'nlsr'}
    print("# boundaries (median ms since spawn):", {k: round(v, 2) for k, v in sorted(b.items(), key=lambda kv: kv[1])})
    print("# wrapped libc/zlib totals (ms, median):")
    for k, v in sorted(cnt_c.items()):
        v = sorted(v, key=lambda x: x[1])[len(v) // 2]
        if v[0]: print(f"   {k:9s} calls {v[0]:7d}  total {v[1]/1e6:8.2f} ms  bytes {v[2]}")
    print("# phase time from samples (ms/run):")
    order = ['main_to_cnf', 'kpse_cnf_lsr', 'fmt_undump', 'preamble', 'body', 'enddoc_after_lastpage']
    for ph in order:
        tot = sum(cats[ph].values()) * w
        print(f"  {ph:24s} {tot:9.1f}")
        for c, n in cats[ph].most_common(14):
            print(f"      {c:40s} {n*w:9.1f}")
    print(f"# self (top {topn}), ms/run")
    for n, c in self_prof.most_common(topn):
        print(f"  {c*w:8.1f}  {n}")
    print(f"# inclusive (top {topn}), ms/run")
    for n, c in incl.most_common(topn):
        print(f"  {c*w:8.1f}  {n}")
    res = dict(job=job, runs=nr, boundaries=b, cats={k: {c: n * w for c, n in v.items()} for k, v in cats.items()}, self={n: c * w for n, c in self_prof.most_common(150)}, incl={n: c * w for n, c in incl.most_common(150)}, ph_self={ph: {n: c * w for n, c in v.most_common(30)} for ph, v in ph_self.items()})
    return res

def category(names):
    S = set(names)
    anyz = any(n.startswith('libz!') for n in names)
    if 'loadfmtfile' in S:
        return 'undump:inflate' if anyz else 'undump:other'
    if 'writezip' in S or any(n.startswith('libz!deflate') or n=='libz!longest_match' for n in names):
        if 'writet1' in S or 'write_fontfile' in S: return 'compress:font-streams'
        if 'zpdfshipout' in S: return 'compress:page-streams'
        return 'compress:other(objstm/xmp)'
    if 'fm_read_info' in S or 'fm_scan_line' in S: return 'pdf:map-file-parse'
    if 'closefilesandterminate' in S:
        if 'writet1' in S: return 'finish:t1-subset(writet1)'
        if 'write_fontfile' in S: return 'finish:font-other'
        if any(n.startswith('write_font') or n in ('create_fontdescriptor','write_tounicode','load_enc_file','write_charwidth_array','create_charwidth_array') for n in names): return 'finish:font-dicts'
        if any(n.startswith('libkpathsea') for n in names): return 'finish:kpathsea'
        return 'finish:other(objects,xref,outlines)'
    if 'zpdfshipout' in S:
        if 'zpdfwriteimage' in S or 'zoutimage' in S: return 'shipout:image'
        if 'zdovf' in S: return 'shipout:vf'
        if any(n.startswith('libkpathsea') for n in names): return 'shipout:kpathsea'
        return 'shipout:nodes->pdf'
    if any(n.startswith('libkpathsea') for n in names): return 'kpathsea(any)'
    if 'zreadfontinfo' in S: return 'tex:font-load(tfm)'
    if 'zlinebreak' in S:
        if 'zhpack' in S: return 'tex:linebreak>hpack'
        return 'tex:linebreak'
    if 'mlisttohlist' in S: return 'tex:mlist_to_hlist'
    if 'buildpage' in S: return 'tex:page-builder'
    if 'zhpack' in S: return 'tex:hpack'
    if 'zvpackage' in S: return 'tex:vpack'
    if 'input_line' in S: return 'tex:input_line'
    if 'startinput' in S: return 'tex:startinput(fopen,etc)'
    leaf = names[0]
    if leaf.startswith('libc'): return 'libc-other:' + leaf.split('!')[-1][:24]
    if '!' in leaf: return 'other-lib:' + leaf[:30]
    return 'tex:interp(macro/expand/assign)'

if __name__ == '__main__':
    job = sys.argv[1]; files = sys.argv[2:]
    res = analyze(files, job)
    json.dump(res, open(f'{S}/result-{job}.json', 'w'), indent=1)
