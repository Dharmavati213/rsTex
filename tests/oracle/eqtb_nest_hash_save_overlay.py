#!/usr/bin/env python3
"""Runs the `#[ignore]`d unit tests of eqtb.rs, nest.rs, hash.rs and save.rs before the routines they need exist.

Run with python3 -I.  print_overlay.py does this for the token machine.  This one takes its bodies, drops
those of hash.rs and eqtb.rs, which are real now, and adds what these tests need: literal ports of §45,
§46, §91, §103, §106, §138, §143-§148, §195, §196, §219, §1162, §1661, §1662, §1679-§1681 and §1774, and
test doubles for show_box (no node lists: the tables leave those lines out) and flush_node_list (the empty
list only).  It works on a scratch copy of the tree; nothing in the real tree changes.

    eqtb_nest_hash_save_overlay.py SCRATCH_DIR [cargo test filters]

With OVERLAY_CHECKED=1 in the environment it builds with --features checked.

Remove this script with print_overlay.py, and the `#[ignore]`s, once the routines are merged.  At that point
hold on to one thing: the tests of an overflow panic only here, since the real `overflow` ends the process.
"""
import os
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True
import print_overlay  # noqa: E402

ROOT = print_overlay.ROOT

BODIES = {k: dict(v) for k, v in print_overlay.BODIES.items() if k not in ("hash.rs", "eqtb.rs")}

BODIES["strings.rs"].update({
    "str_eq_buf": r"""
        let mut k = k;
        let mut j = self.str_start[s];
        while j < self.str_start[s + 1] {
            if self.str_pool.at(j) != self.buffer.at(k) { return false; }
            j += 1;
            k += 1;
        }
        true""",
    "str_eq_str": r"""
        if self.length(s) != self.length(t) { return false; }
        let mut j = self.str_start[s];
        let mut k = self.str_start[t];
        while j < self.str_start[s + 1] {
            if self.str_pool.at(j) != self.str_pool.at(k) { return false; }
            j += 1;
            k += 1;
        }
        true""",
})

BODIES["errors.rs"].update({
    "int_error": r"""
        self.print_s(b" (");
        self.print_int(n);
        self.print_char(b')' as i32);
        self.error();""",
})

BODIES["arith.rs"] = {
    "print_scaled": r"""
        let mut s = s;
        if s < 0 {
            self.print_char(b'-' as i32);
            s = -s;
        }
        self.print_int(s / UNITY);
        self.print_char(b'.' as i32);
        s = 10 * (s % UNITY) + 5;
        let mut delta = 10;
        loop {
            if delta > UNITY { s = s + 0o100000 - 50000; }
            self.print_char(b'0' as i32 + (s / UNITY));
            s = 10 * (s % UNITY);
            delta = delta * 10;
            if s <= delta { break; }
        }""",
    "x_over_n": r"""
        let (mut x, mut n) = (x, n);
        let mut negative = false;
        let r;
        if n == 0 {
            self.arith_error = true;
            self.remainder = x;
            return 0;
        }
        if n < 0 {
            x = -x;
            n = -n;
            negative = true;
        }
        if x >= 0 {
            r = x / n;
            self.remainder = x % n;
        } else {
            r = -((-x) / n);
            self.remainder = -((-x) % n);
        }
        if negative { self.remainder = -self.remainder; }
        r""",
}

BODIES["show.rs"] = {
    "print_glue": r"""
        let mut order = order;
        self.print_scaled(d);
        if order < NORMAL || order > FILLL {
            self.print_s(b"foul");
        } else if order > NORMAL {
            self.print_s(b"fil");
            while order > FIL {
                self.print_char(b'l' as i32);
                order -= 1;
            }
        } else if !s.is_empty() {
            self.print_s(s);
        }""",
    "print_spec": r"""
        if p < MEM_MIN || p >= self.lo_mem_max {
            self.print_char(b'*' as i32);
        } else {
            self.print_scaled(self.width(p));
            if !s.is_empty() { self.print_s(s); }
            if self.stretch(p) != 0 {
                self.print_s(b" plus ");
                self.print_glue(self.stretch(p), self.stretch_order(p), s);
            }
            if self.shrink(p) != 0 {
                self.print_s(b" minus ");
                self.print_glue(self.shrink(p), self.shrink_order(p), s);
            }
        }""",
    # A test double: the tables leave out the lines of node lists.
    "show_box": r"""
        let _ = p;
        self.print_ln();""",
}

