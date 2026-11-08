#!/usr/bin/env -S python3 -I
"""run.py: run the documents under tests/integration on the oracle and on a candidate engine; say per case if they agree.

    run.py                                 candidate is ${CARGO_TARGET_DIR:-target}/debug/rstex
    run.py --engine PATH                   candidate is PATH; plain and latex cases use formats it builds itself
    run.py --engine /usr/bin/pdftex --installed-formats     the oracle against itself: the gate for this tool

    --only MODULE     one module (repeatable)      --filter REGEX    cases whose name matches
    --failed          rerun the failures of the last run            --list   show the cases and stop
    --first-diff NAME run one case and print a longer context       --json FILE   write all results
    -j N              parallel cases, default CPUs minus 2          --timeout S   per step, default 20
    --keep            keep the raw results of cases that agree too, under candidate/
    --top N           how many of the most common first differences to show, default 10
    --differences DIR writes DIR/<case>.txt for each case that differs, all it differs in: for reading, on a system that cannot be reached from here

Everything this tool writes is under target/integration/, never in the tree, because logs are engine output:
    fmt/<engine hash>/   the candidate's own pdftex.fmt and pdflatex.fmt, built once per engine binary
    oracle/<key>/        cached oracle results, keyed by the case's input files, its recipe and the oracle's identity
    candidate/<case>/    raw results of a failing case
    work/<case>/         the scratch tree; the same absolute path for both engines, one after the other
    master/              the shared copy of tests/integration that the scratch trees link to, made per run
    shared/              the one PDF that the oracle makes for image/pair_image
    last.json            the last run, for --failed

A case is tests/integration/<module>/*.tex whose first line is "% mode: ...", plus the table PAIRS (a \\dump, then a run
in that format) and the table COMMAND_LINES (what one line of "% mode:" cannot say).  Each run gets a fresh scratch tree,
with every file's mtime fixed, stdin /dev/null unless a command line says otherwise, and the environment ENVIRONMENT.  The
scratch tree is the equivalent of a copy: its top level is fresh, which is where TeX writes, and each module is a symbolic link
to one shared copy made per run, because 700 small files per case took most of a run's time.  A case that writes inside a module
directory would change that copy for the others, so that is checked after every case and reported as kind "tree".

Compared per step, in this order: crash or timeout; every *.log, masked; stdout and stderr, masked; exit status; *.dvi
byte for byte; *.pdf through the comparison of tests/harness/compare_pdf.py without its raster pass, then object by object
on qpdf's decoded view; *.fmt only for existing, because a format is the engine's own memory image; any other new file byte for byte.
A timeout or a signal on the candidate is reported as such.  Exit status: 0 all agree or only accepted ones differ, 1 some
differ, 2 could not run.

known_divergences.txt holds the accepted differences, one per line: the case, how it differs, then the reason.  Such a case that differs in just that way is
listed apart, with its first difference and its reason, and does not count: not in "agree/cases" and not in the exit status.  If it differs in any other way, or
not at all, it fails, and so does a crash, a timeout or anything the tool itself finds.  The file's header has the forms and list_verdict() the rules.

Under RSTEX_ORACLE_DIR the oracle is another build of pdfTeX, and second_oracle.txt takes the place of known_divergences.txt, whose reasons are about Arch's.  It has the same form.
A build for macOS or Windows comes from another compiler, whose wrap and uninitialised reads may be its own: second_oracle_macos.txt or second_oracle_windows.txt, if there is one, then says what differs from second_oracle.txt
on that system, line by line, and each of its lines has been read.  A line there replaces the case's line in second_oracle.txt, and "agrees" takes it out.

On Windows the tool goes by what is there: no limit on a file's size, a kill by taskkill, what a crash ends with as the signal it stands for, and the oracle's standard output and standard error have their CRLF taken out,
which is the one thing DESIGN section 12 has differ there.

Nothing that follows from the date is masked: FORCE_SOURCE_DATE=1 makes it equal, and a difference there is a real one.
"""
import argparse
import base64
import collections
import hashlib
import json
import multiprocessing
import os
import re
import shutil
import signal
import struct
import subprocess
import sys
import time
import zlib

try:
    import resource
except ImportError:
    resource = None  # Windows

sys.dont_write_bytecode = True  # importing compare_pdf.py writes nothing in the tree

WINDOWS = os.name == "nt"
EXE = ".exe" if WINDOWS else ""

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
TREE = os.path.join(ROOT, "tests", "integration")
# RSTEX_INTEGRATION_OUT: worktrees share target/, and two runs in one directory take its scratch trees from each other.
OUT = os.environ.get("RSTEX_INTEGRATION_OUT") or os.path.join(ROOT, "target", "integration")
COMPARE_PDF = os.path.join(ROOT, "tests", "harness", "compare_pdf.py")
# RSTEX_ORACLE_DIR: where a TeX Live of its own has its binaries, as in CI.
ORACLE_DIR = os.environ.get("RSTEX_ORACLE_DIR") or "/usr/bin"
SECOND_ORACLE = bool(os.environ.get("RSTEX_ORACLE_DIR"))
MODES = ("ini", "ini-etex", "plain", "latex")
KNOWN_DIVERGENCES = os.path.join(TREE, "known_divergences.txt")
SECOND_ORACLE_LIST = os.path.join(TREE, "second_oracle.txt")
# What a build of another system differs in from the list above; see the top of this file.  The tool's own files, which are no part of what a case reads.
SYSTEM_OVERLAY = {"win32": "second_oracle_windows.txt", "darwin": "second_oracle_macos.txt"}.get(sys.platform)
TOOL_FILES = ("run.py", "README.md", "known_divergences.txt", "second_oracle.txt", "second_oracle_windows.txt", "second_oracle_macos.txt")
NEVER_ACCEPTED = {"tool", "tree", "oracle", "format", "mode", "crash", "timeout", "second"}

# PATH and HOME come from the caller, and nothing else that can change a result.  The MKTEX* are there because the port has no mktex* but mktexpk (PORTING
# section 19): with them at texmf.cnf's values the oracle, on a case that names a missing font or format, runs mktextfm or mktexfmt,
# which starts METAFONT, prints on stderr and writes missfont.log.  Checked on the oracle with \font\x=nosuchfontatall.  A case can say
# otherwise on its second line, "% env: NAME=VALUE ...", with @WORK@ for its scratch tree and @PATH@ for the PATH: so a case of mktexpk
# keeps its bitmaps in a TEXMFVAR of its own, which each engine has fresh, and they are compared like any other new file.
ENVIRONMENT = {
    "SOURCE_DATE_EPOCH": "1700000000",
    "FORCE_SOURCE_DATE": "1",
    "TZ": "UTC",
    "LC_ALL": "C",
    "MKTEXFMT": "0",
    "MKTEXTFM": "0",
    "MKTEXTEX": "0",
    "MKTEXPK": "0",
}
FIXED_MTIME = 1700000000
CACHE_VERSION = 4               # bump when how an engine is run changes: the oracle's results are cached
MAX_PRINT_LINE = 79             # texmf.cnf; a physical log line of exactly this length was wrapped by TeX
OUTPUT_LIMIT = 200 << 20        # RLIMIT_FSIZE: a runaway candidate stops with SIGXFSZ, reported as a crash
DEVICE_LIMIT = 4 << 30          # RLIMIT_AS of both engines in a case of "% setup: device": one that reads a device as a whole file would take all the machine's memory
ORACLE_TIMEOUT = 120            # seconds per step: the oracle is known to end, so only a loaded machine can stop it
CUT = 150                       # a report line shows this much of each side
VERSION_STRINGS = r"\((?:TeX Live 2026(?:/[^)]*)?|rstex)\)"
# kpathsea on Windows has the base name of the program's long name here, ".exe" and all.
INVOCATION = r"(?:rstex|pdftex|pdflatex)(?:\.exe)?"
# What a name by a whole path begins with.
WHOLE_PATH = r"(?:[A-Za-z]:)?[/\\]\S*[/\\]" if WINDOWS else r"/\S*/"

# ----------------------------------------------------------------------------------------------------------------------
# The masks.  A mask that hides a real difference is the worst bug this tool can have, so each is one line shape, says why
# it exists, and leaves everything else on the line alone.  Every line is compared exactly unless it matches one of these.
# The numbers a mask keeps are real comparisons: for example the first number of "N strings of total length" is masked but
# "N multiletter control sequences" is not, because that one is the same by construction.
#
# Log wrapping: TeX wraps at MAX_PRINT_LINE columns, and a masked item of another length moves the wrap.  So a line matching
# REJOIN, and only such a line, is joined with its continuation lines before the masks run.  Every other line is compared
# as it wrapped, because a different wrap there is a real difference.
# ----------------------------------------------------------------------------------------------------------------------
REJOIN = re.compile(r"^(?:Output written on |pdfTeX warning: (?:" + WHOLE_PATH + r")?" + INVOCATION + r"\b|!pdfTeX error: (?:" + WHOLE_PATH + r")?" + INVOCATION + r"\b)")
# A blank line after a full line is part of it: TeX ended the line by wrapping, then print_ln made an empty one, and a text one
# column shorter has neither.  A logical line that is exactly full and ended by print_nl is followed by one of these.
REJOIN_STOP = re.compile(r"^(?:Output written on |PDF statistics:|Transcript written on |pdfTeX warning|!pdfTeX error| ==> Fatal )")

# (name, pattern, replacement, reason).  All log and terminal text goes through all of them.
MASKS = [
    ("version-string", r"(This is pdfTeX, Version 3\.141592653-2\.6-1\.40\.29 )" + VERSION_STRINGS, r"\1(VERSION)",
     "the banner, in the log's first line, the terminal's and \\pdftexbanner: the port says (rstex), TeX Live says its own"),
    ("invocation-warning", r"^pdfTeX warning: " + INVOCATION + r"(?=[: ])", "pdfTeX warning: NAME",
     "kpse_invocation_name: rstex is called rstex, and some sites of the port spell it out"),
    ("invocation-error", r"^!pdfTeX error: " + INVOCATION + r"(?=[: ])", "!pdfTeX error: NAME",
     "the same in pdftex_fail"),
    ("invocation-path", r"^(pdfTeX warning: |!pdfTeX error: )" + WHOLE_PATH + INVOCATION + r"(?=[: ])", r"\1PATH/NAME",
     "kpse_invocation_name is argv[0] as it came, so a program called by a path prints the path, which is its own: not the same as NAME, so that one printing the bare name differs"),
    ("invocation-cli", r"^" + INVOCATION + r"(?=: )", "NAME",
     "the same in what texmfmp.c and getopt say about options: `pdftex: unrecognized option' and the like"),
    ("invocation-try", r"^Try `" + INVOCATION + r" --help'", "Try `NAME --help'",
     "the same, in the line after getopt's complaint"),
    ("recorder-pid", r"^(NAME: \S*/pdftex)\d+(\.fls: )", r"\1PID\2",
     "recorder_start names its file by the process id until the job has a name, which a run cannot share"),
    # sections 1487 and 1489 of \dump, as tests/trip/run_trip.sh masks them.  Both follow from design, not from the document.
    ("dump-strings", r"^\d+ strings of total length \d+$", "N strings of total length M",
     "section 1487 of \\dump: no pool file, so the count and total of strings start far lower (PORTING, Known divergences)"),
    ("dump-memory", r"^\d+ memory locations dumped; (current usage is \d+&\d+)$", r"N memory locations dumped; \1",
     "section 1489 of \\dump: token lists are not in mem (PORTING section 15), so what is dumped of mem is smaller.  The usage is compared: webmem.rs works out WEB's dyn_used"),
    # \\tracingstats.  Of "Here is how much of TeX's memory you used" only these three lines differ by design.  The lines on
    # multiletter control sequences, font info, hyphenation exceptions and stack positions are exact, capacities included,
    # and so is the "PDF statistics" block: nothing is known to differ there.
    ("stats-strings", r"^ \d+ strings? out of \d+$", " N strings out of M",
     "no pool file, so init_str_ptr and what max_strings leaves differ (PORTING, Known divergences)"),
    ("stats-string-chars", r"^ \d+ string characters out of \d+$", " N string characters out of M",
     "the same for pool_ptr and pool_size"),
    ("stats-memory", r"^ \d+ words of memory out of \d+$", " N words of memory out of M",
     "token lists are not in mem, and mem has its capacity whatever main_memory says (main.rs main_body)"),
    # var_used is kept, since nodes are in mem as in WEB, and so is dyn_used, which webmem.rs works out as WEB's.  What is untouched follows from the most cells ever in use, which it only knows near the end of memory.
    ("stats-usage", r"^(Memory usage before: \d+&\d+; after: \d+&\d+); still untouched: \d+$", r"\1",
     "section 639: the untouched size of mem, as above"),
]
MASKS = [(n, re.compile(p), r, why) for n, p, r, why in MASKS]

# Dropped from stderr only: what the oracle's C libraries say on their own, "\\nlibxpdf: ...\\n" and "libpng error: ...\\n".  The port
# has its own image readers (crates/image), and pdfTeX's own message about the same failure, on the terminal and in the log,
# stays compared.  One program's stderr, nothing else, and only these two prefixes.
STDERR_DROP = re.compile(rb"\nlibxpdf: [^\n]*\n|^libpng (?:error|warning): [^\n]*\n", re.M)

