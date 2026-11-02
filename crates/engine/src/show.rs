//! §191-216 Part 12: displaying boxes.
//!
//! `show_node_list` holds the cases of the whatsits §1603, of e-TeX §1704, and of the mlists §866-§873, as
//! tangle has them.  Its output is compared with the oracle's `\showbox` by the tests below: see
//! `tests/oracle/gen_show_tables.py`.
//!
//! Lists in nodes are headers in `toks`, so `print_mark` is on those; see docs/PORTING.md, section 15.
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
    /// §192
    pub fn print_font_identifier(&mut self, f: i32) {
        if self.pdf_font_blink[f] == NULL_FONT {
            self.print_esc(self.font_id_text(f));
        } else {
            self.print_esc(self.font_id_text(self.pdf_font_blink[f]));
        }
        if self.pdf_tracing_fonts() > 0 {
            self.print_s(b" (");
            self.print(self.font_name[f]);
            if self.font_size[f] != self.font_dsize[f] {
                self.print(b'@' as i32);
                self.print_scaled(self.font_size[f]);
                self.print_s(b"pt");
            }
            self.print(b')' as i32);
        } else if self.pdf_font_expand_ratio[f] != 0 {
            self.print_s(b" (");
            if self.pdf_font_expand_ratio[f] > 0 {
                self.print(b'+' as i32);
            }
            self.print_int(self.pdf_font_expand_ratio[f]);
            self.print(b')' as i32);
        }
    }

    /// §192
    pub fn short_display(&mut self, p: i32) {
        let mut p = p;
        let mut n: i32;
        while p > MEM_MIN {
            if self.is_char_node(p) {
                if p <= self.mem_end {
                    if self.font(p) != self.font_in_short_display {
                        if self.font(p) > self.font_max {
                            self.print_char(b'*' as i32);
                        } else {
                            self.print_font_identifier(self.font(p));
                        }
                        self.print_char(b' ' as i32);
                        self.font_in_short_display = self.font(p);
                    }
                    self.print(self.character(p));
                }
            } else {
                // §193 Print a short indication of the contents of node p
                match self.ty(p) {
                    HLIST_NODE | VLIST_NODE | INS_NODE | WHATSIT_NODE | MARK_NODE | ADJUST_NODE | UNSET_NODE => {
                        self.print_s(b"[]");
                    }
                    RULE_NODE => self.print_char(b'|' as i32),
                    GLUE_NODE => {
                        if self.glue_ptr(p) != ZERO_GLUE {
                            self.print_char(b' ' as i32);
                        }
                    }
                    MATH_NODE => {
                        if self.subtype(p) >= L_CODE {
                            self.print_s(b"[]");
                        } else {
                            self.print_char(b'$' as i32);
                        }
                    }
                    LIGATURE_NODE => self.short_display(self.lig_ptr(p)),
                    DISC_NODE => {
                        self.short_display(self.pre_break(p));
                        self.short_display(self.post_break(p));
                        n = self.replace_count(p);
                        while n > 0 {
                            if self.link(p) != NULL {
                                p = self.link(p);
                            }
                            n -= 1;
                        }
                    }
                    _ => {}
                }
            }
            p = self.link(p);
        }
    }

    /// §194
    pub fn print_font_and_char(&mut self, p: i32) {
        if p > self.mem_end {
            self.print_esc_s(b"CLOBBERED.");
        } else {
            if self.font(p) > self.font_max {
                self.print_char(b'*' as i32);
            } else {
                self.print_font_identifier(self.font(p));
            }
            self.print_char(b' ' as i32);
            self.print(self.character(p));
        }
    }

    /// §194
    pub fn print_mark(&mut self, p: i32) {
        self.print_char(b'{' as i32);
        // docs/PORTING.md section 15 has p<0 here.  p=0 is clobbered as well: the oracle's null, and the 0 that
        // `\pdfrefobj` of a reserved object reads from pdf_mem, are both below hi_mem_min, so it prints
        // `{\CLOBBERED.}`.  No list made by scan_toks is 0 here: tb_commit_ref always makes a block.
        if p <= 0 || p >= self.toks_top {
            self.print_esc_s(b"CLOBBERED.");
        } else {
            self.show_token_list(p + 1, NULL, self.max_print_line - 10);
        }
        self.print_char(b'}' as i32);
    }

    /// §194
    pub fn print_rule_dimen(&mut self, d: i32) {
        if self.is_running(d) {
            self.print_char(b'*' as i32);
        } else {
            self.print_scaled(d);
        }
    }

    /// §195
    pub fn print_glue(&mut self, d: i32, order: i32, s: &[u8]) {
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
        }
    }

    /// §196
    pub fn print_spec(&mut self, p: i32, s: &[u8]) {
        if p < MEM_MIN || p >= self.lo_mem_max {
            self.print_char(b'*' as i32);
        } else {
            self.print_scaled(self.width(p));
            if !s.is_empty() {
                self.print_s(s);
            }
            if self.stretch(p) != 0 {
                self.print_s(b" plus ");
                self.print_glue(self.stretch(p), self.stretch_order(p), s);
            }
            if self.shrink(p) != 0 {
                self.print_s(b" minus ");
                self.print_glue(self.shrink(p), self.shrink_order(p), s);
            }
        }
    }

    /// §1601 Display <rule spec> for whatsit node created by pdfTeX
    fn show_pdf_rule_spec(&mut self, p: i32) {
        self.print(b'(' as i32);
        self.print_rule_dimen(self.pdf_height(p));
        self.print_char(b'+' as i32);
        self.print_rule_dimen(self.pdf_depth(p));
        self.print_s(b")x");
        self.print_rule_dimen(self.pdf_width(p));
    }

    /// §1603 Display the whatsit node p
    fn show_whatsit(&mut self, p: i32) {
        match self.subtype(p) {
            OPEN_NODE => {
                self.print_write_whatsit(b"openout", p);
                self.print_char(b'=' as i32);
                self.print_file_name(self.open_name(p), self.open_area(p), self.open_ext(p));
            }
            WRITE_NODE => {
                self.print_write_whatsit(b"write", p);
                self.print_mark(self.write_tokens(p));
            }
            CLOSE_NODE => self.print_write_whatsit(b"closeout", p),
            SPECIAL_NODE => {
                self.print_esc_s(b"special");
                self.print_mark(self.write_tokens(p));
            }
            LATESPECIAL_NODE => {
                self.print_esc_s(b"special");
                self.print_s(b" shipout");
                self.print_mark(self.write_tokens(p));
            }
            LANGUAGE_NODE => {
                self.print_esc_s(b"setlanguage");
                self.print_int(self.what_lang(p));
                self.print_s(b" (hyphenmin ");
                self.print_int(self.what_lhm(p));
                self.print_char(b',' as i32);
                self.print_int(self.what_rhm(p));
                self.print_char(b')' as i32);
            }
            PDF_LITERAL_NODE | PDF_LATELITERAL_NODE => {
                self.print_esc_s(b"pdfliteral");
                if self.subtype(p) == PDF_LATELITERAL_NODE {
                    self.print_s(b" shipout");
                }
                match self.pdf_literal_mode(p) {
                    SET_ORIGIN => {}
                    DIRECT_PAGE => self.print_s(b" page"),
                    DIRECT_ALWAYS => self.print_s(b" direct"),
                    _ => self.confusion(b"literal2"),
                }
                self.print_mark(self.pdf_literal_data(p));
            }
            PDF_COLORSTACK_NODE => {
                self.print_esc_s(b"pdfcolorstack ");
                self.print_int(self.pdf_colorstack_stack(p));
                match self.pdf_colorstack_cmd(p) {
                    COLORSTACK_SET => self.print_s(b" set "),
                    COLORSTACK_PUSH => self.print_s(b" push "),
                    COLORSTACK_POP => self.print_s(b" pop"),
                    COLORSTACK_CURRENT => self.print_s(b" current"),
                    _ => self.confusion(b"pdfcolorstack"),
                }
                if self.pdf_colorstack_cmd(p) <= COLORSTACK_DATA {
                    self.print_mark(self.pdf_colorstack_data(p));
                }
            }
            PDF_SETMATRIX_NODE => {
                self.print_esc_s(b"pdfsetmatrix");
                self.print_mark(self.pdf_setmatrix_data(p));
            }
            PDF_SAVE_NODE => self.print_esc_s(b"pdfsave"),
            PDF_RESTORE_NODE => self.print_esc_s(b"pdfrestore"),
            PDF_REFOBJ_NODE => {
                self.print_esc_s(b"pdfrefobj");
                if self.obj_obj_is_stream(self.pdf_obj_objnum(p)) > 0 {
                    if self.obj_obj_stream_attr(self.pdf_obj_objnum(p)) != NULL {
                        self.print_s(b" attr");
                        self.print_mark(self.obj_obj_stream_attr(self.pdf_obj_objnum(p)));
                    }
                    self.print_s(b" stream");
                }
                if self.obj_obj_is_file(self.pdf_obj_objnum(p)) > 0 {
                    self.print_s(b" file");
                }
                self.print_mark(self.obj_obj_data(self.pdf_obj_objnum(p)));
            }
            PDF_REFXFORM_NODE => {
                self.print_esc_s(b"pdfrefxform");
                self.print(b'(' as i32);
                self.print_scaled(self.obj_xform_height(self.pdf_xform_objnum(p)));
                self.print_char(b'+' as i32);
                self.print_scaled(self.obj_xform_depth(self.pdf_xform_objnum(p)));
                self.print_s(b")x");
                self.print_scaled(self.obj_xform_width(self.pdf_xform_objnum(p)));
            }
            PDF_REFXIMAGE_NODE => {
                self.print_esc_s(b"pdfrefximage");
                self.print(b'(' as i32);
                self.print_scaled(self.obj_ximage_height(self.pdf_ximage_objnum(p)));
                self.print_char(b'+' as i32);
                self.print_scaled(self.obj_ximage_depth(self.pdf_ximage_objnum(p)));
                self.print_s(b")x");
                self.print_scaled(self.obj_ximage_width(self.pdf_ximage_objnum(p)));
            }
            PDF_ANNOT_NODE => {
                self.print_esc_s(b"pdfannot");
                self.show_pdf_rule_spec(p); // §1601
                self.print_mark(self.pdf_annot_data(p));
            }
            PDF_START_LINK_NODE => {
                self.print_esc_s(b"pdfstartlink");
                self.show_pdf_rule_spec(p); // §1601
                if self.pdf_link_attr(p) != NULL {
                    self.print_s(b" attr");
                    self.print_mark(self.pdf_link_attr(p));
                }
                self.print_s(b" action");
                let a = self.pdf_link_action(p);
                if self.pdf_action_type(a) == PDF_ACTION_USER {
                    self.print_s(b" user");
                    self.print_mark(self.pdf_action_user_tokens(a));
                } else {
                    if self.pdf_action_file(a) != NULL {
                        self.print_s(b" file");
                        self.print_mark(self.pdf_action_file(a));
                    }
                    match self.pdf_action_type(a) {
                        PDF_ACTION_GOTO => {
                            if self.pdf_action_named_id(a) % 2 == 1 {
                                self.print_s(b" goto name");
                                self.print_mark(self.pdf_action_id(a));
                            } else {
                                self.print_s(b" goto num");
                                self.print_int(self.pdf_action_id(a));
                            }
                        }
                        PDF_ACTION_PAGE => {
                            self.print_s(b" page");
                            self.print_int(self.pdf_action_id(a));
                            self.print_mark(self.pdf_action_page_tokens(a));
                        }
                        PDF_ACTION_THREAD => {
                            if self.pdf_action_named_id(a) % 2 == 1 {
                                self.print_s(b" thread name");
                                self.print_mark(self.pdf_action_id(a));
                            } else {
                                self.print_s(b" thread num");
                                self.print_int(self.pdf_action_id(a));
                            }
                        }
                        _ => self.pdf_error(b"displaying", b"unknown action type"),
                    }
                }
            }
            PDF_END_LINK_NODE => self.print_esc_s(b"pdfendlink"),
            PDF_DEST_NODE => {
                self.print_esc_s(b"pdfdest");
                if self.pdf_dest_objnum(p) != NULL {
                    self.print_s(b" struct");
                    self.print_int(self.pdf_dest_objnum(p));
                }
                if self.pdf_dest_named_id(p) > 0 {
                    self.print_s(b" name");
                    self.print_mark(self.pdf_dest_id(p));
                } else {
                    self.print_s(b" num");
                    self.print_int(self.pdf_dest_id(p));
                }
                self.print(b' ' as i32);
                match self.pdf_dest_type(p) {
                    PDF_DEST_XYZ => {
                        self.print_s(b"xyz");
                        // PDF_ZOOM_NULL for WEB's null: a zoom of 0 is a number, docs/PORTING.md section 6
                        if self.pdf_dest_xyz_zoom(p) != PDF_ZOOM_NULL {
                            self.print_s(b" zoom");
                            self.print_int(self.pdf_dest_xyz_zoom(p));
                        }
                    }
                    PDF_DEST_FITBH => self.print_s(b"fitbh"),
                    PDF_DEST_FITBV => self.print_s(b"fitbv"),
                    PDF_DEST_FITB => self.print_s(b"fitb"),
                    PDF_DEST_FITH => self.print_s(b"fith"),
                    PDF_DEST_FITV => self.print_s(b"fitv"),
                    PDF_DEST_FITR => {
                        self.print_s(b"fitr");
                        self.show_pdf_rule_spec(p); // §1601
                    }
                    PDF_DEST_FIT => self.print_s(b"fit"),
                    _ => self.print_s(b"unknown!"),
                }
            }
            PDF_THREAD_NODE | PDF_START_THREAD_NODE => {
                if self.subtype(p) == PDF_THREAD_NODE {
                    self.print_esc_s(b"pdfthread");
                } else {
                    self.print_esc_s(b"pdfstartthread");
                }
                self.print(b'(' as i32);
                self.print_rule_dimen(self.pdf_height(p));
                self.print_char(b'+' as i32);
                self.print_rule_dimen(self.pdf_depth(p));
                self.print_s(b")x");
                self.print_rule_dimen(self.pdf_width(p));
                if self.pdf_thread_attr(p) != NULL {
                    self.print_s(b" attr");
                    self.print_mark(self.pdf_thread_attr(p));
                }
                if self.pdf_thread_named_id(p) > 0 {
                    self.print_s(b" name");
                    self.print_mark(self.pdf_thread_id(p));
                } else {
                    self.print_s(b" num");
                    self.print_int(self.pdf_thread_id(p));
                }
            }
            PDF_END_THREAD_NODE => self.print_esc_s(b"pdfendthread"),
            PDF_SAVE_POS_NODE => self.print_esc_s(b"pdfsavepos"),
            PDF_SNAP_REF_POINT_NODE => self.print_esc_s(b"pdfsnaprefpoint"),
            PDF_SNAPY_NODE => {
                self.print_esc_s(b"pdfsnapy");
                self.print_char(b' ' as i32);
                self.print_spec(self.snap_glue_ptr(p), b"");
                self.print_char(b' ' as i32);
                // final_skip is a dimension, and WEB prints it as a pointer to a glue specification.  It is 0 until
                // ship_out, §1573, and on the oracle, whose mem_bot is 0, that is zero_glue: `\pdfsnapy ... 0.0`.  Our
                // mem_bot is 1 and 0 is null, which would print `*`.  Adding mem_bot reads the same word as the oracle.
                self.print_spec(self.final_skip(p) + MEM_BOT, b"");
            }
            PDF_SNAPY_COMP_NODE => {
                self.print_esc_s(b"pdfsnapycomp");
                self.print_char(b' ' as i32);
                self.print_int(self.snapy_comp_ratio(p));
            }
            PDF_INTERWORD_SPACE_ON_NODE => self.print_esc_s(b"pdfinterwordspaceon"),
            PDF_INTERWORD_SPACE_OFF_NODE => self.print_esc_s(b"pdfinterwordspaceoff"),
            PDF_FAKE_SPACE_NODE => self.print_esc_s(b"pdffakespace"),
            PDF_RUNNING_LINK_OFF_NODE => self.print_esc_s(b"pdfrunninglinkoff"),
            PDF_RUNNING_LINK_ON_NODE => self.print_esc_s(b"pdfrunninglinkon"),
            _ => self.print_s(b"whatsit?"),
        }
    }

    /// §200
    pub fn show_node_list(&mut self, p: i32) {
        let mut p = p;
        let mut n: i32;
        let mut g: f64;
        if self.cur_length() > self.depth_threshold {
            if p > NULL {
                self.print_s(b" []");
            }
            return;
        }
        n = 0;
        while p > MEM_MIN {
            self.print_ln();
            self.print_current_string();
            if p > self.mem_end {
                self.print_s(b"Bad link, display aborted.");
                return;
            }
            n += 1;
            if n > self.breadth_max {
                self.print_s(b"etc.");
                return;
            }
            // §201 Display node p
            if self.is_char_node(p) {
                self.print_font_and_char(p);
            } else {
                match self.ty(p) {
                    HLIST_NODE | VLIST_NODE | UNSET_NODE => {
                        // §202 Display box p
                        if self.ty(p) == HLIST_NODE {
                            self.print_esc(b'h' as i32);
                        } else if self.ty(p) == VLIST_NODE {
                            self.print_esc(b'v' as i32);
                        } else {
                            self.print_esc_s(b"unset");
                        }
                        self.print_s(b"box(");
                        self.print_scaled(self.height(p));
                        self.print_char(b'+' as i32);
                        self.print_scaled(self.depth(p));
                        self.print_s(b")x");
                        self.print_scaled(self.width(p));
                        if self.ty(p) == UNSET_NODE {
                            // §203 Display special fields of the unset node p
                            if self.span_count(p) != MIN_QUARTERWORD {
                                self.print_s(b" (");
                                self.print_int(self.span_count(p) + 1);
                                self.print_s(b" columns)");
                            }
                            if self.glue_stretch(p) != 0 {
                                self.print_s(b", stretch ");
                                self.print_glue(self.glue_stretch(p), self.glue_order(p), b"");
                            }
                            if self.glue_shrink(p) != 0 {
                                self.print_s(b", shrink ");
                                self.print_glue(self.glue_shrink(p), self.glue_sign(p), b"");
                            }
                        } else {
                            // §204 Display the value of glue_set(p)
                            g = self.glue_set(p);
                            if g != 0.0 && self.glue_sign(p) != NORMAL {
                                self.print_s(b", glue set ");
                                if self.glue_sign(p) == SHRINKING {
                                    self.print_s(b"- ");
                                }
                                if g.abs() > 20000.0 {
                                    if g > 0.0 {
                                        self.print_char(b'>' as i32);
                                    } else {
                                        self.print_s(b"< -");
                                    }
                                    self.print_glue(20000 * UNITY, self.glue_order(p), b"");
                                } else {
                                    self.print_glue(round(UNITY as f64 * g), self.glue_order(p), b"");
                                }
                            }
                            if self.shift_amount(p) != 0 {
                                self.print_s(b", shifted ");
                                self.print_scaled(self.shift_amount(p));
                            }
                            if self.eTeX_ex() {
                                // §1704 Display if this box is never to be reversed
                                if self.ty(p) == HLIST_NODE && self.box_lr(p) == DLIST {
                                    self.print_s(b", display");
                                }
                            }
                        }
                        self.node_list_display(self.list_ptr(p)); // recursive call
                    }
                    RULE_NODE => {
                        // §205 Display rule p
                        self.print_esc_s(b"rule(");
                        self.print_rule_dimen(self.height(p));
                        self.print_char(b'+' as i32);
                        self.print_rule_dimen(self.depth(p));
                        self.print_s(b")x");
                        self.print_rule_dimen(self.width(p));
                    }
                    INS_NODE => {
                        // §206 Display insertion p
                        self.print_esc_s(b"insert");
                        self.print_int(self.subtype(p));
                        self.print_s(b", natural size ");
                        self.print_scaled(self.height(p));
                        self.print_s(b"; split(");
                        self.print_spec(self.split_top_ptr(p), b"");
                        self.print_char(b',' as i32);
                        self.print_scaled(self.depth(p));
                        self.print_s(b"); float cost ");
                        self.print_int(self.float_cost(p));
                        self.node_list_display(self.ins_ptr(p)); // recursive call
                    }
                    WHATSIT_NODE => self.show_whatsit(p), // §1603
                    GLUE_NODE => {
                        // §207 Display glue p
                        if self.subtype(p) >= A_LEADERS {
                            // §208 Display leaders p
                            self.print_esc_s(b"");
                            if self.subtype(p) == C_LEADERS {
                                self.print_char(b'c' as i32);
                            } else if self.subtype(p) == X_LEADERS {
                                self.print_char(b'x' as i32);
                            }
                            self.print_s(b"leaders ");
                            self.print_spec(self.glue_ptr(p), b"");
                            self.node_list_display(self.leader_ptr(p)); // recursive call
                        } else {
                            self.print_esc_s(b"glue");
                            if self.subtype(p) != NORMAL {
                                self.print_char(b'(' as i32);
                                if self.subtype(p) < COND_MATH_GLUE {
                                    self.print_skip_param(self.subtype(p) - 1);
                                } else if self.subtype(p) == COND_MATH_GLUE {
                                    self.print_esc_s(b"nonscript");
                                } else {
                                    self.print_esc_s(b"mskip");
                                }
                                self.print_char(b')' as i32);
                            }
                            if self.subtype(p) != COND_MATH_GLUE {
                                self.print_char(b' ' as i32);
                                if self.subtype(p) < COND_MATH_GLUE {
                                    self.print_spec(self.glue_ptr(p), b"");
                                } else {
                                    self.print_spec(self.glue_ptr(p), b"mu");
                                }
                            }
                        }
                    }
                    MARGIN_KERN_NODE => {
                        self.print_esc_s(b"kern");
                        self.print_scaled(self.width(p));
                        if self.subtype(p) == LEFT_SIDE {
                            self.print_s(b" (left margin)");
                        } else {
                            self.print_s(b" (right margin)");
                        }
                    }
                    KERN_NODE => {
                        // §209 Display kern p
                        if self.subtype(p) != MU_GLUE {
                            self.print_esc_s(b"kern");
                            if self.subtype(p) != NORMAL {
                                self.print_char(b' ' as i32);
                            }
                            self.print_scaled(self.width(p));
                            if self.subtype(p) == ACC_KERN {
                                self.print_s(b" (for accent)");
                            }
                            if self.subtype(p) == AUTO_KERN {
                                self.print_s(b" (for \\pdfprependkern/\\pdfappendkern)");
                            }
                        } else {
                            self.print_esc_s(b"mkern");
                            self.print_scaled(self.width(p));
                            self.print_s(b"mu");
                        }
                    }
                    MATH_NODE => {
                        // §210 Display math node p
                        if self.subtype(p) > AFTER {
                            if self.end_LR(p) {
                                self.print_esc_s(b"end");
                            } else {
                                self.print_esc_s(b"begin");
                            }
                            if self.subtype(p) > R_CODE {
                                self.print_char(b'R' as i32);
                            } else if self.subtype(p) > L_CODE {
                                self.print_char(b'L' as i32);
                            } else {
                                self.print_char(b'M' as i32);
                            }
                        } else {
                            self.print_esc_s(b"math");
                            if self.subtype(p) == BEFORE {
                                self.print_s(b"on");
                            } else {
                                self.print_s(b"off");
                            }
                            if self.width(p) != 0 {
                                self.print_s(b", surrounded ");
                                self.print_scaled(self.width(p));
                            }
                        }
                    }
                    LIGATURE_NODE => {
                        // §211 Display ligature p
                        self.print_font_and_char(self.lig_char(p));
                        self.print_s(b" (ligature ");
                        if self.subtype(p) > 1 {
                            self.print_char(b'|' as i32);
                        }
                        self.font_in_short_display = self.font(self.lig_char(p));
                        self.short_display(self.lig_ptr(p));
                        if odd(self.subtype(p)) {
                            self.print_char(b'|' as i32);
                        }
                        self.print_char(b')' as i32);
                    }
                    PENALTY_NODE => {
                        // §212 Display penalty p
                        self.print_esc_s(b"penalty ");
                        self.print_int(self.penalty(p));
                    }
                    DISC_NODE => {
                        // §213 Display discretionary p
                        self.print_esc_s(b"discretionary");
                        if self.replace_count(p) > 0 {
                            self.print_s(b" replacing ");
                            self.print_int(self.replace_count(p));
                        }
                        self.node_list_display(self.pre_break(p)); // recursive call
                        self.append_char(b'|' as i32);
                        self.show_node_list(self.post_break(p)); // recursive call
                        self.flush_char();
                    }
                    MARK_NODE => {
                        // §214 Display mark p
                        self.print_esc_s(b"mark");
                        if self.mark_class(p) != 0 {
                            self.print_char(b's' as i32);
                            self.print_int(self.mark_class(p));
                        }
                        self.print_mark(self.mark_ptr(p));
                    }
                    ADJUST_NODE => {
                        // §215 Display adjustment p
                        self.print_esc_s(b"vadjust");
                        if self.adjust_pre(p) != 0 {
                            self.print_s(b" pre ");
                        }
                        self.node_list_display(self.adjust_ptr(p)); // recursive call
                    }
                    // §866 Cases of show_node_list that arise in mlists only
                    STYLE_NODE => self.print_style(self.subtype(p)),
                    CHOICE_NODE => {
                        // §871 Display choice node p
                        self.print_esc_s(b"mathchoice");
                        self.append_char(b'D' as i32);
                        self.show_node_list(self.display_mlist(p));
                        self.flush_char();
                        self.append_char(b'T' as i32);
                        self.show_node_list(self.text_mlist(p));
                        self.flush_char();
                        self.append_char(b'S' as i32);
                        self.show_node_list(self.script_mlist(p));
                        self.flush_char();
                        self.append_char(b's' as i32);
                        self.show_node_list(self.script_script_mlist(p));
                        self.flush_char();
                    }
                    ORD_NOAD | OP_NOAD | BIN_NOAD | REL_NOAD | OPEN_NOAD | CLOSE_NOAD | PUNCT_NOAD | INNER_NOAD
                    | RADICAL_NOAD | OVER_NOAD | UNDER_NOAD | VCENTER_NOAD | ACCENT_NOAD | LEFT_NOAD | RIGHT_NOAD => {
                        // §872 Display normal noad p
                        match self.ty(p) {
                            ORD_NOAD => self.print_esc_s(b"mathord"),
                            OP_NOAD => self.print_esc_s(b"mathop"),
                            BIN_NOAD => self.print_esc_s(b"mathbin"),
                            REL_NOAD => self.print_esc_s(b"mathrel"),
                            OPEN_NOAD => self.print_esc_s(b"mathopen"),
                            CLOSE_NOAD => self.print_esc_s(b"mathclose"),
                            PUNCT_NOAD => self.print_esc_s(b"mathpunct"),
                            INNER_NOAD => self.print_esc_s(b"mathinner"),
                            OVER_NOAD => self.print_esc_s(b"overline"),
                            UNDER_NOAD => self.print_esc_s(b"underline"),
                            VCENTER_NOAD => self.print_esc_s(b"vcenter"),
                            RADICAL_NOAD => {
                                self.print_esc_s(b"radical");
                                self.print_delimiter(self.left_delimiter(p));
                            }
                            ACCENT_NOAD => {
                                self.print_esc_s(b"accent");
                                self.print_fam_and_char(self.accent_chr(p));
                            }
                            LEFT_NOAD => {
                                self.print_esc_s(b"left");
                                self.print_delimiter(self.delimiter(p));
                            }
                            RIGHT_NOAD => {
                                if self.subtype(p) == NORMAL {
                                    self.print_esc_s(b"right");
                                } else {
                                    self.print_esc_s(b"middle");
                                }
                                self.print_delimiter(self.delimiter(p));
                            }
                            _ => {}
                        }
                        if self.ty(p) < LEFT_NOAD {
                            if self.subtype(p) != NORMAL {
                                if self.subtype(p) == LIMITS {
                                    self.print_esc_s(b"limits");
                                } else {
                                    self.print_esc_s(b"nolimits");
                                }
                            }
                            self.print_subsidiary_data(self.nucleus(p), b'.' as i32);
                        }
                        self.print_subsidiary_data(self.supscr(p), b'^' as i32);
                        self.print_subsidiary_data(self.subscr(p), b'_' as i32);
                    }
                    FRACTION_NOAD => {
                        // §873 Display fraction noad p
                        self.print_esc_s(b"fraction, thickness ");
                        if self.thickness(p) == DEFAULT_CODE {
                            self.print_s(b"= default");
                        } else {
                            self.print_scaled(self.thickness(p));
                        }
                        let d = self.left_delimiter(p);
                        if self.small_fam(d) != 0
                            || self.small_char(d) != MIN_QUARTERWORD
                            || self.large_fam(d) != 0
                            || self.large_char(d) != MIN_QUARTERWORD
                        {
                            self.print_s(b", left-delimiter ");
                            self.print_delimiter(d);
                        }
                        let d = self.right_delimiter(p);
                        if self.small_fam(d) != 0
                            || self.small_char(d) != MIN_QUARTERWORD
                            || self.large_fam(d) != 0
                            || self.large_char(d) != MIN_QUARTERWORD
                        {
                            self.print_s(b", right-delimiter ");
                            self.print_delimiter(d);
                        }
                        self.print_subsidiary_data(self.numerator(p), b'\\' as i32);
                        self.print_subsidiary_data(self.denominator(p), b'/' as i32);
                    }
                    _ => self.print_s(b"Unknown node type!"),
                }
            }
            p = self.link(p);
        }
    }

    /// §216
    pub fn show_box(&mut self, p: i32) {
        // §254 Assign the values depth_threshold:=show_box_depth and breadth_max:=show_box_breadth
        self.depth_threshold = self.show_box_depth();
        self.breadth_max = self.show_box_breadth();
        if self.breadth_max <= 0 {
            self.breadth_max = 5;
        }
        // In i64: \showboxdepth=2147483647 is usual, and on the oracle, whose integer is 64 bits, this does not wrap.
        if self.pool_ptr as i64 + self.depth_threshold as i64 >= self.pool_size as i64 {
            self.depth_threshold = self.pool_size - self.pool_ptr - 1;
        }
        // now there's enough room for prefix string
        self.show_node_list(p); // the show starts at p
        self.print_ln();
    }
}

