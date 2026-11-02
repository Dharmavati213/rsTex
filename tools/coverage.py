#!/usr/bin/env python3
"""The mechanical completeness gate.  Run with python3 -I.

    coverage.py FILE A-B[,C-D...]   check one Rust file against a section range
    coverage.py --module NAME       the same, with the file and ranges from tools/modmap.py
    coverage.py --all               every module, one summary line each
    coverage.py --stubs             WEB procedures against the Rust functions in crates/engine/src

The unit of porting is the procedure.  As tangle does, a procedure's Rust contains every WEB module
reachable from its body, wherever that module was defined; so a section's code "lands" in the file of
each procedure reaching it (`web.py landing N` shows where, `web.py tangle NAME` shows a procedure
with its modules inlined).

--module NAME requires a marker for every section that lands in the module's file, from any range,
and for what is left in the module's own range.  With an explicit FILE and range it checks the range,
excusing a section that lands only in other files.

A required section is either ported, with a `// §N` or `/// §N` comment in the file, or listed in
the module's header comment as `//! DROPPED §N: reason` or `//! DROPPED §N-M: reason`.  Exempt:

  - a section with no code: only TeX commentary;
  - a section whose only content is @d macros, all of which exist already (a const in consts.rs, or
    a method anywhere in the engine), or a declaration of globals, types or constants, which live
    in state.rs and consts.rs;
  - the dropped features in GLOBALLY_DROPPED below.

And every procedure defined in the range must exist as a function with a body and no `todo!`.

Two checks for a misspelt constant in a pattern, which compiles as a variable binding.  `deny(unreachable_patterns)`
does not catch `(HMODE, SPACERR) => ...`: it catches every remaining hmode command, and the `_` arm stays reachable
for the other modes.  So an upper-case name that no `const` defines is a problem, and a module counts as COMPLETE
only when its header says `#![deny(unreachable_patterns, unused_variables)]` and no longer allows `unused_variables`.

Options: --strict drops the exemptions, --quiet lists only the problems.
Exit status: 0 if complete, 1 if not.
"""
import glob
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import web as weblib  # noqa: E402
from modmap import MODULES, section_ranges  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(ROOT, "crates", "engine", "src")

RENAME = {"type": "ty", "box": "box_reg", "set_lc_code": "set_hc0_lc_code"}
RUST_NAME_OF_PROC = {"flush_list": "mem_flush_list"}
# Control macros and others with no Rust counterpart, by design: see docs/PORTING.md.
MACROS_WITHOUT_RUST = set("""
debug gubed stat tats init tini Init Tini othercases endcases loop do_nothing return text_char pointer
sc mtype term_in term_out t_open_out TEX hi ho qi qo si so float unfloat float_constant
set_glue_ratio_zero set_glue_ratio_one negate double remainder free log_name wterm wterm_ln wlog
wlog_ln print_ASCII hlp1 hlp2 hlp3 hlp4 hlp5 hlp6 help0 help1 help2 help3 help4 help5 help6
next hash_is_full prim_is_full store_new_token fast_store_new_token
char_info_end orig_char_info_end char_width_end char_italic_end char_height_end char_depth_end
char_kern_end lig_kern_restart_end param_end mathsy_end scanned_result_end set_conversion_end
append_list_end pdf_append_list pdf_append_list_end setup_bound_var_end setup_bound_var_end_end
set_box_lr_end any_mode non_math any_state_plus add_delims_to four_cases abort call_func global
natural edit_file active_height trie_ref cur_file do_final_end vet_glue banner_k file_name_size
print_err print_ignored_err prompt_input fget fbyte const_chk setup_bound_var too_small undump
undump_end undump_end_end undump_size undump_size_end undump_size_end_end format_debug
format_debug_end dump_four_ASCII undump_four_ASCII check_effective_tail add_char_stretch_end
add_char_shrink_end sub_char_stretch_end sub_char_shrink_end add_kern_stretch_end add_kern_shrink_end
sub_kern_stretch_end sub_kern_shrink_end print_plus_end cancel_glue_cont cancel_glue_cont_cont
cancel_glue_end cancel_glue_end_end
""".split())


