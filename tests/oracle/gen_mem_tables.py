#!/usr/bin/env python3
"""The allocator's addresses, read from the oracle's own format file.

Run with python3 -I.  It writes crates/engine/testdata/mem_oracle.tsv.

Nothing the oracle prints says where a node is, but `\\dump` writes `mem`: after §1489's sort_avail, the
file has `lo_mem_max`, `rover`, and every free block of the variable-size memory, with its address and
size.  So each case here is a short sequence of box assignments, run by `pdftex -ini -etex` and dumped.
The test in mem.rs replays the same sequence through `get_node` and `free_node`, and compares.

Each case is a list of operations, in a small notation that the Rust test reads too:

    B3 kp(gk)r   \\setbox3=\\hbox{\\kern1pt \\penalty0 \\hbox{\\hskip1pt \\kern1pt }\\vrule}
    C2 3         \\setbox2=\\copy3
    V3           \\setbox3=\\box255, which voids box 3: box 255 is void throughout

These do nothing else in the variable-size memory, in this order: `k` is new_kern, 4 words; `p` is new_penalty, 4
words; `g` is new_spec(zero_glue) and new_glue, 4 words each; `r` is new_rule, 6 words; `(...)` is the
contents and then hpack's get_node(box_node_size), 9 words.  A list's tokens and its nest head are in
hi memory.  An assignment flushes the old contents of the register last, in eq_destroy.

Format: name, operations, lo_mem_max, rover, var_used, then the free blocks as `address:size`, all
fields separated by a tab.  Addresses are the oracle's, whose mem_bot is 0; the port's mem_bot is 1,
so its addresses are one more.
"""
import gzip
import os
import random
import struct
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "engine", "testdata")

PREAMBLE = "\\catcode`\\{=1 \\catcode`\\}=2 \\scrollmode\n"
ITEM = {"k": "\\kern1pt ", "p": "\\penalty0 ", "g": "\\hskip1pt\\relax ", "r": "\\vrule\\relax ", "(": "\\hbox{", ")": "}"}


def to_tex(ops):
    out = []
    for op in ops.split(";"):
        if op[0] == "B":
            reg, _, items = op[1:].partition(" ")
            out.append("\\setbox%s=\\hbox{%s}" % (reg, "".join(ITEM[c] for c in items)))
        elif op[0] == "C":
            dst, _, src = op[1:].partition(" ")
            out.append("\\setbox%s=\\copy%s" % (dst, src))
        elif op[0] == "V":
            out.append("\\setbox%s=\\box255" % op[1:])
        else:
            raise ValueError(op)
    return "\n".join(out)


def read_fmt(path):
    """§1485-§1490 of the oracle's format.  Every dump_int is 8 bytes and a memory word 16: lh, then rh."""
    with gzip.open(path, "rb") as f:
        d = f.read()
    pos = [0]

    def i64():
        v = struct.unpack_from("<q", d, pos[0])[0]
        pos[0] += 8
        return v

    if i64() != 0x57325458:
        sys.exit("not a format file: " + path)
    n = i64()            # the engine's name
    pos[0] += n
    i64()                # the pool's checksum
    pos[0] += 3 * 256    # xord, xchr, xprn
    max_halfword, _hash_high, etex_mode, mem_bot, _mem_top, _eqtb, _hp, _hyp = [i64() for _ in range(8)]
    if (max_halfword, etex_mode, mem_bot) != (0xFFFFFFF, 1, 0):
        sys.exit("unexpected format header: %r" % ((max_halfword, etex_mode, mem_bot),))
    if i64() != 0x4D4C5458 or i64() != 0:   # MLTeX, off
        sys.exit("unexpected MLTeX data")
    if i64() != 0x45435458 or i64() != 0:   # encTeX, off
        sys.exit("unexpected encTeX data")
    pool_ptr, str_ptr = i64(), i64()
    pos[0] += 8 * (str_ptr + 1) + pool_ptr
    lo_mem_max, rover = i64(), i64()
    pos[0] += 8 * 6      # sa_root[int_val..tok_val]

    def word(k):
        lh, rh = struct.unpack_from("<qq", d, pos[0] + 16 * k)
        return lh, rh

    # §1489: p:=mem_bot; q:=rover; repeat dump mem[p..q+1]; p:=q+node_size(q); q:=rlink(q); until q=rover
    free = []
    var_used = 0
    p, q = mem_bot, rover
    while True:
        n = q + 2 - p
        size, flag = word(q - p)
        _llink, rlink = word(q + 1 - p)
        if flag != max_halfword:
            sys.exit("a free block without its flag at %d" % q)
        free.append((q, size))
        var_used += q - p
        pos[0] += 16 * n
        p, q = q + size, rlink
        if q == rover:
            break
    var_used += lo_mem_max - p
    pos[0] += 16 * (lo_mem_max + 1 - p)
    hi_mem_min, avail = i64(), i64()
    pos[0] += 16 * (_mem_top + 1 - hi_mem_min)
    var2, dyn_used = i64(), i64()
    if var2 != var_used:
        sys.exit("§1489's var_used %d, mine %d" % (var2, var_used))
    return {"lo_mem_max": lo_mem_max, "rover": rover, "var_used": var_used, "dyn_used": dyn_used, "free": free}