/// Node lists built by hand, one for each case of `tests/oracle/gen_show_tables.py`, for the tests here and in copy.rs.
#[cfg(test)]
pub(crate) mod showkit {
    use super::*;
    use crate::mem::memkit::*;
    use crate::toks::testkit::*;

    pub const XX: i32 = 1;
    pub const YY: i32 = 2;
    pub const EE: i32 = 3;
    pub const XY: i32 = 4;
    pub const PLUS: i32 = 5;
    pub const MINUS: i32 = 6;

    pub fn pt(x: f64) -> i32 {
        round(x * 65536.0)
    }

    /// A string in the pool, if the strings are started: only the print tests need it.
    fn name(t: &mut Tex, s: &[u8]) -> i32 {
        if t.str_ptr == 0 {
            return S_EMPTY;
        }
        for &c in s {
            t.append_char(c as i32);
        }
        t.make_string()
    }

    /// The machine for a case: `mem` as INITEX leaves it, and what the case's preamble has set.
    pub fn case_machine(id: &str, printing: bool) -> &'static mut Tex {
        let t = mem_machine();
        if printing {
            print_setup(t);
        }
        t.eTeX_mode = 1;
        t.font_max = 20;
        t.set_show_box_depth(1000);
        t.set_show_box_breadth(1000);
        // The preamble's \baselineskip=12pt.
        let s = t.new_spec(ZERO_GLUE);
        t.set_width(s, pt(12.0));
        t.set_glue_par(BASELINE_SKIP_CODE, s);
        for (f, id_text, file) in [
            (XX, &b"xx"[..], &b"cmr10"[..]),
            (YY, b"yy", b"cmmi10"),
            (EE, b"ee", b"ecrm1000"),
            (XY, b"xy", b"cmr10"),
            (PLUS, b"xx", b"cmr10"),
            (MINUS, b"xx", b"cmr10"),
        ] {
            let n = name(t, id_text);
            t.set_font_id_text(f, n);
            t.font_name[f] = name(t, file);
            t.font_size[f] = pt(10.0);
            t.font_dsize[f] = pt(10.0);
        }
        t.font_size[XY] = pt(12.0);
        t.pdf_font_blink[PLUS] = XX;
        t.pdf_font_blink[MINUS] = XX;
        t.pdf_font_expand_ratio[PLUS] = 20;
        t.pdf_font_expand_ratio[MINUS] = -15;
        setup(t, id);
        t
    }

    /// What the case's `setup` has done, before the memory is counted.
    fn setup(t: &mut Tex, id: &str) {
        match id {
            "tracing_fonts" => t.set_pdf_tracing_fonts(1),
            "glue_lineskip" => {
                let s = t.new_spec(ZERO_GLUE);
                t.set_width(s, pt(3.0));
                t.set_glue_par(LINE_SKIP_CODE, s);
            }
            "unset_cells" => {
                let s = spec(t, pt(1.0), pt(2.0), FIL, pt(3.0), FILL);
                t.set_glue_par(TAB_SKIP_CODE, s);
            }
            "pdfrefobj" => {
                // \pdfobj{ab}, \pdfobj stream{c}, \pdfobj stream attr{/A 1}{de}, \pdfobj file{f}
                let (a, b, c, ab, d) = (toks(t, b"ab"), toks(t, b"c"), toks(t, b"de"), toks(t, b"/A 1"), toks(t, b"f"));
                for (n, data, stream, attr, file) in [(1, a, 0, NULL, 0), (2, b, 1, NULL, 0), (3, c, 1, ab, 0), (4, d, 0, NULL, 1)] {
                    t.set_obj_data_ptr(n, 100 + 10 * n);
                    t.set_obj_obj_data(n, data);
                    t.set_obj_obj_is_stream(n, stream);
                    t.set_obj_obj_stream_attr(n, attr);
                    t.set_obj_obj_is_file(n, file);
                }
            }
            "pdfrefobj_reserved" => t.set_obj_data_ptr(1, 0), // §1544: nothing is allocated for it
            "pdfrefxform" => {
                t.set_obj_data_ptr(1, 100);
                t.set_obj_xform_width(1, pt(5.0));
                t.set_obj_xform_height(1, pt(6.0));
                t.set_obj_xform_depth(1, pt(7.0));
            }
            "pdfrefximage" => {
                t.set_obj_data_ptr(1, 100);
                t.set_obj_ximage_width(1, pt(5.0));
                t.set_obj_ximage_height(1, pt(6.0));
                t.set_obj_ximage_depth(1, pt(7.0));
            }
            "limit_depth0" | "limit_depth_noad" => t.set_show_box_depth(0),
            "limit_depth1" | "limit_depth_noad1" => t.set_show_box_depth(1),
            "limit_depth_negative" => t.set_show_box_depth(-1),
            "limit_depth_max" => t.set_show_box_depth(2147483647),
            "limit_breadth1" => t.set_show_box_breadth(1),
            "limit_breadth2" => t.set_show_box_breadth(2),
            "limit_breadth3" => t.set_show_box_breadth(3),
            "limit_breadth0" => t.set_show_box_breadth(0),
            "limit_breadth_negative" => t.set_show_box_breadth(-3),
            "limit_breadth_max" => t.set_show_box_breadth(2147483647),
            _ => {}
        }
    }

    pub fn chain(t: &mut Tex, v: &[i32]) -> i32 {
        for w in v.windows(2) {
            let mut e = w[0];
            // The first of two nodes may head a list of its own, as a list passed in flat.
            while t.link(e) != NULL {
                e = t.link(e);
            }
            t.set_link(e, w[1]);
        }
        v.first().copied().unwrap_or(NULL)
    }

    pub fn boxed(t: &mut Tex, ty: i32, v: &[i32]) -> i32 {
        let l = chain(t, v);
        let b = t.new_null_box();
        t.set_ty(b, ty);
        t.set_list_ptr(b, l);
        b
    }
    pub fn h(t: &mut Tex, v: &[i32]) -> i32 {
        boxed(t, HLIST_NODE, v)
    }
    pub fn v(t: &mut Tex, l: &[i32]) -> i32 {
        boxed(t, VLIST_NODE, l)
    }
    /// A box with its glue set.
    fn gs(t: &mut Tex, l: &[i32], sign: i32, order: i32, ratio: f64) -> i32 {
        let b = h(t, l);
        t.set_glue_sign(b, sign);
        t.set_glue_order(b, order);
        t.set_glue_set(b, ratio);
        b
    }
    fn wd(t: &mut Tex, b: i32, w: f64) -> i32 {
        t.set_width(b, pt(w));
        b
    }
    fn unset(t: &mut Tex, hh: f64, w: f64, l: &[i32]) -> i32 {
        let b = boxed(t, UNSET_NODE, l);
        t.set_height(b, pt(hh));
        t.set_width(b, pt(w));
        b
    }
    fn ch(t: &mut Tex, f: i32, c: u8) -> i32 {
        let p = t.get_avail();
        t.set_font(p, f);
        t.set_character(p, c as i32);
        p
    }
    fn spec(t: &mut Tex, w: i32, st: i32, so: i32, sh: i32, sho: i32) -> i32 {
        let s = t.new_spec(ZERO_GLUE);
        t.set_width(s, w);
        t.set_stretch(s, st);
        t.set_stretch_order(s, so);
        t.set_shrink(s, sh);
        t.set_shrink_order(s, sho);
        s
    }
    /// `\hskip`: append_glue §1238.
    fn glue(t: &mut Tex, w: f64, st: f64, so: i32, sh: f64, sho: i32) -> i32 {
        let s = spec(t, pt(w), pt(st), so, pt(sh), sho);
        let p = t.new_glue(s);
        t.set_glue_ref_count(s, NULL);
        p
    }
    fn gsub(t: &mut Tex, sub: i32, w: f64, st: f64, so: i32, sh: f64, sho: i32) -> i32 {
        let p = glue(t, w, st, so, sh, sho);
        t.set_subtype(p, sub);
        p
    }
    fn leaders(t: &mut Tex, sub: i32, w: f64, st: f64, so: i32, sh: f64, sho: i32, l: i32) -> i32 {
        let p = gsub(t, sub, w, st, so, sh, sho);
        t.set_leader_ptr(p, l);
        p
    }
    fn skip12(t: &mut Tex) -> i32 {
        let p = t.new_skip_param(BASELINE_SKIP_CODE);
        t.set_width(t.temp_ptr, pt(12.0));
        p
    }
    fn kern(t: &mut Tex, w: f64, sub: i32) -> i32 {
        let p = t.new_kern(pt(w));
        t.set_subtype(p, sub);
        p
    }
    fn ke(t: &mut Tex, w: f64) -> i32 {
        kern(t, w, EXPLICIT)
    }
    fn margin(t: &mut Tex, w: f64, side: i32, c: u8) -> i32 {
        let p = t.get_node(MARGIN_KERN_NODE_SIZE);
        t.set_ty(p, MARGIN_KERN_NODE);
        t.set_subtype(p, side);
        t.set_width(p, pt(w));
        let m = ch(t, XX, c);
        t.set_margin_char(p, m);
        p
    }
    fn rule(t: &mut Tex, hh: i32, d: i32, w: i32) -> i32 {
        let p = t.new_rule();
        t.set_height(p, hh);
        t.set_depth(p, d);
        t.set_width(p, w);
        p
    }
    /// The tokens of a text: letters, a space, and others.
    pub fn toks(t: &mut Tex, s: &[u8]) -> i32 {
        let m = t.tb_mark();
        for &c in s {
            t.tb_push(if c == b' ' {
                SPACE_TOKEN
            } else if c.is_ascii_alphabetic() {
                LETTER_TOKEN + c as i32
            } else {
                OTHER_TOKEN + c as i32
            });
        }
        t.tb_commit_ref(m)
    }
    fn whatsit(t: &mut Tex, sub: i32, size: i32) -> i32 {
        let p = t.get_node(size);
        t.set_ty(p, WHATSIT_NODE);
        t.set_subtype(p, sub);
        p
    }
    fn write_like(t: &mut Tex, sub: i32, stream: i32, s: &[u8]) -> i32 {
        let p = whatsit(t, sub, WRITE_NODE_SIZE);
        t.set_write_stream(p, stream);
        let l = toks(t, s);
        t.set_write_tokens(p, l);
        p
    }
    fn close(t: &mut Tex, stream: i32) -> i32 {
        let p = whatsit(t, CLOSE_NODE, WRITE_NODE_SIZE);
        t.set_write_stream(p, stream);
        t.set_write_tokens(p, NULL);
        p
    }
    fn open(t: &mut Tex, stream: i32, n: &[u8], e: &[u8]) -> i32 {
        let p = whatsit(t, OPEN_NODE, OPEN_NODE_SIZE);
        t.set_write_stream(p, stream);
        let nm = name(t, n);
        t.set_open_name(p, nm);
        t.set_open_area(p, S_EMPTY);
        let ex = if e.is_empty() { S_EMPTY } else { name(t, e) };
        t.set_open_ext(p, ex);
        p
    }
    fn lang(t: &mut Tex, l: i32, a: i32, b: i32) -> i32 {
        let p = whatsit(t, LANGUAGE_NODE, SMALL_NODE_SIZE);
        t.set_what_lang(p, l);
        t.set_what_lhm(p, a);
        t.set_what_rhm(p, b);
        p
    }
    fn literal(t: &mut Tex, sub: i32, mode: i32, s: &[u8]) -> i32 {
        let p = whatsit(t, sub, WRITE_NODE_SIZE);
        t.set_pdf_literal_mode(p, mode);
        let l = toks(t, s);
        t.set_pdf_literal_data(p, l);
        p
    }
    fn colorstack(t: &mut Tex, cmd: i32, s: &[u8]) -> i32 {
        let size = if cmd <= COLORSTACK_DATA { PDF_COLORSTACK_SETTER_NODE_SIZE } else { PDF_COLORSTACK_GETTER_NODE_SIZE };
        let p = whatsit(t, PDF_COLORSTACK_NODE, size);
        t.set_pdf_colorstack_stack(p, 0);
        t.set_pdf_colorstack_cmd(p, cmd);
        if cmd <= COLORSTACK_DATA {
            let l = toks(t, s);
            t.set_pdf_colorstack_data(p, l);
        }
        p
    }
    fn small(t: &mut Tex, sub: i32) -> i32 {
        whatsit(t, sub, SMALL_NODE_SIZE)
    }
    /// The rule spec of a pdfTeX whatsit: all running, or height, depth, width.
    fn pdf_rule(t: &mut Tex, p: i32, spec: Option<(f64, f64, f64)>) {
        let (a, b, c) = match spec {
            Some((a, b, c)) => (pt(a), pt(b), pt(c)),
            None => (NULL_FLAG, NULL_FLAG, NULL_FLAG),
        };
        t.set_pdf_height(p, a);
        t.set_pdf_depth(p, b);
        t.set_pdf_width(p, c);
    }
    fn annot(t: &mut Tex, r: Option<(f64, f64, f64)>, s: &[u8]) -> i32 {
        let p = whatsit(t, PDF_ANNOT_NODE, PDF_ANNOT_NODE_SIZE);
        pdf_rule(t, p, r);
        let l = toks(t, s);
        t.set_pdf_annot_data(p, l);
        p
    }
    fn action(t: &mut Tex, ty: i32, named: i32, id: i32, file: i32, third: i32) -> i32 {
        let a = t.get_node(PDF_ACTION_SIZE);
        t.set_pdf_action_type(a, ty);
        t.set_pdf_action_named_id(a, named);
        t.set_pdf_action_id(a, id);
        t.set_pdf_action_file(a, file);
        t.set_pdf_action_new_window(a, 0);
        // pdf_action_page_tokens and pdf_action_user_tokens are one field.
        t.set_pdf_action_user_tokens(a, third);
        t.set_pdf_action_refcount(a, NULL);
        a
    }
    fn start_link(t: &mut Tex, r: Option<(f64, f64, f64)>, attr: i32, a: i32) -> i32 {
        let p = whatsit(t, PDF_START_LINK_NODE, PDF_ANNOT_NODE_SIZE);
        pdf_rule(t, p, r);
        t.set_pdf_link_attr(p, attr);
        t.set_pdf_link_action(p, a);
        t.set_pdf_link_objnum(p, 0);
        p
    }
    fn link_case(t: &mut Tex, a: i32) -> i32 {
        let s = start_link(t, None, NULL, a);
        let e = small(t, PDF_END_LINK_NODE);
        h(t, &[s, e])
    }
    fn dest(t: &mut Tex, ty: i32, id: i32, named: i32, st: i32, zoom: i32) -> i32 {
        let p = whatsit(t, PDF_DEST_NODE, PDF_DEST_NODE_SIZE);
        t.set_pdf_dest_objnum(p, st);
        t.set_pdf_dest_id(p, id);
        t.set_pdf_dest_named_id(p, named);
        t.set_pdf_dest_type(p, ty);
        t.set_pdf_dest_xyz_zoom(p, zoom);
        p
    }
    fn thread(t: &mut Tex, sub: i32, r: Option<(f64, f64, f64)>, attr: i32, id: i32, named: i32) -> i32 {
        let p = whatsit(t, sub, PDF_THREAD_NODE_SIZE);
        pdf_rule(t, p, r);
        t.set_pdf_thread_attr(p, attr);
        t.set_pdf_thread_id(p, id);
        t.set_pdf_thread_named_id(p, named);
        p
    }
    fn mark(t: &mut Tex, class: i32, l: i32) -> i32 {
        let p = t.get_node(SMALL_NODE_SIZE);
        t.set_ty(p, MARK_NODE);
        t.set_subtype(p, 0);
        t.set_mark_class(p, class);
        t.set_mark_ptr(p, l);
        p
    }
    fn adjust(t: &mut Tex, pre: i32, l: &[i32]) -> i32 {
        let p = t.get_node(SMALL_NODE_SIZE);
        t.set_ty(p, ADJUST_NODE);
        t.set_adjust_pre(p, pre);
        let c = chain(t, l);
        t.set_adjust_ptr(p, c);
        p
    }
    fn insert(t: &mut Tex, class: i32, size: f64, top: i32, dp: f64, cost: i32, l: &[i32]) -> i32 {
        let p = t.get_node(INS_NODE_SIZE);
        t.set_ty(p, INS_NODE);
        t.set_subtype(p, class);
        t.set_height(p, pt(size));
        t.set_depth(p, pt(dp));
        t.set_split_top_ptr(p, top);
        t.set_float_cost(p, cost);
        let c = chain(t, l);
        t.set_ins_ptr(p, c);
        p
    }
    fn disc(t: &mut Tex, rep: i32, pre: &[i32], post: &[i32]) -> i32 {
        let p = t.new_disc();
        t.set_replace_count(p, rep);
        let c = chain(t, pre);
        t.set_pre_break(p, c);
        let c = chain(t, post);
        t.set_post_break(p, c);
        p
    }
    fn line(t: &mut Tex, width: f64, l: &[i32]) -> i32 {
        let b = h(t, l);
        wd(t, b, width)
    }
    fn par_end(t: &mut Tex) -> [i32; 3] {
        let p = t.new_penalty(10000);
        let a = t.new_param_glue(PAR_FILL_SKIP_CODE);
        let b = t.new_param_glue(RIGHT_SKIP_CODE);
        [p, a, b]
    }

    // ---- noads ----

    fn noad(t: &mut Tex, ty: i32, size: i32) -> i32 {
        let p = t.get_node(size);
        t.set_ty(p, ty);
        t.set_subtype(p, NORMAL);
        for k in 1..size {
            t.mem[p + k] = Word::ZERO;
        }
        p
    }
    fn set_char(t: &mut Tex, x: i32, f: i32, c: u8) {
        t.set_math_type(x, MATH_CHAR);
        t.set_fam(x, f);
        t.set_character(x, c as i32);
    }
    fn nc(t: &mut Tex, ty: i32, c: u8) -> i32 {
        let p = noad(t, ty, NOAD_SIZE);
        set_char(t, t.nucleus(p), 1, c);
        p
    }
    fn ord(t: &mut Tex, c: u8) -> i32 {
        nc(t, ORD_NOAD, c)
    }
    fn sub_box(t: &mut Tex, x: i32, b: i32) {
        t.set_math_type(x, SUB_BOX);
        t.set_info(x, b);
    }
    fn sub_mlist(t: &mut Tex, x: i32, l: i32) {
        t.set_math_type(x, SUB_MLIST);
        t.set_info(x, l);
    }
    fn delim(t: &mut Tex, d: i32, a: i32, b: i32, c: i32, e: i32) {
        t.set_small_fam(d, a);
        t.set_small_char(d, b);
        t.set_large_fam(d, c);
        t.set_large_char(d, e);
    }
    fn lparen(t: &mut Tex, d: i32) {
        delim(t, d, 0, 0x28, 3, 0x00);
    }
    fn rparen(t: &mut Tex, d: i32) {
        delim(t, d, 0, 0x29, 3, 0x01);
    }
    fn fraction(t: &mut Tex, thickness: i32, left: bool, right: bool) -> i32 {
        let p = noad(t, FRACTION_NOAD, FRACTION_NOAD_SIZE);
        t.set_thickness(p, thickness);
        let (a, b) = (ord(t, b'a'), ord(t, b'b'));
        sub_mlist(t, t.numerator(p), a);
        sub_mlist(t, t.denominator(p), b);
        if left {
            lparen(t, t.left_delimiter(p));
        }
        if right {
            rparen(t, t.right_delimiter(p));
        }
        let q = noad(t, ORD_NOAD, NOAD_SIZE);
        sub_mlist(t, t.nucleus(q), p);
        q
    }
    fn style(t: &mut Tex, s: i32) -> i32 {
        let p = t.get_node(STYLE_NODE_SIZE);
        t.set_ty(p, STYLE_NODE);
        t.set_subtype(p, s);
        p
    }
    fn mu_glue(t: &mut Tex, w: f64, st: f64, sh: f64, sho: i32) -> i32 {
        gsub(t, MU_GLUE, w, st, NORMAL, sh, sho)
    }

    /// The list of a case.  `show_box` of this is what the oracle wrote.
    pub fn build(t: &mut Tex, id: &str) -> i32 {
        const N: i32 = NORMAL;
        match id {
            "hbox_empty" => h(t, &[]),
            "vbox_empty" => v(t, &[]),
            "hbox_dims" => {
                let b = h(t, &[]);
                t.set_height(b, pt(1.0));
                t.set_depth(b, pt(2.0));
                t.set_width(b, pt(3.0));
                b
            }
            "vbox_neg_dims" => {
                let b = v(t, &[]);
                t.set_height(b, pt(-1.5));
                t.set_depth(b, pt(0.25));
                t.set_width(b, pt(-16383.99998));
                b
            }
            "glue_set_normal" => {
                let g = glue(t, 0.0, 3.0, N, 0.0, N);
                gs(t, &[g], STRETCHING, N, 100.0 / 3.0)
            }
            "glue_set_fil" => {
                let g = glue(t, 0.0, 3.0, FIL, 0.0, N);
                gs(t, &[g], STRETCHING, FIL, 100.0 / 3.0)
            }
            "glue_set_fill" => {
                let g = glue(t, 0.0, 1.0, FILL, 0.0, N);
                gs(t, &[g], STRETCHING, FILL, 100.0)
            }
            "glue_set_filll" => {
                let g = glue(t, 0.0, 8.0, FILLL, 0.0, N);
                gs(t, &[g], STRETCHING, FILLL, 12.5)
            }
            "glue_set_shrink" => {
                let (k, g) = (ke(t, 20.0), glue(t, 0.0, 0.0, N, 20.0, N));
                gs(t, &[k, g], SHRINKING, N, 0.5)
            }
            "glue_set_shrink_fil" => {
                let (k, g) = (ke(t, 20.0), glue(t, 0.0, 0.0, N, 4.0, FIL));
                gs(t, &[k, g], SHRINKING, FIL, 2.5)
            }
            "glue_set_huge" => {
                let g = glue(t, 0.0, 0.5, N, 0.0, N);
                gs(t, &[g], STRETCHING, N, 32000.0)
            }
            "glue_set_huge_fil" => {
                let g = glue(t, 0.0, 0.5, FIL, 0.0, N);
                gs(t, &[g], STRETCHING, FIL, 32000.0)
            }
            "glue_set_huge_neg" => {
                let g = glue(t, 0.0, -0.5, N, 0.0, N);
                gs(t, &[g], STRETCHING, N, -32000.0)
            }
            "glue_set_20000" => {
                let g = glue(t, 0.0, 0.5, N, 0.0, N);
                gs(t, &[g], STRETCHING, N, 20000.0)
            }
            "shifted" => {
                let a = h(t, &[]);
                t.set_shift_amount(a, pt(3.0));
                let s = skip12(t);
                let b = v(t, &[]);
                t.set_shift_amount(b, pt(-1.5));
                v(t, &[a, s, b])
            }
            "glue_set_and_shifted" => {
                let g = glue(t, 0.0, 1.0, N, 0.0, N);
                let a = gs(t, &[g], STRETCHING, N, 10.0);
                t.set_width(a, pt(10.0));
                t.set_shift_amount(a, pt(3.0));
                v(t, &[a])
            }
            "display" => {
                let (p1, g1) = (t.new_penalty(0), t.new_param_glue(ABOVE_DISPLAY_SHORT_SKIP_CODE));
                let i = h(t, &[]);
                let b = h(t, &[i]);
                t.set_shift_amount(b, pt(50.0));
                t.set_box_lr(b, DLIST);
                let (p2, g2) = (t.new_penalty(0), t.new_param_glue(BELOW_DISPLAY_SHORT_SKIP_CODE));
                v(t, &[p1, g1, b, p2, g2])
            }
            "chars" => {
                let (a, b, c) = (ch(t, XX, b'a'), ch(t, YY, b'b'), ch(t, XX, b'c'));
                h(t, &[a, b, c])
            }
            "chars_unprintable" => {
                let l = [ch(t, XX, 0), ch(t, XX, 31), ch(t, XX, 127), ch(t, XX, 32), ch(t, XX, b'\\'), ch(t, EE, 128), ch(t, EE, 255)];
                h(t, &l)
            }
            "tracing_fonts" => {
                let (a, b) = (ch(t, XX, b'a'), ch(t, XY, b'b'));
                h(t, &[a, b])
            }
            "font_expand" | "font_expand_shrink" => {
                let (f, w) = if id == "font_expand" { (PLUS, 11.0) } else { (MINUS, 10.4) };
                let (a, b) = (ch(t, f, b'a'), ch(t, f, b'b'));
                let e = par_end(t);
                let l = line(t, w, &[a, b, e[0], e[1], e[2]]);
                t.set_height(l, pt(6.94444));
                v(t, &[l])
            }
            "rule_all" => {
                let r = rule(t, pt(2.0), pt(3.0), pt(1.0));
                h(t, &[r])
            }
            "rule_running_v" => {
                let r = rule(t, NULL_FLAG, NULL_FLAG, DEFAULT_RULE);
                h(t, &[r])
            }
            "rule_running_h" => {
                let r = rule(t, DEFAULT_RULE, 0, NULL_FLAG);
                v(t, &[r])
            }
            "rule_mixed" => {
                let (a, b) = (rule(t, pt(1.0), pt(2.0), pt(3.0)), rule(t, DEFAULT_RULE, 0, NULL_FLAG));
                v(t, &[a, b])
            }
            "glue_natural" => {
                let g = glue(t, 1.0, 2.0, N, 3.0, N);
                h(t, &[g])
            }
            "glue_orders" => {
                let (a, b) = (glue(t, 1.0, 2.0, FIL, 3.0, FILL), glue(t, -1.0, 2.0, FILLL, 1.0, N));
                h(t, &[a, b])
            }
            "glue_zero_parts" => {
                let l = [glue(t, 0.0, 0.0, N, 0.0, N), glue(t, 0.0, 1.0, N, 0.0, N), glue(t, 0.0, 0.0, N, 1.0, N)];
                h(t, &l)
            }
            "glue_hfil" | "glue_vfil" => {
                let l = [t.new_glue(FIL_GLUE), t.new_glue(FILL_GLUE), t.new_glue(SS_GLUE), t.new_glue(FIL_NEG_GLUE)];
                if id == "glue_hfil" {
                    h(t, &l)
                } else {
                    v(t, &l)
                }
            }
            "glue_baselineskip" => {
                let (a, s, b) = (h(t, &[]), skip12(t), h(t, &[]));
                v(t, &[a, s, b])
            }
            "glue_lineskip" => {
                let (a, s, b) = (h(t, &[]), t.new_param_glue(LINE_SKIP_CODE), h(t, &[]));
                v(t, &[a, s, b])
            }
            "leaders_rule" => {
                let r = rule(t, DEFAULT_RULE, 0, NULL_FLAG);
                let g = leaders(t, A_LEADERS, 1.0, 0.0, N, 0.0, N, r);
                h(t, &[g])
            }
            "cleaders_box" => {
                let b = h(t, &[]);
                let g = leaders(t, C_LEADERS, 1.0, 1.0, FIL, 0.0, N, b);
                h(t, &[g])
            }
            "xleaders_box" => {
                let k = ke(t, 1.0);
                let b = v(t, &[k]);
                t.set_height(b, pt(1.0));
                let g = leaders(t, X_LEADERS, 2.0, 0.0, N, 1.0, N, b);
                v(t, &[g])
            }
            "kern_explicit" => {
                let l = [ke(t, 3.0), ke(t, -3.0), ke(t, 0.0)];
                h(t, &l)
            }
            "kern_font" => {
                let l = [ch(t, XX, b'A'), kern(t, -1.11113, N), ch(t, XX, b'V')];
                h(t, &l)
            }
            "kern_accent" => {
                let l = [kern(t, 0.0, ACC_KERN), ch(t, XX, 127), kern(t, -5.00002, ACC_KERN), ch(t, XX, b'a')];
                h(t, &l)
            }
            "kern_auto" => {
                let l = [kern(t, 10.00002, AUTO_KERN), ch(t, XX, b'a'), ch(t, XX, b'b'), kern(t, 10.00002, AUTO_KERN)];
                h(t, &l)
            }
            "margin_kern" => {
                let e = par_end(t);
                let l = [margin(t, -5.00002, LEFT_SIDE, b'a'), ch(t, XX, b'a'), ch(t, XX, b'b'), e[0], margin(t, -5.00002, RIGHT_SIDE, b'b'), e[1], e[2]];
                let b = line(t, 100.0, &l);
                t.set_height(b, pt(6.94444));
                v(t, &[b])
            }
            "math_plain" | "math_surround" => {
                let w = if id == "math_plain" { 0 } else { pt(3.0) };
                let l = [t.new_math(w, BEFORE), t.new_math(w, AFTER)];
                h(t, &l)
            }
            "math_lr" => {
                let l = [t.new_math(0, BEGIN_L_CODE), t.new_math(0, END_L_CODE), t.new_math(0, BEGIN_R_CODE), t.new_math(0, END_R_CODE)];
                h(t, &l)
            }
            "math_m" => {
                let e = par_end(t);
                let (br, er) = (t.new_math(0, BEGIN_R_CODE), t.new_math(0, END_R_CODE));
                let l1 = line(t, 100.0, &[br, e[0], e[1], er, e[2]]);
                let (p1, g1, s1) = (t.new_penalty(0), t.new_param_glue(ABOVE_DISPLAY_SHORT_SKIP_CODE), skip12(t));
                let i = h(t, &[]);
                let d = h(t, &[i]);
                t.set_box_lr(d, DLIST);
                let m = [t.new_math(0, BEGIN_M_CODE), kern(t, 50.0, N), d, kern(t, 50.0, N), t.new_math(0, END_M_CODE)];
                let l2 = line(t, 100.0, &m);
                let (p2, g2, s2) = (t.new_penalty(0), t.new_param_glue(BELOW_DISPLAY_SHORT_SKIP_CODE), skip12(t));
                let e = par_end(t);
                let (br, er) = (t.new_math(0, BEGIN_R_CODE), t.new_math(0, END_R_CODE));
                let l3 = line(t, 100.0, &[br, er, e[0], e[1], e[2]]);
                v(t, &[l1, p1, g1, s1, l2, p2, g2, s2, l3])
            }
            "ligature" => {
                let l = [ch(t, XX, b'f'), ch(t, XX, b'i')];
                let c = chain(t, &l);
                let p = t.new_ligature(XX, 12, c);
                h(t, &[p])
            }
            "ligature_three" => {
                let l = [ch(t, XX, b'f'), ch(t, XX, b'f'), ch(t, XX, b'i')];
                let c = chain(t, &l);
                let p = t.new_ligature(XX, 14, c);
                h(t, &[p])
            }
            "penalties" => {
                let l = [t.new_penalty(5), t.new_penalty(-100), t.new_penalty(0)];
                h(t, &l)
            }
            "disc_empty" => {
                let c = ch(t, XX, 0);
                let d = disc(t, 0, &[c], &[]);
                h(t, &[d])
            }
            "disc_full" => {
                let (a, k, b) = (ch(t, XX, b'a'), ke(t, 1.0), ch(t, XX, b'b'));
                let d = disc(t, 2, &[a], &[k, b]);
                let (c, e) = (ch(t, XX, b'c'), ke(t, 2.0));
                h(t, &[d, c, e])
            }
            "mark" | "mark_empty" => {
                let l = toks(t, if id == "mark" { b"abc" } else { b"" });
                let m = mark(t, 0, l);
                h(t, &[m])
            }
            "marks_class" => {
                let m_ = t.tb_mark();
                t.tb_push(LETTER_TOKEN + b'x' as i32);
                t.tb_push(cs_tok(b'y'));
                t.tb_push(LETTER_TOKEN + b'z' as i32);
                let l = t.tb_commit_ref(m_);
                let m = mark(t, 3, l);
                h(t, &[m])
            }
            "mark_long" => {
                let l = toks(t, b"01234567890123456789012345678901234567890123456789012345678901234567890123456789");
                let m = mark(t, 0, l);
                h(t, &[m])
            }
            "vadjust" => {
                let k = ke(t, 1.0);
                let a = adjust(t, 0, &[k]);
                h(t, &[a])
            }
            "vadjust_pre" => {
                let (k, p) = (ke(t, 1.0), t.new_penalty(3));
                let a = adjust(t, 1, &[k, p]);
                h(t, &[a])
            }
            "insert" => {
                let k = ke(t, 1.0);
                t.add_glue_ref(ZERO_GLUE);
                let i = insert(t, 100, 1.0, ZERO_GLUE, 0.0, 0, &[k]);
                h(t, &[i])
            }
            "insert_spec" => {
                let (p, k) = (t.new_penalty(1), ke(t, -1.0));
                let s = spec(t, pt(1.0), pt(2.0), N, pt(3.0), N);
                let i = insert(t, 5, -1.0, s, 4.0, 7, &[p, k]);
                h(t, &[i])
            }
            "write_streams" => {
                let l = [write_like(t, WRITE_NODE, 3, b"abc"), write_like(t, WRITE_NODE, 16, b"d"), write_like(t, WRITE_NODE, 17, b"e")];
                h(t, &l)
            }
            "openout" => {
                let l = [open(t, 3, b"foo", b".bar"), open(t, 4, b"bar", b"")];
                h(t, &l)
            }
            "openout_quoted" => {
                let l = [open(t, 3, b"a b", b".c")];
                h(t, &l)
            }
            "closeout" => {
                let l = [close(t, 3), close(t, 16), close(t, 17)];
                h(t, &l)
            }
            "special" => {
                let l = [write_like(t, SPECIAL_NODE, 0, b"abc"), write_like(t, LATESPECIAL_NODE, 0, b"d")];
                h(t, &l)
            }
            "setlanguage" => {
                let l = [lang(t, 3, 1, 1), lang(t, 4, 5, 6)];
                h(t, &l)
            }
            "pdfliteral" => {
                let l = [
                    literal(t, PDF_LITERAL_NODE, SET_ORIGIN, b"a"),
                    literal(t, PDF_LITERAL_NODE, DIRECT_PAGE, b"b"),
                    literal(t, PDF_LITERAL_NODE, DIRECT_ALWAYS, b"c"),
                    literal(t, PDF_LATELITERAL_NODE, SET_ORIGIN, b"d"),
                    literal(t, PDF_LATELITERAL_NODE, DIRECT_PAGE, b"e"),
                    literal(t, PDF_LATELITERAL_NODE, DIRECT_ALWAYS, b"f"),
                ];
                h(t, &l)
            }
            "pdfcolorstack" => {
                let l = [colorstack(t, COLORSTACK_SET, b"1 g"), colorstack(t, COLORSTACK_PUSH, b"2 g"), colorstack(t, COLORSTACK_POP, b""), colorstack(t, COLORSTACK_CURRENT, b"")];
                h(t, &l)
            }
            "pdfsetmatrix" => {
                let m = whatsit(t, PDF_SETMATRIX_NODE, PDF_SETMATRIX_NODE_SIZE);
                let d = toks(t, b"1 0");
                t.set_pdf_setmatrix_data(m, d);
                let (a, b) = (whatsit(t, PDF_SAVE_NODE, PDF_SAVE_NODE_SIZE), whatsit(t, PDF_RESTORE_NODE, PDF_RESTORE_NODE_SIZE));
                h(t, &[m, a, b])
            }
            "pdfrefobj" => {
                let mut l = [0; 4];
                for k in 0..4 {
                    l[k] = whatsit(t, PDF_REFOBJ_NODE, PDF_REFOBJ_NODE_SIZE);
                    t.set_pdf_obj_objnum(l[k], k as i32 + 1);
                }
                h(t, &l)
            }
            "pdfrefobj_reserved" => {
                let p = whatsit(t, PDF_REFOBJ_NODE, PDF_REFOBJ_NODE_SIZE);
                t.set_pdf_obj_objnum(p, 1);
                h(t, &[p])
            }
            "pdfrefxform" => {
                let p = whatsit(t, PDF_REFXFORM_NODE, PDF_REFXFORM_NODE_SIZE);
                pdf_rule(t, p, Some((6.0, 7.0, 5.0)));
                t.set_pdf_xform_objnum(p, 1);
                h(t, &[p])
            }
            "pdfrefximage" => {
                let p = whatsit(t, PDF_REFXIMAGE_NODE, PDF_REFXIMAGE_NODE_SIZE);
                pdf_rule(t, p, Some((6.0, 7.0, 5.0)));
                t.set_pdf_ximage_objnum(p, 1);
                h(t, &[p])
            }
            "pdfannot" => {
                let l = [annot(t, Some((2.0, 3.0, 1.0)), b"a"), annot(t, None, b"b")];
                h(t, &l)
            }
            "pdfstartlink_user" => {
                let u = toks(t, b"b");
                let a = action(t, PDF_ACTION_USER, 0, 0, NULL, u);
                let at = toks(t, b"a");
                let s = start_link(t, Some((2.0, 3.0, 1.0)), at, a);
                let e = small(t, PDF_END_LINK_NODE);
                h(t, &[s, e])
            }
            "pdfstartlink_goto_name" => {
                let n = toks(t, b"ab");
                let a = action(t, PDF_ACTION_GOTO, 1, n, NULL, NULL);
                link_case(t, a)
            }
            "pdfstartlink_goto_num" => {
                let a = action(t, PDF_ACTION_GOTO, 0, 7, NULL, NULL);
                link_case(t, a)
            }
            "pdfstartlink_goto_file" => {
                let (n, f) = (toks(t, b"ab"), toks(t, b"f"));
                let a = action(t, PDF_ACTION_GOTO, 1, n, f, NULL);
                link_case(t, a)
            }
            "pdfstartlink_page" => {
                let x = toks(t, b"xy");
                let a = action(t, PDF_ACTION_PAGE, 0, 3, NULL, x);
                link_case(t, a)
            }
            "pdfstartlink_page_file" => {
                let (x, f) = (toks(t, b"xy"), toks(t, b"f"));
                let a = action(t, PDF_ACTION_PAGE, 0, 3, f, x);
                link_case(t, a)
            }
            "pdfstartlink_thread_name" => {
                let n = toks(t, b"ab");
                let a = action(t, PDF_ACTION_THREAD, 1, n, NULL, NULL);
                link_case(t, a)
            }
            "pdfstartlink_thread_num" => {
                let a = action(t, PDF_ACTION_THREAD, 0, 7, NULL, NULL);
                link_case(t, a)
            }
            "pdfstartlink_thread_file" => {
                let f = toks(t, b"f");
                let a = action(t, PDF_ACTION_THREAD, 0, 7, f, NULL);
                link_case(t, a)
            }
            "pdfdest_num_xyz" => {
                let d = dest(t, PDF_DEST_XYZ, 5, 0, NULL, PDF_ZOOM_NULL);
                v(t, &[d])
            }
            "pdfdest_xyz_zoom" => {
                let d = dest(t, PDF_DEST_XYZ, 5, 0, NULL, 1000);
                v(t, &[d])
            }
            "pdfdest_xyz_zoom0" => {
                let d = dest(t, PDF_DEST_XYZ, 5, 0, NULL, 0);
                v(t, &[d])
            }
            "pdfdest_name" => {
                let n = toks(t, b"ab");
                let d = dest(t, PDF_DEST_FIT, n, 1, NULL, 0);
                v(t, &[d])
            }
            "pdfdest_struct" => {
                let d = dest(t, PDF_DEST_FIT, 5, 0, 9, 0);
                v(t, &[d])
            }
            "pdfdest_types" => {
                let l = [
                    dest(t, PDF_DEST_FIT, 1, 0, NULL, 0),
                    dest(t, PDF_DEST_FITH, 2, 0, NULL, 0),
                    dest(t, PDF_DEST_FITV, 3, 0, NULL, 0),
                    dest(t, PDF_DEST_FITB, 4, 0, NULL, 0),
                    dest(t, PDF_DEST_FITBH, 5, 0, NULL, 0),
                    dest(t, PDF_DEST_FITBV, 6, 0, NULL, 0),
                ];
                v(t, &l)
            }
            "pdfdest_fitr" => {
                let (a, b) = (dest(t, PDF_DEST_FITR, 1, 0, NULL, 0), dest(t, PDF_DEST_FITR, 2, 0, NULL, 0));
                pdf_rule(t, a, Some((2.0, 3.0, 1.0)));
                pdf_rule(t, b, None);
                v(t, &[a, b])
            }
            "pdfthread" => {
                let a = thread(t, PDF_THREAD_NODE, None, NULL, 5, 0);
                let n = toks(t, b"ab");
                let b = thread(t, PDF_THREAD_NODE, None, NULL, n, 1);
                let at = toks(t, b"c");
                let c = thread(t, PDF_THREAD_NODE, Some((2.0, 3.0, 1.0)), at, 6, 0);
                v(t, &[a, b, c])
            }
            "pdfstartthread" => {
                let (a, b) = (thread(t, PDF_START_THREAD_NODE, None, NULL, 5, 0), small(t, PDF_END_THREAD_NODE));
                v(t, &[a, b])
            }
            "pdfsavepos" => {
                let l = [small(t, PDF_SAVE_POS_NODE), small(t, PDF_SNAP_REF_POINT_NODE)];
                h(t, &l)
            }
            "pdfsnapy" => {
                let s = spec(t, pt(1.0), pt(2.0), N, pt(3.0), N);
                let p = whatsit(t, PDF_SNAPY_NODE, SNAP_NODE_SIZE);
                t.set_snap_glue_ptr(p, s);
                t.set_final_skip(p, 0);
                h(t, &[p])
            }
            "pdfsnapycomp" => {
                let p = small(t, PDF_SNAPY_COMP_NODE);
                t.set_snapy_comp_ratio(p, 5);
                h(t, &[p])
            }
            "pdfinterword" => {
                let l = [small(t, PDF_INTERWORD_SPACE_ON_NODE), small(t, PDF_INTERWORD_SPACE_OFF_NODE), small(t, PDF_FAKE_SPACE_NODE)];
                h(t, &l)
            }
            "pdfrunninglink" => {
                let l = [small(t, PDF_RUNNING_LINK_OFF_NODE), small(t, PDF_RUNNING_LINK_ON_NODE)];
                h(t, &l)
            }
            "unset_cells" => {
                let a = ch(t, XX, b'a');
                let ca = unset(t, 4.30554, 5.00002, &[a]);
                let b = ch(t, XX, b'b');
                let cb = unset(t, 6.94444, 5.55557, &[b]);
                let l = [t.new_param_glue(TAB_SKIP_CODE), ca, t.new_param_glue(TAB_SKIP_CODE), cb, t.new_param_glue(TAB_SKIP_CODE)];
                unset(t, 6.94444, 13.55559, &l)
            }
            "unset_span" => {
                let (a, b) = (ch(t, XX, b'a'), ch(t, XX, b'b'));
                let c = unset(t, 6.94444, 10.55559, &[a, b]);
                t.set_span_count(c, 1);
                let l = [t.new_param_glue(TAB_SKIP_CODE), c, t.new_param_glue(TAB_SKIP_CODE)];
                unset(t, 6.94444, 10.55559, &l)
            }
            "unset_stretch" => {
                let g = glue(t, 0.0, 1.0, N, 2.0, FIL);
                let c = unset(t, 0.0, 0.0, &[g]);
                t.set_glue_stretch(c, pt(1.0));
                t.set_glue_order(c, N);
                t.set_glue_shrink(c, pt(2.0));
                t.set_glue_sign(c, FIL);
                let a = t.new_param_glue(TAB_SKIP_CODE);
                let b = gsub(t, TAB_SKIP_CODE + 1, 0.0, 2.0, FIL, 3.0, FILL);
                chain(t, &[a, c, b])
            }
            "noads_kinds" => {
                let l = [
                    nc(t, ORD_NOAD, b'a'),
                    nc(t, OP_NOAD, b'b'),
                    nc(t, BIN_NOAD, b'c'),
                    nc(t, REL_NOAD, b'd'),
                    nc(t, OPEN_NOAD, b'e'),
                    nc(t, CLOSE_NOAD, b'f'),
                    nc(t, PUNCT_NOAD, b'g'),
                    nc(t, INNER_NOAD, b'h'),
                    nc(t, ORD_NOAD, b'i'),
                ];
                chain(t, &l)
            }
            "noads_limits" => {
                let (a, b, c) = (nc(t, OP_NOAD, b'a'), nc(t, OP_NOAD, b'b'), nc(t, OP_NOAD, b'c'));
                t.set_subtype(a, LIMITS);
                t.set_subtype(b, NO_LIMITS);
                chain(t, &[a, b, c])
            }
            "noad_scripts" => {
                let a = ord(t, b'a');
                set_char(t, t.supscr(a), 1, b'b');
                set_char(t, t.subscr(a), 1, b'c');
                let b = noad(t, ORD_NOAD, NOAD_SIZE);
                sub_mlist(t, t.nucleus(b), NULL);
                set_char(t, t.supscr(b), 1, b'd');
                chain(t, &[a, b])
            }
            "noad_box" => {
                let a = noad(t, ORD_NOAD, NOAD_SIZE);
                let x = h(t, &[]);
                sub_box(t, t.nucleus(a), x);
                let b = noad(t, ORD_NOAD, NOAD_SIZE);
                let x = h(t, &[]);
                t.set_width(x, pt(1.0));
                sub_box(t, t.nucleus(b), x);
                chain(t, &[a, b])
            }
            "noad_mlist" => {
                let a = ord(t, b'a');
                let i = ord(t, b'a');
                set_char(t, t.supscr(i), 1, b'b');
                let b = noad(t, ORD_NOAD, NOAD_SIZE);
                sub_mlist(t, t.nucleus(b), i);
                let c = noad(t, ORD_NOAD, NOAD_SIZE);
                sub_mlist(t, t.nucleus(c), NULL);
                let d = noad(t, ORD_NOAD, NOAD_SIZE);
                sub_mlist(t, t.nucleus(d), NULL);
                chain(t, &[a, b, c, d])
            }
            "noad_over" => {
                let (a, b) = (nc(t, OVER_NOAD, b'a'), nc(t, UNDER_NOAD, b'b'));
                let c = noad(t, VCENTER_NOAD, NOAD_SIZE);
                let x = v(t, &[]);
                sub_box(t, t.nucleus(c), x);
                chain(t, &[a, b, c])
            }
            "noad_radical" => {
                let p = noad(t, RADICAL_NOAD, RADICAL_NOAD_SIZE);
                set_char(t, t.nucleus(p), 1, b'a');
                delim(t, t.left_delimiter(p), 1, 0x61, 2, 0x70);
                p
            }
            "noad_accent" => {
                let p = noad(t, ACCENT_NOAD, ACCENT_NOAD_SIZE);
                set_char(t, t.nucleus(p), 1, b'a');
                t.set_fam(t.accent_chr(p), 0);
                t.set_character(t.accent_chr(p), 0x16);
                p
            }
            "noad_left_right" => {
                let l = noad(t, LEFT_NOAD, NOAD_SIZE);
                lparen(t, t.delimiter(l));
                let a = ord(t, b'a');
                let r = noad(t, RIGHT_NOAD, NOAD_SIZE);
                rparen(t, t.delimiter(r));
                let c = chain(t, &[l, a, r]);
                let p = noad(t, INNER_NOAD, NOAD_SIZE);
                sub_mlist(t, t.nucleus(p), c);
                p
            }
            "noad_middle" => {
                let l = noad(t, LEFT_NOAD, NOAD_SIZE);
                lparen(t, t.delimiter(l));
                let a = ord(t, b'a');
                let m = noad(t, RIGHT_NOAD, NOAD_SIZE);
                t.set_subtype(m, MIDDLE_NOAD);
                rparen(t, t.delimiter(m));
                let b = ord(t, b'b');
                let r = noad(t, RIGHT_NOAD, NOAD_SIZE);
                let c = chain(t, &[l, a, m, b, r]);
                let p = noad(t, INNER_NOAD, NOAD_SIZE);
                sub_mlist(t, t.nucleus(p), c);
                p
            }
            "noad_left_small_fam" => {
                // The delimiter is in the nucleus's word, and math_type would read it as MATH_CHAR: §872 must not look.
                let l = noad(t, LEFT_NOAD, NOAD_SIZE);
                delim(t, t.delimiter(l), 1, 0, 2, 0);
                let a = ord(t, b'a');
                let r = noad(t, RIGHT_NOAD, NOAD_SIZE);
                let c = chain(t, &[l, a, r]);
                let p = noad(t, INNER_NOAD, NOAD_SIZE);
                sub_mlist(t, t.nucleus(p), c);
                p
            }
            "fraction_default" => fraction(t, DEFAULT_CODE, false, false),
            "fraction_thickness" => fraction(t, pt(1.0), false, false),
            "fraction_atop" => fraction(t, 0, false, false),
            "fraction_delims" => fraction(t, DEFAULT_CODE, true, true),
            "fraction_right_only" => fraction(t, pt(2.0), false, true),
            "styles" => {
                let l = [style(t, DISPLAY_STYLE), style(t, TEXT_STYLE), style(t, SCRIPT_STYLE), style(t, SCRIPT_SCRIPT_STYLE)];
                chain(t, &l)
            }
            "choice" => {
                let p = t.get_node(STYLE_NODE_SIZE);
                t.set_ty(p, CHOICE_NODE);
                t.set_subtype(p, 0);
                let a = ord(t, b'a');
                t.set_display_mlist(p, a);
                t.set_text_mlist(p, NULL);
                let b = ord(t, b'b');
                t.set_script_mlist(p, b);
                let c = ord(t, b'c');
                set_char(t, t.supscr(c), 1, b'd');
                t.set_script_script_mlist(p, c);
                p
            }
            "math_glue" => {
                let n = t.new_glue(ZERO_GLUE);
                t.set_subtype(n, COND_MATH_GLUE);
                let l = [
                    n,
                    mu_glue(t, 3.0, 0.0, 0.0, N),
                    mu_glue(t, 1.0, 2.0, 1.0, FIL),
                    glue(t, 1.0, 0.0, N, 0.0, N),
                    kern(t, 3.0, MU_GLUE),
                    ke(t, 1.0),
                ];
                chain(t, &l)
            }
            "math_nonscript_alone" => {
                let n = t.new_glue(ZERO_GLUE);
                t.set_subtype(n, COND_MATH_GLUE);
                n
            }
            "math_disc_penalty" => {
                let a = ch(t, XX, b'a');
                let d = disc(t, 0, &[a], &[]);
                let p = t.new_penalty(3);
                chain(t, &[d, p])
            }
            "math_vcenter_body" => {
                let k = ke(t, 1.0);
                let x = v(t, &[k]);
                t.set_height(x, pt(1.0));
                let p = noad(t, VCENTER_NOAD, NOAD_SIZE);
                sub_box(t, t.nucleus(p), x);
                p
            }
            "limit_depth0" | "limit_depth1" | "limit_depth_max" => {
                let k = ke(t, 1.0);
                let a = adjust(t, 0, &[k]);
                let k = ke(t, 1.0);
                h(t, &[a, k])
            }
            "limit_depth_negative" => {
                let k = ke(t, 1.0);
                h(t, &[k])
            }
            "limit_breadth1" | "limit_breadth2" | "limit_breadth3" => {
                let l = [t.new_penalty(1), t.new_penalty(2), t.new_penalty(3)];
                h(t, &l)
            }
            "limit_breadth0" | "limit_breadth_negative" => {
                let l = [t.new_penalty(1), t.new_penalty(2), t.new_penalty(3), t.new_penalty(4), t.new_penalty(5), t.new_penalty(6)];
                h(t, &l)
            }
            "limit_breadth_max" => {
                let l = [t.new_penalty(1), t.new_penalty(2)];
                h(t, &l)
            }
            "limit_depth_noad" => {
                let a = ord(t, b'a');
                set_char(t, t.supscr(a), 1, b'b');
                a
            }
            "limit_depth_noad1" => {
                let k = ke(t, 1.0);
                let x = h(t, &[k]);
                t.set_width(x, pt(1.0));
                let a = noad(t, ORD_NOAD, NOAD_SIZE);
                sub_box(t, t.nucleus(a), x);
                set_char(t, t.supscr(a), 1, b'b');
                a
            }
            _ => panic!("no builder for the case {}", id),
        }
    }

    /// The rows of a table made by tests/oracle/gen_show_tables.py.
    pub fn table(data: &'static [u8]) -> Vec<Vec<String>> {
        data.split(|&c| c == b'\n')
            .filter(|l| !l.is_empty())
            .map(|l| {
                l.split(|&c| c == b'\t')
                    .map(|f| f.iter().map(|&c| if c == 0x0C { '\n' } else { c as char }).collect())
                    .collect()
            })
            .collect()
    }

    pub fn show_rows() -> Vec<Vec<String>> {
        table(include_bytes!("../testdata/show_oracle.tsv"))
    }
}

