//! §1068-1143 Parts 40-43: hyphenation.
//!
//! `hyphenate`'s §1100, `new_hyph_exceptions`' §1112 and §1116 and `new_patterns`' §1138, §1140 and §1855 are
//! helpers, each used once: a whole module with no goto out but a result the caller branches on.  As helpers they
//! can be run by the unit tests, on real tokens, without `scan_left_brace` around them.
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
    /// §1072
    pub fn hyphenate(&mut self) {
        // §1078 Local variables for hyphenation, and §1089, §1099, §1106
        let (mut i, mut j, mut l): (i32, i32, i32);
        let (mut q, mut r, mut s): (i32, i32, i32);
        let mut bchar: i32;
        let (mut major_tail, mut minor_tail): (i32, i32);
        let mut c = 0; // set before use: it is read only if hyf_node or c_loc says so
        let mut c_loc: i32;
        let mut r_count: i32;
        let mut hyf_node: i32;
        // §1100 Find hyphen locations for the word in hc, or return
        if !self.hyph_find_hyphens() {
            return;
        }
        // §1079 If no hyphens were found, return
        'found1: {
            j = self.l_hyf;
            let e = self.hn - self.r_hyf;
            while j <= e {
                if odd(self.hyf[j]) {
                    break 'found1;
                }
                j += 1;
            }
            return;
        }
        // §1080 Replace nodes ha..hb by a sequence of nodes that includes the discretionary hyphens
        q = self.link(self.hb);
        self.set_link(self.hb, NULL);
        r = self.link(self.ha);
        self.set_link(self.ha, NULL);
        bchar = self.hyf_bchar;
        'common_ending: {
            'found2: {
                if self.is_char_node(self.ha) {
                    if self.font(self.ha) != self.hf {
                        break 'found2;
                    } else {
                        self.init_list = self.ha;
                        self.init_lig = false;
                        self.hu[0] = self.character(self.ha);
                    }
                } else if self.ty(self.ha) == LIGATURE_NODE {
                    if self.font(self.lig_char(self.ha)) != self.hf {
                        break 'found2;
                    } else {
                        self.init_list = self.lig_ptr(self.ha);
                        self.init_lig = true;
                        self.init_lft = self.subtype(self.ha) > 1;
                        self.hu[0] = self.character(self.lig_char(self.ha));
                        if self.init_list == NULL {
                            if self.init_lft {
                                self.hu[0] = 256;
                                self.init_lig = false;
                            } // in this case a ligature will be reconstructed from scratch
                        }
                        self.free_node(self.ha, SMALL_NODE_SIZE);
                    }
                } else {
                    // no punctuation found; look for left boundary
                    if !self.is_char_node(r) {
                        if self.ty(r) == LIGATURE_NODE {
                            if self.subtype(r) > 1 {
                                break 'found2;
                            }
                        }
                    }
                    j = 1;
                    s = self.ha;
                    self.init_list = NULL;
                    break 'common_ending;
                }
                s = self.cur_p; // we have cur_p<>ha because type(cur_p)=glue_node
                while self.link(s) != self.ha {
                    s = self.link(s);
                }
                j = 0;
                break 'common_ending;
            }
            // found2:
            s = self.ha;
            j = 0;
            self.hu[0] = 256;
            self.init_lig = false;
            self.init_list = NULL;
        }
        // common_ending:
        self.flush_node_list(r);
        // §1090 Reconstitute nodes for the hyphenated word, inserting discretionary hyphens
        loop {
            l = j;
            j = self.reconstitute(j, self.hn, bchar, self.hyf_char) + 1;
            if self.hyphen_passed == 0 {
                self.set_link(s, self.link(HOLD_HEAD));
                while self.link(s) > NULL {
                    s = self.link(s);
                }
                if odd(self.hyf[j - 1]) {
                    l = j;
                    self.hyphen_passed = j - 1;
                    self.set_link(HOLD_HEAD, NULL);
                }
            }
            if self.hyphen_passed > 0 {
                // §1091 Create and append a discretionary node as an alternative to the unhyphenated
                // word, and continue to develop both branches until they become equivalent
                loop {
                    r = self.get_node(SMALL_NODE_SIZE);
                    self.set_link(r, self.link(HOLD_HEAD));
                    self.set_ty(r, DISC_NODE);
                    major_tail = r;
                    r_count = 0;
                    while self.link(major_tail) > NULL {
                        major_tail = self.link(major_tail); // advance_major_tail
                        r_count += 1;
                    }
                    i = self.hyphen_passed;
                    self.hyf[i] = 0;
                    // §1092 Put the characters hu[l..i] and a hyphen into pre_break(r)
                    minor_tail = NULL;
                    self.set_pre_break(r, NULL);
                    hyf_node = self.new_character(self.hf, self.hyf_char);
                    if hyf_node != NULL {
                        i += 1;
                        c = self.hu[i];
                        self.hu[i] = self.hyf_char;
                        self.free_avail(hyf_node);
                    }
                    while l <= i {
                        l = self.reconstitute(l, i, self.font_bchar[self.hf], NON_CHAR) + 1;
                        if self.link(HOLD_HEAD) > NULL {
                            if minor_tail == NULL {
                                self.set_pre_break(r, self.link(HOLD_HEAD));
                            } else {
                                self.set_link(minor_tail, self.link(HOLD_HEAD));
                            }
                            minor_tail = self.link(HOLD_HEAD);
                            while self.link(minor_tail) > NULL {
                                minor_tail = self.link(minor_tail);
                            }
                        }
                    }
                    if hyf_node != NULL {
                        self.hu[i] = c; // restore the character in the hyphen position
                        l = i; // WEB also has decr(i), which nothing reads
                    }
                    // §1093 Put the characters hu[i+1..] into post_break(r), appending to this list and
                    // to major_tail until synchronization has been achieved
                    minor_tail = NULL;
                    self.set_post_break(r, NULL);
                    c_loc = 0;
                    if self.bchar_label[self.hf] != NON_ADDRESS {
                        // put left boundary at beginning of new line
                        l -= 1;
                        c = self.hu[l];
                        c_loc = l;
                        self.hu[l] = 256;
                    }
                    while l < j {
                        loop {
                            l = self.reconstitute(l, self.hn, bchar, NON_CHAR) + 1;
                            if c_loc > 0 {
                                self.hu[c_loc] = c;
                                c_loc = 0;
                            }
                            if self.link(HOLD_HEAD) > NULL {
                                if minor_tail == NULL {
                                    self.set_post_break(r, self.link(HOLD_HEAD));
                                } else {
                                    self.set_link(minor_tail, self.link(HOLD_HEAD));
                                }
                                minor_tail = self.link(HOLD_HEAD);
                                while self.link(minor_tail) > NULL {
                                    minor_tail = self.link(minor_tail);
                                }
                            }
                            if l >= j {
                                break;
                            }
                        }
                        while l > j {
                            // §1094 Append characters of hu[j..] to major_tail, advancing j
                            j = self.reconstitute(j, self.hn, bchar, NON_CHAR) + 1;
                            self.set_link(major_tail, self.link(HOLD_HEAD));
                            while self.link(major_tail) > NULL {
                                major_tail = self.link(major_tail); // advance_major_tail
                                r_count += 1;
                            }
                        }
                    }
                    // §1095 Move pointer s to the end of the current list, and set replace_count(r)
                    // appropriately
                    if r_count > 127 {
                        // we have to forget the discretionary hyphen
                        self.set_link(s, self.link(r));
                        self.set_link(r, NULL);
                        self.flush_node_list(r);
                    } else {
                        self.set_link(s, r);
                        self.set_replace_count(r, r_count);
                    }
                    s = major_tail;
                    self.hyphen_passed = j - 1;
                    self.set_link(HOLD_HEAD, NULL);
                    if !odd(self.hyf[j - 1]) {
                        break;
                    }
                }
            }
            if j > self.hn {
                break;
            }
        }
        self.set_link(s, q);
        // The characters of a ligature, on mem: docs/PORTING.md section 16.
        self.mem_flush_list(self.init_list);
    }

    /// §1100 Find hyphen locations for the word in hc.  False means WEB's `return`.
    fn hyph_find_hyphens(&mut self) -> bool {
        let mut j: i32;
        let mut h: i32;
        for j in 0..=self.hn {
            self.hyf[j] = 0;
        }
        // §1107 Look for the word hc[1..hn] in the exception table, and goto found (with
        // hyf containing the hyphens) if an entry is found
        'found: {
            h = self.hc[1];
            self.hn += 1;
            self.hc[self.hn] = self.cur_lang;
            for j in 2..=self.hn {
                h = (h + h + self.hc[j]) % HYPH_PRIME;
            }
            loop {
                // §1108 If the string hyph_word[h] is less than hc[1..hn], goto not_found;
                // but if the two strings are equal, set hyf to the hyphen positions and goto found
                let k = self.hyph_word[h];
                if k == 0 {
                    break; // goto not_found
                }
                'done: {
                    if self.length(k) == self.hn {
                        j = 1;
                        let mut u = self.str_start[k];
                        loop {
                            if self.str_pool.at(u) != self.hc[j] {
                                break 'done;
                            }
                            j += 1;
                            u += 1;
                            if j > self.hn {
                                break;
                            }
                        }
                        // §1109 Insert hyphens as specified in hyph_list[h]
                        let mut s = self.hyph_list[h];
                        while s != NULL {
                            let k = self.info(s);
                            self.hyf[k] = 1;
                            s = self.link(s);
                        }
                        self.hn -= 1;
                        break 'found;
                    }
                }
                h = self.hyph_link[h];
                if h == 0 {
                    break; // goto not_found
                }
                h -= 1;
            }
            // not_found:
            self.hn -= 1;
            if self.trie_char(self.cur_lang + 1) != self.cur_lang {
                return false; // no patterns for cur_lang
            }
            self.hc[0] = 0;
            self.hc[self.hn + 1] = 0;
            self.hc[self.hn + 2] = 256; // insert delimiters
            for j in 0..=self.hn - self.r_hyf + 1 {
                let mut z = self.trie_link(self.cur_lang + 1) + self.hc[j];
                let mut l = j;
                while self.hc[l] == self.trie_char(z) {
                    if self.trie_op(z) != MIN_TRIE_OP {
                        // §1101 Store maximum values in the hyf table
                        let mut v = self.trie_op(z);
                        loop {
                            v += self.op_start[self.cur_lang];
                            let i = l - self.hyf_distance[v];
                            if self.hyf_num[v] > self.hyf[i] {
                                self.hyf[i] = self.hyf_num[v];
                            }
                            v = self.hyf_next[v];
                            if v == MIN_TRIE_OP {
                                break;
                            }
                        }
                    }
                    l += 1;
                    z = self.trie_link(z) + self.hc[l];
                }
            }
        }
        // found:
        for j in 0..=self.l_hyf - 1 {
            self.hyf[j] = 0;
        }
        for j in 0..=self.r_hyf - 1 {
            self.hyf[self.hn - j] = 0;
        }
        true
    }

    /// §1085 `append_charnode_to_t(c)`; returns the new `t`.
    fn append_charnode_to_t(&mut self, t: i32, c: i32) -> i32 {
        let a = self.get_avail();
        self.set_link(t, a);
        self.set_font(a, self.hf);
        self.set_character(a, c);
        a
    }

    /// §1085 `set_cur_r`; returns the new `cur_rh`.
    fn set_cur_r(&mut self, j: i32, n: i32, bchar: i32, hchar: i32) -> i32 {
        if j < n {
            self.cur_r = self.hu[j + 1];
        } else {
            self.cur_r = bchar;
        }
        if odd(self.hyf[j]) { hchar } else { NON_CHAR }
    }

    /// §1087 `wrap_lig(b)`; `t` becomes the ligature if there is one.
    fn wrap_lig(&mut self, b: bool, t: &mut i32) {
        if self.ligature_present {
            let p = self.new_ligature(self.hf, self.cur_l, self.link(self.cur_q));
            if self.lft_hit {
                self.set_subtype(p, 2);
                self.lft_hit = false;
            }
            if b {
                if self.lig_stack == NULL {
                    self.set_subtype(p, self.subtype(p) + 1);
                    self.rt_hit = false;
                }
            }
            self.set_link(self.cur_q, p);
            *t = p;
            self.ligature_present = false;
        }
    }

    /// §1087 `pop_lig_stack`.
    fn pop_lig_stack(
        &mut self,
        t: &mut i32,
        j: &mut i32,
        n: i32,
        bchar: i32,
        hchar: i32,
        cur_rh: &mut i32,
    ) {
        if self.lig_ptr(self.lig_stack) > NULL {
            self.set_link(*t, self.lig_ptr(self.lig_stack)); // this is a charnode for hu[j+1]
            *t = self.link(*t);
            *j += 1;
        }
        let p = self.lig_stack;
        self.lig_stack = self.link(p);
        self.free_node(p, SMALL_NODE_SIZE);
        if self.lig_stack == NULL {
            *cur_rh = self.set_cur_r(*j, n, bchar, hchar);
        } else {
            self.cur_r = self.character(self.lig_stack);
        } // if lig_stack isn't null we have cur_rh=non_char
    }

    /// §1083
    pub fn reconstitute(&mut self, j: i32, n: i32, bchar: i32, hchar: i32) -> i32 {
        let mut j = j;
        let mut bchar = bchar;
        let mut hchar = hchar;
        let mut p: i32;
        let mut q: FourQuarters;
        let mut k: i32;
        self.hyphen_passed = 0;
        let mut t = HOLD_HEAD; // a node being appended to
        let mut w = 0; // amount of kerning
        self.set_link(HOLD_HEAD, NULL);
        // at this point ligature_present=lft_hit=rt_hit=false
        // §1085 Set up data structures with the cursor following position j
        self.cur_l = self.hu[j];
        self.cur_q = t;
        if j == 0 {
            self.ligature_present = self.init_lig;
            p = self.init_list;
            if self.ligature_present {
                self.lft_hit = self.init_lft;
            }
            while p > NULL {
                t = self.append_charnode_to_t(t, self.character(p));
                p = self.link(p);
            }
        } else if self.cur_l < NON_CHAR {
            t = self.append_charnode_to_t(t, self.cur_l);
        }
        self.lig_stack = NULL;
        let mut cur_rh = self.set_cur_r(j, n, bchar, hchar);
        'continue_: loop {
            // §1086 If there's a ligature or kern at the cursor position, update the data structures,
            // possibly advancing j; continue until the cursor moves
            'done: {
                if self.cur_l == NON_CHAR {
                    k = self.bchar_label[self.hf];
                    if k == NON_ADDRESS {
                        break 'done;
                    } else {
                        q = self.font_info[k].qqqq();
                    }
                } else {
                    q = self.char_info(self.hf, self.cur_l);
                    if self.char_tag(q) != LIG_TAG {
                        break 'done;
                    }
                    k = self.lig_kern_start(self.hf, q);
                    q = self.font_info[k].qqqq();
                    if self.skip_byte(q) > STOP_FLAG {
                        k = self.lig_kern_restart(self.hf, q);
                        q = self.font_info[k].qqqq();
                    }
                } // now k is the starting address of the lig/kern program
                let test_char = if cur_rh < NON_CHAR {
                    cur_rh
                } else {
                    self.cur_r
                };
                loop {
                    if self.next_char(q) == test_char {
                        if self.skip_byte(q) <= STOP_FLAG {
                            if cur_rh < NON_CHAR {
                                self.hyphen_passed = j;
                                hchar = NON_CHAR;
                                cur_rh = NON_CHAR;
                                continue 'continue_;
                            } else {
                                if hchar < NON_CHAR {
                                    if odd(self.hyf[j]) {
                                        self.hyphen_passed = j;
                                        hchar = NON_CHAR;
                                    }
                                }
                                if self.op_byte(q) < KERN_FLAG {
                                    // §1088 Carry out a ligature replacement, updating the cursor
                                    // structure and possibly advancing j; goto continue if the cursor
                                    // doesn't advance, otherwise goto done
                                    if self.cur_l == NON_CHAR {
                                        self.lft_hit = true;
                                    }
                                    if j == n {
                                        if self.lig_stack == NULL {
                                            self.rt_hit = true;
                                        }
                                    }
                                    self.check_interrupt(); // allow a way out in case there's an infinite ligature loop
                                    match self.op_byte(q) {
                                        1 | 5 => {
                                            self.cur_l = self.rem_byte(q); // =:|, =:|>
                                            self.ligature_present = true;
                                        }
                                        2 | 6 => {
                                            self.cur_r = self.rem_byte(q); // |=:, |=:>
                                            if self.lig_stack > NULL {
                                                self.set_character(self.lig_stack, self.cur_r);
                                            } else {
                                                self.lig_stack = self.new_lig_item(self.cur_r);
                                                if j == n {
                                                    bchar = NON_CHAR;
                                                } else {
                                                    p = self.get_avail();
                                                    self.set_lig_ptr(self.lig_stack, p);
                                                    self.set_character(p, self.hu[j + 1]);
                                                    self.set_font(p, self.hf);
                                                }
                                            }
                                        }
                                        3 => {
                                            self.cur_r = self.rem_byte(q); // |=:|
                                            p = self.lig_stack;
                                            self.lig_stack = self.new_lig_item(self.cur_r);
                                            self.set_link(self.lig_stack, p);
                                        }
                                        7 | 11 => {
                                            self.wrap_lig(false, &mut t); // |=:|>, |=:|>>
                                            self.cur_q = t;
                                            self.cur_l = self.rem_byte(q);
                                            self.ligature_present = true;
                                        }
                                        _ => {
                                            self.cur_l = self.rem_byte(q);
                                            self.ligature_present = true; // =:
                                            if self.lig_stack > NULL {
                                                self.pop_lig_stack(
                                                    &mut t,
                                                    &mut j,
                                                    n,
                                                    bchar,
                                                    hchar,
                                                    &mut cur_rh,
                                                );
                                            } else if j == n {
                                                break 'done;
                                            } else {
                                                t = self.append_charnode_to_t(t, self.cur_r);
                                                j += 1;
                                                cur_rh = self.set_cur_r(j, n, bchar, hchar);
                                            }
                                        }
                                    }
                                    if self.op_byte(q) > 4 {
                                        if self.op_byte(q) != 7 {
                                            break 'done;
                                        }
                                    }
                                    continue 'continue_;
                                }
                                w = self.char_kern(self.hf, q);
                                break 'done; // this kern will be inserted below
                            }
                        }
                    }
                    if self.skip_byte(q) >= STOP_FLAG {
                        if cur_rh == NON_CHAR {
                            break 'done;
                        } else {
                            cur_rh = NON_CHAR;
                            continue 'continue_;
                        }
                    }
                    k = k + self.skip_byte(q) + 1;
                    q = self.font_info[k].qqqq();
                }
            }
            // done:
            // §1087 Append a ligature and/or kern to the translation; goto continue if the stack of
            // inserted ligatures is nonempty
            self.wrap_lig(self.rt_hit, &mut t);
            if w != 0 {
                let a = self.new_kern(w);
                self.set_link(t, a);
                t = a;
                w = 0;
                self.set_sync_tag(t + MEDIUM_NODE_SIZE, 0); // SyncTeX: do nothing, it is too late
            }
            if self.lig_stack > NULL {
                self.cur_q = t;
                self.cur_l = self.character(self.lig_stack);
                self.ligature_present = true;
                self.pop_lig_stack(&mut t, &mut j, n, bchar, hchar, &mut cur_rh);
                continue 'continue_;
            }
            return j;
        }
    }

    /// §1111
    pub fn new_hyph_exceptions(&mut self) {
        self.scan_left_brace(); // a left brace must follow \hyphenation
        self.set_cur_lang();
        if self.trie_not_ready {
            self.hyph_index = 0; // goto not_found1
        } else {
            self.set_hyph_index();
        }
        // not_found1:
        self.hyph_enter_exceptions();
    }

    /// §1112 Enter as many hyphenation exceptions as are listed, until coming to a right brace; then return
    fn hyph_enter_exceptions(&mut self) {
        let mut n = 0; // length of current word; not always a small_number
        let mut p = NULL; // head of a list of hyphen positions
        loop {
            self.get_x_token();
            'reswitch: loop {
                match self.cur_cmd {
                    LETTER | OTHER_CHAR | CHAR_GIVEN => {
                        // §1114 Append a new letter or hyphen
                        if self.cur_chr == b'-' as i32 {
                            // §1115 Append the value n to list p
                            if n < 63 {
                                let q = self.get_avail();
                                self.set_link(q, p);
                                self.set_info(q, n);
                                p = q;
                            }
                        } else {
                            self.set_hc0_lc_code(self.cur_chr);
                            if self.hc[0] == 0 {
                                self.print_err_s(b"Not a letter");
                                self.help(&[
                                    b"Letters in \\hyphenation words must have \\lccode>0.",
                                    b"Proceed; I'll ignore the character I just read.",
                                ]);
                                self.error();
                            } else if n < 63 {
                                n += 1;
                                self.hc[n] = self.hc[0];
                            }
                        }
                    }
                    CHAR_NUM => {
                        self.scan_char_num();
                        self.cur_chr = self.cur_val;
                        self.cur_cmd = CHAR_GIVEN;
                        continue 'reswitch;
                    }
                    SPACER | RIGHT_BRACE => {
                        if n > 1 {
                            self.hyph_enter_exception(n, p);
                        }
                        if self.cur_cmd == RIGHT_BRACE {
                            return;
                        }
                        n = 0;
                        p = NULL;
                    }
                    _ => {
                        // §1113 Give improper \hyphenation error
                        self.print_err_s(b"Improper ");
                        self.print_esc_s(b"hyphenation");
                        self.print_s(b" will be flushed");
                        self.help(&[
                            b"Hyphenation exceptions must contain only letters",
                            b"and hyphens. But continue; I'll forgive and forget.",
                        ]);
                        self.error();
                    }
                }
                break;
            }
        }
    }

    /// §1116 Enter a hyphenation exception: the word `hc[1..n]`, with hyphen positions `p`.
    fn hyph_enter_exception(&mut self, n: i32, p: i32) {
        let mut n = n + 1;
        self.hc[n] = self.cur_lang;
        self.str_room(n);
        let mut h = 0;
        for j in 1..=n {
            h = (h + h + self.hc[j]) % HYPH_PRIME;
            self.append_char(self.hc[j]);
        }
        let mut s = self.make_string();
        // §1117 Insert the pair (s,p) into the exception table
        if self.hyph_next <= HYPH_PRIME {
            while self.hyph_next > 0 && self.hyph_word[self.hyph_next - 1] > 0 {
                self.hyph_next -= 1;
            }
        }
        if self.hyph_count == self.hyph_size || self.hyph_next == 0 {
            self.overflow(b"exception dictionary", self.hyph_size);
        }
        self.hyph_count += 1;
        'found: {
            while self.hyph_word[h] != 0 {
                // §1118 If the string hyph_word[h] is less than or equal to s, interchange
                // (hyph_word[h],hyph_list[h]) with (s,p)
                // This is now a simple hash list, not an ordered one, so the module title is no longer
                // descriptive.
                'not_found: {
                    let k = self.hyph_word[h];
                    if self.length(k) != self.length(s) {
                        break 'not_found;
                    }
                    let mut u = self.str_start[k];
                    let mut v = self.str_start[s];
                    loop {
                        if self.str_pool.at(u) != self.str_pool.at(v) {
                            break 'not_found;
                        }
                        u += 1;
                        v += 1;
                        if u == self.str_start[k + 1] {
                            break;
                        }
                    }
                    // repeat hyphenation exception; flushing old data
                    self.flush_string();
                    s = self.hyph_word[h]; // avoid slow_make_string!
                    self.hyph_count -= 1;
                    // We could also flush_list(hyph_list[h]), but it interferes with trip.log.
                    break 'found;
                }
                // not_found:
                if self.hyph_link[h] == 0 {
                    self.hyph_link[h] = self.hyph_next;
                    if self.hyph_next >= self.hyph_size {
                        self.hyph_next = HYPH_PRIME;
                    }
                    if self.hyph_next > HYPH_PRIME {
                        self.hyph_next += 1;
                    }
                }
                h = self.hyph_link[h] - 1;
            }
        }
        // found:
        self.hyph_word[h] = s;
        self.hyph_list[h] = p;
    }

    /// §1121
    pub fn new_trie_op(&mut self, d: i32, n: i32, v: i32) -> i32 {
        let mut h = abs(n + 313 * d + 361 * v + 1009 * self.cur_lang)
            % (TRIE_OP_SIZE - NEG_TRIE_OP_SIZE)
            + NEG_TRIE_OP_SIZE;
        loop {
            let l = self.trie_op_hash[h];
            if l == 0 {
                // empty position found for a new op
                if self.trie_op_ptr == TRIE_OP_SIZE {
                    self.overflow(b"pattern memory ops", TRIE_OP_SIZE);
                }
                let mut u = self.trie_used[self.cur_lang];
                if u == MAX_TRIE_OP {
                    self.overflow(
                        b"pattern memory ops per language",
                        MAX_TRIE_OP - MIN_TRIE_OP,
                    );
                }
                self.trie_op_ptr += 1;
                u += 1;
                self.trie_used[self.cur_lang] = u;
                if u > self.max_op_used {
                    self.max_op_used = u;
                }
                self.hyf_distance[self.trie_op_ptr] = d;
                self.hyf_num[self.trie_op_ptr] = n;
                self.hyf_next[self.trie_op_ptr] = v;
                self.trie_op_lang[self.trie_op_ptr] = self.cur_lang;
                self.trie_op_hash[h] = self.trie_op_ptr;
                self.trie_op_val[self.trie_op_ptr] = u;
                return u;
            }
            if self.hyf_distance[l] == d
                && self.hyf_num[l] == n
                && self.hyf_next[l] == v
                && self.trie_op_lang[l] == self.cur_lang
            {
                return self.trie_op_val[l];
            }
            if h > -TRIE_OP_SIZE {
                h -= 1;
            } else {
                h = TRIE_OP_SIZE;
            }
        }
    }

    /// §1125
    pub fn trie_node(&mut self, p: i32) -> i32 {
        // In i64: 3142*trie_r[p] passes 2^31 at the default trie_size, and abs of a wrapped sum could
        // be negative.  The hash decides only where a node is found, so its value is not observable.
        let key = self.trie_c.at(p) as i64
            + 1009 * self.trie_o[p] as i64
            + 2718 * self.trie_l[p] as i64
            + 3142 * self.trie_r[p] as i64;
        let mut h = (key.abs() % self.trie_size as i64) as i32;
        loop {
            let q = self.trie_hash[h];
            if q == 0 {
                self.trie_hash[h] = p;
                return p;
            }
            if self.trie_c.at(q) == self.trie_c.at(p)
                && self.trie_o[q] == self.trie_o[p]
                && self.trie_l[q] == self.trie_l[p]
                && self.trie_r[q] == self.trie_r[p]
            {
                return q;
            }
            if h > 0 {
                h -= 1;
            } else {
                h = self.trie_size;
            }
        }
    }

    /// §1126
    pub fn compress_trie(&mut self, p: i32) -> i32 {
        if p == 0 {
            0
        } else {
            let l = self.compress_trie(self.trie_l[p]);
            self.trie_l[p] = l;
            let r = self.compress_trie(self.trie_r[p]);
            self.trie_r[p] = r;
            self.trie_node(p)
        }
    }

    /// §1130
    pub fn first_fit(&mut self, p: i32) {
        let c = self.trie_c.at(p); // smallest character in the family
        let mut z = self.trie_min[c]; // get the first conceivably good hole
        let h = 'found: loop {
            let h = z - c; // candidate for trie_ref[p]
            // §1131 Ensure that trie_max>=h+256
            if self.trie_max < h + 256 {
                if self.trie_size <= h + 256 {
                    self.overflow(b"pattern memory", self.trie_size);
                }
                loop {
                    self.trie_max += 1;
                    self.trie_taken[self.trie_max] = false;
                    self.set_trie_link(self.trie_max, self.trie_max + 1);
                    self.set_trie_back(self.trie_max, self.trie_max - 1);
                    if self.trie_max == h + 256 {
                        break;
                    }
                }
            }
            'not_found: {
                if self.trie_taken[h] {
                    break 'not_found;
                }
                // §1132 If all characters of the family fit relative to h, then goto found,
                // otherwise goto not_found
                let mut q = self.trie_r[p];
                while q > 0 {
                    if self.trie_link(h + self.trie_c.at(q)) == 0 {
                        break 'not_found;
                    }
                    q = self.trie_r[q];
                }
                break 'found h;
            }
            // not_found:
            z = self.trie_link(z); // move to the next hole
        };
        // found:
        // §1133 Pack the family into trie relative to h
        self.trie_taken[h] = true;
        self.trie_hash[p] = h; // trie_ref[p]:=h
        let mut q = p;
        loop {
            let z = h + self.trie_c.at(q);
            let mut l = self.trie_back(z);
            let r = self.trie_link(z);
            self.set_trie_back(r, l);
            self.set_trie_link(l, r);
            self.set_trie_link(z, 0);
            if l < 256 {
                let ll = if z < 256 { z } else { 256 };
                loop {
                    self.trie_min[l] = r;
                    l += 1;
                    if l == ll {
                        break;
                    }
                }
            }
            q = self.trie_r[q];
            if q == 0 {
                break;
            }
        }
    }

    /// §1134
    pub fn trie_pack(&mut self, p: i32) {
        let mut p = p;
        loop {
            let q = self.trie_l[p];
            if q > 0 && self.trie_hash[q] == 0 {
                self.first_fit(q);
                self.trie_pack(q);
            }
            p = self.trie_r[p];
            if p == 0 {
                break;
            }
        }
    }

    /// §1136
    pub fn trie_fix(&mut self, p: i32) {
        let mut p = p;
        let z = self.trie_hash[p]; // trie reference; this local variable must be saved
        loop {
            let q = self.trie_l[p];
            let c = self.trie_c.at(p);
            self.set_trie_link(z + c, self.trie_hash[q]);
            self.set_trie_char(z + c, c);
            self.set_trie_op(z + c, self.trie_o[p]);
            if q > 0 {
                self.trie_fix(q);
            }
            p = self.trie_r[p];
            if p == 0 {
                break;
            }
        }
    }

    /// §1137
    pub fn new_patterns(&mut self) {
        if self.trie_not_ready {
            self.set_cur_lang();
            self.scan_left_brace(); // a left brace must follow \patterns
            self.hyph_enter_patterns();
            // done:
            if self.saving_hyph_codes() > 0 {
                self.hyph_store_codes();
            }
        } else {
            self.print_err_s(b"Too late for ");
            self.print_esc_s(b"patterns");
            self.help(&[b"All patterns must be given before typesetting begins."]);
            self.error();
            // link(garbage):=scan_toks(false,false); flush_list(def_ref)
            let d = self.scan_toks(false, false);
            self.flush_list(d);
        }
    }

    /// §1138 Enter all of the patterns into a linked trie, until coming to a right brace
    fn hyph_enter_patterns(&mut self) {
        let mut k = 0;
        self.hyf[0] = 0;
        let mut digit_sensed = false;
        loop {
            self.get_x_token();
            match self.cur_cmd {
                LETTER | OTHER_CHAR => {
                    // §1139 Append a new letter or a hyphen level
                    if digit_sensed || self.cur_chr < b'0' as i32 || self.cur_chr > b'9' as i32 {
                        if self.cur_chr == b'.' as i32 {
                            self.cur_chr = 0; // edge-of-word delimiter
                        } else {
                            self.cur_chr = self.lc_code(self.cur_chr);
                            if self.cur_chr == 0 {
                                self.print_err_s(b"Nonletter");
                                self.help(&[b"(See Appendix H.)"]);
                                self.error();
                            }
                        }
                        if k < 63 {
                            k += 1;
                            self.hc[k] = self.cur_chr;
                            self.hyf[k] = 0;
                            digit_sensed = false;
                        }
                    } else if k < 63 {
                        self.hyf[k] = self.cur_chr - b'0' as i32;
                        digit_sensed = true;
                    }
                }
                SPACER | RIGHT_BRACE => {
                    if k > 0 {
                        self.hyph_insert_pattern(k);
                    }
                    if self.cur_cmd == RIGHT_BRACE {
                        break; // goto done
                    }
                    k = 0;
                    self.hyf[0] = 0;
                    digit_sensed = false;
                }
                _ => {
                    self.print_err_s(b"Bad ");
                    self.print_esc_s(b"patterns");
                    self.help(&[b"(See Appendix H.)"]);
                    self.error();
                }
            }
        }
    }

    /// §1140 Insert a new pattern into the linked trie: `hc[1..k]`, with levels `hyf[0..k]`.
    fn hyph_insert_pattern(&mut self, k: i32) {
        // §1142 Compute the trie op code, v, and set l:=0
        if self.hc[1] == 0 {
            self.hyf[0] = 0;
        }
        if self.hc[k] == 0 {
            self.hyf[k] = 0;
        }
        let mut l = k;
        let mut v = MIN_TRIE_OP;
        loop {
            if self.hyf[l] != 0 {
                v = self.new_trie_op(k - l, self.hyf[l], v);
            }
            if l > 0 {
                l -= 1;
            } else {
                break; // goto done1
            }
        }
        // done1:
        let mut q = 0;
        self.hc[0] = self.cur_lang;
        while l <= k {
            let c = self.hc[l];
            l += 1;
            let mut p = self.trie_l[q];
            let mut first_child = true;
            while p > 0 && c > self.trie_c.at(p) {
                q = p;
                p = self.trie_r[q];
                first_child = false;
            }
            if p == 0 || c < self.trie_c.at(p) {
                p = self.hyph_insert_trie_node(q, p, first_child, c);
            }
            q = p; // now node q represents p_1...p_{l-1}
        }
        if self.trie_o[q] != MIN_TRIE_OP {
            self.print_err_s(b"Duplicate pattern");
            self.help(&[b"(See Appendix H.)"]);
            self.error();
        }
        self.trie_o[q] = v;
    }

    /// §1141 Insert a new trie node between q and p, and make p point to it; returns p.
    fn hyph_insert_trie_node(&mut self, q: i32, p: i32, first_child: bool, c: i32) -> i32 {
        if self.trie_ptr == self.trie_size {
            self.overflow(b"pattern memory", self.trie_size);
        }
        self.trie_ptr += 1;
        self.trie_r[self.trie_ptr] = p;
        let p = self.trie_ptr;
        self.trie_l[p] = 0;
        if first_child {
            self.trie_l[q] = p;
        } else {
            self.trie_r[q] = p;
        }
        self.trie_c.put(p, c);
        self.trie_o[p] = MIN_TRIE_OP;
        p
    }

    /// §1855 Store hyphenation codes for current language
    fn hyph_store_codes(&mut self) {
        let c = self.cur_lang;
        let mut first_child = false;
        let mut p = 0;
        let mut q;
        loop {
            q = p;
            p = self.trie_r[q];
            if p == 0 || c <= self.trie_c.at(p) {
                break;
            }
        }
        if p == 0 || c < self.trie_c.at(p) {
            p = self.hyph_insert_trie_node(q, p, first_child, c);
        }
        q = p; // now node q represents cur_lang
        // §1856 Store all current lc_code values
        p = self.trie_l[q];
        first_child = true;
        for c in 0..=255 {
            if self.lc_code(c) > 0 || (c == 255 && first_child) {
                if p == 0 {
                    p = self.hyph_insert_trie_node(q, p, first_child, c);
                } else {
                    self.trie_c.put(p, c);
                }
                self.trie_o[p] = self.lc_code(c);
                q = p;
                p = self.trie_r[q];
                first_child = false;
            }
        }
        if first_child {
            self.trie_l[q] = 0;
        } else {
            self.trie_r[q] = 0;
        }
    }

    /// §1135 `clear_trie`.
    fn clear_trie(&mut self, r: i32) {
        self.set_trie_link(r, 0);
        self.set_trie_op(r, MIN_TRIE_OP);
        self.set_trie_char(r, 0);
    }

    /// §1143
    pub fn init_trie(&mut self) {
        // §1129 Get ready to compress the trie
        // §1122 Sort the hyphenation op tables into proper order
        self.op_start[0] = -MIN_TRIE_OP;
        for j in 1..=255 {
            self.op_start[j] = self.op_start[j - 1] + self.trie_used[j - 1];
        }
        for j in 1..=self.trie_op_ptr {
            self.trie_op_hash[j] = self.op_start[self.trie_op_lang[j]] + self.trie_op_val[j]; // destination
        }
        for j in 1..=self.trie_op_ptr {
            while self.trie_op_hash[j] > j {
                let k = self.trie_op_hash[j];
                let t = self.hyf_distance[k];
                self.hyf_distance[k] = self.hyf_distance[j];
                self.hyf_distance[j] = t;
                let t = self.hyf_num[k];
                self.hyf_num[k] = self.hyf_num[j];
                self.hyf_num[j] = t;
                let t = self.hyf_next[k];
                self.hyf_next[k] = self.hyf_next[j];
                self.hyf_next[j] = t;
                self.trie_op_hash[j] = self.trie_op_hash[k];
                self.trie_op_hash[k] = k;
            }
        }
        for p in 0..=self.trie_size {
            self.trie_hash[p] = 0;
        }
        let root = self.compress_trie(self.hyph_root());
        self.set_hyph_root(root);
        let root = self.compress_trie(self.trie_root()); // identify equivalent subtries
        self.set_trie_root(root);
        for p in 0..=self.trie_ptr {
            self.trie_hash[p] = 0; // trie_ref[p]:=0
        }
        for p in 0..=255 {
            self.trie_min[p] = p + 1;
        }
        self.set_trie_link(0, 1);
        self.trie_max = 0;
        if self.trie_root() != 0 {
            self.first_fit(self.trie_root());
            self.trie_pack(self.trie_root());
        }
        if self.hyph_root() != 0 {
            // §1857 Pack all stored hyph_codes
            if self.trie_root() == 0 {
                for p in 0..=255 {
                    self.trie_min[p] = p + 2;
                }
            }
            self.first_fit(self.hyph_root());
            self.trie_pack(self.hyph_root());
            self.hyph_start = self.trie_hash[self.hyph_root()];
        }
        // §1135 Move the data into trie
        if self.trie_max == 0 {
            // no patterns were given
            for r in 0..=256 {
                self.clear_trie(r);
            }
            self.trie_max = 256;
        } else {
            if self.hyph_root() > 0 {
                self.trie_fix(self.hyph_root());
            }
            if self.trie_root() > 0 {
                self.trie_fix(self.trie_root()); // this fixes the non-holes in trie
            }
            let mut r = 0; // now we will zero out all the holes
            loop {
                let s = self.trie_link(r);
                self.clear_trie(r);
                r = s;
                if r > self.trie_max {
                    break;
                }
            }
        }
        self.set_trie_char(0, b'?' as i32); // make trie_char(c)<>c for all c
        self.trie_not_ready = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::testkit::*;
    use crate::toks::testkit::*;

    fn fresh(trie_size: i32) -> &'static mut Tex {
        let t = table_machine();
        print_setup(t);
        t.hyph_size = 8191;
        t.hyph_next = HYPH_PRIME + 1;
        t.trie_size = trie_size;
        t.trie_not_ready = true;
        t
    }

    /// The text as tokens with INITEX's catcodes, ending in a right brace, as the current input.
    fn feed_group(t: &mut Tex, text: &str) {
        let mut v = Vec::new();
        for c in text.bytes() {
            let cmd = if c == b' ' {
                SPACER
            } else {
                t.cat_code(c as i32)
            };
            v.push(cmd * 256 + c as i32);
        }
        v.push(RIGHT_BRACE * 256 + b'}' as i32);
        feed(t, &v);
    }

    /// What new_patterns and new_hyph_exceptions do once their scan_left_brace is past.
    fn run_program(t: &mut Tex, program: &str) {
        let mut pats: Vec<&str> = Vec::new();
        for w in program.split(' ').filter(|w| !w.is_empty()).chain([""]) {
            if w.is_empty()
                || w.starts_with("L=")
                || w.starts_with("S=")
                || w.starts_with("C=")
                || w.starts_with("X=")
            {
                if !pats.is_empty() {
                    t.set_cur_lang();
                    feed_group(t, &pats.join(" "));
                    t.hyph_enter_patterns();
                    if t.saving_hyph_codes() > 0 {
                        t.hyph_store_codes();
                    }
                    pats.clear();
                }
            }
            if w.is_empty() {
            } else if let Some(v) = w.strip_prefix("L=") {
                t.set_language(v.parse().unwrap());
            } else if let Some(v) = w.strip_prefix("S=") {
                t.set_saving_hyph_codes(v.parse().unwrap());
            } else if let Some(v) = w.strip_prefix("C=") {
                let (c, x) = v.split_once(':').unwrap();
                t.set_lc_code(c.parse().unwrap(), x.parse().unwrap());
            } else if let Some(v) = w.strip_prefix("X=") {
                t.set_cur_lang();
                t.hyph_index = 0;
                feed_group(t, v);
                t.hyph_enter_exceptions();
            } else {
                pats.push(w);
            }
        }
    }

    fn fnv(h: &mut u64, v: i32) {
        for b in (v as i64).to_le_bytes() {
            *h = (*h ^ b as u64).wrapping_mul(0x100000001b3);
        }
    }

    fn digest(n: std::ops::RangeInclusive<i32>, f: impl Fn(i32) -> i32) -> String {
        let mut h = 0xcbf29ce484222325u64;
        for k in n {
            fnv(&mut h, f(k));
        }
        h.to_string()
    }

    /// The columns of hyph_trie_oracle.tsv after the program, as the format's hyphenation section would hold them.
    fn dump_columns(t: &mut Tex) -> Vec<String> {
        let slots: Vec<String> = (0..=t.hyph_size)
            .filter(|&k| t.hyph_word[k] != 0)
            .map(|k| format!("{}:{}", k, t.hyph_link[k]))
            .collect();
        let next = if t.hyph_next <= HYPH_PRIME {
            t.hyph_size
        } else {
            t.hyph_next
        };
        if t.trie_not_ready {
            t.init_trie();
        }
        let used: Vec<String> = (0..=255)
            .rev()
            .filter(|&k| t.trie_used[k] > 0)
            .map(|k| format!("{}:{}", k, t.trie_used[k]))
            .collect();
        let m = t.trie_max;
        let ops = t.trie_op_ptr;
        let mut h = 0xcbf29ce484222325u64;
        for k in 1..=ops {
            fnv(&mut h, t.hyf_distance[k]);
        }
        for k in 1..=ops {
            fnv(&mut h, t.hyf_num[k]);
        }
        for k in 1..=ops {
            fnv(&mut h, t.hyf_next[k]);
        }
        vec![
            t.hyph_count.to_string(),
            next.to_string(),
            slots.join(","),
            m.to_string(),
            t.hyph_start.to_string(),
            ops.to_string(),
            used.join(","),
            digest(0..=m, |k| t.trie_link(k)),
            digest(0..=m, |k| t.trie_op(k)),
            digest(0..=m, |k| t.trie_char(k)),
            h.to_string(),
        ]
    }

    fn lines(data: &'static [u8]) -> Vec<Vec<&'static str>> {
        core::str::from_utf8(data)
            .unwrap()
            .lines()
            .map(|l| l.split('\t').collect())
            .collect()
    }

    #[test]
    fn the_packed_trie_is_the_oracles_word_for_word() {
        let rows = lines(include_bytes!("../testdata/hyph_trie_oracle.tsv"));
        assert!(rows.len() >= 20);
        let mut bad = Vec::new();
        for row in &rows {
            let t = fresh(1100000);
            run_program(t, row[0]);
            let got = dump_columns(t);
            if got != row[1..] {
                bad.push(format!(
                    "--- {}\nwant {:?}\ngot  {:?}",
                    &row[0][..row[0].len().min(60)],
                    &row[1..],
                    got
                ));
            }
            Tex::dealloc(t);
        }
        assert!(
            bad.is_empty(),
            "{} of {} differ:\n{}",
            bad.len(),
            rows.len(),
            bad.join("\n")
        );
    }

    /// The hash of trie_node decides only where a node is found, so trie_size does not change the trie.
    #[test]
    fn the_trie_does_not_depend_on_trie_size() {
        let rows = lines(include_bytes!("../testdata/hyph_trie_oracle.tsv"));
        for row in rows.iter().filter(|r| r[0].len() > 1000) {
            let t = fresh(20011);
            run_program(t, row[0]);
            assert_eq!(dump_columns(t), row[1..], "{}", &row[0][..60]);
            Tex::dealloc(t);
        }
    }

    #[test]
    fn hyphens_are_found_where_the_oracle_finds_them() {
        let rows = lines(include_bytes!("../testdata/hyph_words_oracle.tsv"));
        assert!(rows.len() >= 140);
        let mut bad = Vec::new();
        let mut t = fresh(1100000);
        let mut first = true;
        for row in &rows {
            if row[0] == "P" {
                if !first {
                    Tex::dealloc(t);
                    t = fresh(1100000);
                }
                first = false;
                run_program(t, row[1]);
                t.init_trie();
                continue;
            }
            let (lang, l, r, word) = (
                row[1].parse().unwrap(),
                row[2].parse().unwrap(),
                row[3].parse().unwrap(),
                row[4],
            );
            t.set_language(lang);
            t.set_cur_lang();
            t.l_hyf = l;
            t.r_hyf = r;
            t.set_hyph_index();
            t.hn = 0;
            for c in word.bytes() {
                t.set_hc0_lc_code(c as i32);
                t.hn += 1;
                t.hc[t.hn] = t.hc[0];
            }
            let mut pos = Vec::new();
            if t.hn >= l + r && t.hyph_find_hyphens() {
                for j in l..=t.hn - r {
                    if odd(t.hyf[j]) {
                        pos.push(j.to_string());
                    }
                }
            }
            if pos.join(",") != row[5] {
                bad.push(format!(
                    "{} lang {} {},{} want {} got {}",
                    word,
                    lang,
                    l,
                    r,
                    row[5],
                    pos.join(",")
                ));
            }
        }
        Tex::dealloc(t);
        assert!(
            bad.is_empty(),
            "{} of {} differ:\n{}",
            bad.len(),
            rows.len(),
            bad.join("\n")
        );
    }

    /// The font of a row of hyph_recon_oracle.tsv, in font_info as the TFM loader would leave it.
    fn recon_font(t: &mut Tex, program: &str) {
        let f = 1;
        t.hf = f;
        t.char_base[f] = 0;
        t.lig_kern_base[f] = 256;
        t.font_bchar[f] = NON_CHAR;
        t.bchar_label[f] = NON_ADDRESS;
        let steps: Vec<Vec<&str>> = program
            .split(';')
            .filter(|s| !s.is_empty())
            .map(|s| s.split(' ').collect())
            .collect();
        let mut firsts: Vec<&str> = Vec::new();
        for x in &steps {
            if !firsts.contains(&x[0]) {
                firsts.push(x[0]);
            }
        }
        let mut k = 256;
        let mut kerns = Vec::new();
        for first in firsts {
            let group: Vec<&Vec<&str>> = steps.iter().filter(|x| x[0] == first).collect();
            if first == "|" {
                t.bchar_label[f] = k;
            } else {
                let info = FourQuarters {
                    b0: 1,
                    b1: 0,
                    b2: LIG_TAG,
                    b3: k - 256,
                };
                t.font_info[first.as_bytes()[0] as i32].set_qqqq(info);
            }
            for (n, x) in group.iter().enumerate() {
                let next = if x[1] == "|" {
                    t.font_bchar[f] = b'z' as i32;
                    b'z' as i32
                } else {
                    x[1].as_bytes()[0] as i32
                };
                let (op, rem) = if x[2] == "KRN" {
                    kerns.push(x[3].parse::<i32>().unwrap() * 10 * 65536);
                    (KERN_FLAG, kerns.len() as i32 - 1)
                } else {
                    let op = [
                        "LIG", "LIG/", "/LIG", "/LIG/", "", "LIG/>", "/LIG>", "/LIG/>", "", "", "",
                        "/LIG/>>",
                    ]
                    .iter()
                    .position(|o| *o == x[2])
                    .unwrap();
                    (op as i32, x[3].as_bytes()[0] as i32)
                };
                let skip = if n + 1 == group.len() { STOP_FLAG } else { 0 };
                t.font_info[k].set_qqqq(FourQuarters {
                    b0: skip,
                    b1: next,
                    b2: op,
                    b3: rem,
                });
                k += 1;
            }
        }
        t.kern_base[f] = k - KERN_BASE_OFFSET;
        for v in kerns {
            t.font_info[k].set_sc(v);
            k += 1;
        }
    }

    /// The list reconstitute made, as the generator writes what the oracle's box shows.
    fn take_list(t: &mut Tex, out: &mut Vec<String>) {
        let mut p = t.link(HOLD_HEAD);
        while p > NULL {
            if t.is_char_node(p) {
                out.push((t.character(p) as u8 as char).to_string());
            } else if t.ty(p) == LIGATURE_NODE {
                let mut s = format!("{}(", t.character(t.lig_char(p)) as u8 as char);
                if t.subtype(p) > 1 {
                    s.push('|');
                }
                let mut q = t.lig_ptr(p);
                while q > NULL {
                    s.push(t.character(q) as u8 as char);
                    q = t.link(q);
                }
                if odd(t.subtype(p)) {
                    s.push('|');
                }
                s.push(')');
                out.push(s);
            } else {
                assert_eq!(t.ty(p), KERN_NODE);
                out.push(format!("k{}", t.width(p) / 65536));
            }
            p = t.link(p);
        }
    }

    /// What hyphenate does with these, since it cannot run until new_character can: the word up to the hyphen, the
    /// hyphen's pre_break, and the rest.  Exercises each ligature operation and both boundaries.
    #[test]
    fn reconstitute_rebuilds_what_the_oracle_rebuilds() {
        let rows = lines(include_bytes!("../testdata/hyph_recon_oracle.tsv"));
        assert!(rows.len() >= 30);
        let mut bad = Vec::new();
        for row in &rows {
            let t = fresh(1000);
            recon_font(t, row[1]);
            let word = row[2].as_bytes();
            let n = word.len() as i32;
            for (i, c) in word.iter().enumerate() {
                t.hu[i as i32 + 1] = *c as i32;
            }
            t.hu[0] = 256;
            t.hyf[n - 1] = 1;
            t.init_lig = false;
            t.init_list = NULL;
            t.hyf_char = b'-' as i32;
            let bchar = if row[4] == "F" {
                t.font_bchar[1]
            } else {
                NON_CHAR
            };
            let mut j: i32 = row[3].parse().unwrap();
            let (mut prefix, mut pre, mut after) = (Vec::new(), Vec::new(), Vec::new());
            loop {
                j = t.reconstitute(j, n, bchar, t.hyf_char) + 1;
                assert_eq!(t.hyphen_passed, 0, "{}", row[0]);
                take_list(t, &mut prefix);
                if odd(t.hyf[j - 1]) {
                    break;
                }
                assert!(j <= n, "{}", row[0]);
            }
            t.hyf[j - 1] = 0;
            let i = j;
            let c = t.hu[i];
            t.hu[i] = t.hyf_char;
            let mut l = j;
            while l <= i {
                l = t.reconstitute(l, i, t.font_bchar[1], NON_CHAR) + 1;
                take_list(t, &mut pre);
            }
            t.hu[i] = c;
            while j <= n {
                j = t.reconstitute(j, n, bchar, t.hyf_char) + 1;
                assert_eq!(t.hyphen_passed, 0, "{}", row[0]);
                take_list(t, &mut after);
            }
            let got = [prefix.join(" "), pre.join(" "), after.join(" ")];
            if got != row[5..8] {
                bad.push(format!(
                    "--- {}\nwant {:?}\ngot  {:?}",
                    row[0],
                    &row[5..8],
                    got
                ));
            }
            Tex::dealloc(t);
        }
        assert!(
            bad.is_empty(),
            "{} of {} differ:\n{}",
            bad.len(),
            rows.len(),
            bad.join("\n")
        );
    }

    #[test]
    fn new_trie_op_finds_an_op_again_for_the_same_language_only() {
        let t = fresh(1000);
        t.cur_lang = 3;
        let a = t.new_trie_op(1, 2, 0);
        let b = t.new_trie_op(1, 3, 0);
        assert_eq!((a, b), (1, 2));
        assert_eq!(t.new_trie_op(1, 2, 0), a);
        t.cur_lang = 4;
        assert_eq!(t.new_trie_op(1, 2, 0), 1, "op codes are per language");
        assert_eq!(
            (t.trie_op_ptr, t.trie_used[3], t.trie_used[4], t.max_op_used),
            (3, 2, 1, 2)
        );
        Tex::dealloc(t);
    }

    /// A key of 3142*trie_r[p] past 2^31 must not make a negative h.
    #[test]
    fn trie_node_hashes_past_two_to_the_31() {
        let t = fresh(1100000);
        t.trie_c.put(1, 255);
        t.trie_o[1] = 65535;
        t.trie_l[1] = 1099999;
        t.trie_r[1] = 1099999;
        assert_eq!(t.trie_node(1), 1);
        t.trie_c.put(2, 255);
        t.trie_o[2] = 65535;
        t.trie_l[2] = 1099999;
        t.trie_r[2] = 1099999;
        assert_eq!(t.trie_node(2), 1, "an equal node is found, not entered");
        Tex::dealloc(t);
    }
}
