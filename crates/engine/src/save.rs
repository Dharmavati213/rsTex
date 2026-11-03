//! §290-310 Part 19: saving and restoring equivalents.
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

// §290 declares e-TeX's procedures for tracing and input, which are in etex.rs.
impl Tex {
    /// §296
    pub fn new_save_level(&mut self, c: i32) {
        self.check_full_save_stack();
        if self.eTeX_ex() {
            self.set_saved(0, self.line);
            self.save_ptr += 1;
        }
        self.set_save_type(self.save_ptr, LEVEL_BOUNDARY);
        self.set_save_level(self.save_ptr, self.cur_group);
        self.set_save_index(self.save_ptr, self.cur_boundary);
        if self.cur_level == WEB_MAX_QUARTERWORD {
            // quit if |(cur_level+1)| is too big to be stored in |eqtb|
            self.overflow(b"grouping levels", WEB_MAX_QUARTERWORD);
        }
        self.cur_boundary = self.save_ptr;
        self.cur_group = c;
        if self.tracing_groups() > 0 {
            self.group_trace(false);
        }
        self.cur_level += 1;
        self.save_ptr += 1;
    }

    /// §297
    pub fn eq_destroy(&mut self, w: Word) {
        match self.eq_type_field(w) {
            CALL | LONG_CALL | OUTER_CALL | LONG_OUTER_CALL => self.delete_token_ref(self.equiv_field(w)),
            GLUE_REF => self.delete_glue_ref(self.equiv_field(w)),
            SHAPE_REF => {
                let q = self.equiv_field(w); // we need to free a \parshape block
                if q != NULL {
                    self.free_node(q, self.info(q) + self.info(q) + 1); // such a block is 2n+1 words long, where n=info(q)
                }
            }
            BOX_REF => self.flush_node_list(self.equiv_field(w)),
            // §1834
            TOKS_REGISTER | REGISTER => {
                if self.equiv_field(w) < MEM_BOT || self.equiv_field(w) > LO_MEM_STAT_MAX {
                    self.delete_sa_ref(self.equiv_field(w));
                }
            }
            _ => {}
        }
    }

    /// §298
    pub fn eq_save(&mut self, p: i32, l: i32) {
        self.check_full_save_stack();
        if l == LEVEL_ZERO {
            self.set_save_type(self.save_ptr, RESTORE_ZERO);
        } else {
            self.save_stack[self.save_ptr] = self.eqtb[p];
            self.save_ptr += 1;
            self.set_save_type(self.save_ptr, RESTORE_OLD_VALUE);
        }
        self.set_save_level(self.save_ptr, l);
        self.set_save_index(self.save_ptr, p);
        self.save_ptr += 1;
    }

    /// §299
    pub fn eq_define(&mut self, p: i32, t: i32, e: i32) {
        self.note_outer_meaning(t);
        if self.eTeX_ex() && self.eq_type(p) == t && self.equiv(p) == e {
            self.assign_trace(p, b"reassigning");
            self.eq_destroy(self.eqtb[p]);
            return;
        }
        self.assign_trace(p, b"changing");
        if self.eq_level(p) == self.cur_level {
            self.eq_destroy(self.eqtb[p]);
        } else if self.cur_level > LEVEL_ONE {
            self.eq_save(p, self.eq_level(p));
        }
        self.set_eq_level(p, self.cur_level);
        self.set_eq_type(p, t);
        self.set_equiv(p, e);
        self.assign_trace(p, b"into");
    }

    /// §300
    pub fn eq_word_define(&mut self, p: i32, w: i32) {
        if self.eTeX_ex() && self.eqtb[p].int() == w {
            self.assign_trace(p, b"reassigning");
            return;
        }
        self.assign_trace(p, b"changing");
        if self.xeq_level[p] != self.cur_level {
            self.eq_save(p, self.xeq_level[p]);
            self.xeq_level[p] = self.cur_level;
        }
        self.eqtb[p].set_int(w);
        self.assign_trace(p, b"into");
    }

    /// §301
    pub fn geq_define(&mut self, p: i32, t: i32, e: i32) {
        self.note_outer_meaning(t);
        self.assign_trace(p, b"globally changing");
        self.eq_destroy(self.eqtb[p]);
        self.set_eq_level(p, LEVEL_ONE);
        self.set_eq_type(p, t);
        self.set_equiv(p, e);
        self.assign_trace(p, b"into");
    }

    /// §301
    pub fn geq_word_define(&mut self, p: i32, w: i32) {
        self.assign_trace(p, b"globally changing");
        self.eqtb[p].set_int(w);
        self.xeq_level[p] = LEVEL_ONE;
        self.assign_trace(p, b"into");
    }

    /// §302
    pub fn save_for_after(&mut self, t: i32) {
        if self.cur_level > LEVEL_ONE {
            self.check_full_save_stack();
            self.set_save_type(self.save_ptr, INSERT_TOKEN);
            self.set_save_level(self.save_ptr, LEVEL_ZERO);
            self.set_save_index(self.save_ptr, t);
            self.save_ptr += 1;
        }
    }