BODIES["mem.rs"] = {
    "get_avail": r"""
        let mut p = self.avail;
        if p != NULL {
            self.avail = self.link(self.avail);
        } else {
            self.hi_mem_min -= 1;
            p = self.hi_mem_min;
            if self.hi_mem_min < MEM_SPLIT {
                self.overflow(b"main memory size", MEM_MAX + 1 - MEM_MIN);
            }
        }
        self.set_link(p, NULL);
        self.dyn_used += 1;
        p""",
    "get_node": r"""
        'restart: loop {
            let mut p = self.rover;
            let r;
            'found: {
                loop {
                    let mut q = p + self.node_size(p);
                    while self.is_empty(q) {
                        let t = self.rlink(q);
                        if q == self.rover { self.rover = t; }
                        self.set_llink(t, self.llink(q));
                        self.set_rlink(self.llink(q), t);
                        q = q + self.node_size(q);
                    }
                    let c = q - s;
                    if c > p + 1 {
                        self.set_node_size(p, c - p);
                        self.rover = p;
                        r = c;
                        break 'found;
                    }
                    if c == p && self.rlink(p) != p {
                        self.rover = self.rlink(p);
                        let t = self.llink(p);
                        self.set_llink(self.rover, t);
                        self.set_rlink(t, self.rover);
                        r = c;
                        break 'found;
                    }
                    self.set_node_size(p, q - p);
                    p = self.rlink(p);
                    if p == self.rover { break; }
                }
                if self.lo_mem_max + 2 < MEM_SPLIT {
                    let t = if MEM_SPLIT - self.lo_mem_max >= 1998 { self.lo_mem_max + 1000 } else { self.lo_mem_max + 1 + (MEM_SPLIT - self.lo_mem_max) / 2 };
                    let p = self.llink(self.rover);
                    let q = self.lo_mem_max;
                    self.set_rlink(p, q);
                    self.set_llink(self.rover, q);
                    self.set_rlink(q, self.rover);
                    self.set_llink(q, p);
                    self.set_link(q, EMPTY_FLAG);
                    self.set_node_size(q, t - self.lo_mem_max);
                    self.lo_mem_max = t;
                    self.set_link(self.lo_mem_max, NULL);
                    self.set_info(self.lo_mem_max, NULL);
                    self.rover = q;
                    continue 'restart;
                }
                self.overflow(b"main memory size", MEM_MAX + 1 - MEM_MIN);
            }
            self.set_link(r, NULL);
            self.var_used += s;
            return r;
        }""",
    "free_node": r"""
        self.set_node_size(p, s);
        self.set_link(p, EMPTY_FLAG);
        let q = self.llink(self.rover);
        self.set_llink(p, q);
        self.set_rlink(p, self.rover);
        self.set_llink(self.rover, p);
        self.set_rlink(q, p);
        self.var_used -= s;""",
}

BODIES["copy.rs"] = {
    "delete_glue_ref": "self.fast_delete_glue_ref(p);",
    # A test double: only the empty list.
    "flush_node_list": r'''if p != NULL { panic!("flush_node_list of a real list needs nodes.rs"); }''',
}

BODIES["page.rs"] = {
    "print_totals": r"""
        self.print_scaled(self.page_total());
        for (i, s) in [(2, &b""[..]), (3, &b"fil"[..]), (4, &b"fill"[..]), (5, &b"filll"[..])] {
            if self.page_so_far[i] != 0 {
                self.print_s(b" plus ");
                self.print_scaled(self.page_so_far[i]);
                self.print_s(s);
            }
        }
        if self.page_shrink() != 0 {
            self.print_s(b" minus ");
            self.print_scaled(self.page_shrink());
        }""",
}

