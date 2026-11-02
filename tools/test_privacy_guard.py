#!/usr/bin/env -S python3 -I
"""Tests for tools/privacy_guard.py and the hooks of tools/install-hooks.sh, on made-up documents in a temporary
directory.  Everything private here is invented for this test.  Run: python3 -I tools/test_privacy_guard.py"""
import base64
import gzip
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
GUARD = os.path.join(HERE, "privacy_guard.py")
INSTALL = os.path.join(HERE, "install-hooks.sh")
WORD = "the" + "sis"    # split so that no name below is a real mention under rule f

PRIVATE = r"""\documentclass[12pt]{scrartcl}
\usepackage[T1]{fontenc}
\usepackage{tikz-cd}
\newcommand{\marmaladefunctor}{\operatorname{Mar}}
\newtheorem{lem}{Lemma}
\title{Ornamental Marmalade Spectra over Windswept Lighthouses}
\begin{document}
\section{Introduction}
\section{Tidal Bookkeeping for Velvet Harbours}\label{sec:tidal-bookkeeping}
The harbourmaster insists that every velvet ledger % a marginal remark
must be weighed against the $X$-twisted barometer before the morning
tide returns to the ornamental quay, which he then records twice.
\cite{Quillfeather1887,velvet-ledgers}
\[
  \operatorname{Mar}(X) \cong \varinjlim_{i} \operatorname{Mar}(X_i) \otimes \mathcal{L}^{\otimes 3}
\]
\begin{tikzcd}[column sep=large,row sep=large]
|[alias=lanternpath]| \mathcal{P}_1 \arrow[r, hook, "\alpha"] \arrow[d, two heads, "\beta"'] & \mathcal{P}_2 \arrow[d, "\gamma"] \arrow[r, dashed] & \mathcal{P}_3 \\
\mathcal{Q}_1 \arrow[r, "\delta"'] \arrow[rru, bend right] & \mathcal{Q}_2 \arrow[ru, "\epsilon"'] &
\end{tikzcd}
\end{document}
"""

RELETTERED = r"""\begin{tikzcd}[row sep=tiny]
\mathcal{A}_1 \arrow[r, hook, "\phi"] \arrow[d, two heads, "\psi"'] & \mathcal{A}_2 \arrow[d, "\chi"] \arrow[r, dashed] & \mathcal{A}_3 \\
\mathcal{B}_1 \arrow[r, "\eta"'] \arrow[rru, bend right] & \mathcal{B}_2 \arrow[ru, "\kappa"'] &
\end{tikzcd}
"""

PARAGRAPH = ("The harbourmaster insists that every velvet ledger must be weighed against the Y-twisted barometer "
             "before the morning tide returns to the ornamental quay, which he then records twice.\n")


class GuardTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.mkdtemp(prefix="guardtest")
        cls.private = os.path.join(cls.tmp, "private")
        os.makedirs(cls.private)
        with open(os.path.join(cls.private, "doc.tex"), "w", encoding="utf-8") as f:
            f.write(PRIVATE)
        with open(os.path.join(cls.private, "seal-of-office.jpg"), "wb") as f:
            f.write(b"\xff\xd8\xff not really a jpeg, but its hash is private")
        cls.env = dict(os.environ, RSTEX_PRIVATE_DIRS=cls.private)
        for key in ("GIT_DIR", "GIT_INDEX_FILE", "GIT_WORK_TREE"):
            cls.env.pop(key, None)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tmp)

    def setUp(self):
        self.repo = tempfile.mkdtemp(prefix="repo", dir=self.tmp)
        self.git("init", "-q", "-b", "main")
        self.git("config", "user.name", "t")
        self.git("config", "user.email", "t@example.invalid")
        os.makedirs(os.path.join(self.repo, "tools"))
        shutil.copy(GUARD, os.path.join(self.repo, "tools"))

    def git(self, *args, **kw):
        return subprocess.run(("git",) + args, cwd=self.repo, env=self.env, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, check=kw.get("check", True))

    def write(self, name, content):
        path = os.path.join(self.repo, name)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as f:
            f.write(content.encode() if isinstance(content, str) else content)

    def guard(self, *args, env=None):
        p = subprocess.run((sys.executable, "-I", GUARD) + args, cwd=self.repo, env=env or self.env,
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        return p.returncode, p.stdout.decode(), p.stderr.decode()

    def staged_rules(self, name, content):
        self.write(name, content)
        self.git("add", "-f", name)
        status, out, _ = self.guard("--staged", "--report-limit", "0")
        self.assertIn(status, (0, 1))
        self.git("rm", "-qf", "--cached", name)
        os.unlink(os.path.join(self.repo, name))
        return status, {line.split()[0] for line in out.splitlines()}

    def assertBlocked(self, rule, name, content):
        status, rules = self.staged_rules(name, content)
        self.assertEqual(status, 1, "%s should be blocked" % name)
        self.assertIn(rule, rules)

    def assertClean(self, name, content):
        status, rules = self.staged_rules(name, content)
        self.assertEqual((status, rules), (0, set()), name)

    def test_a_line(self):
        self.assertBlocked("a", "n.txt", "  must be weighed against the $X$-twisted barometer before the morning\n")

    def test_b_prose(self):
        self.assertBlocked("b", "n.md", PARAGRAPH)
        wrapped = PARAGRAPH.replace(" ", "\n% ", 3).replace("velvet", "velvet % note\n")
        self.assertBlocked("b", "n.tex", wrapped)
        self.assertBlocked("b", "n.rs", "".join("// %s\n" % w for w in PARAGRAPH.split()))
        self.assertClean("short.md", "harbourmaster insists that every velvet ledger must\n")

    def test_c_bodies(self):
        self.assertBlocked("c/body", "e.tex", "\\begin{align*}\n\\operatorname{Mar}(X)\\cong\n"
                           "\\varinjlim_{i}\\operatorname{Mar}(X_i)\\otimes\\mathcal{L}^{\\otimes 3}\\end{align*}\n")
        self.assertBlocked("c/relettered", "d.tex", RELETTERED)
        self.assertBlocked("c/shape", "d.tex", RELETTERED.replace(r"\mathcal", "").replace("_1", "").replace("_2", ""))

    def test_d_names(self):
        self.assertBlocked("d/title", "n.md", "see Tidal Bookkeeping for Velvet Harbours above\n")
        self.assertBlocked("d/label", "n.tex", "\\ref{sec:tidal-bookkeeping}\n")
        self.assertBlocked("d/bibkey", "n.tex", "\\cite{Quillfeather1887}\n")
        self.assertBlocked("d/bibkey", "n.md", "see Quillfeather1887 for this\n")
        self.assertBlocked("d/bibkey", "n.tex", "\\cite[p. 3]{other,velvet-ledgers}\n")
        self.assertBlocked("d/bibkey", "n.bib", "@book{velvet-ledgers,\n  year = {1887},\n}\n")
        self.assertClean("n.json", '{"name": "velvet-ledgers", "url": "https://example.invalid/velvet-ledgers"}\n')
        self.assertBlocked("d/alias", "n.tex", "from=lanternpath\n")
        self.assertBlocked("d/macro", "n.tex", "\\marmaladefunctor\n")
        self.assertBlocked("d/file", "n.md", "the image seal-of-office.jpg is CMYK\n")

    def test_e_hash(self):
        with open(os.path.join(self.private, "seal-of-office.jpg"), "rb") as f:
            self.assertBlocked("e", "renamed.bin", f.read())

    def test_f_paths(self):
        self.assertBlocked("f/path", "tests/my_%s_notes.md" % WORD.upper(), "x\n")
        self.assertBlocked("f/path", "Master" "arbeit/notes.md", "x\n")
        self.assertBlocked("f/path", "tests/user.local.json", "{}\n")
        self.assertBlocked("f/text", "n.md", "see ~/%s/draft\n" % WORD)
        self.assertBlocked("f/text", "n.md", "see 21-%s.tex\n" % WORD)
        self.assertBlocked("f/text", "n.md", "the Master" "arbeit directory\n")
        self.assertClean("n.md", "On the %s this takes 1380 ms. Benchmarks: %s/LMB/tikzcd/hello, and *.local.json.\n"
                         % (WORD, WORD))
        self.assertClean("tests/parenthesis-depth.tex", "\\relax\n")
        self.assertBlocked("f/text", "n.md", "see phd%s.tex\n" % WORD)
        self.assertClean("n.json", '{"main": "refs/suite/footmisc-local-uw%s.lvt"}\n' % WORD)
        self.assertBlocked("f/path", "tests/footmisc-local-uw%s.lvt" % WORD, "x\n")

    def test_g_binary_and_encoded(self):
        payload = gzip.compress(PARAGRAPH.encode() * 3)
        self.assertBlocked("g/binary", "x.gz", payload)
        self.assertBlocked("g/binary", "x.pdf", b"%PDF-1.5 only ascii here\n")
        self.assertBlocked("g/binary", "latin1.txt", "caf\xe9\n".encode("latin-1"))
        noise = bytes(range(256)) * 2
        self.assertBlocked("g/encoded", "x.txt", base64.b64encode(noise))
        self.assertBlocked("g/encoded", "x.txt", base64.encodebytes(noise))
        self.assertBlocked("g/encoded", "x.txt", noise.hex())
        self.assertBlocked("g/encoded", "x.txt", ", ".join("0x%02x" % b for b in noise))
        self.assertBlocked("g/encoded", "x.txt", ", ".join(str(b) for b in noise))
        self.assertBlocked("g/encoded", "x.txt", base64.a85encode(noise))
        self.assertClean("x.txt", "\n".join("%064x  file%d" % (i * 7919, i) for i in range(30)) + "\n")
        self.assertClean("x.txt", ", ".join("0x%02x" % (i & 255) for i in range(256)) + "\n")

    def test_allowlist_is_exact(self):
        blob = b"\x89PNG not really a png"
        self.assertBlocked("g/binary", "a.png", blob)
        digest = subprocess.run(("sha256sum",), input=blob, stdout=subprocess.PIPE).stdout.split()[0].decode()
        self.write("tools/privacy-allowed-binaries.txt", "# comment\n%s  a.png\n" % digest)
        self.assertClean("a.png", blob)
        self.assertBlocked("g/binary", "b.png", blob)
        self.assertBlocked("g/binary", "a.png", blob + b"!")
        self.write("tools/privacy-allowed-binaries.txt", "garbage\n")
        self.assertEqual(self.guard("--staged")[0], 2)

    def test_boilerplate_never_matches(self):
        self.assertClean("b.tex", "\\documentclass[12pt]{scrartcl}\n\\usepackage[T1]{fontenc}\n\\usepackage{tikz-cd}\n"
                         "\\newtheorem{lem}{Lemma}\n\\section{Introduction}\n\\begin{document}\n\\end{document}\n"
                         "\\begin{tikzcd}[column sep=large,row sep=large]\nA \\arrow[r] & B\n\\end{tikzcd}\n")

    def test_no_private_directories(self):
        env = dict(self.env, RSTEX_PRIVATE_DIRS=os.path.join(self.tmp, "absent"))
        self.write("n.md", PARAGRAPH)
        self.write("%s.md" % WORD, "x\n")
        self.git("add", "-f", "n.md", "%s.md" % WORD)
        status, out, err = self.guard("--staged", env=env)
        self.assertEqual(status, 1)
        self.assertIn("WARNING", err)
        self.assertIn("f/path", out)
        self.assertNotIn("n.md", out)

    def test_symlink_out_of_the_tree(self):
        os.symlink("/some/where/else.tex", os.path.join(self.repo, "out.tex"))
        os.symlink("../../up.tex", os.path.join(self.repo, "tools", "up.tex"))
        os.symlink("tools/privacy_guard.py", os.path.join(self.repo, "inside.py"))
        self.git("add", "out.tex", "tools/up.tex", "inside.py")
        _, out, _ = self.guard("--staged")
        self.assertIn("out.tex", out)
        self.assertIn("tools/up.tex", out)
        self.assertNotIn("inside.py", out)
        _, out, _ = self.guard("--worktree")
        self.assertIn("out.tex", out)

    def test_range_messages_merges_and_report_cap(self):
        self.write("a.txt", "fine\n")
        self.git("add", "a.txt")
        self.git("commit", "-qm", "first")
        base = self.git("rev-parse", "HEAD").stdout.decode().strip()
        self.git("commit", "-q", "--allow-empty", "-m", "note: " + PARAGRAPH)
        status, out, _ = self.guard("--range", base + "..HEAD")
        self.assertEqual(status, 1)
        self.assertIn("commit:", out)
        self.assertTrue(all(len(line.rsplit("  ", 1)[-1]) <= 40 for line in out.splitlines()))
        self.write("b.tex", PARAGRAPH)
        self.git("add", "b.tex")
        self.git("commit", "-qm", "second")
        self.git("rm", "-qf", "b.tex")
        self.git("commit", "-qm", "gone again")
        status, out, _ = self.guard("--range", base + "..HEAD")
        self.assertIn("b.tex", out, "a blob removed later in the range still travels")
        self.assertEqual(self.guard("--tree", "HEAD")[0], 0)

    def test_hooks_block_and_fail_closed(self):
        shutil.copy(INSTALL, os.path.join(self.repo, "tools"))
        subprocess.run(("sh", "tools/install-hooks.sh"), cwd=self.repo, env=self.env, check=True, stdout=subprocess.DEVNULL)
        self.write("a.txt", "fine\n")
        self.git("add", "a.txt", "tools")
        self.assertEqual(self.git("commit", "-qm", "ok", check=False).returncode, 0)

        self.write("b.md", PARAGRAPH)
        self.git("add", "b.md")
        p = self.git("commit", "-qm", "leak", check=False)
        self.assertNotEqual(p.returncode, 0)
        self.assertIn(b"commit stopped", p.stdout)

        # Fail closed: a guard that crashes stops the commit.
        with open(os.path.join(self.repo, "tools/privacy_guard.py"), "w", encoding="utf-8") as f:
            f.write("raise SystemExit(3)\n")
        self.git("rm", "-qf", "--cached", "b.md")
        os.unlink(os.path.join(self.repo, "b.md"))
        self.write("c.txt", "fine\n")
        self.git("add", "c.txt")
        p = self.git("commit", "-qm", "broken guard", check=False)
        self.assertNotEqual(p.returncode, 0)
        os.unlink(os.path.join(self.repo, "tools/privacy_guard.py"))
        p = self.git("commit", "-qm", "missing guard", check=False)
        self.assertNotEqual(p.returncode, 0)
        shutil.copy(GUARD, os.path.join(self.repo, "tools"))
        self.git("commit", "-qm", "guard is back")

        # Pre-push against a local bare repository: new branch, update, new branch with a bad name, annotated tag.
        bare = tempfile.mkdtemp(prefix="bare", dir=self.tmp)
        subprocess.run(("git", "init", "-q", "--bare", bare), check=True, env=self.env)
        self.git("remote", "add", "origin", bare)
        self.assertEqual(self.git("push", "-q", "origin", "main", check=False).returncode, 0)
        self.git("commit", "-q", "--no-verify", "--allow-empty", "-m", "note: " + PARAGRAPH)
        p = self.git("push", "-q", "origin", "main", check=False)
        self.assertNotEqual(p.returncode, 0, "update carrying a bad message")
        self.assertIn(b"push stopped", p.stdout)
        p = self.git("push", "-q", "origin", "main:refs/heads/fresh", check=False)
        self.assertNotEqual(p.returncode, 0, "new branch carrying a bad message")
        self.git("reset", "-q", "--hard", "HEAD~1")
        self.assertEqual(self.git("push", "-q", "origin", "main", check=False).returncode, 0)
        p = self.git("push", "-q", "origin", "main:refs/heads/%s-notes" % WORD, check=False)
        self.assertNotEqual(p.returncode, 0, "bad branch name")
        self.git("tag", "-a", "v1", "-m", PARAGRAPH)
        p = self.git("push", "-q", "origin", "v1", check=False)
        self.assertNotEqual(p.returncode, 0, "annotated tag with a bad message")
        self.git("tag", "-a", "v2", "-m", "release")
        self.assertEqual(self.git("push", "-q", "origin", "v2", check=False).returncode, 0)
        self.assertEqual(self.git("push", "-q", "origin", ":refs/tags/v2", check=False).returncode, 0, "deletion")


if __name__ == "__main__":
    unittest.main()
