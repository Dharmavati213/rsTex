#!/usr/bin/env -S python3 -I
"""The arXiv source of the corpus: a sample nobody tuned the engine against.

    python3 -I tests/corpus/arxiv/sample.py select            # API only; writes tests/corpus/arxiv/selection.json
    python3 -I tests/corpus/arxiv/sample.py fetch             # downloads e-prints, sequentially, into refs/corpus/arxiv/raw
    python3 -I tests/corpus/arxiv/sample.py extract           # unpacks into refs/corpus/arxiv/papers/<id>, finds main files
    python3 -I tests/corpus/arxiv/sample.py manifest          # writes tests/corpus/manifests/arxiv.json
    python3 -I tests/corpus/run.py prepare --manifest tests/corpus/manifests/arxiv.json -j 8
    python3 -I tests/corpus/arxiv/sample.py finalize RUN_DIR  # tiers, expected classes and reasons, from that prepare run
    python3 -I tests/corpus/arxiv/sample.py report RUN_DIR    # requested, obtained, compiled; reasons; distributions

The order is the point.  `select` reads metadata only (id, version, primary category, date), so the sample is fixed before any
content is seen, and a paper that turns out to be awkward is recorded rather than replaced.  A fresh clone has selection.json and
the manifest but no papers: `fetch` and `extract` rebuild them from the pinned versions.

Everything downloaded is untrusted data.  This file never runs anything from a download, reads it only as bytes, and runs under
`python3 -I`.  It is kept outside the download directory, which it takes as --dir.
"""
import argparse
import datetime as dt
import gzip
import hashlib
import io
import json
import os
import re
import shutil
import sys
import tarfile
import time
import urllib.error
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
DEFAULT_DIR = os.path.join(REPO, 'refs', 'corpus', 'arxiv')
SELECTION = os.path.join(HERE, 'selection.json')
MANIFEST = os.path.join(REPO, 'tests', 'corpus', 'manifests', 'arxiv.json')

USER_AGENT = 'rstex-corpus-sampler/1.0 (TeX engine regression testing; sequential requests, 3 s apart)'
API = 'https://export.arxiv.org/api/query'
EPRINT = 'https://export.arxiv.org/e-print/'
MIN_GAP = 3.1                  # seconds from the end of one request to the start of the next, across all requests and processes
MAX_DOWNLOAD = 30 * 1024 * 1024
MAX_UNPACKED = 256 * 1024 * 1024
MAX_MEMBERS = 20000
MAX_SCAN = 32 * 1024 * 1024    # a .tex larger than this is not read when looking for the main file

T0 = dt.datetime(2015, 1, 1, tzinfo=dt.timezone.utc)
T1 = dt.datetime(2026, 11, 1, tzinfo=dt.timezone.utc)   # exclusive: the last whole month before the sample was drawn

# (stratum, quota, categories).  A stratum with several categories cycles through them window by window, so that a large subject
# cannot crowd out small ones.  The lists are fixed here, ahead of any content; they aim at diversity of classes, not at any paper.
STRATA = [
    ('math.AG', 80, ['math.AG']),
    ('math.CT', 40, ['math.CT']),
    ('math.AT', 30, ['math.AT']),
    ('math.NT', 30, ['math.NT']),
    ('math.RT', 20, ['math.RT']),
    ('math.DG', 20, ['math.DG']),
    ('math.AC', 20, ['math.AC']),
    ('math.PR', 15, ['math.PR']),
    ('math.CO', 15, ['math.CO']),
    ('math.AP', 15, ['math.AP']),
    ('math.LO', 15, ['math.LO']),
    ('cs', 40, ['cs.AI', 'cs.AR', 'cs.CC', 'cs.CL', 'cs.CR', 'cs.CV', 'cs.DB', 'cs.DC', 'cs.DS', 'cs.GT',
                'cs.HC', 'cs.IR', 'cs.IT', 'cs.LG', 'cs.LO', 'cs.NI', 'cs.PL', 'cs.RO', 'cs.SE', 'cs.SI']),
    ('hep-th', 15, ['hep-th']),
    ('hep-ph', 15, ['hep-ph']),
    ('stat', 15, ['stat.ME', 'stat.AP', 'stat.ML', 'stat.CO']),
    ('q-bio', 15, ['q-bio.QM', 'q-bio.PE', 'q-bio.NC', 'q-bio.BM', 'q-bio.GN', 'q-bio.MN']),
]
CATEGORY_STRATUM = {c: s for s, _, cats in STRATA for c in cats}
RULE = ('For a stratum of quota q, cut [2015-01-01, 2026-11-01) UTC into q equal windows. In window i take category '
        'categories[i mod len(categories)] and query cat:C AND submittedDate:[midpoint TO window end] sorted by submittedDate ascending. '
        'The pick is the first result whose primary category is exactly C and which is not already picked. If the second half of the '
        'window has none, the same is done on the first half, sorted descending, from the midpoint backwards. No filter looks at anything '
        'but id, primary category and date. A pick that later proves unusable is recorded and never replaced. The version is the latest '
        'at selection time and is pinned.')

ID_RE = re.compile(r'^\d{4}\.\d{4,5}v\d+$')
ATOM = '{http://www.w3.org/2005/Atom}'
ARXIV = '{http://arxiv.org/schemas/atom}'


# ------------------------------------------------------------------------------------------------ polite HTTP
def lower(headers):
    """HTTP/2 proxies send lower-case names, and a plain dict is case-sensitive."""
    return {k.lower(): v for k, v in headers.items()}