# What a case cannot ask on Windows.  kpathsea looks for mktexpk beside the program there and not on the PATH, so that this one's own on its PATH, which is a shell script as well, is never run, and the real one runs on a PATH with a colon in it.
NOT_ON_WINDOWS = {"pdffont/pk_mktexpk_fails"}

# Streams not compared for one case, with its owner's reason.
NOT_COMPARED = {
    # A command's output and TeX's buffered text interleave by when each flushes, which says nothing about TeX.  The case's
    # header: "Its output on the terminal is the command's and not compared; the log has what section 1617 prints".
    "ext/write18": {"stdout"},
}

# Masks that only apply to a step that loads an installed format on the oracle and the candidate's own on the candidate.
FORMAT_MASKS = [
    ("format-ident", r"^(This is pdfTeX, Version \S+ \(VERSION\)) \(preloaded format=\S+(?: \d{4}\.\d+\.\d+)?\)", r"\1 (preloaded format=FMT)",
     "the format's name and build date on the banner: fmtutil made the oracle's on another day; the terminal prints the"
     " name as -fmt gave it, a path here; and the name is the job's, which rstex takes wrongly while it ignores -jobname"),
]
FORMAT_MASKS = [(n, re.compile(p), r, why) for n, p, r, why in FORMAT_MASKS]

# "Output written on NAME (P pages, N bytes)."  N is not masked by a pattern; see output_written().  It stays exact for a
# file with no compressed stream, apart from the length of the banner in /PTEX.Fullbanner, which is the one item the PDF
# holds that differs on purpose: the version string above; and the number after startxref, which moves with it.  With a /FlateDecode stream or an object stream N is not
# comparable, since the port's deflate bytes are not the oracle's (PORTING section 18), so it is masked for such a PDF.
# NAME is as print_file_name gives it: in quotes if it has a space, and a byte that is not printable in ^^ notation, as
# a run with no -8bit or TCX file has the bytes of UTF-8.  logged_name() makes that the name the file system has.
OUTPUT_WRITTEN = re.compile(r'^(Output written on ("?)(.*\.pdf)\2 \(\d+ pages?, )(\d+)( bytes\)\.)$')

# Masks in the PDF comparison: the version string in any string that holds the banner (/PTEX.Fullbanner, which PDF 2.0 spells
# PTEX_Fullbanner and which compare_pdf.py does not know by that name; and a malformed \\pdfinfo can put it under another key).
# compare_pdf.py itself ignores object numbers, dates, subset tags, /Producer, /Creator and /PTEX.Fullbanner.  The object
# comparison after it ignores none of those.  It drops: /ID in the trailer, an MD5 of the time and the path and no fact about
# the document; /Length of a stream; cross-reference streams and those of their keys in the trailer that hold compressed sizes.
XREF_TRAILER_KEYS = {"/ID", "/Length", "/W", "/Index", "/Filter", "/DecodeParms", "/Type"}

# ----------------------------------------------------------------------------------------------------------------------
# What one line of "% mode:" cannot say.  Taken over from the old image/run_pairs.py.
# ----------------------------------------------------------------------------------------------------------------------
PAIRS = [
    ("pair_basic", "image/pair_basic.load"),
    ("pair_etex", "image/pair_etex.load"),
    ("pair_hyph_next", "image/pair_hyph_next.load"),
    ("pair_pdf", "image/pair_pdf.load"),
    ("pair_unicode", "image/pair_unicode.load"),
    ("pair_image", "image/pair_image.load"),
    ("pair_jbig2", "image/pair_jbig2.load"),
    ("pair_fonts", "image/pair_fonts.load"),
    ("pair_fonts_slant", "image/pair_fonts_slant.load"),
    ("pair_outer", "image/pair_outer.load"),
    # \endlinechar=-1 in the format: a file name on the first line would have nothing to end it.
    ("pair_endline", "\\input{image/pair_endline.load}"),
]
# What an INITEX first line needs to say anything.
B = "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\^=7 "
# (name, arguments after the program, standard input, the pair whose format it needs).  Aimed at maininit, section 1517 or
# section 1648, as the name says; the log's first lines and what follows them count.
COMMAND_LINES = [
    ("star_no_spaces", ["-ini", "*\\relax\\end"], "", None),
    # A first argument that is no \ or & is a file name, and one with spaces gets quotes: the star is not first.
    ("first_argument_with_spaces", ["-ini", "*" + B + "\\message{[\\the\\eTeXversion]}\\end"], "", None),
    ("etex_option", ["-ini", "-etex", B + "\\message{[\\the\\eTeXversion]}\\end"], "", None),
    ("no_etex_without_star", ["-ini", B + "\\message{\\ifx\\middle\\undefined undefined\\fi}\\end"], "", None),
    ("first_line_at_prompt", ["-ini", "-interaction=nonstopmode"], B + "\\message{typed at the prompt}\\end\n", None),
    ("star_at_prompt", ["-ini", "-interaction=nonstopmode"], "*" + B + "\\message{[\\the\\eTeXversion]}\\end\n", None),
    ("if_on_line_zero", ["-ini", "-interaction=nonstopmode", "\\iftrue\\end"], "", None),
    ("group_on_line_zero", ["-ini", "-interaction=nonstopmode", "\\begingroup\\end"], "", None),
    ("blank_first_lines", ["-ini", "-interaction=nonstopmode"], "\n   \n\\end\n", None),
    ("eof_at_prompt", ["-ini"], "", None),
    ("mark_on_the_main_file", ["-ini", "-interaction=nonstopmode", "files/bom.inc"], "", None),
    ("jobname", ["-ini", "-jobname=otherjob", B + "\\message{\\jobname}\\end"], "", None),
    ("jobname_quoted", ["-ini", "-jobname=\"other job\"", B + "\\message{\\jobname}\\end"], "", None),
    # "Output written on" a name in quotes, with the bytes the banner moves, and one in ^^ notation, whose byte count is no comparison, as the font is compressed: OUTPUT_WRITTEN.
    ("output_quoted_name", ["-ini", "-interaction=nonstopmode", "-jobname=\"out put\"", B + "\\pdfoutput=1 \\pdfcompresslevel=0 \\font\\f=cmr10 \\f \\shipout\\hbox{x}\\end"], "", None),
    ("output_utf8_name", ["-ini", "-interaction=nonstopmode", "-jobname=über", B + "\\pdfoutput=1 \\pdfcompresslevel=9 \\font\\f=cmr10 \\f \\shipout\\hbox{x}\\end"], "", None),
    ("interaction_batch", ["-ini", "-interaction=batchmode", "\\undefinedone\\end"], "", None),
    ("interaction_unknown", ["-ini", "-interaction=loud", "-interaction=nonstopmode", "\\end"], "", None),
    ("file_line_error", ["-ini", "-file-line-error", "-interaction=nonstopmode", "image/run_pairs_error.inc"], "", None),
    ("no_file_line_error", ["-ini", "-no-file-line-error", "-interaction=nonstopmode", "image/run_pairs_error.inc"], "", None),
    ("halt_on_error", ["-ini", "-halt-on-error", "image/run_pairs_error.inc"], "", None),
    ("eight_bit", ["-ini", "-8bit", B + "\\message{^^80^^ff^^01}\\end"], "", None),
    ("not_eight_bit", ["-ini", B + "\\message{^^80^^ff^^01}\\end"], "", None),
    ("translate_file", ["-ini", "-translate-file=cp227.tcx", B + "\\message{^^80^^ff^^01^^09}\\end"], "", None),
    ("output_format_dvi", ["-ini", "-output-format=dvi", B + "\\pdfoutput=1 \\message{[\\the\\pdfoutput]}\\end"], "", None),
    ("output_format_pdf", ["-ini", "-output-format=pdf", B + "\\message{[\\the\\pdfoutput]}\\end"], "", None),
    ("draftmode", ["-ini", "-draftmode", B + "\\message{[\\the\\pdfdraftmode]}\\end"], "", None),
    ("shell_escape", ["-ini", "-shell-escape", B + "\\message{[\\the\\pdfshellescape]}\\end"], "", None),
    ("shell_restricted", ["-ini", "-shell-restricted", B + "\\message{[\\the\\pdfshellescape]}\\end"], "", None),
    ("no_shell_escape", ["-ini", "-no-shell-escape", B + "\\message{[\\the\\pdfshellescape]}\\end"], "", None),
    ("output_directory", ["-ini", "-output-directory=outdir", B + "\\immediate\\openout3=written \\immediate\\closeout3 \\end"], "", None),
    ("recorder", ["-ini", "-recorder", "-jobname=rec", "\\end"], "", None),
    # kpathsea's xfopen in recorder_start, at the first thing it reads: the job ends there, with nothing on the terminal.
    ("recorder_no_directory", ["-ini", "-recorder", "-output-directory=no-such-directory", "\\end"], "", None),
    # find_input_file looks in the output directory by kpse_readable_file, which records nothing: each of these is one INPUT line of the .fls, and none on the file's own name.
    ("recorder_file_primitives", ["-ini", "-recorder", "-jobname=recf", "-output-directory=.", B + "\\immediate\\openout3=written.txt \\immediate\\write3{abc}\\immediate\\closeout3 "
                                  "\\message{[\\pdffilesize{written.txt}][\\pdfmdfivesum file{written.txt}][\\pdffiledump length 2{written.txt}]}\\end"], "", None),
    # A length whose 2*length wraps in getfiledump's int (buildtoks/filedump_wrap): the file is opened and recorded before the fread that reads nothing.
    ("recorder_filedump_wrap", ["-ini", "-recorder", "-jobname=recw", B + "\\message{[\\pdffiledump length 1073741824{buildtoks/read_data.txt}]}\\end"], "", None),
    ("kpathsea_debug", ["-ini", "-kpathsea-debug=0", "\\end"], "", None),
    ("double_dash", ["--ini", "--interaction", "nonstopmode", "\\end"], "", None),
    ("dash_dash_ends", ["-ini", "--", "\\end"], "", None),
    ("prefix", ["-ini", "-inter=nonstopmode", "\\end"], "", None),
    ("unknown_option", ["-ini", "-nosuchoption", "\\end"], "", None),
    ("progname_initex", ["--progname=initex", B + "\\message{initex}\\end"], "", None),
    ("format_missing", ["-fmt=nosuchformat", "\\end"], "", None),
    ("ampersand_missing", ["-fmt=pair_basic", "&nosuchformat", "\\end"], "", "pair_basic"),
    ("format_not_a_format", ["-ini", "&notaformat", "\\end"], "", None),
    ("format_named_on_first_line", ["-interaction=nonstopmode", "&pair_basic", "\\message{[\\the\\count10]}\\end"], "", "pair_basic"),
    ("format_erases_initex", ["-ini", "-interaction=nonstopmode", "&pair_basic", "\\message{[\\the\\count10]}\\end"], "", "pair_basic"),
    ("first_line_format", ["-parse-first-line", "image/run_pairs_percent.inc"], "", "pair_basic"),
    ("first_line_off", ["-ini", "-no-parse-first-line", "image/run_pairs_percent.inc"], "", None),
    # input_ln stops the job at a line longer than buf_size, with no section 1513: what pdf_flush wrote stays in the file.
    ("stop_after_pages", ["-ini", "-interaction=nonstopmode", "-cnf-line=buf_size=1000", "pdfout/stop_after_pages.inc"], "", None),
    # A line that leaves first at buf_size: section 400 stops only when max_buf_stack comes to buf_size exactly, and a name from first on goes past it.
    ("csname_past_buf_size", ["-ini", "-interaction=nonstopmode", "-cnf-line=buf_size=1000", "capture/csname_past_buf_size.inc"], "", None),
    ("ifcsname_past_buf_size", ["-ini", "-etex", "-interaction=nonstopmode", "-cnf-line=buf_size=1000", "*capture/ifcsname_past_buf_size.inc"], "", None),
    ("csname_to_buf_size", ["-ini", "-interaction=nonstopmode", "-cnf-line=buf_size=1000", "capture/csname_to_buf_size.inc"], "", None),
    # The same line, with an error on it: texmfmp.c's input_line does not read when first is at buf_size, so the prompt finds the terminal at its end, though it holds an answer.
    ("prompt_at_buf_size", ["-ini", "-cnf-line=buf_size=1000", "capture/prompt_at_buf_size.inc"], "\n\n", None),
    # See known_divergences.txt.
    ("stack_end_at_a_file", ["-ini", "-etex", "-interaction=nonstopmode", "-cnf-line=stack_size=200", "*capture/stack_end_at_a_file.inc"], "", None),
    # kpathsea_maketex_option: the command line overrules MKTEXPK in the environment, one way and the other, and says so of a format it does not know.
    ("no_mktex_pk", ["-ini", "-no-mktex=pk", "-interaction=nonstopmode", "pdffont/pk_not_found.tex"], "", None),
    ("mktex_pk", ["-ini", "-mktex=pk", "-interaction=nonstopmode", "pdffont/pk_not_found.tex"], "", None),
    ("mktex_unknown", ["-ini", "-mktex=nosuch", "-interaction=nonstopmode", "pdffont/pk_not_found.tex"], "", None),
    # pdftex_warn and pdftex_fail print kpse_invocation_name, which is argv[0] as it came: these two are run by their whole path, as CLI_WHOLE_ARGV0 says.
    ("warning_by_path", ["-ini", "-interaction=nonstopmode", "pdfout/warning_by_path.inc"], "", None),
    ("error_by_path", ["-ini", "-interaction=nonstopmode", "pdfout/error_by_path.inc"], "", None),
]
CLI_WHOLE_ARGV0 = {"warning_by_path", "error_by_path"}
# The environment of a command line, as "% env:" gives it to a case.
CLI_ENV = {"no_mktex_pk": {"MKTEXPK": "1"},
           "mktex_pk": {"TEXMFVAR": "@WORK@/var", "VARTEXFONTS": "@WORK@/var/fonts"}}