# Features the whole port drops, DESIGN.md section 3.  No file needs a marker for these sections.
GLOBALLY_DROPPED = {}
for _k in range(1881, 1895):
    GLOBALLY_DROPPED[_k] = "MLTeX"


def rust_tokens_clean(text):
    """Rust source with comments and strings blanked, for brace matching."""
    out = []
    i = 0
    n = len(text)
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            i = n if j < 0 else j
            continue
        if text.startswith("/*", i):
            j = text.find("*/", i + 2)
            i = n if j < 0 else j + 2
            continue
        if c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            out.append('""')
            i = j + 1
            continue
        m = re.compile(r"b?'(\\.|\\x[0-9a-fA-F]{2}|[^\\'])'").match(text, i)
        if m:
            out.append("' '")
            i = m.end()
            continue
        out.append(c)
        i += 1
    return "".join(out)


def function_bodies(text):
    """dict name -> body text, for every `fn name` with a block."""
    clean = rust_tokens_clean(text)
    out = {}
    for m in re.finditer(r"\bfn\s+([A-Za-z_][A-Za-z_0-9]*)\s*(?:<[^>]*>)?\s*\(", clean):
        i = m.end()
        depth = 1
        while i < len(clean) and depth:
            if clean[i] == "(":
                depth += 1
            elif clean[i] == ")":
                depth -= 1
            i += 1
        # up to the opening brace, or a semicolon for a declaration
        while i < len(clean) and clean[i] not in "{;":
            i += 1
        if i >= len(clean) or clean[i] == ";":
            continue
        start = i
        depth = 0
        while i < len(clean):
            if clean[i] == "{":
                depth += 1
            elif clean[i] == "}":
                depth -= 1
                if depth == 0:
                    break
            i += 1
        out[m.group(1)] = clean[start : i + 1]
    return out


def markers(text):
    """Sections with a `// §N` or `/// §N` comment, and sections listed as dropped."""
    ported = set()
    dropped = {}
    for line in text.split("\n"):
        s = line.strip()
        if s.startswith("//!"):
            m = re.match(r"//!\s*DROPPED\s+§(\d+)(?:-(\d+))?\s*:\s*(.*)", s)
            if m:
                a = int(m.group(1))
                b = int(m.group(2) or a)
                for k in range(a, b + 1):
                    dropped[k] = m.group(3)
            continue
        i = line.find("//")
        if i < 0:
            continue
        for m in re.finditer(r"§(\d+)(?:-(\d+))?", line[i:]):
            a = int(m.group(1))
            b = int(m.group(2) or a)
            ported.update(range(a, b + 1))
    return ported, dropped


class Engine:
    """What exists in crates/engine/src."""

    def __init__(self):
        self.functions = {}  # name -> (file, body)
        self.consts = set()
        self.upper_names = set()  # what an upper-case name may resolve to, anywhere in the crate
        for path in glob.glob(os.path.join(SRC, "**", "*.rs"), recursive=True):
            text = open(path, encoding="utf-8").read()
            for m in re.finditer(r"\b(?:const|static|struct|enum|type|trait|union)\s+(?:mut\s+)?([A-Za-z_][A-Za-z_0-9]*)", text):
                self.upper_names.add(m.group(1))
            for m in re.finditer(r"\benum\s+[A-Za-z_0-9]+[^{]*\{([^}]*)\}", text):
                self.upper_names.update(re.findall(r"\b[A-Za-z_][A-Za-z_0-9]*\b", m.group(1)))
            if path != os.path.join(SRC, os.path.basename(path)):
                continue
            base = os.path.basename(path)
            for name, body in function_bodies(text).items():
                self.functions.setdefault(name, (base, body))
            for m in re.finditer(r"pub const ([A-Z_0-9a-z]+)\s*:", text):
                self.consts.add(m.group(1))

    def has_macro(self, name):
        if name in MACROS_WITHOUT_RUST:
            return True
        if name.upper() in self.consts:
            return True
        return RENAME.get(name, name) in self.functions


