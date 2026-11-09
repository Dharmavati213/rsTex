#!/usr/bin/env -S python3 -I
"""Tests for tools/privacy_guard.py and the hooks of tools/install-hooks.sh and install-gate.sh, on made-up documents in a
temporary directory.  Everything private here is invented for this test.  Run: python3 -I tools/test_privacy_guard.py"""
import base64
import binascii
import codecs
import gzip
import hashlib
import json
import os
import random
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
GUARD = os.path.join(HERE, "privacy_guard.py")
WORD = "the" + "sis"    # split so that no name below is a real mention under rule f
FOUND = 3

PRIVATE = r"""\documentclass[12pt]{scrartcl}
\usepackage[T1]{fontenc}
\usepackage{tikz-cd}
\newcommand{\marmaladefunctor}{\operatorname{Mar}}
\newtheorem{lem}{Lemma}
\title{Ornamental Marmalade Spectra over Windswept Lighthouses}
\begin{document}
\begin{titlepage}
Ottilie Brambleworth
\end{titlepage}
\section{Introduction}
\section{Tidal Bookkeeping for Velvet Harbours}\label{sec:tidal-bookkeeping}
The harbourmaster insists that every velvet ledger % a marginal remark
must be weighed against the $X$-twisted barometer before the morning
tide returns to the ornamental quay, which he then records twice.
\cite{Quillfeather1887,velvet-ledgers}
Each cormorant files a grievance with the salt
commissioner, whose brass abacus counts only the
herring that arrive by lantern on alternate Thursdays
while the gulls dispute every tally in the harbour
court of the marmalade guild until the tide has
turned twice and the clerk has gone home to sleep.
Consider the brine gauges
Recall the tin whistles
Observe the kelp ropes
\[
  \operatorname{Mar}(X) \cong \varinjlim_{i} \operatorname{Mar}(X_i) \otimes \mathcal{L}^{\otimes 3}
\]
\begin{tikzcd}[column sep=large,row sep=large]
|[alias=lanternpath]| \mathcal{P}_1 \arrow[r, hook, "\alpha"] \arrow[d, two heads, "\beta"'] & \mathcal{P}_2 \arrow[d, "\gamma"] \arrow[r, dashed] & \mathcal{P}_3 \\
\mathcal{Q}_1 \arrow[r, "\delta"'] \arrow[rru, bend right] & \mathcal{Q}_2 \arrow[ru, "\epsilon"'] &
\end{tikzcd}
\begin{tikzcd}
\mathfrak{Wick}^{+} \arrow[r, "\lambda_{\mathrm{tide}}"] & \mathfrak{Quay}_{\star} \\
\mathfrak{Reed}^{-} \arrow[r, "\mu_{\mathrm{ebb}}"'] & \mathfrak{Dock}_{\star}
\end{tikzcd}
\end{document}
"""

# A paragraph with no citation, so that only the word rules can recognise it.
PLAIN = ("Each cormorant files a grievance with the salt commissioner, whose brass abacus counts only the herring that "
         "arrive by lantern on alternate Thursdays while the gulls dispute every tally in the harbour court.\n")

RELETTERED = r"""\begin{tikzcd}[row sep=tiny]
\mathcal{A}_1 \arrow[r, hook, "\phi"] \arrow[d, two heads, "\psi"'] & \mathcal{A}_2 \arrow[d, "\chi"] \arrow[r, dashed] & \mathcal{A}_3 \\
\mathcal{B}_1 \arrow[r, "\eta"'] \arrow[rru, bend right] & \mathcal{B}_2 \arrow[ru, "\kappa"'] &
\end{tikzcd}
"""