class Http:
    """One request at a time, with the gap kept across processes through a timestamp file."""

    def __init__(self, stamp):
        self.stamp = stamp
        self.requests = 0

    def _wait(self):
        try:
            last = os.path.getmtime(self.stamp)
        except OSError:
            last = 0
        gap = last + MIN_GAP - time.time()
        if gap > 0:
            time.sleep(gap)

    def _done(self):
        os.makedirs(os.path.dirname(self.stamp), exist_ok=True)
        with open(self.stamp, 'w'):
            pass
        os.utime(self.stamp)

    def get(self, url, limit=None, sink=None):
        """Returns (status, headers, body).  With limit, stops reading past it and returns status 'too-large'.
        Retries transient failures with a long backoff; each attempt counts as a request and keeps the gap."""
        delay = 30
        for attempt in range(6):
            self._wait()
            self.requests += 1
            try:
                req = urllib.request.Request(url, headers={'User-Agent': USER_AGENT})
                with urllib.request.urlopen(req, timeout=120) as r:
                    length = r.headers.get('Content-Length')
                    if limit is not None and length and int(length) > limit:
                        return 'too-large', lower(r.headers), None
                    buf, total = [], 0
                    while True:
                        chunk = r.read(1 << 16)
                        if not chunk:
                            break
                        total += len(chunk)
                        if limit is not None and total > limit:
                            return 'too-large', lower(r.headers), None
                        buf.append(chunk)
                    return r.status, lower(r.headers), b''.join(buf)
            except urllib.error.HTTPError as e:
                if e.code == 429 or e.code >= 500:
                    retry = e.headers.get('Retry-After', '')
                    wait = int(retry) if retry.isdigit() else delay
                    print(f'  HTTP {e.code}, waiting {wait} s (attempt {attempt + 1})', file=sys.stderr, flush=True)
                    self._done()
                    time.sleep(min(wait, 900))
                    delay *= 2
                    continue
                return e.code, lower(e.headers), None
            except (urllib.error.URLError, TimeoutError, ConnectionError, OSError) as e:
                print(f'  {type(e).__name__}: {e}, waiting {delay} s (attempt {attempt + 1})', file=sys.stderr, flush=True)
                self._done()
                time.sleep(delay)
                delay *= 2
                continue
            finally:
                self._done()
        return 'gave-up', {}, None


# ------------------------------------------------------------------------------------------------ selection
def windows(q):
    span = T1 - T0
    res = []
    for i in range(q):
        a = T0 + span * i / q
        b = T0 + span * (i + 1) / q
        res.append((a.replace(second=0, microsecond=0), b.replace(second=0, microsecond=0)))
    return res


def fmt_time(t):
    return t.strftime('%Y%m%d%H%M')


def api_page(http, cat, lo, hi, order, start, size):
    query = f'cat:{cat} AND submittedDate:[{fmt_time(lo)} TO {fmt_time(hi)}]'
    url = API + '?' + urllib.parse.urlencode({'search_query': query, 'sortBy': 'submittedDate', 'sortOrder': order,
                                              'start': start, 'max_results': size})
    for attempt in range(4):
        status, _, body = http.get(url)
        if status != 200 or body is None:
            raise RuntimeError(f'API returned {status} for {query}')
        root = ET.fromstring(body)
        total = int(root.find('{http://a9.com/-/spec/opensearch/1.1/}totalResults').text)
        entries = []
        for en in root.findall(ATOM + 'entry'):
            eid = en.find(ATOM + 'id').text.rsplit('/abs/', 1)[-1]
            prim = en.find(ARXIV + 'primary_category')
            entries.append({'id': eid, 'primary': prim.get('term') if prim is not None else None,
                            'published': en.find(ATOM + 'published').text})
        if entries or total <= start:
            return total, entries
        print(f'  empty page despite {total} results, retrying', file=sys.stderr, flush=True)
    return total, entries


def pick(http, cat, lo, hi, order, taken):
    """First result on this side of the midpoint with primary category cat that is not taken."""
    start, walked, size = 0, 0, 25
    while True:
        total, entries = api_page(http, cat, lo, hi, order, start, size)
        for en in entries:
            if en['primary'] == cat and ID_RE.match(en['id']) and en['id'].split('v')[0] not in taken:
                return en, walked, total
            walked += 1
        start += size
        if not entries or start >= total:
            return None, walked, total


def cmd_select(args):
    if os.path.exists(args.selection):
        sel = read_json(args.selection)
        if sel.get('rule') != RULE or sel.get('strata') != [list(s[:2]) + [s[2]] for s in STRATA]:
            sys.exit(f'{args.selection} was drawn under a different rule; move it away to draw again')
    else:
        sel = {'rule': RULE, 'window_start': T0.isoformat(), 'window_end_exclusive': T1.isoformat(),
               'strata': [list(s[:2]) + [s[2]] for s in STRATA], 'drawn': dt.date.today().isoformat(),
               'picks': []}
    done = {(p['stratum'], p['window_index']) for p in sel['picks']}
    taken = {p['id'].split('v')[0] for p in sel['picks'] if p.get('id')}
    http = Http(os.path.join(args.dir, '.last-request'))
    for stratum, q, cats in STRATA:
        for i, (a, b) in enumerate(windows(q)):
            if (stratum, i) in done:
                continue
            cat = cats[i % len(cats)]
            mid = a + (b - a) / 2
            mid = mid.replace(second=0, microsecond=0)
            end = b - dt.timedelta(minutes=1)
            en, walked, total = pick(http, cat, mid, end, 'ascending', taken)
            side = 'after-midpoint'
            if en is None:
                en, walked, total = pick(http, cat, a, mid - dt.timedelta(minutes=1), 'descending', taken)
                side = 'before-midpoint'
            rec = {'stratum': stratum, 'window_index': i, 'category': cat, 'window': [a.isoformat(), b.isoformat()]}
            if en is None:
                rec.update(id=None, unfilled='no result with this primary category in the window')
            else:
                taken.add(en['id'].split('v')[0])
                rec.update(id=en['id'], primary=en['primary'], published=en['published'], side=side, walked=walked,
                           results_in_half_window=total)
            sel['picks'].append(rec)
            write_json(args.selection, sel)
            print(f'{stratum:8} {i + 1:>3}/{q:<3} {cat:9} {a.date()} {rec.get("id")} walked {walked} of {total}', flush=True)
    write_json(args.selection, sel)
    filled = sum(1 for p in sel['picks'] if p.get('id'))
    print(f'selection: {filled} picks of {sum(s[1] for s in STRATA)} requested, {http.requests} requests, '
          f'sha256 {sha256_file(args.selection)}')


