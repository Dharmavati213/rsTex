"""The module map: which crates/engine/src/*.rs file owns which WEB sections.

Section numbers are weave's numbers in refs/merged/pdftex-merged.web.  The map is a
list of (first, last, module); every section 1..1933 belongs to exactly one module.
"""

# (first section, last section, module, what)
SECTION_MAP = [
    (1, 37, "print", "Parts 1-3: introduction, character set, input and output"),
    (38, 53, "strings", "Part 4: string handling"),
    (54, 71, "print", "Part 5: on-line and off-line printing"),
    (72, 98, "errors", "Part 6: reporting errors"),
    (99, 127, "arith", "Parts 7, 7b: arithmetic with scaled dimensions, random numbers"),
    (128, 150, "mem", "Parts 8-9: packed data, dynamic memory allocation"),
    (151, 179, "nodes", "Part 10: data structures for boxes and their friends"),
    (180, 190, "mem", "Part 11: memory layout"),
    (191, 216, "show", "Part 12: displaying boxes"),
    (217, 224, "copy", "Parts 13-14: destroying and copying boxes"),
    (225, 228, "eqtb", "Part 15: the command codes"),
    (229, 237, "nest", "Part 16: the semantic nest"),
    (238, 273, "eqtb", "Part 17: the table of equivalents"),
    (274, 289, "hash", "Part 18: the hash table"),
    (290, 310, "save", "Part 19: saving and restoring equivalents"),
    (311, 318, "toks", "Part 20: token lists"),
    (319, 353, "input", "Parts 21-23: syntactic routines, input stacks and states"),
    (354, 387, "getnext", "Part 24: getting the next token"),
    (388, 413, "expand", "Part 25: expanding the next token, before macro_call"),
    (414, 427, "macrocall", "Part 25: macro_call"),
    (428, 489, "scan", "Part 26: basic scanning subroutines"),
    (490, 512, "buildtoks", "Part 27: building token lists"),
    (513, 536, "cond", "Part 28: conditional processing"),
    (537, 564, "files", "Part 29: file names"),
    (565, 609, "tfm", "Part 30: font metric data"),
    (610, 671, "dvi", "Parts 31-32: device-independent file format, shipping pages out"),
    (672, 693, "pdfout", "Parts 32a-c: pdfTeX basic, output subroutines, page description"),
    (694, 702, "pdfobj", "Part 32d: the cross-reference table"),
    (703, 726, "pdffont", "Part 32e: font processing"),
    (727, 815, "pdfship", "Part 32f: PDF shipping out"),
    (816, 855, "pack", "Part 33: packaging"),
    (856, 874, "mathdata", "Part 34: data structures for math mode"),
    (875, 943, "mlist", "Parts 35-36: subroutines for math mode, typesetting math formulas"),
    (944, 988, "align", "Part 37: alignment"),
    (989, 1067, "linebreak", "Parts 38-39: breaking paragraphs into lines"),
    (1068, 1143, "hyph", "Parts 40-43: hyphenation"),
    (1144, 1205, "page", "Parts 44-45: breaking vertical lists into pages, the page builder"),
    (1206, 1232, "control", "Part 46: the chief executive"),
    (1233, 1313, "build", "Part 47: building boxes and lists"),
    (1314, 1385, "mathbuild", "Part 48: building math lists"),
    (1386, 1476, "prefixed", "Part 49: mode-independent processing"),
    (1477, 1509, "image", "Part 50: dumping and undumping the tables"),
    (1510, 1519, "main", "Parts 51-52: the main program, debugging"),
    (1520, 1647, "ext", "Part 53: extensions"),
    (1648, 1866, "etex", "Part 53a: the extended features of e-TeX"),
    (1867, 1933, "web2c", "Parts 54, 55: web2c, string recycling, MLTeX, SyncTeX, index"),
]

MODULES = []
for _a, _b, _m, _w in SECTION_MAP:
    if _m not in MODULES:
        MODULES.append(_m)


def module_of(sec):
    for a, b, m, _ in SECTION_MAP:
        if a <= sec <= b:
            return m
    raise ValueError("no module for section %d" % sec)


def section_ranges(module):
    return [(a, b, what) for a, b, m, what in SECTION_MAP if m == module]