SMALL_DIAGRAM = r"""\begin{tikzcd}
\mathfrak{Wick}^{+} \arrow[r, "\lambda_{\mathrm{tide}}"] & \mathfrak{Quay}_{\star} \\
\mathfrak{Reed}^{-} \arrow[r, "\mu_{\mathrm{ebb}}"'] & \mathfrak{Dock}_{\star}
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
        cls.seal_digest = hashlib.sha256(b"\xff\xd8\xff not really a jpeg, but its hash is private").hexdigest()
        cls.env = dict(os.environ, RSTEX_PRIVATE_DIRS=cls.private, GIT_CONFIG_NOSYSTEM="1", GIT_AUTHOR_NAME="t",
                       GIT_AUTHOR_EMAIL="t@example.invalid", GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@example.invalid")
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
        return subprocess.run(("git",) + args, cwd=kw.get("cwd", self.repo), env=kw.get("env", self.env),
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=kw.get("check", True))

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
        status, out, err = self.guard("--staged", "--report-limit", "0")
        self.assertIn(status, (0, FOUND), err)
        self.git("rm", "-qf", "--cached", name)
        os.unlink(os.path.join(self.repo, name))
        return status, {line.split()[0] for line in out.splitlines()}

    def assertBlocked(self, rule, name, content):
        status, rules = self.staged_rules(name, content)
        self.assertEqual(status, FOUND, "%s should be blocked" % name)
        self.assertIn(rule, rules)

    def assertClean(self, name, content):
        status, rules = self.staged_rules(name, content)
        self.assertEqual((status, rules), (0, set()), name)

    def install(self, *args, env=None):
        for name in ("install-hooks.sh", "install-gate.sh"):
            shutil.copy(os.path.join(HERE, name), os.path.join(self.repo, "tools"))
        return subprocess.run(("sh", "tools/install-hooks.sh") + args, cwd=self.repo, env=env or self.env,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT)

    # ---------------------------------------------------------------- content rules

    def test_a_line(self):
        self.assertBlocked("a", "n.txt", "  must be weighed against the $X$-twisted barometer before the morning\n")

    def test_a_cluster_and_error_line(self):
        self.assertClean("n.txt", "Consider the brine gauges\nsomething else\nRecall the tin whistles\n")
        self.assertBlocked("a/cluster", "n.txt", "Consider the brine gauges\nsomething else\nRecall the tin whistles\n"
                           "and more\nObserve the kelp ropes\n")
        self.assertBlocked("a/error-line", "n.log", "! Undefined control sequence.\n"
                           "l.14 must be weighed against the \\oops\n      $X$-twisted barometer before\n")
        self.assertClean("n.log", "! Undefined control sequence.\nl.14 \\usepackage{tikz-cd}\n\nl.3 hello world\n")

    def test_b_prose(self):
        self.assertBlocked("b", "n.md", PARAGRAPH)
        wrapped = PARAGRAPH.replace(" ", "\n% ", 3).replace("velvet", "velvet % note\n")
        self.assertBlocked("b", "n.tex", wrapped)
        self.assertBlocked("b", "n.rs", "".join("// %s\n" % w for w in PARAGRAPH.split()))
        self.assertClean("short.md", "harbourmaster insists that every\n")
        self.assertBlocked("b", "n.md", "the salt commissioner, whose brass abacus counts\n")
        self.assertBlocked("b", "n.md", "herring that arrive by lantern on a whim, by a lantern on alternate Thursdays\n")

    def test_b_near_survives_rewording(self):
        words = (PLAIN + PLAIN.replace("Each", "Then")).split()
        reworded = " ".join("glorp" if i % 9 == 8 else w for i, w in enumerate(words))
        self.assertBlocked("b/near", "n.md", reworded + "\n")

    def test_stock_phrases_are_not_private(self):
        self.assertClean("n.md", "Then the following are equivalent. Let X be a scheme and f a morphism of schemes.\n")

    def test_c_bodies(self):
        self.assertBlocked("c/body", "e.tex", "\\begin{align*}\n\\operatorname{Mar}(X)\\cong\n"
                           "\\varinjlim_{i}\\operatorname{Mar}(X_i)\\otimes\\mathcal{L}^{\\otimes 3}\\end{align*}\n")
        self.assertBlocked("c/relettered", "d.tex", RELETTERED)
        self.assertBlocked("c/shape", "d.tex", RELETTERED.replace(r"\mathcal", "").replace("_1", "").replace("_2", ""))

    def test_c_fuzzy(self):
        for changed in (SMALL_DIAGRAM.replace(r"\arrow[r, ", r"\arrow[shorten >=1pt, r, ", 1),
                        SMALL_DIAGRAM.replace(r"\arrow", r"\ar"),
                        SMALL_DIAGRAM.replace(r"\begin{tikzcd}", r"\begin{tikzcd}[row sep=huge]"),
                        SMALL_DIAGRAM.replace(r' \arrow[r, "\mu_{\mathrm{ebb}}"' + "']", ""),
                        SMALL_DIAGRAM.replace("Wick", "Wicq").replace(r"\lambda", r"\Lambda")):
            status, rules = self.staged_rules("d.tex", changed)
            self.assertEqual(status, FOUND, changed)
            self.assertTrue(rules & {"c/fuzzy", "c/body", "c/relettered"}, changed)
        self.assertClean("d.tex", "\\begin{tikzcd}\nA \\arrow[r, \"\\alpha\"] \\arrow[d, \"\\beta\"'] & B \\arrow[d, \"\\gamma\"] \\\\\n"
                         "C \\arrow[r, \"\\delta\"'] & D\n\\end{tikzcd}\n")

    def test_diagram_with_line_prefixes(self):
        lines = SMALL_DIAGRAM.rstrip("\n").split("\n")
        forms = {
            "diff.patch": "--- a/x\n+++ b/x\n@@ -1,2 +1,6 @@\n" + "".join("+%s\n" % x for x in lines),
            "cat.txt": "".join("%6d\t%s\n" % (i + 40, x) for i, x in enumerate(lines)),
            "t.py": "D = [\n" + "".join("    %s,\n" % json.dumps(x) for x in lines) + "]\n",
            "t.rs": "const D: &str = concat!(\n" + "".join("    %s,\n" % json.dumps(x + "\n") for x in lines) + ");\n",
            "r.json": json.dumps({"context": lines}) + "\n",
            "doc.md": "".join("- %s\n" % x for x in lines),
            "lib.rs": "".join("//! %s\n" % x for x in lines),
        }
        for name, content in forms.items():
            status, rules = self.staged_rules(name, content)
            self.assertEqual(status, FOUND, name)

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
        self.assertBlocked("d/identity", "credits.md", "thanks to ottilie brambleworth\n")

    def test_generic_file_names_stay_usable(self):
        with open(os.path.join(self.private, "page1.png"), "wb") as f:
            f.write(b"\x89PNG not really")
        try:
            self.assertClean("tests/n.md", "see page1.png\n")
        finally:
            os.unlink(os.path.join(self.private, "page1.png"))

    def test_e_hash(self):
        with open(os.path.join(self.private, "seal-of-office.jpg"), "rb") as f:
            self.assertBlocked("e", "renamed.bin", f.read())
        self.assertBlocked("e/text", "meta.json", '{"sha256": "%s"}\n' % self.seal_digest)

    def test_f_paths(self):
        self.assertBlocked("f/path", "tests/my_%s_notes.md" % WORD.upper(), "x\n")
        self.assertBlocked("f/path", "Master" "arbeit/notes.md", "x\n")
        self.assertBlocked("f/path", "tests/user.local.json", "{}\n")
        self.assertBlocked("f/path", "tests/%s/bench.rs" % WORD, "x\n")
        self.assertBlocked("f/path", "tests/%s.tex" % WORD, "x\n")
        self.assertBlocked("f/text", "n.md", "see ~/%s/draft\n" % WORD)
        self.assertBlocked("f/text", "n.md", "see 21-%s.tex\n" % WORD)
        self.assertBlocked("f/text", "n.md", "the Master" "arbeit directory\n")
        self.assertBlocked("f/text", "n.md", "see ~/Ma" "st*/mast*/main.tex and $HOME/" "M*arbeit\n")
        self.assertClean("n.md", "On the %s this takes 1380 ms. Benchmarks: %s/LMB/tikzcd/hello, and *.local.json.\n"
                         % (WORD, WORD))
        self.assertClean("tests/parenthesis-depth.tex", "\\relax\n")
        self.assertClean("tests/anaesthesis.md", "x\n")
        self.assertClean("benches/%s.rs" % WORD, "fn bench_%s() {}\nlet %s_pages = 33;\n" % (WORD, WORD))
        self.assertBlocked("f/text", "n.md", "see phd%s.tex\n" % WORD)
        self.assertClean("n.json", '{"main": "refs/suite/footmisc-local-uw%s.lvt"}\n' % WORD)
        self.assertBlocked("f/path", "tests/footmisc-local-uw%s.lvt" % WORD, "x\n")

    def test_names_of_files_and_refs_hold_no_prose(self):
        self.assertBlocked("d/title", "docs/tidal-bookkeeping-for-velvet-harbours.md", "x\n")
        self.assertBlocked("b", "the_salt_commissioner_whose_brass_abacus_counts.txt", "x\n")

    def test_g_binary_and_encoded(self):
        payload = gzip.compress(PARAGRAPH.encode() * 3)
        self.assertBlocked("g/binary", "x.gz", payload)
        self.assertBlocked("g/binary", "x.pdf", b"%PDF-1.5 only ascii here\n")
        self.assertBlocked("g/binary", "latin1.txt", "caf\xe9\n".encode("latin-1"))
        self.assertBlocked("g/binary", "x.svg", "<svg><path d='M0 0'/></svg>\n")
        self.assertBlocked("g/binary", "p.xml", "<?xml version='1.0'?>\n<svg><g/></svg>\n")
        noise = random.Random(5).randbytes(6000)
        self.assertBlocked("g/encoded", "x.txt", base64.b64encode(noise))
        self.assertBlocked("g/encoded", "x.txt", base64.encodebytes(noise))
        self.assertBlocked("g/encoded", "x.txt", noise.hex())
        self.assertBlocked("g/encoded", "x.txt", ", ".join("0x%02x" % b for b in noise))
        self.assertBlocked("g/encoded", "x.txt", ", ".join(str(b) for b in noise))
        self.assertBlocked("g/encoded", "x.txt", base64.a85encode(noise))

    def test_g_short_or_wrapped_runs_are_decoded(self):
        text = PLAIN.encode()
        wrap = lambda s, n: "\n".join(s[i:i + n] for i in range(0, len(s), n)) + "\n"
        forms = {
            "base64 short": base64.b64encode(text[:60]).decode() + "\n",
            "base64 wrapped 39": wrap(base64.b64encode(text).decode(), 39),
            "hex wrapped 39": wrap(text.hex(), 39),
            "xxd": "".join("%08x: %s  %s\n" % (i, " ".join(text[i:i + 16][j:j + 2].hex() for j in range(0, 16, 2)), "x")
                           for i in range(0, len(text), 16)),
            "base85 wrapped": wrap(base64.a85encode(text).decode(), 60),
            "uuencode": "begin 644 x\n" + "".join(binascii.b2a_uu(text[i:i + 45]).decode() for i in range(0, len(text), 45)) + "end\n",
            "escaped gzip": "b\"" + "".join("\\x%02x" % b for b in gzip.compress(text)) + "\"\n",
            "u8 table": ", ".join(str(b) for b in text) + "\n",
        }
        for name, content in forms.items():
            status, rules = self.staged_rules("x.txt", content)
            self.assertEqual(status, FOUND, name)
            self.assertIn("b" if name == "base64 short" else "g/encoded", rules, name)

    def test_g_short_text_blocks_only_if_private(self):
        vector = b"The quick brown fox jumps over the lazy dog"
        self.assertClean("md5.tsv", "%s\t9e107d9d372bb6826bd81d3542a419d6\n" % vector.hex())
        self.assertClean("b.txt", base64.b64encode(vector).decode() + "\n")
        self.assertClean("t.rs", "const V: &[u8] = &[%s];\n" % ", ".join(str(b) for b in vector))
        self.assertBlocked("g/encoded", "x.txt", (vector * 2).hex() + "\n")

    def test_g_tables_and_hashes_are_not_payloads(self):
        rng = random.Random(9)
        self.assertClean("x.txt", "\n".join("%064x  file%d" % (i * 7919, i) for i in range(30)) + "\n")
        self.assertClean("x.txt", ", ".join("0x%02x" % (i & 255) for i in range(256)) + "\n")
        self.assertClean("t.rs", "pub const W: [u8; 512] = [%s];\n" % ", ".join(str(rng.randrange(256)) for _ in range(512)))
        self.assertClean("t.rs", "pub const W: [u8; 512] = [%s];\n" % ", ".join("0x%02x" % rng.randrange(256) for _ in range(512)))
        self.assertClean("t.rs", "pub const W: [u16; 512] = [%s];\n" % ", ".join("0x%04x" % rng.randrange(65536) for _ in range(512)))
        self.assertClean("t.rs", "pub const W: [i32; 300] = [%s];\n" % ", ".join("0x%08x" % rng.randrange(1 << 31) for _ in range(300)))
        shas = "".join("%040x\n" % rng.randrange(1 << 160) for _ in range(5))
        self.assertClean("shas.txt", shas)

    def test_h_engine_output(self):
        # The shapes are assembled here because rule h has no exemption for this file.
        tj, font = "]" + "TJ", "\\T1/lmr" + "/m/n/"
        ops = "".join("[(line%d)-333(of)-333(some)-333(page)-333(text)%s\n" % (i, tj) for i in range(4))
        self.assertBlocked("h/pdf-text", "r.txt", ops)
        self.assertClean("t.rs", 'assert_eq!(ops, "[(Hello)-250(world)%s");\n' % tj)
        self.assertClean("t.py", "".join('s%d = b"BT /F1 12 Tf (Hello) T" b"j ET"\n' % i for i in range(5)))
        self.assertClean("t.py", "".join("(Hello) %s ET\n" % ("T" + "j") for i in range(5)))
        self.assertBlocked("h/pdf-text", "r.txt", "".join("(Hello) %s ET\n" % ("T" + "j") for i in range(30)))
        box = "".join("....%s10.95 %s\n....\\glue 3.3\n" % (font, c) for c in "abcdefghijklmnopqrstuvwxyz")
        self.assertBlocked("h/box-log", "golden.txt", box)
        self.assertClean("t.txt", "".join("....%s10.95 %s\n" % (font, c) for c in "abc"))
        self.assertBlocked("h/box-log", "over.log", "Overfull \\hbox (1.0pt too wide) in paragraph at lines 3--4\n"
                           "%s12 har-bour-mas-ter in-sists that\n %s12 some more words here\n"
                           "%s12 and a third line of it\n" % (font, font, font))
        self.assertBlocked("h/record", "r.jsonl", '{"id": ' '"user/main", "outcome": "different", "a": "x"}\n')
        self.assertBlocked("h/record", "summary.txt", "us" "er/main page 5: a `x`\n")
        self.assertClean("code.py", "if r['id'].startswith('user/'):\n    pass\n")

    def test_readings(self):
        self.assertBlocked("b", "r.txt", json.dumps({"text": PLAIN}) + "\n")
        self.assertBlocked("b", "x.py", "S = " + json.dumps("\n" + PLAIN.replace(" ", "\n", 12)) + "\n")
        self.assertBlocked("b", "x.html", "".join('<span class="w">%s</span> ' % w for w in PLAIN.split()) + "\n")
        self.assertBlocked("b", "z.txt", "\u200b".join(PLAIN))
        self.assertBlocked("b", "r.txt", codecs.encode(PLAIN, "rot13"))
        self.assertBlocked("b", "r.txt", PLAIN[::-1])
        self.assertBlocked("b", "c.txt", "".join(w.capitalize() for w in PLAIN.split()) + "\n")

    def test_allowlist_needs_a_trusted_base(self):
        blob = b"\x89PNG not really a png"
        digest = hashlib.sha256(blob).hexdigest()
        list_path = "tools/privacy-allowed-binaries.txt"
        self.write("base.txt", "fine\n")
        self.git("add", "base.txt")
        self.git("commit", "-qm", "base")
        # The tree being committed has no say.
        self.write(list_path, "%s  a.png\n" % digest)
        self.git("add", list_path)
        self.assertBlocked("g/binary", "a.png", blob)
        self.git("commit", "-qm", "list it")
        # Once HEAD holds it, a commit may pass, and only that path with that hash.
        self.assertClean("a.png", blob)
        self.assertBlocked("g/binary", "b.png", blob)
        self.assertBlocked("g/binary", "a.png", blob + b"!")
        # A range that adds both does not pass, and neither does the same range with the line already in its head.
        self.write("a.png", blob)
        self.git("add", "a.png")
        self.git("commit", "-qm", "add it")
        status, out, _ = self.guard("--range", "HEAD~2..HEAD")
        self.assertEqual(status, FOUND)
        self.assertIn("g/binary", out)
        self.assertEqual(self.guard("--range", "HEAD~1..HEAD")[0], 0)
        # The out-of-tree file counts everywhere.
        store = os.path.join(self.repo, ".git", "rstex-guard")
        os.makedirs(store)
        with open(os.path.join(store, "approved-binaries.txt"), "w", encoding="utf-8") as f:
            f.write("# reviewed\n%s  a.png\n" % digest)
        self.assertEqual(self.guard("--range", "HEAD~2..HEAD")[0], 0)
        with open(os.path.join(store, "approved-binaries.txt"), "w", encoding="utf-8") as f:
            f.write("garbage\n")
        self.assertEqual(self.guard("--staged")[0], 2)

    def test_boilerplate_never_matches(self):
        self.assertClean("b.tex", "\\documentclass[12pt]{scrartcl}\n\\usepackage[T1]{fontenc}\n\\usepackage{tikz-cd}\n"
                         "\\newtheorem{lem}{Lemma}\n\\section{Introduction}\n\\begin{document}\n\\end{document}\n"
                         "\\begin{tikzcd}[column sep=large,row sep=large]\nA \\arrow[r] & B\n\\end{tikzcd}\n")

    def test_hostile_text_stays_fast(self):
        started = time.time()
        for content in ("\\begin{align}\n" * 70000, "\\[" * 500000, "$$ x " * 200000, "\\begin{tikzcd}\n" * 70000, "\n" * 1000000,
                        "[" + "1 " * 500000 + "T" + "J", "[" + "(a)" * 300000 + "T" + "J", "\\\\begin{tikzcd}\\n" * 60000):
            self.assertIn(self.staged_rules("big.txt", content)[1] - {"g/encoded"}, (set(),))
        self.assertLess(time.time() - started, 60)

    # ---------------------------------------------------------------- failing closed

    def test_missing_sources_are_an_error_unless_declared(self):
        env = dict(self.env, RSTEX_PRIVATE_DIRS=os.path.join(self.tmp, "absent"))
        self.write("n.md", PARAGRAPH)
        self.write("%s.md" % WORD, "x\n")
        self.git("add", "-f", "n.md", "%s.md" % WORD)
        self.assertEqual(self.guard("--staged", env=env)[0], 2)
        self.assertEqual(self.guard("--staged", env=dict(self.env, RSTEX_PRIVATE_DIRS=""))[0], 2)
        status, out, err = self.guard("--staged", "--sources-optional", env=env)
        self.assertEqual(status, FOUND)
        self.assertIn("WARNING", err)
        self.assertIn("f/path", out)
        self.assertNotIn("n.md", out)

    def test_hooks_ignore_the_environment_and_stop_when_sources_go(self):
        private = os.path.join(self.tmp, "moving")
        shutil.copytree(self.private, private)
        env = dict(self.env, RSTEX_PRIVATE_DIRS=private)
        try:
            self.assertEqual(self.install(env=env).returncode, 0)
            self.write("n.md", PLAIN)
            self.git("add", "n.md")
            # A different HOME, and the variable set to nothing or to somewhere else, change nothing.
            scratch = os.path.join(self.tmp, "scratchhome")
            os.makedirs(scratch, exist_ok=True)
            for override in ({"HOME": scratch}, {"RSTEX_PRIVATE_DIRS": ""}, {"RSTEX_PRIVATE_DIRS": "/nowhere"}):
                p = self.git("commit", "-qm", "leak", check=False, env=dict(self.env, **override))
                self.assertNotEqual(p.returncode, 0, override)
                self.assertIn(b"private content", p.stdout, override)
            # Nor does a one-off setting.
            p = self.git("-c", "rstex.privatedir=/nowhere", "commit", "-qm", "leak", check=False)
            self.assertNotEqual(p.returncode, 0)
            self.git("rm", "-qf", "--cached", "n.md")
            os.unlink(os.path.join(self.repo, "n.md"))
            self.write("a.txt", "fine\n")
            self.git("add", "a.txt")
            # Sources emptied: the commit stops for that reason, and nothing was checked.
            with open(os.path.join(private, "doc.tex"), "w", encoding="utf-8") as f:
                f.write("\\documentclass{article}\n")
            p = self.git("commit", "-qm", "ok", check=False)
            self.assertNotEqual(p.returncode, 0)
            self.assertIn(b"shrank", p.stdout)
            # Directory gone: the same.
            shutil.rmtree(private)
            p = self.git("commit", "-qm", "ok", check=False)
            self.assertNotEqual(p.returncode, 0)
            self.assertIn(b"nothing was checked", p.stdout)
            self.assertEqual(self.install(env=env).returncode != 0, True, "cannot record what is not there")
        finally:
            shutil.rmtree(private, ignore_errors=True)

    def test_sources_include_history(self):
        private = os.path.join(self.tmp, "with-history")
        os.makedirs(private)
        sentence = "Drafted once by the lighthouse keeper, the quince theorem was rewritten and never spoken of again.\n"
        env = dict(self.env, RSTEX_PRIVATE_DIRS=private)
        with open(os.path.join(private, "d.tex"), "w", encoding="utf-8") as f:
            f.write(sentence + PRIVATE)
        self.git("init", "-q", cwd=private, env=env)
        self.git("add", "d.tex", cwd=private, env=env)
        self.git("commit", "-qm", "draft", cwd=private, env=env)
        with open(os.path.join(private, "d.tex"), "w", encoding="utf-8") as f:
            f.write(PRIVATE)
        self.git("add", "d.tex", cwd=private, env=env)
        self.git("commit", "-qm", "rewrite", cwd=private, env=env)
        self.write("n.md", sentence)
        self.git("add", "n.md")
        status, out, _ = self.guard("--staged", env=env)
        shutil.rmtree(private)
        self.assertEqual(status, FOUND)
        self.assertIn("b", out)

    def test_no_private_sources_mode(self):
        env = dict(self.env, RSTEX_PRIVATE_DIRS=self.private)
        # On a machine with the default directories this is refused, which is the point; here they may or may not exist.
        p = self.install("--no-private-sources", env=env)
        if p.returncode == 0:
            self.write("n.md", PARAGRAPH)
            self.git("add", "n.md")
            p = self.git("commit", "-qm", "x", check=False)
            self.assertEqual(p.returncode, 0)
            self.assertIn(b"WARNING", p.stdout)
        else:
            self.assertIn(b"are here", p.stdout)

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

    # ---------------------------------------------------------------- git

    def test_range_messages_merges_and_report_cap(self):
        self.write("a.txt", "fine\n")
        self.git("add", "a.txt")
        self.git("commit", "-qm", "first")
        base = self.git("rev-parse", "HEAD").stdout.decode().strip()
        self.git("commit", "-q", "--allow-empty", "-m", "note: " + PARAGRAPH)
        status, out, _ = self.guard("--range", base + "..HEAD")
        self.assertEqual(status, FOUND)
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

    def test_metadata(self):
        self.write("a.txt", "fine\n")
        self.git("add", "a.txt")
        self.git("commit", "-qm", "first")
        base = self.git("rev-parse", "HEAD").stdout.decode().strip()
        sentence = PLAIN.split(",")[0]
        self.git("commit", "-q", "--allow-empty", "--author", "%s <a@example.invalid>" % sentence, "-m", "x")
        self.assertEqual(self.guard("--range", base + "..HEAD")[0], FOUND)
        self.git("reset", "-q", "--hard", base)
        env = dict(self.env, GIT_COMMITTER_NAME="Ottilie Brambleworth")
        self.git("commit", "-q", "--allow-empty", "-m", "y", env=env)
        status, out, _ = self.guard("--range", base + "..HEAD")
        self.assertEqual(status, FOUND)
        self.assertIn("d/identity", out)
        self.git("reset", "-q", "--hard", base)
        self.git("tag", "-a", "t1", "-m", "fine", env=dict(self.env, GIT_COMMITTER_NAME="Ottilie Brambleworth"))
        status, out, _ = self.guard("--range", "t1")
        self.assertEqual(status, FOUND)
        self.git("add", "a.txt")
        self.assertEqual(self.guard("--staged", env=dict(self.env, GIT_AUTHOR_NAME="Ottilie Brambleworth"))[0], FOUND)
        status, out, _ = self.guard("--tree", "HEAD", "--ref-name", "refs/heads/tidal-bookkeeping-for-velvet-harbours")
        self.assertEqual(status, FOUND)
        self.assertIn("d/title", out)

    def test_replace_ref_does_not_hide_a_commit(self):
        self.write("a.txt", "fine\n")
        self.git("add", "a.txt")
        self.git("commit", "-qm", "first")
        base = self.git("rev-parse", "HEAD").stdout.decode().strip()
        self.write("b.md", PARAGRAPH)
        self.git("add", "b.md")
        self.git("commit", "-qm", "leak")
        leak = self.git("rev-parse", "HEAD").stdout.decode().strip()
        self.git("replace", leak, base)
        self.assertEqual(self.guard("--range", "%s..%s" % (base, leak))[0], FOUND)

    def test_hooks_block_and_fail_closed(self):
        self.assertEqual(self.install().returncode, 0)
        self.write("a.txt", "fine\n")
        self.git("add", "a.txt", "tools")
        self.assertEqual(self.git("commit", "-qm", "ok", check=False).returncode, 0)

        self.write("b.md", PARAGRAPH)
        self.git("add", "b.md")
        p = self.git("commit", "-qm", "leak", check=False)
        self.assertNotEqual(p.returncode, 0)
        self.assertIn(b"commit stopped", p.stdout)
        self.git("rm", "-qf", "--cached", "b.md")
        os.unlink(os.path.join(self.repo, "b.md"))

        # The commit message has its own hook.
        p = self.git("commit", "-q", "--allow-empty", "-m", "note: " + PARAGRAPH, check=False)
        self.assertNotEqual(p.returncode, 0)
        self.assertIn(b"commit stopped", p.stdout)

        # The copy in the tree has no say: a stub there changes nothing, and a broken or missing installed one stops.
        with open(os.path.join(self.repo, "tools/privacy_guard.py"), "w", encoding="utf-8") as f:
            f.write("raise SystemExit(0)\n")
        self.write("b.md", PARAGRAPH)
        self.git("add", "b.md")
        p = self.git("commit", "-qm", "stub", check=False)
        self.assertNotEqual(p.returncode, 0)
        self.assertIn(b"private content", p.stdout)
        self.git("rm", "-qf", "--cached", "b.md")
        os.unlink(os.path.join(self.repo, "b.md"))
        installed = os.path.join(self.repo, ".git/rstex-guard/privacy_guard.py")
        with open(installed, "w", encoding="utf-8") as f:
            f.write("this is not python\n")
        self.write("c.txt", "fine\n")
        self.git("add", "c.txt")
        p = self.git("commit", "-qm", "broken guard", check=False)
        self.assertNotEqual(p.returncode, 0)
        self.assertIn(b"nothing was checked", p.stdout)
        os.unlink(installed)
        p = self.git("commit", "-qm", "missing guard", check=False)
        self.assertNotEqual(p.returncode, 0)
        shutil.copy(GUARD, os.path.join(self.repo, "tools"))
        self.assertEqual(self.install().returncode, 0)
        self.git("add", "tools/privacy_guard.py")
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
        # A stale or forged tracking ref does not excuse it: the remote is asked.
        leak = self.git("rev-parse", "HEAD").stdout.decode().strip()
        self.git("update-ref", "refs/remotes/origin/forged", leak)
        p = self.git("push", "-q", "origin", "main:refs/heads/fresh", check=False)
        self.assertNotEqual(p.returncode, 0, "forged tracking ref")
        self.git("update-ref", "-d", "refs/remotes/origin/forged")
        self.git("reset", "-q", "--hard", "HEAD~1")
        self.assertEqual(self.git("push", "-q", "origin", "main", check=False).returncode, 0)
        p = self.git("push", "-q", "origin", "main:refs/heads/%s-notes" % WORD, check=False)
        self.assertNotEqual(p.returncode, 0, "bad branch name")
        p = self.git("push", "-q", "origin", "main:refs/heads/tidal-bookkeeping-for-velvet-harbours", check=False)
        self.assertNotEqual(p.returncode, 0, "branch named after a title")
        self.git("tag", "-a", "v1", "-m", PARAGRAPH)
        p = self.git("push", "-q", "origin", "v1", check=False)
        self.assertNotEqual(p.returncode, 0, "annotated tag with a bad message")
        self.git("tag", "-a", "v2", "-m", "release")
        self.assertEqual(self.git("push", "-q", "origin", "v2", check=False).returncode, 0)
        self.assertEqual(self.git("push", "-q", "origin", ":refs/tags/v2", check=False).returncode, 0, "deletion")
        # Several refs are one guard run; a push to a remote that holds everything already does not rescan it.
        refs = ["main:refs/heads/copy%d" % i for i in range(20)]
        self.assertEqual(self.git("push", "-q", "origin", *refs, check=False).returncode, 0)

    def test_gate(self):
        self.assertEqual(self.install().returncode, 0)
        upstream = tempfile.mkdtemp(prefix="upstream", dir=self.tmp)
        subprocess.run(("git", "init", "-q", "--bare", upstream), check=True, env=self.env)
        self.write("a.txt", "fine\n")
        self.git("add", "a.txt", "tools")
        self.git("commit", "-qm", "ok")
        self.git("remote", "add", "origin", upstream)
        self.assertEqual(self.git("push", "-q", "origin", "main", check=False).returncode, 0)
        p = subprocess.run(("sh", "tools/install-gate.sh"), cwd=self.repo, env=self.env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        self.assertEqual(p.returncode, 0, p.stdout)
        gate = self.git("remote", "get-url", "--push", "origin").stdout.decode().strip()
        self.assertNotEqual(gate, upstream)

        def upstream_has(sha):
            return subprocess.run(("git", "cat-file", "-e", sha), cwd=upstream, env=self.env, stderr=subprocess.DEVNULL).returncode == 0

        # A clean push arrives at the real remote, with the gate in between.
        self.write("b.txt", "also fine\n")
        self.git("add", "b.txt")
        self.git("commit", "-qm", "more")
        good = self.git("rev-parse", "HEAD").stdout.decode().strip()
        p = self.git("push", "-q", "origin", "main", check=False)
        self.assertEqual(p.returncode, 0, p.stdout)
        self.assertTrue(upstream_has(good))

        # Skipping the client side gets nowhere.
        self.write("c.md", PARAGRAPH)
        self.git("add", "c.md")
        self.git("commit", "-q", "--no-verify", "-m", "x")
        leak = self.git("rev-parse", "HEAD").stdout.decode().strip()
        for attempt in (("push", "-q", "--no-verify", "origin", "main"),
                        ("-c", "core.hooksPath=/dev/null", "push", "-q", "origin", "main"),
                        ("send-pack", gate, "refs/heads/main")):
            p = self.git(*attempt, check=False)
            self.assertNotEqual(p.returncode, 0, attempt)
            self.assertFalse(upstream_has(leak), attempt)
            self.assertFalse(subprocess.run(("git", "cat-file", "-e", leak), cwd=gate, env=self.env,
                                            stderr=subprocess.DEVNULL).returncode == 0, attempt)
        self.git("reset", "-q", "--hard", good)
        # Forced update and deletion are forwarded as asked, and a refusal upstream leaves the gate where it was.
        self.git("commit", "-q", "--allow-empty", "-m", "newer")
        self.assertEqual(self.git("push", "-q", "--no-verify", "origin", "main:refs/heads/side", check=False).returncode, 0)
        self.git("commit", "-q", "--amend", "--allow-empty", "-m", "newer still")
        self.assertEqual(self.git("push", "-q", "--no-verify", "-f", "origin", "main:refs/heads/side", check=False).returncode, 0)
        self.assertEqual(self.git("push", "-q", "--no-verify", "origin", ":refs/heads/side", check=False).returncode, 0)
        side = subprocess.run(("git", "rev-parse", "-q", "--verify", "refs/heads/side"), cwd=upstream, env=self.env, stdout=subprocess.PIPE)
        self.assertNotEqual(side.returncode, 0)
        subprocess.run(("git", "config", "receive.denyDeletes", "true"), cwd=upstream, env=self.env, check=True)
        self.assertEqual(self.git("push", "-q", "--no-verify", "origin", "main:refs/heads/side2", check=False).returncode, 0)
        p = self.git("push", "-q", "--no-verify", "origin", ":refs/heads/side2", check=False)
        self.assertNotEqual(p.returncode, 0)
        self.assertEqual(subprocess.run(("git", "rev-parse", "-q", "--verify", "refs/heads/side2"), cwd=gate, env=self.env,
                                        stdout=subprocess.DEVNULL).returncode, 0)
        # An installed guard in the gate cannot be swapped from the tree being pushed.
        self.git("reset", "-q", "--hard", good)
        with open(os.path.join(self.repo, "tools/privacy_guard.py"), "w", encoding="utf-8") as f:
            f.write("raise SystemExit(0)\n")
        self.write("c.md", PARAGRAPH)
        self.git("add", "tools/privacy_guard.py", "c.md")
        self.git("commit", "-q", "--no-verify", "-m", "stub")
        self.assertNotEqual(self.git("push", "-q", "--no-verify", "origin", "main", check=False).returncode, 0)


if __name__ == "__main__":
    unittest.main()