# ------------------------------------------------------------------------------------------------ fetch
def cmd_fetch(args):
    sel = read_json(args.selection)
    raw_dir = os.path.join(args.dir, 'raw')
    os.makedirs(raw_dir, exist_ok=True)
    log_path = os.path.join(args.dir, 'fetch-log.json')
    log = read_json(log_path) if os.path.exists(log_path) else {'selection_sha256': sha256_file(args.selection), 'papers': {}}
    if log['selection_sha256'] != sha256_file(args.selection):
        sys.exit('fetch-log.json belongs to another selection')
    http = Http(os.path.join(args.dir, '.last-request'))
    picks = [p for p in sel['picks'] if p.get('id')]
    terminal = ('ok', 'too-large', 'http-404', 'http-403', 'http-410')
    for n, p in enumerate(picks, 1):
        pid = p['id']
        if not ID_RE.match(pid):
            sys.exit(f'bad id in selection: {pid!r}')
        rec = log['papers'].get(pid)
        if rec and rec['status'] in terminal and not (args.retry_failed and rec['status'] != 'ok'):
            continue
        t = time.time()
        status, headers, body = http.get(EPRINT + pid, limit=MAX_DOWNLOAD)
        rec = {'status': None, 'category': p['category'], 'stratum': p['stratum'], 'fetched': dt.datetime.now(dt.timezone.utc).isoformat(timespec='seconds'),
               'content_type': headers.get('content-type'), 'content_encoding': headers.get('content-encoding')}
        if status == 200 and body is not None:
            path = os.path.join(raw_dir, pid + '.eprint')
            with open(path + '.tmp', 'wb') as f:
                f.write(body)
            os.replace(path + '.tmp', path)
            rec.update(status='ok', bytes=len(body), sha256=hashlib.sha256(body).hexdigest())
        elif status == 'too-large':
            rec.update(status='too-large', content_length=headers.get('content-length'))
        elif isinstance(status, int):
            rec.update(status=f'http-{status}')
        else:
            rec.update(status=str(status))
        log['papers'][pid] = rec
        write_json(log_path, log)
        print(f'[{n}/{len(picks)}] {pid:18} {p["stratum"]:8} {rec["status"]} {rec.get("bytes", 0)} bytes, {time.time() - t:.1f} s', flush=True)
    write_json(log_path, log)
    counts = {}
    for r in log['papers'].values():
        counts[r['status']] = counts.get(r['status'], 0) + 1
    print(f'fetch: {counts}, {http.requests} requests, {sum(r.get("bytes", 0) for r in log["papers"].values())} bytes')


# ------------------------------------------------------------------------------------------------ safe extraction
REFUSED_NAMES = {'texmf.cnf', 'ls-r', 'ls-R'}      # would reconfigure kpathsea
REFUSED_SUFFIX = ('.fmt', '.base', '.mem')         # TEXFORMATS searches '.', so a planted .fmt would replace the format
TEX_SUFFIX = ('.tex', '.ltx', '.latex', '.ltex')
ARCHIVE_SUFFIX = ('.tar', '.tgz', '.tar.gz', '.zip', '.tar.bz2', '.tbz', '.7z', '.rar')


def clean_member_name(name):
    """A list of path components, or a string saying why the name is refused."""
    if not name or '\0' in name:
        return 'empty or NUL in name'
    if name.startswith(('/', '\\')) or re.match(r'^[A-Za-z]:', name):
        return 'absolute path'
    if '\\' in name:
        return 'backslash in name'
    if any(ord(c) < 32 or ord(c) == 127 for c in name):
        return 'control character in name'
    parts = [c for c in name.split('/') if c not in ('', '.')]
    if '..' in parts:
        return "'..' in path"
    if not parts:
        return 'no path components'
    if parts[-1] in REFUSED_NAMES or parts[-1].lower().endswith(REFUSED_SUFFIX):
        return 'file that would change TeX configuration'
    return parts


def inside(root, path):
    root = os.path.realpath(root)
    return os.path.realpath(path).startswith(root + os.sep)


def write_member(root, parts, data):
    path = os.path.join(root, *parts)
    os.makedirs(os.path.dirname(path), mode=0o755, exist_ok=True)
    if not inside(root, path):
        raise OSError('resolves outside the extraction directory')
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o644)
    with os.fdopen(fd, 'wb') as f:
        f.write(data)


def unpack_tar(blob, root, rec):
    """Regular files and directories only, written by this code at a path this code built; no exec bits, no owners, no mtimes."""
    total = n = 0
    refused = {}
    with tarfile.open(fileobj=io.BytesIO(blob), mode='r:') as t:
        for m in t:
            n += 1
            if n > MAX_MEMBERS:
                rec['too_big'] = f'over {MAX_MEMBERS} members'
                break
            why = None
            if m.issym() or m.islnk():
                why = 'symlink or hard link'
            elif m.isdev() or m.isfifo() or m.ischr() or m.isblk():
                why = 'device or fifo'
            elif not (m.isreg() or m.isdir()):
                why = f'member type {m.type!r}'
            else:
                parts = clean_member_name(m.name)
                if isinstance(parts, str):
                    why = parts
            if why:
                refused[why] = refused.get(why, 0) + 1
                continue
            if m.isdir():
                path = os.path.join(root, *parts)
                os.makedirs(path, mode=0o755, exist_ok=True)
                if not inside(root, path):
                    raise OSError('directory resolves outside the extraction directory')
                continue
            total += m.size
            if total > MAX_UNPACKED:
                rec['too_big'] = f'over {MAX_UNPACKED} bytes unpacked'
                break
            f = t.extractfile(m)
            data = f.read() if f else b''
            try:
                write_member(root, parts, data)
            except OSError as e:
                why = f'could not write: {type(e).__name__}'
                refused[why] = refused.get(why, 0) + 1
    rec['members'] = n
    rec['unpacked_bytes'] = total
    if refused:
        rec['refused'] = refused


def gunzip_limited(blob):
    with gzip.GzipFile(fileobj=io.BytesIO(blob)) as g:
        data = g.read(MAX_UNPACKED + 1)
    return data


def looks_like_tar(blob):
    if len(blob) >= 512 and blob[257:262] == b'ustar':
        return True
    try:
        with tarfile.open(fileobj=io.BytesIO(blob), mode='r:') as t:
            return t.next() is not None
    except (tarfile.TarError, ValueError, EOFError, OSError):
        return False


def uncomment(data):
    out = []
    for line in data.replace(b'\r\n', b'\n').replace(b'\r', b'\n').split(b'\n'):
        i = 0
        while True:
            i = line.find(b'%', i)
            if i < 0:
                break
            j = i
            while j > 0 and line[j - 1] == 0x5c:
                j -= 1
            if (i - j) % 2 == 0:
                line = line[:i]
                break
            i += 1
        out.append(line)
    return b'\n'.join(out)


PENALTY_STEMS = ('supp', 'appendix', 'cover', 'letter', 'response', 'reply', 'poster', 'slides', 'talk', 'beamer', 'abstract',
                 'highlights', 'template', 'sample', 'example', 'readme', 'instructions', 'guide', 'figure', 'fig_', 'fig-', 'tikz')
PREFERRED_STEMS = ('main', 'ms', 'paper', 'article', 'arxiv', 'root', 'manuscript', 'thesis')
SUBDOC_RE = re.compile(rb'\\documentclass\s*(\[[^\]]*\])?\s*\{\s*(standalone|subfiles)\s*\}')
DOCCLASS_RE = re.compile(rb'\\document(class|style)\b')
BEGIN_RE = re.compile(rb'\\begin\s*\{document\}')
INPUT_RE = re.compile(rb'\\(?:input|include|subfile)\s*\{?\s*([^\s{}%]+)')
PLAIN_RE = re.compile(rb'\\input\s+(amstex|harvmac|lanlmac|phyzzx|mtexsis)\b')


