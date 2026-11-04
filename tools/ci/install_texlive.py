#!/usr/bin/env -S python3 -I
"""install_texlive.py DIR: a TeX Live for CI in DIR, with what the integration cases and tests/cross_os load and no documentation.  The same on all three systems, from one repository.

Prints the directory of the binaries, and adds it to $GITHUB_PATH when there is such a file.  With the binaries already there it only does that, so a job that restored DIR from its
cache costs nothing here.  About 500 MB; some 10 minutes the first time and then the cache, which .github/workflows/ci.yml keys on the year.

TEXLIVE_REPO is the repository, default the current one on CTAN's mirrors.  A year after its release that is another year's: then set it to that year's frozen tlnet-final under historic/.
"""
import glob
import os
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
import zipfile

COLLECTIONS = ["basic", "latex", "latexrecommended", "fontsrecommended", "pictures", "mathscience"]
# dvitype, in texware, for what tests/integration/run.py says of a DVI file that differs.  Its only collection is binextra, far more than this needs, so it goes in by name.
# The rest are the images and the font the integration cases load, each from a package of a few MB at most; without one of them a case stops at the missing file and agrees with anything after it.
PACKAGES = ["texware", "mwe", "optex", "alertmessage", "fancymag", "notes", "beamertheme-npbt", "elpres", "spark-otf"]
FALLBACKS = ["https://mirrors.rit.edu/CTAN/systems/texlive/tlnet", "https://ftp.fau.de/ctan/systems/texlive/tlnet"]


def bin_dir(root):
    found = glob.glob(os.path.join(root, "bin", "*"))
    return found[0] if found else None


def fetch_from_a_mirror(repo, name, archive):
    """Saves repo/name as archive and gives back the repository that answered.  CTAN's address redirects to any mirror, and one of them may be down or have a bad certificate.
    The installer then gets the mirror itself, so that one run does not meet several."""
    last = None
    for candidate in [repo, repo, repo] + FALLBACKS:
        try:
            with urllib.request.urlopen(f"{candidate}/{name}", timeout=60) as r, open(archive, "wb") as f:
                shutil.copyfileobj(r, f)
                return r.geturl().removesuffix(f"/{name}")
        except OSError as e:  # URLError, HTTPError and timeouts are all OSError
            print(f"{candidate}: {e}", file=sys.stderr)
            last = e
    raise last


def install(root, repo):
    work = tempfile.mkdtemp(prefix="install-tl-")
    windows = platform.system() == "Windows"
    name = "install-tl.zip" if windows else "install-tl-unx.tar.gz"
    archive = os.path.join(work, name)
    repo = fetch_from_a_mirror(repo, name, archive)
    if windows:
        with zipfile.ZipFile(archive) as z:
            z.extractall(work)
    else:
        with tarfile.open(archive) as t:
            t.extractall(work, filter="data")
    installer = glob.glob(os.path.join(work, "install-tl-*", "install-tl" + ("-windows.bat" if windows else "")))[0]
    profile = os.path.join(work, "profile")
    with open(profile, "w") as f:
        f.write("selected_scheme scheme-minimal\n")
        f.write(f"TEXDIR {root}\nTEXMFLOCAL {root}/texmf-local\nTEXMFSYSCONFIG {root}/texmf-config\nTEXMFSYSVAR {root}/texmf-var\n")
        f.write(f"TEXMFCONFIG {root}/texmf-config\nTEXMFVAR {root}/texmf-var\nTEXMFHOME {root}/texmf-home\n")
        f.write("tlpdbopt_install_docfiles 0\ntlpdbopt_install_srcfiles 0\ntlpdbopt_autobackup 0\ninstopt_adjustpath 0\n")
        for c in COLLECTIONS:
            f.write(f"collection-{c} 1\n")
    subprocess.run([installer, "-profile", profile, "-repository", repo, "-no-interaction"], check=True, shell=windows)
    subprocess.run([os.path.join(bin_dir(root), "tlmgr" + (".bat" if windows else "")), "--repository", repo, "install"] + PACKAGES, check=True, shell=windows)
    shutil.rmtree(work, ignore_errors=True)


def main():
    root = os.path.abspath(sys.argv[1])
    if not bin_dir(root):
        install(root.replace("\\", "/"), os.environ.get("TEXLIVE_REPO", "https://mirror.ctan.org/systems/texlive/tlnet"))
    b = bin_dir(root)
    print(b)
    path = os.environ.get("GITHUB_PATH")
    if path:
        with open(path, "a") as f:
            f.write(b + "\n")


if __name__ == "__main__":
    main()
