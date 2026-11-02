//! §229-237 Part 16: the semantic nest.
#![forbid(unsafe_code)]
#![deny(unreachable_patterns, unused_variables)]
#![allow(non_snake_case, unused_mut, unused_imports, dead_code)]

use crate::access::*;
use crate::consts::*;
use crate::host::*;
use crate::raw::*;
use crate::state::*;

crate::pod_struct! {
    /// POD state private to this module.  Add fields here, not to state.rs.
    pub struct Vars {}
}

/// Non-POD state private to this module, in `self.host`.  Rarely needed.
#[derive(Default)]
pub struct HVars {}

impl Tex {
    /// §229
    pub fn print_mode(&mut self, m: i32) {
        if m > 0 {
            match m / (MAX_COMMAND + 1) {
                0 => self.print_s(b"vertical mode"),
                1 => self.print_s(b"horizontal mode"),
                2 => self.print_s(b"display math mode"),
                _ => {}
            }
        } else if m == 0 {
            self.print_s(b"no mode");
        } else {
            match (-m) / (MAX_COMMAND + 1) {
                0 => self.print_s(b"internal vertical mode"),
                1 => self.print_s(b"restricted horizontal mode"),
                2 => self.print_s(b"math mode"),
                _ => {}
            }
        }
    }

    /// §229
    pub fn print_in_mode(&mut self, m: i32) {
        if m > 0 {
            match m / (MAX_COMMAND + 1) {
                0 => self.print_s(b"' in vertical mode"),
                1 => self.print_s(b"' in horizontal mode"),
                2 => self.print_s(b"' in display math mode"),
                _ => {}
            }
        } else if m == 0 {
            self.print_s(b"' in no mode");
        } else {
            match (-m) / (MAX_COMMAND + 1) {
                0 => self.print_s(b"' in internal vertical mode"),
                1 => self.print_s(b"' in restricted horizontal mode"),
                2 => self.print_s(b"' in math mode"),
                _ => {}
            }
        }
    }

    /// §234
    pub fn push_nest(&mut self) {
        if self.nest_ptr > self.max_nest_stack {
            self.max_nest_stack = self.nest_ptr;
            if self.nest_ptr == self.nest_size {
                self.overflow(b"semantic nest size", self.nest_size);
            }
        }
        self.nest[self.nest_ptr] = self.cur_list;
        self.nest_ptr += 1;
        let h = self.get_avail();
        self.set_head(h);
        self.set_tail(h);
        self.set_prev_graf(0);
        self.set_mode_line(self.line);
        self.set_eTeX_aux(NULL);
    }

    /// §235
    pub fn pop_nest(&mut self) {
        self.free_avail(self.head());
        self.nest_ptr -= 1;
        self.cur_list = self.nest[self.nest_ptr];
    }