def tex_files(root):
    for d, subdirs, files in os.walk(root):
        subdirs[:] = sorted(s for s in subdirs if not s.startswith('.') and s != '__MACOSX')
        for name in sorted(files):
            if not name.startswith('.') and name.lower().endswith(TEX_SUFFIX):
                yield os.path.relpath(os.path.join(d, name), root)


def readme_hints(root):
    """arXiv's own statement of the main file: 00README.json (current) or 00README.XXX (older)."""
    hints = {'toplevel': None, 'ignored': set(), 'compiler': None, 'source': None}
    p = os.path.join(root, '00README.json')
    if os.path.isfile(p) and os.path.getsize(p) < 1 << 20:
        try:
            with open(p, 'rb') as f:
                j = json.loads(f.read().decode('utf-8', 'replace'))
            proc = j.get('process') if isinstance(j, dict) else None
            if isinstance(proc, dict) and isinstance(proc.get('compiler'), str):
                hints['compiler'] = proc['compiler'][:40]
            for s in j.get('sources', []) if isinstance(j, dict) else []:
                if isinstance(s, dict) and isinstance(s.get('filename'), str):
                    if s.get('usage') in ('toplevel', 'toplevelfile') and not hints['toplevel']:
                        hints['toplevel'] = s['filename']
                        hints['source'] = '00README.json'
                    elif s.get('usage') == 'ignore':
                        hints['ignored'].add(s['filename'])
        except (ValueError, AttributeError):
            hints['readme_error'] = '00README.json unreadable'
    for name in ('00README.XXX', '00README', '00readme.xxx'):
        p = os.path.join(root, name)
        if os.path.isfile(p) and os.path.getsize(p) < 1 << 20:
            with open(p, 'rb') as f:
                for line in f.read().decode('latin-1').splitlines():
                    w = line.split()
                    if len(w) >= 2 and w[1] == 'toplevelfile' and not hints['toplevel']:
                        hints['toplevel'] = w[0]
                        hints['source'] = name
                    elif len(w) >= 2 and w[1] == 'ignore':
                        hints['ignored'].add(w[0])
    return hints


def find_main(root):
    """Returns (relative main or None, mode, how, number of candidates).  Every rule here is about being the main file, none about quality."""
    hints = readme_hints(root)
    if hints['toplevel']:
        parts = clean_member_name(hints['toplevel'])
        if isinstance(parts, list):
            rel = '/'.join(parts)
            if os.path.isfile(os.path.join(root, rel)) and inside(root, os.path.join(root, rel)):
                return rel, 'latex', hints['source'], 1, hints
        hints['readme_error'] = 'names a file not in the source'
    scanned = []
    for rel in tex_files(root):
        if rel in hints['ignored']:
            continue
        path = os.path.join(root, rel)
        if os.path.getsize(path) > MAX_SCAN:
            continue
        with open(path, 'rb') as f:
            data = f.read()
        if data[:64].lstrip().lower().startswith(b'%auto-ignore'):
            continue
        scanned.append((rel, uncomment(data), len(data)))
    refs = set()
    for rel, text, _ in scanned:
        for m in INPUT_RE.finditer(text):
            target = m.group(1).decode('latin-1')
            target = target[2:] if target.startswith('./') else target
            refs.add((rel, target))
            refs.add((rel, target + '.tex'))
    tiers = {1: [], 2: [], 3: []}
    for rel, text, size in scanned:
        has_class = DOCCLASS_RE.search(text) is not None
        has_begin = BEGIN_RE.search(text) is not None
        if PLAIN_RE.search(text) and not has_begin:     # AMS-TeX has \documentstyle too, and no \begin{document}
            continue
        if has_class and has_begin:
            tiers[1].append((rel, text, size))
        elif has_class:
            tiers[2].append((rel, text, size))
        elif has_begin:
            tiers[3].append((rel, text, size))
    for tier in (1, 2, 3):
        cands = tiers[tier]
        if not cands:
            continue

        def score(c):
            rel, text, size = c
            stem = os.path.splitext(os.path.basename(rel))[0].lower()
            sub = 1 if SUBDOC_RE.search(text) else 0
            pen = 1 if any(s in stem for s in PENALTY_STEMS) else 0
            referenced = 1 if any((o, rel) in refs for o, _, _ in cands if o != rel) else 0
            depth = rel.count('/')
            pref = 0 if stem in PREFERRED_STEMS else 1
            return (sub, referenced, pen, depth, pref, -size, rel)

        best = min(cands, key=score)
        return best[0], 'latex', f'scan-tier{tier}', len(cands), hints
    # no LaTeX document: plain TeX or AMS-TeX
    plain = []
    for rel, text, size in scanned:
        if PLAIN_RE.search(text) or re.search(rb'\\bye\b', text) or re.search(rb'\\end\s*$', text):
            plain.append((rel, size))
    if plain:
        best = min(plain, key=lambda c: (c[0].count('/'), -c[1], c[0]))
        return best[0], 'plain', 'scan-plain', len(plain), hints
    return None, None, 'no-main', 0, hints