def misspelt_names(text, eng):
    """Upper-case names that no `const` defines.  Anywhere but in a pattern rustc reports them; in a
    pattern they silently bind a variable."""
    clean = rust_tokens_clean(text)
    out = set()
    for m in re.finditer(r"(?<![A-Za-z_0-9:$])([A-Z][A-Z0-9_]+)\b(?!\s*::|!)", clean):
        if m.group(1) not in eng.upper_names:
            out.add(m.group(1))
    return sorted(out)


def header_problem(text):
    """A finished module denies unused_variables: a misspelt constant in a tuple arm is one."""
    code = rust_tokens_clean(text)
    allow = re.search(r"#!\[allow\(([^)]*)\)\]", code)
    deny = re.search(r"#!\[deny\(([^)]*)\)\]", code)
    if allow and "unused_variables" in allow.group(1):
        return "the header still allows unused_variables: use #![deny(unreachable_patterns, unused_variables)]"
    if not deny or "unreachable_patterns" not in deny.group(1) or "unused_variables" not in deny.group(1):
        return "the header must say #![deny(unreachable_patterns, unused_variables)]"
    return None


def rust_proc_name(n):
    return RUST_NAME_OF_PROC.get(n, RENAME.get(n, n))


def check(w, eng, path, ranges, strict=False, quiet=False, module=None, imports=False):
    """Check one file.  `module` is the Rust module the file holds; `imports` also requires every
    section that lands in the file from outside the ranges."""
    text = open(path, encoding="utf-8").read()
    ported, dropped = markers(text)
    base = os.path.basename(path)
    missing = []
    exempt = 0
    exported = 0
    total = 0
    macros_missing = {}
    inrange = []
    for a, b in ranges:
        inrange.extend(range(a, b + 1))
    secs = set(inrange)
    files = weblib.landing_files(w)
    work = list(inrange)
    if imports:
        extra = sorted(k for k, fs in files.items() if module in fs and k not in secs)
        work = sorted(set(work) | set(extra))
    for k in work:
        total += 1
        if k in ported or k in dropped:
            continue
        s = w.sections[k]
        if not strict and k in GLOBALLY_DROPPED:
            exempt += 1
            continue
        if not strict and module is not None and k in files and module not in files[k]:
            exported += 1  # its code goes in another file
            continue
        if not strict:
            if not s.defs.strip() and not s.pascal.strip():
                exempt += 1
                continue
            # Only forward declarations: all procedures are methods, so nothing to port.
            rest = re.sub(r"@<.*?@>", " ", weblib.strip_comments(s.pascal), flags=re.S)
            rest = re.sub(r"@t.*?@>", " ", rest, flags=re.S)
            rest = re.sub(r"(procedure|function)\s+[A-Za-z_0-9]+\s*;\s*forward\s*;", " ", rest)
            if s.pascal.strip() and not rest.strip() and not s.defs.strip():
                exempt += 1
                continue
            decl = s.is_def_of is not None and weblib._module_full_name(w, s.is_def_of) in DECL_MODULES
            if decl or not s.pascal.strip():
                names = [m.name for m in w.macro_list if m.sec == k]
                absent = [n for n in names if not eng.has_macro(n)]
                if not absent:
                    exempt += 1
                    continue
                macros_missing[k] = absent
        missing.append(k)
    problems = []
    for p in w.procs:
        if p.forward or p.sec not in secs:
            continue
        fn = rust_proc_name(p.name)
        found = eng.functions.get(fn)
        if not found:
            problems.append("§%d %s: no function %s" % (p.sec, p.name, fn))
            continue
        fname, body = found
        if re.search(r"\b(todo|unimplemented)!", body):
            problems.append("§%d %s: still a stub, in %s" % (p.sec, p.name, fname))
    for name in misspelt_names(text, eng):
        problems.append("%s: no const of this name: a misspelt constant in a pattern binds a variable" % name)
    if module is not None and not missing and not problems:
        hp = header_problem(text)
        if hp:
            problems.append(hp)
    if missing and not quiet:
        print("%s: %d sections with no marker:" % (base, len(missing)))
    for k in missing:
        s = w.sections[k]
        what = s.is_def_of or ("@p" if s.pascal.strip() else "@d")
        note = " macros: " + " ".join(macros_missing[k]) if k in macros_missing else ""
        print("  §%d: %s%s" % (k, what[:70], note))
    for pr in problems:
        print("  " + pr)
    ok = not missing and not problems
    if not quiet:
        print(
            "%s: %d sections, %d ported, %d dropped, %d exempt, %d land in other files, %d missing; %d procedures unfinished: %s"
            % (base, total, len(ported & set(work)), len(set(dropped) & set(work)), exempt, exported, len(missing),
               len(problems), "COMPLETE" if ok else "INCOMPLETE")
        )
    return ok, len(missing), len(problems)