BODIES["etex.rs"] = {
    "print_group": r"""
        match self.cur_group {
            BOTTOM_LEVEL => { self.print_s(b"bottom level"); return; }
            SIMPLE_GROUP | SEMI_SIMPLE_GROUP => {
                if self.cur_group == SEMI_SIMPLE_GROUP { self.print_s(b"semi "); }
                self.print_s(b"simple");
            }
            HBOX_GROUP | ADJUSTED_HBOX_GROUP => {
                if self.cur_group == ADJUSTED_HBOX_GROUP { self.print_s(b"adjusted "); }
                self.print_s(b"hbox");
            }
            VBOX_GROUP => self.print_s(b"vbox"),
            VTOP_GROUP => self.print_s(b"vtop"),
            ALIGN_GROUP | NO_ALIGN_GROUP => {
                if self.cur_group == NO_ALIGN_GROUP { self.print_s(b"no "); }
                self.print_s(b"align");
            }
            OUTPUT_GROUP => self.print_s(b"output"),
            DISC_GROUP => self.print_s(b"disc"),
            INSERT_GROUP => self.print_s(b"insert"),
            VCENTER_GROUP => self.print_s(b"vcenter"),
            MATH_GROUP | MATH_CHOICE_GROUP | MATH_SHIFT_GROUP | MATH_LEFT_GROUP => {
                self.print_s(b"math");
                if self.cur_group == MATH_CHOICE_GROUP { self.print_s(b" choice"); }
                else if self.cur_group == MATH_SHIFT_GROUP { self.print_s(b" shift"); }
                else if self.cur_group == MATH_LEFT_GROUP { self.print_s(b" left"); }
            }
            _ => {}
        }
        self.print_s(b" group (level ");
        self.print_int(self.cur_level);
        self.print_char(b')' as i32);
        if self.saved(-1) != 0 {
            if e { self.print_s(b" entered at line "); } else { self.print_s(b" at line "); }
            self.print_int(self.saved(-1));
        }""",
    "group_trace": r"""
        self.begin_diagnostic();
        self.print_char(b'{' as i32);
        if e { self.print_s(b"leaving "); } else { self.print_s(b"entering "); }
        self.print_group(e);
        self.print_char(b'}' as i32);
        self.end_diagnostic(false);""",
    "group_warning": r"""
        self.base_ptr = self.input_ptr;
        self.input_stack[self.base_ptr] = self.cur_input;
        let mut i = self.in_open;
        let mut w = false;
        while self.grp_stack[i] == self.cur_boundary && i > 0 {
            if self.tracing_nesting() > 0 {
                while self.input_stack[self.base_ptr].state_field == TOKEN_LIST || self.input_stack[self.base_ptr].index_field > i {
                    self.base_ptr -= 1;
                }
                if self.input_stack[self.base_ptr].name_field > 17 { w = true; }
            }
            self.grp_stack[i] = self.save_index(self.save_ptr);
            i -= 1;
        }
        if w {
            self.print_nl_s(b"Warning: end of ");
            self.print_group(true);
            self.print_s(b" of a different file");
            self.print_ln();
            if self.tracing_nesting() > 1 { self.show_context(); }
            if self.history == SPOTLESS { self.history = WARNING_ISSUED; }
        }""",
    "show_save_groups": r"""
        #[derive(Clone, Copy, PartialEq)]
        enum To { BoxContext, Found1, Found2, Found }
        let mut p = self.nest_ptr;
        self.nest[p] = self.cur_list;
        let (v, l, c) = (self.save_ptr, self.cur_level, self.cur_group);
        self.save_ptr = self.cur_boundary;
        self.cur_level -= 1;
        let mut a = 1;
        self.print_nl_s(b"");
        self.print_ln();
        loop {
            self.print_nl_s(b"### ");
            self.print_group(true);
            if self.cur_group == BOTTOM_LEVEL { break; }
            let mut m;
            loop {
                m = self.nest[p].mode_field;
                if p > 0 { p -= 1; } else { m = VMODE; }
                if m != HMODE { break; }
            }
            self.print_s(b" (");
            let mut s: &[u8] = b"";
            let to = match self.cur_group {
                SIMPLE_GROUP => { p += 1; To::Found2 }
                HBOX_GROUP | ADJUSTED_HBOX_GROUP => { s = b"hbox"; To::BoxContext }
                VBOX_GROUP => { s = b"vbox"; To::BoxContext }
                VTOP_GROUP => { s = b"vtop"; To::BoxContext }
                ALIGN_GROUP => {
                    if a == 0 {
                        if m == -VMODE { s = b"halign"; } else { s = b"valign"; }
                        a = 1;
                        To::Found1
                    } else {
                        if a == 1 { self.print_s(b"align entry"); } else { self.print_esc_s(b"cr"); }
                        if p >= a { p -= a; }
                        a = 0;
                        To::Found
                    }
                }
                NO_ALIGN_GROUP => { p += 1; a = -1; self.print_esc_s(b"noalign"); To::Found2 }
                OUTPUT_GROUP => { self.print_esc_s(b"output"); To::Found }
                MATH_GROUP => To::Found2,
                DISC_GROUP | MATH_CHOICE_GROUP => {
                    if self.cur_group == DISC_GROUP { self.print_esc_s(b"discretionary"); }
                    else { self.print_esc_s(b"mathchoice"); }
                    for i in 1..=3 {
                        if i <= self.saved(-2) { self.print_s(b"{}"); }
                    }
                    To::Found2
                }
                INSERT_GROUP => {
                    if self.saved(-2) == 255 { self.print_esc_s(b"vadjust"); }
                    else {
                        self.print_esc_s(b"insert");
                        self.print_int(self.saved(-2));
                    }
                    To::Found2
                }
                VCENTER_GROUP => { s = b"vcenter"; To::Found1 }
                SEMI_SIMPLE_GROUP => { p += 1; self.print_esc_s(b"begingroup"); To::Found }
                MATH_SHIFT_GROUP => {
                    let mut done = false;
                    if m == MMODE {
                        self.print_char(b'$' as i32);
                    } else if self.nest[p].mode_field == MMODE {
                        self.print_cmd_chr(EQ_NO, self.saved(-2));
                        done = true;
                    }
                    if !done { self.print_char(b'$' as i32); }
                    To::Found
                }
                MATH_LEFT_GROUP => {
                    if self.ty(self.nest[p + 1].eTeX_aux_field) == LEFT_NOAD { self.print_esc_s(b"left"); }
                    else { self.print_esc_s(b"middle"); }
                    To::Found
                }
                _ => To::Found,
            };
            if to == To::BoxContext {
                let mut i = self.saved(-4);
                if i != 0 {
                    if i < BOX_FLAG {
                        let j = if abs(self.nest[p].mode_field) == VMODE { HMOVE } else { VMOVE };
                        if i > 0 { self.print_cmd_chr(j, 0); } else { self.print_cmd_chr(j, 1); }
                        self.print_scaled(abs(i));
                        self.print_s(b"pt");
                    } else if i < SHIP_OUT_FLAG {
                        if i >= GLOBAL_BOX_FLAG {
                            self.print_esc_s(b"global");
                            i = i - (GLOBAL_BOX_FLAG - BOX_FLAG);
                        }
                        self.print_esc_s(b"setbox");
                        self.print_int(i - BOX_FLAG);
                        self.print_char(b'=' as i32);
                    } else {
                        self.print_cmd_chr(LEADER_SHIP, i - (LEADER_FLAG - A_LEADERS));
                    }
                }
            }
            if to == To::BoxContext || to == To::Found1 {
                self.print_esc_s(s);
                if self.saved(-2) != 0 {
                    self.print_char(b' ' as i32);
                    if self.saved(-3) == EXACTLY { self.print_s(b"to"); } else { self.print_s(b"spread"); }
                    self.print_scaled(self.saved(-2));
                    self.print_s(b"pt");
                }
            }
            if to != To::Found { self.print_char(b'{' as i32); }
            self.print_char(b')' as i32);
            self.cur_level -= 1;
            self.cur_group = self.save_level(self.save_ptr);
            self.save_ptr = self.save_index(self.save_ptr);
        }
        self.save_ptr = v;
        self.cur_level = l;
        self.cur_group = c;""",
}