def extract_one(pid, raw_path, papers_dir):
    rec = {}
    with open(raw_path, 'rb') as f:
        blob = f.read()
    root = os.path.join(papers_dir, pid)
    if not inside(papers_dir, root):
        raise RuntimeError('bad id')
    if os.path.isdir(root):
        shutil.rmtree(root)
    os.makedirs(root)
    kind = 'raw'
    data = blob
    if blob[:2] == b'\x1f\x8b':
        try:
            data = gunzip_limited(blob)
            kind = 'gz'
        except (OSError, EOFError, gzip.BadGzipFile, ValueError) as e:
            rec.update(kind='corrupt-gzip', status='unusable', reason=f'gzip error: {type(e).__name__}')
            return rec
        if len(data) > MAX_UNPACKED:
            rec.update(kind='gz', status='unusable', reason=f'over {MAX_UNPACKED} bytes unpacked')
            return rec
    if data[:5] == b'%PDF-':
        rec.update(kind=kind + '+pdf', status='unusable', reason='PDF only, no TeX source at arXiv')
        return rec
    if data[:4] == b'%!PS':
        rec.update(kind=kind + '+ps', status='unusable', reason='PostScript only, no TeX source at arXiv')
        return rec
    if data[:2] == b'PK':
        rec.update(kind=kind + '+zip', status='unusable', reason='zip container, not a TeX source arXiv serves as tar')
        return rec
    if looks_like_tar(data):
        kind += '+tar'
        try:
            unpack_tar(data, root, rec)
        except (tarfile.TarError, OSError, EOFError, ValueError) as e:
            rec.update(kind=kind, status='unusable', reason=f'tar error: {type(e).__name__}')
            return rec
    else:
        kind += '+single'
        lead = data[:4096].lstrip().lower()
        if lead.startswith((b'<!doctype html', b'<html')):
            rec.update(kind=kind, status='unusable', reason='HTML only, no TeX source at arXiv')
            return rec
        write_member(root, ['paper.tex'], data)
        rec.update(members=1, unpacked_bytes=len(data))
    rec['kind'] = kind
    if rec.get('too_big'):
        rec.update(status='unusable', reason=rec['too_big'])
        return rec
    main, mode, how, ncand, hints = find_main(root)
    rec.update(how=how, candidates=ncand)
    if hints.get('compiler'):
        rec['compiler_hint'] = hints['compiler']
    if hints.get('readme_error'):
        rec['readme_error'] = hints['readme_error']
    names = []
    for d, _, files in os.walk(root):
        names += [n.lower() for n in files]
    rec['nested_archives'] = sum(n.endswith(ARCHIVE_SUFFIX) for n in names)
    rec['eps_figures'] = sum(n.endswith(('.eps', '.ps', '.eps.gz')) for n in names)
    rec['bbl_shipped'] = sum(n.endswith('.bbl') for n in names)
    if main is None:
        ext = {}
        for n in names:
            e = os.path.splitext(n)[1]
            ext[e] = ext.get(e, 0) + 1
        rec.update(status='unusable', reason='no TeX main file found', extensions=dict(sorted(ext.items(), key=lambda kv: -kv[1])[:6]))
        return rec
    if '"' in main or not main.isprintable():
        rec.update(status='unusable', reason='main file name cannot be passed to the runner (quote or unprintable)')
        return rec
    rec.update(status='ok', main=main, mode=mode)
    return rec


def cmd_extract(args):
    sel = read_json(args.selection)
    log = read_json(os.path.join(args.dir, 'fetch-log.json'))
    papers = os.path.join(args.dir, 'papers')
    os.makedirs(papers, exist_ok=True)
    out = {}
    for p in sel['picks']:
        pid = p.get('id')
        if not pid or log['papers'].get(pid, {}).get('status') != 'ok':
            continue
        raw = os.path.join(args.dir, 'raw', pid + '.eprint')
        with open(raw, 'rb') as f:
            if hashlib.sha256(f.read()).hexdigest() != log['papers'][pid]['sha256']:
                sys.exit(f'{raw} does not match the sha256 in fetch-log.json')
        try:
            out[pid] = extract_one(pid, raw, papers)
        except Exception as e:      # a damaged archive must never stop the run, and must not vanish
            out[pid] = {'status': 'unusable', 'reason': f'extraction failed: {type(e).__name__}'}
        r = out[pid]
        print(f'{pid:18} {r["status"]:9} {r.get("kind", "?"):18} {r.get("how", "")} {r.get("candidates", "")} {r.get("reason", "")}', flush=True)
    write_json(os.path.join(args.dir, 'extract-log.json'), out)
    counts = {}
    for r in out.values():
        k = r['status'] if r['status'] == 'ok' else r['reason']
        counts[k] = counts.get(k, 0) + 1
    print(json.dumps(counts, indent=1))


# ------------------------------------------------------------------------------------------------ manifest
def rel_repo(path):
    return os.path.relpath(path, REPO)


def manifest_exclusions(sel, fetch, extract):
    return [
        {'what': 'Papers are never replaced', 'why': 'A pick that cannot be used stays in the manifest as excluded or oracle-fails, so the denominator is the sample as drawn. Replacing it would tune the set.'},
        {'what': f'E-prints over {MAX_DOWNLOAD // (1024 * 1024)} MB compressed and archives over {MAX_UNPACKED // (1024 * 1024)} MB unpacked are not used', 'why': 'Bound on download volume and on decompression bombs. Listed as excluded entries.'},
        {'what': 'Archive members that are symlinks, hard links, devices or fifos, that have absolute or .. paths, and any .fmt, texmf.cnf or ls-R, are refused at extraction', 'why': 'Untrusted input. kpathsea searches . for formats, so a planted .fmt would replace the oracle format. Refused members are counted in refs/corpus/arxiv/extract-log.json, and a document that needed one fails visibly.'},
        {'what': 'Nested archives inside a source are not unpacked', 'why': 'arXiv itself does not need them for the top-level compile in nearly all cases. Counted in extract-log.json.'},
        {'what': 'pdflatex is the only route. XeLaTeX, LuaLaTeX and latex+dvips papers are compiled with pdflatex anyway', 'why': 'rstex ports pdfTeX. Such papers land in oracle-fails with the reason, or in normal with TeX errors, which is error recovery to reproduce. EPS figures are not converted: with -no-shell-escape pdflatex reports them and carries on.'},
        {'what': 'The main file is found by a fixed rule (00README first, then a scan), not by asking the authors', 'why': 'A wrong guess on a paper with several documents still gives a real document to compile. How each was found is in extract-log.json.'},
        {'what': 'Biber is not run, and no paper-specific texinputs are added', 'why': 'The runner bounds bibliography tools. Papers ship a .bbl when they need one, which arXiv itself requires.'},
        {'what': 'The latest version at selection time, pinned; only submission dates 2015-01-01 to 2026-10-31', 'why': 'Reproducibility, and a sample of the modern source format.'},
    ]