def run_dump(body, tmp, job="t", preamble=PREAMBLE):
    """Runs body and \\dump.  Returns the statistics of the format, and the log."""
    path = os.path.join(tmp, job + ".tex")
    with open(path, "wb") as f:
        f.write((preamble + body + "\n\\dump\n").encode("latin-1"))
    fmt = os.path.join(tmp, job + ".fmt")
    if os.path.exists(fmt):
        os.unlink(fmt)
    subprocess.run(["pdftex", "-ini", "-etex", "-interaction=nonstopmode", job + ".tex"], cwd=tmp,
                   stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
    with open(os.path.join(tmp, job + ".log"), "rb") as f:
        log = f.read().decode("latin-1")
    if "\n! " in log or not os.path.exists(fmt):
        sys.exit("the oracle complained; see %s\n%s" % (tmp, log[-1500:]))
    return read_fmt(fmt), log


def random_items(rng, depth, n):
    out = []
    for _ in range(rng.randint(0, n)):
        c = rng.choice("kkkppggrr(")
        if c == "(" and depth < 3:
            out.append("(" + random_items(rng, depth + 1, n // 2) + ")")
        elif c != "(":
            out.append(c)
    return "".join(out)


def random_ops(seed, count, regs, n):
    rng = random.Random(seed)
    ops = []
    for _ in range(count):
        r = rng.random()
        a = rng.randrange(regs)
        if r < 0.6:
            ops.append("B%d %s" % (a, random_items(rng, 0, n)))
        elif r < 0.8:
            ops.append("C%d %d" % (a, rng.randrange(regs)))
        else:
            ops.append("V%d" % a)
    return ";".join(ops)


CASES = [
    ("one_kern", "B1 k"),
    ("two_kerns_a_box", "B1 kp"),
    ("empty_box", "B1 "),
    ("a_rule", "B1 r"),
    ("glue", "B1 g"),
    ("replaced", "B1 kp;B1 k"),
    ("voided", "B1 kpg;V1"),
    ("freed_and_reused_exactly", "B1 k;B2 k;V1;B3 "),
    ("reused_for_the_same_size", "B1 kp;B2 r;V1;B1 kp"),
    ("freed_in_the_other_order", "B1 k;B2 p;B3 g;V1;V3;V2"),
    ("merge_on_the_next_request", "B1 kkkk;B2 pppp;V1;V2;B3 r"),
    ("one_word_cannot_be_left", "B1 r;V1;B2 k;B3 k"),
    ("nested", "B1 k(p(g)k)r"),
    ("copied", "B1 k(p)g;C2 1"),
    ("copied_and_voided", "B1 k(p)g;C2 1;V1"),
    ("copied_and_all_voided", "B1 k(p)g;C2 1;V1;V2"),
    ("copy_of_void", "C2 1"),
    ("copy_over_itself", "B1 kk;C1 1"),
    ("exactly_all_of_the_first_block", "B1 " + "k" * 247),
    ("more_than_the_first_block", "B1 " + "k" * 300),
    ("two_blocks", "B1 " + "k" * 240 + ";B2 " + "p" * 240),
    ("grown_and_freed", "B1 " + "k" * 300 + ";V1"),
    ("grown_twice_with_holes", "B1 " + "kr" * 300 + ";B2 " + "g" * 250 + ";V1;B3 " + "pr" * 100),
    ("rule_too_big_for_every_hole", "B1 kkkk;B2 pppp;B3 k;V1;B4 r;B5 r;B6 r"),
]
for _seed, _count, _regs, _n in [(1, 40, 4, 6), (2, 60, 8, 8), (3, 80, 6, 12), (4, 120, 8, 24), (5, 200, 16, 40), (6, 300, 12, 60)]:
    CASES.append(("random%d" % _seed, random_ops(_seed, _count, _regs, _n)))


def main():
    os.makedirs(OUT, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp, open(os.path.join(OUT, "mem_oracle.tsv"), "w") as f:
        for name, ops in CASES:
            s, _ = run_dump(to_tex(ops), tmp)
            f.write("%s\t%s\t%d\t%d\t%d\t%s\n" % (name, ops, s["lo_mem_max"], s["rover"], s["var_used"],
                                                 " ".join("%d:%d" % x for x in s["free"])))
    print("mem:", len(CASES), "cases")


if __name__ == "__main__":
    main()
