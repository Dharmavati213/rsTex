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


def bin_dir(root):
    found = glob.glob(os.path.join(root, "bin", "*"))
    return found[0] if found else None


def install(root, repo):
    work = tempfile.mkdtemp(prefix="install-tl-")
    windows = platform.system() == "Windows"
    name = "install-tl.zip" if windows else "install-tl-unx.tar.gz"
    archive = os.path.join(work, name)
    with urllib.request.urlopen(f"{repo}/{name}") as r, open(archive, "wb") as f:
        shutil.copyfileobj(r, f)
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
