//! §151-179 Part 10: data structures for boxes and their friends.
//!
//! The field macros of this part are in access.rs; what is here is the node constructors, in WEB's
//! order of stores.  A node's words other than those stored here keep what the allocator left in
//! them, as in WEB: see `get_node` for the two SyncTeX words of a node of 4 words or more.
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
    /// §154
    pub fn new_null_box(&mut self) -> i32 {
        let p = self.get_node(BOX_NODE_SIZE);
        self.set_ty(p, HLIST_NODE);
        self.set_subtype(p, MIN_QUARTERWORD);
        self.set_width(p, 0);
        self.set_depth(p, 0);
        self.set_height(p, 0);
        self.set_shift_amount(p, 0);
        self.set_list_ptr(p, NULL);
        self.set_glue_sign(p, NORMAL);
        self.set_glue_order(p, NORMAL);
        self.set_glue_set(p, 0.0);
        p
    }

    /// §157
    pub fn new_rule(&mut self) -> i32 {
        let p = self.get_node(RULE_NODE_SIZE);
        self.set_ty(p, RULE_NODE);
        self.set_subtype(p, 0); // the subtype is not used
        self.set_width(p, NULL_FLAG);
        self.set_depth(p, NULL_FLAG);
        self.set_height(p, NULL_FLAG);
        p
    }

    /// §162
    pub fn new_ligature(&mut self, f: i32, c: i32, q: i32) -> i32 {
        let p = self.get_node(SMALL_NODE_SIZE);
        self.set_ty(p, LIGATURE_NODE);
        let lc = self.lig_char(p);
        self.set_font(lc, f);
        self.set_character(lc, c);
        self.set_lig_ptr(p, q);
        self.set_subtype(p, 0);
        p
    }

    /// §162
    pub fn new_lig_item(&mut self, c: i32) -> i32 {
        let p = self.get_node(SMALL_NODE_SIZE);
        self.set_character(p, c);
        self.set_lig_ptr(p, NULL);
        p
    }

    /// §163
    pub fn new_disc(&mut self) -> i32 {
        let p = self.get_node(SMALL_NODE_SIZE);
        self.set_ty(p, DISC_NODE);
        self.set_replace_count(p, 0);
        self.set_pre_break(p, NULL);
        self.set_post_break(p, NULL);
        p
    }

    /// §165
    pub fn new_math(&mut self, w: i32, s: i32) -> i32 {
        let p = self.get_node(MEDIUM_NODE_SIZE);
        self.set_ty(p, MATH_NODE);
        self.set_subtype(p, s);
        self.set_width(p, w);
        p
    }

    /// §169
    pub fn new_spec(&mut self, p: i32) -> i32 {
        let q = self.get_node(GLUE_SPEC_SIZE);
        self.mem[q] = self.mem[p];
        self.set_glue_ref_count(q, NULL);
        self.set_width(q, self.width(p));
        self.set_stretch(q, self.stretch(p));
        self.set_shrink(q, self.shrink(p));
        q
    }

    /// §170
    pub fn new_param_glue(&mut self, n: i32) -> i32 {
        let p = self.get_node(MEDIUM_NODE_SIZE);
        self.set_ty(p, GLUE_NODE);
        self.set_subtype(p, n + 1);
        self.set_leader_ptr(p, NULL);
        // §242 Current mem equivalent of glue parameter number n
        let q = self.glue_par(n);
        self.set_glue_ptr(p, q);
        self.set_glue_ref_count(q, self.glue_ref_count(q) + 1);
        p
    }

    /// §171
    pub fn new_glue(&mut self, q: i32) -> i32 {
        let p = self.get_node(MEDIUM_NODE_SIZE);
        self.set_ty(p, GLUE_NODE);
        self.set_subtype(p, NORMAL);
        self.set_leader_ptr(p, NULL);
        self.set_glue_ptr(p, q);
        self.set_glue_ref_count(q, self.glue_ref_count(q) + 1);
        p
    }

    /// §172
    pub fn new_skip_param(&mut self, n: i32) -> i32 {
        // §242
        let g = self.glue_par(n);
        self.temp_ptr = self.new_spec(g);
        let p = self.new_glue(self.temp_ptr);
        self.set_glue_ref_count(self.temp_ptr, NULL);
        self.set_subtype(p, n + 1);
        p
    }

    /// §174
    pub fn new_kern(&mut self, w: i32) -> i32 {
        let p = self.get_node(MEDIUM_NODE_SIZE);
        self.set_ty(p, KERN_NODE);
        self.set_subtype(p, NORMAL);
        self.set_width(p, w);
        p
    }

    /// §176
    pub fn new_penalty(&mut self, m: i32) -> i32 {
        let p = self.get_node(MEDIUM_NODE_SIZE);
        self.set_ty(p, PENALTY_NODE);
        self.set_subtype(p, 0); // the subtype is not used
        self.set_penalty(p, m);
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::memkit::*;

    #[test]
    fn new_null_box_is_all_zero_and_normal() {
        let t = mem_machine();
        let v = t.var_used;
        // A used block, so nothing here passes by being zero already.
        let junk = t.get_node(BOX_NODE_SIZE);
        for k in 0..BOX_NODE_SIZE {
            t.mem[junk + k] = Word::hh(-1, -1);
        }
        t.free_node(junk, BOX_NODE_SIZE);
        let p = t.new_null_box();
        assert_eq!(p, junk);
        assert_eq!(t.var_used, v + BOX_NODE_SIZE);
        assert_eq!((t.ty(p), t.subtype(p), t.link(p)), (HLIST_NODE, MIN_QUARTERWORD, NULL));
        assert_eq!((t.width(p), t.depth(p), t.height(p), t.shift_amount(p)), (0, 0, 0, 0));
        assert_eq!((t.list_ptr(p), t.glue_sign(p), t.glue_order(p)), (NULL, NORMAL, NORMAL));
        assert_eq!(t.glue_set(p), 0.0);
        Tex::dealloc(t);
    }

    #[test]
    fn new_rule_is_running_in_all_directions() {
        let t = mem_machine();
        let p = t.new_rule();
        assert_eq!(t.var_used, 20 + RULE_NODE_SIZE);
        assert_eq!((t.ty(p), t.subtype(p), t.link(p)), (RULE_NODE, 0, NULL));
        assert_eq!((t.width(p), t.depth(p), t.height(p)), (NULL_FLAG, NULL_FLAG, NULL_FLAG));
        Tex::dealloc(t);
    }

    #[test]
    fn new_ligature_and_new_lig_item() {
        let t = mem_machine();
        let p = t.new_ligature(5, 200, 77);
        assert_eq!(t.var_used, 20 + SMALL_NODE_SIZE);
        assert_eq!((t.ty(p), t.subtype(p)), (LIGATURE_NODE, 0));
        assert_eq!((t.font(t.lig_char(p)), t.character(t.lig_char(p)), t.lig_ptr(p)), (5, 200, 77));
        let q = t.new_lig_item(300);
        assert_eq!((t.character(q), t.lig_ptr(q), t.link(q)), (300, NULL, NULL));
        Tex::dealloc(t);
    }

    #[test]
    fn new_disc_is_empty() {
        let t = mem_machine();
        let p = t.new_disc();
        assert_eq!(t.var_used, 20 + SMALL_NODE_SIZE);
        assert_eq!((t.ty(p), t.replace_count(p), t.pre_break(p), t.post_break(p)), (DISC_NODE, 0, NULL, NULL));
        Tex::dealloc(t);
    }

    #[test]
    fn new_math_kern_penalty_are_medium() {
        let t = mem_machine();
        let p = t.new_math(5, AFTER);
        assert_eq!((t.ty(p), t.subtype(p), t.width(p), t.var_used), (MATH_NODE, AFTER, 5, 20 + MEDIUM_NODE_SIZE));
        let p = t.new_kern(-6);
        assert_eq!((t.ty(p), t.subtype(p), t.width(p), t.var_used), (KERN_NODE, NORMAL, -6, 20 + 2 * MEDIUM_NODE_SIZE));
        let p = t.new_penalty(-10000);
        assert_eq!((t.ty(p), t.subtype(p), t.penalty(p), t.var_used), (PENALTY_NODE, 0, -10000, 20 + 3 * MEDIUM_NODE_SIZE));
        Tex::dealloc(t);
    }

    #[test]
    fn new_spec_copies_all_but_the_reference_count() {
        let t = mem_machine();
        t.set_width(SS_GLUE, 3);
        let q = t.new_spec(SS_GLUE);
        assert_eq!(t.var_used, 20 + GLUE_SPEC_SIZE);
        assert_eq!(t.glue_ref_count(q), NULL);
        assert_eq!((t.stretch_order(q), t.shrink_order(q)), (FIL, FIL));
        assert_eq!((t.width(q), t.stretch(q), t.shrink(q)), (3, UNITY, UNITY));
        Tex::dealloc(t);
    }

    #[test]
    fn new_glue_takes_a_reference() {
        let t = mem_machine();
        let rc = t.glue_ref_count(FIL_GLUE);
        let p = t.new_glue(FIL_GLUE);
        assert_eq!(t.glue_ref_count(FIL_GLUE), rc + 1);
        assert_eq!((t.ty(p), t.subtype(p), t.leader_ptr(p), t.glue_ptr(p)), (GLUE_NODE, NORMAL, NULL, FIL_GLUE));
        assert_eq!(t.var_used, 20 + MEDIUM_NODE_SIZE);
        Tex::dealloc(t);
    }

    #[test]
    fn new_param_glue_shares_the_parameter() {
        let t = mem_machine();
        t.set_glue_par(TAB_SKIP_CODE, FILL_GLUE);
        let rc = t.glue_ref_count(FILL_GLUE);
        let p = t.new_param_glue(TAB_SKIP_CODE);
        assert_eq!(t.glue_ref_count(FILL_GLUE), rc + 1);
        assert_eq!((t.ty(p), t.subtype(p), t.leader_ptr(p), t.glue_ptr(p)), (GLUE_NODE, TAB_SKIP_CODE + 1, NULL, FILL_GLUE));
        Tex::dealloc(t);
    }

    #[test]
    fn new_skip_param_copies_the_parameter() {
        let t = mem_machine();
        t.set_glue_par(BASELINE_SKIP_CODE, SS_GLUE);
        let rc = t.glue_ref_count(SS_GLUE);
        let p = t.new_skip_param(BASELINE_SKIP_CODE);
        assert_eq!(t.glue_ref_count(SS_GLUE), rc, "not a reference to the parameter");
        let q = t.temp_ptr;
        assert_ne!(q, SS_GLUE);
        assert_eq!(t.glue_ptr(p), q);
        assert_eq!(t.glue_ref_count(q), NULL, "one reference, the node's");
        assert_eq!((t.stretch(q), t.shrink(q)), (UNITY, UNITY));
        assert_eq!((t.ty(p), t.subtype(p), t.leader_ptr(p)), (GLUE_NODE, BASELINE_SKIP_CODE + 1, NULL));
        assert_eq!(t.var_used, 20 + GLUE_SPEC_SIZE + MEDIUM_NODE_SIZE);
        Tex::dealloc(t);
    }

    /// The order of the two allocations in new_skip_param fixes the addresses.
    #[test]
    fn new_skip_param_takes_the_spec_first() {
        let t = mem_machine();
        let top = t.rover + 1000;
        let p = t.new_skip_param(BASELINE_SKIP_CODE);
        assert_eq!(t.temp_ptr, top - GLUE_SPEC_SIZE);
        assert_eq!(p, top - GLUE_SPEC_SIZE - MEDIUM_NODE_SIZE);
        Tex::dealloc(t);
    }

    /// §1907: get_node writes the two SyncTeX words of a node of 4 words or more, and not of a smaller one.
    #[test]
    fn the_synctex_words_are_set_by_get_node() {
        let t = mem_machine();
        t.set_synctex_tag(7);
        t.line = 8;
        let p = t.new_kern(1);
        assert_eq!((t.sync_tag(p + MEDIUM_NODE_SIZE), t.sync_line(p + MEDIUM_NODE_SIZE)), (7, 8));
        let p = t.new_null_box();
        assert_eq!((t.sync_tag(p + BOX_NODE_SIZE), t.sync_line(p + BOX_NODE_SIZE)), (7, 8));
        let p = t.new_rule();
        assert_eq!((t.sync_tag(p + RULE_NODE_SIZE), t.sync_line(p + RULE_NODE_SIZE)), (7, 8));
        // Those words overlap the node's own fields in a small node, so nothing may be written there.
        let junk = t.get_node(3);
        t.mem[junk + 1] = Word::hh(55, 66);
        t.free_node(junk, 3);
        let p = t.get_node(3);
        assert_eq!(p, junk);
        assert_eq!(t.link(p), NULL);
        assert_eq!((t.mem[p + 1].rh(), t.mem[p + 1].lh()), (t.rlink(p), t.llink(p)));
        assert_eq!(t.rlink(p), t.rover, "as free_node left it, and not the tag");
        let p = t.get_node(2);
        assert_eq!(t.link(p), NULL, "and not the tag");
        Tex::dealloc(t);
    }
}