def cmd_manifest(args):
    sel = read_json(args.selection)
    fetch = read_json(os.path.join(args.dir, 'fetch-log.json'))['papers']
    extract = read_json(os.path.join(args.dir, 'extract-log.json'))
    previous = {}
    if os.path.exists(args.manifest) and not args.reset:
        previous = {e['id']: e for e in read_json(args.manifest)['entries']}
    entries = []
    for p in sel['picks']:
        pid = p.get('id')
        if not pid:
            continue
        eid = f'arxiv/{p["category"]}/{pid}'
        e = {'id': eid}
        f = fetch.get(pid, {})
        x = extract.get(pid, {})
        root = rel_repo(os.path.join(args.dir, 'papers', pid))
        notes = [f'{p["category"]}, {p["published"][:7]}']
        if f.get('status') != 'ok':
            e['main'] = root + '/paper.tex'
            e['excluded'] = {'too-large': f'e-print over {MAX_DOWNLOAD // (1024 * 1024)} MB'}.get(f.get('status'), f'download failed: {f.get("status")}')
        elif x.get('status') != 'ok':
            e['main'] = root + '/paper.tex'
            e['excluded'] = x.get('reason', 'not extracted')
        else:
            e['main'] = root + '/' + x['main']
            if x['mode'] != 'latex':
                e['mode'] = x['mode']
            notes.append(f'main by {x["how"]}' + (f' of {x["candidates"]}' if x['candidates'] > 1 else ''))
            if x.get('compiler_hint') and 'pdflatex' not in x['compiler_hint']:
                notes.append(f'00README asks for {x["compiler_hint"]}')
            if x.get('refused'):
                notes.append(f'{sum(x["refused"].values())} archive members refused')
        old = previous.get(eid)
        if old:     # what finalize decided survives a regeneration
            for k in ('tier', 'expected_class', 'prelude'):
                if k in old:
                    e[k] = old[k]
        e['notes'] = '; '.join(notes)
        entries.append(e)
    m = {'source': 'arxiv',
         'description': ('Real-world papers selected before any content was seen. ' + RULE + ' Sources stay in refs/corpus/arxiv/ and are never committed; '
                         'this file lists arXiv ids only. Rebuild with tests/corpus/arxiv/sample.py fetch, extract.'),
         'defaults': {'passes': 'auto', 'bib': 'auto', 'index': 'makeindex', 'tier': 'full'},
         'exclusions': manifest_exclusions(sel, fetch, extract),
         'entries': entries}
    write_json(args.manifest, m)
    n_ex = sum('excluded' in e for e in entries)
    print(f'{args.manifest}: {len(entries)} entries, {n_ex} excluded')


# ------------------------------------------------------------------------------------------------ after prepare
def load_run(run_dir):
    res = {}
    with open(os.path.join(run_dir, 'results.jsonl'), 'rb') as f:
        for line in f:
            if line.strip():
                r = json.loads(line)
                res[r['id']] = r
    return res


def read_log(r):
    if not r.get('dir') or not r.get('id'):
        return b''
    try:
        with open(os.path.join(r['dir'], 'meta.json'), 'rb') as f:
            job = json.load(f)['jobname']
        with open(os.path.join(r['dir'], 'final', job + '.log'), 'rb') as f:
            return f.read()
    except OSError:
        return b''


GRAPHICS_EXT = ('.pdf', '.png', '.jpg', '.jpeg', '.eps', '.ps', '.mps', '.pgf', '.gif', '.tif', '.tiff', '.svg')


def fail_reason(r, log):
    """A category, built from fixed strings and TeX Live file names: never paper text.  Order: the cause, not the aftermath."""
    reason = r.get('reason') or ''
    joined = log.replace(b'\n', b'')
    if 'timeout' in reason:
        return 'timeout (300 s per pass)'
    if 'signal' in reason:
        return f'oracle killed by a signal: {reason[:60]}'
    if 'main file missing' in reason:
        return 'main file missing'
    if re.search(rb'requires either XeTeX or LuaTeX|fontspec Error|This package may only be run using|XeTeX or LuaTeX|requires? (Xe|Lua)(La)?TeX|must be compiled with (Xe|Lua)|Cannot be run with pdfLaTeX|cannot be run with pdf', joined, re.I):
        return 'needs XeLaTeX or LuaLaTeX'
    m = re.search(rb"! LaTeX Error: File `([^']*)' not found", joined)
    if m:
        name = m.group(1).decode('latin-1')
        ext = os.path.splitext(name)[1].lower()
        if ext in ('.sty', '.cls'):
            return f'missing package or class: {name}'
        if ext == '' or ext in ('.tex', '.ltx', '.def', '.cfg', '.clo', '.bbl', '.bst'):
            return 'missing input file'
        if ext in GRAPHICS_EXT:
            return 'missing figure file'
        return 'missing input file'
    if re.search(rb"! I can't find file", joined):
        return 'missing input file'
    m = re.search(rb'Font ([^ ]+)(?: at \d+)? not found', joined)
    if m or re.search(rb'pdfTeX error.*[Ff]ont', joined):
        return 'font not available (no mktex here)'
    if re.search(rb"Metric \(TFM\) file[^!]*not found", joined):
        return 'font not available (no mktex here)'
    if re.search(rb"Package babel Error: Unknown option", joined):
        return 'babel language not installed'
    if re.search(rb"Cannot find the file", joined):
        return 'missing input file'
    if b'TeX capacity exceeded' in joined:
        return 'TeX capacity exceeded'
    if b'That makes 100 errors' in joined:
        return '100 errors, TeX gave up'
    if b'Emergency stop' in joined:
        return 'emergency stop'
    if b'No pages of output' in joined:
        return 'no pages of output'
    first = re.sub(r'[^ -~]', '?', (r.get('first_error') or '').lstrip('! '))[:60]
    return f'document error: {first}' if first else 'fatal error'


def error_kinds(log):
    """For entries with a PDF and errors: which kinds of TeX error, masked.  Counted per document, and a document can have several."""
    kinds = set()
    joined = log.replace(b'\n', b'')
    tests = [
        (rb"pdftex\.def Error: File|Unknown graphics extension|Cannot determine size of graphic", 'figure not found or not readable (EPS is not converted without shell escape)'),
        (rb"LaTeX Error: File `[^']*' not found", 'another file not found'),
        (rb"Invalid UTF-8 byte|Unicode character", 'source not UTF-8, or a character with no definition'),
        (rb"! Font [^\n]*not loadable", 'font metric not installed'),
        (rb"Encoding file `|babel Error: Unknown option", 'encoding or language not installed'),
        (rb"already defined", 'a command defined twice'),
        (rb"! Undefined control sequence|Command \\[A-Za-z@]+ undefined", 'undefined control sequence'),
        (rb"! Missing ", 'missing token (Missing $, number, ...)'),
        (rb"! Extra |! Argument of|! Paragraph ended|! Runaway|! File ended|! Illegal|! Too many|! Double|! Misplaced", 'structural error (runaway, misplaced, extra, ...)'),
        (rb"! Package (?!pdftex\.def|babel|fontenc)", 'other package error'),
        (rb"! Class ", 'class error'),
        (rb"! LaTeX Error: (?!File|Invalid UTF|Unicode char|Command)", 'other LaTeX error'),
    ]
    for pat, name in tests:
        if re.search(pat, joined):
            kinds.add(name)
    return kinds or {'other TeX error'}


def fls_files(r):
    """Distinct classes and packages loaded in the last pass, split into TeX Live's and the paper's own."""
    try:
        with open(os.path.join(r['dir'], 'meta.json'), 'rb') as f:
            job = json.load(f)['jobname']
        with open(os.path.join(r['dir'], 'final', job + '.fls'), 'rb') as f:
            data = f.read().decode('utf-8', 'surrogateescape')
    except OSError:
        return set(), set()
    tl, own = set(), set()
    for line in data.splitlines():
        if line.startswith('INPUT '):
            p = line[6:]
            if p.endswith(('.sty', '.cls')):
                (tl if p.startswith(('/usr/share/texmf', '/var/lib/texmf', '/etc/texmf')) else own).add(os.path.basename(p))
    return tl, own