    /// §303
    pub fn unsave(&mut self) {
        let mut a = false; // have we already processed an \aftergroup?
        let mut l = 0; // saved level, if in fullword regions of eqtb
        if self.cur_level > LEVEL_ONE {
            self.cur_level -= 1;
            // §304 Clear off top level from |save_stack|
            loop {
                self.save_ptr -= 1;
                if self.save_type(self.save_ptr) == LEVEL_BOUNDARY {
                    break; // done:
                }
                let p = self.save_index(self.save_ptr);
                if self.save_type(self.save_ptr) == INSERT_TOKEN {
                    // §348 Insert token |p| into TeX's input
                    let t = self.cur_tok;
                    self.cur_tok = p;
                    if a {
                        self.back_prepend(self.cur_tok);
                        if self.cur_tok < RIGHT_BRACE_LIMIT {
                            if self.cur_tok < LEFT_BRACE_LIMIT {
                                self.align_state -= 1;
                            } else {
                                self.align_state += 1;
                            }
                        }
                    } else {
                        self.back_input();
                        a = self.eTeX_ex();
                    }
                    self.cur_tok = t;
                } else if self.save_type(self.save_ptr) == RESTORE_SA {
                    self.sa_restore();
                    self.sa_chain = p;
                    self.sa_level = self.save_level(self.save_ptr);
                } else {
                    if self.save_type(self.save_ptr) == RESTORE_OLD_VALUE {
                        l = self.save_level(self.save_ptr);
                        self.save_ptr -= 1;
                    } else {
                        self.save_stack[self.save_ptr] = self.eqtb[UNDEFINED_CONTROL_SEQUENCE];
                    }
                    // §305 Store |save_stack[save_ptr]| in |eqtb[p]|, unless |eqtb[p]| holds a global value
                    if p < INT_BASE || p > EQTB_SIZE {
                        if self.eq_level(p) == LEVEL_ONE {
                            self.eq_destroy(self.save_stack[self.save_ptr]); // destroy the saved value
                            if self.tracing_restores() > 0 {
                                self.restore_trace(p, b"retaining");
                            }
                        } else {
                            self.eq_destroy(self.eqtb[p]); // destroy the current value
                            self.eqtb[p] = self.save_stack[self.save_ptr]; // restore the saved value
                            if self.tracing_restores() > 0 {
                                self.restore_trace(p, b"restoring");
                            }
                        }
                    } else if self.xeq_level[p] != LEVEL_ONE {
                        self.eqtb[p] = self.save_stack[self.save_ptr];
                        self.xeq_level[p] = l;
                        if self.tracing_restores() > 0 {
                            self.restore_trace(p, b"restoring");
                        }
                    } else if self.tracing_restores() > 0 {
                        self.restore_trace(p, b"retaining");
                    }
                }
            }
            // done:
            if self.tracing_groups() > 0 {
                self.group_trace(true);
            }
            if self.grp_stack[self.in_open] == self.cur_boundary {
                self.group_warning(); // groups possibly not properly nested with files
            }
            self.cur_group = self.save_level(self.save_ptr);
            self.cur_boundary = self.save_index(self.save_ptr);
            if self.eTeX_ex() {
                self.save_ptr -= 1;
            }
        } else {
            self.confusion(b"curlevel"); // |unsave| is not used when |cur_group=bottom_level|
        }
    }

    /// §306
    pub fn restore_trace(&mut self, p: i32, s: &[u8]) {
        self.begin_diagnostic();
        self.print_char(b'{' as i32);
        self.print_s(s);
        self.print_char(b' ' as i32);
        self.show_eqtb(p);
        self.print_char(b'}' as i32);
        self.end_diagnostic(false);
    }

    /// §310
    pub fn prepare_mag(&mut self) {
        if self.mag_set > 0 && self.mag() != self.mag_set {
            self.print_err_s(b"Incompatible magnification (");
            self.print_int(self.mag());
            self.print_s(b");");
            self.print_nl_s(b" the previous value will be retained");
            self.help(&[
                b"I can handle only one magnification ratio per job. So I've",
                b"reverted to the magnification you used earlier on this run.",
            ]);
            self.int_error(self.mag_set);
            self.geq_word_define(INT_BASE + MAG_CODE, self.mag_set); // mag:=mag_set
        }
        if self.mag() <= 0 || self.mag() > 32768 {
            self.print_err_s(b"Illegal magnification has been changed to 1000");
            self.help(&[b"The magnification ratio must be between 1 and 32768."]);
            self.int_error(self.mag());
            self.geq_word_define(INT_BASE + MAG_CODE, 1000);
        }
        self.mag_set = self.mag();
    }
}

/// What the tests of save.rs, eqtb.rs, nest.rs and hash.rs share: a machine with the tables as INITEX
/// leaves them, and the operations of the tables in testdata/, carried out as the commands of
/// main_control would.  tests/oracle/gen_eqtb_nest_hash_save_tables.py writes those tables.
#[cfg(test)]
pub(crate) mod testkit {
    use super::*;
    use crate::toks::testkit::*;