DECL_MODULES = weblib.DECL_MODULES


def parse_ranges(s):
    out = []
    for part in s.split(","):
        x = part.split("-")
        out.append((int(x[0]), int(x[-1])))
    return out


def cmd_stubs(w, eng):
    defs = w.proc_defs()
    absent = [p for p in defs.values() if rust_proc_name(p.name) not in eng.functions]
    stub = [p for p in defs.values() if rust_proc_name(p.name) in eng.functions
            and re.search(r"\b(todo|unimplemented)!", eng.functions[rust_proc_name(p.name)][1])]
    print("WEB procedures and functions: %d" % len(defs))
    print("with a Rust function: %d" % (len(defs) - len(absent)))
    print("still todo!: %d" % len(stub))
    print("implemented: %d" % (len(defs) - len(absent) - len(stub)))
    for p in absent:
        print("  missing: §%d %s" % (p.sec, p.name))
    return not absent


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    opts = [a for a in sys.argv[1:] if a.startswith("--")]
    strict = "--strict" in opts
    quiet = "--quiet" in opts
    w = weblib.Web()
    eng = Engine()
    if "--stubs" in opts:
        sys.exit(0 if cmd_stubs(w, eng) else 1)
    if "--all" in opts:
        allok = True
        for m in MODULES:
            ranges = [(a, b) for a, b, _ in section_ranges(m)]
            path = os.path.join(SRC, m + ".rs")
            import io
            import contextlib

            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                ok, nm, np = check(w, eng, path, ranges, strict, True, m, True)
            print("%-10s %4d sections missing, %3d procedures unfinished" % (m, nm, np))
            allok = allok and ok
        sys.exit(0 if allok else 1)
    if "--module" in opts:
        i = sys.argv.index("--module")
        m = sys.argv[i + 1]
        ranges = [(a, b) for a, b, _ in section_ranges(m)]
        if not ranges:
            print("no such module: " + m, file=sys.stderr)
            sys.exit(2)
        path = os.path.join(SRC, m + ".rs")
        ok, _, _ = check(w, eng, path, ranges, strict, quiet, m, True)
        sys.exit(0 if ok else 1)
    elif len(args) == 2:
        path = args[0]
        ranges = parse_ranges(args[1])
        m = os.path.splitext(os.path.basename(path))[0]
        ok, _, _ = check(w, eng, path, ranges, strict, quiet, m if m in MODULES else None, False)
        sys.exit(0 if ok else 1)
    print(__doc__)
    sys.exit(2)


if __name__ == "__main__":
    main()