#[cfg(test)]
mod tests {
    use super::showkit::*;
    use super::*;
    use crate::mem::memkit::*;
    use crate::toks::testkit::*;

    #[test]
    #[ignore = "needs print.rs, arith.rs, strings.rs, hash.rs, eqtb.rs, mathdata.rs, ext.rs, files.rs"]
    fn show_box_matches_the_oracle_byte_for_byte() {
        let mut bad = Vec::new();
        let rows = show_rows();
        assert!(rows.len() >= 130);
        for row in &rows {
            let t = case_machine(&row[0], true);
            let p = build(t, &row[0]);
            t.show_box(p);
            let got = printed(t);
            let want = format!("{}\n", row[1]);
            if got != want {
                bad.push(format!("--- {}\nwant {:?}\ngot  {:?}", row[0], want, got));
            }
            Tex::dealloc(t);
        }
        assert!(bad.is_empty(), "{} of {} differ:\n{}", bad.len(), rows.len(), bad.join("\n"));
    }

    /// A copy is displayed as the original is, so every field that shows was copied, with the exceptions the
    /// oracle has: copy_show_oracle.tsv.  §1604 leaves `words` at 1 for \\pdfsnapycomp, so its ratio is whatever the
    /// allocator left there, 0 in memory not yet used.  Mlists are not copied.
    #[test]
    #[ignore = "needs print.rs, arith.rs, strings.rs, hash.rs, eqtb.rs, ext.rs, files.rs"]
    fn a_copy_shows_as_the_original_does() {
        let mut bad = Vec::new();
        let rows = show_rows();
        let copied = table(include_bytes!("../testdata/copy_oracle.tsv"));
        let differ = table(include_bytes!("../testdata/copy_show_oracle.tsv"));
        assert!(copied.len() >= 100);
        assert_eq!(differ.iter().map(|r| &r[0][..]).collect::<Vec<_>>(), ["pdfsnapycomp"], "see the table's generator");
        for row in &rows {
            let id = &row[0][..];
            if !copied.iter().any(|c| c[0] == id) && !id.starts_with("unset_") {
                continue;
            }
            let t = case_machine(id, true);
            let p = build(t, id);
            let c = t.copy_node_list(p);
            t.show_box(c);
            let got = printed(t);
            let shown = differ.iter().find(|r| r[0] == id).unwrap_or(row);
            let want = format!("{}\n", shown[1]);
            if got != want {
                bad.push(format!("--- {}\nwant {:?}\ngot  {:?}", id, want, got));
            }
            // And once the original has gone: nothing of it is shared but what holds a reference.
            t.flush_node_list(p);
            t.show_box(c);
            let got = printed(t);
            if got != want {
                bad.push(format!("--- {} without its original\nwant {:?}\ngot  {:?}", id, want, got));
            }
            Tex::dealloc(t);
        }
        assert!(bad.is_empty(), "{} differ:\n{}", bad.len(), bad.join("\n"));
    }

