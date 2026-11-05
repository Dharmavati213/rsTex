#!/usr/bin/env -S python3 -I
"""install_texlive.py DIR: a TeX Live for CI in DIR, with what the integration cases and tests/cross_os load and no documentation.  The same on all three systems, from one repository.

Prints the directory of the binaries, and on Windows those of qpdf and pdftoppm, which it fetches, and adds them to $GITHUB_PATH when there is such a file.  With the binaries already there it only does that, so a job that restored DIR from its
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
# synctex, the program, is no part of any collection here either, and tests/cross_os/synctex_tool.py asks it.  enumitem and lipsum are what tests/ladder loads beyond these collections, and standalone and preview what
# tests/cd/compare_cli.py does.
PACKAGES = ["texware", "mwe", "optex", "alertmessage", "fancymag", "notes", "beamertheme-npbt", "elpres", "spark-otf", "synctex", "enumitem", "lipsum", "standalone", "preview"]
# What the comparisons need that a runner does not have, on Windows, where they stay in DIR, in its cache.  On macOS they are Homebrew's, as qpdf's own build there is for macOS 15, and on Linux apt's.
QPDF = "https://github.com/qpdf/qpdf/releases/download/v12.4.2/qpdf-12.4.2-msvc64.zip"
POPPLER = "https://github.com/oschwartz10612/poppler-windows/releases/download/v26.09.0-0/Release-26.09.0-0.zip"
# For tests/cd/native/new_profile.py, with `install_texlive.py DIR --profile`: docs/BUILD.md, "When TeX Live updates".  CI has no use for them unless it makes a profile.  First what gen_table.py loads, for a table with as much to say as on a full tree,
# then what its cases load, then what the diagrams of tests/corpus/manifests/cd.json load: without those, 731 of them can be compiled on the tree above.  Each round of the last only shows what the one before had hidden.
PROFILE_PACKAGES = ["doublestroke", "bbm", "bbm-macros", "mathabx", "relsize", "cancel", "bbold", "was", "esint", "yfonts", "mleftright", "scalerel", "multirow", "braket", "mathdots", "bbding", "ulem", "soul", "mnsymbol", "cleveref",
                    "multido", "preview",
                    "enumitem", "comment", "ntgclass", "todonotes", "biblatex", "amsrefs", "standalone", "makecell", "lipsum", "elsarticle", "marginnote", "stix2-type1", "aliascnt", "moresize", "newtx", "cclicenses", "titlesec", "llncs", "floatrow", "ifsym", "capt-of",
                    "chngcntr", "threeparttable", "tcolorbox", "wrapfig", "preprint", "ucs", "ifoddpage", "appendix", "mathalfa", "idxlayout", "hypdoc",
                    "csquotes", "imakeidx", "a4wide", "nag", "pdfcol", "manfnt", "tabularray", "sectsty", "tocbibind", "kpfonts", "iitem", "systeme", "framed", "orcidlink", "hyphenat", "outlines", "diagbox", "fontaxes", "ieeetran",
                    "nth", "srcltx", "answers", "xargs", "fixme", "zref-clever", "listingsutf8", "inconsolata", "amsaddr", "calligra", "datetime", "glossaries",
                    "subfiles", "minitoc", "fmtcount", "insbox", "lkproof", "mathalpha",
                    # Without its language babel goes round for ever, on the oracle as well.
                    "babel-german", "babel-french", "babel-turkish", "babel-spanish", "babel-russian", "babel-greek"]
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
    # A mirror can be out of step with itself, and then a package does not match its sum: the whole install again, from another.
    mirrors = [repo] + [m for m in FALLBACKS if m != repo]
    for i, mirror in enumerate(mirrors):
        try:
            subprocess.run([installer, "-profile", profile, "-repository", mirror, "-no-interaction"], check=True, shell=windows)
            subprocess.run([os.path.join(bin_dir(root), "tlmgr" + (".bat" if windows else "")), "--repository", mirror, "install"] + PACKAGES + (PROFILE_PACKAGES if "--profile" in sys.argv else []), check=True, shell=windows)
            break
        except subprocess.CalledProcessError as e:
            print(f"{mirror}: {e}", file=sys.stderr)
            if i + 1 == len(mirrors):
                raise
            shutil.rmtree(root, ignore_errors=True)
    shutil.rmtree(work, ignore_errors=True)


def unpack(url, into):
    """A zip file."""
    os.makedirs(into, exist_ok=True)
    archive = os.path.join(into, "download.zip")
    for attempt in range(3):
        try:
            with urllib.request.urlopen(url, timeout=120) as r, open(archive, "wb") as f:
                shutil.copyfileobj(r, f)
            break
        except OSError as e:
            print(f"{url}: {e}", file=sys.stderr)
            if attempt == 2:
                raise
    with zipfile.ZipFile(archive) as z:
        z.extractall(into)
    os.unlink(archive)


def tools(root):
    """What goes on the PATH beside TeX Live, on a system where no package manager gives it for nothing."""
    if platform.system() != "Windows":
        return []
    found = []
    for name, url, program in (("qpdf", QPDF, "qpdf.exe"), ("poppler", POPPLER, "pdftoppm.exe")):
        where = os.path.join(root, "ci-tools", name)
        if not glob.glob(os.path.join(where, "**", program), recursive=True):
            unpack(url, where)
        found.append(os.path.dirname(glob.glob(os.path.join(where, "**", program), recursive=True)[0]))
    return found


def main():
    root = os.path.abspath(sys.argv[1])
    if not bin_dir(root):
        install(root.replace("\\", "/"), os.environ.get("TEXLIVE_REPO", "https://mirror.ctan.org/systems/texlive/tlnet"))
    dirs = [bin_dir(root)] + tools(root)
    for b in dirs:
        print(b)
    path = os.environ.get("GITHUB_PATH")
    if path:
        with open(path, "a") as f:
            f.writelines(b + "\n" for b in dirs)


if __name__ == "__main__":
    main()
