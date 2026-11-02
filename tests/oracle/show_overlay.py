#!/usr/bin/env python3
"""Runs the `#[ignore]`d tests of mem.rs, show.rs and copy.rs before the routines they print with are ported.

Run with python3 -I.  Those tests compare `show_box` with the oracle, and until those modules arrive the routines
that print are stubs.  So this does what print_overlay.py does, on a scratch copy of the tree: it replaces
the stubs there with literal ports, and runs `cargo test --include-ignored`.  It takes print_overlay.py's
bodies, and adds §70, §103, §243, §544, §867-§870 and §1602.  A body whose stub has gone from the tree is
skipped, so the real routine is what runs once its module is merged.

    show_overlay.py SCRATCH_DIR [cargo test filters]

With OVERLAY_CHECKED=1 in the environment it builds with --features checked.  CARGO_TARGET_DIR is honoured,
with `overlay` added to it.  Nothing in the real tree changes.

Remove this script, and the `#[ignore]`s, once M1 has ported the real routines.
"""
import os
import re
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import print_overlay as po  # noqa: E402

ROOT = po.ROOT

MORE = {
    "print.rs": {
        "print_current_string": r"""
        let mut j = self.str_start[self.str_ptr];
        while j < self.pool_ptr { self.print_char(self.str_pool.at(j)); j += 1; }""",
    },
    "arith.rs": {
        "print_scaled": r"""
        let mut s = s;
        if s < 0 { self.print_char(b'-' as i32); s = -s; }
        self.print_int(s / UNITY);
        self.print_char(b'.' as i32);
        s = 10 * (s % UNITY) + 5;
        let mut delta = 10;
        loop {
            if delta > UNITY { s = s + 0o100000 - 50000; }
            self.print_char(b'0' as i32 + s / UNITY);
            s = 10 * (s % UNITY);
            delta = delta * 10;
            if s <= delta { break; }
        }""",
    },
    "eqtb.rs": {
        "print_skip_param": r"""
        match n {
            LINE_SKIP_CODE => self.print_esc_s(b"lineskip"),
            BASELINE_SKIP_CODE => self.print_esc_s(b"baselineskip"),
            PAR_SKIP_CODE => self.print_esc_s(b"parskip"),
            ABOVE_DISPLAY_SKIP_CODE => self.print_esc_s(b"abovedisplayskip"),
            BELOW_DISPLAY_SKIP_CODE => self.print_esc_s(b"belowdisplayskip"),
            ABOVE_DISPLAY_SHORT_SKIP_CODE => self.print_esc_s(b"abovedisplayshortskip"),
            BELOW_DISPLAY_SHORT_SKIP_CODE => self.print_esc_s(b"belowdisplayshortskip"),
            LEFT_SKIP_CODE => self.print_esc_s(b"leftskip"),
            RIGHT_SKIP_CODE => self.print_esc_s(b"rightskip"),
            TOP_SKIP_CODE => self.print_esc_s(b"topskip"),
            SPLIT_TOP_SKIP_CODE => self.print_esc_s(b"splittopskip"),
            TAB_SKIP_CODE => self.print_esc_s(b"tabskip"),
            SPACE_SKIP_CODE => self.print_esc_s(b"spaceskip"),
            XSPACE_SKIP_CODE => self.print_esc_s(b"xspaceskip"),
            PAR_FILL_SKIP_CODE => self.print_esc_s(b"parfillskip"),
            THIN_MU_SKIP_CODE => self.print_esc_s(b"thinmuskip"),
            MED_MU_SKIP_CODE => self.print_esc_s(b"medmuskip"),
            THICK_MU_SKIP_CODE => self.print_esc_s(b"thickmuskip"),
            _ => self.print_s(b"[unknown glue parameter!]"),
        }""",
    },
    "mathdata.rs": {
        "print_style": r"""
        match c / 2 {
            0 => self.print_esc_s(b"displaystyle"),
            1 => self.print_esc_s(b"textstyle"),
            2 => self.print_esc_s(b"scriptstyle"),
            3 => self.print_esc_s(b"scriptscriptstyle"),
            _ => self.print_s(b"Unknown style!"),
        }""",
        "print_delimiter": r"""
        let mut a = self.small_fam(p) * 256 + self.small_char(p);
        a = a * 0x1000 + self.large_fam(p) * 256 + self.large_char(p);
        if a < 0 { self.print_int(a); } else { self.print_hex(a); }""",
        "print_fam_and_char": r"""
        self.print_esc_s(b"fam");
        self.print_int(self.fam(p));
        self.print_char(b' ' as i32);
        self.print(self.character(p));""",
        "print_subsidiary_data": r"""
        if self.cur_length() >= self.depth_threshold {
            if self.math_type(p) != EMPTY { self.print_s(b" []"); }
        } else {
            self.append_char(c);
            self.temp_ptr = p;
            match self.math_type(p) {
                MATH_CHAR => { self.print_ln(); self.print_current_string(); self.print_fam_and_char(p); }
                SUB_BOX => self.show_info(),
                SUB_MLIST => {
                    if self.info(p) == NULL { self.print_ln(); self.print_current_string(); self.print_s(b"{}"); }
                    else { self.show_info(); }
                }
                _ => {}
            }
            self.flush_char();
        }""",
        "show_info": r"""self.show_node_list(self.info(self.temp_ptr));""",
    },
    "ext.rs": {
        "print_write_whatsit": r"""
        self.print_esc_s(s);
        if self.write_stream(p) < 16 { self.print_int(self.write_stream(p)); }
        else if self.write_stream(p) == 16 { self.print_char(b'*' as i32); }
        else { self.print_char(b'-' as i32); }""",
    },
    "files.rs": {
        "print_file_name": r"""
        let mut must_quote = false;
        for s in [a, n, e] {
            if s != 0 {
                let mut j = self.str_start[s];
                while !must_quote && j < self.str_start[s + 1] {
                    must_quote = self.str_pool.at(j) == b' ' as i32;
                    j += 1;
                }
            }
        }
        if must_quote { self.print_char(b'"' as i32); }
        for s in [a, n, e] {
            if s != 0 {
                let mut j = self.str_start[s];
                while j < self.str_start[s + 1] {
                    if self.str_pool.at(j) != b'"' as i32 { self.print(self.str_pool.at(j)); }
                    j += 1;
                }
            }
        }
        if must_quote { self.print_char(b'"' as i32); }""",
    },
}