    /// §216's test.  The oracle's integer is 64 bits, so it does not wrap at \showboxdepth=2147483647.
    #[test]
    #[ignore = "needs print.rs"]
    fn a_huge_depth_is_limited_by_the_room_in_the_pool() {
        let t = case_machine("limit_depth_max", true);
        let p = build(t, "limit_depth_max");
        t.show_box(p);
        assert_eq!(t.depth_threshold, t.pool_size - t.pool_ptr - 1);
        Tex::dealloc(t);
    }

    #[test]
    #[ignore = "needs print.rs, arith.rs, strings.rs, hash.rs"]
    fn short_display_matches_the_oracle() {
        let mut bad = Vec::new();
        let rows = table(include_bytes!("../testdata/short_oracle.tsv"));
        assert!(rows.len() >= 18);
        for row in &rows {
            let t = case_machine(&row[0], true);
            let p = build(t, &row[0]);
            // §839
            t.font_in_short_display = NULL_FONT;
            t.short_display(t.list_ptr(p));
            let got = printed(t);
            if got != row[1] {
                bad.push(format!("--- {}\nwant {:?}\ngot  {:?}", row[0], row[1], got));
            }
            Tex::dealloc(t);
        }
        assert!(bad.is_empty(), "{} of {} differ:\n{}", bad.len(), rows.len(), bad.join("\n"));
    }