    /// What `initialize` will do to memory and the tables, in sections 182, 231, 240, 246, 250, 258,
    /// 268 and 1064 of the WEB, in e-TeX's extended mode.
    pub fn table_machine() -> &'static mut Tex {
        let t = machine();
        t.eTeX_mode = 1;
        t.nest_size = 500;
        t.save_size = 100000;
        t.cur_level = LEVEL_ONE;
        t.cur_group = BOTTOM_LEVEL;
        let mut k = MEM_BOT;
        while k <= LO_MEM_STAT_MAX {
            t.set_glue_ref_count(k, NULL + 1);
            t.set_stretch_order(k, NORMAL);
            t.set_shrink_order(k, NORMAL);
            k += GLUE_SPEC_SIZE;
        }
        t.rover = LO_MEM_STAT_MAX + 1;
        t.set_link(t.rover, EMPTY_FLAG);
        t.set_node_size(t.rover, 1000);
        t.set_llink(t.rover, t.rover);
        t.set_rlink(t.rover, t.rover);
        t.lo_mem_max = t.rover + 1000;
        t.set_link(t.lo_mem_max, NULL);
        t.set_info(t.lo_mem_max, NULL);
        t.set_subtype(PAGE_INS_HEAD, 255);
        t.set_ty(PAGE_INS_HEAD, SPLIT_UP);
        t.set_link(PAGE_INS_HEAD, PAGE_INS_HEAD);
        t.set_ty(PAGE_HEAD, GLUE_NODE);
        t.set_subtype(PAGE_HEAD, NORMAL);
        t.avail = NULL;
        t.mem_end = MEM_TOP;
        t.hi_mem_min = HI_MEM_STAT_MIN;
        t.page_tail = PAGE_HEAD;
        t.page_contents = EMPTY;
        t.set_mode(VMODE);
        t.set_head(CONTRIB_HEAD);
        t.set_tail(CONTRIB_HEAD);
        t.set_prev_depth(IGNORE_DEPTH);
        t.set_eq_type(UNDEFINED_CONTROL_SEQUENCE, UNDEFINED_CS);
        t.set_equiv(UNDEFINED_CONTROL_SEQUENCE, NULL);
        t.set_eq_level(UNDEFINED_CONTROL_SEQUENCE, LEVEL_ZERO);
        for k in ACTIVE_BASE..UNDEFINED_CONTROL_SEQUENCE {
            t.eqtb[k] = t.eqtb[UNDEFINED_CONTROL_SEQUENCE];
        }
        t.set_text(FROZEN_NULL_FONT, S_NULLFONT);
        t.set_font_id_text(NULL_FONT, S_NULLFONT);
        t.set_equiv(GLUE_BASE, ZERO_GLUE);
        t.set_eq_level(GLUE_BASE, LEVEL_ONE);
        t.set_eq_type(GLUE_BASE, GLUE_REF);
        for k in GLUE_BASE + 1..LOCAL_BASE {
            t.eqtb[k] = t.eqtb[GLUE_BASE];
        }
        t.set_glue_ref_count(ZERO_GLUE, t.glue_ref_count(ZERO_GLUE) + LOCAL_BASE - GLUE_BASE);
        t.set_eq_type(PAR_SHAPE_LOC, SHAPE_REF);
        t.set_eq_level(PAR_SHAPE_LOC, LEVEL_ONE);
        t.set_equiv(PAR_SHAPE_LOC, NULL);
        for k in ETEX_PEN_BASE..ETEX_PENS {
            t.eqtb[k] = t.eqtb[PAR_SHAPE_LOC];
        }
        for k in OUTPUT_ROUTINE_LOC..TOKS_BASE + 256 {
            t.eqtb[k] = t.eqtb[UNDEFINED_CONTROL_SEQUENCE];
        }
        t.set_eq_type(BOX_BASE, BOX_REF);
        t.set_eq_level(BOX_BASE, LEVEL_ONE);
        t.set_equiv(BOX_BASE, NULL);
        for k in BOX_BASE + 1..BOX_BASE + 256 {
            t.eqtb[k] = t.eqtb[BOX_BASE];
        }
        t.set_eq_type(CUR_FONT_LOC, DATA);
        t.set_eq_level(CUR_FONT_LOC, LEVEL_ONE);
        t.set_equiv(CUR_FONT_LOC, NULL_FONT);
        for k in MATH_FONT_BASE..MATH_FONT_BASE + 48 {
            t.eqtb[k] = t.eqtb[CUR_FONT_LOC];
        }
        // The category codes are machine()'s.
        for k in CAT_CODE_BASE..INT_BASE {
            let e = t.equiv(k);
            t.eqtb[k] = Word::hb(e, DATA, LEVEL_ONE);
        }
        for k in 0..256 {
            t.set_math_code(k, k);
            t.set_sf_code(k, 1000);
        }
        for k in 0..10 {
            t.set_math_code(b'0' as i32 + k, b'0' as i32 + k + VAR_CODE);
        }
        for k in 0..26 {
            let (l, u) = (b'a' as i32 + k, b'A' as i32 + k);
            t.set_math_code(l, l + VAR_CODE + 0x100);
            t.set_math_code(u, u + VAR_CODE + 0x100);
            t.set_lc_code(l, l);
            t.set_lc_code(u, l);
            t.set_uc_code(l, u);
            t.set_uc_code(u, u);
            t.set_sf_code(u, 999);
        }
        for k in INT_BASE..DIMEN_BASE {
            t.eqtb[k] = Word::ZERO;
        }
        t.set_mag(1000);
        t.set_tolerance(10000);
        t.set_hang_after(1);
        t.set_max_dead_cycles(25);
        t.set_escape_char(b'\\' as i32);
        t.set_end_line_char(-1);
        for k in 0..256 {
            t.set_del_code(k, -1);
        }
        t.set_del_code(b'.' as i32, 0);
        for k in DIMEN_BASE..=EQTB_SIZE {
            t.eqtb[k] = Word::ZERO;
        }
        t.set_pdf_ignored_dimen(IGNORE_DEPTH);
        for k in INT_BASE..=EQTB_SIZE {
            t.xeq_level[k] = LEVEL_ONE;
        }
        t
    }

    /// A table of testdata/: lines of fields, with a newline in a field as `|`.
    pub fn table(data: &'static str) -> Vec<Vec<String>> {
        data.lines().filter(|l| !l.is_empty()).map(|l| l.split('\t').map(|f| f.replace('|', "\n")).collect()).collect()
    }

    /// The location of the control sequence of that name, entered if it is new.
    pub fn cs_named(t: &mut Tex, name: &str) -> i32 {
        match name {
            "~" => ACTIVE_BASE + b'~' as i32,
            "NULLCS" => NULL_CS,
            _ if name.len() == 1 => SINGLE_BASE + name.as_bytes()[0] as i32,
            _ => {
                for (i, &c) in name.as_bytes().iter().enumerate() {
                    t.buffer.put(t.first + i as i32, c as i32);
                }
                let saved = t.no_new_control_sequence;
                t.no_new_control_sequence = false;
                let p = t.id_lookup(t.first, name.len() as i32);
                t.no_new_control_sequence = saved;
                p
            }
        }
    }

    fn num(s: &str) -> i32 {
        s.parse().unwrap()
    }

    fn push_math(t: &mut Tex, c: i32) {
        t.push_nest();
        t.set_mode(-MMODE);
        t.set_incompleat_noad(NULL);
        t.new_save_level(c);
    }

    /// new_graf, without the glue, which goes straight away on an empty page, and the indent box.
    fn new_graf(t: &mut Tex) {
        fn norm_min(h: i32) -> i32 {
            if h <= 0 { 1 } else if h >= 64 { 63 } else { h }
        }
        t.push_nest();
        t.set_mode(HMODE);
        t.set_space_factor(1000);
        t.set_cur_lang();
        t.set_clang(t.cur_lang);
        t.set_prev_graf((norm_min(t.left_hyphen_min()) * 0o100 + norm_min(t.right_hyphen_min())) * 0o200000 + t.cur_lang);
    }

    /// begin_box with scan_spec.  `box KIND SPEC AMOUNT CONTEXT [ARG]`.
    fn begin_box(t: &mut Tex, f: &[&str]) {
        let (k, c0) = match f[1] {
            "hbox" => (HMODE, HBOX_GROUP),
            "vbox" => (VMODE, VBOX_GROUP),
            _ => (VMODE, VTOP_GROUP),
        };
        let arg = f.get(5).map_or(0, |s| num(s));
        let context = match f[4] {
            "none" => 0,
            "setbox" => BOX_FLAG + arg,
            "gsetbox" => GLOBAL_BOX_FLAG + arg,
            "moveleft" | "raise" => -(arg * 65536),
            "moveright" | "lower" => arg * 65536,
            "shipout" => SHIP_OUT_FLAG,
            "leaders" => LEADER_FLAG,
            "cleaders" => LEADER_FLAG + 1,
            _ => LEADER_FLAG + 2,
        };
        let c = if k == HMODE && context < BOX_FLAG && abs(t.mode()) == VMODE { ADJUSTED_HBOX_GROUP } else { c0 };
        t.set_saved(0, context);
        t.save_ptr += 1;
        t.set_saved(0, if f[2] == "to" { EXACTLY } else { ADDITIONAL });
        t.set_saved(1, if f[2] == "none" { 0 } else { num(f[3]) * 65536 });
        t.save_ptr += 2;
        t.new_save_level(c);
        t.push_nest();
        t.set_mode(-k);
        if k == VMODE {
            t.set_prev_depth(t.pdf_ignored_dimen());
        } else {
            t.set_space_factor(1000);
        }
    }

    /// One operation that opens or closes a group, or starts a mode: see `group_op_tex` in the generator.
    pub fn group_op(t: &mut Tex, op: &str) {
        let f: Vec<&str> = op.split(' ').collect();
        match f[0] {
            "{" => t.new_save_level(SIMPLE_GROUP),
            "beg" => t.new_save_level(SEMI_SIMPLE_GROUP),
            "}" | "end" => t.unsave(),
            "H" => new_graf(t),
            "box" => begin_box(t, &f),
            "$" => {
                push_math(t, MATH_SHIFT_GROUP);
                t.eq_word_define(INT_BASE + CUR_FAM_CODE, -1);
            }
            "$$" => {
                push_math(t, MATH_SHIFT_GROUP);
                t.set_mode(MMODE);
                t.eq_word_define(INT_BASE + CUR_FAM_CODE, -1);
            }
            "insert" | "vadjust" => {
                // saved(1) is pdfTeX's `pre`, which is why \showgroups shows \insert0 for both.
                t.set_saved(0, if f[0] == "vadjust" { 255 } else { num(f[1]) });
                t.set_saved(1, 0);
                t.save_ptr += 2;
                t.new_save_level(INSERT_GROUP);
                t.push_nest();
                t.set_mode(-VMODE);
                t.set_prev_depth(t.pdf_ignored_dimen());
            }
            "disc" | "disc2" => {
                if f[0] == "disc" {
                    t.save_ptr += 1;
                    t.set_saved(-1, 0);
                } else {
                    t.unsave();
                    t.pop_nest();
                    t.set_saved(-1, t.saved(-1) + 1);
                }
                t.new_save_level(DISC_GROUP);
                t.push_nest();
                t.set_mode(-HMODE);
                t.set_space_factor(1000);
            }
            "choice" | "choice2" => {
                if f[0] == "choice" {
                    t.save_ptr += 1;
                    t.set_saved(-1, 0);
                } else {
                    t.unsave();
                    t.pop_nest();
                    t.set_saved(-1, t.saved(-1) + 1);
                }
                push_math(t, MATH_CHOICE_GROUP);
            }
            "sup" => {
                t.set_saved(0, NULL);
                t.save_ptr += 1;
                push_math(t, MATH_GROUP);
            }
            "vcenter" => {
                t.set_saved(0, if f[1] == "to" { EXACTLY } else { ADDITIONAL });
                t.set_saved(1, if f[1] == "none" { 0 } else { num(f[2]) * 65536 });
                t.save_ptr += 2;
                t.new_save_level(VCENTER_GROUP);
                t.push_nest();
                t.set_mode(-VMODE);
                t.set_prev_depth(t.pdf_ignored_dimen());
            }
            _ => panic!("group op {}", op),
        }
    }

    /// The `eqtb` location of a word register or parameter of the tables.
    pub fn word_addr(kind: &str, n: i32) -> i32 {
        match kind {
            "count" => COUNT_BASE + n,
            "dimen" => SCALED_BASE + n,
            "delcode" => DEL_CODE_BASE + n,
            "tolerance" => INT_BASE + TOLERANCE_CODE,
            "TeXXeTstate" => INT_BASE + ETEX_STATE_CODE + TEXXET_CODE,
            "pdfoutput" => INT_BASE + PDF_OUTPUT_CODE,
            "synctex" => INT_BASE + SYNCTEX_CODE,
            "language" => INT_BASE + LANGUAGE_CODE,
            "lefthyphenmin" => INT_BASE + LEFT_HYPHEN_MIN_CODE,
            "righthyphenmin" => INT_BASE + RIGHT_HYPHEN_MIN_CODE,
            "hsize" => DIMEN_BASE + HSIZE_CODE,
            "pdfpagewidth" => DIMEN_BASE + PDF_PAGE_WIDTH_CODE,
            _ => panic!("word {}", kind),
        }
    }

    /// The message of a panic.
    pub fn panic_message(f: impl FnOnce()) -> String {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        match r {
            Ok(()) => panic!("this did not stop"),
            Err(e) => e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap(),
        }
    }

    /// Runs `f`, which must stop the run, and returns what was printed.  `World::exit` panics in a unit test.  §92 would
    /// open the log for a run with no job name, so it has one; with no DVI begun, §670 says nothing but "No pages".
    pub fn stops(t: &mut Tex, f: impl FnOnce(&mut Tex)) -> String {
        t.job_name = S_TEXPUT;
        t.cur_s = -1;
        let m = panic_message(|| f(t));
        assert!(m.starts_with("exit("), "stopped by something else: {}", m);
        crate::toks::testkit::printed(t)
    }

    /// §94's message: the stop is at the oracle's limit.  With no log open §93 does not call `error`, which adds the `.`.
    pub fn assert_overflow(t: &mut Tex, f: impl FnOnce(&mut Tex), what: &str, n: i32) {
        let s = stops(t, f);
        let want = format!("! TeX capacity exceeded, sorry [{}={}]", what, n);
        assert!(s.contains(&want), "wanted {:?} in {:?}", want, s);
    }
}