# Images for the pairs that take one: always the oracle's or made here, so both engines read the same files.
IMAGE_SOURCE = "\\pdfoutput=1 \\pdfcompresslevel=0 \\pdfobjcompresslevel=0 \\pdfinfoomitdate=1 \\pdftrailerid{} " \
               "\\pdfpagewidth=30pt \\pdfpageheight=40pt \\shipout\\hbox{\\pdfliteral{0 0 m 10 10 l S}}\\end"
# Type 1 fonts to replace or copy, one of them slanted: pair_fonts.
FONTS_SOURCE = "\\pdfoutput=1 \\pdfcompresslevel=0 \\pdfobjcompresslevel=0 \\pdfinfoomitdate=1 \\pdftrailerid{} " \
               "\\pdfmapline{=cmr9 CMR9 \" .167 SlantFont \" <cmr9.pfb}\\font\\f=cmr10 \\font\\b=cmbx12 \\font\\s=cmr9 " \
               "\\pdfpagewidth=200pt \\pdfpageheight=60pt \\shipout\\vbox{\\hbox{\\f abc}\\hbox{\\b xyz}\\hbox{\\s slant}}\\end"
IMAGE_FILES = ("pair_image_src.pdf", "pair_fonts_src.pdf", "pair_jbig2_src.jb2")
# JBIG2 files that state a size and hold no page: image/wrap_scale and image/wrap_scale_huge.
SIZED_IMAGES = {"wrap_wide.jb2": (100000, 1), "wrap_tall.jb2": (1, 100000), "wrap_square.jb2": (65535, 65535), "wrap_32000.jb2": (32000, 1), "wrap_huge.jb2": (0x80000001, 10)}
# Broken files, made here since a binary file is not committed: each says a size past what memory holds.
# A PDF whose cross-reference stream says /Size 2^27, past what xpdf's greallocn takes, and one with no cross-reference, whose
# repair finds an object numbered past it.
# PNGs of 10^6 by 10^6, libpng's limit, cut short after their header: one in 16-bit gray that pdfTeX reads a row at a time, and
# one in gray and alpha whose SMask it allocates whole first.  A PK font, with its TFM, whose one character is 524272 by 2^31-1
# pixels, and one whose character is 40000 pixels wide, past what pkin.c's shalfword holds; and two whose character is 16 pixels
# wide and 65536 or 65537 tall, which that shalfword makes no row or one, with no runs or one row's.
BROKEN_FILES = {
    "bogus_xref_size.pdf": lambda: pdf_xref_stream(1 << 27),
    "bogus_xref_repair.pdf": lambda: pdf_damaged(134217472),
    "short_gray16.png": lambda: png_short(1000000, 1000000, 16, 0),
    "short_gray_alpha.png": lambda: png_short(1000000, 1000000, 8, 4),
    "bigraster.tfm": lambda: tfm_one_char(),
    "bigraster.600pk": lambda: pk_one_char(7, 524272, 2147483647),
    "widepk.tfm": lambda: tfm_one_char(),
    "widepk.600pk": lambda: pk_one_char(4, 40000, 1, black=True),
    "tallpk.tfm": lambda: tfm_one_char(),
    "tallpk.600pk": lambda: pk_one_char(7, 16, 65536, runs=b""),
    "tallrowpk.tfm": lambda: tfm_one_char(),
    "tallrowpk.600pk": lambda: pk_one_char(7, 16, 65537, black=True, runs=b"\xd3"),
}
# These read an image while dumping, or take another branch when it is there, so their dump alone is not an ordinary case.
IMAGE_PAIRS = ("pair_image", "pair_jbig2", "pair_fonts", "pair_fonts_slant")
# What fmtutil runs, by the last line of the installed logs.
FORMAT_RECIPES = {
    "pdftex": ["-ini", "-jobname=pdftex", "-progname=pdftex", "-translate-file=cp227.tcx", "*pdfetex.ini"],
    "pdflatex": ["-ini", "-jobname=pdflatex", "-progname=pdflatex", "-translate-file=cp227.tcx", "*pdflatex.ini"],
}


class Step:
    """One run of an engine: program name for the oracle, arguments, stdin, whether it is compared."""

    def __init__(self, args, program="pdftex", stdin="", fmt=None, compare=True, label="", env=None, whole_argv0=False):
        self.args, self.program, self.stdin, self.fmt, self.compare, self.label, self.env = args, program, stdin, fmt, compare, label, env
        self.whole_argv0 = whole_argv0

    def recipe(self):
        return [self.args, self.program, self.stdin, self.fmt, self.compare] + ([self.env] if self.env else []) + (["whole_argv0"] if self.whole_argv0 else [])


class Case:
    def __init__(self, name, module, mode, steps, setup=(), reads=()):
        self.name, self.module, self.mode, self.steps, self.setup, self.reads = name, module, mode, steps, list(setup), list(reads)

    def slug(self):
        return re.sub(r"[^A-Za-z0-9_.+-]", "-", self.name)


def die(message):
    sys.stderr.write("run.py: %s\n" % message)
    sys.exit(2)


def is_accepted(r, known):
    return bool(r["diffs"]) and r["name"] in known and not any(d["kind"] in NEVER_ACCEPTED for d in r["diffs"])


def discover():
    """(cases, .tex files without a mode line, modes beyond the four known ones)."""
    cases, no_mode, odd = [], [], collections.Counter()
    modes = {}
    for dirpath, dirs, files in os.walk(TREE):
        dirs[:] = sorted(d for d in dirs if d != "__pycache__")
        rel = os.path.relpath(dirpath, TREE)
        for f in sorted(files):
            if not f.endswith(".tex"):
                continue
            # With a slash on every system: this is the case's name, and what it is given on its command line.
            path = os.path.normpath(os.path.join(rel, f)).replace(os.sep, "/")
            with open(os.path.join(TREE, path), encoding="latin-1") as h:
                first, second = h.readline(), h.readline()
            m = re.match(r"% mode: *(\S+)\s*$", first)
            if not m:
                no_mode.append(path)
                continue
            mode = m.group(1)
            modes[path] = mode
            if mode not in MODES:
                odd[mode] += 1
            module = path.split("/")[0] if "/" in path else "(top)"
            name = path[:-4]
            if WINDOWS and name in NOT_ON_WINDOWS:
                continue
            step = mode_step(mode, path)
            e = re.match(r"% env: *(.*\S)\s*$", second)
            if e:
                step.env = dict(x.split("=", 1) for x in e.group(1).split())
            setup = re.match(r"% setup: (image|device)\s*$", second)
            # No /dev/zero there.
            if WINDOWS and setup and setup.group(1) == "device":
                continue
            cases.append(Case(name, module, mode, [step], [setup.group(1)] if setup else []))
    cases.extend(pair_cases(modes))
    cases.sort(key=lambda c: c.name)
    return cases, no_mode, odd


def mode_step(mode, path):
    if mode == "ini":
        return Step(["-ini", "-interaction=nonstopmode", path])
    if mode == "ini-etex":
        return Step(["-ini", "-etex", "-interaction=nonstopmode", "*" + path])
    if mode == "plain":
        return Step(["-interaction=nonstopmode", path], fmt="pdftex")
    if mode == "latex":
        return Step(["-interaction=nonstopmode", path], program="pdflatex", fmt="pdflatex")
    return Step([path], label="unknown mode " + mode)


def dump_step(pair, modes, compare=False):
    path = "image/" + pair + ".tex"
    step = mode_step(modes[path], path)
    step.compare, step.label = compare, "dump"
    return step


def pair_cases(modes):
    out = []
    for pair, line in PAIRS:
        job = pair + "-run"
        # The dump alone is an ordinary case too, except for IMAGE_PAIRS.
        image = pair in IMAGE_PAIRS
        setup = ["image"] if image else []
        load = Step(["-jobname=" + job, "-interaction=nonstopmode", "&" + pair, line], label="load")
        out.append(Case("image/%s+load" % pair, "image", "pair", [dump_step(pair, modes, compare=image), load], setup,
                        reads=[pair, line]))
    # Section 1507: -interaction wins over the format's; the format named by -fmt, not on the line.
    load = Step(["-jobname=pair_endline-run2", "-interaction=scrollmode", "-fmt=pair_endline", "\\input{image/pair_endline.load}"], label="load")
    out.append(Case("image/fmt_option+load", "image", "pair", [dump_step("pair_endline", modes), load], reads=["pair_endline"]))
    for name, args, stdin, needs in COMMAND_LINES:
        steps = [dump_step(needs, modes)] if needs else []
        steps.append(Step(args, stdin=stdin, label="cli", env=CLI_ENV.get(name), whole_argv0=name in CLI_WHOLE_ARGV0))
        setup = ["outdir"] if name == "output_directory" else ["notaformat"] if name == "format_not_a_format" else []
        out.append(Case("image/cli:" + name, "image", "cli", steps, setup, reads=([needs] if needs else []) + args))
    return out


# ----------------------------------------------------------------------------------------------------------------------
# Identity and cache keys
# ----------------------------------------------------------------------------------------------------------------------
def sha(*chunks):
    h = hashlib.sha256()
    for c in chunks:
        h.update(c if isinstance(c, bytes) else str(c).encode("utf-8", "surrogateescape"))
        h.update(b"\0")
    return h.hexdigest()


def file_sha(path):
    with open(path, "rb") as f:
        return sha(f.read())


def oracle_program(name):
    return os.path.join(ORACLE_DIR, name + EXE)


def installed_format(name):
    r = subprocess.run(["kpsewhich", "-engine=pdftex", name + ".fmt"], capture_output=True, text=True, check=False)
    return r.stdout.strip()


def oracle_identity():
    parts = [file_sha(oracle_program("pdftex"))]
    version = subprocess.run([oracle_program("pdftex"), "--version"], capture_output=True, text=True, check=False)
    parts.append(version.stdout.split("\n")[0])
    for fmt in FORMAT_RECIPES:
        p = installed_format(fmt)
        parts.append(file_sha(p) if p else "none")
    return sha(*parts)


class Inputs:
    """The files a case may read: all data files of the tree, and any case file whose name occurs in what it reads.

    A name built by macros would escape this, so the set is wider than needed: every .inc, .load and data file counts for
    every case.  Case files are only reached by name; check with a -recorder run if a new case builds a name of one.
    """

    def __init__(self):
        self.stems = collections.defaultdict(list)
        self.common = []
        for dirpath, dirs, files in os.walk(TREE):
            dirs[:] = sorted(d for d in dirs if d != "__pycache__")
            for f in sorted(files):
                rel = os.path.relpath(os.path.join(dirpath, f), TREE)
                if f in TOOL_FILES:
                    continue
                if f.endswith(".tex") and rel != "prelude.tex":
                    self.stems[f[:-4]].append(rel)
                else:
                    self.common.append(rel)
        self.common_sha = sha(*[x for rel in self.common for x in (rel, file_sha(os.path.join(TREE, rel)))])

    def closure(self, seeds):
        seen, todo = set(), list(seeds)
        while todo:
            for tok in re.findall(r"[A-Za-z0-9_.+-]+", todo.pop()):
                stem = tok[:-4] if tok.endswith(".tex") else tok
                for rel in self.stems.get(stem, ()):
                    if rel not in seen:
                        seen.add(rel)
                        with open(os.path.join(TREE, rel), encoding="latin-1") as h:
                            todo.append(h.read())
        return sorted(seen)

    def key(self, case, identity, work):
        seeds = list(case.reads) + [" ".join(s.args) for s in case.steps]
        rels = self.closure(seeds)
        hashes = [x for rel in rels for x in (rel, file_sha(os.path.join(TREE, rel)))]
        recipe = json.dumps([s.recipe() for s in case.steps] + list(case.setup))
        if "image" in case.setup:
            # The files this tool makes are not in the tree, and the oracle's result depends on their bytes.
            recipe += sha(*[x for name, data in sorted(made_files().items()) for x in (name, data)])
        return sha(CACHE_VERSION, identity, self.common_sha, recipe, work, json.dumps(ENVIRONMENT, sort_keys=True), *hashes)


# ----------------------------------------------------------------------------------------------------------------------
# Running
# ----------------------------------------------------------------------------------------------------------------------
# What a process on Windows needs to start and to find a shell, and where a cache and a home are: nothing here is read by a case.
WINDOWS_ENVIRONMENT = ("SystemRoot", "SystemDrive", "COMSPEC", "PATHEXT", "TEMP", "TMP", "USERPROFILE", "LOCALAPPDATA", "APPDATA", "ProgramData", "windir") if WINDOWS else ()