    fn shown(t: &mut Tex, p: i32) -> String {
        t.show_box(p);
        printed(t)
    }

    /// The oracle reaches these only with a corrupted memory, or a font it does not have, so what is
    /// expected here comes from reading the WEB, and not from a table.
    #[test]
    #[ignore = "needs print.rs, arith.rs, strings.rs"]
    fn what_no_oracle_input_can_reach_prints_as_web_says() {
        let t = case_machine("", true);
        // §200 Bad link
        assert_eq!(shown(t, MEM_TOP + 1), "\nBad link, display aborted.\n");
        // §201 othercases
        let p = t.get_node(SMALL_NODE_SIZE);
        t.set_ty(p, 99);
        assert_eq!(shown(t, p), "\nUnknown node type!\n");
        // §1603 othercases
        t.set_ty(p, WHATSIT_NODE);
        t.set_subtype(p, 99);
        assert_eq!(shown(t, p), "\nwhatsit?\n");
        // §194 print_mark
        t.print_mark(-1);
        assert_eq!(printed(t), "{\\CLOBBERED.}");
        t.print_mark(NULL);
        assert_eq!(printed(t), "{\\CLOBBERED.}", "the oracle's null is below hi_mem_min");
        let e = toks(t, b"");
        t.print_mark(e);
        assert_eq!(printed(t), "{}", "an empty list is not null");
        t.print_mark(t.toks_top);
        assert_eq!(printed(t), "{\\CLOBBERED.}");
        // §194 print_font_and_char
        t.print_font_and_char(MEM_TOP + 1);
        assert_eq!(printed(t), "\\CLOBBERED.");
        let c = t.get_avail();
        t.set_font(c, t.font_max + 1);
        t.set_character(c, b'a' as i32);
        t.print_font_and_char(c);
        assert_eq!(printed(t), "* a");
        // §192 short_display, the same font test, and only once
        t.font_in_short_display = NULL_FONT;
        t.short_display(c);
        assert_eq!(printed(t), "* a");
        t.short_display(c);
        assert_eq!(printed(t), "a");
        // §195 print_glue
        t.print_glue(UNITY, 4, b"pt");
        assert_eq!(printed(t), "1.0foul");
        t.print_glue(UNITY, -1, b"pt");
        assert_eq!(printed(t), "1.0foul");
        t.print_glue(UNITY, NORMAL, b"pt");
        assert_eq!(printed(t), "1.0pt");
        t.print_glue(UNITY, NORMAL, b"");
        assert_eq!(printed(t), "1.0");
        t.print_glue(UNITY, FILLL, b"pt");
        assert_eq!(printed(t), "1.0filll");
        // §196 print_spec: with no pointer, and with one that is not in the variable-size memory
        t.print_spec(NULL, b"");
        assert_eq!(printed(t), "*");
        t.print_spec(t.lo_mem_max, b"");
        assert_eq!(printed(t), "*");
        t.print_spec(FIL_GLUE, b"pt");
        assert_eq!(printed(t), "0.0pt plus 1.0fil");
        t.print_spec(SS_GLUE, b"mu");
        assert_eq!(printed(t), "0.0mu plus 1.0fil minus 1.0fil");
        // §194 print_rule_dimen
        t.print_rule_dimen(NULL_FLAG);
        t.print_rule_dimen(UNITY);
        assert_eq!(printed(t), "*1.0");
        Tex::dealloc(t);
    }