#[cfg(test)]
mod tests {
    use super::testkit::*;
    use super::*;
    use crate::toks::testkit::*;

    fn define(t: &mut Tex, global: bool, p: i32, ty: i32, e: i32) {
        if global {
            t.geq_define(p, ty, e);
        } else {
            t.eq_define(p, ty, e);
        }
    }

    fn word(t: &mut Tex, global: bool, p: i32, w: i32) {
        if global {
            t.geq_word_define(p, w);
        } else {
            t.eq_word_define(p, w);
        }
    }

    fn glue_addr(kind: &str, n: i32) -> i32 {
        match kind {
            "skip" => SKIP_BASE + n,
            "muskip" => MU_SKIP_BASE + n,
            "baselineskip" => GLUE_BASE + BASELINE_SKIP_CODE,
            _ => GLUE_BASE + THIN_MU_SKIP_CODE,
        }
    }

    fn toks_addr(kind: &str, n: i32) -> i32 {
        if kind == "toks" { TOKS_BASE + n } else { EVERY_PAR_LOC }
    }

    /// One operation of save_trace_oracle.tsv, as the assignments of prefixed_command would do it.
    fn trace_op(t: &mut Tex, op: &str) {
        let f: Vec<&str> = op.split(' ').collect();
        let g = f[0].starts_with(|c: char| c.is_ascii_uppercase());
        let n = |i: usize| f[i].parse::<i32>().unwrap();
        match f[0].to_ascii_lowercase().as_str() {
            "beg" | "end" => group_op(t, f[0]),
            "w" => word(t, g, word_addr(f[1], n(2)), n(3)),
            "h" => {
                let base = match f[1] {
                    "catcode" => CAT_CODE_BASE,
                    "lccode" => LC_CODE_BASE,
                    "uccode" => UC_CODE_BASE,
                    "sfcode" => SF_CODE_BASE,
                    _ => MATH_CODE_BASE,
                };
                define(t, g, base + n(2), DATA, n(3));
            }
            "f" => {
                let p = match f[1] {
                    "textfont" => MATH_FONT_BASE + n(2),
                    "scriptfont" => MATH_FONT_BASE + 16 + n(2),
                    "scriptscriptfont" => MATH_FONT_BASE + 32 + n(2),
                    _ => CUR_FONT_LOC,
                };
                define(t, g, p, DATA, NULL_FONT);
            }
            "g" => {
                // with trap_zero_glue
                let (w, s, so, h, ho) = (n(3), n(4), n(5), n(6), n(7));
                let q = if w == 0 && s == 0 && h == 0 {
                    t.add_glue_ref(ZERO_GLUE);
                    ZERO_GLUE
                } else {
                    let q = t.get_node(GLUE_SPEC_SIZE);
                    t.set_glue_ref_count(q, NULL);
                    t.set_width(q, w);
                    t.set_stretch(q, s);
                    t.set_stretch_order(q, so);
                    t.set_shrink(q, h);
                    t.set_shrink_order(q, ho);
                    q
                };
                define(t, g, glue_addr(f[1], n(2)), GLUE_REF, q);
            }
            "t" => {
                let p = toks_addr(f[1], n(2));
                let text = f.get(3).copied().unwrap_or("");
                if text.is_empty() {
                    define(t, g, p, UNDEFINED_CS, NULL);
                } else {
                    let m = t.tb_mark();
                    for c in text.bytes() {
                        t.tb_push(LETTER_TOKEN + c as i32);
                    }
                    let q = t.tb_commit_ref(m);
                    define(t, g, p, CALL, q);
                }
            }
            "q" => {
                let (p, q) = (toks_addr(f[1], n(2)), t.equiv(toks_addr(f[3], n(4))));
                if q == NULL {
                    define(t, g, p, UNDEFINED_CS, NULL);
                } else {
                    t.add_token_ref(q);
                    define(t, g, p, CALL, q);
                }
            }
            "d" => {
                let p = cs_named(t, f[1]);
                if f[1] == "NULLCS" && t.eq_type(p) == UNDEFINED_CS {
                    t.eq_define(p, RELAX, 256); // \csname\endcsname makes it \relax first
                }
                let m = t.tb_mark();
                t.tb_push(END_MATCH_TOKEN);
                for c in f.get(2).copied().unwrap_or("").bytes() {
                    t.tb_push(LETTER_TOKEN + c as i32);
                }
                let q = t.tb_commit_ref(m);
                define(t, g, p, CALL, q);
            }
            "l" => {
                let p = cs_named(t, f[1]);
                if f[2] == "relax" {
                    define(t, g, p, RELAX, 256);
                } else {
                    let q = cs_named(t, f[2]);
                    let (cmd, chr) = (t.eq_type(q), t.equiv(q));
                    t.add_token_ref(chr);
                    define(t, g, p, cmd, chr);
                }
            }
            "s" => {
                let c = n(1);
                let p = if c <= 0 {
                    NULL
                } else {
                    let p = t.get_node(2 * c + 1);
                    t.set_info(p, c);
                    for j in 1..=c {
                        t.mem[p + 2 * j - 1].set_sc(65536);
                        t.mem[p + 2 * j].set_sc(2 * 65536);
                    }
                    p
                };
                define(t, g, PAR_SHAPE_LOC, SHAPE_REF, p);
            }
            "p" => {
                let q = match f[1] {
                    "interlinepenalties" => INTER_LINE_PENALTIES_LOC,
                    "clubpenalties" => CLUB_PENALTIES_LOC,
                    "widowpenalties" => WIDOW_PENALTIES_LOC,
                    _ => DISPLAY_WIDOW_PENALTIES_LOC,
                };
                let c = f.len() as i32 - 2;
                let m = (c / 2) + 1;
                let p = t.get_node(2 * m + 1);
                t.set_info(p, m);
                t.mem[p + 1].set_int(c);
                for j in 0..c {
                    let v = n(2 + j as usize);
                    t.mem[p + 2 + j].set_int(v);
                }
                if !odd(c) {
                    t.mem[p + c + 2].set_int(0);
                }
                define(t, g, q, SHAPE_REF, p);
            }
            "b" => define(t, g, BOX_BASE + n(1), BOX_REF, NULL),
            _ => panic!("op {}", op),
        }
    }

