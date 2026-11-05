#!/usr/bin/env -S python3 -I
r"""shell.py --engine PATH: what goes to the system's shell and what comes back, and what a document's source can hold, each against what that system's own pdfTeX does in the same directory.

  \write18      -shell-escape, -shell-restricted, -no-shell-escape; a quoted program and quoted arguments, a command that fails, a name with an accent in it
  pipes         \input|"...", \openin|"...", \openout|"..."
  an editor     e at an error's prompt, by TEXEDIT: calledit finds the editor itself on Windows, and refuses some names
  line endings  CRLF sources, CR alone, a last line with none, in a main file, an \input file, \read and \verb
  backslashes   a document's own \input by a name with a backslash, \openin, \openout, \pdffilesize and the command line's: on Windows a separator, and elsewhere a character

Compared: the exit status, the log, every file the run leaves, a PDF by tests/integration/run.py's comparison.  Not the terminal: a command's output and TeX's interleave by when each flushes.
"""
import argparse
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c  # noqa: E402

CAT = "type" if c.WINDOWS else "cat"
# A character \ of catcode 12, in a macro.
BS = "\\begingroup\\catcode`\\|=0 |catcode`|\\=12 |gdef|bs{\\}|endgroup\n"
END = "\\bye\n"
SUB = {"sub/in.tex": "\\message{[in sub]}\n", "sub/data.txt": "abc\n"}
CASES = []


def case(name, files, args=(), program="pdftex", stdin=None, env=None, main="doc.tex"):
    CASES.append({"name": name, "files": files, "args": list(args), "program": program, "stdin": stdin, "env": env or {}, "main": main})


def doc(text):
    return {"doc.tex": text + END}


SE = ["-shell-escape"]
case("write18/plain", doc("\\immediate\\write18{echo hello>out.txt}\n"), SE)
case("write18/restricted", doc("\\immediate\\write18{echo hello>out.txt}\n\\immediate\\write18{kpsewhich --version}\n"))
case("write18/restricted_by_option", doc("\\immediate\\write18{echo hello>out.txt}\n\\immediate\\write18{kpsewhich plain.tex}\n"), ["-shell-restricted"])
case("write18/off", doc("\\immediate\\write18{echo hello>out.txt}\n\\message{[\\the\\pdfshellescape]}\n"), ["-no-shell-escape"])
case("write18/failing", doc("\\immediate\\write18{exit 3}\n\\immediate\\write18{nosuchprogramanywhere}\n"), SE)
case("write18/quotes", doc('\\immediate\\write18{kpsewhich "plain.tex">quoted.txt}\n\\immediate\\write18{kpsewhich \'plain.tex\'>single.txt}\n'), SE)
case("write18/a_program_by_a_quoted_name", doc(f'\\immediate\\write18{{"kpsewhich" "plain.tex" "story.tex">both.txt}}\n'), SE)
case("write18/accent", doc("\\immediate\\write18{echo caf\u00e9>caf\u00e9.txt}\n"), SE + ["-8bit"])
case("write18/shipped", doc("\\pdfoutput=1 \\font\\f=cmr10 \\f \\write18{echo at the page>page.txt}\nhello\n\\vfill\\eject\n"), SE)
case("write18/output_directory", doc("\\immediate\\write18{kpsewhich -var-value=TEXMF_OUTPUT_DIRECTORY>here.txt}\n\\immediate\\openout3=o.txt \\immediate\\write3{x}\\immediate\\closeout3\n"), SE + ["-output-directory=o"])
case("pipe/input", {"doc.tex": f'\\input|"{CAT} in.tex"\n\\input|"{CAT} in.tex" \n' + END, "in.tex": "\\message{[piped]}\n"}, SE)
case("pipe/input_restricted", {"doc.tex": '\\input|"kpsewhich --version"\n' + END}, ["-shell-restricted"])
case("pipe/input_off", {"doc.tex": f'\\input|"{CAT} in.tex"\n' + END, "in.tex": "\\message{[piped]}\n"}, ["-no-shell-escape"])
case("pipe/openin", {"doc.tex": f'\\openin3=|"{CAT} in.txt"\n\\read3 to\\x \\message{{[\\x]}}\n\\read3 to\\y \\message{{[\\y]}}\n\\closein3\n' + END, "in.txt": "first line\nsecond line\n"}, SE)
case("pipe/openout", doc('\\immediate\\openout3=|"sort>sorted.txt"\n\\immediate\\write3{b}\\immediate\\write3{a}\\immediate\\closeout3\n'), SE)
case("pipe/nothing", doc('\\openin3=|"nosuchprogramanywhere"\n\\message{[\\ifeof3 no input\\else input\\fi]}\n'), SE)
# An editor.  The script says what it was given, and the status is 1 whatever it does.
EDITOR_FILE = {"myeditor.bat": "@echo editor %*>edited.txt\n", "myeditor": "#!/bin/sh\necho editor \"$@\">edited.txt\n"}
ASK = "\\undefinedname\n"
case("edit/found_on_the_path", dict(doc(ASK), **EDITOR_FILE), stdin=b"e\n", env={"TEXEDIT": "myeditor +%d \"%s\"", "PATH": "@WORK@" + os.pathsep + "@PATH@"})
case("edit/not_found", doc(ASK), stdin=b"e\n", env={"TEXEDIT": "nosucheditoranywhere +%d %s"})
case("edit/refused_dot", dict(doc(ASK), **EDITOR_FILE), stdin=b"e\n", env={"TEXEDIT": "./myeditor +%d %s"})
# On Windows C has no end for a name with nothing after it, and asks for what lies beyond it in memory: "I cannot find myeditorDF%i^^fe^^7f in the PATH."  That is no answer to hold rstex to.
if not c.WINDOWS:
    case("edit/one_word", dict(doc(ASK), **EDITOR_FILE), stdin=b"e\n", env={"TEXEDIT": "myeditor", "PATH": "@WORK@" + os.pathsep + "@PATH@"})
