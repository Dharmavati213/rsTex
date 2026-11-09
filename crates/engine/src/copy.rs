//! §217-224 Parts 13-14: destroying and copying boxes.
//!
//! Token lists held by nodes are headers in `toks`: `add_token_ref` and `delete_token_ref` are in
//! toks.rs, and nothing here reads them.  Glue specifications, char nodes, `margin_char` and the
//! copy's temporary head stay in `mem`.
//!
//! DROPPED §218: delete_token_ref is in toks.rs; token_ref_count is in the toks header.
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

/// Math, glue, kern and penalty nodes, whose codes are 9 to 12.
#[inline(always)]
const fn is_medium_node(t: i32) -> bool {
    (t - MATH_NODE) as u32 <= (PENALTY_NODE - MATH_NODE) as u32
}

/// `x` if `c`, otherwise `y`, with no branch.
#[inline(always)]
const fn pick(c: bool, x: i32, y: i32) -> i32 {
    let m = -(c as i32);
    (x & m) | (y & !m)
}

impl Tex {
    /// A node whose copy is its four words: not a glue node with leaders, nor one when `copy_node_list(NULL)` would take a cell from `get_avail`.
    #[inline(always)]
    fn copy_is_plain_medium(&self, p: i32) -> bool {
        let t = self.ty(p);
        is_medium_node(t) & ((t != GLUE_NODE) | ((self.leader_ptr(p) == NULL) & (self.avail != NULL)))
    }

    /// §219
    pub fn delete_glue_ref(&mut self, p: i32) {
        self.fast_delete_glue_ref(p);
    }