    // ---------------------------------------------------------------------------------------------
    // ---------------------------------------------------------------------------------------------

    /// The two words of a level in e-TeX's mode, one in compatibility mode.
    #[test]
    fn a_level_on_the_stack() {
        for (mode, words) in [(1, 2), (0, 1)] {
            let t = table_machine();
            t.eTeX_mode = mode;
            t.grp_stack[0] = -1; // keeps unsave away from group_warning, which would do nothing at in_open=0
            t.line = 17;
            t.new_save_level(HBOX_GROUP);
            assert_eq!(t.save_ptr, words);
            assert_eq!(t.cur_level, LEVEL_ONE + 1);
            assert_eq!(t.cur_group, HBOX_GROUP);
            assert_eq!(t.cur_boundary, words - 1);
            assert_eq!(t.save_type(words - 1), LEVEL_BOUNDARY);
            assert_eq!(t.save_level(words - 1), BOTTOM_LEVEL);
            assert_eq!(t.save_index(words - 1), 0);
            if mode == 1 {
                assert_eq!(t.save_stack[0].int(), 17, "saved(0):=line");
            }
            t.new_save_level(SIMPLE_GROUP);
            assert_eq!(t.save_ptr, 2 * words);
            assert_eq!(t.save_level(2 * words - 1), HBOX_GROUP);
            assert_eq!(t.save_index(2 * words - 1), words - 1);
            assert_eq!(t.max_save_stack, words, "check_full_save_stack runs before the push");
            t.unsave();
            assert_eq!((t.cur_group, t.cur_level, t.save_ptr, t.cur_boundary), (HBOX_GROUP, LEVEL_ONE + 1, words, words - 1));
            t.unsave();
            assert_eq!((t.cur_group, t.cur_level, t.save_ptr, t.cur_boundary), (BOTTOM_LEVEL, LEVEL_ONE, 0, 0));
            Tex::dealloc(t);
        }
    }

