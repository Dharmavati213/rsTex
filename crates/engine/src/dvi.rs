//! §610-671 Parts 31-32: device-independent file format, shipping pages out.
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
    /// §625
    pub fn dvi_swap(&mut self) {
        if self.dvi_ptr > 0x7FFFFFFF - self.dvi_offset {
            self.cur_s = -2;
            self.fatal_error(b"dvi length exceeds \"7FFFFFFF");
        }
        if self.dvi_limit == self.dvi_buf_size {
            self.write_dvi(0, self.half_buf - 1);
            self.dvi_limit = self.half_buf;
            self.dvi_offset += self.dvi_buf_size;
            self.dvi_ptr = 0;
        } else {
            self.write_dvi(self.half_buf, self.dvi_buf_size - 1);
            self.dvi_limit = self.dvi_buf_size;
        }
        self.dvi_gone += self.half_buf;
    }

    /// §627
    pub fn dvi_four(&mut self, x: i32) {
        let mut x = x;
        if x >= 0 {
            self.dvi_out(x / 0o100000000);
        } else {
            x += 0o10000000000;
            x += 0o10000000000;
            self.dvi_out((x / 0o100000000) + 128);
        }
        x %= 0o100000000;
        self.dvi_out(x / 0o200000);
        x %= 0o200000;
        self.dvi_out(x / 0o400);
        self.dvi_out(x % 0o400);
    }

    /// §628
    pub fn dvi_pop(&mut self, l: i32) {
        if l == self.dvi_offset + self.dvi_ptr && self.dvi_ptr > 0 {
            self.dvi_ptr -= 1;
        } else {
            self.dvi_out(POP);
        }
    }

    /// §629
    pub fn dvi_font_def(&mut self, f: i32) {
        if f <= 256 + FONT_BASE {
            self.dvi_out(FNT_DEF1);
            self.dvi_out(f - FONT_BASE - 1);
        } else {
            self.dvi_out(FNT_DEF1 + 1);
            self.dvi_out((f - FONT_BASE - 1) / 0o400);
            self.dvi_out((f - FONT_BASE - 1) % 0o400);
        }
        let check = self.font_check[f];
        self.dvi_out(check.b0);
        self.dvi_out(check.b1);
        self.dvi_out(check.b2);
        self.dvi_out(check.b3);
        self.dvi_four(self.font_size[f]);
        self.dvi_four(self.font_dsize[f]);
        self.dvi_out(self.length(self.font_area[f]));
        self.dvi_out(self.length(self.font_name[f]));
        // §630 Output the font name whose internal number is |f|
        for k in self.str_start[self.font_area[f]]..self.str_start[self.font_area[f] + 1] {
            self.dvi_out(self.str_pool.at(k));
        }
        for k in self.str_start[self.font_name[f]]..self.str_start[self.font_name[f] + 1] {
            self.dvi_out(self.str_pool.at(k));
        }
    }

    /// §634
    pub fn movement(&mut self, w: i32, o: i32) {
        let mut w = w;
        let q = self.get_node(MOVEMENT_NODE_SIZE); // new node for the top of the stack
        self.set_width(q, w);
        self.set_location(q, self.dvi_offset + self.dvi_ptr);
        if o == DOWN1 {
            self.set_link(q, self.down_ptr);
            self.down_ptr = q;
        } else {
            self.set_link(q, self.right_ptr);
            self.right_ptr = q;
        }
        // §638 Look at the other stack entries until deciding what sort of DVI command to generate
        let mut p = self.link(q);
        let mut mstate = NONE_SEEN;
        let mut found = false;
        'look: while p != NULL {
            if self.width(p) == w {
                // §639 Consider a node with matching width; |goto found| if it's a hit
                match (mstate, self.info(p)) {
                    (NONE_SEEN, YZ_OK) | (NONE_SEEN, Y_OK) | (Z_SEEN, YZ_OK) | (Z_SEEN, Y_OK) => {
                        if self.location(p) < self.dvi_gone {
                            break 'look;
                        }
                        // §640 Change buffered instruction to |y| or |w| and |goto found|
                        let mut k = self.location(p) - self.dvi_offset;
                        if k < 0 {
                            k += self.dvi_buf_size;
                        }
                        self.dvi_buf.put(k, self.dvi_buf.at(k) + Y1 - DOWN1);
                        self.set_info(p, Y_HERE);
                        found = true;
                        break 'look;
                    }
                    (NONE_SEEN, Z_OK) | (Y_SEEN, YZ_OK) | (Y_SEEN, Z_OK) => {
                        if self.location(p) < self.dvi_gone {
                            break 'look;
                        }
                        // §641 Change buffered instruction to |z| or |x| and |goto found|
                        let mut k = self.location(p) - self.dvi_offset;
                        if k < 0 {
                            k += self.dvi_buf_size;
                        }
                        self.dvi_buf.put(k, self.dvi_buf.at(k) + Z1 - DOWN1);
                        self.set_info(p, Z_HERE);
                        found = true;
                        break 'look;
                    }
                    (NONE_SEEN, Y_HERE)
                    | (NONE_SEEN, Z_HERE)
                    | (Y_SEEN, Z_HERE)
                    | (Z_SEEN, Y_HERE) => {
                        found = true;
                        break 'look;
                    }
                    _ => {}
                }
            } else {
                match (mstate, self.info(p)) {
                    (NONE_SEEN, Y_HERE) => mstate = Y_SEEN,
                    (NONE_SEEN, Z_HERE) => mstate = Z_SEEN,
                    (Y_SEEN, Z_HERE) | (Z_SEEN, Y_HERE) => break 'look,
                    _ => {}
                }
            }
            p = self.link(p);
        }
        // not_found:
        if !found {
            // §637 Generate a |down| or |right| command for |w| and |return|
            self.set_info(q, YZ_OK);
            let abs_w = (w as i64).abs(); // `i64`: a move of exactly -2^31 has no `abs` in `i32`
            if abs_w >= 0o40000000 {
                self.dvi_out(o + 3); // |down4| or |right4|
                self.dvi_four(w);
                return;
            }
            if abs_w >= 0o100000 {
                self.dvi_out(o + 2); // |down3| or |right3|
                if w < 0 {
                    w += 0o100000000;
                }
                self.dvi_out(w / 0o200000);
                w %= 0o200000;
                self.dvi_out(w / 0o400); // 2:
            } else if abs_w >= 0o200 {
                self.dvi_out(o + 1); // |down2| or |right2|
                if w < 0 {
                    w += 0o200000;
                }
                self.dvi_out(w / 0o400); // 2:
            } else {
                self.dvi_out(o); // |down1| or |right1|
                if w < 0 {
                    w += 0o400;
                }
            }
            self.dvi_out(w % 0o400); // 1:
            return;
        }
        // found:
        // §636 Generate a |y0| or |z0| command in order to reuse a previous appearance of |w|
        let mut q = q;
        self.set_info(q, self.info(p));
        if self.info(q) == Y_HERE {
            self.dvi_out(o + Y0 - DOWN1); // |y0| or |w0|
            while self.link(q) != p {
                q = self.link(q);
                match self.info(q) {
                    YZ_OK => self.set_info(q, Z_OK),
                    Y_OK => self.set_info(q, D_FIXED),
                    _ => {}
                }
            }
        } else {
            self.dvi_out(o + Z0 - DOWN1); // |z0| or |x0|
            while self.link(q) != p {
                q = self.link(q);
                match self.info(q) {
                    YZ_OK => self.set_info(q, Y_OK),
                    Z_OK => self.set_info(q, D_FIXED),
                    _ => {}
                }
            }
        }
    }

    /// §642
    pub fn prune_movements(&mut self, l: i32) {
        while self.down_ptr != NULL {
            if self.location(self.down_ptr) < l {
                break;
            }
            let p = self.down_ptr;
            self.down_ptr = self.link(p);
            self.free_node(p, MOVEMENT_NODE_SIZE);
        }
        // done:
        while self.right_ptr != NULL {
            if self.location(self.right_ptr) < l {
                return;
            }
            let p = self.right_ptr;
            self.right_ptr = self.link(p);
            self.free_node(p, MOVEMENT_NODE_SIZE);
        }
    }

    /// §647
    pub fn hlist_out(&mut self) {
        let mut cur_g = 0;
        let mut cur_glue = 0.0f64;
        let mut save_h;
        let mut save_v;
        let mut edge;
        let this_box = self.temp_ptr;
        let g_order = self.glue_order(this_box);
        let g_sign = self.glue_sign(this_box);
        let mut p = self.list_ptr(this_box);
        self.cur_s += 1;
        if self.cur_s > 0 {
            self.dvi_out(PUSH);
        }
        if self.cur_s > self.max_push {
            self.max_push = self.cur_s;
        }
        let save_loc = self.dvi_offset + self.dvi_ptr;
        let base_line = self.cur_v;
        let mut prev_p = this_box + LIST_OFFSET;
        // §1714 Initialize |hlist_out| for mixed direction typesetting
        if self.eTeX_ex() {
            // §1710 Initialize the LR stack
            self.put_LR(BEFORE); // this will never match
            if self.box_lr(this_box) == DLIST {
                if self.cur_dir == RIGHT_TO_LEFT {
                    self.cur_dir = LEFT_TO_RIGHT;
                    self.cur_h -= self.width(this_box);
                } else {
                    self.set_box_lr(this_box, 0);
                }
            }
            if self.cur_dir == RIGHT_TO_LEFT && self.box_lr(this_box) != REVERSED {
                // §1721 Reverse the complete hlist and set the subtype to |reversed|
                save_h = self.cur_h;
                self.temp_ptr = p;
                p = self.new_kern(0);
                self.set_sync_tag(p + MEDIUM_NODE_SIZE, 0); // SyncTeX: do nothing, it is too late
                self.set_link(prev_p, p);
                self.cur_h = 0;
                let rv = self.reverse(this_box, NULL, &mut cur_g, &mut cur_glue);
                self.set_link(p, rv);
                self.set_width(p, -self.cur_h);
                self.cur_h = save_h;
                self.set_box_lr(this_box, REVERSED);
            }
        }
        let mut left_edge = self.cur_h;
        // §1918 Start hlist SyncTeX information record
        self.synctex_hlist(this_box);
        // §648 Output node |p| for |hlist_out| and move to the next node
        while p != NULL {
            'reswitch: loop {
                if self.is_char_node(p) {
                    self.synch_h();
                    self.synch_v();
                    loop {
                        self.f = self.font(p);
                        self.c = self.character(p);
                        let (f, c) = (self.f, self.c);
                        if f != self.dvi_f {
                            // §649 Change font |dvi_f| to |f|
                            if !self.font_used[f] {
                                self.dvi_font_def(f);
                                self.font_used[f] = true;
                            }
                            if f <= 64 + FONT_BASE {
                                self.dvi_out(f - FONT_BASE - 1 + FNT_NUM_0);
                            } else if f <= 256 + FONT_BASE {
                                self.dvi_out(FNT1);
                                self.dvi_out(f - FONT_BASE - 1);
                            } else {
                                self.dvi_out(FNT1 + 1);
                                self.dvi_out((f - FONT_BASE - 1) / 0o400);
                                self.dvi_out((f - FONT_BASE - 1) % 0o400);
                            }
                            self.dvi_f = f;
                        }
                        if self.font_ec[f] >= c
                            && self.font_bc[f] <= c
                            && self.char_exists(self.orig_char_info(f, c))
                        {
                            // N.B.: not |char_info|
                            if c >= 128 {
                                self.dvi_out(SET1);
                            }
                            self.dvi_out(c);
                            self.cur_h += self.char_width(f, self.orig_char_info(f, c));
                        }
                        // dropped: MLTeX, §1886's substitution, under |mltex_enabled_p|, which is always false.
                        // continue:
                        prev_p = self.link(prev_p); // N.B.: not |prev_p:=p|, |p| might be |lig_trick|
                        p = self.link(p);
                        if !self.is_char_node(p) {
                            break;
                        }
                    }
                    // §1921 Record current point SyncTeX information
                    self.synctex_current();
                    self.dvi_h = self.cur_h;
                } else {
                    // §650 Output the non-|char_node| |p| for |hlist_out| and move to the next node
                    'next_p: {
                        'move_past: {
                            'fin_rule: {
                                match self.ty(p) {
                                    HLIST_NODE | VLIST_NODE => {
                                        // §651 Output a box in an hlist
                                        if self.list_ptr(p) == NULL {
                                            // §1920 Record void list SyncTeX information
                                            if self.ty(p) == VLIST_NODE {
                                                self.synctex_void_vlist(p, this_box);
                                            } else {
                                                self.synctex_void_hlist(p, this_box);
                                            }
                                            self.cur_h += self.width(p);
                                        } else {
                                            save_h = self.dvi_h;
                                            save_v = self.dvi_v;
                                            self.cur_v = base_line + self.shift_amount(p); // shift the box down
                                            self.temp_ptr = p;
                                            edge = self.cur_h + self.width(p);
                                            if self.cur_dir == RIGHT_TO_LEFT {
                                                self.cur_h = edge;
                                            }
                                            if self.ty(p) == VLIST_NODE {
                                                self.vlist_out();
                                            } else {
                                                self.hlist_out();
                                            }
                                            self.dvi_h = save_h;
                                            self.dvi_v = save_v;
                                            self.cur_h = edge;
                                            self.cur_v = base_line;
                                        }
                                    }
                                    RULE_NODE => {
                                        self.rule_ht = self.height(p);
                                        self.rule_dp = self.depth(p);
                                        self.rule_wd = self.width(p);
                                        break 'fin_rule;
                                    }
                                    WHATSIT_NODE => {
                                        // §1614 Output the whatsit node |p| in an hlist
                                        self.out_what(p);
                                    }
                                    GLUE_NODE => {
                                        // §653 Move right or output leaders
                                        self.round_glue(
                                            p,
                                            this_box,
                                            g_sign,
                                            g_order,
                                            &mut cur_g,
                                            &mut cur_glue,
                                        );
                                        if self.eTeX_ex() {
                                            // §1699 Handle a glue node for mixed direction typesetting
                                            let g = self.g;
                                            if (g_sign == STRETCHING
                                                && self.stretch_order(g) == g_order)
                                                || (g_sign == SHRINKING
                                                    && self.shrink_order(g) == g_order)
                                            {
                                                self.fast_delete_glue_ref(g);
                                                if self.subtype(p) < A_LEADERS {
                                                    self.set_ty(p, KERN_NODE);
                                                    self.set_width(p, self.rule_wd);
                                                } else {
                                                    self.g = self.get_node(GLUE_SPEC_SIZE);
                                                    let g = self.g;
                                                    self.set_stretch_order(g, FILLL + 1); // will never match
                                                    self.set_shrink_order(g, FILLL + 1);
                                                    self.set_width(g, self.rule_wd);
                                                    self.set_stretch(g, 0);
                                                    self.set_shrink(g, 0);
                                                    self.set_glue_ptr(p, g);
                                                }
                                            }
                                        }
                                        if self.subtype(p) >= A_LEADERS {
                                            // §654 Output leaders in an hlist, |goto fin_rule| if a rule or to |next_p| if done
                                            let leader_box = self.leader_ptr(p);
                                            if self.ty(leader_box) == RULE_NODE {
                                                self.rule_ht = self.height(leader_box);
                                                self.rule_dp = self.depth(leader_box);
                                                break 'fin_rule;
                                            }
                                            let leader_wd = self.width(leader_box);
                                            if leader_wd > 0 && self.rule_wd > 0 {
                                                self.rule_wd += 10; // compensate for floating-point rounding
                                                if self.cur_dir == RIGHT_TO_LEFT {
                                                    self.cur_h -= 10;
                                                }
                                                edge = self.cur_h + self.rule_wd;
                                                let mut lx = 0;
                                                // §655 Let |cur_h| be the position of the first box, and set |leader_wd+lx|
                                                if self.subtype(p) == A_LEADERS {
                                                    save_h = self.cur_h;
                                                    self.cur_h = left_edge
                                                        + leader_wd
                                                            * ((self.cur_h - left_edge)
                                                                / leader_wd);
                                                    if self.cur_h < save_h {
                                                        self.cur_h += leader_wd;
                                                    }
                                                } else {
                                                    self.lq = self.rule_wd / leader_wd; // the number of box copies
                                                    self.lr = self.rule_wd % leader_wd; // the remaining space
                                                    if self.subtype(p) == C_LEADERS {
                                                        self.cur_h += self.lr / 2;
                                                    } else {
                                                        lx = self.lr / (self.lq + 1);
                                                        self.cur_h +=
                                                            (self.lr - (self.lq - 1) * lx) / 2;
                                                    }
                                                }
                                                while self.cur_h + leader_wd <= edge {
                                                    // §656 Output a leader box at |cur_h|, then advance |cur_h| by |leader_wd+lx|
                                                    self.cur_v =
                                                        base_line + self.shift_amount(leader_box);
                                                    self.synch_v();
                                                    save_v = self.dvi_v;
                                                    self.synch_h();
                                                    save_h = self.dvi_h;
                                                    self.temp_ptr = leader_box;
                                                    if self.cur_dir == RIGHT_TO_LEFT {
                                                        self.cur_h += leader_wd;
                                                    }
                                                    let outer_doing_leaders = self.doing_leaders;
                                                    self.doing_leaders = true;
                                                    if self.ty(leader_box) == VLIST_NODE {
                                                        self.vlist_out();
                                                    } else {
                                                        self.hlist_out();
                                                    }
                                                    self.doing_leaders = outer_doing_leaders;
                                                    self.dvi_v = save_v;
                                                    self.dvi_h = save_h;
                                                    self.cur_v = base_line;
                                                    self.cur_h = save_h + leader_wd + lx;
                                                }
                                                if self.cur_dir == RIGHT_TO_LEFT {
                                                    self.cur_h = edge;
                                                } else {
                                                    self.cur_h = edge - 10;
                                                }
                                                break 'next_p;
                                            }
                                        }
                                        break 'move_past;
                                    }
                                    MARGIN_KERN_NODE => {
                                        self.cur_h += self.width(p); // separate the |margin_kern_node| case in |hlist_out|
                                    }
                                    KERN_NODE => {
                                        // §1923 Record |kern_node| SyncTeX information
                                        self.synctex_kern(p, this_box);
                                        self.cur_h += self.width(p);
                                    }
                                    MATH_NODE => {
                                        // §1924 Record |math_node| SyncTeX information
                                        self.synctex_math(p, this_box);
                                        // §1716 Handle a math node in |hlist_out|
                                        if self.eTeX_ex() {
                                            // §1717 Adjust the LR stack for the |hlist_out| routine; if necessary reverse an hlist segment and |goto reswitch|
                                            if self.end_LR(p) {
                                                if self.info(self.LR_ptr) == self.end_LR_type(p) {
                                                    self.pop_LR();
                                                } else if self.subtype(p) > L_CODE {
                                                    self.LR_problems += 1;
                                                }
                                            } else {
                                                self.push_LR(p);
                                                if self.LR_dir(p) != self.cur_dir {
                                                    // §1722 Reverse an hlist segment and |goto reswitch|
                                                    save_h = self.cur_h;
                                                    self.temp_ptr = self.link(p);
                                                    self.rule_wd = self.width(p);
                                                    self.free_node(p, MEDIUM_NODE_SIZE); // SyncTeX: p is a |math_node|
                                                    self.cur_dir = self.reflected();
                                                    p = self.new_edge(self.cur_dir, self.rule_wd);
                                                    self.set_link(prev_p, p);
                                                    self.cur_h =
                                                        self.cur_h - left_edge + self.rule_wd;
                                                    let e = self.new_edge(self.reflected(), 0);
                                                    let rv = self.reverse(
                                                        this_box,
                                                        e,
                                                        &mut cur_g,
                                                        &mut cur_glue,
                                                    );
                                                    self.set_link(p, rv);
                                                    self.set_edge_dist(p, self.cur_h);
                                                    self.cur_dir = self.reflected();
                                                    self.cur_h = save_h;
                                                    continue 'reswitch;
                                                }
                                            }
                                            self.set_ty(p, KERN_NODE);
                                        }
                                        self.cur_h += self.width(p);
                                    }
                                    LIGATURE_NODE => {
                                        // §826 Make node |p| look like a |char_node| and |goto reswitch|
                                        self.mem[LIG_TRICK] = self.mem[self.lig_char(p)];
                                        self.set_link(LIG_TRICK, self.link(p));
                                        p = LIG_TRICK;
                                        continue 'reswitch;
                                    }
                                    EDGE_NODE => {
                                        // §1720 Cases of |hlist_out| that arise in mixed direction text only
                                        self.cur_h += self.width(p);
                                        left_edge = self.cur_h + self.edge_dist(p);
                                        self.cur_dir = self.subtype(p);
                                    }
                                    _ => {}
                                }
                                break 'next_p;
                            }
                            // fin_rule:
                            // §652 Output a rule in an hlist
                            if self.is_running(self.rule_ht) {
                                self.rule_ht = self.height(this_box);
                            }
                            if self.is_running(self.rule_dp) {
                                self.rule_dp = self.depth(this_box);
                            }
                            self.rule_ht += self.rule_dp; // this is the rule thickness
                            if self.rule_ht > 0 && self.rule_wd > 0 {
                                // we don't output empty rules
                                self.synch_h();
                                self.cur_v = base_line + self.rule_dp;
                                self.synch_v();
                                self.dvi_out(SET_RULE);
                                self.dvi_four(self.rule_ht);
                                self.dvi_four(self.rule_wd);
                                self.cur_v = base_line;
                                self.dvi_h += self.rule_wd;
                            }
                        }
                        // move_past:
                        self.cur_h += self.rule_wd;
                        // §1922 Record horizontal |rule_node| or |glue_node| SyncTeX information
                        self.synctex_horizontal_rule_or_glue(p, this_box);
                    }
                    // next_p:
                    prev_p = p;
                    p = self.link(p);
                }
                break;
            }
        }
        // §1919 Finish hlist SyncTeX information record
        self.synctex_tsilh(this_box);
        // §1715 Finish |hlist_out| for mixed direction typesetting
        if self.eTeX_ex() {
            // §1718 Check for LR anomalies at the end of |hlist_out|
            while self.info(self.LR_ptr) != BEFORE {
                if self.info(self.LR_ptr) > L_CODE {
                    self.LR_problems += 10000;
                }
                self.pop_LR();
            }
            self.pop_LR();
            if self.box_lr(this_box) == DLIST {
                self.cur_dir = RIGHT_TO_LEFT;
            }
        }
        self.prune_movements(save_loc);
        if self.cur_s > 0 {
            self.dvi_pop(save_loc);
        }
        self.cur_s -= 1;
    }

    /// §657
    pub fn vlist_out(&mut self) {
        let mut cur_g = 0;
        let mut cur_glue = 0.0f64;
        let mut save_h;
        let mut save_v;
        let this_box = self.temp_ptr;
        let g_order = self.glue_order(this_box);
        let g_sign = self.glue_sign(this_box);
        let mut p = self.list_ptr(this_box);
        self.cur_s += 1;
        if self.cur_s > 0 {
            self.dvi_out(PUSH);
        }
        if self.cur_s > self.max_push {
            self.max_push = self.cur_s;
        }
        let save_loc = self.dvi_offset + self.dvi_ptr;
        let left_edge = self.cur_h;
        // §1916 Start vlist SyncTeX information record
        self.synctex_vlist(this_box);
        self.cur_v -= self.height(this_box);
        let top_edge = self.cur_v;
        // §658 Output node |p| for |vlist_out| and move to the next node
        while p != NULL {
            if self.is_char_node(p) {
                self.confusion(b"vlistout");
            }
            // §659 Output the non-|char_node| |p| for |vlist_out|
            'next_p: {
                'move_past: {
                    'fin_rule: {
                        match self.ty(p) {
                            HLIST_NODE | VLIST_NODE => {
                                // §660 Output a box in a vlist
                                if self.list_ptr(p) == NULL {
                                    self.cur_v += self.height(p);
                                    // §1920 Record void list SyncTeX information
                                    if self.ty(p) == VLIST_NODE {
                                        self.synctex_void_vlist(p, this_box);
                                    } else {
                                        self.synctex_void_hlist(p, this_box);
                                    }
                                    self.cur_v += self.depth(p);
                                } else {
                                    self.cur_v += self.height(p);
                                    self.synch_v();
                                    save_h = self.dvi_h;
                                    save_v = self.dvi_v;
                                    if self.cur_dir == RIGHT_TO_LEFT {
                                        self.cur_h = left_edge - self.shift_amount(p);
                                    } else {
                                        self.cur_h = left_edge + self.shift_amount(p); // shift the box right
                                    }
                                    self.temp_ptr = p;
                                    if self.ty(p) == VLIST_NODE {
                                        self.vlist_out();
                                    } else {
                                        self.hlist_out();
                                    }
                                    self.dvi_h = save_h;
                                    self.dvi_v = save_v;
                                    self.cur_v = save_v + self.depth(p);
                                    self.cur_h = left_edge;
                                }
                            }
                            RULE_NODE => {
                                self.rule_ht = self.height(p);
                                self.rule_dp = self.depth(p);
                                self.rule_wd = self.width(p);
                                break 'fin_rule;
                            }
                            WHATSIT_NODE => {
                                // §1613 Output the whatsit node |p| in a vlist
                                self.out_what(p);
                            }
                            GLUE_NODE => {
                                // §662 Move down or output leaders
                                self.g = self.glue_ptr(p);
                                let g = self.g;
                                self.rule_ht = self.width(g) - cur_g;
                                if g_sign != NORMAL {
                                    if g_sign == STRETCHING {
                                        if self.stretch_order(g) == g_order {
                                            cur_glue += self.stretch(g) as f64;
                                            let glue_temp =
                                                vet_glue(self.glue_set(this_box) * cur_glue);
                                            cur_g = round(glue_temp);
                                        }
                                    } else if self.shrink_order(g) == g_order {
                                        cur_glue -= self.shrink(g) as f64;
                                        let glue_temp =
                                            vet_glue(self.glue_set(this_box) * cur_glue);
                                        cur_g = round(glue_temp);
                                    }
                                }
                                self.rule_ht += cur_g;
                                if self.subtype(p) >= A_LEADERS {
                                    // §663 Output leaders in a vlist, |goto fin_rule| if a rule or to |next_p| if done
                                    let leader_box = self.leader_ptr(p);
                                    if self.ty(leader_box) == RULE_NODE {
                                        self.rule_wd = self.width(leader_box);
                                        self.rule_dp = 0;
                                        break 'fin_rule;
                                    }
                                    let leader_ht =
                                        self.height(leader_box) + self.depth(leader_box);
                                    if leader_ht > 0 && self.rule_ht > 0 {
                                        self.rule_ht += 10; // compensate for floating-point rounding
                                        let edge = self.cur_v + self.rule_ht;
                                        let mut lx = 0;
                                        // §664 Let |cur_v| be the position of the first box, and set |leader_ht+lx|
                                        if self.subtype(p) == A_LEADERS {
                                            save_v = self.cur_v;
                                            self.cur_v = top_edge
                                                + leader_ht * ((self.cur_v - top_edge) / leader_ht);
                                            if self.cur_v < save_v {
                                                self.cur_v += leader_ht;
                                            }
                                        } else {
                                            self.lq = self.rule_ht / leader_ht; // the number of box copies
                                            self.lr = self.rule_ht % leader_ht; // the remaining space
                                            if self.subtype(p) == C_LEADERS {
                                                self.cur_v += self.lr / 2;
                                            } else {
                                                lx = self.lr / (self.lq + 1);
                                                self.cur_v += (self.lr - (self.lq - 1) * lx) / 2;
                                            }
                                        }
                                        while self.cur_v + leader_ht <= edge {
                                            // §665 Output a leader box at |cur_v|, then advance |cur_v| by |leader_ht+lx|
                                            if self.cur_dir == RIGHT_TO_LEFT {
                                                self.cur_h =
                                                    left_edge - self.shift_amount(leader_box);
                                            } else {
                                                self.cur_h =
                                                    left_edge + self.shift_amount(leader_box);
                                            }
                                            self.synch_h();
                                            save_h = self.dvi_h;
                                            self.cur_v += self.height(leader_box);
                                            self.synch_v();
                                            save_v = self.dvi_v;
                                            self.temp_ptr = leader_box;
                                            let outer_doing_leaders = self.doing_leaders;
                                            self.doing_leaders = true;
                                            if self.ty(leader_box) == VLIST_NODE {
                                                self.vlist_out();
                                            } else {
                                                self.hlist_out();
                                            }
                                            self.doing_leaders = outer_doing_leaders;
                                            self.dvi_v = save_v;
                                            self.dvi_h = save_h;
                                            self.cur_h = left_edge;
                                            self.cur_v =
                                                save_v - self.height(leader_box) + leader_ht + lx;
                                        }
                                        self.cur_v = edge - 10;
                                        break 'next_p;
                                    }
                                }
                                break 'move_past;
                            }
                            KERN_NODE => {
                                self.cur_v += self.width(p);
                            }
                            _ => {}
                        }
                        break 'next_p;
                    }
                    // fin_rule:
                    // §661 Output a rule in a vlist, |goto next_p|
                    if self.is_running(self.rule_wd) {
                        self.rule_wd = self.width(this_box);
                    }
                    self.rule_ht += self.rule_dp; // this is the rule thickness
                    self.cur_v += self.rule_ht;
                    if self.rule_ht > 0 && self.rule_wd > 0 {
                        // we don't output empty rules
                        if self.cur_dir == RIGHT_TO_LEFT {
                            self.cur_h -= self.rule_wd;
                        }
                        self.synch_h();
                        self.synch_v();
                        self.dvi_out(PUT_RULE);
                        self.dvi_four(self.rule_ht);
                        self.dvi_four(self.rule_wd);
                        self.cur_h = left_edge;
                    }
                    break 'next_p;
                }
                // move_past:
                self.cur_v += self.rule_ht;
            }
            // next_p:
            p = self.link(p);
        }
        // §1917 Finish vlist SyncTeX information record
        self.synctex_tsilv(this_box);
        self.prune_movements(save_loc);
        if self.cur_s > 0 {
            self.dvi_pop(save_loc);
        }
        self.cur_s -= 1;
    }

    /// §666
    pub fn dvi_ship_out(&mut self, p: i32) {
        // §1914 Start sheet SyncTeX information record
        self.synctex_sheet(self.mag());
        if self.tracing_output() > 0 {
            self.print_nl_s(b"");
            self.print_ln();
            self.print_s(b"Completed box being shipped out");
        }
        if self.term_offset > self.max_print_line - 9 {
            self.print_ln();
        } else if self.term_offset > 0 || self.file_offset > 0 {
            self.print_char(b' ' as i32);
        }
        self.print_char(b'[' as i32);
        let mut j = 9;
        while self.count(j) == 0 && j > 0 {
            j -= 1;
        }
        for k in 0..=j {
            self.print_int(self.count(k));
            if k < j {
                self.print_char(b'.' as i32);
            }
        }
        self.update_terminal();
        if self.tracing_output() > 0 {
            self.print_char(b']' as i32);
            self.begin_diagnostic();
            self.show_box(p);
            self.end_diagnostic(true);
        }
        // §668 Ship box |p| out
        'done: {
            // §669 Update the values of |max_h| and |max_v|; but if the page is too large, |goto done|
            // In `i64`: the oracle's dimensions are 64 bits, and here a clamped box dimension plus an offset would wrap.
            let total_v = self.height(p) as i64 + self.depth(p) as i64 + self.v_offset() as i64;
            let total_h = self.width(p) as i64 + self.h_offset() as i64;
            if self.height(p) > MAX_DIMEN
                || self.depth(p) > MAX_DIMEN
                || total_v > MAX_DIMEN as i64
                || total_h > MAX_DIMEN as i64
            {
                self.print_err_s(b"Huge page cannot be shipped out");
                self.help(&[
                    b"The page just created is more than 18 feet tall or",
                    b"more than 18 feet wide, so I suspect something went wrong.",
                ]);
                self.error();
                if self.tracing_output() <= 0 {
                    self.begin_diagnostic();
                    self.print_nl_s(b"The following box has been deleted:");
                    self.show_box(p);
                    self.end_diagnostic(true);
                }
                break 'done;
            }
            if total_v > self.max_v as i64 {
                self.max_v = sat32(total_v);
            }
            if total_h > self.max_h as i64 {
                self.max_h = sat32(total_h);
            }
            // §645 Initialize variables as |ship_out| begins
            self.dvi_h = 0;
            self.dvi_v = 0;
            self.cur_h = self.h_offset();
            self.dvi_f = NULL_FONT;
            // §644 Calculate DVI page dimensions and margins
            self.cur_h_offset = self.h_offset();
            self.cur_v_offset = self.v_offset();
            // 4736286 = 1in, the funny DVI origin offset.  `i64`, as the two sums of §669.
            if self.pdf_page_width() != 0 {
                self.cur_page_width = self.pdf_page_width();
            } else {
                self.cur_page_width =
                    sat32(self.width(p) as i64 + 2 * self.cur_h_offset as i64 + 2 * 4736286);
            }
            if self.pdf_page_height() != 0 {
                self.cur_page_height = self.pdf_page_height();
            } else {
                self.cur_page_height = sat32(
                    self.height(p) as i64
                        + self.depth(p) as i64
                        + 2 * self.cur_v_offset as i64
                        + 2 * 4736286,
                );
            }
            self.ensure_dvi_open();
            if self.total_pages == 0 {
                self.dvi_out(PRE);
                self.dvi_out(ID_BYTE); // output the preamble
                self.dvi_four(25400000);
                self.dvi_four(473628672); // conversion ratio for sp
                self.prepare_mag();
                self.dvi_four(self.mag()); // magnification factor is frozen
                if let Some(comment) = self.host.output_comment.take() {
                    self.l = comment.len() as i32;
                    self.dvi_out(self.l);
                    for &b in comment.iter() {
                        self.dvi_out(b as i32);
                    }
                    self.host.output_comment = Some(comment);
                } else {
                    // the default code is unchanged
                    let old_setting = self.selector;
                    self.selector = NEW_STRING;
                    self.print_s(b" TeX output ");
                    self.print_int(self.year());
                    self.print_char(b'.' as i32);
                    self.print_two(self.month());
                    self.print_char(b'.' as i32);
                    self.print_two(self.day());
                    self.print_char(b':' as i32);
                    self.print_two(self.time() / 60);
                    self.print_two(self.time() % 60);
                    self.selector = old_setting;
                    self.dvi_out(self.cur_length());
                    for s in self.str_start[self.str_ptr]..self.pool_ptr {
                        self.dvi_out(self.str_pool.at(s));
                    }
                    self.pool_ptr = self.str_start[self.str_ptr]; // flush the current string
                }
            }
            let page_loc = self.dvi_offset + self.dvi_ptr;
            self.dvi_out(BOP);
            for k in 0..=9 {
                self.dvi_four(self.count(k));
            }
            self.dvi_four(self.last_bop);
            self.last_bop = page_loc;
            self.cur_v = self.height(p) + self.v_offset();
            self.temp_ptr = p;
            if self.ty(p) == VLIST_NODE {
                self.vlist_out();
            } else {
                self.hlist_out();
            }
            self.dvi_out(EOP);
            self.total_pages += 1;
            self.cur_s = -1;
            // dropped: IPC
        }
        // done:
        if self.eTeX_ex() {
            // §1730 Check for LR anomalies at the end of |ship_out|
            if self.LR_problems > 0 {
                // §1713 Report LR problems
                self.print_ln();
                self.print_nl_s(b"\\endL or \\endR problem (");
                self.print_int(self.LR_problems / 10000);
                self.print_s(b" missing, ");
                self.print_int(self.LR_problems % 10000);
                self.print_s(b" extra");
                self.LR_problems = 0;
                self.print_char(b')' as i32);
                self.print_ln();
            }
            if self.LR_ptr != NULL || self.cur_dir != LEFT_TO_RIGHT {
                self.confusion(b"LR3");
            }
        }
        if self.tracing_output() <= 0 {
            self.print_char(b']' as i32);
        }
        self.dead_cycles = 0;
        self.update_terminal(); // progress report
        // §667 Flush the box from memory, showing statistics if requested
        if self.tracing_stats() > 1 {
            self.print_nl_s(b"Memory usage before: ");
            self.print_int(self.var_used);
            self.print_char(b'&' as i32);
            self.print_int(self.dyn_used);
            self.print_char(b';' as i32);
        }
        self.flush_node_list(p);
        if self.tracing_stats() > 1 {
            self.print_s(b" after: ");
            self.print_int(self.var_used);
            self.print_char(b'&' as i32);
            self.print_int(self.dyn_used);
            self.print_s(b"; still untouched: ");
            self.print_int(self.hi_mem_min - self.lo_mem_max - 1);
            self.print_ln();
        }
        // §1915 Finish sheet SyncTeX information record
        self.synctex_teehs();
    }

    // ---- External functions: the C side of web2c and pdfTeX, called by WEB. ----

    /// External: dvi_buf[a..=b] to DVI_FILE, as texmfmp.h's fwrite.  What `put_bytes` does, written on the fields,
    /// since the slice is itself part of `self`.
    pub fn write_dvi(&mut self, a: i32, b: i32) {
        let s = self.dvi_buf.range(a, b + 1);
        match &mut self.host.files[DVI_FILE as usize] {
            Slot::Out(o) => o.bytes(s),
            Slot::Pdf(p) => p.write(s),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::packkit::*;
    use crate::toks::testkit::*;

    /// The bytes written so far and those still in the buffer, in order, as section 670 finishes the file.
    fn dvi_file(t: &mut Tex) -> Vec<u8> {
        if t.dvi_limit == t.half_buf {
            t.write_dvi(t.half_buf, t.dvi_buf_size - 1);
        }
        if t.dvi_ptr > 0 {
            t.write_dvi(0, t.dvi_ptr - 1);
        }
        match &t.host.files[DVI_FILE as usize] {
            Slot::Pdf(p) => p.contents(),
            _ => panic!("no DVI file"),
        }
    }

    /// `dvi_ship_out` of each box of pack_dvi_oracle.tsv, against the oracle's DVI up to `post`, its display, and the
    /// numbers of the postamble.  All of hlist_out, vlist_out, movement and prune_movements run here, and dvi_swap on
    /// a buffer of 800, except for what needs another module: whatsits, and reversing an hlist.
    #[test]
    fn ship_out_matches_the_oracle() {
        let mut bad = Vec::new();
        let mut n = 0;
        for row in rows() {
            if row.params.contains("noship") {
                continue;
            }
            n += 1;
            let t = pack_machine(&row.params);
            t.line = row.line;
            let b = build_box(t, &row.spec);
            printed(t);
            t.dvi_ship_out(b);
            let got = printed(t);
            let compared = !row.params.contains("notrace");
            if compared && got.trim_start_matches('\n') != row.shipped.trim_start_matches('\n') {
                bad.push(format!(
                    "--- {} ({})\nwant {:?}\ngot  {:?}",
                    row.name, row.spec, row.shipped, got
                ));
            }
            let file = dvi_file(t);
            if (file.len(), fnv(&file)) != (row.dvi_len, row.dvi_fnv) {
                bad.push(format!(
                    "--- {} ({})\nDVI of {} bytes, wanted {}: gen_pack_dvi_tables.py {} prints the oracle's\n{:?}",
                    row.name,
                    row.spec,
                    file.len(),
                    row.dvi_len,
                    row.name,
                    &file[..file.len().min(400)]
                ));
            }
            let post = vec![t.max_v, t.max_h, t.max_push];
            if post != row.post {
                bad.push(format!(
                    "--- {} ({})\npostamble: want {:?}\ngot  {:?}",
                    row.name, row.spec, row.post, post
                ));
            }
            // section 642 and section 667: the movement stacks and the page are gone.
            assert_eq!((t.down_ptr, t.right_ptr), (NULL, NULL), "{}", row.name);
            Tex::dealloc(t);
        }
        assert!(n >= 170);
        assert!(bad.is_empty(), "{} differ:\n{}", bad.len(), bad.join("\n"));
    }

    /// section 634-section 641, section 625 and section 642 on the oracle's bytes, for the cases of pack_dvi_oracle.tsv that are one flat list of
    /// kerns and rules: all `hlist_out` or `vlist_out` adds to them is a known rule, written out here.  So the buffer
    /// locations are the oracle's too, and `swap_h` and `swap_v` reach `location(p)<dvi_gone` at a buffer of 800.
    #[test]
    fn movement_matches_the_oracle() {
        let mut bad = Vec::new();
        let mut n = 0;
        for row in rows() {
            if !(row.name.starts_with("move_") || row.name.starts_with("swap_")) {
                continue;
            }
            let words: Vec<&str> = row.spec.split_whitespace().collect();
            if words[1] != "{"
                || words[2..words.len() - 1]
                    .iter()
                    .any(|w| *w == "h" || *w == "v")
            {
                continue; // a nested case: only dvi_ship_out does those
            }
            n += 1;
            let vert = words[0] == "v";
            let t = pack_machine(&row.params);
            // The preamble and `bop`, as the generator's cases have them.
            t.dvi_out(PRE);
            t.dvi_out(ID_BYTE);
            t.dvi_four(25400000);
            t.dvi_four(473628672);
            t.dvi_four(1000);
            let comment = b" TeX output 2026.11.02:1234";
            t.dvi_out(comment.len() as i32);
            for &c in comment {
                t.dvi_out(c as i32);
            }
            t.dvi_out(BOP);
            t.dvi_four(1);
            for _ in 1..10 {
                t.dvi_four(0);
            }
            t.dvi_four(-1);
            let save_loc = t.dvi_offset + t.dvi_ptr;
            let mut w = 0;
            let mut i = 2;
            let mut first = true;
            while i < words.len() - 1 {
                if words[i] == "k" {
                    w += words[i + 1].parse::<i32>().unwrap();
                    i += 2;
                    continue;
                }
                assert_eq!(&words[i..i + 4], ["r", "1", "1", "0"]);
                i += 4;
                // section 661: the thickness moves down first.  section 652: it moves right after.
                if vert {
                    w += 1;
                }
                if w != 0 {
                    t.movement(w, if vert { DOWN1 } else { RIGHT1 });
                }
                w = 0;
                // section 652's synch_v: the page's baseline is at its height, which is 1, and stays there.
                if !vert && first {
                    t.movement(1, DOWN1);
                }
                first = false;
                t.dvi_out(if vert { PUT_RULE } else { SET_RULE });
                t.dvi_four(1);
                t.dvi_four(1);
            }
            t.prune_movements(save_loc);
            t.dvi_out(EOP);
            let file = dvi_file(t);
            if (file.len(), fnv(&file)) != (row.dvi_len, row.dvi_fnv) {
                bad.push(format!("--- {}: DVI of {} bytes, wanted {}\n{:?}", row.name, file.len(), row.dvi_len, &file[..file.len().min(400)]));
            }
            if row.name == "swap_gone_h" {
                // The width 1000 comes again after its command has left the buffer: a second right2, not a w.
                let n = file
                    .windows(3)
                    .filter(|w| w == &[RIGHT1 as u8 + 1, 3, 232])
                    .count();
                assert_eq!(n, 2, "location(p)<dvi_gone was not reached");
            }
            assert_eq!((t.down_ptr, t.right_ptr), (NULL, NULL), "{}", row.name);
            Tex::dealloc(t);
        }
        assert!(n >= 14, "{}", n);
        assert!(bad.is_empty(), "{} differ:\n{}", bad.len(), bad.join("\n"));
    }

    /// section 627: the four bytes, two's complement, at the ends of the range.
    #[test]
    fn dvi_four_is_big_endian_twos_complement() {
        let t = pack_machine("-");
        let mut want = Vec::new();
        for x in [
            0,
            1,
            255,
            256,
            65535,
            65536,
            16777215,
            16777216,
            i32::MAX,
            -1,
            -256,
            -257,
            -16777216,
            -i32::MAX,
            i32::MIN,
        ] {
            t.dvi_four(x);
            want.extend_from_slice(&x.to_be_bytes());
        }
        assert_eq!(&t.dvi_buf.as_slice()[..want.len()], &want[..]);
        Tex::dealloc(t);
    }

    /// section 628: a `pop` right after a `push` takes the push back, except at the start of the half buffer in `dvi_ptr`.
    #[test]
    fn dvi_pop_takes_back_a_push() {
        let t = pack_machine("-");
        t.dvi_out(PUSH);
        let l = t.dvi_offset + t.dvi_ptr;
        t.dvi_pop(l);
        assert_eq!(t.dvi_ptr, 0, "taken back");
        t.dvi_out(PUSH);
        t.dvi_out(EOP);
        t.dvi_pop(1);
        assert_eq!(t.dvi_ptr, 3, "something came between");
        assert_eq!(t.dvi_buf.at(2), POP);
        t.dvi_ptr = 0;
        t.dvi_pop(0);
        assert_eq!(
            (t.dvi_ptr, t.dvi_buf.at(0)),
            (1, POP),
            "dvi_ptr=0: the byte before it may be gone"
        );
        Tex::dealloc(t);
    }

    /// section 625: the halves in turn.
    #[test]
    fn dvi_swap_writes_each_half_in_turn() {
        let t = pack_machine("dvibuf=800");
        for k in 0..2000 {
            t.dvi_out(k % 251);
        }
        // Swaps at 800, 1200, 1600 and 2000 bytes: four halves gone, and the lower half still in the buffer.
        assert_eq!(t.dvi_gone, 1600);
        assert_eq!(t.dvi_offset, 1600);
        assert_eq!(t.dvi_ptr, 400);
        assert_eq!(t.dvi_limit, 800);
        let file = dvi_file(t);
        let want: Vec<u8> = (0..2000).map(|k| (k % 251) as u8).collect();
        assert_eq!(file, want);
        Tex::dealloc(t);
    }

    /// section 629, section 630, section 649: a font number past 64 and past 256.  Too many fonts for a small table of the oracle's.
    #[test]
    fn font_numbers_take_the_long_forms() {
        let t = pack_machine("-");
        for (f, cmd) in [(300, vec![FNT_DEF1 + 1, 1, 43]), (100, vec![FNT_DEF1, 99])] {
            t.font_area[f] = S_EMPTY;
            t.font_name[f] = S_EMPTY;
            t.font_check[f] = FourQuarters {
                b0: 1,
                b1: 2,
                b2: 3,
                b3: 4,
            };
            t.font_size[f] = 5;
            t.font_dsize[f] = 6;
            t.dvi_ptr = 0;
            t.dvi_font_def(f);
            let mut want = cmd;
            want.extend_from_slice(&[1, 2, 3, 4, 0, 0, 0, 5, 0, 0, 0, 6, 0, 0]);
            let got: Vec<i32> = (0..t.dvi_ptr).map(|k| t.dvi_buf.at(k)).collect();
            assert_eq!(got, want);
        }
        Tex::dealloc(t);
    }
}