    /// §220
    pub fn flush_node_list(&mut self, p: i32) {
        let mut p = p;
        let mut q: i32;
        while p != NULL {
            q = self.link(p);
            if self.is_char_node(p) {
                self.free_avail(p);
            } else {
                'done: {
                    // The commonest cases of the jump below, with no jump.  A list of mixed kinds is what a predictor cannot learn: on
                    // SGA3 the jump is missed on 40% of the nodes, and these are 3/4 of what is not a character.  A glue node takes off a
                    // reference that is not the last, and any other node takes nothing off its own first word.
                    let t = self.ty(p);
                    let is_glue = t == GLUE_NODE;
                    let g = pick(is_glue, self.glue_ptr(p), p);
                    if is_medium_node(t) & (!is_glue | ((self.glue_ref_count(g) != NULL) & (self.leader_ptr(p) == NULL))) {
                        self.set_glue_ref_count(g, self.glue_ref_count(g) - is_glue as i32);
                        self.free_node(p, MEDIUM_NODE_SIZE);
                        break 'done;
                    }
                    match t {
                        HLIST_NODE | VLIST_NODE | UNSET_NODE => {
                            self.flush_node_list(self.list_ptr(p));
                            self.free_node(p, BOX_NODE_SIZE);
                            break 'done;
                        }
                        RULE_NODE => {
                            self.free_node(p, RULE_NODE_SIZE);
                            break 'done;
                        }
                        INS_NODE => {
                            self.flush_node_list(self.ins_ptr(p));
                            self.delete_glue_ref(self.split_top_ptr(p));
                            self.free_node(p, INS_NODE_SIZE);
                            break 'done;
                        }
                        WHATSIT_NODE => {
                            // §1605 Wipe out the whatsit node p and goto done
                            match self.subtype(p) {
                                OPEN_NODE => self.free_node(p, OPEN_NODE_SIZE),
                                WRITE_NODE | SPECIAL_NODE | LATESPECIAL_NODE => {
                                    self.delete_token_ref(self.write_tokens(p));
                                    self.free_node(p, WRITE_NODE_SIZE);
                                    break 'done;
                                }
                                CLOSE_NODE | LANGUAGE_NODE => self.free_node(p, SMALL_NODE_SIZE),
                                PDF_LITERAL_NODE | PDF_LATELITERAL_NODE => {
                                    self.delete_token_ref(self.pdf_literal_data(p));
                                    self.free_node(p, WRITE_NODE_SIZE);
                                }
                                PDF_COLORSTACK_NODE => {
                                    if self.pdf_colorstack_cmd(p) <= COLORSTACK_DATA {
                                        self.delete_token_ref(self.pdf_colorstack_data(p));
                                        self.free_node(p, PDF_COLORSTACK_SETTER_NODE_SIZE);
                                    } else {
                                        self.free_node(p, PDF_COLORSTACK_GETTER_NODE_SIZE);
                                    }
                                }
                                PDF_SETMATRIX_NODE => {
                                    self.delete_token_ref(self.pdf_setmatrix_data(p));
                                    self.free_node(p, PDF_SETMATRIX_NODE_SIZE);
                                }
                                PDF_SAVE_NODE => self.free_node(p, PDF_SAVE_NODE_SIZE),
                                PDF_RESTORE_NODE => self.free_node(p, PDF_RESTORE_NODE_SIZE),
                                PDF_REFOBJ_NODE => self.free_node(p, PDF_REFOBJ_NODE_SIZE),
                                PDF_REFXFORM_NODE => self.free_node(p, PDF_REFXFORM_NODE_SIZE),
                                PDF_REFXIMAGE_NODE => self.free_node(p, PDF_REFXIMAGE_NODE_SIZE),
                                PDF_ANNOT_NODE => {
                                    self.delete_token_ref(self.pdf_annot_data(p));
                                    self.free_node(p, PDF_ANNOT_NODE_SIZE);
                                }
                                PDF_START_LINK_NODE => {
                                    if self.pdf_link_attr(p) != NULL {
                                        self.delete_token_ref(self.pdf_link_attr(p));
                                    }
                                    self.delete_action_ref(self.pdf_link_action(p));
                                    self.free_node(p, PDF_ANNOT_NODE_SIZE);
                                }
                                PDF_END_LINK_NODE => self.free_node(p, SMALL_NODE_SIZE),
                                PDF_DEST_NODE => {
                                    if self.pdf_dest_named_id(p) > 0 {
                                        self.delete_token_ref(self.pdf_dest_id(p));
                                    }
                                    self.free_node(p, PDF_DEST_NODE_SIZE);
                                }
                                PDF_THREAD_NODE | PDF_START_THREAD_NODE => {
                                    if self.pdf_thread_named_id(p) > 0 {
                                        self.delete_token_ref(self.pdf_thread_id(p));
                                    }
                                    if self.pdf_thread_attr(p) != NULL {
                                        self.delete_token_ref(self.pdf_thread_attr(p));
                                    }
                                    self.free_node(p, PDF_THREAD_NODE_SIZE);
                                }
                                PDF_END_THREAD_NODE => self.free_node(p, SMALL_NODE_SIZE),
                                PDF_SAVE_POS_NODE => self.free_node(p, SMALL_NODE_SIZE),
                                PDF_SNAP_REF_POINT_NODE => self.free_node(p, SMALL_NODE_SIZE),
                                PDF_SNAPY_NODE => {
                                    self.delete_glue_ref(self.snap_glue_ptr(p));
                                    self.free_node(p, SNAP_NODE_SIZE);
                                }
                                PDF_SNAPY_COMP_NODE => self.free_node(p, SMALL_NODE_SIZE),
                                PDF_INTERWORD_SPACE_ON_NODE => self.free_node(p, SMALL_NODE_SIZE),
                                PDF_INTERWORD_SPACE_OFF_NODE => self.free_node(p, SMALL_NODE_SIZE),
                                PDF_FAKE_SPACE_NODE => self.free_node(p, SMALL_NODE_SIZE),
                                PDF_RUNNING_LINK_OFF_NODE => self.free_node(p, SMALL_NODE_SIZE),
                                PDF_RUNNING_LINK_ON_NODE => self.free_node(p, SMALL_NODE_SIZE),
                                _ => self.confusion(b"ext3"),
                            }
                            break 'done;
                        }
                        GLUE_NODE => {
                            self.fast_delete_glue_ref(self.glue_ptr(p));
                            if self.leader_ptr(p) != NULL {
                                self.flush_node_list(self.leader_ptr(p));
                            }
                            self.free_node(p, MEDIUM_NODE_SIZE);
                            break 'done;
                        }
                        KERN_NODE | MATH_NODE | PENALTY_NODE => {
                            self.free_node(p, MEDIUM_NODE_SIZE);
                            break 'done;
                        }
                        MARGIN_KERN_NODE => {
                            self.free_avail(self.margin_char(p));
                            self.free_node(p, MARGIN_KERN_NODE_SIZE);
                            break 'done;
                        }
                        LIGATURE_NODE => self.flush_node_list(self.lig_ptr(p)),
                        MARK_NODE => self.delete_token_ref(self.mark_ptr(p)),
                        DISC_NODE => {
                            self.flush_node_list(self.pre_break(p));
                            self.flush_node_list(self.post_break(p));
                        }
                        ADJUST_NODE => self.flush_node_list(self.adjust_ptr(p)),
                        // §874 Cases of flush_node_list that arise in mlists only
                        STYLE_NODE => {
                            self.free_node(p, STYLE_NODE_SIZE);
                            break 'done;
                        }
                        CHOICE_NODE => {
                            self.flush_node_list(self.display_mlist(p));
                            self.flush_node_list(self.text_mlist(p));
                            self.flush_node_list(self.script_mlist(p));
                            self.flush_node_list(self.script_script_mlist(p));
                            self.free_node(p, STYLE_NODE_SIZE);
                            break 'done;
                        }
                        ORD_NOAD | OP_NOAD | BIN_NOAD | REL_NOAD | OPEN_NOAD | CLOSE_NOAD | PUNCT_NOAD | INNER_NOAD
                        | RADICAL_NOAD | OVER_NOAD | UNDER_NOAD | VCENTER_NOAD | ACCENT_NOAD => {
                            if self.math_type(self.nucleus(p)) >= SUB_BOX {
                                self.flush_node_list(self.info(self.nucleus(p)));
                            }
                            if self.math_type(self.supscr(p)) >= SUB_BOX {
                                self.flush_node_list(self.info(self.supscr(p)));
                            }
                            if self.math_type(self.subscr(p)) >= SUB_BOX {
                                self.flush_node_list(self.info(self.subscr(p)));
                            }
                            if self.ty(p) == RADICAL_NOAD {
                                self.free_node(p, RADICAL_NOAD_SIZE);
                            } else if self.ty(p) == ACCENT_NOAD {
                                self.free_node(p, ACCENT_NOAD_SIZE);
                            } else {
                                self.free_node(p, NOAD_SIZE);
                            }
                            break 'done;
                        }
                        LEFT_NOAD | RIGHT_NOAD => {
                            self.free_node(p, NOAD_SIZE);
                            break 'done;
                        }
                        FRACTION_NOAD => {
                            self.flush_node_list(self.info(self.numerator(p)));
                            self.flush_node_list(self.info(self.denominator(p)));
                            self.free_node(p, FRACTION_NOAD_SIZE);
                            break 'done;
                        }
                        _ => self.confusion(b"flushing"),
                    }
                    self.free_node(p, SMALL_NODE_SIZE);
                } // done:
            }
            p = q;
        }
    }

    /// §222
    pub fn copy_node_list(&mut self, p: i32) -> i32 {
        // Nothing to copy, as with the leaders of every glue node and most boxes' lists: 70% of the calls on SGA3.  `h`
        // would come off `avail` and go back as it was.
        if p == NULL && self.avail != NULL {
            return NULL;
        }
        let mut p = p;
        let h = self.get_avail();
        let mut q = h;
        let mut r: i32;
        let mut words: i32;
        while p != NULL {
            // §223 Make a copy of node p in node r
            words = 1; // this setting occurs in more branches than any other
            if self.is_char_node(p) {
                r = self.fast_get_avail();
                self.mem[r] = self.mem[p];
                words = 0;
            } else if self.copy_is_plain_medium(p) {
                // The cases of glue, kern, math and penalty nodes below, with no jump: `flush_node_list` says why.  A glue node
                // adds a reference, and any other node adds nothing to its own first word.
                r = self.get_node_leaf(MEDIUM_NODE_SIZE);
                let is_glue = self.ty(p) == GLUE_NODE;
                let g = pick(is_glue, self.glue_ptr(p), p);
                self.set_glue_ref_count(g, self.glue_ref_count(g) + is_glue as i32);
                self.mem[r] = self.mem[p];
                self.mem[r + 1] = self.mem[p + 1];
                // §224 and §1928: the last two words of a kern, math or penalty node go over whole, and a glue node's as `int`s, which leave `lh`
                // as it was.  A line box whose glue fields were never set reads an old `lh` as its glue sign and order: PORTING section 15.
                let old = pick(is_glue, r, p);
                self.mem[r + 2] = Word::hh(self.mem[p + 2].rh(), self.mem[old + 2].lh());
                self.mem[r + 3] = Word::hh(self.mem[p + 3].rh(), self.mem[old + 3].lh());
                words = 0;
            } else {
                // §224 Case statement to copy different types and set words to the number of initial words not yet copied
                match self.ty(p) {
                    HLIST_NODE | VLIST_NODE | UNSET_NODE => {
                        r = self.get_node(BOX_NODE_SIZE);
                        // §1926 Copy the box SyncTeX information
                        self.set_sync_tag(r + BOX_NODE_SIZE, self.sync_tag(p + BOX_NODE_SIZE));
                        self.set_sync_line(r + BOX_NODE_SIZE, self.sync_line(p + BOX_NODE_SIZE));
                        self.mem[r + 6] = self.mem[p + 6];
                        self.mem[r + 5] = self.mem[p + 5]; // copy the last two words
                        let c = self.copy_node_list(self.list_ptr(p));
                        self.set_list_ptr(r, c); // this affects mem[r+5]
                        words = 5;
                    }
                    RULE_NODE => {
                        r = self.get_node(RULE_NODE_SIZE);
                        // §1927: WEB does not copy the SyncTeX information of a rule
                        words = RULE_NODE_SIZE - SYNCTEX_FIELD_SIZE;
                    }
                    INS_NODE => {
                        r = self.get_node(INS_NODE_SIZE);
                        self.mem[r + 4] = self.mem[p + 4];
                        self.add_glue_ref(self.split_top_ptr(p));
                        let c = self.copy_node_list(self.ins_ptr(p));
                        self.set_ins_ptr(r, c); // this affects mem[r+4]
                        words = INS_NODE_SIZE - 1;
                    }
                    WHATSIT_NODE => {
                        // §1604 Make a partial copy of the whatsit node p and make r point to it; set words to the number of initial words not yet copied
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
                    GLUE_NODE => {
                        r = self.get_node(MEDIUM_NODE_SIZE);
                        self.add_glue_ref(self.glue_ptr(p));
                        // §1928 Copy the medium sized node SyncTeX information
                        self.set_sync_tag(r + MEDIUM_NODE_SIZE, self.sync_tag(p + MEDIUM_NODE_SIZE));
                        self.set_sync_line(r + MEDIUM_NODE_SIZE, self.sync_line(p + MEDIUM_NODE_SIZE));
                        self.set_glue_ptr(r, self.glue_ptr(p));
                        let c = self.copy_node_list(self.leader_ptr(p));
                        self.set_leader_ptr(r, c);
                    }
                    KERN_NODE | MATH_NODE | PENALTY_NODE => {
                        r = self.get_node(MEDIUM_NODE_SIZE);
                        words = MEDIUM_NODE_SIZE;
                    }
                    MARGIN_KERN_NODE => {
                        r = self.get_node(MARGIN_KERN_NODE_SIZE);
                        let c = self.fast_get_avail();
                        self.set_margin_char(r, c);
                        self.set_font(self.margin_char(r), self.font(self.margin_char(p)));
                        self.set_character(self.margin_char(r), self.character(self.margin_char(p)));
                        words = SMALL_NODE_SIZE;
                    }
                    LIGATURE_NODE => {
                        r = self.get_node(SMALL_NODE_SIZE);
                        let lc = self.lig_char(r);
                        self.mem[lc] = self.mem[self.lig_char(p)]; // copy font and character
                        let c = self.copy_node_list(self.lig_ptr(p));
                        self.set_lig_ptr(r, c);
                    }
                    DISC_NODE => {
                        r = self.get_node(SMALL_NODE_SIZE);
                        let c = self.copy_node_list(self.pre_break(p));
                        self.set_pre_break(r, c);
                        let c = self.copy_node_list(self.post_break(p));
                        self.set_post_break(r, c);
                    }
                    MARK_NODE => {
                        r = self.get_node(SMALL_NODE_SIZE);
                        self.add_token_ref(self.mark_ptr(p));
                        words = SMALL_NODE_SIZE;
                    }
                    ADJUST_NODE => {
                        r = self.get_node(SMALL_NODE_SIZE);
                        let c = self.copy_node_list(self.adjust_ptr(p));
                        self.set_adjust_ptr(r, c);
                        // words=1=small_node_size-1
                    }
                    _ => self.confusion(b"copying"),
                }
            }
            while words > 0 {
                words -= 1;
                self.mem[r + words] = self.mem[p + words];
            }
            self.set_link(q, r);
            q = r;
            p = self.link(p);
        }
        self.set_link(q, NULL);
        q = self.link(h);
        self.free_avail(h);
        q
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::memkit::*;
    use crate::show::showkit::*;

    /// Everything allocated is given back: the two counts return, the ring holds the rest, and `check_mem`, which
    /// prints only on trouble, says nothing.
    fn memory_is_whole(t: &mut Tex, v: i32, d: i32, id: &str) {
        assert_eq!((t.var_used, t.dyn_used), (v, d), "the counts, {}", id);
        let _ = t.get_node(0o10000000000); // merge
        t.sort_avail();
        assert_eq!(recount_var_used(t), v, "the ring, {}", id);
        t.check_mem(false);
    }

    /// The memory `\copy` takes and flushing gives back: from the oracle's statistics in the format
    /// file, tests/oracle/gen_show_tables.py.
    #[test]
    fn copy_and_flush_take_and_give_what_the_oracle_does_and_nothing_leaks() {
        let rows = table(include_bytes!("../testdata/copy_oracle.tsv"));
        assert!(rows.len() >= 100);
        for row in &rows {
            let id = &row[0][..];
            let n = |k: usize| row[k].parse::<i32>().unwrap();
            let t = case_machine(id, false);
            let (v0, d0) = (t.var_used, t.dyn_used);
            let p = build(t, id);
            let (v1, d1) = (t.var_used, t.dyn_used);

            let c = t.copy_node_list(p);
            assert_ne!(c, p, "{}", id);
            assert_eq!((t.var_used - v1, t.dyn_used - d1), (n(1), n(2)), "what \\copy takes, {}", id);
            let top = t.toks_top;

            t.flush_node_list(c);
            assert_eq!((t.var_used, t.dyn_used), (v1, d1), "what flushing the copy gives back, {}", id);
            assert_eq!(t.toks_top, top, "a copy shares its token lists, {}", id);

            t.flush_node_list(p);
            assert_eq!((v1 - t.var_used, d1 - t.dyn_used), (n(3), n(4)), "what flushing the original gives back, {}", id);
            memory_is_whole(t, v0, d0, id);

            // Build, copy and flush the other way round: the original goes first, and the token lists are
            // recycled, so the token heap does not grow a second time.
            let p = build(t, id);
            let c = t.copy_node_list(p);
            let top = t.toks_top;
            t.flush_node_list(p);
            t.flush_node_list(c);
            memory_is_whole(t, v0, d0, id);
            let p = build(t, id);
            let c = t.copy_node_list(p);
            assert_eq!(t.toks_top, top, "the token heap is reused, {}", id);
            t.flush_node_list(c);
            t.flush_node_list(p);
            memory_is_whole(t, v0, d0, id);
            Tex::dealloc(t);
        }
    }

    /// Unset nodes: no box of the oracle holds one once its alignment ends, so this has no table.
    #[test]
    fn unset_nodes_copy_and_flush() {
        for id in ["unset_cells", "unset_span", "unset_stretch"] {
            let t = case_machine(id, false);
            let (v0, d0) = (t.var_used, t.dyn_used);
            let p = build(t, id);
            let (v1, d1) = (t.var_used, t.dyn_used);
            let c = t.copy_node_list(p);
            t.flush_node_list(c);
            assert_eq!((t.var_used, t.dyn_used), (v1, d1), "{}", id);
            t.flush_node_list(p);
            memory_is_whole(t, v0, d0, id);
            Tex::dealloc(t);
        }
    }

    #[test]
    fn a_copy_shares_glue_and_tokens_and_duplicates_the_rest() {
        let t = mem_machine();
        let s = t.new_spec(ZERO_GLUE);
        let g = t.new_glue(s);
        t.set_glue_ref_count(s, NULL);
        let b = t.new_null_box();
        t.set_list_ptr(b, g);
        let w = t.tb_mark();
        t.tb_push(OTHER_TOKEN + 97);
        let l = t.tb_commit_ref(w);
        let m = t.get_node(SMALL_NODE_SIZE);
        t.set_ty(m, MARK_NODE);
        t.set_mark_ptr(m, l);
        t.set_mark_class(m, 0);
        t.set_link(b, m);
        let c = t.copy_node_list(b);
        // The box and the glue node are new; the spec and the token list are the same, with one more reference.
        let g2 = t.list_ptr(c);
        assert_ne!(c, b);
        assert_ne!(g2, g);
        assert_eq!(t.glue_ptr(g2), s);
        assert_eq!(t.glue_ref_count(s), NULL + 1);
        let m2 = t.link(c);
        assert_ne!(m2, m);
        assert_eq!(t.mark_ptr(m2), l);
        assert_eq!(t.token_ref_count(l), 1);
        assert_eq!(t.link(m2), NULL);
        t.flush_node_list(b);
        assert_eq!(t.glue_ref_count(s), NULL);
        assert_eq!(t.token_ref_count(l), 0);
        Tex::dealloc(t);
    }

    /// Without printing, what a_copy_shows_as_the_original_does sees in show.rs.
    #[test]
    fn a_copy_has_every_field() {
        let t = mem_machine();
        let k = t.new_kern(5);
        let b = t.new_null_box();
        t.set_ty(b, VLIST_NODE);
        t.set_subtype(b, 3);
        t.set_width(b, 1);
        t.set_depth(b, 2);
        t.set_height(b, 3);
        t.set_shift_amount(b, 4);
        t.set_glue_sign(b, SHRINKING);
        t.set_glue_order(b, FILL);
        t.set_glue_set(b, 0.25);
        t.set_list_ptr(b, k);
        let c = t.copy_node_list(b);
        assert_ne!(c, b);
        assert_eq!((t.ty(c), t.subtype(c), t.link(c)), (VLIST_NODE, 3, NULL));
        assert_eq!((t.width(c), t.depth(c), t.height(c), t.shift_amount(c)), (1, 2, 3, 4));
        assert_eq!((t.glue_sign(c), t.glue_order(c), t.glue_set(c)), (SHRINKING, FILL, 0.25));
        let k2 = t.list_ptr(c);
        assert_ne!(k2, k, "the list is copied too");
        assert_eq!((t.ty(k2), t.width(k2)), (KERN_NODE, 5));

        let s = t.new_spec(ZERO_GLUE);
        let i = t.get_node(INS_NODE_SIZE);
        t.set_ty(i, INS_NODE);
        t.set_subtype(i, 100);
        t.set_height(i, 1);
        t.set_depth(i, 2);
        t.set_split_top_ptr(i, s);
        t.set_float_cost(i, 7);
        t.set_ins_ptr(i, NULL);
        let c = t.copy_node_list(i);
        assert_eq!((t.ty(c), t.subtype(c)), (INS_NODE, 100));
        assert_eq!((t.height(c), t.depth(c), t.split_top_ptr(c), t.float_cost(c), t.ins_ptr(c)), (1, 2, s, 7, NULL));
        assert_eq!(t.glue_ref_count(s), NULL + 1);
        Tex::dealloc(t);
    }

    /// §1926-§1928: SyncTeX words are copied for boxes and medium nodes, and left to get_node's for a rule.
    #[test]
    fn the_synctex_words_of_a_copy() {
        let t = mem_machine();
        t.set_synctex_tag(7);
        t.line = 8;
        let k = t.new_kern(1);
        let g = t.new_glue(ZERO_GLUE);
        let r = t.new_rule();
        let b = t.new_null_box();
        t.set_link(k, g);
        t.set_link(g, r);
        t.set_link(r, b);
        t.set_synctex_tag(9);
        t.line = 10;
        let c = t.copy_node_list(k);
        let (k2, g2) = (c, t.link(c));
        let (r2, b2) = (t.link(g2), t.link(t.link(g2)));
        // A kern's four words are copied whole, so its SyncTeX words come along.
        assert_eq!((t.sync_tag(k2 + MEDIUM_NODE_SIZE), t.sync_line(k2 + MEDIUM_NODE_SIZE)), (7, 8));
        assert_eq!((t.sync_tag(g2 + MEDIUM_NODE_SIZE), t.sync_line(g2 + MEDIUM_NODE_SIZE)), (7, 8));
        assert_eq!((t.sync_tag(b2 + BOX_NODE_SIZE), t.sync_line(b2 + BOX_NODE_SIZE)), (7, 8));
        assert_eq!((t.sync_tag(r2 + RULE_NODE_SIZE), t.sync_line(r2 + RULE_NODE_SIZE)), (9, 10), "§1927: not copied");
        Tex::dealloc(t);
    }

    #[test]
    fn copy_of_nothing_is_nothing_and_takes_nothing() {
        let t = mem_machine();
        let (v, d) = (t.var_used, t.dyn_used);
        assert_eq!(t.copy_node_list(NULL), NULL);
        assert_eq!((t.var_used, t.dyn_used), (v, d));
        t.flush_node_list(NULL);
        assert_eq!((t.var_used, t.dyn_used), (v, d));
        Tex::dealloc(t);
    }

    #[test]
    fn delete_glue_ref_frees_at_the_last_reference() {
        let t = mem_machine();
        let v = t.var_used;
        let s = t.new_spec(ZERO_GLUE);
        t.add_glue_ref(s);
        t.delete_glue_ref(s);
        assert_eq!(t.glue_ref_count(s), NULL);
        assert_eq!(t.var_used, v + GLUE_SPEC_SIZE);
        t.delete_glue_ref(s);
        assert_eq!(t.var_used, v);
        Tex::dealloc(t);
    }
}