    /// §236
    pub fn show_activities(&mut self) {
        self.nest[self.nest_ptr] = self.cur_list;
        self.print_nl_s(b"");
        self.print_ln();
        for p in (0..=self.nest_ptr).rev() {
            let m = self.nest[p].mode_field;
            let a = self.nest[p].aux_field;
            self.print_nl_s(b"### ");
            self.print_mode(m);
            self.print_s(b" entered at line ");
            self.print_int(abs(self.nest[p].ml_field));
            if m == HMODE && self.nest[p].pg_field != 0o40600000 {
                self.print_s(b" (language");
                self.print_int(self.nest[p].pg_field % 0o200000);
                self.print_s(b":hyphenmin");
                self.print_int(self.nest[p].pg_field / 0o20000000);
                self.print_char(b',' as i32);
                self.print_int((self.nest[p].pg_field / 0o200000) % 0o100);
                self.print_char(b')' as i32);
            }
            if self.nest[p].ml_field < 0 {
                self.print_s(b" (\\output routine)");
            }
            if p == 0 {
                // §1163 Show the status of the current page
                if PAGE_HEAD != self.page_tail {
                    self.print_nl_s(b"### current page:");
                    if self.output_active {
                        self.print_s(b" (held over for next output)");
                    }
                    self.show_box(self.link(PAGE_HEAD));
                    if self.page_contents > EMPTY {
                        self.print_nl_s(b"total height ");
                        self.print_totals();
                        self.print_nl_s(b" goal height ");
                        self.print_scaled(self.page_goal());
                        let mut r = self.link(PAGE_INS_HEAD);
                        while r != PAGE_INS_HEAD {
                            self.print_ln();
                            self.print_esc_s(b"insert");
                            let mut t = self.subtype(r);
                            self.print_int(t);
                            self.print_s(b" adds ");
                            if self.count(t) == 1000 {
                                t = self.height(r);
                            } else {
                                // PORTING section 11: the product can leave the range.
                                let x = self.x_over_n(self.height(r), 1000);
                                t = sat32(x as i64 * self.count(t) as i64);
                            }
                            self.print_scaled(t);
                            if self.ty(r) == SPLIT_UP {
                                let mut q = PAGE_HEAD;
                                t = 0;
                                loop {
                                    q = self.link(q);
                                    if self.ty(q) == INS_NODE && self.subtype(q) == self.subtype(r) {
                                        t += 1;
                                    }
                                    if q == self.broken_ins(r) {
                                        break;
                                    }
                                }
                                self.print_s(b", #");
                                self.print_int(t);
                                self.print_s(b" might split");
                            }
                            r = self.link(r);
                        }
                    }
                }
                if self.link(CONTRIB_HEAD) != NULL {
                    self.print_nl_s(b"### recent contributions:");
                }
            }
            self.show_box(self.link(self.nest[p].head_field));
            // §237 Show the auxiliary field, |a|
            match abs(m) / (MAX_COMMAND + 1) {
                0 => {
                    self.print_nl_s(b"prevdepth ");
                    if a.sc() <= self.pdf_ignored_dimen() {
                        self.print_s(b"ignored");
                    } else {
                        self.print_scaled(a.sc());
                    }
                    if self.nest[p].pg_field != 0 {
                        self.print_s(b", prevgraf ");
                        self.print_int(self.nest[p].pg_field);
                        if self.nest[p].pg_field != 1 {
                            self.print_s(b" lines");
                        } else {
                            self.print_s(b" line");
                        }
                    }
                }
                1 => {
                    self.print_nl_s(b"spacefactor ");
                    self.print_int(a.lh());
                    if m > 0 && a.rh() > 0 {
                        self.print_s(b", current language ");
                        self.print_int(a.rh());
                    }
                }
                2 => {
                    if a.int() != NULL {
                        self.print_s(b"this will begin denominator of:");
                        self.show_box(a.int());
                    }
                }
                _ => {} // there are no other cases
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::testkit::*;
    use crate::toks::testkit::*;

    #[test]
    fn pop_nest_gives_back_the_list_and_its_head() {
        let t = table_machine();
        let mut saved = t.cur_list;
        saved.mode_field = -HMODE;
        saved.pg_field = 7;
        t.nest[0] = saved;
        t.nest_ptr = 1;
        t.set_head(MEM_TOP - 100);
        t.dyn_used = 1;
        t.pop_nest();
        assert_eq!(t.nest_ptr, 0);
        assert_eq!((t.mode(), t.prev_graf()), (-HMODE, 7));
        assert_eq!(t.avail, MEM_TOP - 100, "the head goes back on the avail stack");
        assert_eq!(t.dyn_used, 0);
        Tex::dealloc(t);
    }

    #[test]
    #[ignore = "needs get_avail in mem.rs"]
    fn push_nest_starts_an_empty_list() {
        let t = table_machine();
        t.line = 12;
        t.set_mode(HMODE);
        t.set_eTeX_aux(99);
        t.set_prev_graf(3);
        t.set_space_factor(777);
        let outer = t.cur_list;
        t.push_nest();
        assert_eq!(t.nest_ptr, 1);
        assert_eq!(t.max_nest_stack, 0, "only a level above the old high-water mark counts");
        assert_eq!(t.nest[0].mode_field, HMODE);
        assert_eq!(t.nest[0].pg_field, 3);
        assert_eq!(t.nest[0].eTeX_aux_field, 99);
        assert_eq!(t.head(), t.tail());
        assert_eq!(t.link(t.head()), NULL);
        assert_eq!((t.prev_graf(), t.mode_line(), t.eTeX_aux(), t.mode()), (0, 12, NULL, HMODE));
        assert_eq!(t.space_factor(), 777, "mode and aux stay until the caller sets them");
        t.push_nest();
        assert_eq!(t.max_nest_stack, 1);
        t.pop_nest();
        t.pop_nest();
        assert_eq!((t.nest_ptr, t.cur_list.head_field, t.cur_list.pg_field), (0, outer.head_field, outer.pg_field));
        Tex::dealloc(t);
    }

    /// The oracle stopped at this many, for this `nest_size`.
    #[test]
    #[ignore = "needs get_avail in mem.rs; reaches overflow, in errors.rs, whose real form ends the process"]
    fn the_nest_stops_where_the_oracle_does() {
        let rows = table(include_str!("../testdata/save_limits_oracle.tsv"));
        let r = rows.iter().find(|r| r[0] == "semantic nest size").unwrap();
        let (limit, steps): (i32, i32) = (r[1].parse().unwrap(), r[2].parse().unwrap());
        let t = table_machine();
        t.nest_size = limit;
        for _ in 0..steps {
            t.push_nest();
        }
        assert_overflow(&panic_message(|| t.push_nest()), "semantic nest size", limit);
    }

    #[test]
    #[ignore = "needs print.rs and strings.rs"]
    fn print_mode_prints_every_mode() {
        let t = machine();
        print_setup(t);
        // What \showlists prints; the oracle has no way to reach "no mode".
        for (m, want) in [
            (VMODE, "vertical mode"),
            (HMODE, "horizontal mode"),
            (MMODE, "display math mode"),
            (0, "no mode"),
            (-VMODE, "internal vertical mode"),
            (-HMODE, "restricted horizontal mode"),
            (-MMODE, "math mode"),
        ] {
            t.print_mode(m);
            assert_eq!(printed(t), want);
        }
        t.print_in_mode(0);
        assert_eq!(printed(t), "' in no mode");
        Tex::dealloc(t);
    }

    #[test]
    #[ignore = "needs print.rs and strings.rs"]
    fn print_in_mode_against_the_oracle() {
        let t = machine();
        print_setup(t);
        for r in table(include_str!("../testdata/nest_in_mode_oracle.tsv")) {
            let m = match r[0].as_str() {
                "vmode" => VMODE,
                "hmode" => HMODE,
                "mmode" => MMODE,
                "-vmode" => -VMODE,
                "-hmode" => -HMODE,
                _ => -MMODE,
            };
            t.print_in_mode(m);
            assert_eq!(printed(t), r[1]);
        }
        Tex::dealloc(t);
    }

    /// The lines of `\showlists` that are this module's.  What `show_box` prints is show.rs's, so the
    /// tables leave it out, and so does this when it is real.
    fn showlists_lines(text: &str) -> String {
        let keep = |l: &&str| {
            l.starts_with("### ")
                || l.starts_with("prevdepth ")
                || l.starts_with("spacefactor ")
                || *l == "this will begin denominator of:"
                || l.starts_with("total height ")
                || l.starts_with(" goal height ")
                || (l.starts_with("\\insert") && l.contains(" adds "))
        };
        text.lines().filter(keep).collect::<Vec<_>>().join("\n")
    }

    fn show_lists(t: &mut Tex) -> String {
        t.begin_diagnostic();
        t.show_activities();
        t.end_diagnostic(true);
        showlists_lines(&printed(t))
    }

    fn rule(t: &mut Tex) -> i32 {
        let p = t.get_node(RULE_NODE_SIZE);
        t.set_ty(p, RULE_NODE);
        t.set_subtype(p, 0);
        t.set_width(p, NULL_FLAG);
        t.set_depth(p, 0);
        t.set_height(p, 65536);
        p
    }

    /// `\showlists` in each mode, with the semantic nest built as the commands would.
    #[test]
    #[ignore = "needs print.rs, strings.rs, arith.rs, show.rs, mem.rs, page.rs"]
    fn showlists_against_the_oracle() {
        for r in table(include_str!("../testdata/nest_oracle.tsv")) {
            let t = table_machine();
            print_setup(t);
            let first: i32 = r[0].parse().unwrap();
            let mut got = String::new();
            for (i, op) in r[1].split(';').enumerate() {
                t.line = first + i as i32;
                let f: Vec<&str> = op.split(' ').collect();
                match f[0] {
                    "S" => got = show_lists(t),
                    "w" => t.eq_word_define(word_addr(f[1], f[2].parse().unwrap()), f[3].parse().unwrap()),
                    "prevdepth" => t.set_prev_depth(f[1].parse().unwrap()),
                    "prevgraf" => t.set_prev_graf(f[1].parse().unwrap()),
                    "spacefactor" => t.set_space_factor(f[1].parse().unwrap()),
                    "over" => {
                        // what math_fraction leaves, with both lists empty
                        let p = t.get_node(FRACTION_NOAD_SIZE);
                        for k in 1..FRACTION_NOAD_SIZE {
                            t.mem[p + k] = Word::ZERO;
                        }
                        t.set_ty(p, FRACTION_NOAD);
                        t.set_subtype(p, NORMAL);
                        t.set_thickness(p, DEFAULT_CODE);
                        t.set_incompleat_noad(p);
                    }
                    "$$" => {
                        // An empty paragraph with its indentation goes to the page first.
                        let q = rule(t);
                        t.set_link(PAGE_HEAD, q);
                        t.page_tail = q;
                        t.page_contents = BOX_THERE;
                        t.set_prev_depth(0);
                        t.set_prev_graf(1);
                        group_op(t, op);
                    }
                    _ => group_op(t, op),
                }
            }
            assert_eq!(got, r[2], "operations {}", r[1]);
            Tex::dealloc(t);
        }
    }

    /// The status of the current page: the state the oracle's scenario in the generator reaches.
    #[test]
    #[ignore = "needs print.rs, strings.rs, arith.rs (x_over_n), show.rs, mem.rs, page.rs"]
    fn the_current_page_against_the_oracle() {
        let rows = table(include_str!("../testdata/nest_page_oracle.tsv"));
        let t = table_machine();
        print_setup(t);
        // Three insertions on the page, the third of them split up, and something still to contribute.
        let ins = |t: &mut Tex, n: i32, next: i32| {
            let q = t.get_node(INS_NODE_SIZE);
            t.set_ty(q, INS_NODE);
            t.set_subtype(q, n);
            t.set_height(q, 5 * 65536);
            t.set_depth(q, 0);
            t.set_split_top_ptr(q, ZERO_GLUE);
            t.set_float_cost(q, 0);
            t.set_ins_ptr(q, NULL);
            t.set_link(q, next);
            q
        };
        let page_ins = |t: &mut Tex, n: i32, h: i32, ty: i32, next: i32| {
            let r = t.get_node(PAGE_INS_NODE_SIZE);
            t.set_subtype(r, n);
            t.set_ty(r, ty);
            t.set_height(r, h);
            t.set_link(r, next);
            r
        };
        let i102 = ins(t, 102, NULL);
        let i101 = ins(t, 101, i102);
        let i100 = ins(t, 100, i101);
        t.set_link(PAGE_HEAD, i100);
        t.page_tail = i102;
        t.page_contents = INSERTS_ONLY;
        let r102 = page_ins(t, 102, 65536, SPLIT_UP, PAGE_INS_HEAD);
        t.set_broken_ins(r102, i102);
        let r101 = page_ins(t, 101, 6 * 65536, INSERTING, r102);
        let r100 = page_ins(t, 100, 5 * 65536, INSERTING, r101);
        t.set_link(PAGE_INS_HEAD, r100);
        t.eqtb[COUNT_BASE + 100].set_int(500);
        t.eqtb[COUNT_BASE + 101].set_int(1000);
        t.eqtb[COUNT_BASE + 102].set_int(1000);
        t.set_page_goal(6553600 - 163500 - 6 * 65536 - 65536);
        let c = rule(t);
        t.set_link(CONTRIB_HEAD, c);
        t.set_tail(c);
        assert_eq!(show_lists(t), rows[0][0]);

        // The output routine: its level has a negative mode_line, and the page is held over.
        t.set_link(PAGE_HEAD, i102);
        t.set_link(PAGE_INS_HEAD, PAGE_INS_HEAD);
        t.output_active = true;
        t.line = 15;
        t.push_nest();
        t.set_mode(-VMODE);
        t.set_prev_depth(t.pdf_ignored_dimen());
        t.set_mode_line(-t.line);
        t.page_contents = EMPTY;
        assert_eq!(show_lists(t), rows[1][0]);
        Tex::dealloc(t);
    }

    /// `x_over_n(height,1000)*count` is the one product in here that can leave the range: the oracle's
    /// `t` is 64 bits, and `print_scaled` must not get a wrapped sign.
    #[test]
    #[ignore = "needs print.rs, strings.rs, arith.rs (x_over_n), show.rs, mem.rs, page.rs"]
    fn the_insert_product_does_not_wrap() {
        let t = table_machine();
        print_setup(t);
        let r = t.get_node(PAGE_INS_NODE_SIZE);
        t.set_subtype(r, 100);
        t.set_ty(r, INSERTING);
        t.set_height(r, 1 << 30);
        t.set_link(r, PAGE_INS_HEAD);
        t.set_link(PAGE_INS_HEAD, r);
        t.set_link(PAGE_HEAD, r);
        t.page_tail = r;
        t.page_contents = INSERTS_ONLY;
        t.eqtb[COUNT_BASE + 100].set_int(1 << 20);
        let shown = show_lists(t);
        assert!(shown.contains("\\insert100 adds 32767.99998"), "clamped by sat32, not wrapped: {}", shown);
        Tex::dealloc(t);
    }
}