def environment():
    env = dict(ENVIRONMENT)
    # RSTEX_DEFLATE_WORKERS too, which may change nothing: the same cases run with workers from the first stream, and the oracle ignores it.
    for k in ("PATH", "HOME", "RSTEX_DEFLATE_WORKERS") + WINDOWS_ENVIRONMENT:
        if k in os.environ:
            env[k] = os.environ[k]
    # A case judges a cold run, and has no business in the user's cache.  Restored runs are judged in tests/corpus/snapshot.  The oracle ignores this.
    env["RSTEX_NO_SNAPSHOT"] = "1"
    # And the literal engine, not the native diagram path: tests/cd/native judges that.
    env["RSTEX_NO_COMMUTE"] = "1"
    return env


def limit_child():
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    resource.setrlimit(resource.RLIMIT_FSIZE, (OUTPUT_LIMIT, OUTPUT_LIMIT))


def limit_device_child():
    limit_child()
    # A system that does not take the limit, or does not hold a process to it, still runs the case.
    try:
        resource.setrlimit(resource.RLIMIT_AS, (DEVICE_LIMIT, DEVICE_LIMIT))
    except (ValueError, OSError):
        pass


# What Windows ends a process with where Unix sends a signal, as the signal.  A status of this size is a crash whatever it is, and what is not here keeps its number.
NT_STATUS_SIGNAL = {0xC0000005: 11, 0xC00000FD: 11, 0xC0000409: 6, 0xC0000094: 8, 0xC0000095: 8, 0xC000001D: 4, 0x80000003: 5}


def status_and_signal(rc):
    """(exit status or None, signal number or None) of a process that has ended."""
    if rc < 0:
        return None, -rc
    if WINDOWS and rc >= 0x80000000:
        return None, NT_STATUS_SIGNAL.get(rc, rc)
    return rc, None


def signal_name(n):
    try:
        return signal.Signals(n).name
    except ValueError:
        return {6: "SIGABRT", 5: "SIGTRAP", 7: "SIGBUS", 10: "SIGBUS"}.get(n, "0x%x" % n)