def main():
    scratch = sys.argv[1]
    filters = sys.argv[2:] or ["eqtb::", "nest::", "hash::", "save::"]
    copy = os.path.join(scratch, "overlay")
    shutil.rmtree(copy, ignore_errors=True)
    os.makedirs(os.path.join(copy, "tests"))
    shutil.copytree(os.path.join(ROOT, "crates"), os.path.join(copy, "crates"))
    shutil.copy(os.path.join(ROOT, "tests", "primitives.json"), os.path.join(copy, "tests"))
    for f in ("Cargo.toml", "Cargo.lock"):
        shutil.copy(os.path.join(ROOT, f), copy)
    src = os.path.join(copy, "crates", "engine", "src")
    for file, bodies in BODIES.items():
        print_overlay.patch(os.path.join(src, file), bodies)
    target = os.environ.get("CARGO_TARGET_DIR") or os.path.join(ROOT, "target")
    env = dict(os.environ, CARGO_TARGET_DIR=os.path.join(target, "overlay"))
    features = ["--features", "checked"] if os.environ.get("OVERLAY_CHECKED") else []
    cmd = ["cargo", "test", "-p", "rstex-engine", "-j4", "--offline", "--lib"] + features + ["--"] + filters + ["--include-ignored"]
    sys.exit(subprocess.run(cmd, cwd=copy, env=env).returncode)


if __name__ == "__main__":
    main()