    /// §211.  cmr10 has no boundary character, so this case is not in the oracle's table.
    #[test]
    #[ignore = "needs print.rs, arith.rs, strings.rs"]
    fn ligatures_show_their_boundaries() {
        let t = case_machine("", true);
        let (f, i) = (t.get_avail(), t.get_avail());
        t.set_font(f, XX);
        t.set_character(f, b'f' as i32);
        t.set_font(i, XX);
        t.set_character(i, b'i' as i32);
        t.set_link(f, i);
        let p = t.new_ligature(XX, 12, f);
        for (sub, want) in [(0, "fi"), (1, "fi|"), (2, "|fi"), (3, "|fi|")] {
            t.set_subtype(p, sub);
            assert_eq!(shown(t, p), format!("\n\\xx ^^L (ligature {})\n", want));
        }
        Tex::dealloc(t);
    }

    /// §1603's last arm of the destination types.
    #[test]
    #[ignore = "needs print.rs, arith.rs, strings.rs"]
    fn an_unknown_destination_type_prints_so() {
        let t = case_machine("", true);
        let p = t.get_node(PDF_DEST_NODE_SIZE);
        t.set_ty(p, WHATSIT_NODE);
        t.set_subtype(p, PDF_DEST_NODE);
        t.set_pdf_dest_objnum(p, NULL);
        t.set_pdf_dest_id(p, 5);
        t.set_pdf_dest_named_id(p, 0);
        t.set_pdf_dest_type(p, 99);
        assert_eq!(shown(t, p), "\n\\pdfdest num5 unknown!\n");
        Tex::dealloc(t);
    }