def kill_tree(p):
    """The process and what it started: a program of \\write18 may be what hangs."""
    if WINDOWS:
        subprocess.run(["taskkill", "/T", "/F", "/PID", str(p.pid)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
        p.kill()
    else:
        try:
            os.killpg(p.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def transient(act, *args):
    """A file system call that, on Windows, can meet a handle that has not gone yet: a process that just ended, or a scanner looking at what it left."""
    for delay in (0.02, 0.05, 0.1, 0.2, 0.5, 1, 2) if WINDOWS else ():
        try:
            return act(*args)
        except PermissionError:
            time.sleep(delay)
    return act(*args)


def spawn(argv, cwd, stdin_bytes, stdout_path, stderr_path, timeout, extra_env=None, whole_argv0=False, device=False):
    """(exit status or None, signal number or None, timed out, seconds).

    argv[0] is the full path of the program; the program sees only its base name, because that is how a user runs it and
    pdfTeX prints argv[0] as it came ("pdfTeX warning: /usr/bin/pdftex: ...").  With whole_argv0 it sees all of it, to check that it does.
    """
    start = time.monotonic()
    env = environment()
    if WINDOWS:
        # mktexpk there makes its files in the temporary directory by a font's name, and two cases at once with one font read each other's.
        env["TEMP"] = env["TMP"] = os.path.join(OUT, "tmp", os.path.basename(cwd))
        os.makedirs(env["TEMP"], exist_ok=True)
    for k, v in (extra_env or {}).items():
        env[k] = v.replace("@WORK@", cwd).replace("@PATH@", os.environ.get("PATH", ""))
    with open(stdout_path, "wb") as out, open(stderr_path, "wb") as err:
        if stdin_bytes:
            stdin_path = stdout_path + ".stdin"
            with open(stdin_path, "wb") as f:
                f.write(stdin_bytes)
            stdin = open(stdin_path, "rb")
        else:
            stdin = open(os.devnull, "rb")
        with stdin:
            try:
                own = {} if WINDOWS else {"start_new_session": True, "preexec_fn": limit_device_child if device else limit_child}
                p = subprocess.Popen([argv[0] if whole_argv0 else os.path.basename(argv[0])] + argv[1:], executable=argv[0], cwd=cwd, env=env,
                                     stdin=stdin, stdout=out, stderr=err, **own)
            except OSError as e:
                err.write(("cannot start: %s\n" % e).encode())
                return 127, None, False, 0.0
            try:
                rc = p.wait(timeout=timeout)
                timed_out = False
            except subprocess.TimeoutExpired:
                kill_tree(p)
                rc = p.wait()
                timed_out = True
    if stdin_bytes:
        transient(os.remove, stdout_path + ".stdin")
    seconds = time.monotonic() - start
    status, sig = status_and_signal(rc)
    return status, sig, timed_out, seconds


def scan(work):
    found = {}
    for dirpath, dirs, files in os.walk(work):
        for f in files:
            p = os.path.join(dirpath, f)
            st = os.lstat(p)
            found[os.path.relpath(p, work).replace(os.sep, "/")] = (st.st_size, st.st_mtime_ns)
    return found


def clean_dir(path):
    """Empties a scratch directory without removing it.  The oracle's kpathsea finds the working directory by walking up and
    stating every entry of each parent, and stops with "NAME: No such file or directory" if one goes away meanwhile, so the
    directories in work/ have to stay while cases run in them side by side."""
    os.makedirs(path, exist_ok=True)
    for entry in os.scandir(path):
        if entry.is_dir(follow_symlinks=False):
            transient(shutil.rmtree, entry.path)
        else:
            transient(os.unlink, entry.path)


def prepare_work(case, work, master, image_src):
    clean_dir(work)
    for entry in os.listdir(master):
        src = os.path.join(master, entry)
        if os.path.isdir(src):
            try:
                os.symlink(src, os.path.join(work, entry), target_is_directory=True)
            except OSError:
                # On Windows a link takes a privilege that a user may not have, and a runner has.
                shutil.copytree(src, os.path.join(work, entry), copy_function=shutil.copy2)
        else:
            shutil.copy2(src, os.path.join(work, entry))
    for item in case.setup:
        if item == "outdir":
            os.makedirs(os.path.join(work, "outdir"))
        elif item == "notaformat":
            shutil.copy2(os.path.join(work, "image", "run_pairs_error.inc"), os.path.join(work, "notaformat.fmt"))
        elif item == "image":
            for name in IMAGE_FILES + tuple(SIZED_IMAGES) + tuple(BROKEN_FILES):
                shutil.copy2(os.path.join(image_src, name), os.path.join(work, name))
        elif item == "device":
            pass  # a limit on the engines' memory, in spawn
        else:
            raise ValueError(item)


def run_case(case, argv_of, result_dir, ctx, timeout):
    """Runs all steps of a case in a fresh scratch copy; leaves stdout, stderr and changed files under result_dir."""
    work = os.path.join(OUT, "work", case.slug())
    prepare_work(case, work, ctx["master"], ctx.get("image_src"))
    tmp = result_dir + ".tmp"
    if os.path.exists(tmp):
        transient(shutil.rmtree, tmp)
    os.makedirs(tmp)
    meta = {"case": case.name, "steps": []}
    before = scan(work)
    for i, step in enumerate(case.steps):
        sdir = os.path.join(tmp, "s%d" % i)
        os.makedirs(sdir)
        argv = argv_of(step)
        rc, sig, timed_out, seconds = spawn(argv, work, step.stdin.encode("latin-1"), os.path.join(sdir, "stdout"),
                                            os.path.join(sdir, "stderr"), timeout, step.env, step.whole_argv0, "device" in case.setup)
        after = scan(work)
        changed = sorted(k for k in after if before.get(k) != after[k])
        files = {}
        for rel in changed:
            files[rel] = after[rel][0]
            # A format is big, the engine's own, and compared only for existing.
            if rel.endswith(".fmt"):
                continue
            dest = os.path.join(sdir, "files", rel)
            os.makedirs(os.path.dirname(dest), exist_ok=True)
            shutil.copy2(os.path.join(work, rel), dest)
        before = after
        meta["steps"].append({"argv": argv, "exit": rc, "signal": sig, "timed_out": timed_out,
                              "seconds": round(seconds, 3), "files": files})
    clean_dir(work)
    with open(os.path.join(tmp, "meta.json"), "w") as f:
        json.dump(meta, f)
    if os.path.exists(result_dir):
        transient(shutil.rmtree, result_dir)
    transient(os.rename, tmp, result_dir)
    return meta


def load_meta(result_dir):
    try:
        with open(os.path.join(result_dir, "meta.json")) as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def build_formats(engine, needed):
    """{format: {path, problem}}: the candidate's own formats, built once per engine binary, as fmtutil builds them."""
    tag = sha(file_sha(engine), json.dumps(FORMAT_RECIPES), json.dumps(ENVIRONMENT, sort_keys=True))[:16]
    base = os.path.join(OUT, "fmt", tag)
    os.makedirs(base, exist_ok=True)
    os.utime(base)
    out = {}
    for name in needed:
        d = os.path.join(base, name)
        marker = os.path.join(d, "result.json")
        if not os.path.exists(marker):
            shutil.rmtree(d, ignore_errors=True)
            os.makedirs(d)
            rc, sig, timed_out, seconds = spawn([engine] + FORMAT_RECIPES[name], d, b"", os.path.join(d, "stdout"), os.path.join(d, "stderr"), 600)
            info = {"exit": rc, "signal": sig, "timed_out": timed_out, "seconds": round(seconds, 2)}
            fmts = [f for f in os.listdir(d) if f.endswith(".fmt")]
            info["fmts"] = fmts
            with open(marker, "w") as f:
                json.dump(info, f)
        with open(marker) as f:
            info = json.load(f)
        out[name] = format_report(name, d, info)
    # Old engines' formats are 14 MB each: keep the three most recent.
    root = os.path.join(OUT, "fmt")
    older = sorted(os.listdir(root), key=lambda x: os.path.getmtime(os.path.join(root, x)), reverse=True)[3:]
    for x in older:
        shutil.rmtree(os.path.join(root, x), ignore_errors=True)
    return out


def log_inputs(path):
    """The files a log says it read, in order."""
    with open(path, encoding="latin-1") as f:
        return re.findall(r"\(((?:[A-Za-z]:)?/[^\s()]+)", f.read())


def format_report(name, d, info):
    """Says whether a format is usable and whether it was built from what the installed one was built from."""
    notes = []
    fmts = info["fmts"]
    if name + ".fmt" in fmts:
        path = os.path.join(d, name)
        logname = name + ".log"
    elif len(fmts) == 1:
        path = os.path.join(d, fmts[0][:-4])
        logname = fmts[0][:-4] + ".log"
        notes.append("wrote %s, not %s.fmt: the engine ignored -jobname=%s; using it" % (fmts[0], name, name))
    else:
        return {"path": None, "problem": "no format: exit %s, signal %s, timed out %s, files %s" % (info["exit"], info["signal"], info["timed_out"], fmts),
                "notes": notes}
    if info["exit"] != 0:
        notes.append("exit status %s while building it" % info["exit"])
    installed = installed_format(name)
    if not installed:
        notes.append("no installed format to check against")
        return {"path": path, "problem": None, "notes": notes}
    log = os.path.join(d, logname)
    installed_log = installed[:-4] + ".log"
    if not os.path.exists(log) or not os.path.exists(installed_log):
        notes.append("no log to check its inputs: %s, %s" % (log, installed_log))
        return {"path": path, "problem": None, "notes": notes}
    ours, theirs = log_inputs(log), log_inputs(installed_log)
    if ours == theirs:
        notes.append("same %d input files as the installed format" % len(ours))
        newer = [x for x in ours if os.path.exists(x) and os.path.getmtime(x) > os.path.getmtime(installed)]
        if newer:
            notes.append("INPUTS NEWER than the installed format: %d, first %s" % (len(newer), newer[0]))
    else:
        i = next((i for i, (x, y) in enumerate(zip(ours, theirs)) if x != y), min(len(ours), len(theirs)))
        notes.append("INPUTS DIFFER from the installed format at %d: %s, %s" % (i, ours[i:i + 1], theirs[i:i + 1]))
    with open(installed_log, encoding="latin-1") as f:
        last = f.read().strip().split("\n")[-1]
    said = last.lstrip("# ").split()[1:]
    if said[-1:] in (["</dev/null"], ["<nul"]):
        said = said[:-1]
    if said != FORMAT_RECIPES[name]:
        notes.append("RECIPE DIFFERS from the installed log's last line: " + last)
    return {"path": path, "problem": None, "notes": notes}


def jbig2_with_globals():
    """Two pages and two page 0 segments, each page using one: what flushjbig2page0objects writes."""

    def seg(num, ty, refs, page, data=b""):
        return struct.pack(">IBB", num, ty, len(refs) << 5) + bytes(refs) + struct.pack(">BI", page, len(data)) + data

    def page_info(w, h, xres, yres):
        return struct.pack(">IIIIBH", w, h, xres, yres, 0, 0)

    def region(n):
        return bytes([n]) * (18 + n)

    out = b"\x97JB2\r\n\x1a\n\x01" + struct.pack(">I", 2)
    out += seg(0, 0, [], 0, region(5)) + seg(1, 0, [], 0, region(6))
    out += seg(2, 48, [], 1, page_info(20, 10, 0, 0)) + seg(3, 6, [0], 1, region(7)) + seg(4, 49, [], 1)
    out += seg(5, 48, [], 2, page_info(30, 15, 3937, 7874)) + seg(6, 6, [1], 2, region(8)) + seg(7, 49, [], 2)
    return out + seg(8, 51, [], 0)


def jbig2_of_size(w, h):
    """One page, with only the page information segment: the size is all that is read of it."""
    d = struct.pack(">IIIIBH", w, h, 0, 0, 0, 0)
    s = b"\x97JB2\r\n\x1a\n" + b"\x01" + struct.pack(">I", 1)
    s += struct.pack(">IBBBI", 0, 48, 0, 1, len(d)) + d
    s += struct.pack(">IBBBI", 1, 49, 0, 1, 0)
    return s + struct.pack(">IBBBI", 2, 51, 0, 0, 0)


def pdf_one_page(tail):
    """A PDF of one page in four objects, then tail(offsets, end): its cross-reference and trailer."""
    objs = [b"<< /Type /Catalog /Pages 2 0 R >>", b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 60] >>",
            b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>", b"<< /Length 25 >>\nstream\n0 0 1 rg 10 10 80 40 re f\nendstream"]
    s, offsets = b"%PDF-1.5\n", []
    for i, o in enumerate(objs, 1):
        offsets.append(len(s))
        s += b"%d 0 obj\n%s\nendobj\n" % (i, o)
    return s + tail(offsets, len(s))


def pdf_xref_stream(size):
    """The cross-reference a stream, object 5, that says /Size SIZE and has the first six entries."""
    def tail(offsets, end):
        data = struct.pack(">BIH", 0, 0, 0xffff) + b"".join(struct.pack(">BIH", 1, o, 0) for o in offsets + [end])
        return (b"5 0 obj\n<< /Type /XRef /Size %d /Index [0 6] /Root 1 0 R /W [1 4 2] /Length %d >>\nstream\n%s\nendstream\nendobj\n"
                b"startxref\n%d\n%%%%EOF\n" % (size, len(data), data, end))
    return pdf_one_page(tail)


def pdf_damaged(num):
    """No cross-reference and no startxref, so xpdf scans the file for objects, and one of them is numbered NUM."""
    return pdf_one_page(lambda offsets, end: b"%d 0 obj\n<< >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n" % num)


def png_short(width, height, depth, color):
    """A PNG whose header says WIDTH by HEIGHT, DEPTH bits, color type COLOR, and whose one IDAT holds eight bytes: cut short."""
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    ihdr = struct.pack(">IIBBBBB", width, height, depth, color, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(b"\0" * 8, 9)) + chunk(b"IEND", b"")


def tfm_one_char():
    """A TFM of 10pt with one character, A, 5pt wide: for the PK files here."""
    lf, lh, bc, ec, nw, nh, nd, ni = 14, 2, 65, 65, 2, 1, 1, 1
    return (struct.pack(">12H", lf, lh, bc, ec, nw, nh, nd, ni, 0, 0, 0, 0) + struct.pack(">ii", 0, 10 << 20)
            + bytes([1, 0, 0, 0]) + struct.pack(">5i", 0, 1 << 19, 0, 0, 0))


def pk_one_char(form, width, height, black=False, runs=b"\x11" * 8):
    """A PK font at 600 dpi of one character, A, WIDTH by HEIGHT pixels, dyn_f 13, the first run white or BLACK, then the bytes of
    RUNS, eight by default, and the end: in the long preamble (FORM 7) or the extended short one (FORM 4).  unpack does not skip to
    the end of a packet, so runs it does not read are read as the next command."""
    flag = 13 << 4 | (8 if black else 0) | form
    ppp = round(600 / 72.27 * 65536)
    if form == 7:
        char = struct.pack(">9i", 32 + len(runs), 65, 0, 0, 0, width, height, 0, 0) + runs
    else:
        body = bytes([65]) + bytes(3) + struct.pack(">HHHhh", 10, width & 0xffff, height, 0, 0) + runs
        char = struct.pack(">H", len(body)) + body
    return bytes([247, 89, 0]) + struct.pack(">iiii", 10 << 20, 0, ppp, ppp) + bytes([flag]) + char + bytes([245, 246, 246, 246])


def made_files():
    """What the setup "image" puts in the scratch tree that is made here, by name: SIZED_IMAGES and BROKEN_FILES."""
    out = {name: jbig2_of_size(*size) for name, size in SIZED_IMAGES.items()}
    out.update((name, make()) for name, make in BROKEN_FILES.items())
    return out


def build_image_source(identity):
    """The directory holding IMAGE_FILES, made once: the two PDFs by the oracle."""
    d = os.path.join(OUT, "shared", sha(identity, IMAGE_SOURCE, FONTS_SOURCE)[:16])
    os.makedirs(d, exist_ok=True)
    for job, source in (("pair_image_src", IMAGE_SOURCE), ("pair_fonts_src", FONTS_SOURCE)):
        if not os.path.exists(os.path.join(d, job + ".pdf")):
            spawn([oracle_program("pdftex"), "-ini", "-jobname=" + job, "-interaction=batchmode", B + source],
                  d, b"", os.path.join(d, job + ".stdout"), os.path.join(d, job + ".stderr"), 30)
    with open(os.path.join(d, "pair_jbig2_src.jb2"), "wb") as f:
        f.write(jbig2_with_globals())
    for name, data in made_files().items():
        with open(os.path.join(d, name), "wb") as f:
            f.write(data)
    for name in IMAGE_FILES:
        if not os.path.exists(os.path.join(d, name)):
            die("no " + name)
    return d


# ----------------------------------------------------------------------------------------------------------------------
# Comparing
# ----------------------------------------------------------------------------------------------------------------------
class Diff:
    """One difference: kind, where, a line number if any, and the two sides."""

    def __init__(self, kind, where, line, a, b, context=None):
        self.kind, self.where, self.line, self.a, self.b, self.context = kind, where, line, a, b, context

    def as_json(self):
        return {"kind": self.kind, "where": self.where, "line": self.line, "oracle": self.a, "candidate": self.b}


def read_bytes(path):
    try:
        with open(path, "rb") as f:
            return f.read()
    except OSError:
        return None


def pdf_facts(path):
    """(has a compressed stream, length of the /PTEX.Fullbanner string and of the number after startxref)."""
    data = read_bytes(path)
    if data is None:
        return None
    # The document's own comes last: an included PDF's /PTEX.InfoDict can hold another.  PDF 2.0 spells it PTEX_Fullbanner.
    found = re.findall(rb"/PTEX[._]Fullbanner \(([^)\n]*(?:\([^)\n]*\)[^)\n]*)*)\)", data)
    # The banner's other length moves the cross-reference table, and at a power of ten that number is one figure longer on one side.
    xref = re.findall(rb"\nstartxref\n(\d+)\n", data)
    return (b"/FlateDecode" in data or b"/ObjStm" in data, (len(found[-1]) if found else 0) + (len(xref[-1]) if xref else 0))


def output_written(line, pdf):
    """Keeps N for a PDF with nothing compressed, less what the banner's length moves; see OUTPUT_WRITTEN."""
    m = OUTPUT_WRITTEN.match(line)
    if not m:
        return line
    if pdf is None:
        return line
    compressed, moved = pdf
    n = "N" if compressed else str(int(m.group(4)) - moved) + "+banner"
    return m.group(1) + n + m.group(5)


def logged_name(name):
    """A file name as a line of a log has it, read as latin-1, to the name that os.listdir gives: ^^ notation undone, and the bytes read as
    UTF-8, which is what TeX Live makes of a name on every system."""
    name = re.sub(r"\^\^([0-9a-f]{2})", lambda m: chr(int(m.group(1), 16)), name)
    try:
        return name.encode("latin-1").decode("utf-8")
    except UnicodeDecodeError:
        return name


def mask_lines(data, pdfs, format_masks, width=MAX_PRINT_LINE):
    """Bytes to masked lines.  pdfs maps a PDF's file name to pdf_facts; see the block comment on masks.  width is the run's max_print_line, which
    tests/token_fuzz.py sets on the command line under --small; the cases here have texmf.cnf's."""
    masks = MASKS + FORMAT_MASKS if format_masks else MASKS
    lines = data.decode("latin-1").split("\n")
    out, i, n = [], 0, len(lines)
    while i < n:
        line = lines[i]
        carried = False
        if REJOIN.match(line):
            while len(lines[i]) == width and i + 1 < n and not REJOIN_STOP.match(lines[i + 1]):
                i += 1
                line += lines[i]
            # A message with a newline of its own, as make_subset_tag's, prints that as a character: TeX's column goes on over it, counting it, so the
            # next line wraps where the masked name and the path put it.  Short paths never get there, a long one does, and pdftex and rstex then wrap one apart.
            carried = lines[i].endswith(": ") and i + 2 < n and len(lines[i]) + 1 + len(lines[i + 1]) == width
        for _, pat, rep, _ in masks:
            line = pat.sub(rep, line)
        if line.startswith("Output written on "):
            m = OUTPUT_WRITTEN.match(line)
            line = output_written(line, pdfs.get(os.path.basename(logged_name(m.group(3)))) if m else None)
        out.append(line)
        i += 1
        if carried:
            out.append(lines[i] + lines[i + 1])
            i += 2
    return out


def first_line_diff(a, b):
    for i, (x, y) in enumerate(zip(a, b)):
        if x != y:
            return i
    return None if len(a) == len(b) else min(len(a), len(b))


def text_diff(kind, where, a, b, want_context):
    """Diff of two lists of lines, or None."""
    i = first_line_diff(a, b)
    if i is None:
        return None
    x = a[i] if i < len(a) else "<end of file, %d lines>" % len(a)
    y = b[i] if i < len(b) else "<end of file, %d lines>" % len(b)
    context = None
    if want_context:
        lo = max(0, i - 12)
        context = {"from": lo + 1, "oracle": a[lo:i + 13], "candidate": b[lo:i + 13]}
    return Diff(kind, where, i + 1, x, y, context)


def oracle_terminal(data):
    """The oracle's standard output or standard error: in text mode on Windows, where this engine's has LF as a redirected stream of pdfTeX has it elsewhere.  DESIGN.md section 12."""
    return data.replace(b"\r\n", b"\n") if WINDOWS else data


def candidate_stderr(data):
    """What a program prints on the candidate's standard error is in that program's text mode, mktexpk's on Windows.  What the candidate itself prints has no CR to take out."""
    return data.replace(b"\r\n", b"\n") if WINDOWS else data


def compare_step_text(o_dir, c_dir, rel, kind, where, o_pdfs, c_pdfs, fm, want_context):
    a = read_bytes(os.path.join(o_dir, rel)) or b""
    b = read_bytes(os.path.join(c_dir, rel)) or b""
    if rel in ("stdout", "stderr"):
        a = oracle_terminal(a)
    if rel == "stderr":
        b = candidate_stderr(b)
    if a == b:
        return None
    if rel == "stderr":
        a, b = STDERR_DROP.sub(b"", a), STDERR_DROP.sub(b"", b)
    return text_diff(kind, where, mask_lines(a, o_pdfs, fm), mask_lines(b, c_pdfs, fm), want_context)


def bytes_diff(kind, where, a, b, want_context):
    """Byte comparison; shows the first differing line if it looks like text."""
    if a == b:
        return None
    if b"\0" not in a[:4096] and b"\0" not in b[:4096]:
        d = text_diff(kind, where, a.decode("latin-1").split("\n"), b.decode("latin-1").split("\n"), want_context)
        if d:
            return d
    k = next((k for k, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
    return Diff(kind, where, None, "offset %d of %d: %s" % (k, len(a), a[k:k + 16].hex(" ")), "offset %d of %d: %s" % (k, len(b), b[k:k + 16].hex(" ")))


def dvitype(path):
    args = ["dvitype", "-output-level=2", "-dpi=72.27", "-page-start=*.*.*.*.*.*.*.*.*.*", path]
    r = subprocess.run(args, capture_output=True, check=False, env=environment())
    # In text mode on Windows, and the same on every system to a hash of it.
    return r.stdout.replace(b"\r\n", b"\n").decode("latin-1").split("\n") if r.returncode == 0 else None


def dvi_diff(where, o_path, c_path, want_context):
    """Byte for byte; the report line comes from dvitype if it can read both."""
    a, b = read_bytes(o_path) or b"", read_bytes(c_path) or b""
    if a == b:
        return None
    x, y = dvitype(o_path), dvitype(c_path)
    if x and y:
        # dvitype's first line names its file.
        d = text_diff("dvi", where, x[1:], y[1:], want_context)
        if d:
            return d
    return bytes_diff("dvi", where, a, b, want_context)


def qpdf_objects(path):
    r = subprocess.run(["qpdf", "--json=2", "--json-key=qpdf", "--json-stream-data=inline", "--decode-level=generalized", path, "-"],
                       capture_output=True, check=False)
    if not r.stdout:
        return None, r.stderr.decode("latin-1").strip()
    try:
        return json.loads(r.stdout)["qpdf"][1], None
    except (ValueError, KeyError, IndexError) as e:
        return None, "qpdf: %s" % e


def canon_trailer(t, xref_stream):
    t = dict(t.get("value", {}))
    for k in XREF_TRAILER_KEYS if xref_stream else ("/ID",):
        t.pop(k, None)
    return t


def canon_value(v):
    """What the banner mask does inside a value."""
    if isinstance(v, str):
        return re.sub(VERSION_STRINGS, "(VERSION)", v) if "This is pdfTeX" in v else v
    if isinstance(v, list):
        return [canon_value(x) for x in v]
    if isinstance(v, dict):
        return {k: canon_value(x) for k, x in v.items()}
    return v


def is_xref_stream(o):
    return "stream" in o and o["stream"]["dict"].get("/Type") == "/XRef"


def value_diff(a, b, path):
    """First path at which two JSON values differ."""
    if type(a) is not type(b):
        return path, a, b
    if isinstance(a, dict):
        for k in sorted(set(a) | set(b)):
            if k not in a or k not in b:
                return path + k, a.get(k, "<none>"), b.get(k, "<none>")
            d = value_diff(a[k], b[k], path + k)
            if d:
                return d
        return None
    if isinstance(a, list):
        for i, (x, y) in enumerate(zip(a, b)):
            d = value_diff(x, y, "%s[%d]" % (path, i))
            if d:
                return d
        return (path + "[len]", len(a), len(b)) if len(a) != len(b) else None
    return None if a == b else (path, a, b)


def object_key(k):
    m = re.match(r"obj:(\d+) ", k)
    return (0, int(m.group(1))) if m else (1, 0)


def objects_diff(where, o_path, c_path, want_context):
    """Object by object on qpdf's decoded view; the port keeps pdfTeX's numbering (PORTING section 18)."""
    a, err_a = qpdf_objects(o_path)
    b, err_b = qpdf_objects(c_path)
    if a is None or b is None:
        return (err_a, err_b)
    xa = any(is_xref_stream(o) for o in a.values() if isinstance(o, dict))
    xb = any(is_xref_stream(o) for o in b.values() if isinstance(o, dict))
    a = {k: o for k, o in a.items() if not is_xref_stream(o)}
    b = {k: o for k, o in b.items() if not is_xref_stream(o)}
    for k in sorted(set(a) | set(b), key=object_key):
        if k not in a or k not in b:
            return Diff("pdf", where, None, "objects: %s %s" % (k, "present" if k in a else "absent"), "objects: %s %s" % (k, "present" if k in b else "absent"))
        x, y = a[k], b[k]
        if k == "trailer":
            d = value_diff(canon_value(canon_trailer(x, xa)), canon_value(canon_trailer(y, xb)), "trailer ")
        elif "stream" in x and "stream" in y:
            dx, dy = dict(x["stream"]["dict"]), dict(y["stream"]["dict"])
            dx.pop("/Length", None)
            dy.pop("/Length", None)
            d = value_diff(dx, dy, k + " ")
            if not d:
                sx, sy = base64.b64decode(x["stream"].get("data", "")), base64.b64decode(y["stream"].get("data", ""))
                if sx != sy:
                    return bytes_diff("pdf", where + ": " + k + " stream", sx, sy, want_context)
        else:
            d = value_diff(canon_value(x), canon_value(y), k + " ")
        if d:
            return Diff("pdf", where, None, "objects: %s %s" % (d[0], json.dumps(d[1])), "objects: %s %s" % (d[0], json.dumps(d[2])))
    return None


def compare_pdf_module():
    """tests/harness/compare_pdf.py, imported so that the values it compares can get the banner mask."""
    if "compare_pdf" not in sys.modules:
        sys.path.insert(0, os.path.dirname(COMPARE_PDF))
        import compare_pdf
        sys.path.pop(0)
        sys.setrecursionlimit(100000)
    return sys.modules["compare_pdf"]


def compare_pdf_tool(o_path, c_path):
    """compare_pdf.py's comparison without its raster pass.  Returns None if same, (oracle's text, candidate's text, context)
    for the first difference, or the string "unreadable" if it cannot walk one of them, which is no statement about them."""
    cp = compare_pdf_module()
    try:
        a, b = cp.Pdf(o_path), cp.Pdf(c_path)
    except (SystemExit, Exception):
        return "unreadable"
    try:
        if len(a.pages) != len(b.pages):
            return "pages: %d" % len(a.pages), "pages: %d" % len(b.pages), None
        for p in range(len(a.pages)):
            x, y = a.page_ops(p), b.page_ops(p)
            k = cp.first_diff(x, y, 0.0)
            if k is not None:
                show = lambda ops: " ".join(ops[k]) if k < len(ops) else "<end of stream>"
                lo = max(0, k - 12)
                near = {"from": lo + 1, "oracle": [" ".join(o) for o in x[lo:k + 13]], "candidate": [" ".join(o) for o in y[lo:k + 13]]}
                return "page %d operator %d: %s" % (p + 1, k, show(x)), "page %d operator %d: %s" % (p + 1, k, show(y)), near
            d = value_diff(canon_value(a.page_meta(p)), canon_value(b.page_meta(p)), "page %d " % (p + 1))
            if d:
                return "%s: %s" % (d[0], json.dumps(d[1])), "%s: %s" % (d[0], json.dumps(d[2])), None
        d = value_diff(canon_value(a.document()), canon_value(b.document()), "document ")
        if d:
            return "%s: %s" % (d[0], json.dumps(d[1])), "%s: %s" % (d[0], json.dumps(d[2])), None
    except (SystemExit, Exception):
        return "unreadable"
    return None


def pdf_diff(where, o_path, c_path, want_context):
    tool = compare_pdf_tool(o_path, c_path)
    if isinstance(tool, tuple):
        return Diff("pdf", where, None, tool[0], tool[1], tool[2] if want_context else None)
    objects = objects_diff(where, o_path, c_path, want_context)
    if isinstance(objects, tuple):
        # qpdf could not read one of them: a PDF the oracle wrote that way is a fact to agree with.
        a, b = read_bytes(o_path), read_bytes(c_path)
        if objects[0] and objects[1]:
            return None if a == b else bytes_diff("pdf", where, a, b, want_context)
        return Diff("pdf", where, None, "qpdf: " + (objects[0] or "reads it"), "qpdf: " + (objects[1] or "reads it"))
    return objects


def crash_line(stderr):
    """A Rust panic's two lines, without the thread's number; or nothing."""
    lines = stderr.decode("latin-1").split("\n")
    for i, line in enumerate(lines):
        m = re.search(r"panicked at (\S+)", line)
        if m:
            return "panic at %s %s" % (m.group(1), " ".join(lines[i + 1:i + 2]))
    return ""


def compare_results(case, o_dir, o_meta, c_dir, c_meta, want_context=False):
    diffs = []
    for i, step in enumerate(case.steps):
        o = o_meta["steps"][i]
        c = c_meta["steps"][i]
        tag = (step.label + ": ") if step.label and len(case.steps) > 1 else ""
        sdir_o, sdir_c = os.path.join(o_dir, "s%d" % i), os.path.join(c_dir, "s%d" % i)
        stderr_c = read_bytes(os.path.join(sdir_c, "stderr")) or b""
        crashed = False
        if c["timed_out"]:
            diffs.append(Diff("timeout", tag + "candidate", None, "ran to its end", "killed after %.0f s" % c["seconds"]))
            crashed = True
        elif c["signal"] is not None:
            diffs.append(Diff("crash", tag + "candidate", None, "exit %s" % o["exit"], "signal %d (%s) %s" % (c["signal"], signal_name(c["signal"]), crash_line(stderr_c))))
            crashed = True
        elif crash_line(stderr_c):
            diffs.append(Diff("crash", tag + "candidate", None, "exit %s" % o["exit"], "exit %s, %s" % (c["exit"], crash_line(stderr_c))))
            crashed = True
        if o["timed_out"] or o["signal"] is not None:
            diffs.append(Diff("oracle", tag + "oracle", None, "timed out %s, signal %s" % (o["timed_out"], o["signal"]), ""))
        if not step.compare:
            if not c["timed_out"] and c["signal"] is None and any(f.endswith(".fmt") for f in o["files"]) and not any(f.endswith(".fmt") for f in c["files"]):
                diffs.append(Diff("output", tag + "format", None, "dumped", "no format: exit %s" % c["exit"]))
            continue
        fm = step.fmt is not None
        o_files, c_files = o["files"], c["files"]
        o_pdfs = {os.path.basename(r): pdf_facts(os.path.join(sdir_o, "files", r)) for r in o_files if r.endswith(".pdf")}
        c_pdfs = {os.path.basename(r): pdf_facts(os.path.join(sdir_c, "files", r)) for r in c_files if r.endswith(".pdf")}
        for r in sorted(set(o_files) | set(c_files)):
            o_p, c_p = os.path.join(sdir_o, "files", r), os.path.join(sdir_c, "files", r)
            kind = "log" if r.endswith(".log") else "pdf" if r.endswith(".pdf") else "dvi" if r.endswith(".dvi") else "output"
            if r not in o_files or r not in c_files:
                if not crashed or r in c_files:
                    said = lambda files: "file written, %d bytes" % files[r] if r in files else "no such file"
                    diffs.append(Diff(kind, tag + r, None, said(o_files), said(c_files)))
                continue
            if r.endswith(".fmt"):
                continue
            where = tag + r
            if kind == "log":
                d = compare_step_text(sdir_o, sdir_c, os.path.join("files", r), "log", where, o_pdfs, c_pdfs, fm, want_context)
            elif kind == "pdf":
                d = pdf_diff(where, o_p, c_p, want_context)
            elif kind == "dvi":
                d = dvi_diff(where, o_p, c_p, want_context)
            else:
                d = bytes_diff("output", where, read_bytes(o_p) or b"", read_bytes(c_p) or b"", want_context)
            if d:
                diffs.append(d)
        for stream in ("stdout", "stderr"):
            if stream in NOT_COMPARED.get(case.name, ()):
                continue
            d = compare_step_text(sdir_o, sdir_c, stream, "terminal", tag + stream, o_pdfs, c_pdfs, fm, want_context)
            if d and not crashed:
                diffs.append(d)
        if o["exit"] != c["exit"] and not crashed:
            diffs.append(Diff("exit", tag + "exit status", None, str(o["exit"]), str(c["exit"])))
    return diffs


KIND_ORDER = ["tool", "tree", "second", "oracle", "format", "mode", "crash", "timeout", "log", "terminal", "exit", "dvi", "pdf", "output"]


# ----------------------------------------------------------------------------------------------------------------------
# The lists.  compare_results stops at a file's first difference, which is enough to say that two runs differ.  An entry of
# known_divergences.txt or second_oracle.txt says more: these are all the differences there are.  So here each thing compared becomes lines, which are the masked
# text of a log or a terminal, an object of a PDF or a line of dvitype, and an entry holds a hash of what lies between the first and the
# last line that differ, on each side, and how many lines that is.  Anything else that changes moves those limits or changes that text.
# ----------------------------------------------------------------------------------------------------------------------
def read_list(path, names, overlay):
    """{case: (what, reason)} of one file."""
    out = {}
    with open(path, encoding="utf-8") as f:
        for n, line in enumerate(f, 1):
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            parts = line.split(None, 2)
            if len(parts) != 3 or not (re.fullmatch(r"(?:differs|random|banner):-\d+\+\d+:[0-9a-f]{16}|candidate:\+\d+:[0-9a-f]{16}|dies:\d+:\+\d+:[0-9a-f]{16}", parts[1]) or overlay and parts[1] == "agrees"):
                die("%s:%d: a case, one of the forms the header of the file has, and the reason, please" % (path, n))
            if parts[0] not in names:
                die("%s:%d: no case %s" % (path, n, parts[0]))
            if parts[0] in out:
                die("%s:%d: %s again" % (path, n, parts[0]))
            out[parts[0]] = (parts[1], parts[2])
    return out


def allowed_list(path, cases):
    """{case: (what, reason)}, from the list in force, with what this system's build has of its own.  A line that names no case or has no reason is an error, so a renamed case cannot slip off."""
    names = {c.name for c in cases}
    out = read_list(path, names, False)
    overlay = os.path.join(TREE, SYSTEM_OVERLAY) if SECOND_ORACLE and SYSTEM_OVERLAY else None
    if overlay and os.path.exists(overlay):
        for name, entry in read_list(overlay, names, True).items():
            if entry[0] == "agrees":
                out.pop(name, None)
            else:
                out[name] = entry
    return out


def logical_lines(data, pdfs, format_masks):
    """The masked lines with TeX's wrapping undone and the tree's place taken out, so that a difference recorded on one machine holds on another.
    That goes for the new line too that section 537 starts for a file name too long for the rest of this one."""
    # Arch's oracle has its tree at one place, with the same paths for both programs, and only this machine's run reads known_divergences.txt.
    root = oracle_root() if SECOND_ORACLE else None
    # But not its font map: see font_map().  Under the second oracle the map is in the tree, and the lists have its path as "TEXLIVE/...".
    fontmap = None if SECOND_ORACLE else font_map()
    out, held = [], ""
    for line in mask_lines(data, pdfs, format_masks) + [""]:
        held += line
        if len(line) != MAX_PRINT_LINE:
            if fontmap:
                held = held.replace("{" + fontmap, "{pdftex.map")
            if root and held.startswith("(") and root.match(held, 1) and out:
                out[-1] += " " + root.sub("TEXLIVE", held)
            else:
                out.append(root.sub("TEXLIVE", held) if root else held)
            held = ""
    return out


FONT_MAP = []


def font_map():
    """The pdftex.map that kpathsea finds for the oracle, as both programs print it at their first page, "{PATH": TEXMFVAR's, which is the user's own,
    ~/.texlive/texmf-var/..., once updmap has run for that user, and the system's otherwise.  For the tokens of the lists alone, so that one recorded
    on one machine holds on another: the comparison itself sees the path as it is, and a candidate that prints another one differs there and in its
    token.  None if kpsewhich gives no answer."""
    if not FONT_MAP:
        try:
            p = subprocess.run([os.path.join(ORACLE_DIR, "kpsewhich" + EXE), "-progname=pdftex", "-format=map", "pdftex.map"], env=environment(),
                               stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=60, check=False)
            found = p.stdout.decode("latin-1").strip()
        except (OSError, subprocess.SubprocessError):
            found = ""
        FONT_MAP.append(found if found and found != "pdftex.map" else None)
    return FONT_MAP[0]


def oracle_root():
    """The tree's place, as a log has it: on Windows in forward slashes, and with the drive letter in the one case or the other."""
    root = os.path.dirname(os.path.dirname(os.path.realpath(ORACLE_DIR)))
    if WINDOWS:
        return re.compile(re.escape(root.replace("\\", "/")), re.I)
    return re.compile(re.escape(root))


def pdf_lines(path):
    """A line per object, as objects_diff sees it."""
    objects, err = qpdf_objects(path)
    if objects is None:
        # An empty file, for one, and qpdf's message names it: where the tree is stays out of a token.
        return ["qpdf: %s" % err.replace(path, "PDF"), sha(read_bytes(path) or b"")]
    xref = any(is_xref_stream(o) for o in objects.values() if isinstance(o, dict))
    out = []
    for k in sorted(objects, key=object_key):
        o = objects[k]
        if is_xref_stream(o):
            continue
        if k == "trailer":
            v = canon_trailer(o, xref)
        elif "stream" in o:
            d = dict(o["stream"]["dict"])
            d.pop("/Length", None)
            v = [d, sha(base64.b64decode(o["stream"].get("data", "")))]
        else:
            v = o
        out.append("%s %s" % (k, json.dumps(canon_value(v), sort_keys=True)))
    return out


def between_differences(where, a, b):
    """[where, the oracle's lines, the candidate's] from the first line that differs to the last, or None."""
    i = 0
    while i < len(a) and i < len(b) and a[i] == b[i]:
        i += 1
    if i == len(a) == len(b):
        return None
    j = 0
    while j < len(a) - i and j < len(b) - i and a[-1 - j] == b[-1 - j]:
        j += 1
    return [where, a[i:len(a) - j], b[i:len(b) - j]]


def all_differences(case, o_dir, o_meta, c_dir, c_meta):
    """What compare_results looks at, in full."""
    out = []
    for i, step in enumerate(case.steps):
        if not step.compare:
            continue
        o, c = o_meta["steps"][i], c_meta["steps"][i]
        sdir_o, sdir_c = os.path.join(o_dir, "s%d" % i), os.path.join(c_dir, "s%d" % i)
        o_pdfs = {os.path.basename(r): pdf_facts(os.path.join(sdir_o, "files", r)) for r in o["files"] if r.endswith(".pdf")}
        c_pdfs = {os.path.basename(r): pdf_facts(os.path.join(sdir_c, "files", r)) for r in c["files"] if r.endswith(".pdf")}
        fm = step.fmt is not None
        for r in sorted(set(o["files"]) | set(c["files"])):
            o_p, c_p = os.path.join(sdir_o, "files", r), os.path.join(sdir_c, "files", r)
            if r.endswith(".fmt"):
                continue
            if r not in o["files"] or r not in c["files"]:
                out.append(["%d %s" % (i, r), [r] if r in o["files"] else [], [r] if r in c["files"] else []])
            elif r.endswith(".log"):
                out.append(between_differences("%d %s" % (i, r), logical_lines(read_bytes(o_p) or b"", o_pdfs, fm), logical_lines(read_bytes(c_p) or b"", c_pdfs, fm)))
            elif r.endswith(".pdf"):
                out.append(between_differences("%d %s" % (i, r), pdf_lines(o_p), pdf_lines(c_p)))
            elif r.endswith(".dvi"):
                if read_bytes(o_p) != read_bytes(c_p):
                    out.append(between_differences("%d %s" % (i, r), dvitype(o_p)[1:], dvitype(c_p)[1:]))
            else:
                out.append(between_differences("%d %s" % (i, r), (read_bytes(o_p) or b"").decode("latin-1").split("\n"), (read_bytes(c_p) or b"").decode("latin-1").split("\n")))
        for stream in ("stdout", "stderr"):
            if stream in NOT_COMPARED.get(case.name, ()):
                continue
            a, b = oracle_terminal(read_bytes(os.path.join(sdir_o, stream)) or b""), read_bytes(os.path.join(sdir_c, stream)) or b""
            if stream == "stderr":
                a, b = STDERR_DROP.sub(b"", a), STDERR_DROP.sub(b"", candidate_stderr(b))
            out.append(between_differences("%d %s" % (i, stream), logical_lines(a, o_pdfs, fm), logical_lines(b, c_pdfs, fm)))
        if o["exit"] != c["exit"]:
            out.append(["%d exit" % i, [str(o["exit"])], [str(c["exit"])]])
    return [d for d in out if d]


def differences_token(differences):
    """-N+M:HASH: the lines between first and last difference, on the oracle and on the candidate, summed over all that is compared, and a hash of those lines.
    What the pool has left depends on what the format holds, and with it on where the tree is, so that one number stays out of the hash."""
    text = re.sub(r"pool size=\d+", "pool size=N", json.dumps(differences))
    return "-%d+%d:%s" % (sum(len(d[1]) for d in differences), sum(len(d[2]) for d in differences), sha(text)[:16])


def candidate_token(lines):
    """+M:HASH: the same of the candidate's side alone, for an oracle whose side is not the same twice."""
    return "+%d:%s" % (sum(len(d[-1]) for d in lines), sha(json.dumps([[d[0], d[-1]] for d in lines]))[:16])


def candidate_result(case, c_dir, c_meta):
    """All that is compared, of the candidate's side alone: for an oracle that dies, so that the case still says what the candidate must give."""
    out = []
    for i, step in enumerate(case.steps):
        if not step.compare:
            continue
        c = c_meta["steps"][i]
        sdir = os.path.join(c_dir, "s%d" % i)
        pdfs = {os.path.basename(r): pdf_facts(os.path.join(sdir, "files", r)) for r in c["files"] if r.endswith(".pdf")}
        fm = step.fmt is not None
        for r in sorted(c["files"]):
            p = os.path.join(sdir, "files", r)
            if r.endswith(".fmt"):
                continue
            if r.endswith(".log"):
                out.append(["%d %s" % (i, r), logical_lines(read_bytes(p) or b"", pdfs, fm)])
            elif r.endswith(".pdf"):
                out.append(["%d %s" % (i, r), pdf_lines(p)])
            elif r.endswith(".dvi"):
                out.append(["%d %s" % (i, r), dvitype(p)[1:]])
            else:
                out.append(["%d %s" % (i, r), (read_bytes(p) or b"").decode("latin-1").split("\n")])
        for stream in ("stdout", "stderr"):
            if stream not in NOT_COMPARED.get(case.name, ()):
                data = read_bytes(os.path.join(sdir, stream)) or b""
                if stream == "stderr":
                    data = STDERR_DROP.sub(b"", candidate_stderr(data))
                out.append(["%d %s" % (i, stream), logical_lines(data, pdfs, fm)])
        out.append(["%d exit" % i, [str(c["exit"])]])
    return out


def list_verdict(entry, case, o_dir, o_meta, c_dir, c_meta, diffs):
    """The differences of a case under its entry, or under none: the list to report, with a kind "second" if it does not hold.  What a difference can be is exact, so a stale entry fails."""
    listed = os.path.basename(SECOND_ORACLE_LIST if SECOND_ORACLE else KNOWN_DIVERGENCES)
    fail = lambda said, now: [Diff("second", listed, None, said, now)] + diffs
    bad = [d for d in diffs if d.kind in NEVER_ACCEPTED and d.kind != "oracle"]
    if entry is None:
        if not SECOND_ORACLE or not diffs or bad or any(d.kind == "oracle" for d in diffs):
            return diffs
        return fail("no entry", "differs:" + differences_token(all_differences(case, o_dir, o_meta, c_dir, c_meta)))
    what, _ = entry
    signals = [s["signal"] for s in o_meta["steps"] if s["signal"] is not None]
    if bad:
        return diffs
    if what.startswith("dies:"):
        signal, token = what[5:].split(":", 1)
        if signals != [int(signal)]:
            return fail(what, "the oracle's signals: %s" % signals)
        now = candidate_token(candidate_result(case, c_dir, c_meta))
        if now != token:
            return fail(what, "dies:%s:%s" % (signal, now))
        # The oracle's result ends there: what is left to say is that it died, and the candidate did not, and gave what it gave.
        return [Diff("exit", "exit status", None, "signal %d" % signals[0], str(c_meta["steps"][-1]["exit"]))]
    if signals:
        return diffs
    kind, token = what.split(":", 1)
    if not diffs:
        return diffs if kind != "differs" else fail(what, "agrees")
    differences = all_differences(case, o_dir, o_meta, c_dir, c_meta)
    now = candidate_token(differences) if kind == "candidate" else differences_token(differences)
    return diffs if now == token else fail(what, kind + ":" + now)


# ----------------------------------------------------------------------------------------------------------------------
# One case in a worker
# ----------------------------------------------------------------------------------------------------------------------
CTX = {}


def worker_init(ctx):
    CTX.update(ctx)
    if resource:
        resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    elif WINDOWS:
        # SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX, which a child takes over: an oracle that dies waits for no one to close a box.
        import ctypes
        ctypes.windll.kernel32.SetErrorMode(0x8003)
    CTX["cases"] = {c.name: c for c in discover()[0]}
    CTX["inputs"] = Inputs()


def candidate_argv(step):
    argv = [CTX["engine"]]
    if step.fmt:
        fmt = CTX["formats"][step.fmt]
        argv += ["-progname=" + step.fmt, "-fmt=" + fmt["path"]]
    return argv + step.args


def oracle_argv(step):
    return [oracle_program(step.program)] + step.args


def run_one(name, want_context=False):
    """One case in a worker.  An exception here is this tool's bug: it becomes that case's result, so the run goes on."""
    try:
        return run_case_compare(name, want_context)
    except Exception as e:
        case = CTX["cases"][name]
        return {"name": name, "module": case.module, "mode": case.mode,
                "diffs": [Diff("tool", "run.py", None, "", "%s: %s" % (type(e).__name__, e)).as_json()]}


def run_case_compare(name, want_context):
    case = CTX["cases"][name]
    start = time.monotonic()
    result = {"name": name, "module": case.module, "mode": case.mode, "diffs": []}
    if case.mode not in MODES + ("pair", "cli"):
        result["diffs"] = [Diff("mode", "% mode:", None, "known modes", case.mode).as_json()]
        return result
    for step in case.steps:
        if step.fmt and not CTX["formats"][step.fmt]["path"]:
            result["diffs"] = [Diff("format", step.fmt + ".fmt", None, "built", CTX["formats"][step.fmt]["problem"]).as_json()]
            return result
    work = os.path.join(OUT, "work", case.slug())
    key = CTX["inputs"].key(case, CTX["identity"], work)
    o_dir = os.path.join(OUT, "oracle", key)
    o_meta = load_meta(o_dir)
    if o_meta is None:
        o_meta = run_case(case, oracle_argv, o_dir, CTX, ORACLE_TIMEOUT)
    c_dir = os.path.join(OUT, "candidate", case.slug())
    c_meta = run_case(case, candidate_argv, c_dir, CTX, CTX["timeout"])
    diffs = compare_results(case, o_dir, o_meta, c_dir, c_meta, want_context)
    diffs = list_verdict(CTX["listed"].get(name), case, o_dir, o_meta, c_dir, c_meta, diffs)
    if diffs and CTX.get("differences"):
        write_differences(CTX["differences"], case, o_dir, o_meta, c_dir, c_meta)
    wrote = sorted(k for k, v in scan(CTX["master"]).items() if CTX["manifest"].get(k) != v)
    if wrote:
        diffs.insert(0, Diff("tree", wrote[0], None, "the shared copy of the tree is as it was", "it changed in this case, %d files" % len(wrote)))
    if any(s["timed_out"] or s["signal"] is not None for s in o_meta["steps"]):
        # Says something about this moment on this machine, so it is not kept.
        shutil.rmtree(o_dir, ignore_errors=True)
    result["diffs"] = [d.as_json() for d in diffs]
    result["seconds"] = round(time.monotonic() - start, 3)
    result["kept"] = c_dir
    result["oracle"] = o_dir
    if not diffs and not CTX.get("keep"):
        shutil.rmtree(c_dir, ignore_errors=True)
    if want_context:
        result["context"] = [d.context for d in diffs]
    return result


def write_dvi_bytes(f, case, o_dir, o_meta, c_dir, c_meta):
    """Where two DVI files differ that dvitype shows the same at its level: what that level leaves out."""
    for i in range(len(case.steps)):
        for r in o_meta["steps"][i]["files"]:
            if r.endswith(".dvi") and r in c_meta["steps"][i]["files"]:
                a, b = read_bytes(os.path.join(o_dir, "s%d" % i, "files", r)) or b"", read_bytes(os.path.join(c_dir, "s%d" % i, "files", r)) or b""
                if a != b:
                    at = next((k for k, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
                    same = sum(x != y for x, y in zip(a, b))
                    f.write("%d %s: %d and %d bytes, %d differ, the first at %d: %s and %s\n" % (i, r, len(a), len(b), same, at, a[max(0, at - 8):at + 24].hex(" "), b[max(0, at - 8):at + 24].hex(" ")))


def write_differences(root, case, o_dir, o_meta, c_dir, c_meta):
    """All that the case differs in, for a reader: what a list's entry has only the hash of, and the two logs as they came, for what a mask or the wrapping hides."""
    for i in range(len(case.steps)):
        for side, d, meta in (("oracle", o_dir, o_meta), ("candidate", c_dir, c_meta)):
            for r in meta["steps"][i]["files"]:
                if r.endswith(".log"):
                    data = read_bytes(os.path.join(d, "s%d" % i, "files", r)) or b""
                    if len(data) < 1 << 20:
                        with open(os.path.join(root, "%s.%d.%s.log.txt" % (case.slug(), i, side)), "wb") as f:
                            f.write(data)
    with open(os.path.join(root, case.slug() + ".txt"), "w", encoding="utf-8", errors="backslashreplace", newline="\n") as f:
        try:
            differences = all_differences(case, o_dir, o_meta, c_dir, c_meta)
        except Exception as e:
            f.write("cannot say: %s: %s\n" % (type(e).__name__, e))
            return
        f.write("%s differs:%s\n" % (case.name, differences_token(differences) if differences else "(nothing)"))
        write_dvi_bytes(f, case, o_dir, o_meta, c_dir, c_meta)
        for where, a, b in differences:
            f.write("\n=== %s: %d lines of the oracle's, %d of the candidate's\n" % (where, len(a), len(b)))
            for sign, lines in (("-", a), ("+", b)):
                for line in lines[:300]:
                    f.write("%s %s\n" % (sign, line[:400]))
                if len(lines) > 300:
                    f.write("%s ... and %d more\n" % (sign, len(lines) - 300))


# ----------------------------------------------------------------------------------------------------------------------
# Reporting
# ----------------------------------------------------------------------------------------------------------------------
def cut(s):
    """One side of a report line: cut, and with TeX's notation for what would break the line."""
    s = re.sub(r"[\x00-\x1f\x7f]", lambda m: "^^%c" % (ord(m.group()) ^ 64), str(s))
    return s if len(s) <= CUT else s[:CUT] + "..."


def report_line(r):
    d = min(r["diffs"], key=lambda d: KIND_ORDER.index(d["kind"]))
    kinds = sorted({x["kind"] for x in r["diffs"]}, key=KIND_ORDER.index)
    at = "%s:%d" % (d["where"], d["line"]) if d["line"] else d["where"]
    return "FAIL %s [%s] %s | oracle: %s | candidate: %s" % (r["name"], "+".join(kinds), at, cut(d["oracle"]), cut(d["candidate"]))


def signature(r):
    """What groups a failure with others in the list of common first differences."""
    d = min(r["diffs"], key=lambda d: KIND_ORDER.index(d["kind"]))
    norm = lambda s: re.sub(r"[0-9]+", "#", re.sub(r"Output written on \S+", "Output written on FILE", str(s)))[:110]
    a, b = norm(d["oracle"]), norm(d["candidate"])
    # An error on one side says more than whatever the other side happens to have on that line.
    if b.startswith("! "):
        a = "(any)"
    elif a.startswith("! "):
        b = "(any)"
    return d["kind"], a, b


def print_report(results, top, known):
    modules = collections.OrderedDict()
    for r in sorted(results, key=lambda r: r["name"]):
        modules.setdefault(r["module"], []).append(r)
    print("%-12s %s" % ("module", "agree/cases"))
    for m, rs in modules.items():
        n = sum(1 for r in rs if is_accepted(r, known))
        print("%-12s %d/%d%s" % (m, sum(1 for r in rs if not r["diffs"]), len(rs) - n, ", and %d accepted" % n if n else ""))
    good = sum(1 for r in results if not r["diffs"])
    accepted = [r for r in sorted(results, key=lambda r: r["name"]) if is_accepted(r, known)]
    counted = len(results) - len(accepted)
    print("%-12s %d/%d (%.1f%%)%s" % ("total", good, counted, 100.0 * good / max(1, counted), ", and %d accepted" % len(accepted) if accepted else ""))
    if accepted:
        print()
        for r in accepted:
            print(report_line(r).replace("FAIL ", "ACCEPTED ", 1))
            print("    " + known[r["name"]])
    for r in sorted(results, key=lambda r: r["name"]):
        if not r["diffs"] and r["name"] in known:
            print("AGREES, on %s: %s" % ("second_oracle.txt" if SECOND_ORACLE else "known_divergences.txt", r["name"]))
    bad = [r for r in sorted(results, key=lambda r: r["name"]) if r["diffs"] and not is_accepted(r, known)]
    if bad:
        print()
        for r in bad:
            print(report_line(r))
        kinds = collections.Counter()
        for r in bad:
            kinds[min(r["diffs"], key=lambda d: KIND_ORDER.index(d["kind"]))["kind"]] += 1
        print()
        print("failures by first kind: " + ", ".join("%s %d" % kv for kv in kinds.most_common()))
        print("most common first differences:")
        groups = collections.Counter(signature(r) for r in bad)
        for (kind, a, b), n in groups.most_common(top):
            print("  %4d [%s] oracle: %s | candidate: %s" % (n, kind, a, b))


def print_first_diff(result, known):
    print("case %s (mode %s)" % (result["name"], result["mode"]))
    if result["name"] in known:
        print("on %s: %s" % ("second_oracle.txt" if SECOND_ORACLE else "known_divergences.txt", known[result["name"]]))
    if not result["diffs"]:
        print("agrees with the oracle")
        return
    print("raw results kept in %s, oracle's in %s" % (result.get("kept"), result.get("oracle")))
    for d, ctx in zip(result["diffs"], result.get("context", [None] * len(result["diffs"]))):
        at = "%s:%d" % (d["where"], d["line"]) if d["line"] else d["where"]
        print("\n--- [%s] %s\n  oracle:    %s\n  candidate: %s" % (d["kind"], at, d["oracle"], d["candidate"]))
        if ctx:
            for who in ("oracle", "candidate"):
                print("  %s, from line %d:" % (who, ctx["from"]))
                for j, line in enumerate(ctx[who]):
                    print("    %5d %s" % (ctx["from"] + j, line))


def main():
    sys.stdout.reconfigure(errors="backslashreplace")   # file names and logs are bytes, and may be anything
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--engine")
    ap.add_argument("--installed-formats", action="store_true", help="use the oracle's installed formats for the candidate")
    ap.add_argument("--only", action="append", default=[])
    ap.add_argument("--filter")
    ap.add_argument("--failed", action="store_true")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--first-diff")
    ap.add_argument("--json")
    ap.add_argument("-j", "--jobs", type=int, default=max(1, (os.cpu_count() or 3) - 2))
    ap.add_argument("--timeout", type=float, default=20.0)
    ap.add_argument("--top", type=int, default=10)
    ap.add_argument("--keep", action="store_true", help="keep the raw results of cases that agree too")
    ap.add_argument("--differences", metavar="DIR", help="all that each case differs in, one file a case")
    args = ap.parse_args()

    cases, no_mode, odd = discover()
    # A list says how rstex differs, so the oracle against itself, --installed-formats, has none: anything that differs there is a fault of this tool, or an oracle that is not the same twice.
    listed = {} if args.installed_formats else allowed_list(SECOND_ORACLE_LIST if SECOND_ORACLE else KNOWN_DIVERGENCES, cases)
    known = {name: reason for name, (_, reason) in listed.items()}
    selected = cases
    if args.only:
        modules = {c.module for c in cases}
        for m in args.only:
            if m not in modules:
                die("no module %s; there are: %s" % (m, ", ".join(sorted(modules))))
        selected = [c for c in selected if c.module in args.only]
    if args.filter:
        rx = re.compile(args.filter)
        selected = [c for c in selected if rx.search(c.name)]
    if args.failed:
        last = read_bytes(os.path.join(OUT, "last.json"))
        if last is None:
            die("no last run in " + OUT)
        names = {r["name"] for r in json.loads(last) if r["diffs"]}
        selected = [c for c in selected if c.name in names]
    if args.first_diff:
        selected = [c for c in cases if c.name == args.first_diff or c.name == re.sub(r"\.tex$", "", args.first_diff)]
        if not selected:
            die("no case " + args.first_diff)
    if args.list:
        for c in selected:
            print("%-9s %s" % (c.mode, c.name))
        print("%d cases" % len(selected))
        print("not cases, no mode line: " + (", ".join(no_mode) or "none"))
        print("modes beyond %s: %s" % (", ".join(MODES), ", ".join("%s (%d)" % kv for kv in odd.items()) or "none"))
        return 0

    engine = args.engine or os.path.join(os.environ.get("CARGO_TARGET_DIR") or os.path.join(ROOT, "target"), "debug", "rstex")
    engine = os.path.abspath(engine)
    if WINDOWS and not os.path.exists(engine) and os.path.exists(engine + EXE):
        engine += EXE
    if not os.access(engine, os.X_OK):
        die("no engine at %s; give --engine or cargo build -p rstex" % engine)
    for tool in ("qpdf", "kpsewhich", oracle_program("pdftex")):
        if not shutil.which(tool):
            die("this needs " + tool)

    os.makedirs(OUT, exist_ok=True)
    print("oracle %s/pdftex; candidate %s" % (ORACLE_DIR, engine))
    print("not cases, no mode line: " + (", ".join(no_mode) or "none"))
    if odd:
        print("modes beyond %s: %s" % (", ".join(MODES), ", ".join("%s (%d)" % kv for kv in odd.items())))
    identity = oracle_identity()

    needed = sorted({s.fmt for c in selected for s in c.steps if s.fmt})
    if args.installed_formats:
        formats = {f: {"path": f, "problem": None, "notes": ["installed format"]} for f in needed}
    else:
        formats = build_formats(engine, needed)
    for f, info in sorted(formats.items()):
        print("format %s: %s%s" % (f, info["path"] or info["problem"], "".join("; " + n for n in info["notes"])))

    master = os.path.join(OUT, "master")
    shutil.rmtree(master, ignore_errors=True)
    shutil.copytree(TREE, master, copy_function=shutil.copy2, ignore=shutil.ignore_patterns("__pycache__", "run.py", "known_divergences.txt", "second_oracle*.txt"))
    for dirpath, _, files in os.walk(master):
        for f in files:
            os.utime(os.path.join(dirpath, f), (FIXED_MTIME, FIXED_MTIME))

    differences = os.path.abspath(args.differences) if args.differences else None
    if differences:
        os.makedirs(differences, exist_ok=True)
        # What each format was built from, by the logs of the two builds.
        for f, info in formats.items():
            installed = installed_format(f)
            for side, log in (("oracle", installed[:-4] + ".log" if installed else ""), ("candidate", (info["path"] or "") + ".log")):
                if os.path.exists(log):
                    shutil.copy(log, os.path.join(differences, "format-%s.%s.log.txt" % (f, side)))
    ctx = {"engine": engine, "formats": formats, "identity": identity, "timeout": args.timeout, "master": master, "differences": differences,
           "image_src": build_image_source(identity), "keep": args.keep, "manifest": scan(master), "listed": listed}
    names = [c.name for c in selected]

    if args.first_diff:
        worker_init(ctx)
        print_first_diff(run_one(names[0], want_context=True), known)
        return 0

    start = time.monotonic()
    results = []
    with multiprocessing.Pool(args.jobs, worker_init, (ctx,)) as pool:
        for i, r in enumerate(pool.imap_unordered(run_one, names), 1):
            results.append(r)
            if sys.stderr.isatty():
                sys.stderr.write("\r%d/%d " % (i, len(names)))
    if sys.stderr.isatty():
        sys.stderr.write("\r")
    print("ran %d cases in %.1f s with -j%d" % (len(names), time.monotonic() - start, args.jobs))
    print_report(results, args.top, known)
    if not args.failed and not args.only and not args.filter:
        with open(os.path.join(OUT, "last.json"), "w") as f:
            json.dump(results, f)
    elif os.path.exists(os.path.join(OUT, "last.json")):
        # A partial run updates those cases in the record of the last whole run.
        with open(os.path.join(OUT, "last.json")) as f:
            old = {r["name"]: r for r in json.load(f)}
        old.update({r["name"]: r for r in results})
        with open(os.path.join(OUT, "last.json"), "w") as f:
            json.dump(list(old.values()), f)
    if args.json:
        with open(args.json, "w") as f:
            json.dump(sorted(results, key=lambda r: r["name"]), f, indent=1)
    return 0 if all(not r["diffs"] or is_accepted(r, known) for r in results) else 1


if __name__ == "__main__":
    sys.exit(main())
