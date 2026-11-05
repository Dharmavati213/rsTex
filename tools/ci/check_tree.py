#!/usr/bin/env -S python3 -I
"""check_tree.py RSTEX: what `rstex --check-tree` says of the tree CI installed, each path that is off as a warning on the run, and a run that gives no answer as one too.  Always exit status 0: an upstream release must not turn a commit red.  docs/BUILD.md says what to do."""
import subprocess
import sys

# What a format's build and one small document can take on a slow machine.
TIMEOUT = 900


def escaped(text):
    """What a workflow command holds has to be on one line, with % and newlines escaped."""
    return text.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')


def no_answer(why):
    print('::warning title=rstex --check-tree has no answer::' + escaped(why))


def main():
    # A byte the console's code page does not have must not be what stops this.
    sys.stdout.reconfigure(errors='replace')
    try:
        r = subprocess.run([sys.argv[1], '--check-tree'], capture_output=True, text=True, errors='replace', stdin=subprocess.DEVNULL, timeout=TIMEOUT)
    except (OSError, subprocess.SubprocessError) as e:
        no_answer(str(e))
        return 0
    print(r.stdout, end='')
    for line in r.stdout.split('\n'):
        if line.startswith('path=') and ' state=off' in line:
            print('::warning title=A fast path is off on TeX Live of today::' + escaped(line) + '. Output stays right and it is slower. See docs/BUILD.md, "When TeX Live updates".')
    # 0 and 1 are answers.  Anything else, a crash too, is not, whatever it printed first.
    if r.returncode not in (0, 1):
        no_answer(f'status {r.returncode}: ' + (r.stdout + r.stderr).strip()[-400:])
    return 0


if __name__ == '__main__':
    sys.exit(main())