    /// `print_font_identifier` without `\pdftracingfonts`, for a font at another size: nothing more.
    #[test]
    #[ignore = "needs print.rs, arith.rs, strings.rs"]
    fn print_font_identifier_says_more_only_when_asked() {
        let t = case_machine("", true);
        t.print_font_identifier(XY);
        assert_eq!(printed(t), "\\xy");
        t.print_font_identifier(PLUS);
        assert_eq!(printed(t), "\\xx (+20)");
        t.print_font_identifier(MINUS);
        assert_eq!(printed(t), "\\xx (-15)");
        t.set_pdf_tracing_fonts(1);
        t.print_font_identifier(PLUS);
        assert_eq!(printed(t), "\\xx (cmr10)", "a tracing font shows its name, not the ratio");
        t.print_font_identifier(XY);
        assert_eq!(printed(t), "\\xy (cmr10@12.0pt)");
        Tex::dealloc(t);
    }

    /// The display uses the pool as its prefix stack and leaves it as it was.
    #[test]
    #[ignore = "needs print.rs, arith.rs, strings.rs, mathdata.rs"]
    fn the_prefix_is_given_back() {
        for row in show_rows() {
            let t = case_machine(&row[0], true);
            let p = build(t, &row[0]);
            let pool = t.pool_ptr;
            let _ = shown(t, p);
            assert_eq!(t.pool_ptr, pool, "{}", row[0]);
            Tex::dealloc(t);
        }
    }

    #[test]
    fn every_case_of_the_oracle_can_be_built_and_flushed() {
        for row in show_rows() {
            let t = case_machine(&row[0], false);
            let (v, d) = (t.var_used, t.dyn_used);
            let p = build(t, &row[0]);
            t.flush_node_list(p);
            assert_eq!((t.var_used, t.dyn_used), (v, d), "{}", row[0]);
            let _ = ring(t);
            Tex::dealloc(t);
        }
    }
}
