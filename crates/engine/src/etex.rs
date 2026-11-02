//! §1648-1866 Part 53a: the extended features of e-TeX.
//!
//! Sparse arrays hold token lists as headers in `toks`: §1823 shows `p + 1`, and §1828 asks
//! `tok_is_empty`, since `link` of such a field would read `mem`.  The other one-word lists here, the
//! LR stack and the pseudo files, stay in `mem` (PORTING section 16).
//!
//! §1683 appends to the caller's token builder and leaves no list in `cur_val`, so §1753 commits its own.
//! §1811 chooses the help line from `max_reg_help_line`, 0 for 255 and 1 for 32767 (PORTING section 7).
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
    /// §1656
    pub fn eTeX_enabled(&mut self, b: bool, j: i32, k: i32) -> bool {
        if !b {
            self.print_err_s(b"Improper ");
            self.print_cmd_chr(j, k);
            self.help(&[b"Sorry, this optional e-TeX feature has been disabled."]);
            self.error();
        }
        b
    }

    /// §1661
    pub fn print_group(&mut self, e: bool) {
        match self.cur_group {
            BOTTOM_LEVEL => {
                self.print_s(b"bottom level");
                return;
            }
            SIMPLE_GROUP | SEMI_SIMPLE_GROUP => {
                if self.cur_group == SEMI_SIMPLE_GROUP {
                    self.print_s(b"semi ");
                }
                self.print_s(b"simple");
            }
            HBOX_GROUP | ADJUSTED_HBOX_GROUP => {
                if self.cur_group == ADJUSTED_HBOX_GROUP {
                    self.print_s(b"adjusted ");
                }
                self.print_s(b"hbox");
            }
            VBOX_GROUP => self.print_s(b"vbox"),
            VTOP_GROUP => self.print_s(b"vtop"),
            ALIGN_GROUP | NO_ALIGN_GROUP => {
                if self.cur_group == NO_ALIGN_GROUP {
                    self.print_s(b"no ");
                }
                self.print_s(b"align");
            }
            OUTPUT_GROUP => self.print_s(b"output"),
            DISC_GROUP => self.print_s(b"disc"),
            INSERT_GROUP => self.print_s(b"insert"),
            VCENTER_GROUP => self.print_s(b"vcenter"),
            MATH_GROUP | MATH_CHOICE_GROUP | MATH_SHIFT_GROUP | MATH_LEFT_GROUP => {
                self.print_s(b"math");
                if self.cur_group == MATH_CHOICE_GROUP {
                    self.print_s(b" choice");
                } else if self.cur_group == MATH_SHIFT_GROUP {
                    self.print_s(b" shift");
                } else if self.cur_group == MATH_LEFT_GROUP {
                    self.print_s(b" left");
                }
            }
            _ => {} // there are no other cases
        }
        self.print_s(b" group (level ");
        self.print_int(self.cur_level);
        self.print_char(b')' as i32);
        if self.saved(-1) != 0 {
            if e {
                self.print_s(b" entered at line ");
            } else {
                self.print_s(b" at line ");
            }
            self.print_int(self.saved(-1));
        }
    }

    /// §1662
    pub fn group_trace(&mut self, e: bool) {
        self.begin_diagnostic();
        self.print_char(b'{' as i32);
        if e {
            self.print_s(b"leaving ");
        } else {
            self.print_s(b"entering ");
        }
        self.print_group(e);
        self.print_char(b'}' as i32);
        self.end_diagnostic(false);
    }

    /// §1679
    pub fn show_save_groups(&mut self) {
        let mut p = self.nest_ptr;
        self.nest[p] = self.cur_list; // put the top level into the array
        let v = self.save_ptr;
        let l = self.cur_level;
        let c = self.cur_group;
        self.save_ptr = self.cur_boundary;
        self.cur_level -= 1;
        let mut a = 1; // to keep track of alignments
        self.print_nl_s(b"");
        self.print_ln();
        loop {
            self.print_nl_s(b"### ");
            self.print_group(true);
            if self.cur_group == BOTTOM_LEVEL {
                break; // done:
            }
            let mut m: i32;
            loop {
                m = self.nest[p].mode_field;
                if p > 0 {
                    p -= 1;
                } else {
                    m = VMODE;
                }
                if m != HMODE {
                    break;
                }
            }
            self.print_s(b" (");
            let mut s: &[u8] = b"";
            'found: {
                'found2: {
                    'found1: {
                        match self.cur_group {
                            SIMPLE_GROUP => {
                                p += 1;
                                break 'found2;
                            }
                            HBOX_GROUP | ADJUSTED_HBOX_GROUP => s = b"hbox",
                            VBOX_GROUP => s = b"vbox",
                            VTOP_GROUP => s = b"vtop",
                            ALIGN_GROUP => {
                                if a == 0 {
                                    s = if m == -VMODE { b"halign" } else { b"valign" };
                                    a = 1;
                                    break 'found1;
                                } else {
                                    if a == 1 {
                                        self.print_s(b"align entry");
                                    } else {
                                        self.print_esc_s(b"cr");
                                    }
                                    if p >= a {
                                        p -= a;
                                    }
                                    a = 0;
                                    break 'found;
                                }
                            }
                            NO_ALIGN_GROUP => {
                                p += 1;
                                a = -1;
                                self.print_esc_s(b"noalign");
                                break 'found2;
                            }
                            OUTPUT_GROUP => {
                                self.print_esc_s(b"output");
                                break 'found;
                            }
                            MATH_GROUP => break 'found2,
                            DISC_GROUP | MATH_CHOICE_GROUP => {
                                if self.cur_group == DISC_GROUP {
                                    self.print_esc_s(b"discretionary");
                                } else {
                                    self.print_esc_s(b"mathchoice");
                                }
                                for i in 1..=3 {
                                    if i <= self.saved(-2) {
                                        self.print_s(b"{}");
                                    }
                                }
                                break 'found2;
                            }
                            INSERT_GROUP => {
                                if self.saved(-2) == 255 {
                                    self.print_esc_s(b"vadjust");
                                } else {
                                    self.print_esc_s(b"insert");
                                    self.print_int(self.saved(-2));
                                }
                                break 'found2;
                            }
                            VCENTER_GROUP => {
                                s = b"vcenter";
                                break 'found1;
                            }
                            SEMI_SIMPLE_GROUP => {
                                p += 1;
                                self.print_esc_s(b"begingroup");
                                break 'found;
                            }
                            MATH_SHIFT_GROUP => {
                                if m == MMODE {
                                    self.print_char(b'$' as i32);
                                } else if self.nest[p].mode_field == MMODE {
                                    self.print_cmd_chr(EQ_NO, self.saved(-2));
                                    break 'found;
                                }
                                self.print_char(b'$' as i32);
                                break 'found;
                            }
                            MATH_LEFT_GROUP => {
                                if self.ty(self.nest[p + 1].eTeX_aux_field) == LEFT_NOAD {
                                    self.print_esc_s(b"left");
                                } else {
                                    self.print_esc_s(b"middle");
                                }
                                break 'found;
                            }
                            _ => {} // there are no other cases
                        }
                        // §1681 Show the box context
                        let mut i = self.saved(-4);
                        if i != 0 {
                            if i < BOX_FLAG {
                                let j = if abs(self.nest[p].mode_field) == VMODE { HMOVE } else { VMOVE };
                                if i > 0 {
                                    self.print_cmd_chr(j, 0);
                                } else {
                                    self.print_cmd_chr(j, 1);
                                }
                                self.print_scaled(abs(i));
                                self.print_s(b"pt");
                            } else if i < SHIP_OUT_FLAG {
                                if i >= GLOBAL_BOX_FLAG {
                                    self.print_esc_s(b"global");
                                    i -= GLOBAL_BOX_FLAG - BOX_FLAG;
                                }
                                self.print_esc_s(b"setbox");
                                self.print_int(i - BOX_FLAG);
                                self.print_char(b'=' as i32);
                            } else {
                                self.print_cmd_chr(LEADER_SHIP, i - (LEADER_FLAG - A_LEADERS));
                            }
                        }
                    }
                    // found1:
                    self.print_esc_s(s);
                    // §1680 Show the box packaging info
                    if self.saved(-2) != 0 {
                        self.print_char(b' ' as i32);
                        if self.saved(-3) == EXACTLY {
                            self.print_s(b"to");
                        } else {
                            self.print_s(b"spread");
                        }
                        self.print_scaled(self.saved(-2));
                        self.print_s(b"pt");
                    }
                }
                // found2:
                self.print_char(b'{' as i32);
            }
            // found:
            self.print_char(b')' as i32);
            self.cur_level -= 1;
            self.cur_group = self.save_level(self.save_ptr);
            self.save_ptr = self.save_index(self.save_ptr);
        }
        // done:
        self.save_ptr = v;
        self.cur_level = l;
        self.cur_group = c;
    }

    /// §1683
    /// Appends to the current token builder; cur_val is not set; see PORTING.md.
    pub fn scan_general_text(&mut self) {
        let (s, w, d) = (self.scanner_status, self.warning_index, self.def_ref);
        self.scanner_status = ABSORBING;
        self.warning_index = self.cur_cs;
        let saved_mark = self.tb_def_mark;
        self.tb_def_mark = self.tb_mark();
        self.scan_left_brace(); // remove the compulsory left brace
        let mut unbalance = 1;
        loop {
            self.get_token();
            if self.cur_tok < RIGHT_BRACE_LIMIT {
                if self.cur_cmd < RIGHT_BRACE {
                    unbalance += 1;
                } else {
                    unbalance -= 1;
                    if unbalance == 0 {
                        break; // found:
                    }
                }
            }
            self.tb_push(self.cur_tok);
        }
        self.tb_def_mark = saved_mark;
        self.cur_val = TEMP_HEAD; // WEB leaves a pointer here, and §497 can read it: PORTING section 15
        self.scanner_status = s;
        self.warning_index = w;
        self.def_ref = d;
    }

    /// §1719
    pub fn new_edge(&mut self, s: i32, w: i32) -> i32 {
        let p = self.get_node(EDGE_NODE_SIZE);
        self.set_ty(p, EDGE_NODE);
        self.set_subtype(p, s);
        self.set_width(p, w);
        self.set_edge_dist(p, 0); // the edge_dist field will be set later
        p
    }

    /// §1723
    pub fn reverse(&mut self, this_box: i32, t: i32, cur_g: &mut i32, cur_glue: &mut f64) -> i32 {
        let g_order = self.glue_order(this_box);
        let g_sign = self.glue_sign(this_box);
        let mut l = t;
        let mut p = self.temp_ptr;
        let mut q: i32;
        let mut m = MIN_HALFWORD;
        let mut n = MIN_HALFWORD;
        'done: loop {
            'reswitch: while p != NULL {
                // §1724 Move node p to the new list and go to the next node; or goto done if the end of
                // the reflected segment has been reached.  `reswitch:` is the top of this loop.
                if self.is_char_node(p) {
                    loop {
                        self.f = self.font(p);
                        self.c = self.character(p);
                        let i = self.char_info(self.f, self.c);
                        self.cur_h += self.char_width(self.f, i);
                        q = self.link(p);
                        self.set_link(p, l);
                        l = p;
                        p = q;
                        if !self.is_char_node(p) {
                            break;
                        }
                    }
                } else {
                    // §1725 Move the non-char_node p to the new list
                    q = self.link(p);
                    'next_p: {
                        match self.ty(p) {
                            HLIST_NODE | VLIST_NODE | RULE_NODE | KERN_NODE => self.rule_wd = self.width(p),
                            // §1726
                            GLUE_NODE => {
                                self.round_glue(p, this_box, g_sign, g_order, cur_g, cur_glue);
                                // §1699 Handle a glue node for mixed direction typesetting
                                if (g_sign == STRETCHING && self.stretch_order(self.g) == g_order)
                                    || (g_sign == SHRINKING && self.shrink_order(self.g) == g_order)
                                {
                                    self.fast_delete_glue_ref(self.g);
                                    if self.subtype(p) < A_LEADERS {
                                        self.set_ty(p, KERN_NODE);
                                        self.set_width(p, self.rule_wd);
                                    } else {
                                        self.g = self.get_node(GLUE_SPEC_SIZE);
                                        self.set_stretch_order(self.g, FILLL + 1); // will never match
                                        self.set_shrink_order(self.g, FILLL + 1);
                                        self.set_width(self.g, self.rule_wd);
                                        self.set_stretch(self.g, 0);
                                        self.set_shrink(self.g, 0);
                                        self.set_glue_ptr(p, self.g);
                                    }
                                }
                            }
                            // §1727
                            LIGATURE_NODE => {
                                self.flush_node_list(self.lig_ptr(p));
                                self.temp_ptr = p;
                                p = self.get_avail();
                                self.mem[p] = self.mem[self.lig_char(self.temp_ptr)];
                                self.set_link(p, q);
                                self.free_node(self.temp_ptr, SMALL_NODE_SIZE);
                                continue 'reswitch; // p is now a char node
                            }
                            // §1728
                            MATH_NODE => {
                                self.rule_wd = self.width(p);
                                if self.end_LR(p) {
                                    if self.info(self.LR_ptr) != self.end_LR_type(p) {
                                        self.set_ty(p, KERN_NODE);
                                        self.LR_problems += 1;
                                    } else {
                                        self.pop_LR();
                                        if n > MIN_HALFWORD {
                                            n -= 1;
                                            self.set_subtype(p, self.subtype(p) - 1); // change after into before
                                        } else {
                                            self.set_ty(p, KERN_NODE);
                                            if m > MIN_HALFWORD {
                                                m -= 1;
                                            } else {
                                                // §1729 Finish the reversed hlist segment and goto done
                                                self.free_node(p, MEDIUM_NODE_SIZE); // SyncTeX: p is a kern_node
                                                self.set_link(t, q);
                                                self.set_width(t, self.rule_wd);
                                                self.set_edge_dist(t, sat32(-(self.cur_h as i64) - self.rule_wd as i64));
                                                break 'done;
                                            }
                                        }
                                    }
                                } else {
                                    self.push_LR(p);
                                    if n > MIN_HALFWORD || self.LR_dir(p) != self.cur_dir {
                                        n += 1;
                                        self.set_subtype(p, self.subtype(p) + 1); // change before into after
                                    } else {
                                        self.set_ty(p, KERN_NODE);
                                        m += 1;
                                    }
                                }
                            }
                            EDGE_NODE => self.confusion(b"LR2"),
                            _ => break 'next_p,
                        }
                        self.cur_h += self.rule_wd;
                    }
                    // next_p:
                    self.set_link(p, l);
                    if self.ty(p) == KERN_NODE && (self.rule_wd == 0 || l == NULL) {
                        self.free_node(p, MEDIUM_NODE_SIZE);
                        p = l;
                    }
                    l = p;
                    p = q;
                }
            }
            if t == NULL && m == MIN_HALFWORD && n == MIN_HALFWORD {
                break 'done;
            }
            p = self.new_math(0, self.info(self.LR_ptr));
            self.LR_problems += 10000; // manufacture one missing math node
        }
        l
    }

    /// §1733
    pub fn just_copy(&mut self, p: i32, h: i32, t: i32) {
        let (mut p, mut h) = (p, h);
        let mut r: i32;
        let mut words: i32;
        while p != NULL {
            'not_found: {
                'found: {
                    words = 1; // this setting occurs in more branches than any other
                    if self.is_char_node(p) {
                        r = self.get_avail();
                    } else {
                        match self.ty(p) {
                            HLIST_NODE | VLIST_NODE => {
                                r = self.get_node(BOX_NODE_SIZE);
                                // §1926 Copy the box SyncTeX information
                                self.set_sync_tag(r + BOX_NODE_SIZE, self.sync_tag(p + BOX_NODE_SIZE));
                                self.set_sync_line(r + BOX_NODE_SIZE, self.sync_line(p + BOX_NODE_SIZE));
                                self.mem[r + 6] = self.mem[p + 6];
                                self.mem[r + 5] = self.mem[p + 5]; // copy the last two words
                                words = 5;
                                self.set_list_ptr(r, NULL); // this affects mem[r+5]
                            }
                            RULE_NODE => {
                                r = self.get_node(RULE_NODE_SIZE);
                                words = RULE_NODE_SIZE;
                            }
                            LIGATURE_NODE => {
                                r = self.get_avail(); // only font and character are needed
                                self.mem[r] = self.mem[self.lig_char(p)];
                                break 'found;
                            }
                            KERN_NODE | MATH_NODE => {
                                words = MEDIUM_NODE_SIZE; // SyncTeX: proper size for math and kern
                                r = self.get_node(words);
                            }
                            GLUE_NODE => {
                                r = self.get_node(MEDIUM_NODE_SIZE); // SyncTeX: proper size for glue
                                self.add_glue_ref(self.glue_ptr(p));
                                // §1928 Copy the medium sized node SyncTeX information
                                self.set_sync_tag(r + MEDIUM_NODE_SIZE, self.sync_tag(p + MEDIUM_NODE_SIZE));
                                self.set_sync_line(r + MEDIUM_NODE_SIZE, self.sync_line(p + MEDIUM_NODE_SIZE));
                                self.set_glue_ptr(r, self.glue_ptr(p));
                                self.set_leader_ptr(r, NULL);
                            }
                            WHATSIT_NODE => {
                                // §1604 Make a partial copy of the whatsit node p and make r point to it; set
                                // words to the number of initial words not yet copied
                                match self.subtype(p) {
                                    OPEN_NODE => {
                                        r = self.get_node(OPEN_NODE_SIZE);
                                        words = OPEN_NODE_SIZE;
                                    }
                                    WRITE_NODE | SPECIAL_NODE | LATESPECIAL_NODE => {
                                        r = self.get_node(WRITE_NODE_SIZE);
                                        self.add_token_ref(self.write_tokens(p));
                                        words = WRITE_NODE_SIZE;
                                    }
                                    CLOSE_NODE | LANGUAGE_NODE => {
                                        r = self.get_node(SMALL_NODE_SIZE);
                                        words = SMALL_NODE_SIZE;
                                    }
                                    PDF_LITERAL_NODE | PDF_LATELITERAL_NODE => {
                                        r = self.get_node(WRITE_NODE_SIZE);
                                        self.add_token_ref(self.pdf_literal_data(p));
                                        words = WRITE_NODE_SIZE;
                                    }
                                    PDF_COLORSTACK_NODE => {
                                        if self.pdf_colorstack_cmd(p) <= COLORSTACK_DATA {
                                            r = self.get_node(PDF_COLORSTACK_SETTER_NODE_SIZE);
                                            self.add_token_ref(self.pdf_colorstack_data(p));
                                            words = PDF_COLORSTACK_SETTER_NODE_SIZE;
                                        } else {
                                            r = self.get_node(PDF_COLORSTACK_GETTER_NODE_SIZE);
                                            words = PDF_COLORSTACK_GETTER_NODE_SIZE;
                                        }
                                    }
                                    PDF_SETMATRIX_NODE => {
                                        r = self.get_node(PDF_SETMATRIX_NODE_SIZE);
                                        self.add_token_ref(self.pdf_setmatrix_data(p));
                                        words = PDF_SETMATRIX_NODE_SIZE;
                                    }
                                    PDF_SAVE_NODE => {
                                        r = self.get_node(PDF_SAVE_NODE_SIZE);
                                        words = PDF_SAVE_NODE_SIZE;
                                    }
                                    PDF_RESTORE_NODE => {
                                        r = self.get_node(PDF_RESTORE_NODE_SIZE);
                                        words = PDF_RESTORE_NODE_SIZE;
                                    }
                                    PDF_REFOBJ_NODE => {
                                        r = self.get_node(PDF_REFOBJ_NODE_SIZE);
                                        words = PDF_REFOBJ_NODE_SIZE;
                                    }
                                    PDF_REFXFORM_NODE => {
                                        r = self.get_node(PDF_REFXFORM_NODE_SIZE);
                                        words = PDF_REFXFORM_NODE_SIZE;
                                    }
                                    PDF_REFXIMAGE_NODE => {
                                        r = self.get_node(PDF_REFXIMAGE_NODE_SIZE);
                                        words = PDF_REFXIMAGE_NODE_SIZE;
                                    }
                                    PDF_ANNOT_NODE => {
                                        r = self.get_node(PDF_ANNOT_NODE_SIZE);
                                        self.add_token_ref(self.pdf_annot_data(p));
                                        words = PDF_ANNOT_NODE_SIZE;
                                    }
                                    PDF_START_LINK_NODE => {
                                        r = self.get_node(PDF_ANNOT_NODE_SIZE);
                                        self.set_pdf_height(r, self.pdf_height(p));
                                        self.set_pdf_depth(r, self.pdf_depth(p));
                                        self.set_pdf_width(r, self.pdf_width(p));
                                        self.set_pdf_link_attr(r, self.pdf_link_attr(p));
                                        if self.pdf_link_attr(r) != NULL {
                                            self.add_token_ref(self.pdf_link_attr(r));
                                        }
                                        self.set_pdf_link_action(r, self.pdf_link_action(p));
                                        self.add_action_ref(self.pdf_link_action(r));
                                        self.set_pdf_link_objnum(r, self.pdf_link_objnum(p));
                                    }
                                    PDF_END_LINK_NODE => r = self.get_node(SMALL_NODE_SIZE),
                                    PDF_DEST_NODE => {
                                        r = self.get_node(PDF_DEST_NODE_SIZE);
                                        if self.pdf_dest_named_id(p) > 0 {
                                            self.add_token_ref(self.pdf_dest_id(p));
                                        }
                                        words = PDF_DEST_NODE_SIZE;
                                    }
                                    PDF_THREAD_NODE | PDF_START_THREAD_NODE => {
                                        r = self.get_node(PDF_THREAD_NODE_SIZE);
                                        if self.pdf_thread_named_id(p) > 0 {
                                            self.add_token_ref(self.pdf_thread_id(p));
                                        }
                                        if self.pdf_thread_attr(p) != NULL {
                                            self.add_token_ref(self.pdf_thread_attr(p));
                                        }
                                        words = PDF_THREAD_NODE_SIZE;
                                    }
                                    PDF_END_THREAD_NODE => r = self.get_node(SMALL_NODE_SIZE),
                                    PDF_SAVE_POS_NODE => r = self.get_node(SMALL_NODE_SIZE),
                                    PDF_SNAP_REF_POINT_NODE => r = self.get_node(SMALL_NODE_SIZE),
                                    PDF_SNAPY_NODE => {
                                        self.add_glue_ref(self.snap_glue_ptr(p));
                                        r = self.get_node(SNAP_NODE_SIZE);
                                        words = SNAP_NODE_SIZE;
                                    }
                                    PDF_SNAPY_COMP_NODE => r = self.get_node(SMALL_NODE_SIZE),
                                    PDF_INTERWORD_SPACE_ON_NODE => r = self.get_node(SMALL_NODE_SIZE),
                                    PDF_INTERWORD_SPACE_OFF_NODE => r = self.get_node(SMALL_NODE_SIZE),
                                    PDF_FAKE_SPACE_NODE => r = self.get_node(SMALL_NODE_SIZE),
                                    PDF_RUNNING_LINK_OFF_NODE => r = self.get_node(SMALL_NODE_SIZE),
                                    PDF_RUNNING_LINK_ON_NODE => r = self.get_node(SMALL_NODE_SIZE),
                                    _ => self.confusion(b"ext2"),
                                }
                            }
                            _ => break 'not_found,
                        }
                    }
                    while words > 0 {
                        words -= 1;
                        self.mem[r + words] = self.mem[p + words];
                    }
                }
                // found:
                self.set_link(h, r);
                h = r;
            }
            // not_found:
            p = self.link(p);
        }
        self.set_link(h, t);
    }

    /// §1738
    pub fn just_reverse(&mut self, p: i32) {
        let mut p = p;
        let mut q: i32;
        let mut m = MIN_HALFWORD;
        let mut n = MIN_HALFWORD;
        if self.link(TEMP_HEAD) == NULL {
            self.just_copy(self.link(p), TEMP_HEAD, NULL);
            q = self.link(TEMP_HEAD);
        } else {
            q = self.link(p);
            self.set_link(p, NULL);
            self.flush_node_list(self.link(TEMP_HEAD));
        }
        let t = self.new_edge(self.cur_dir, 0);
        let mut l = t;
        self.cur_dir = self.reflected();
        'done: {
            while q != NULL {
                if self.is_char_node(q) {
                    loop {
                        p = q;
                        q = self.link(p);
                        self.set_link(p, l);
                        l = p;
                        if !self.is_char_node(q) {
                            break;
                        }
                    }
                } else {
                    p = q;
                    q = self.link(p);
                    if self.ty(p) == MATH_NODE {
                        // §1739 Adjust the LR stack for the just_reverse routine
                        if self.end_LR(p) {
                            if self.info(self.LR_ptr) != self.end_LR_type(p) {
                                self.set_ty(p, KERN_NODE); // SyncTeX: a math_node has a kern_node's size
                                self.LR_problems += 1;
                            } else {
                                self.pop_LR();
                                if n > MIN_HALFWORD {
                                    n -= 1;
                                    self.set_subtype(p, self.subtype(p) - 1); // change after into before
                                } else {
                                    if m > MIN_HALFWORD {
                                        m -= 1;
                                    } else {
                                        self.set_width(t, self.width(p));
                                        self.set_link(t, q);
                                        self.free_node(p, MEDIUM_NODE_SIZE);
                                        break 'done; // SyncTeX: no more "goto found", and proper node size
                                    }
                                    self.set_ty(p, KERN_NODE);
                                }
                            }
                        } else {
                            self.push_LR(p);
                            if n > MIN_HALFWORD || self.LR_dir(p) != self.cur_dir {
                                n += 1;
                                self.set_subtype(p, self.subtype(p) + 1); // change before into after
                            } else {
                                self.set_ty(p, KERN_NODE);
                                m += 1;
                            }
                        }
                    }
                    self.set_link(p, l);
                    l = p;
                }
            }
            // WEB has `goto done` here, then the old found: code, which nothing reaches any more.
        }
        self.set_link(TEMP_HEAD, l);
    }

    /// §1744
    ///
    /// `s`, `d` and `e` are `i64`: three dimensions are added here, which Knuth's bound of 2^30 does not cover
    /// and the oracle's 64-bit `scaled` holds (PORTING section 11).  Stores clamp.
    pub fn app_display(&mut self, j: i32, b: i32, d: i32) {
        let (mut j, mut b) = (j, b);
        let mut d = d as i64;
        let mut s = self.display_indent() as i64;
        let x = self.pre_display_direction();
        if x == 0 {
            self.set_shift_amount(b, sat32(s + d));
        } else {
            let z = self.display_width() as i64;
            let mut p = b;
            let mut e: i64;
            let mut q: i32;
            let mut r: i32;
            let mut t: i32;
            // §1745 Set up the hlist for the display line
            if x > 0 {
                e = z - d - self.width(p) as i64;
            } else {
                e = d;
                d = z - e - self.width(p) as i64;
            }
            if j != NULL {
                b = self.copy_node_list(j);
                self.set_height(b, self.height(p));
                self.set_depth(b, self.depth(p));
                s -= self.shift_amount(b) as i64;
                d += s;
                e = e + self.width(b) as i64 - z - s;
            }
            if self.box_lr(p) == DLIST {
                q = p; // display or equation number
            } else {
                // display and equation number
                r = self.list_ptr(p);
                self.free_node(p, BOX_NODE_SIZE);
                if r == NULL {
                    self.confusion(b"LR4");
                }
                if x > 0 {
                    p = r;
                    loop {
                        q = r;
                        r = self.link(r); // find tail of list
                        if r == NULL {
                            break;
                        }
                    }
                } else {
                    p = NULL;
                    q = r;
                    loop {
                        t = self.link(r);
                        self.set_link(r, p);
                        p = r;
                        r = t; // reverse list
                        if r == NULL {
                            break;
                        }
                    }
                }
            }
            // §1746 Package the display line
            if j == NULL {
                r = self.new_kern(0);
                t = self.new_kern(0); // the widths will be set later
            } else {
                r = self.list_ptr(b);
                t = self.link(r);
            }
            let mut u = self.new_math(0, END_M_CODE);
            if self.ty(t) == GLUE_NODE {
                // t is \rightskip glue
                j = self.etex_cancel_glue(RIGHT_SKIP_CODE, q, u, t, sat32(e));
                self.set_link(u, t);
            } else {
                self.set_width(t, sat32(e));
                self.set_link(t, u);
                self.set_link(q, t);
            }
            u = self.new_math(0, BEGIN_M_CODE);
            if self.ty(r) == GLUE_NODE {
                // r is \leftskip glue
                self.etex_cancel_glue(LEFT_SKIP_CODE, u, p, r, sat32(d));
                self.set_link(r, u);
            } else {
                self.set_width(r, sat32(d));
                self.set_link(r, p);
                self.set_link(u, r);
                if j == NULL {
                    b = self.hpack_natural(u);
                    self.set_shift_amount(b, sat32(s));
                } else {
                    self.set_list_ptr(b, u);
                }
            }
        }
        self.append_to_vlist(b);
    }

    /// §1746 `cancel_glue(code)(a)(b)(c)(w)`: returns the new `j`, since WEB uses `j` as its temporary and
    /// all it asks of it afterwards is `j<>null`.
    fn etex_cancel_glue(&mut self, code: i32, a: i32, b: i32, c: i32, w: i32) -> i32 {
        let mut j = self.new_skip_param(code);
        self.set_link(a, j);
        self.set_link(j, b);
        j = self.glue_ptr(c);
        let tp = self.temp_ptr;
        self.set_stretch_order(tp, self.stretch_order(j));
        self.set_shrink_order(tp, self.shrink_order(j));
        self.set_width(tp, w - self.width(j));
        self.set_stretch(tp, -self.stretch(j));
        self.set_shrink(tp, -self.shrink(j));
        j
    }

    /// §1753
    pub fn pseudo_start(&mut self) {
        let m = self.tb_mark();
        self.scan_general_text();
        let p = self.tb_commit(m);
        let old_setting = self.selector;
        self.selector = NEW_STRING;
        self.token_show(p);
        self.selector = old_setting;
        self.flush_list(p);
        self.str_room(1);
        let s = self.make_string();
        self.etex_string_to_pseudo_file(s);
        self.flush_string();
        // §1755 Initiate input from new pseudo file
        self.begin_file_reading(); // set up cur_file and new level of input
        self.line = 0;
        self.set_limit(self.start());
        self.set_loc(self.limit() + 1); // force line read
        if self.tracing_scan_tokens() > 0 {
            if self.term_offset > self.max_print_line - 3 {
                self.print_ln();
            } else if self.term_offset > 0 || self.file_offset > 0 {
                self.print_char(b' ' as i32);
            }
            self.set_name(19);
            self.print_s(b"( ");
            self.open_parens += 1;
            self.update_terminal();
        } else {
            self.set_name(18);
            // §1911 Prepare pseudo file SyncTeX information
            self.set_synctex_tag(0);
        }
    }

    /// §1754 Convert string s into a new pseudo file.  Apart from pseudo_start, so a test can reach it.
    fn etex_string_to_pseudo_file(&mut self, s: i32) {
        self.str_pool.put(self.pool_ptr, b' ' as i32);
        let mut l = self.str_start[s];
        let nl = self.new_line_char();
        let p = self.get_avail();
        let mut q = p;
        while l < self.pool_ptr {
            let mut m = l;
            while l < self.pool_ptr && self.str_pool.at(l) != nl {
                l += 1;
            }
            let mut sz = (l - m + 7) / 4;
            if sz == 1 {
                sz = 2;
            }
            let mut r = self.get_node(sz);
            self.set_link(q, r);
            q = r;
            self.set_info(q, sz);
            let mut w: FourQuarters;
            while sz > 2 {
                sz -= 1;
                r += 1;
                w = FourQuarters {
                    b0: self.str_pool.at(m),
                    b1: self.str_pool.at(m + 1),
                    b2: self.str_pool.at(m + 2),
                    b3: self.str_pool.at(m + 3),
                };
                self.mem[r].set_qqqq(w);
                m += 4;
            }
            let sp = b' ' as i32;
            w = FourQuarters { b0: sp, b1: sp, b2: sp, b3: sp };
            if l > m {
                w.b0 = self.str_pool.at(m);
                if l > m + 1 {
                    w.b1 = self.str_pool.at(m + 1);
                    if l > m + 2 {
                        w.b2 = self.str_pool.at(m + 2);
                        if l > m + 3 {
                            w.b3 = self.str_pool.at(m + 3);
                        }
                    }
                }
            }
            self.mem[r + 1].set_qqqq(w);
            if self.str_pool.at(l) == nl {
                l += 1;
            }
        }
        self.set_info(p, self.link(p));
        self.set_link(p, self.pseudo_files);
        self.pseudo_files = p;
    }

    /// §1756
    pub fn pseudo_input(&mut self) -> bool {
        self.last = self.first; // cf. Matthew 19:30
        let p = self.info(self.pseudo_files);
        if p == NULL {
            false
        } else {
            self.set_info(self.pseudo_files, self.link(p));
            let sz = self.info(p);
            if 4 * sz - 3 >= self.buf_size - self.last {
                // §35 Report overflow of the input buffer, and abort
                self.cur_input.loc_field = self.first;
                self.cur_input.limit_field = self.last - 1;
                self.overflow(b"buffer size", self.buf_size);
            }
            self.last = self.first;
            for r in p + 1..=p + sz - 1 {
                let w = self.mem[r].qqqq();
                self.buffer.put(self.last, w.b0);
                self.buffer.put(self.last + 1, w.b1);
                self.buffer.put(self.last + 2, w.b2);
                self.buffer.put(self.last + 3, w.b3);
                self.last += 4;
            }
            if self.last >= self.max_buf_stack {
                self.max_buf_stack = self.last + 1;
            }
            while self.last > self.first && self.buffer.at(self.last - 1) == b' ' as i32 {
                self.last -= 1;
            }
            self.free_node(p, sz);
            true
        }
    }

    /// §1757
    pub fn pseudo_close(&mut self) {
        let mut p = self.link(self.pseudo_files);
        let mut q = self.info(self.pseudo_files);
        self.free_avail(self.pseudo_files);
        self.pseudo_files = p;
        while q != NULL {
            p = q;
            q = self.link(p);
            self.free_node(p, self.info(p));
        }
    }

    /// §1772
    pub fn get_x_or_protected(&mut self) {
        loop {
            self.get_token();
            if self.cur_cmd <= MAX_COMMAND {
                return;
            }
            if self.cur_cmd >= CALL && self.cur_cmd < END_TEMPLATE {
                if self.toks[self.cur_chr + 1] == PROTECTED_TOKEN {
                    return;
                }
            }
            self.expand();
        }
    }

    /// §1775 Set variable w to indicate if this case should be reported
    fn etex_nesting_report(&mut self, i: i32, w: &mut bool) {
        if self.tracing_nesting() > 0 {
            while self.input_stack[self.base_ptr].state_field == TOKEN_LIST
                || self.input_stack[self.base_ptr].index_field > i
            {
                self.base_ptr -= 1;
            }
            if self.input_stack[self.base_ptr].name_field > 17 {
                *w = true;
            }
        }
    }

    /// §1774
    pub fn group_warning(&mut self) {
        self.base_ptr = self.input_ptr;
        self.input_stack[self.base_ptr] = self.cur_input; // store current state
        let mut i = self.in_open;
        let mut w = false;
        while self.grp_stack[i] == self.cur_boundary && i > 0 {
            self.etex_nesting_report(i, &mut w);
            self.grp_stack[i] = self.save_index(self.save_ptr);
            i -= 1;
        }
        if w {
            self.print_nl_s(b"Warning: end of ");
            self.print_group(true);
            self.print_s(b" of a different file");
            self.print_ln();
            if self.tracing_nesting() > 1 {
                self.show_context();
            }
            if self.history == SPOTLESS {
                self.history = WARNING_ISSUED;
            }
        }
    }

    /// §1776
    pub fn if_warning(&mut self) {
        self.base_ptr = self.input_ptr;
        self.input_stack[self.base_ptr] = self.cur_input; // store current state
        let mut i = self.in_open;
        let mut w = false;
        while self.if_stack[i] == self.cond_ptr {
            self.etex_nesting_report(i, &mut w);
            self.if_stack[i] = self.link(self.cond_ptr);
            i -= 1;
        }
        if w {
            self.print_nl_s(b"Warning: end of ");
            self.print_cmd_chr(IF_TEST, self.cur_if);
            self.print_if_line(self.if_line);
            self.print_s(b" of a different file");
            self.print_ln();
            if self.tracing_nesting() > 1 {
                self.show_context();
            }
            if self.history == SPOTLESS {
                self.history = WARNING_ISSUED;
            }
        }
    }

    /// §1777
    pub fn file_warning(&mut self) {
        let mut p = self.save_ptr;
        let mut l = self.cur_level;
        let mut c = self.cur_group;
        self.save_ptr = self.cur_boundary;
        while self.grp_stack[self.in_open] != self.save_ptr {
            self.cur_level -= 1;
            self.print_nl_s(b"Warning: end of file when ");
            self.print_group(true);
            self.print_s(b" is incomplete");
            self.cur_group = self.save_level(self.save_ptr);
            self.save_ptr = self.save_index(self.save_ptr);
        }
        self.save_ptr = p;
        self.cur_level = l;
        self.cur_group = c; // restore old values
        p = self.cond_ptr;
        l = self.if_limit;
        c = self.cur_if;
        let i = self.if_line;
        while self.if_stack[self.in_open] != self.cond_ptr {
            self.print_nl_s(b"Warning: end of file when ");
            self.print_cmd_chr(IF_TEST, self.cur_if);
            if self.if_limit == FI_CODE {
                self.print_esc_s(b"else");
            }
            self.print_if_line(self.if_line);
            self.print_s(b" is incomplete");
            self.if_line = self.if_line_field(self.cond_ptr);
            self.cur_if = self.subtype(self.cond_ptr);
            self.if_limit = self.ty(self.cond_ptr);
            self.cond_ptr = self.link(self.cond_ptr);
        }
        self.cond_ptr = p;
        self.if_limit = l;
        self.cur_if = c;
        self.if_line = i; // restore old values
        self.print_ln();
        if self.tracing_nesting() > 1 {
            self.show_context();
        }
        if self.history == SPOTLESS {
            self.history = WARNING_ISSUED;
        }
    }

    /// §1782
    ///
    /// §1790's `f>infinity` cannot be true of an `i32`.  It can on the oracle, for a register that an `\advance` took
    /// past 2^31: the "Values at or above 2^31" divergence of PORTING section 15, which clamps that register to
    /// 2^31-1.  So `\numexpr` of it is an "Arithmetic overflow" there and 2147483647 here.
    pub fn scan_expr(&mut self) {
        let mut l = self.cur_val_level; // type of expression
        let a = self.arith_error; // saved value of arith_error
        let mut b = false;
        let mut p = NULL; // top of expression stack
        let mut q: i32;
        let mut r: i32; // state of expression so far
        let mut s: i32; // state of term so far
        let mut o: i32; // next operation or type of next factor
        let mut e: i32; // expression so far
        let mut t: i32; // term so far
        let mut f: i32; // current factor
        let mut n: i32; // numerator of combined multiplication and division
        self.expand_depth_count += 1;
        if self.expand_depth_count >= self.expand_depth {
            self.overflow(b"expansion depth", self.expand_depth);
        }
        // §1783 Scan and evaluate an expression e of type l
        'restart: loop {
            r = EXPR_NONE;
            e = 0;
            s = EXPR_NONE;
            t = 0;
            n = 0;
            'continue_: loop {
                o = if s == EXPR_NONE { l } else { INT_VAL };
                // §1785 Scan a factor f of type o or start a subexpression
                // §432 Get the next non-blank non-call token
                loop {
                    self.get_x_token();
                    if self.cur_cmd != SPACER {
                        break;
                    }
                }
                if self.cur_tok == OTHER_TOKEN + b'(' as i32 {
                    // §1788 Push the expression stack and goto restart
                    q = self.get_node(EXPR_NODE_SIZE);
                    self.set_link(q, p);
                    self.set_ty(q, l);
                    self.set_subtype(q, 4 * s + r);
                    self.set_expr_e_field(q, e);
                    self.set_expr_t_field(q, t);
                    self.set_expr_n_field(q, n);
                    p = q;
                    l = o;
                    continue 'restart;
                }
                self.back_input();
                if o == INT_VAL {
                    self.scan_int();
                } else if o == DIMEN_VAL {
                    self.scan_normal_dimen();
                } else if o == GLUE_VAL {
                    self.scan_normal_glue();
                } else {
                    self.scan_mu_glue();
                }
                f = self.cur_val;
                'found: loop {
                    // §1784 Scan the next operator and set o
                    // §432 Get the next non-blank non-call token
                    loop {
                        self.get_x_token();
                        if self.cur_cmd != SPACER {
                            break;
                        }
                    }
                    if self.cur_tok == OTHER_TOKEN + b'+' as i32 {
                        o = EXPR_ADD;
                    } else if self.cur_tok == OTHER_TOKEN + b'-' as i32 {
                        o = EXPR_SUB;
                    } else if self.cur_tok == OTHER_TOKEN + b'*' as i32 {
                        o = EXPR_MULT;
                    } else if self.cur_tok == OTHER_TOKEN + b'/' as i32 {
                        o = EXPR_DIV;
                    } else {
                        o = EXPR_NONE;
                        if p == NULL {
                            if self.cur_cmd != RELAX {
                                self.back_input();
                            }
                        } else if self.cur_tok != OTHER_TOKEN + b')' as i32 {
                            self.print_err_s(b"Missing ) inserted for expression");
                            self.help(&[b"I was expecting to see `+', `-', `*', `/', or `)'. Didn't."]);
                            self.back_error();
                        }
                    }
                    self.arith_error = b;
                    // §1790 Make sure that f is in the proper range
                    if l == INT_VAL || s > EXPR_SUB {
                        if f > INFINITY || f < -INFINITY {
                            self.arith_error = true; // num_error(f)
                            f = 0;
                        }
                    } else if l == DIMEN_VAL {
                        if abs(f) > MAX_DIMEN {
                            self.arith_error = true; // num_error(f)
                            f = 0;
                        }
                    } else if abs(self.width(f)) > MAX_DIMEN
                        || abs(self.stretch(f)) > MAX_DIMEN
                        || abs(self.shrink(f)) > MAX_DIMEN
                    {
                        self.arith_error = true; // glue_error(f)
                        self.delete_glue_ref(f);
                        f = self.new_spec(ZERO_GLUE);
                    }
                    match s {
                        // §1791 Cases for evaluation of the current term
                        EXPR_NONE => {
                            if l >= GLUE_VAL && o != EXPR_NONE {
                                t = self.new_spec(f);
                                self.delete_glue_ref(f);
                                self.normalize_glue(t);
                            } else {
                                t = f;
                            }
                        }
                        // §1795
                        EXPR_MULT => {
                            if o == EXPR_DIV {
                                n = f;
                                o = EXPR_SCALE;
                            } else if l == INT_VAL {
                                t = self.mult_integers(t, f);
                            } else if l == DIMEN_VAL {
                                t = self.nx_plus_y(t, f, 0); // expr_m
                            } else {
                                let v = self.nx_plus_y(self.width(t), f, 0);
                                self.set_width(t, v);
                                let v = self.nx_plus_y(self.stretch(t), f, 0);
                                self.set_stretch(t, v);
                                let v = self.nx_plus_y(self.shrink(t), f, 0);
                                self.set_shrink(t, v);
                            }
                        }
                        // §1796
                        EXPR_DIV => {
                            if l < GLUE_VAL {
                                t = self.quotient(t, f); // expr_d
                            } else {
                                let v = self.quotient(self.width(t), f);
                                self.set_width(t, v);
                                let v = self.quotient(self.stretch(t), f);
                                self.set_stretch(t, v);
                                let v = self.quotient(self.shrink(t), f);
                                self.set_shrink(t, v);
                            }
                        }
                        // §1798
                        EXPR_SCALE => {
                            if l == INT_VAL {
                                t = self.fract(t, n, f, INFINITY);
                            } else if l == DIMEN_VAL {
                                t = self.fract(t, n, f, MAX_DIMEN); // expr_s
                            } else {
                                let v = self.fract(self.width(t), n, f, MAX_DIMEN);
                                self.set_width(t, v);
                                let v = self.fract(self.stretch(t), n, f, MAX_DIMEN);
                                self.set_stretch(t, v);
                                let v = self.fract(self.shrink(t), n, f, MAX_DIMEN);
                                self.set_shrink(t, v);
                            }
                        }
                        _ => {} // there are no other cases
                    }
                    if o > EXPR_SUB {
                        s = o;
                    } else {
                        // §1792 Evaluate the current expression
                        s = EXPR_NONE;
                        if r == EXPR_NONE {
                            e = t;
                        } else if l == INT_VAL {
                            e = self.add_or_sub(e, t, INFINITY, r == EXPR_SUB);
                        } else if l == DIMEN_VAL {
                            e = self.add_or_sub(e, t, MAX_DIMEN, r == EXPR_SUB);
                        } else {
                            // §1794 Compute the sum or difference of two glue specs
                            let v = self.add_or_sub(self.width(e), self.width(t), MAX_DIMEN, r == EXPR_SUB);
                            self.set_width(e, v);
                            if self.stretch_order(e) == self.stretch_order(t) {
                                let v = self.add_or_sub(self.stretch(e), self.stretch(t), MAX_DIMEN, r == EXPR_SUB);
                                self.set_stretch(e, v);
                            } else if self.stretch_order(e) < self.stretch_order(t) && self.stretch(t) != 0 {
                                self.set_stretch(e, self.stretch(t));
                                self.set_stretch_order(e, self.stretch_order(t));
                            }
                            if self.shrink_order(e) == self.shrink_order(t) {
                                let v = self.add_or_sub(self.shrink(e), self.shrink(t), MAX_DIMEN, r == EXPR_SUB);
                                self.set_shrink(e, v);
                            } else if self.shrink_order(e) < self.shrink_order(t) && self.shrink(t) != 0 {
                                self.set_shrink(e, self.shrink(t));
                                self.set_shrink_order(e, self.shrink_order(t));
                            }
                            self.delete_glue_ref(t);
                            self.normalize_glue(e);
                        }
                        r = o;
                    }
                    b = self.arith_error;
                    if o != EXPR_NONE {
                        continue 'continue_;
                    }
                    if p != NULL {
                        // §1789 Pop the expression stack and goto found
                        f = e;
                        q = p;
                        e = self.expr_e_field(q);
                        t = self.expr_t_field(q);
                        n = self.expr_n_field(q);
                        s = self.subtype(q) / 4;
                        r = self.subtype(q) % 4;
                        l = self.ty(q);
                        p = self.link(q);
                        self.free_node(q, EXPR_NODE_SIZE);
                        continue 'found;
                    }
                    break 'restart;
                }
            }
        }
        self.expand_depth_count -= 1;
        if b {
            self.print_err_s(b"Arithmetic overflow");
            self.help(&[b"I can't evaluate this expression,", b"since the result is out of range."]);
            self.error();
            if l >= GLUE_VAL {
                self.delete_glue_ref(e);
                e = ZERO_GLUE;
                self.add_glue_ref(e);
            } else {
                e = 0;
            }
        }
        self.arith_error = a;
        self.cur_val = e;
        self.cur_val_level = l;
    }

    /// §1787
    pub fn scan_normal_glue(&mut self) {
        self.scan_glue(GLUE_VAL);
    }

    /// §1787
    pub fn scan_mu_glue(&mut self) {
        self.scan_glue(MU_VAL);
    }

    /// §1793
    pub fn add_or_sub(&mut self, x: i32, y: i32, max_answer: i32, negative: bool) -> i32 {
        let mut y = y;
        let a: i32;
        if negative {
            y = -y;
        }
        if x >= 0 {
            if y <= max_answer - x {
                a = x + y;
            } else {
                self.arith_error = true; // num_error(a)
                a = 0;
            }
        } else if y >= -max_answer - x {
            a = x + y;
        } else {
            self.arith_error = true; // num_error(a)
            a = 0;
        }
        a
    }

    /// §1797
    pub fn quotient(&mut self, n: i32, d: i32) -> i32 {
        let (mut n, mut d) = (n, d);
        let mut negative: bool;
        let mut a: i32;
        if d == 0 {
            self.arith_error = true; // num_error(a)
            a = 0;
        } else {
            if d > 0 {
                negative = false;
            } else {
                d = -d;
                negative = true;
            }
            if n < 0 {
                n = -n;
                negative = !negative;
            }
            a = n / d;
            n -= a * d;
            d = n - d; // avoid certain compiler optimizations!
            if d + n >= 0 {
                a += 1;
            }
            if negative {
                a = -a;
            }
        }
        a
    }

    /// §1799
    pub fn fract(&mut self, x: i32, n: i32, d: i32, max_answer: i32) -> i32 {
        let (mut x, mut n, mut d) = (x, n, d);
        let mut negative: bool;
        let mut a: i32;
        let mut f: i32;
        let mut t: i32;
        'done: {
            'too_big: {
                if d == 0 {
                    break 'too_big;
                }
                a = 0;
                if d > 0 {
                    negative = false;
                } else {
                    d = -d;
                    negative = true;
                }
                if x < 0 {
                    x = -x;
                    negative = !negative;
                } else if x == 0 {
                    break 'done;
                }
                if n < 0 {
                    n = -n;
                    negative = !negative;
                }
                'found: {
                    t = n / d;
                    if t > max_answer / x {
                        break 'too_big;
                    }
                    a = t * x;
                    n -= t * d;
                    if n == 0 {
                        break 'found;
                    }
                    t = x / d;
                    if t > (max_answer - a) / n {
                        break 'too_big;
                    }
                    a += t * n;
                    x -= t * d;
                    if x == 0 {
                        break 'found;
                    }
                    if x < n {
                        t = x;
                        x = n;
                        n = t;
                    } // now 0<n<=x<d
                    // §1800 Compute f=floor(xn/d+1/2)
                    f = 0;
                    let mut r = (d / 2) - d;
                    let h = -r;
                    loop {
                        if odd(n) {
                            r += x;
                            if r >= 0 {
                                r -= d;
                                f += 1;
                            }
                        }
                        n /= 2;
                        if n == 0 {
                            break; // found1:
                        }
                        if x < h {
                            x += x;
                        } else {
                            t = x - d;
                            x += t;
                            f += n;
                            if x < n {
                                if x == 0 {
                                    break; // found1:
                                }
                                t = x;
                                x = n;
                                n = t;
                            }
                        }
                    }
                    // found1:
                    if f > max_answer - a {
                        break 'too_big;
                    }
                    a += f;
                }
                // found:
                if negative {
                    a = -a;
                }
                break 'done;
            }
            // too_big:
            self.arith_error = true; // num_error(a)
            a = 0;
        }
        // done:
        a
    }

    /// §1811
    pub fn scan_register_num(&mut self) {
        self.scan_int();
        if self.cur_val < 0 || self.cur_val > self.max_reg_num {
            self.print_err_s(b"Bad register code");
            let line: &'static [u8] = if self.max_reg_help_line == 0 {
                b"A register number must be between 0 and 255." // §1812
            } else {
                b"A register number must be between 0 and 32767." // §1813
            };
            self.help(&[line, b"I changed this one to zero."]);
            self.int_error(self.cur_val);
            self.cur_val = 0;
        }
    }

    /// §1815
    pub fn new_index(&mut self, i: i32, q: i32) {
        self.cur_ptr = self.get_node(INDEX_NODE_SIZE);
        self.set_sa_index(self.cur_ptr, i);
        self.set_sa_used(self.cur_ptr, 0);
        self.set_link(self.cur_ptr, q);
        for k in 1..=INDEX_NODE_SIZE - 1 {
            self.mem[self.cur_ptr + k] = self.sa_null; // clear all 16 pointers
        }
    }

    /// §1819 `get_sa_ptr`: set cur_ptr to the pointer indexed by i from index node q.
    #[inline]
    fn get_sa_ptr(&mut self, q: i32, i: i32) {
        self.cur_ptr = if odd(i) { self.link(q + (i / 2) + 1) } else { self.info(q + (i / 2) + 1) };
    }

    /// §1819 `put_sa_ptr(v)`: store the pointer indexed by i in index node q.
    #[inline]
    fn put_sa_ptr(&mut self, q: i32, i: i32, v: i32) {
        if odd(i) {
            self.set_link(q + (i / 2) + 1, v);
        } else {
            self.set_info(q + (i / 2) + 1, v);
        }
    }

    /// §1819 `add_sa_ptr`: add cur_ptr as the pointer indexed by i in index node q.
    #[inline]
    fn add_sa_ptr(&mut self, q: i32, i: i32) {
        self.put_sa_ptr(q, i, self.cur_ptr);
        self.set_sa_used(q, self.sa_used(q) + 1);
    }

    /// §1819 `delete_sa_ptr`: delete the pointer indexed by i in index node q.
    #[inline]
    fn delete_sa_ptr(&mut self, q: i32, i: i32) {
        self.put_sa_ptr(q, i, NULL);
        self.set_sa_used(q, self.sa_used(q) - 1);
    }

    /// §1819
    pub fn find_sa_element(&mut self, t: i32, n: i32, w: bool) {
        let mut q: i32;
        let mut i: i32;
        self.cur_ptr = self.sa_root[t];
        'not_found4: {
            'not_found3: {
                'not_found2: {
                    'not_found1: {
                        'not_found: {
                            if self.cur_ptr == NULL {
                                if w {
                                    break 'not_found;
                                } else {
                                    return;
                                }
                            }
                            q = self.cur_ptr;
                            i = self.hex_dig1(n);
                            self.get_sa_ptr(q, i);
                            if self.cur_ptr == NULL {
                                if w {
                                    break 'not_found1;
                                } else {
                                    return;
                                }
                            }
                            q = self.cur_ptr;
                            i = self.hex_dig2(n);
                            self.get_sa_ptr(q, i);
                            if self.cur_ptr == NULL {
                                if w {
                                    break 'not_found2;
                                } else {
                                    return;
                                }
                            }
                            q = self.cur_ptr;
                            i = self.hex_dig3(n);
                            self.get_sa_ptr(q, i);
                            if self.cur_ptr == NULL {
                                if w {
                                    break 'not_found3;
                                } else {
                                    return;
                                }
                            }
                            q = self.cur_ptr;
                            i = self.hex_dig4(n);
                            self.get_sa_ptr(q, i);
                            if self.cur_ptr == NULL && w {
                                break 'not_found4;
                            }
                            return;
                        }
                        // not_found:
                        self.new_index(t, NULL); // create first level index node
                        self.sa_root[t] = self.cur_ptr;
                        q = self.cur_ptr;
                        i = self.hex_dig1(n);
                    }
                    // not_found1:
                    self.new_index(i, q); // create second level index node
                    self.add_sa_ptr(q, i);
                    q = self.cur_ptr;
                    i = self.hex_dig2(n);
                }
                // not_found2:
                self.new_index(i, q); // create third level index node
                self.add_sa_ptr(q, i);
                q = self.cur_ptr;
                i = self.hex_dig3(n);
            }
            // not_found3:
            self.new_index(i, q); // create fourth level index node
            self.add_sa_ptr(q, i);
            q = self.cur_ptr;
            i = self.hex_dig4(n);
        }
        // not_found4:
        // §1820 Create a new array element of type t with index i
        if t == MARK_VAL {
            // a mark class
            self.cur_ptr = self.get_node(MARK_CLASS_NODE_SIZE);
            self.mem[self.cur_ptr + 1] = self.sa_null;
            self.mem[self.cur_ptr + 2] = self.sa_null;
            self.mem[self.cur_ptr + 3] = self.sa_null;
        } else {
            if t <= DIMEN_VAL {
                // a count or dimen register
                self.cur_ptr = self.get_node(WORD_NODE_SIZE);
                self.set_sa_int(self.cur_ptr, 0);
                self.set_sa_num(self.cur_ptr, n);
            } else {
                self.cur_ptr = self.get_node(POINTER_NODE_SIZE);
                if t <= MU_VAL {
                    // a skip or muskip register
                    self.set_sa_ptr(self.cur_ptr, ZERO_GLUE);
                    self.add_glue_ref(ZERO_GLUE);
                } else {
                    self.set_sa_ptr(self.cur_ptr, NULL); // a box or token list register
                }
            }
            self.set_sa_ref(self.cur_ptr, NULL); // all registers have a reference count
        }
        self.set_sa_index(self.cur_ptr, 16 * t + i);
        self.set_sa_lev(self.cur_ptr, LEVEL_ONE);
        self.set_link(self.cur_ptr, q);
        self.add_sa_ptr(q, i);
    }

    /// §1821
    pub fn delete_sa_ref(&mut self, q: i32) {
        let mut q = q;
        let mut s: i32;
        self.set_sa_ref(q, self.sa_ref(q) - 1);
        if self.sa_ref(q) != NULL {
            return;
        }
        if self.sa_index(q) < DIMEN_VAL_LIMIT {
            if self.sa_int(q) == 0 {
                s = WORD_NODE_SIZE;
            } else {
                return;
            }
        } else {
            if self.sa_index(q) < MU_VAL_LIMIT {
                if self.sa_ptr(q) == ZERO_GLUE {
                    self.delete_glue_ref(ZERO_GLUE);
                } else {
                    return;
                }
            } else if self.sa_ptr(q) != NULL {
                return;
            }
            s = POINTER_NODE_SIZE;
        }
        loop {
            let i = self.hex_dig4(self.sa_index(q));
            let p = q;
            q = self.link(p);
            self.free_node(p, s);
            if q == NULL {
                // the whole tree has been freed
                self.sa_root[i] = NULL;
                return;
            }
            self.delete_sa_ptr(q, i);
            s = INDEX_NODE_SIZE; // node q is an index node
            if self.sa_used(q) > 0 {
                break;
            }
        }
    }

    /// §1822
    pub fn print_sa_num(&mut self, q: i32) {
        let mut q = q;
        let mut n: i32;
        if self.sa_index(q) < DIMEN_VAL_LIMIT {
            n = self.sa_num(q); // the easy case
        } else {
            n = self.hex_dig4(self.sa_index(q));
            q = self.link(q);
            n += 16 * self.sa_index(q);
            q = self.link(q);
            n += 256 * (self.sa_index(q) + 16 * self.sa_index(self.link(q)));
        }
        self.print_int(n);
    }

    /// §1823
    pub fn show_sa(&mut self, p: i32, s: &[u8]) {
        let mut p = p;
        self.begin_diagnostic();
        self.print_char(b'{' as i32);
        self.print_s(s);
        self.print_char(b' ' as i32);
        if p == NULL {
            self.print_char(b'?' as i32); // this can't happen
        } else {
            let t = self.sa_type(p);
            if t < BOX_VAL {
                self.print_cmd_chr(REGISTER, p);
            } else if t == BOX_VAL {
                self.print_esc_s(b"box");
                self.print_sa_num(p);
            } else if t == TOK_VAL {
                self.print_cmd_chr(TOKS_REGISTER, p);
            } else {
                self.print_char(b'?' as i32); // this can't happen either
            }
            self.print_char(b'=' as i32);
            if t == INT_VAL {
                self.print_int(self.sa_int(p));
            } else if t == DIMEN_VAL {
                self.print_scaled(self.sa_dim(p));
                self.print_s(b"pt");
            } else {
                p = self.sa_ptr(p);
                if t == GLUE_VAL {
                    self.print_spec(p, b"pt");
                } else if t == MU_VAL {
                    self.print_spec(p, b"mu");
                } else if t == BOX_VAL {
                    if p == NULL {
                        self.print_s(b"void");
                    } else {
                        self.depth_threshold = 0;
                        self.breadth_max = 1;
                        self.show_node_list(p);
                    }
                } else if t == TOK_VAL {
                    if p != NULL {
                        self.show_token_list(p + 1, NULL, 32);
                    }
                } else {
                    self.print_char(b'?' as i32); // this can't happen either
                }
            }
        }
        self.print_char(b'}' as i32);
        self.end_diagnostic(false);
    }

    /// §1825
    pub fn do_marks(&mut self, a: i32, l: i32, q: i32) -> bool {
        let mut q = q;
        if l < 4 {
            // q is an index node
            for i in 0..=15 {
                self.get_sa_ptr(q, i);
                if self.cur_ptr != NULL {
                    if self.do_marks(a, l + 1, self.cur_ptr) {
                        self.delete_sa_ptr(q, i);
                    }
                }
            }
            if self.sa_used(q) == 0 {
                self.free_node(q, INDEX_NODE_SIZE);
                q = NULL;
            }
        } else {
            // q is the node for a mark class
            match a {
                // §1826
                VSPLIT_INIT => {
                    if self.sa_split_first_mark(q) != NULL {
                        self.delete_token_ref(self.sa_split_first_mark(q));
                        self.set_sa_split_first_mark(q, NULL);
                        self.delete_token_ref(self.sa_split_bot_mark(q));
                        self.set_sa_split_bot_mark(q, NULL);
                    }
                }
                // §1828
                FIRE_UP_INIT => {
                    if self.sa_bot_mark(q) != NULL {
                        if self.sa_top_mark(q) != NULL {
                            self.delete_token_ref(self.sa_top_mark(q));
                        }
                        self.delete_token_ref(self.sa_first_mark(q));
                        self.set_sa_first_mark(q, NULL);
                        if self.tok_is_empty(self.sa_bot_mark(q)) {
                            // an empty token list
                            self.delete_token_ref(self.sa_bot_mark(q));
                            self.set_sa_bot_mark(q, NULL);
                        } else {
                            self.add_token_ref(self.sa_bot_mark(q));
                        }
                        self.set_sa_top_mark(q, self.sa_bot_mark(q));
                    }
                }
                // §1829
                FIRE_UP_DONE => {
                    if self.sa_top_mark(q) != NULL && self.sa_first_mark(q) == NULL {
                        self.set_sa_first_mark(q, self.sa_top_mark(q));
                        self.add_token_ref(self.sa_top_mark(q));
                    }
                }
                // §1831
                DESTROY_MARKS => {
                    for i in TOP_MARK_CODE..=SPLIT_BOT_MARK_CODE {
                        self.get_sa_ptr(q, i);
                        if self.cur_ptr != NULL {
                            self.delete_token_ref(self.cur_ptr);
                            self.put_sa_ptr(q, i, NULL);
                        }
                    }
                }
                _ => {} // there are no other cases
            }
            if self.sa_bot_mark(q) == NULL && self.sa_split_bot_mark(q) == NULL {
                self.free_node(q, MARK_CLASS_NODE_SIZE);
                q = NULL;
            }
        }
        q == NULL
    }

    /// §1837
    pub fn sa_save(&mut self, p: i32) {
        let q: i32;
        if self.cur_level != self.sa_level {
            self.check_full_save_stack();
            self.set_save_type(self.save_ptr, RESTORE_SA);
            self.set_save_level(self.save_ptr, self.sa_level);
            self.set_save_index(self.save_ptr, self.sa_chain);
            self.save_ptr += 1;
            self.sa_chain = NULL;
            self.sa_level = self.cur_level;
        }
        let mut i = self.sa_index(p);
        if i < DIMEN_VAL_LIMIT {
            if self.sa_int(p) == 0 {
                q = self.get_node(POINTER_NODE_SIZE);
                i = TOK_VAL_LIMIT;
            } else {
                q = self.get_node(WORD_NODE_SIZE);
                self.set_sa_int(q, self.sa_int(p));
            }
            self.set_sa_ptr(q, NULL);
        } else {
            q = self.get_node(POINTER_NODE_SIZE);
            self.set_sa_ptr(q, self.sa_ptr(p));
        }
        self.set_sa_loc(q, p);
        self.set_sa_index(q, i);
        self.set_sa_lev(q, self.sa_lev(p));
        self.set_link(q, self.sa_chain);
        self.sa_chain = q;
        self.add_sa_ref(p);
    }

    /// §1838
    pub fn sa_destroy(&mut self, p: i32) {
        if self.sa_index(p) < MU_VAL_LIMIT {
            self.delete_glue_ref(self.sa_ptr(p));
        } else if self.sa_ptr(p) != NULL {
            if self.sa_index(p) < BOX_VAL_LIMIT {
                self.flush_node_list(self.sa_ptr(p));
            } else {
                self.delete_token_ref(self.sa_ptr(p));
            }
        }
    }

    /// §1839
    pub fn sa_def(&mut self, p: i32, e: i32) {
        self.add_sa_ref(p);
        if self.sa_ptr(p) == e {
            if self.tracing_assigns() > 0 {
                self.show_sa(p, b"reassigning");
            }
            self.sa_destroy(p);
        } else {
            if self.tracing_assigns() > 0 {
                self.show_sa(p, b"changing");
            }
            if self.sa_lev(p) == self.cur_level {
                self.sa_destroy(p);
            } else {
                self.sa_save(p);
            }
            self.set_sa_lev(p, self.cur_level);
            self.set_sa_ptr(p, e);
            if self.tracing_assigns() > 0 {
                self.show_sa(p, b"into");
            }
        }
        self.delete_sa_ref(p);
    }

    /// §1839
    pub fn sa_w_def(&mut self, p: i32, w: i32) {
        self.add_sa_ref(p);
        if self.sa_int(p) == w {
            if self.tracing_assigns() > 0 {
                self.show_sa(p, b"reassigning");
            }
        } else {
            if self.tracing_assigns() > 0 {
                self.show_sa(p, b"changing");
            }
            if self.sa_lev(p) != self.cur_level {
                self.sa_save(p);
            }
            self.set_sa_lev(p, self.cur_level);
            self.set_sa_int(p, w);
            if self.tracing_assigns() > 0 {
                self.show_sa(p, b"into");
            }
        }
        self.delete_sa_ref(p);
    }

    /// §1840
    pub fn gsa_def(&mut self, p: i32, e: i32) {
        self.add_sa_ref(p);
        if self.tracing_assigns() > 0 {
            self.show_sa(p, b"globally changing");
        }
        self.sa_destroy(p);
        self.set_sa_lev(p, LEVEL_ONE);
        self.set_sa_ptr(p, e);
        if self.tracing_assigns() > 0 {
            self.show_sa(p, b"into");
        }
        self.delete_sa_ref(p);
    }

    /// §1840
    pub fn gsa_w_def(&mut self, p: i32, w: i32) {
        self.add_sa_ref(p);
        if self.tracing_assigns() > 0 {
            self.show_sa(p, b"globally changing");
        }
        self.set_sa_lev(p, LEVEL_ONE);
        self.set_sa_int(p, w);
        if self.tracing_assigns() > 0 {
            self.show_sa(p, b"into");
        }
        self.delete_sa_ref(p);
    }

    /// §1841
    pub fn sa_restore(&mut self) {
        loop {
            let mut p = self.sa_loc(self.sa_chain);
            if self.sa_lev(p) == LEVEL_ONE {
                if self.sa_index(p) >= DIMEN_VAL_LIMIT {
                    self.sa_destroy(self.sa_chain);
                }
                if self.tracing_restores() > 0 {
                    self.show_sa(p, b"retaining");
                }
            } else {
                if self.sa_index(p) < DIMEN_VAL_LIMIT {
                    if self.sa_index(self.sa_chain) < DIMEN_VAL_LIMIT {
                        self.set_sa_int(p, self.sa_int(self.sa_chain));
                    } else {
                        self.set_sa_int(p, 0);
                    }
                } else {
                    self.sa_destroy(p);
                    self.set_sa_ptr(p, self.sa_ptr(self.sa_chain));
                }
                self.set_sa_lev(p, self.sa_lev(self.sa_chain));
                if self.tracing_restores() > 0 {
                    self.show_sa(p, b"restoring");
                }
            }
            self.delete_sa_ref(p);
            p = self.sa_chain;
            self.sa_chain = self.link(p);
            if self.sa_index(p) < DIMEN_VAL_LIMIT {
                self.free_node(p, WORD_NODE_SIZE);
            } else {
                self.free_node(p, POINTER_NODE_SIZE);
            }
            if self.sa_chain == NULL {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::testkit::*;
    use crate::toks::testkit::*;

    fn rows(data: &'static str) -> Vec<Vec<&'static str>> {
        data.lines().map(|l| l.split('\t').collect()).collect()
    }

    fn num(s: &str) -> i32 {
        s.parse().unwrap()
    }

    #[test]
    fn add_or_sub_quotient_and_fract_against_the_oracle() {
        let t = machine();
        let table = rows(include_str!("../testdata/etex_expr_oracle.tsv"));
        assert!(table.len() > 700);
        for r in table {
            let (x, y, z) = (num(r[1]), num(r[2]), num(r[3]));
            t.arith_error = false;
            let got = match r[0] {
                "add" => t.add_or_sub(x, y, INFINITY, false),
                "sub" => t.add_or_sub(x, y, INFINITY, true),
                "dadd" => t.add_or_sub(x, y, MAX_DIMEN, false),
                "dsub" => t.add_or_sub(x, y, MAX_DIMEN, true),
                "div" | "ddiv" => t.quotient(x, y),
                "scale" => t.fract(x, y, z, INFINITY),
                _ => t.fract(x, y, z, MAX_DIMEN),
            };
            assert_eq!((got, t.arith_error), (num(r[4]), r[5] == "1"), "{:?}", r);
        }
        Tex::dealloc(t);
    }

    /// `\skip`, `\muskip`: `trap_zero_glue` is what prefixed_command does before it defines.
    fn glue_value(t: &mut Tex, w: i32, p: i32) -> i32 {
        if w == 0 && p == 0 {
            t.add_glue_ref(ZERO_GLUE);
            return ZERO_GLUE;
        }
        let s = t.new_spec(ZERO_GLUE);
        t.set_width(s, w);
        t.set_stretch(s, p);
        s
    }

    /// One operation of etex_sa_oracle.tsv, in what prefixed_command calls in this module.
    fn sa_op(t: &mut Tex, op: &str) {
        let f: Vec<&str> = op.split(' ').collect();
        if f[0] == "beg" || f[0] == "end" {
            return group_op(t, f[0]);
        }
        let a = if f[0].as_bytes()[0].is_ascii_uppercase() { 4 } else { 0 };
        let n = num(f[1]);
        match &*f[0].to_ascii_lowercase() {
            "c" => {
                t.find_sa_element(INT_VAL, n, true);
                t.sa_word_define(a, true, t.cur_ptr, num(f[2]));
            }
            "d" => {
                t.find_sa_element(DIMEN_VAL, n, true);
                t.sa_word_define(a, true, t.cur_ptr, num(f[2]));
            }
            k @ ("s" | "m") => {
                t.find_sa_element(if k == "s" { GLUE_VAL } else { MU_VAL }, n, true);
                let p = t.cur_ptr;
                let e = glue_value(t, num(f[2]), num(f[3]));
                t.sa_define(a, true, p, e, p, GLUE_REF, e);
            }
            "t" => {
                t.find_sa_element(TOK_VAL, n, true);
                let p = t.cur_ptr;
                if f.len() == 2 {
                    t.sa_define(a, true, p, NULL, p, UNDEFINED_CS, NULL);
                } else {
                    let m = t.tb_mark();
                    for c in f[2].bytes() {
                        t.tb_push(LETTER_TOKEN + c as i32);
                    }
                    let e = t.tb_commit_ref(m);
                    t.sa_define(a, true, p, e, p, CALL, e);
                }
            }
            "b" => {
                t.cur_box = NULL;
                if f.len() == 3 {
                    t.cur_box = t.new_null_box();
                    t.set_width(t.cur_box, num(f[2]));
                }
                t.cur_val = n;
                t.sa_def_box(a);
            }
            _ => panic!("{}", op),
        }
    }

    #[test]
    fn sparse_registers_trace_as_the_oracle_does() {
        for r in rows(include_str!("../testdata/etex_sa_oracle.tsv")) {
            let t = table_machine();
            print_setup(t);
            t.set_tracing_assigns(1);
            t.set_tracing_restores(1);
            for op in r[0].split(';') {
                sa_op(t, op);
            }
            assert_eq!(printed(t).trim_end(), r[1].replace('|', "\n"), "operations {}", r[0]);
            Tex::dealloc(t);
        }
    }

    #[test]
    fn a_register_back_at_its_default_goes_with_its_index_nodes() {
        for (ty, ops) in [
            (INT_VAL, "beg;c 300 5;end"),
            (DIMEN_VAL, "beg;d 300 5;end"),
            (GLUE_VAL, "beg;s 300 5 0;end"),
            (MU_VAL, "beg;m 300 65536 0;end"),
            (TOK_VAL, "beg;t 300 abc;end"),
            (BOX_VAL, "beg;b 300 5;end"),
            (INT_VAL, "c 32767 1;c 32767 0"),
            (GLUE_VAL, "S 300 3 0;S 300 0 0"),
        ] {
            let t = table_machine();
            let (v, z) = (t.var_used, t.glue_ref_count(ZERO_GLUE));
            for op in ops.split(';') {
                sa_op(t, op);
            }
            assert_eq!(t.sa_root[ty], NULL, "{}", ops);
            assert_eq!(t.var_used, v, "{}", ops);
            assert_eq!(t.glue_ref_count(ZERO_GLUE), z, "{}", ops);
            Tex::dealloc(t);
        }
    }

    /// What an assignment does to the reference count, with nothing assigned.
    fn touch(t: &mut Tex, q: i32) {
        t.add_sa_ref(q);
        t.delete_sa_ref(q);
    }

    #[test]
    fn find_sa_element_makes_only_what_it_is_asked_for() {
        let t = table_machine();
        let v = t.var_used;
        t.find_sa_element(INT_VAL, 300, false);
        assert_eq!((t.cur_ptr, t.var_used), (NULL, v));
        t.find_sa_element(INT_VAL, 0x1234, true);
        let a = t.cur_ptr;
        assert_eq!(t.var_used, v + 4 * INDEX_NODE_SIZE + WORD_NODE_SIZE);
        assert_eq!((t.sa_type(a), t.sa_index(a) % 16, t.sa_num(a), t.sa_lev(a)), (INT_VAL, 4, 0x1234, LEVEL_ONE));
        let mut q = t.link(a);
        for (digit, used) in [(3, 1), (2, 1), (1, 1), (INT_VAL, 1)] {
            assert_eq!((t.sa_index(q), t.sa_used(q)), (digit, used));
            q = t.link(q);
        }
        assert_eq!(q, NULL);
        t.find_sa_element(INT_VAL, 0x1234, false);
        assert_eq!(t.cur_ptr, a);
        t.find_sa_element(INT_VAL, 0x1235, false);
        assert_eq!(t.cur_ptr, NULL);
        // A neighbour shares all four index nodes; one in another fourth digit shares only the first.
        t.find_sa_element(INT_VAL, 0x1235, true);
        assert_eq!(t.var_used, v + 4 * INDEX_NODE_SIZE + 2 * WORD_NODE_SIZE);
        let b = t.cur_ptr;
        assert_eq!(t.link(a), t.link(b));
        assert_eq!(t.sa_used(t.link(a)), 2);
        t.find_sa_element(INT_VAL, 0x5234, true);
        assert_eq!(t.var_used, v + 7 * INDEX_NODE_SIZE + 3 * WORD_NODE_SIZE);
        let c = t.cur_ptr;
        assert_eq!(t.sa_used(t.sa_root[INT_VAL]), 2);
        // They go one at a time, with the nodes nothing else uses.
        touch(t, c);
        assert_eq!(t.var_used, v + 4 * INDEX_NODE_SIZE + 2 * WORD_NODE_SIZE);
        touch(t, b);
        assert_eq!(t.var_used, v + 4 * INDEX_NODE_SIZE + WORD_NODE_SIZE);
        touch(t, a);
        assert_eq!((t.var_used, t.sa_root[INT_VAL]), (v, NULL));
        Tex::dealloc(t);
    }

    #[test]
    fn a_nonzero_register_or_one_with_a_reference_stays() {
        let t = table_machine();
        t.find_sa_element(INT_VAL, 300, true);
        let p = t.cur_ptr;
        t.sa_w_def(p, 5);
        t.find_sa_element(INT_VAL, 300, false);
        assert_eq!(t.cur_ptr, p, "a value other than the default");
        t.add_sa_ref(p); // what \countdef does
        t.sa_w_def(p, 0);
        t.find_sa_element(INT_VAL, 300, false);
        assert_eq!(t.cur_ptr, p, "a reference from \\countdef");
        t.delete_sa_ref(p);
        assert_eq!(t.sa_root[INT_VAL], NULL);
        Tex::dealloc(t);
    }

    fn mark_list(t: &mut Tex, s: &[u8]) -> i32 {
        let m = t.tb_mark();
        for &c in s {
            t.tb_push(LETTER_TOKEN + c as i32);
        }
        t.tb_commit_ref(m)
    }

    fn mark_class(t: &mut Tex, n: i32) -> i32 {
        t.find_sa_element(MARK_VAL, n, true);
        t.cur_ptr
    }

    #[test]
    fn do_marks_moves_the_bottom_mark_to_the_top_when_a_page_fires() {
        let t = table_machine();
        let v = t.var_used;
        let (first, bot) = (mark_list(t, b"a"), mark_list(t, b"b"));
        let q = mark_class(t, 5);
        t.set_sa_first_mark(q, first);
        t.set_sa_bot_mark(q, bot);
        assert!(!t.do_marks(FIRE_UP_INIT, 0, t.sa_mark()));
        assert_eq!((t.sa_top_mark(q), t.sa_first_mark(q), t.sa_bot_mark(q)), (bot, NULL, bot));
        assert_eq!(t.token_ref_count(bot), 1);
        assert!(!t.do_marks(FIRE_UP_DONE, 0, t.sa_mark()));
        assert_eq!(t.sa_first_mark(q), bot);
        assert_eq!(t.token_ref_count(bot), 2);
        // A page with no new mark: first goes, and top is bottom again.
        assert!(!t.do_marks(FIRE_UP_INIT, 0, t.sa_mark()));
        assert_eq!((t.sa_top_mark(q), t.sa_first_mark(q), t.sa_bot_mark(q)), (bot, NULL, bot));
        assert_eq!(t.token_ref_count(bot), 1);
        assert!(t.do_marks(DESTROY_MARKS, 0, t.sa_mark()));
        assert_eq!(t.var_used, v);
        Tex::dealloc(t);
    }

    #[test]
    fn an_empty_bottom_mark_is_forgotten_and_the_class_goes_with_it() {
        let t = table_machine();
        let v = t.var_used;
        let (top, first, bot) = (mark_list(t, b"a"), mark_list(t, b"b"), mark_list(t, b""));
        let q = mark_class(t, 300);
        t.set_sa_top_mark(q, top);
        t.set_sa_first_mark(q, first);
        t.set_sa_bot_mark(q, bot);
        assert!(t.do_marks(FIRE_UP_INIT, 0, t.sa_mark()));
        assert_eq!(t.var_used, v);
        Tex::dealloc(t);
    }

    #[test]
    fn a_nonempty_mark_is_not_taken_for_an_empty_one() {
        // A literal `link(sa_bot_mark(q))=null` would read this word of mem: PORTING section 15.
        let t = table_machine();
        let (first, bot) = (mark_list(t, b"a"), mark_list(t, b"bc"));
        t.set_link(bot, NULL);
        let q = mark_class(t, 300);
        t.set_sa_first_mark(q, first);
        t.set_sa_bot_mark(q, bot);
        assert!(!t.do_marks(FIRE_UP_INIT, 0, t.sa_mark()));
        assert_eq!(t.sa_top_mark(q), bot);
        Tex::dealloc(t);
    }

    #[test]
    fn do_marks_clears_split_marks_and_keeps_a_class_that_still_has_a_bottom_mark() {
        let t = table_machine();
        let (sf, sb, bot) = (mark_list(t, b"a"), mark_list(t, b"b"), mark_list(t, b"c"));
        let q = mark_class(t, 4097);
        let r = mark_class(t, 4098);
        t.set_sa_split_first_mark(q, sf);
        t.set_sa_split_bot_mark(q, sb);
        t.set_sa_bot_mark(q, bot);
        t.add_token_ref(sf);
        t.add_token_ref(sb);
        t.set_sa_split_first_mark(r, sf);
        t.set_sa_split_bot_mark(r, sb);
        assert!(!t.do_marks(VSPLIT_INIT, 0, t.sa_mark()));
        // r had only split marks, so it is gone, and q keeps its bottom mark.
        t.find_sa_element(MARK_VAL, 4098, false);
        assert_eq!(t.cur_ptr, NULL);
        t.find_sa_element(MARK_VAL, 4097, false);
        assert_eq!(t.cur_ptr, q);
        assert_eq!((t.sa_split_first_mark(q), t.sa_split_bot_mark(q), t.sa_bot_mark(q)), (NULL, NULL, bot));
        Tex::dealloc(t);
    }

    #[test]
    fn pseudo_files_give_the_lines_the_oracle_gives() {
        for r in rows(include_str!("../testdata/etex_pseudo_oracle.tsv")) {
            let t = table_machine();
            print_setup(t);
            t.first = 100;
            let (v, d) = (t.var_used, t.dyn_used);
            for c in r[0].bytes() {
                t.append_char(c as i32);
            }
            t.str_room(1);
            let s = t.make_string();
            t.set_new_line_char(num(r[1]));
            t.etex_string_to_pseudo_file(s);
            t.flush_string();
            let mut got = String::new();
            while t.pseudo_input() {
                for k in t.first..t.last {
                    got.push(t.buffer.at(k) as u8 as char);
                }
                got.push('|');
            }
            t.pseudo_close();
            assert_eq!(got, r[2], "{:?}", r);
            assert_eq!((t.pseudo_files, t.var_used, t.dyn_used), (NULL, v, d), "{:?}", r);
            Tex::dealloc(t);
        }
    }

    fn buffer_line(t: &Tex) -> Vec<u8> {
        (t.first..t.last).map(|k| t.buffer.at(k) as u8).collect()
    }

    #[test]
    fn pseudo_close_frees_lines_not_read_yet_and_leaves_the_file_below() {
        let t = table_machine();
        print_setup(t);
        t.first = 100;
        t.set_new_line_char(b'!' as i32);
        let (v, d) = (t.var_used, t.dyn_used);
        for text in [&b"low!er"[..], b"up!per!lines"] {
            for &c in text {
                t.append_char(c as i32);
            }
            t.str_room(1);
            let s = t.make_string();
            t.etex_string_to_pseudo_file(s);
            t.flush_string();
        }
        assert!(t.pseudo_input());
        assert_eq!(buffer_line(t), b"up");
        t.pseudo_close();
        assert!(t.pseudo_input());
        assert_eq!(buffer_line(t), b"low");
        t.pseudo_close();
        assert_eq!((t.pseudo_files, t.var_used, t.dyn_used), (NULL, v, d));
        Tex::dealloc(t);
    }

    #[test]
    fn get_x_or_protected_expands_all_but_protected_macros() {
        let t = table_machine();
        def_macro(t, cs(b'a'), CALL, &[], &[LETTER_TOKEN + b'x' as i32]);
        def_macro(t, cs(b'b'), CALL, &[PROTECTED_TOKEN], &[LETTER_TOKEN + b'y' as i32]);
        def_macro(t, cs(b'c'), CALL, &[], &[cs_tok(b'a'), cs_tok(b'b')]);
        t.set_eq_type(cs(b'r'), RELAX);
        t.set_equiv(cs(b'r'), 256);
        feed(t, &[cs_tok(b'c'), cs_tok(b'r'), LETTER_TOKEN + b'z' as i32]);
        t.get_x_or_protected();
        assert_eq!((t.cur_cmd, t.cur_chr), (LETTER, b'x' as i32), "a macro inside a macro expands");
        t.get_x_or_protected();
        assert_eq!((t.cur_cmd, t.cur_tok), (CALL, cs_tok(b'b')), "a protected macro does not");
        t.get_x_or_protected();
        assert_eq!((t.cur_cmd, t.cur_tok), (RELAX, cs_tok(b'r')), "nor does a command");
        t.get_x_or_protected();
        assert_eq!(t.cur_tok, LETTER_TOKEN + b'z' as i32);
        Tex::dealloc(t);
    }

    #[test]
    fn eTeX_enabled_says_nothing_when_enabled_and_complains_when_not() {
        let t = table_machine();
        print_setup(t);
        assert!(t.eTeX_enabled(true, VALIGN, BEGIN_L_CODE));
        assert_eq!(printed(t), "");
        assert!(!t.eTeX_enabled(false, VALIGN, BEGIN_L_CODE));
        let text = printed(t);
        assert!(text.starts_with("! Improper \\beginL."), "{}", text);
        assert_eq!(t.history, ERROR_MESSAGE_ISSUED);
        Tex::dealloc(t);
    }

    const A: i32 = 65;
    const R_BEGIN: i32 = R_CODE + BEGIN_M_CODE;
    const R_END: i32 = R_CODE + END_M_CODE;
    const L_BEGIN: i32 = L_CODE + BEGIN_M_CODE;
    const L_END: i32 = L_CODE + END_M_CODE;

    /// A font with one character, `A`, 5pt wide.
    fn lr_machine() -> &'static mut Tex {
        let t = table_machine();
        t.char_base[1] = 0;
        t.width_base[1] = 1000;
        t.font_info[A].set_qqqq(FourQuarters { b0: 1, b1: 0, b2: 0, b3: 0 });
        t.font_info[1001].set_sc(5 * UNITY);
        t
    }

    fn ch(t: &mut Tex) -> i32 {
        let p = t.get_avail();
        t.set_font(p, 1);
        t.set_character(p, A);
        p
    }

    /// Chains nodes as a list, and returns its first.
    fn chain(t: &mut Tex, nodes: &[i32]) -> i32 {
        for w in nodes.windows(2) {
            t.set_link(w[0], w[1]);
        }
        nodes.first().copied().unwrap_or(NULL)
    }

    /// `reverse` of a list starting at temp_ptr.
    fn rev(t: &mut Tex, this_box: i32, edge: i32, nodes: &[i32]) -> i32 {
        t.temp_ptr = chain(t, nodes);
        let (mut g, mut c) = (0, 0.0);
        t.reverse(this_box, edge, &mut g, &mut c)
    }

    /// The nodes of a list, as (-1, character) or (type, subtype).
    fn kinds(t: &Tex, p: i32) -> Vec<(i32, i32)> {
        let mut v = Vec::new();
        let mut p = p;
        while p != NULL {
            v.push(if t.is_char_node(p) { (-1, t.character(p)) } else { (t.ty(p), t.subtype(p)) });
            p = t.link(p);
        }
        v
    }

    #[test]
    fn reverse_turns_a_segment_round_and_stops_at_its_end() {
        let t = lr_machine();
        t.cur_dir = 1;
        t.put_LR(R_END);
        let b = t.new_null_box();
        let edge = t.new_edge(0, 0);
        let (c, k, z, r, m, rest) =
            (ch(t), t.new_kern(3 * UNITY), t.new_kern(0), t.new_rule(), t.new_math(UNITY, R_END), t.new_penalty(7));
        t.set_width(r, 2 * UNITY);
        let l = rev(t, b, edge, &[c, k, z, r, m, rest]);
        assert_eq!(l, r);
        assert_eq!(t.link(r), k, "the empty kern has gone");
        assert_eq!(t.link(k), c);
        assert_eq!(t.link(c), edge);
        assert_eq!(t.link(edge), rest, "the rest of the list comes after the edge");
        assert_eq!(t.cur_h, 10 * UNITY, "the widths reversed, and not the math node's");
        assert_eq!(t.width(edge), UNITY, "the math node's width goes to the edge");
        assert_eq!(t.edge_dist(edge), -11 * UNITY);
        assert_eq!((t.LR_ptr, t.LR_problems), (NULL, 0));
        Tex::dealloc(t);
    }

    #[test]
    fn reverse_keeps_the_math_nodes_of_an_opposite_direction_and_kerns_those_of_its_own() {
        let t = lr_machine();
        t.cur_dir = 1;
        t.put_LR(R_END);
        let b = t.new_null_box();
        let edge = t.new_edge(0, 0);
        let (a, e) = (t.new_math(0, L_BEGIN), t.new_math(0, L_END));
        let (x, y) = (t.new_math(UNITY, R_BEGIN), t.new_math(UNITY, R_END));
        let end = t.new_math(0, R_END);
        let l = rev(t, b, edge, &[a, e, x, y, end]);
        // The inner pair changes before into after and back; the pair in this direction are kerns.
        assert_eq!(
            kinds(t, l),
            vec![(KERN_NODE, R_END), (KERN_NODE, R_BEGIN), (MATH_NODE, L_BEGIN), (MATH_NODE, L_END), (EDGE_NODE, 0)]
        );
        assert_eq!(t.subtype(a), L_END, "before became after");
        assert_eq!(t.subtype(e), L_BEGIN, "after became before");
        assert_eq!(t.cur_h, 2 * UNITY);
        assert_eq!(t.LR_problems, 0);
        Tex::dealloc(t);
    }

    #[test]
    fn reverse_counts_what_does_not_match() {
        let t = lr_machine();
        t.cur_dir = 1;
        t.put_LR(R_END);
        let b = t.new_null_box();
        let edge = t.new_edge(0, 0);
        // An \endL with \endR awaited is a problem and a kern; running out of list makes the missing \endR.
        let e = t.new_math(UNITY, L_END);
        let l = rev(t, b, edge, &[e]);
        assert_eq!(t.LR_problems, 10001);
        assert_eq!(t.LR_ptr, NULL);
        assert_eq!(kinds(t, l), vec![(KERN_NODE, L_END), (EDGE_NODE, 0)]);
        assert_eq!(t.cur_h, UNITY);
        Tex::dealloc(t);
    }

    #[test]
    fn reverse_of_a_whole_list_drops_the_kern_at_its_end() {
        let t = lr_machine();
        t.cur_dir = 1;
        let b = t.new_null_box();
        let v = t.var_used;
        let (k, r) = (t.new_kern(2 * UNITY), t.new_rule());
        t.set_width(r, 3 * UNITY);
        let l = rev(t, b, NULL, &[k, r]);
        assert_eq!(l, r);
        assert_eq!(t.link(r), NULL);
        assert_eq!(t.cur_h, 5 * UNITY, "its width still counts");
        assert_eq!(t.var_used, v + RULE_NODE_SIZE);
        Tex::dealloc(t);
    }

    #[test]
    fn reverse_replaces_a_ligature_by_its_character() {
        let t = lr_machine();
        t.cur_dir = 1;
        let b = t.new_null_box();
        let (v, d) = (t.var_used, t.dyn_used);
        let inner = ch(t);
        let lig = t.new_ligature(1, A, inner);
        let l = rev(t, b, NULL, &[lig]);
        assert_eq!(kinds(t, l), vec![(-1, A)]);
        assert_eq!(t.font(l), 1);
        assert_eq!(t.cur_h, 5 * UNITY);
        assert_eq!((t.var_used, t.dyn_used), (v, d + 1), "the ligature and its original characters are gone");
        Tex::dealloc(t);
    }

    #[test]
    fn reverse_fixes_stretched_glue_at_its_width_and_keeps_leaders() {
        let t = lr_machine();
        t.cur_dir = 1;
        let b = t.new_null_box();
        t.set_glue_sign(b, STRETCHING);
        t.set_glue_order(b, FIL);
        t.set_glue_set(b, 0.5);
        let spec = |t: &mut Tex, order| {
            let s = t.new_spec(ZERO_GLUE);
            t.set_width(s, UNITY);
            t.set_stretch(s, 2 * UNITY);
            t.set_stretch_order(s, order);
            s
        };
        let (s1, s2, s3) = (spec(t, FIL), spec(t, FIL), spec(t, FILL));
        let (g1, g2, g3) = (t.new_glue(s1), t.new_glue(s2), t.new_glue(s3));
        t.set_subtype(g2, A_LEADERS);
        t.set_glue_ref_count(s1, NULL);
        t.set_glue_ref_count(s2, NULL);
        t.set_glue_ref_count(s3, NULL);
        let l = rev(t, b, NULL, &[g3, g1, g2]);
        assert_eq!(l, g2);
        assert_eq!(t.link(g1), g3);
        assert_eq!((t.ty(g3), t.glue_ptr(g3)), (GLUE_NODE, s3), "glue of another order stays, at its own width");
        // Each stretch of 2pt at ratio 1/2 adds a point.
        assert_eq!((t.ty(g1), t.width(g1)), (KERN_NODE, 2 * UNITY));
        assert_eq!(t.ty(g2), GLUE_NODE);
        let n = t.glue_ptr(g2);
        assert_ne!(n, s2);
        assert_eq!((t.width(n), t.stretch_order(n), t.shrink_order(n)), (2 * UNITY, FILLL + 1, FILLL + 1));
        assert_eq!(t.cur_h, 5 * UNITY);
        Tex::dealloc(t);
    }

    #[test]
    fn just_copy_copies_shallow_and_skips_what_a_display_does_not_measure() {
        let t = lr_machine();
        let inner = ch(t);
        let h = t.new_null_box();
        t.set_list_ptr(h, inner);
        t.set_width(h, 7 * UNITY);
        let inner2 = ch(t);
        let (c, lig) = (ch(t), t.new_ligature(1, A, inner2));
        let (r, k, m) = (t.new_rule(), t.new_kern(UNITY), t.new_math(2 * UNITY, L_BEGIN));
        let s = t.new_spec(ZERO_GLUE);
        t.set_glue_ref_count(s, NULL);
        let g = t.new_glue(s);
        let leader = ch(t);
        t.set_leader_ptr(g, leader);
        let (pen, d) = (t.new_penalty(5), t.new_disc());
        let list = chain(t, &[c, h, r, lig, k, m, pen, d, g]);
        let tail = t.new_penalty(9);
        t.just_copy(list, TEMP_HEAD, tail);
        let got = t.link(TEMP_HEAD);
        assert_eq!(
            kinds(t, got),
            vec![
                (-1, A),
                (HLIST_NODE, NORMAL),
                (RULE_NODE, NORMAL),
                (-1, A),
                (KERN_NODE, NORMAL),
                (MATH_NODE, L_BEGIN),
                (GLUE_NODE, NORMAL),
                (PENALTY_NODE, 0),
            ]
        );
        let box_copy = t.link(got);
        assert_eq!((t.width(box_copy), t.list_ptr(box_copy)), (7 * UNITY, NULL), "a box loses its list");
        let mut glue_copy = got;
        while t.is_char_node(glue_copy) || t.ty(glue_copy) != GLUE_NODE {
            glue_copy = t.link(glue_copy);
        }
        assert_eq!((t.glue_ptr(glue_copy), t.leader_ptr(glue_copy)), (s, NULL), "glue loses its leaders");
        assert_eq!(t.glue_ref_count(s), NULL + 2, "and shares its spec");
        Tex::dealloc(t);
    }

    #[test]
    fn just_reverse_copies_the_line_when_it_may_and_reverses_up_to_the_matching_end() {
        let t = lr_machine();
        t.cur_dir = 0;
        t.put_LR(R_END);
        let head = t.new_kern(0);
        let (c, k, m, c2) = (ch(t), t.new_kern(UNITY), t.new_math(4 * UNITY, R_END), ch(t));
        chain(t, &[head, c, k, m, c2]);
        t.set_link(TEMP_HEAD, NULL);
        t.just_reverse(head);
        assert_eq!(t.cur_dir, 1);
        let l = t.link(TEMP_HEAD);
        // k then c then the edge, whose width is the math node's, and then what followed the math node.
        assert_eq!(kinds(t, l), vec![(KERN_NODE, NORMAL), (-1, A), (EDGE_NODE, 0), (-1, A)]);
        let e = t.link(t.link(l));
        assert_eq!(t.width(e), 4 * UNITY);
        assert_eq!(t.LR_ptr, NULL);
        assert_eq!(kinds(t, t.link(head)).len(), 4, "the original is untouched, since it was copied");
        Tex::dealloc(t);
    }

    #[test]
    fn just_reverse_with_a_prepared_copy_takes_the_line_itself() {
        let t = lr_machine();
        t.cur_dir = 0;
        t.put_LR(R_END);
        let head = t.new_kern(0);
        let (c, x, y) = (ch(t), t.new_math(0, R_BEGIN), t.new_math(0, R_END));
        chain(t, &[head, c, x, y]);
        let stale = t.new_kern(0);
        t.set_link(TEMP_HEAD, stale);
        let v = t.var_used;
        t.just_reverse(head);
        // The stale copy is flushed, the line comes off head, and the pair in the reflected direction are kerns.
        assert_eq!(t.link(head), NULL);
        assert_eq!(t.var_used, v - MEDIUM_NODE_SIZE + EDGE_NODE_SIZE);
        let l = t.link(TEMP_HEAD);
        assert_eq!(kinds(t, l), vec![(KERN_NODE, R_END), (KERN_NODE, R_BEGIN), (-1, A), (EDGE_NODE, 0)]);
        Tex::dealloc(t);
    }

    #[test]
    fn warnings_at_the_end_of_a_file_name_what_is_open() {
        let t = table_machine();
        print_setup(t);
        t.history = SPOTLESS;
        t.max_print_line = 200;
        t.set_tracing_nesting(1);
        t.in_open = 1;
        t.grp_stack[1] = t.cur_boundary;
        t.if_stack[1] = t.cond_ptr;
        t.line = 3;
        t.new_save_level(SEMI_SIMPLE_GROUP);
        // What conditional does for an \ifnum with no \else seen, then an \iftrue in its \else part.
        let push = |t: &mut Tex, chr: i32, limit: i32, line: i32| {
            let p = t.get_node(IF_NODE_SIZE);
            t.set_link(p, t.cond_ptr);
            t.set_ty(p, t.if_limit);
            t.set_subtype(p, t.cur_if);
            t.set_if_line_field(p, t.if_line);
            t.cond_ptr = p;
            t.cur_if = chr;
            t.if_limit = limit;
            t.if_line = line;
        };
        push(t, IF_INT_CODE, IF_CODE, 4);
        push(t, IF_TRUE_CODE, FI_CODE, 5);
        t.file_warning();
        assert_eq!(
            printed(t),
            "Warning: end of file when semi simple group (level 1) entered at line 3 is incomplete\n\
             Warning: end of file when \\iftrue\\else entered on line 5 is incomplete\n\
             Warning: end of file when \\ifnum entered on line 4 is incomplete\n"
        );
        assert_eq!(t.history, WARNING_ISSUED);
        Tex::dealloc(t);
    }

    #[test]
    fn group_and_if_warnings_need_tracing_and_a_file_level() {
        for (nesting, name, warned) in [(0, 18, false), (1, 17, false), (1, 18, true), (1, 19, true)] {
            let t = table_machine();
            print_setup(t);
            t.history = SPOTLESS;
            t.set_tracing_nesting(nesting);
            t.set_name(name);
            t.set_index(1);
            t.in_open = 1;
            t.line = 3;
            t.max_print_line = 200;
            t.new_save_level(SIMPLE_GROUP);
            t.grp_stack[1] = t.cur_boundary;
            // Where unsave calls it from.
            t.save_ptr -= 1;
            t.cur_level -= 1;
            t.group_warning();
            assert_ne!(t.grp_stack[1], t.cur_boundary);
            assert_eq!(
                printed(t),
                if warned { "Warning: end of simple group (level 1) entered at line 3 of a different file\n" } else { "" }
            );
            assert_eq!(t.history, if warned { WARNING_ISSUED } else { SPOTLESS });

            t.history = SPOTLESS;
            let p = t.get_node(IF_NODE_SIZE);
            t.set_link(p, NULL);
            t.cond_ptr = p;
            t.cur_if = IF_TRUE_CODE;
            t.if_line = 7;
            t.if_stack[1] = p;
            t.if_warning();
            assert_eq!(t.if_stack[1], NULL);
            assert_eq!(
                printed(t),
                if warned { "Warning: end of \\iftrue entered on line 7 of a different file\n" } else { "" }
            );
            assert_eq!(t.history, if warned { WARNING_ISSUED } else { SPOTLESS });
            Tex::dealloc(t);
        }
    }
}