def total_ms(r):
    return sum(r.get('pass_ms') or [0])


def meta_of(r):
    try:
        with open(os.path.join(r['dir'], 'meta.json'), 'rb') as f:
            return json.load(f)
    except (OSError, KeyError, TypeError, ValueError):
        return {}


def cpu_ms(r):
    return meta_of(r).get('pass_cpu_ms') or []


def pages_of(r, log):
    """From the log with its line wraps undone: a long file name can wrap the statistics line, and the runner's count misses that."""
    if r.get('pages') is not None:
        return r['pages']
    m = re.search(rb'\((\d+) pages?, \d+ bytes\)', log.replace(b'\n', b''))
    return int(m.group(1)) if m else None


def choose_standard(entries, results, limit=40):
    """Greedy, deterministic: among normal entries with no TeX errors and whose total oracle CPU time is in the fastest half, repeatedly take the
    one adding the most new classes (weight 8), strata (4) and packages (1), ties to the faster, then the id.  At most 5 per stratum."""
    pool = []
    for e in entries:
        r = results.get(e['id'])
        if not r or r.get('class') != 'normal' or r.get('errors') or e.get('excluded'):
            continue
        tl, own = fls_files(r)
        pool.append({'id': e['id'], 'ms': sum(cpu_ms(r)) or total_ms(r), 'stratum': CATEGORY_STRATUM[e['id'].split('/')[1]],
                     'cls': {n for n in tl | own if n.endswith('.cls')}, 'pkg': {n for n in tl | own if n.endswith('.sty')}})
    pool.sort(key=lambda c: c['ms'])
    pool = pool[:max(limit, (len(pool) + 1) // 2)]
    chosen, seen_c, seen_p, per = [], set(), set(), {}
    while pool and len(chosen) < limit:
        def gain(c):
            return (8 * len(c['cls'] - seen_c) + 4 * (per.get(c['stratum'], 0) == 0) + len(c['pkg'] - seen_p), -c['ms'], c['id'])
        ok = [c for c in pool if per.get(c['stratum'], 0) < 5] or pool
        best = max(ok, key=gain)
        pool.remove(best)
        chosen.append(best['id'])
        seen_c |= best['cls']
        seen_p |= best['pkg']
        per[best['stratum']] = per.get(best['stratum'], 0) + 1
    return chosen, sorted(seen_c), sorted(seen_p)


def cmd_finalize(args):
    m = read_json(args.manifest)
    results = load_run(args.run)
    standard, seen_c, seen_p = choose_standard(m['entries'], results)
    print(f'standard tier: {len(standard)} entries, covering {len(seen_c)} classes and {len(seen_p)} packages')
    for e in m['entries']:
        r = results.get(e['id'])
        e.pop('expected_class', None)
        e['tier'] = 'standard' if e['id'] in standard else 'full'
        e['notes'] = e['notes'].split(' | ')[0]
        if not r or e.get('excluded'):
            continue
        if r['class'] in ('oracle-fails', 'nondeterministic'):
            e['expected_class'] = r['observed_class']
            if r['class'] == 'oracle-fails':
                e['notes'] += ' | oracle fails: ' + fail_reason(r, read_log(r))
            else:
                e['notes'] += ' | nondeterministic'
        elif r.get('errors'):
            e['notes'] += f' | PDF with {r["errors"]} TeX errors'
    m['description'] = m['description'].split(' Tiers:')[0] + (
        ' Tiers: standard holds 40 picks chosen after the oracle run, for speed and variety only: normal, no TeX errors, in the fastest half by total oracle CPU time, '
        'greedy on new classes, then strata, then packages, at most 5 per stratum. Everything else is full.')
    missing = [e['id'] for e in m['entries'] if 'missing package or class' in e.get('notes', '')]
    m['exclusions'] = [x for x in m['exclusions'] if not x['what'].startswith('Papers needing a package or class')]
    m['exclusions'].append({'what': f'Papers needing a package or class this TeX Live does not have: {len(missing)} of {len(m["entries"])} entries when last finalized ({dt.date.today().isoformat()})',
                            'why': 'They stay here as oracle-fails with the file named in their notes. The TeX Live on this machine is a partial Arch install, so most of these are coverage lost to the machine, not to the papers.'})
    write_json(args.manifest, m)


def percentiles(values):
    v = sorted(values)
    if not v:
        return 'none'
    q = lambda p: v[min(len(v) - 1, int(p * len(v)))]
    return f'n={len(v)} min={v[0]:.6g} p10={q(.10):.6g} p25={q(.25):.6g} median={q(.50):.6g} p75={q(.75):.6g} p90={q(.90):.6g} p99={q(.99):.6g} max={v[-1]:.6g} sum={sum(v):.6g}'


def cmd_report(args):
    sel = read_json(args.selection)
    fetch = read_json(os.path.join(args.dir, 'fetch-log.json'))['papers']
    extract = read_json(os.path.join(args.dir, 'extract-log.json'))
    m = read_json(args.manifest)
    results = load_run(args.run)
    by_pid = {e['id'].split('/')[-1]: e for e in m['entries']}
    rows = {}
    order = []
    for stratum, q, _ in STRATA:
        rows[stratum] = dict(requested=q, selected=0, downloaded=0, tex=0, compiled=0, clean=0, errors=0, fails=0, nondet=0, excluded=0, other=0)
        order.append(stratum)
    fail_counts, excl_counts, err_kinds = {}, {}, {}
    pages, last_ms, sum_ms, passes, last_cpu, sum_cpu = [], [], [], [], [], []
    cls_tl, pkg_tl, own_cls, own_pkg = {}, {}, {}, {}
    no_pages = []
    for p in sel['picks']:
        row = rows[p['stratum']]
        pid = p.get('id')
        if not pid:
            continue
        row['selected'] += 1
        if fetch.get(pid, {}).get('status') == 'ok':
            row['downloaded'] += 1
        x = extract.get(pid, {})
        if x.get('status') == 'ok':
            row['tex'] += 1
        e = by_pid[pid]
        r = results.get(e['id'])
        if r is None:
            row['other'] += 1
            continue
        c = r['class']
        if c == 'normal':
            row['compiled'] += 1
            if r.get('errors'):
                row['errors'] += 1
                for k in error_kinds(read_log(r)):
                    err_kinds[k] = err_kinds.get(k, 0) + 1
            else:
                row['clean'] += 1
        elif c == 'oracle-fails':
            row['fails'] += 1
            why = fail_reason(r, read_log(r))
            fail_counts[why] = fail_counts.get(why, 0) + 1
        elif c == 'nondeterministic':
            row['nondet'] += 1
        elif c == 'excluded':
            row['excluded'] += 1
            excl_counts[r['reason']] = excl_counts.get(r['reason'], 0) + 1
        else:
            row['other'] += 1
        if c in ('normal', 'nondeterministic'):
            n = pages_of(r, read_log(r))
            if n is not None:
                pages.append(n)
            else:
                no_pages.append(e['id'])
            if r.get('pass_ms'):
                last_ms.append(r['pass_ms'][-1] / 1000)
                sum_ms.append(total_ms(r) / 1000)
                passes.append(r.get('passes_run') or len(r['pass_ms']))
            cpu = cpu_ms(r)
            if cpu:
                last_cpu.append(cpu[-1] / 1000)
                sum_cpu.append(sum(cpu) / 1000)
            tl, own = fls_files(r)
            for n in tl:
                d = cls_tl if n.endswith('.cls') else pkg_tl
                d[n] = d.get(n, 0) + 1
            for n in own:
                d = own_cls if n.endswith('.cls') else own_pkg
                d[n] = d.get(n, 0) + 1
    head = ('stratum', 'req', 'sel', 'dl', 'tex', 'compiled', 'clean', 'w/err', 'fails', 'nondet', 'excl', 'other')
    print(' '.join(f'{h:>9}' for h in head))
    totals = {k: 0 for k in rows[order[0]]}
    groups = {'math': dict(totals), 'others': dict(totals)}
    for s in order:
        r = rows[s]
        print(f'{s:>9} ' + ' '.join(f'{r[k]:>9}' for k in ('requested', 'selected', 'downloaded', 'tex', 'compiled', 'clean', 'errors', 'fails', 'nondet', 'excluded', 'other')))
        g = groups['math' if s.startswith('math') else 'others']
        for k, v in r.items():
            totals[k] += v
            g[k] += v
    for name, r in list(groups.items()) + [('all', totals)]:
        print(f'{name:>9} ' + ' '.join(f'{r[k]:>9}' for k in ('requested', 'selected', 'downloaded', 'tex', 'compiled', 'clean', 'errors', 'fails', 'nondet', 'excluded', 'other')))

    def show(title, d, limit=None):
        print(f'\n{title} ({sum(d.values())}):')
        items = sorted(d.items(), key=lambda kv: (-kv[1], kv[0]))
        for k, v in items[:limit]:
            print(f'  {v:>4}  {k}')
        if limit and len(items) > limit:
            print(f'  ... {len(items) - limit} more names')

    show('oracle-fails by reason', fail_counts)
    show('excluded by reason', excl_counts)
    print(f'\nnormal entries with TeX errors, kinds of error (a document can count in several): {sum(d["errors"] for d in rows.values())} documents')
    for k, v in sorted(err_kinds.items(), key=lambda kv: -kv[1]):
        print(f'  {v:>4}  {k}')
    print(f'\npages:                    {percentiles(pages)}')
    print(f'entries with no page count: {no_pages}')
    print(f'oracle last pass, CPU s:  {percentiles(last_cpu)}')
    print(f'oracle all passes, CPU s: {percentiles(sum_cpu)}')
    print(f'oracle last pass, wall s: {percentiles(last_ms)}')
    print(f'oracle all passes, wall s:{percentiles(sum_ms)}')
    print(f'passes:                   {percentiles(passes)}')
    show('classes from TeX Live, documents loading each', cls_tl, 25)
    show('classes shipped by the paper', own_cls, 15)
    show('packages from TeX Live, documents loading each (transitive included)', pkg_tl, 40)
    show('packages shipped by the paper', own_pkg, 15)
    kinds, hows, refused = {}, {}, {}
    nested = eps = bbl = 0
    for x in extract.values():
        kinds[x.get('kind', '?')] = kinds.get(x.get('kind', '?'), 0) + 1
        hows[x.get('how', '?')] = hows.get(x.get('how', '?'), 0) + 1
        for k, v in (x.get('refused') or {}).items():
            refused[k] = refused.get(k, 0) + v
        nested += 1 if x.get('nested_archives') else 0
        eps += 1 if x.get('eps_figures') else 0
        bbl += 1 if x.get('bbl_shipped') else 0
    print(f'\ne-print forms: {kinds}\nmain file found by: {hows}\nrefused archive members: {refused}\n'
          f'papers with nested archives {nested}, with .eps/.ps files {eps}, shipping a .bbl {bbl}')
    hints = {}
    for x in extract.values():
        if x.get('compiler_hint'):
            hints[x['compiler_hint']] = hints.get(x['compiler_hint'], 0) + 1
    print(f'compiler hints in 00README: {hints}')
    bytes_total = sum(r.get('bytes', 0) for r in fetch.values())
    print(f'downloaded bytes: {bytes_total}')


# ------------------------------------------------------------------------------------------------ helpers, CLI
def read_json(path):
    with open(path, encoding='utf-8') as f:
        return json.load(f)


def write_json(path, obj):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + '.tmp'
    with open(tmp, 'w', encoding='utf-8') as f:
        json.dump(obj, f, indent=1)
        f.write('\n')
    os.replace(tmp, path)


def sha256_file(path):
    with open(path, 'rb') as f:
        return hashlib.sha256(f.read()).hexdigest()


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--dir', default=DEFAULT_DIR, help='download directory (default refs/corpus/arxiv)')
    ap.add_argument('--selection', default=SELECTION)
    ap.add_argument('--manifest', default=MANIFEST)
    sub = ap.add_subparsers(dest='cmd', required=True)
    sub.add_parser('select')
    p = sub.add_parser('fetch')
    p.add_argument('--retry-failed', action='store_true', help='download again papers whose last attempt ended in a terminal failure')
    sub.add_parser('extract')
    p = sub.add_parser('manifest')
    p.add_argument('--reset', action='store_true', help='forget tiers and expected classes decided by finalize')
    p = sub.add_parser('finalize')
    p.add_argument('run', help='a prepare run directory: target/corpus/runs/<run>')
    p = sub.add_parser('report')
    p.add_argument('run', help='a prepare run directory: target/corpus/runs/<run>')
    args = ap.parse_args()
    {'select': cmd_select, 'fetch': cmd_fetch, 'extract': cmd_extract, 'manifest': cmd_manifest, 'finalize': cmd_finalize, 'report': cmd_report}[args.cmd](args)


if __name__ == '__main__':
    main()