    #[test]
    fn words_at_nested_levels() {
        let t = table_machine();
        let p = COUNT_BASE + 1;
        t.eq_word_define(p, 1);
        assert_eq!(t.save_ptr, 0, "at level one nothing is saved");
        assert_eq!(t.xeq_level[p], LEVEL_ONE);
        t.new_save_level(SIMPLE_GROUP);
        t.eq_word_define(p, 2);
        assert_eq!(t.xeq_level[p], LEVEL_ONE + 1);
        assert_eq!(t.save_ptr, 4, "line, boundary, old value, restore word");
        t.eq_word_define(p, 3);
        assert_eq!(t.save_ptr, 4, "saved once per level");
        t.new_save_level(SIMPLE_GROUP);
        t.eq_word_define(p, 4);
        assert_eq!(t.save_type(t.save_ptr - 1), RESTORE_OLD_VALUE);
        assert_eq!(t.save_level(t.save_ptr - 1), LEVEL_ONE + 1);
        assert_eq!(t.save_index(t.save_ptr - 1), p);
        assert_eq!(t.save_stack[t.save_ptr - 2].int(), 3);
        t.unsave();
        assert_eq!(t.eqtb[p].int(), 3);
        assert_eq!(t.xeq_level[p], LEVEL_ONE + 1);
        t.unsave();
        assert_eq!(t.eqtb[p].int(), 1);
        assert_eq!(t.xeq_level[p], LEVEL_ONE);
        assert_eq!(t.save_ptr, 0);
        Tex::dealloc(t);
    }

    #[test]
    fn a_global_word_survives_every_level() {
        let t = table_machine();
        let p = DIMEN_BASE + HSIZE_CODE;
        t.new_save_level(SIMPLE_GROUP);
        t.eq_word_define(p, 1);
        t.new_save_level(SIMPLE_GROUP);
        t.geq_word_define(p, 9);
        assert_eq!(t.xeq_level[p], LEVEL_ONE);
        t.unsave();
        assert_eq!(t.eqtb[p].int(), 9, "retained");
        assert_eq!(t.xeq_level[p], LEVEL_ONE);
        t.unsave();
        assert_eq!(t.eqtb[p].int(), 9);
        Tex::dealloc(t);
    }

