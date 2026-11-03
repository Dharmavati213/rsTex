#!/usr/bin/env -S python3 -I
r"""Which primitives of tests/primitives.json do the generated tests really reach?  Read from the oracle logs of `run.py prepare`.

    python3 -I tests/corpus/gen/reach.py [--manifest FILE] [--out FILE]

Levels, strongest first, for each primitive:

  executed   \tracingcommands logged it as a command, in a run of its own document: {\name}
  read       the log shows it read through \the or \showthe, which \tracingcommands cannot see
  shown      only reported on by \show or \meaning
  undefined  the oracle says it is undefined here (the encTeX and MLTeX names)
  none       no generated test touches it

Also lists per-primitive documents that did not reach [[end of probes]].
"""
import argparse
import importlib.util
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location('corpus_run', os.path.join(os.path.dirname(HERE), 'run.py'))
run = importlib.util.module_from_spec(spec)
spec.loader.exec_module(run)

DISPLAY = {' ': 'ctrl-space', '-': 'ctrl-hyphen', '/': 'ctrl-slash'}
# With \tracingifs a conditional carries its nesting after a colon: {\ifnum: (level 1) entered on line 5}
TRACE = re.compile(rb'^\{(?:[a-z ]+ mode: )?(?:\\|[^A-Za-z{}])?([A-Za-z]+|[ /-])(?:[0-9"][^\n]*|: [^\n]*)?\}$', re.M)
UNLESS = re.compile(rb'^\{(?:[a-z ]+ mode: )?\\?unless\\?(if[a-z]*)(?:: [^\n]*)?\}$', re.M)


def traced_names(data):
    res = set()
    for m in TRACE.finditer(data):
        res.add(m.group(1).decode('latin-1'))
    if b'{select font nullfont}' in data or b'mode: select font nullfont}' in data:
        res.add('nullfont')
    if UNLESS.search(data):
        res.add('unless')
        for m in UNLESS.finditer(data):
            res.add(m.group(1).decode('latin-1'))
    return res


def unwrapped(data):
    """Logs wrap at 79 columns: join lines of exactly that length so a name split across lines is found."""
    out, buf = [], b''
    for line in data.split(b'\n'):
        if len(line) == 79:
            buf += line
        else:
            out.append(buf + line)
            buf = b''
    return b'\n'.join(out)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--manifest', action='append', default=[os.path.join(run.MANIFEST_DIR, 'coverage.json')])
    ap.add_argument('--out', help='also write the table as JSON here')
    args = ap.parse_args()
    args.manifest = [os.path.abspath(m) for m in args.manifest]
    os.chdir(run.REPO)
    entries, _, _ = run.load_manifests(args.manifest)
    with open(os.path.join(run.REPO, 'tests', 'primitives.json')) as f:
        inventory = json.load(f)
    logs = {}
    missing = []
    for e in entries:
        meta = run.load_valid_oracle(e)
        if meta is None:
            missing.append(e['id'])
            continue
        logs[e['id']] = run.open_bytes(os.path.join(meta['dir'], 'final', meta['jobname'] + '.log'))
    everywhere = set()
    for data in logs.values():
        everywhere |= traced_names(data)
    rows, early = {}, []
    for rec in inventory:
        p = rec['name']
        disp = DISPLAY.get(p, p)
        own = b''
        for kind in ('prim', 'prim-bare'):
            own += logs.get(f'coverage/{kind}/{disp}', b'') + b'\n'
        for eid, data in logs.items():
            if eid.startswith((f'coverage/prim-fatal/{disp}-', f'coverage/prim-value/{disp}=')):
                own += data + b'\n'
        joined = unwrapped(own)
        name = p.encode('latin-1')
        cs = re.escape(b'\\' + name)
        names = traced_names(own)
        level = 'none'
        if rec['class'] == 'enc/ml' and b'Undefined control sequence' in own and b'undefined' in own:
            level = 'undefined'
        elif p in names:
            level = 'executed'
        elif re.search(rb'\\(?:the|showthe) ?' + cs + rb'(?![A-Za-z])', joined):
            level = 'read'
        elif re.search(rb'> ' + cs + rb'=', joined):
            level = 'shown'
        rows[p] = {'class': rec['class'], 'level': level, 'traced_anywhere': p in everywhere}
        main_log = logs.get(f'coverage/prim/{disp}', b'')
        if b'[[end of probes]]' not in main_log:
            early.append(disp)
    counts = {}
    for row in rows.values():
        counts[row['level']] = counts.get(row['level'], 0) + 1
    print(f'{len(inventory)} primitives in the inventory; oracle logs read for {len(logs)} of {len(entries)} coverage entries')
    if missing:
        print(f'  {len(missing)} entries have no valid oracle yet: run prepare first')
    for level in ('executed', 'read', 'shown', 'undefined', 'none'):
        names = sorted(p for p, r in rows.items() if r['level'] == level)
        print(f'  {level}: {counts.get(level, 0)}')
        if level != 'executed' and names:
            print('    ' + ' '.join('\\' + n for n in names))
    print(f'main documents that stop before [[end of probes]]: {len(early)}' + (': ' + ' '.join(early) if early else ''))
    if args.out:
        with open(args.out, 'w') as f:
            json.dump({'rows': rows, 'early': early, 'counts': counts}, f, indent=1)
    return 0


if __name__ == '__main__':
    sys.exit(main())