# Line endings.
BODY = "\\pdfoutput=1 \\font\\f=cmr10 \\f\n% a comment\nA line with a trailing space \nand another.\n\n\\input part\n\\openin3=part.tex \\read3 to\\x \\message{[\\x]}\\closein3\nThe end.\n\\bye\n"
PART = "\\message{[a part]}\nWords from the part.\n\n"
for name, nl in (("crlf", "\r\n"), ("cr", "\r"), ("lf_then_crlf", None)):
    if nl is None:
        case(f"lines/{name}", {"doc.tex": BODY.replace("\n", "\r\n", 3), "part.tex": PART.replace("\n", "\r\n", 1)})
    else:
        case(f"lines/{name}", {"doc.tex": BODY.replace("\n", nl), "part.tex": PART.replace("\n", nl)})
case("lines/no_last_newline", {"doc.tex": BODY[:-1], "part.tex": PART[:-1]})
case("lines/crlf_latex", {"doc.tex": "\\documentclass{article}\\begin{document}\nA \\verb|verb| and\n\\begin{verbatim}\nverbatim  \n\\end{verbatim}\n\\input{part}\n\\end{document}\n".replace("\n", "\r\n"), "part.tex": "Some words.\r\n"}, program="pdflatex")
case("lines/ctrl_z_at_the_end", {"doc.tex": BODY + "\x1a", "part.tex": PART + "\x1a"})
# Backslashes.
case("backslash/input", dict(SUB, **{"doc.tex": BS + "\\input sub\\bs in\n\\input sub\\bs in.tex\n" + END}))
case("backslash/input_latex", dict(SUB, **{"doc.tex": BS + "\\documentclass{article}\\begin{document}\\input{sub\\bs in}\\IfFileExists{sub\\bs data.txt}{\\message{[there]}}{\\message{[not there]}}\\end{document}\n"}), program="pdflatex")
case("backslash/openin", dict(SUB, **{"doc.tex": BS + "\\openin3=sub\\bs data.txt \\message{[\\ifeof3 no\\else yes\\fi]}\\closein3\n" + END}))
case("backslash/openout", dict(SUB, **{"doc.tex": BS + "\\immediate\\openout3=sub\\bs out.txt \\immediate\\write3{x}\\immediate\\closeout3\n" + END}))
case("backslash/file_primitives", dict(SUB, **{"doc.tex": BS + "\\message{[\\pdffilesize{sub\\bs data.txt}][\\pdfmdfivesum file{sub\\bs data.txt}][\\pdffilemoddate{nosuch\\bs file}]}\n" + END}))
case("backslash/jobname", dict(SUB, **{"sub/doc.tex": "\\message{[\\jobname]}\n" + END}), main="sub\\doc.tex")
case("backslash/dot", dict(SUB, **{"doc.tex": "\\message{[\\jobname]}\n" + END}), main=".\\doc.tex")
case("backslash/output_directory", dict(SUB, **{"doc.tex": "\\message{[\\jobname]}\n" + END}), ["-output-directory=sub\\o"])
case("slash/input_with_dots", dict(SUB, **{"doc.tex": "\\input ./sub/../sub/in\n\\input sub//in\n" + END}))