    #[test]
    fn an_unchanged_word_saves_nothing_in_extended_mode_only() {
        let t = table_machine();
        let p = COUNT_BASE + 1;
        t.new_save_level(SIMPLE_GROUP);
        let v = t.save_ptr;
        t.eq_word_define(p, 0);
        assert_eq!(t.save_ptr, v, "reassigning");
        t.eTeX_mode = 0;
        t.eq_word_define(p, 0);
        assert_eq!(t.save_ptr, v + 2, "compatibility mode saves it");
        t.eTeX_mode = 1;
        t.unsave();
        Tex::dealloc(t);
    }

    /// A macro at each level, with its reference count: `delete_token_ref` frees at null, so a freed
    /// block is reused by the next list of its size class.
    #[test]
    fn macros_at_nested_levels_keep_their_references() {
        let t = table_machine();
        let p = SINGLE_BASE + b'x' as i32;
        let mk = |t: &mut Tex, c: i32| {
            let m = t.tb_mark();
            t.tb_push(END_MATCH_TOKEN);
            t.tb_push(LETTER_TOKEN + c);
            t.tb_commit_ref(m)
        };
        assert_eq!(t.eq_level(p), LEVEL_ZERO);
        t.new_save_level(SIMPLE_GROUP);
        let a = mk(t, 97);
        t.eq_define(p, CALL, a);
        assert_eq!(t.eq_level(p), LEVEL_ONE + 1);
        assert_eq!(t.save_type(t.save_ptr - 1), RESTORE_ZERO, "at level zero there is no value to keep");
        t.new_save_level(SIMPLE_GROUP);
        let b = mk(t, 98);
        t.eq_define(p, CALL, b);
        assert_eq!(t.eq_level(p), LEVEL_ONE + 2);
        assert_eq!(t.save_type(t.save_ptr - 1), RESTORE_OLD_VALUE);
        assert_eq!(t.save_level(t.save_ptr - 1), LEVEL_ONE + 1);
        // Assigning again at this level destroys what was there and saves nothing.
        let v = t.save_ptr;
        let c = mk(t, 99);
        t.eq_define(p, CALL, c);
        assert_eq!(t.save_ptr, v);
        assert_eq!(mk(t, 100), b, "the block of b was freed and is reused");
        t.unsave();
        assert_eq!(t.equiv(p), a, "the saved value is back");
        assert_eq!(t.eq_level(p), LEVEL_ONE + 1);
        assert_eq!(list(t, a), vec![END_MATCH_TOKEN, LETTER_TOKEN + 97]);
        t.unsave();
        assert_eq!((t.eq_type(p), t.equiv(p), t.eq_level(p)), (UNDEFINED_CS, NULL, LEVEL_ZERO), "eqtb[undefined_control_sequence]");
        Tex::dealloc(t);
    }

    #[test]
    fn a_global_macro_destroys_what_was_saved() {
        let t = table_machine();
        let p = SINGLE_BASE + b'x' as i32;
        let mk = |t: &mut Tex| {
            let m = t.tb_mark();
            t.tb_push(END_MATCH_TOKEN);
            t.tb_commit_ref(m)
        };
        let a = mk(t);
        t.eq_define(p, CALL, a);
        t.new_save_level(SIMPLE_GROUP);
        let b = mk(t);
        t.eq_define(p, CALL, b);
        t.add_token_ref(b); // a second reference
        let c = mk(t);
        t.geq_define(p, CALL, c); // destroys b's reference held by eqtb
        assert_eq!(t.token_ref_count(b), 0);
        assert_eq!(t.eq_level(p), LEVEL_ONE);
        t.unsave(); // retaining: destroys a, the saved value
        assert_eq!(t.equiv(p), c);
        assert_eq!(mk(t), a, "a was freed by retaining");
        Tex::dealloc(t);
    }

    #[test]
    fn reassigning_a_macro_gives_back_the_extra_reference() {
        let t = table_machine();
        let p = SINGLE_BASE + b'x' as i32;
        let m = t.tb_mark();
        t.tb_push(END_MATCH_TOKEN);
        let a = t.tb_commit_ref(m);
        t.eq_define(p, CALL, a);
        t.new_save_level(SIMPLE_GROUP);
        let v = t.save_ptr;
        t.add_token_ref(a); // \let\x=\x
        assert_eq!(t.token_ref_count(a), 1);
        t.eq_define(p, CALL, a);
        assert_eq!(t.token_ref_count(a), 0, "eq_destroy in the reassigning path");
        assert_eq!(t.save_ptr, v);
        t.eTeX_mode = 0;
        t.add_token_ref(a);
        t.eq_define(p, CALL, a);
        assert_eq!(t.save_ptr, v + 2, "compatibility mode saves it");
        t.eTeX_mode = 1;
        t.unsave();
        assert_eq!(t.token_ref_count(a), 0);
        Tex::dealloc(t);
    }

    /// A location in the static part of mem is a register code, not a sparse array element.
    #[test]
    fn eq_destroy_of_a_register_code_does_nothing() {
        let t = table_machine();
        for e in [MEM_BOT, LO_MEM_STAT_MAX] {
            for ty in [TOKS_REGISTER, REGISTER] {
                t.eq_destroy(Word::hb(e, ty, LEVEL_ONE));
            }
        }
        t.eq_destroy(Word::hb(12345, DATA, LEVEL_ONE));
        t.eq_destroy(Word::hb(12345, UNDEFINED_CS, LEVEL_ONE));
        Tex::dealloc(t);
    }