def patch(path, bodies):
    with open(path, encoding="utf-8") as f:
        s = f.read()
    for name, body in bodies.items():
        pat = re.compile(r'todo!\("§\d+ ' + re.escape(name) + r'"\)')
        s = pat.sub(lambda m: body.strip(), s, count=1)
    with open(path, "w", encoding="utf-8") as f:
        f.write(s)


def main():
    scratch = sys.argv[1]
    filters = sys.argv[2:] or ["mem::", "nodes::", "show::", "copy::"]
    copy = os.path.join(scratch, "overlay")
    shutil.rmtree(os.path.join(copy, "crates"), ignore_errors=True)
    os.makedirs(copy, exist_ok=True)
    shutil.copytree(os.path.join(ROOT, "crates"), os.path.join(copy, "crates"))
    for f in ("Cargo.toml", "Cargo.lock"):
        shutil.copy(os.path.join(ROOT, f), copy)
    src = os.path.join(copy, "crates", "engine", "src")
    for table in (po.BODIES, MORE):
        for file, bodies in table.items():
            patch(os.path.join(src, file), bodies)
    target = os.path.join(os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target")), "overlay")
    env = dict(os.environ, CARGO_TARGET_DIR=target)
    features = ["--features", "checked"] if os.environ.get("OVERLAY_CHECKED") else []
    cmd = ["cargo", "test", "-p", "rstex-engine", "-j4", "--offline", "--lib"] + features + ["--"] + filters + ["--include-ignored"]
    sys.exit(subprocess.run(cmd, cwd=copy, env=env).returncode)


if __name__ == "__main__":
    main()
