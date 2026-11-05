#!/usr/bin/env -S python3 -I
"""check_tree.py RSTEX: what `rstex --check-tree` says of the tree CI installed, each path that is off as a warning on the run.  Always exit status 0: an upstream release must not turn a commit red.  docs/BUILD.md says what to do."""
import subprocess
import sys


def main():
    r = subprocess.run([sys.argv[1], '--check-tree'], capture_output=True, text=True, errors='replace', stdin=subprocess.DEVNULL)
    print(r.stdout, end='')
    off = [line for line in r.stdout.split('\n') if line.startswith('path=') and ' state=off' in line]
    for line in off:
        # What a workflow command holds has to be on one line, with % and newlines escaped.
        print('::warning title=A fast path is off on TeX Live of today::' + line.replace('%', '%25') + '. Output stays right and it is slower. See docs/BUILD.md, "When TeX Live updates".')
    if r.returncode == 2:
        print('::warning title=rstex --check-tree has no answer::' + r.stdout.strip().replace('%', '%25').replace('\n', ' '))
    return 0


if __name__ == '__main__':
    sys.exit(main())