def within(env_value, work):
    return env_value.replace("@WORK@", work).replace("@PATH@", os.environ.get("PATH", ""))


def one(engine, case_, root):
    name = case_["name"]
    work = os.path.join(root, re.sub(r"\W", "_", name))
    results = []
    for prog, extra in ((c.tool(case_["program"]), []), (engine, ["-progname=" + case_["program"]])):
        c.fresh(work)
        os.makedirs(os.path.join(work, "sub", "o"), exist_ok=True)
        os.makedirs(os.path.join(work, "o"), exist_ok=True)
        for n, t in case_["files"].items():
            c.write(os.path.join(work, n), t.encode("utf-8"))
            if n == "myeditor" and not c.WINDOWS:
                os.chmod(os.path.join(work, n), 0o755)
        before = set(c.state(work))
        env = c.base_env(os.path.join(root, "cache"))
        env.update({k: within(v, work) for k, v in case_["env"].items()})
        r = c.run([prog, *extra, "-interaction=nonstopmode" if case_["stdin"] is None else "-interaction=errorstopmode", *case_["args"], case_["main"]], work, env, stdin=case_["stdin"] if case_["stdin"] is not None else None)
        results.append((r, {k: v for k, v in c.state(work).items() if k not in before}))
    (o, of), (r, rf) = results
    problems = []
    if o[0] != r[0]:
        problems.append(f"status {o[0]} and {r[0]}: {r[2][-200:]!r} {r[1][-200:]!r}")
    if sorted(of) != sorted(rf):
        problems.append(f"files: only pdfTeX left {sorted(set(of) - set(rf))}, only rstex {sorted(set(rf) - set(of))}")
    m = c.integration()
    for k in sorted(set(of) & set(rf)):
        if k.endswith(".log"):
            d = c.against_the_oracle(of[k], rf[k])
            if d:
                problems.append(f"{k}: {d}")
        elif k.endswith(".pdf"):
            a, b = os.path.join(root, "a.pdf"), os.path.join(root, "b.pdf")
            c.write(a, of[k])
            c.write(b, rf[k])
            d = m.pdf_diff(k, a, b, False)
            if d:
                problems.append(f"{k}: {d.a!r} and {d.b!r}")
        elif of[k] != rf[k]:
            problems.append(f"{k}: {of[k][:200]!r} and {rf[k][:200]!r}")
    # What rstex says of a format it has to build, since an environment that says something of the tree is another format.
    said = re.sub(rb"(?m)^rstex: building the format \w+\r?\n", b"", r[2])
    if case_["stdin"] is not None and o[2].replace(b"\r\n", b"\n") != said:
        problems.append(f"standard error: {o[2][-200:]!r} and {said[-200:]!r}")
    c.say(name, not problems, "; ".join(problems) or f"status {r[0]}, {sorted(rf)}")


def main():
    c.start()
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--engine", required=True)
    ap.add_argument("--label", default="here")
    ap.add_argument("--only")
    a = ap.parse_args()
    engine = c.program(a.engine)
    root = c.fresh(os.path.join(c.OUT, a.label, "shell"))
    # The formats are not what any case is about: what a run says on building one would be compared.
    for name, text in (("pdftex", "\\end\n"), ("pdflatex", "\\documentclass{article}\\begin{document}\\end{document}\n")):
        d = c.fresh(os.path.join(root, "formats"))
        c.write(os.path.join(d, "doc.tex"), text)
        c.run([engine, "-progname=" + name, "-interaction=nonstopmode", "doc.tex"], d, c.base_env(os.path.join(root, "cache")))
    for x in CASES:
        if not a.only or re.search(a.only, x["name"]):
            c.guarded(x["name"], one, engine, x, root)
    return c.finish("the shell, line endings and backslashes")


if __name__ == "__main__":
    sys.exit(main())