    #[test]
    fn aftergroup_tokens_come_back_in_order() {
        for mode in [1, 0] {
            let t = table_machine();
            t.eTeX_mode = mode;
            t.grp_stack[0] = -1;
            t.save_for_after(LETTER_TOKEN + 122); // at level one it is ignored
            assert_eq!(t.save_ptr, 0);
            t.new_save_level(SIMPLE_GROUP);
            for c in [97, 98, 99] {
                t.save_for_after(LETTER_TOKEN + c);
            }
            t.save_for_after(CS_TOKEN_FLAG + SINGLE_BASE + b'x' as i32);
            t.cur_tok = 77;
            let level = t.input_ptr;
            t.unsave();
            assert_eq!(t.cur_tok, 77, "cur_tok is kept");
            // e-TeX puts them in one list; compatibility mode has a level for each.
            assert_eq!(t.input_ptr, level + if mode == 1 { 1 } else { 4 });
            assert_eq!(
                drain(t, -1),
                vec![LETTER_TOKEN + 97, LETTER_TOKEN + 98, LETTER_TOKEN + 99, CS_TOKEN_FLAG + SINGLE_BASE + b'x' as i32]
            );
            Tex::dealloc(t);
        }
    }

    /// The test on later tokens, and back_input's on the first, both move align_state.
    #[test]
    fn aftergroup_braces_move_align_state() {
        let t = table_machine();
        let s = t.align_state;
        t.new_save_level(SIMPLE_GROUP);
        t.save_for_after(LEFT_BRACE_TOKEN + 123);
        t.save_for_after(LEFT_BRACE_TOKEN + 123);
        t.save_for_after(RIGHT_BRACE_TOKEN + 125);
        t.unsave();
        assert_eq!(t.align_state, s + 1 - 2);
        Tex::dealloc(t);
    }

    #[test]
    fn the_group_limit_lets_as_many_levels_through_as_the_oracle() {
        let rows = table(include_str!("../testdata/save_limits_oracle.tsv"));
        let r = rows.iter().find(|r| r[0] == "grouping levels").unwrap();
        assert_eq!(r[3], format!("TeX capacity exceeded, sorry [grouping levels={}].", WEB_MAX_QUARTERWORD));
        let t = table_machine();
        for _ in 0..r[2].parse::<i32>().unwrap() {
            t.new_save_level(SIMPLE_GROUP);
        }
        assert_eq!(t.cur_level, WEB_MAX_QUARTERWORD, "one more level is the overflow");
        Tex::dealloc(t);
    }

    #[test]
    fn the_group_limit_stops_at_255() {
        let t = table_machine();
        print_setup(t);
        for _ in 0..254 {
            t.new_save_level(SIMPLE_GROUP);
        }
        assert_overflow(t, |t| t.new_save_level(SIMPLE_GROUP), "grouping levels", WEB_MAX_QUARTERWORD);
    }

    #[test]
    fn save_size_stops_where_the_oracle_does() {
        let rows = table(include_str!("../testdata/save_limits_oracle.tsv"));
        let r = rows.iter().find(|r| r[0] == "save size").unwrap();
        let (limit, steps): (i32, i32) = (r[1].parse().unwrap(), r[2].parse().unwrap());
        assert_eq!(r[3], format!("TeX capacity exceeded, sorry [save size={}].", limit));
        let t = table_machine();
        print_setup(t);
        t.save_size = limit;
        let p = COUNT_BASE + 1;
        for k in 1..=steps {
            t.new_save_level(SIMPLE_GROUP);
            t.eq_word_define(p, k);
        }
        t.new_save_level(SIMPLE_GROUP);
        assert_overflow(t, |t| t.eq_word_define(p, steps + 1), "save size", limit);
    }

    #[test]
    fn unsave_at_level_one_is_a_confusion() {
        let t = table_machine();
        print_setup(t);
        let s = stops(t, |t| t.unsave());
        assert!(s.contains("! This can't happen (curlevel)"), "{:?}", s);
    }

    // ---------------------------------------------------------------------------------------------
    // These print, and compare with the oracle's log for the same operations.
    // ---------------------------------------------------------------------------------------------

    #[test]
    fn tracing_against_the_oracle() {
        for r in table(include_str!("../testdata/save_trace_oracle.tsv")) {
            let t = table_machine();
            print_setup(t);
            t.set_tracing_restores(1);
            if r[0] == "e" {
                t.set_tracing_assigns(1);
            } else {
                t.eTeX_mode = 0;
            }
            for op in r[1].split(';') {
                trace_op(t, op);
            }
            assert_eq!(printed(t), r[2], "operations {}", r[1]);
            Tex::dealloc(t);
        }
    }

    #[test]
    fn groups_against_the_oracle() {
        for r in table(include_str!("../testdata/save_groups_oracle.tsv")) {
            let t = table_machine();
            print_setup(t);
            let first: i32 = r[0].parse().unwrap();
            t.set_tracing_groups(r[1].parse().unwrap());
            for (i, op) in r[2].split(';').enumerate() {
                t.line = first + i as i32;
                if op == "S" {
                    t.begin_diagnostic();
                    t.show_save_groups();
                    t.end_diagnostic(true);
                } else {
                    group_op(t, op);
                }
            }
            let text = printed(t);
            let got: Vec<&str> = text.lines().filter(|l| l.starts_with("### ") || l.starts_with('{')).collect();
            assert_eq!(got.join("\n"), r[3], "operations {}", r[2]);
            Tex::dealloc(t);
        }
    }

    #[test]
    fn prepare_mag_against_the_oracle() {
        for r in table(include_str!("../testdata/save_mag_oracle.tsv")) {
            let t = table_machine();
            print_setup(t);
            let mut got = String::new();
            for m in r[0].split(';') {
                t.set_mag(m.parse().unwrap());
                t.prepare_mag();
                let text = printed(t);
                for l in text.lines() {
                    if l.starts_with("! ") || l.starts_with(" the previous value") {
                        got.push_str(l);
                        got.push('\n');
                    }
                }
                got.push_str(&format!("MAG={}\n", t.mag()));
            }
            assert_eq!(got.trim_end(), r[1], "\\mag values {}", r[0]);
            Tex::dealloc(t);
        }
    }

    #[test]
    fn restore_trace_prints_its_word() {
        let t = table_machine();
        print_setup(t);
        t.restore_trace(COUNT_BASE + 3, b"anything");
        assert_eq!(printed(t), "{anything \\count3=0}\n");
        Tex::dealloc(t);
    }
}
