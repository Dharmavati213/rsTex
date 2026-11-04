//! §694-702 Part 32d: the cross-reference table.
//!
//! `avl_put_obj` and `avl_find_obj` are `avlstuff.c`, with libavl's `avl_probe` and `avl_find` ported as they
//! are, on nodes in the arena.  `avlstuff.c` never walks its trees, so nothing depends on their order but
//! the lookups; the literal port keeps those the same even for a name string whose bytes changed after it went
//! in, when the comparison is no longer consistent and the tree's shape decides.  One tree per object type.
//! The C dumps none of this in a format, so `v_pdfobj` starts empty after a format load, as `PdfObjTree` does.
//!
//! A procedure includes every module reachable from it, wherever defined: `tools/web.py tangle NAME`.
#![forbid(unsafe_code)]
#![deny(unreachable_patterns, unused_variables)]
#![allow(non_snake_case, unused_mut, unused_imports, dead_code)]

use crate::access::*;
use crate::consts::*;
use crate::host::*;
use crate::raw::*;
use crate::state::*;

crate::pod_struct! {
    /// libavl's `struct avl_node`, with the `oentry` it points to inline.  Index 0 is not a node, so 0 is null.
    pub struct AvlNode {
        pub link0: i32,
        pub link1: i32,
        /// `oentry.int0`: the object's `obj_info` when it went in.
        pub int0: i32,
        /// `oentry.objptr`.
        pub objptr: i32,
        /// `avl_balance`.
        pub balance: i32,
    }
}

crate::pod_struct! {
    /// POD state private to this module.  Add fields here, not to state.rs.
    pub struct Vars {
        /// `PdfObjTree[t]`: the root's index in `avl_nodes`, or 0.
        pub avl_root: A<i32, { PDF_OBJTYPE_MAX as usize + 1 }>,
        /// The nodes of all the trees.
        pub avl_nodes: PodVec<AvlNode>,
    }
}

/// Non-POD state private to this module, in `self.host`.  Rarely needed.
#[derive(Default)]
pub struct HVars {}

/// `AVL_MAX_HEIGHT` of `avl.h`.
const AVL_MAX_HEIGHT: usize = 32;

/// §702 `is_hex_char`.
#[inline]
fn is_hex_char(c: i32) -> bool {
    (c >= b'0' as i32 && c <= b'9' as i32) || (c >= b'A' as i32 && c <= b'F' as i32) || (c >= b'a' as i32 && c <= b'f' as i32)
}

impl Tex {
    /// §698
    pub fn append_dest_name(&mut self, s: i32, n: i32) {
        if self.pdf_dest_names_ptr == SUP_DEST_NAMES_SIZE {
            self.overflow(b"number of destination names (dest_names_size)", self.dest_names_size);
        }
        if self.pdf_dest_names_ptr == self.dest_names_size {
            let a = (0.2 * self.dest_names_size as f64) as i32;
            if self.dest_names_size < SUP_DEST_NAMES_SIZE - a {
                self.dest_names_size += a;
            } else {
                self.dest_names_size = SUP_DEST_NAMES_SIZE;
            }
            // xrealloc_array: dest_names is inline, at its capacity.
        }
        self.dest_names[self.pdf_dest_names_ptr].objname = s;
        self.dest_names[self.pdf_dest_names_ptr].objnum = n;
        self.pdf_dest_names_ptr += 1;
    }

    /// §698
    pub fn pdf_create_obj(&mut self, t: i32, i: i32) {
        if self.sys_obj_ptr == SUP_OBJ_TAB_SIZE {
            self.overflow(b"indirect objects table size", self.obj_tab_size);
        }
        if self.sys_obj_ptr == self.obj_tab_size {
            let a = (0.2 * self.obj_tab_size as f64) as i32;
            if self.obj_tab_size < SUP_OBJ_TAB_SIZE - a {
                self.obj_tab_size += a;
            } else {
                self.obj_tab_size = SUP_OBJ_TAB_SIZE;
            }
            // xrealloc_array: obj_tab is inline, at its capacity.
        }
        self.sys_obj_ptr += 1;
        self.obj_ptr = self.sys_obj_ptr;
        self.set_obj_info(self.obj_ptr, i);
        self.set_obj_fresh(self.obj_ptr);
        self.set_obj_aux(self.obj_ptr, 0);
        self.avl_put_obj(self.obj_ptr, t);
        if t == OBJ_TYPE_PAGE {
            let mut p = self.head_tab[t];
            // find the right position to insert newly created object
            if p == 0 || self.obj_info(p) < i {
                self.set_obj_link(self.obj_ptr, p);
                self.head_tab[t] = self.obj_ptr;
            } else {
                let mut q = 0;
                while p != 0 {
                    if self.obj_info(p) < i {
                        break;
                    }
                    q = p;
                    p = self.obj_link(p);
                }
                // done:
                self.set_obj_link(q, self.obj_ptr);
                self.set_obj_link(self.obj_ptr, p);
            }
        } else if t != OBJ_TYPE_OTHERS {
            self.set_obj_link(self.obj_ptr, self.head_tab[t]);
            self.head_tab[t] = self.obj_ptr;
            if t == OBJ_TYPE_DEST && i < 0 {
                self.append_dest_name(-self.obj_info(self.obj_ptr), self.obj_ptr);
            }
        }
    }

    /// §698
    pub fn pdf_new_objnum(&mut self) -> i32 {
        self.pdf_create_obj(OBJ_TYPE_OTHERS, 0);
        self.obj_ptr
    }

    /// §698
    pub fn pdf_os_switch(&mut self, pdf_os: bool) {
        if pdf_os && self.pdf_os_enable {
            if !self.pdf_os_mode {
                // back up PDF stream variables
                self.pdf_op_ptr = self.pdf_ptr;
                self.pdf_ptr = self.pdf_os_ptr;
                self.pdf_buf_is_os = true;
                self.pdf_buf_size = self.pdf_os_buf_size;
                self.pdf_os_mode = true; // switch to object stream
            }
        } else if self.pdf_os_mode {
            // back up object stream variables
            self.pdf_os_ptr = self.pdf_ptr;
            self.pdf_ptr = self.pdf_op_ptr;
            self.pdf_buf_is_os = false;
            self.pdf_buf_size = PDF_OP_BUF_SIZE;
            self.pdf_os_mode = false; // switch to PDF stream
        }
    }

    /// §698
    pub fn pdf_os_prepare_obj(&mut self, i: i32, pdf_os_level: i32) {
        self.pdf_os_switch(pdf_os_level > 0 && self.fixed_pdf_objcompresslevel >= pdf_os_level);
        if self.pdf_os_mode {
            if self.pdf_os_cur_objnum == 0 {
                self.pdf_os_cur_objnum = self.pdf_new_objnum();
                self.obj_ptr -= 1; // object stream is not accessible to user
                self.pdf_os_cntr += 1; // only for statistics
                self.pdf_os_objidx = 0;
                self.pdf_ptr = 0; // start fresh object stream
            } else {
                self.pdf_os_objidx += 1;
            }
            self.set_obj_os_idx(i, self.pdf_os_objidx);
            self.set_obj_offset(i, self.pdf_os_cur_objnum as i64);
            self.pdf_os_objnum[self.pdf_os_objidx] = i;
            self.pdf_os_objoff[self.pdf_os_objidx] = self.pdf_ptr;
        } else {
            self.set_obj_offset(i, self.pdf_offset());
            self.set_obj_os_idx(i, -1); // mark it as not included in object stream
        }
    }

    /// §698
    pub fn pdf_begin_obj(&mut self, i: i32, pdf_os_level: i32) {
        self.check_pdfversion();
        self.pdf_os_prepare_obj(i, pdf_os_level);
        if !self.pdf_os_mode {
            self.pdf_print_int(i);
            self.pdf_print_ln_s(b" 0 obj");
        } else if self.pdf_compress_level() == 0 {
            self.pdf_print_s(b"% "); // debugging help
            self.pdf_print_int(i);
            self.pdf_print_ln_s(b" 0 obj");
        }
    }

    /// §698
    pub fn pdf_new_obj(&mut self, t: i32, i: i32, pdf_os: i32) {
        self.pdf_create_obj(t, i);
        self.pdf_begin_obj(self.obj_ptr, pdf_os);
    }

    /// §698
    pub fn pdf_end_obj(&mut self) {
        if self.pdf_os_mode {
            if self.pdf_os_objidx == PDF_OS_MAX_OBJS - 1 {
                self.pdf_os_write_objstream();
            }
        } else {
            self.pdf_print_ln_s(b"endobj"); // end a PDF object
        }
    }

    /// §698
    pub fn pdf_begin_dict(&mut self, i: i32, pdf_os_level: i32) {
        self.check_pdfversion();
        self.pdf_os_prepare_obj(i, pdf_os_level);
        if !self.pdf_os_mode {
            self.pdf_print_int(i);
            self.pdf_print_ln_s(b" 0 obj");
        } else if self.pdf_compress_level() == 0 {
            self.pdf_print_s(b"% "); // debugging help
            self.pdf_print_int(i);
            self.pdf_print_ln_s(b" 0 obj");
        }
        self.pdf_print_ln_s(b"<<");
    }

    /// §698
    pub fn pdf_new_dict(&mut self, t: i32, i: i32, pdf_os: i32) {
        self.pdf_create_obj(t, i);
        self.pdf_begin_dict(self.obj_ptr, pdf_os);
    }

    /// §698
    pub fn pdf_end_dict(&mut self) {
        if self.pdf_os_mode {
            self.pdf_print_ln_s(b">>");
            if self.pdf_os_objidx == PDF_OS_MAX_OBJS - 1 {
                self.pdf_os_write_objstream();
            }
        } else {
            self.pdf_print_ln_s(b">>");
            self.pdf_print_ln_s(b"endobj");
        }
    }

    /// §699
    pub fn pdf_os_write_objstream(&mut self) {
        if self.pdf_os_cur_objnum == 0 {
            return; // no object stream started
        }
        self.host.hv.outcache.kind = b'O';
        let p = self.pdf_ptr;
        let mut i = 0;
        let mut j = 0;
        while i <= self.pdf_os_objidx {
            // assemble object number and byte offset pairs
            self.pdf_print_int(self.pdf_os_objnum[i]);
            self.pdf_print(b' ' as i32);
            self.pdf_print_int(self.pdf_os_objoff[i]);
            if j == 9 {
                // print out in groups of ten for better readability
                self.pdf_out(PDF_NEW_LINE_CHAR);
                j = 0;
            } else {
                self.pdf_print(b' ' as i32);
                j += 1;
            }
            i += 1;
        }
        // no risk of flush, as we are in pdf_os_mode
        if self.pdf_buf_is_os {
            self.pdf_os_buf.put(self.pdf_ptr - 1, PDF_NEW_LINE_CHAR);
        } else {
            self.pdf_op_buf.put(self.pdf_ptr - 1, PDF_NEW_LINE_CHAR);
        }
        let mut q = self.pdf_ptr;
        self.pdf_begin_dict(self.pdf_os_cur_objnum, 0); // switch to PDF stream writing
        self.pdf_print_ln_s(b"/Type /ObjStm");
        self.pdf_print_s(b"/N ");
        self.pdf_print_int_ln(self.pdf_os_objidx + 1);
        self.pdf_print_s(b"/First ");
        self.pdf_print_int_ln(q - p);
        self.pdf_begin_stream();
        self.pdf_room(q - p); // should always fit into the PDF output buffer
        i = p;
        while i < q {
            // write object number and byte offset pairs
            self.pdf_quick_out(self.pdf_os_buf.at(i));
            i += 1;
        }
        i = 0;
        while i < p {
            q = i + self.pdf_buf_size;
            if q > p {
                q = p;
            }
            self.pdf_room(q - i);
            while i < q {
                // write the buffered objects
                self.pdf_quick_out(self.pdf_os_buf.at(i));
                i += 1;
            }
        }
        self.pdf_end_stream();
        self.pdf_os_cur_objnum = 0; // to force object stream generation next time
    }

    /// §700
    pub fn append_ptr(&mut self, p: i32, i: i32) -> i32 {
        let mut p = p;
        let result = p;
        let q = self.fast_get_avail();
        self.set_info(q, i);
        self.set_link(q, NULL);
        if p == NULL {
            return q;
        }
        while self.link(p) != NULL {
            p = self.link(p);
        }
        self.set_link(p, q);
        result
    }

    /// §700
    pub fn pdf_lookup_list(&mut self, p: i32, i: i32) -> i32 {
        let mut p = p;
        while p != NULL {
            if self.info(p) == i {
                return p;
            }
            p = self.link(p);
        }
        NULL
    }

    /// §702
    pub fn pdf_print_fw_int(&mut self, n: impl Into<i64>, w: i32) {
        let mut n: i64 = n.into();
        let mut k = 0;
        loop {
            self.dig[k] = (n % 10) as i32;
            n /= 10;
            k += 1;
            if k == w {
                break;
            }
        }
        self.pdf_room(k);
        while k > 0 {
            k -= 1;
            self.pdf_quick_out(b'0' as i32 + self.dig[k]);
        }
    }

    /// §702
    pub fn pdf_out_bytes(&mut self, n: impl Into<i64>, w: i32) {
        let mut n: i64 = n.into();
        let mut byte = [0i32; 8]; // digits in a number being output
        let mut k = 0;
        loop {
            byte[k] = (n % 256) as i32;
            n /= 256;
            k += 1;
            if k as i32 == w {
                break;
            }
        }
        self.pdf_room(k as i32);
        while k > 0 {
            k -= 1;
            self.pdf_quick_out(byte[k]);
        }
    }

    /// §702
    pub fn pdf_int_entry(&mut self, s: &[u8], v: i32) {
        self.pdf_out(b'/' as i32);
        self.pdf_print_s(s);
        self.pdf_out(b' ' as i32);
        self.pdf_print_int(v);
    }

    /// §702
    pub fn pdf_int_entry_ln(&mut self, s: &[u8], v: i32) {
        self.pdf_int_entry(s, v);
        self.pdf_print_nl();
    }

    /// §702
    pub fn pdf_indirect(&mut self, s: &[u8], o: i32) {
        self.pdf_out(b'/' as i32);
        self.pdf_print_s(s);
        self.pdf_out(b' ' as i32);
        self.pdf_print_int(o);
        self.pdf_print_s(b" 0 R");
    }

    /// §702
    pub fn pdf_indirect_ln(&mut self, s: &[u8], o: i32) {
        self.pdf_indirect(s, o);
        self.pdf_print_nl();
    }

    /// §702
    pub fn pdf_print_str(&mut self, s: i32) {
        let mut i = self.str_start[s];
        let mut j = i + self.length(s) - 1;
        if i > j {
            self.pdf_print_s(b"()"); // null string
            return;
        }
        if self.str_pool.at(i) == b'(' as i32 && self.str_pool.at(j) == b')' as i32 {
            self.pdf_print(s);
            return;
        }
        let mut is_hex_string = false;
        'done: {
            if self.str_pool.at(i) != b'<' as i32 || self.str_pool.at(j) != b'>' as i32 || odd(self.length(s)) {
                break 'done;
            }
            i += 1;
            j -= 1;
            while i < j {
                if is_hex_char(self.str_pool.at(i)) && is_hex_char(self.str_pool.at(i + 1)) {
                    i += 2;
                } else {
                    break 'done;
                }
            }
            is_hex_string = true;
        }
        if is_hex_string {
            self.pdf_print(s);
        } else {
            self.pdf_out(b'(' as i32);
            self.pdf_print(s);
            self.pdf_out(b')' as i32);
        }
    }

    /// §702
    pub fn pdf_print_str_ln(&mut self, s: i32) {
        self.pdf_print_str(s);
        self.pdf_print_nl();
    }

    /// §702
    pub fn pdf_str_entry(&mut self, s: &[u8], v: i32) {
        if v == 0 {
            return;
        }
        self.pdf_out(b'/' as i32);
        self.pdf_print_s(s);
        self.pdf_out(b' ' as i32);
        self.pdf_print_str(v);
    }

    /// §702
    pub fn pdf_str_entry_ln(&mut self, s: &[u8], v: i32) {
        if v == 0 {
            return;
        }
        self.pdf_str_entry(s, v);
        self.pdf_print_nl();
    }

    // ---- avlstuff.c and avl.c. ----

    /// `compare_info` of `avlstuff.c`: two negative numbers are string numbers, by length and then by bytes.
    fn pdfobj_compare_info(&self, a: i32, b: i32) -> i32 {
        if a < 0 && b < 0 {
            let (mut pa, ae) = (self.str_start[-a], self.str_start[-a + 1]);
            let (mut pb, be) = (self.str_start[-b], self.str_start[-b + 1]);
            let (al, bl) = (ae - pa, be - pb);
            if al < bl {
                return -1;
            }
            if al > bl {
                return 1;
            }
            while pa < ae {
                if self.str_pool.at(pa) < self.str_pool.at(pb) {
                    return -1;
                }
                if self.str_pool.at(pa) > self.str_pool.at(pb) {
                    return 1;
                }
                pa += 1;
                pb += 1;
            }
        } else {
            if a < b {
                return -1;
            }
            if a > b {
                return 1;
            }
        }
        0
    }

    /// Node `p`.
    #[inline]
    fn pdfobj_node(&mut self, p: i32) -> AvlNode {
        self.v_pdfobj.avl_nodes.get(&mut self.arena, p)
    }

    /// `p->avl_link[dir]`.  Node 0 stands for `&tree->avl_root`, as libavl's cast does, in tree `t`.
    #[inline]
    fn pdfobj_link(&mut self, t: i32, p: i32, dir: i32) -> i32 {
        if p == 0 {
            return self.v_pdfobj.avl_root[t];
        }
        let n = self.pdfobj_node(p);
        if dir == 0 { n.link0 } else { n.link1 }
    }

    /// `p->avl_link[dir] = v`.
    #[inline]
    fn pdfobj_set_link(&mut self, t: i32, p: i32, dir: i32, v: i32) {
        if p == 0 {
            self.v_pdfobj.avl_root[t] = v;
            return;
        }
        let mut n = self.pdfobj_node(p);
        if dir == 0 {
            n.link0 = v;
        } else {
            n.link1 = v;
        }
        self.v_pdfobj.avl_nodes.set(&mut self.arena, p, n);
    }

    /// `p->avl_balance`.
    #[inline]
    fn pdfobj_balance(&mut self, p: i32) -> i32 {
        self.pdfobj_node(p).balance
    }

    /// `p->avl_balance = v`.
    #[inline]
    fn pdfobj_set_balance(&mut self, p: i32, v: i32) {
        let mut n = self.pdfobj_node(p);
        n.balance = v;
        self.v_pdfobj.avl_nodes.set(&mut self.arena, p, n);
    }

    /// External: avlstuff.c `avlputobj`, with libavl's `avl_probe`: an item equal to one in the tree is not
    /// inserted, so of all the objects of type 0, with info 0, only the first has a node.
    pub fn avl_put_obj(&mut self, objptr: i32, t: i32) {
        let int0 = self.obj_tab[objptr].int0;
        let mut z = 0; // top node to update balance factor, and parent
        let mut y = self.v_pdfobj.avl_root[t];
        let mut dir = 0; // direction to descend
        let mut da = [0i32; AVL_MAX_HEIGHT]; // cached comparison results
        let mut k = 0; // number of cached results
        let mut q = z; // parent
        let mut p = y; // iterator
        while p != 0 {
            let other = self.pdfobj_node(p).int0;
            let cmp = self.pdfobj_compare_info(int0, other);
            if cmp == 0 {
                return;
            }
            if self.pdfobj_balance(p) != 0 {
                z = q;
                y = p;
                k = 0;
            }
            dir = (cmp > 0) as i32;
            da[k] = dir;
            k += 1;
            q = p;
            p = self.pdfobj_link(t, p, dir);
        }

        if self.v_pdfobj.avl_nodes.len == 0 {
            let mut nodes = self.v_pdfobj.avl_nodes;
            self.arena_push(&mut nodes, AvlNode { link0: 0, link1: 0, int0: 0, objptr: 0, balance: 0 });
            self.v_pdfobj.avl_nodes = nodes;
        }
        let n = self.v_pdfobj.avl_nodes.len;
        let mut nodes = self.v_pdfobj.avl_nodes;
        self.arena_push(&mut nodes, AvlNode { link0: 0, link1: 0, int0, objptr, balance: 0 });
        self.v_pdfobj.avl_nodes = nodes;
        self.pdfobj_set_link(t, q, dir, n);
        if y == 0 {
            return;
        }

        p = y;
        k = 0;
        while p != n {
            let b = self.pdfobj_balance(p);
            self.pdfobj_set_balance(p, if da[k] == 0 { b - 1 } else { b + 1 });
            p = self.pdfobj_link(t, p, da[k]);
            k += 1;
        }

        let w; // new root of rebalanced subtree
        let yb = self.pdfobj_balance(y);
        if yb == -2 {
            let x = self.pdfobj_link(t, y, 0);
            if self.pdfobj_balance(x) == -1 {
                w = x;
                let c = self.pdfobj_link(t, x, 1);
                self.pdfobj_set_link(t, y, 0, c);
                self.pdfobj_set_link(t, x, 1, y);
                self.pdfobj_set_balance(x, 0);
                self.pdfobj_set_balance(y, 0);
            } else {
                w = self.pdfobj_link(t, x, 1);
                let c = self.pdfobj_link(t, w, 0);
                self.pdfobj_set_link(t, x, 1, c);
                self.pdfobj_set_link(t, w, 0, x);
                let c = self.pdfobj_link(t, w, 1);
                self.pdfobj_set_link(t, y, 0, c);
                self.pdfobj_set_link(t, w, 1, y);
                let wb = self.pdfobj_balance(w);
                if wb == -1 {
                    self.pdfobj_set_balance(x, 0);
                    self.pdfobj_set_balance(y, 1);
                } else if wb == 0 {
                    self.pdfobj_set_balance(x, 0);
                    self.pdfobj_set_balance(y, 0);
                } else {
                    self.pdfobj_set_balance(x, -1);
                    self.pdfobj_set_balance(y, 0);
                }
                self.pdfobj_set_balance(w, 0);
            }
        } else if yb == 2 {
            let x = self.pdfobj_link(t, y, 1);
            if self.pdfobj_balance(x) == 1 {
                w = x;
                let c = self.pdfobj_link(t, x, 0);
                self.pdfobj_set_link(t, y, 1, c);
                self.pdfobj_set_link(t, x, 0, y);
                self.pdfobj_set_balance(x, 0);
                self.pdfobj_set_balance(y, 0);
            } else {
                w = self.pdfobj_link(t, x, 0);
                let c = self.pdfobj_link(t, w, 1);
                self.pdfobj_set_link(t, x, 0, c);
                self.pdfobj_set_link(t, w, 1, x);
                let c = self.pdfobj_link(t, w, 0);
                self.pdfobj_set_link(t, y, 1, c);
                self.pdfobj_set_link(t, w, 0, y);
                let wb = self.pdfobj_balance(w);
                if wb == 1 {
                    self.pdfobj_set_balance(x, 0);
                    self.pdfobj_set_balance(y, -1);
                } else if wb == 0 {
                    self.pdfobj_set_balance(x, 0);
                    self.pdfobj_set_balance(y, 0);
                } else {
                    self.pdfobj_set_balance(x, 1);
                    self.pdfobj_set_balance(y, 0);
                }
                self.pdfobj_set_balance(w, 0);
            }
        } else {
            return;
        }
        let d = (y != self.pdfobj_link(t, z, 0)) as i32;
        self.pdfobj_set_link(t, z, d, w);
    }

    /// External: avlstuff.c `avlfindobj`, with libavl's `avl_find`
    pub fn avl_find_obj(&mut self, t: i32, i: i32, byname: i32) -> i32 {
        let item = if byname > 0 { -i } else { i };
        let mut p = self.v_pdfobj.avl_root[t];
        while p != 0 {
            let n = self.pdfobj_node(p);
            let cmp = self.pdfobj_compare_info(item, n.int0);
            if cmp < 0 {
                p = n.link0;
            } else if cmp > 0 {
                p = n.link1;
            } else {
                return n.objptr;
            }
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdfout::pdfkit::*;
    use crate::toks::testkit::*;

    /// Object `i` with info `v` of type `t`, by the procedure the engine uses.
    fn obj(t: &mut Tex, ty: i32, v: i32) -> i32 {
        t.obj_tab_size = 100000;
        t.pdf_create_obj(ty, v);
        t.obj_ptr
    }

    /// In order, balanced, with balance factors that are true.  Returns the height.
    fn check_subtree(t: &mut Tex, p: i32, seen: &mut Vec<i32>) -> i32 {
        if p == 0 {
            return 0;
        }
        let n = t.pdfobj_node(p);
        let l = check_subtree(t, n.link0, seen);
        seen.push(n.int0);
        let r = check_subtree(t, n.link1, seen);
        assert_eq!(n.balance, r - l, "balance factor of node {p}");
        assert!(n.balance.abs() <= 1, "balanced at node {p}");
        l.max(r) + 1
    }

    fn check_tree(t: &mut Tex, ty: i32) -> Vec<i32> {
        let mut seen = Vec::new();
        check_subtree(t, t.v_pdfobj.avl_root[ty], &mut seen);
        seen
    }

    #[test]
    fn the_tree_stays_balanced_and_finds_what_went_in() {
        let t = pdf_machine();
        print_setup(t);
        // Ascending, descending, and scattered, which between them take every rotation of avl_probe.
        let ascending: Vec<i32> = (1..=2000).collect();
        let descending: Vec<i32> = (1..=2000).rev().collect();
        let scattered: Vec<i32> = (0..2000).map(|i| 1 + (i * 733) % 2003).collect();
        for (ty, values) in [(OBJ_TYPE_XFORM, &ascending), (OBJ_TYPE_XIMAGE, &descending), (OBJ_TYPE_THREAD, &scattered)] {
            let mut objs = Vec::new();
            for &v in values {
                let o = obj(t, ty, v);
                objs.push((v, o));
                if objs.len() % 97 == 0 {
                    let seen = check_tree(t, ty);
                    assert!(seen.windows(2).all(|w| w[0] < w[1]), "in order");
                    assert_eq!(seen.len(), objs.len());
                }
            }
            for &(v, o) in &objs {
                assert_eq!(t.avl_find_obj(ty, v, 0), o);
            }
            assert_eq!(t.avl_find_obj(ty, 0, 0), 0);
            assert_eq!(t.avl_find_obj(ty, 3000, 0), 0);
        }
        assert_eq!(t.avl_find_obj(OBJ_TYPE_OBJ, 1, 0), 0, "a tree never made");
        Tex::dealloc(t);
    }

    /// A left-right and a right-left case by hand: libavl's shapes, not only balance.
    #[test]
    fn rotations_give_libavls_shapes() {
        let t = pdf_machine();
        print_setup(t);
        for (values, root, left, right) in [
            ([1, 2, 3], 2, 1, 3),
            ([3, 2, 1], 2, 1, 3),
            ([1, 3, 2], 2, 1, 3),
            ([3, 1, 2], 2, 1, 3),
        ] {
            let ty = OBJ_TYPE_OBJ;
            t.v_pdfobj.avl_root[ty] = 0;
            for v in values {
                obj(t, ty, v);
            }
            let r = t.pdfobj_node(t.v_pdfobj.avl_root[ty]);
            assert_eq!(r.int0, root);
            assert_eq!(t.pdfobj_node(r.link0).int0, left);
            assert_eq!(t.pdfobj_node(r.link1).int0, right);
            assert_eq!(r.balance, 0);
        }
        Tex::dealloc(t);
    }

    #[test]
    fn an_equal_item_does_not_go_in_and_the_first_is_found() {
        let t = pdf_machine();
        print_setup(t);
        let first = t.pdf_new_objnum();
        let nodes = t.v_pdfobj.avl_nodes.len;
        for _ in 0..50000 {
            t.pdf_new_objnum();
        }
        assert_eq!(t.v_pdfobj.avl_nodes.len, nodes, "all of type 0 with info 0: one node");
        assert_eq!(t.avl_find_obj(OBJ_TYPE_OTHERS, 0, 0), first);
        Tex::dealloc(t);
    }

    #[test]
    fn names_compare_by_length_then_bytes_and_numbers_as_numbers() {
        let t = pdf_machine();
        print_setup(t);
        let ty = OBJ_TYPE_DEST;
        t.dest_names_size = 1000;
        let names: Vec<&[u8]> = vec![b"b", b"a", b"aa", b"ab", b"z", b"", b"\xff", b"\x80", b"abc"];
        let mut made = Vec::new();
        for n in &names {
            let s = string(t, n);
            made.push((s, obj(t, ty, -s)));
        }
        let five = obj(t, ty, 5);
        // The same bytes under another string number are the same name.
        for (n, &(_, o)) in names.iter().zip(&made) {
            let s = string(t, n);
            assert_eq!(t.avl_find_obj(ty, s, 1), o, "{:?}", n);
            t.flush_string();
        }
        let s = string(t, b"zz");
        assert_eq!(t.avl_find_obj(ty, s, 1), 0);
        // A number is not a name, and a positive int0 is above every negative one.
        assert_eq!(t.avl_find_obj(ty, 5, 0), five);
        assert_eq!(t.avl_find_obj(ty, 6, 0), 0);
        let seen = check_tree(t, ty);
        assert_eq!(*seen.last().unwrap(), 5);
        // Sorted as compare_info sorts: shorter first, then unsigned bytes.
        let in_order: Vec<Vec<u8>> =
            seen[..seen.len() - 1].iter().map(|&i| t.str_pool.range(t.str_start[-i], t.str_start[-i + 1]).to_vec()).collect();
        let want: Vec<&[u8]> = vec![b"", b"a", b"b", b"z", b"\x80", b"\xff", b"aa", b"ab", b"abc"];
        assert_eq!(in_order, want);
        // Each became a destination name, in order of creation.
        assert_eq!(t.pdf_dest_names_ptr, names.len() as i32);
        assert_eq!((t.dest_names[0].objname, t.dest_names[0].objnum), made[0]);
        Tex::dealloc(t);
    }

    #[test]
    fn page_objects_are_kept_sorted_descending() {
        let t = pdf_machine();
        print_setup(t);
        for v in [5, 9, 3, 7, 4, 12, 1] {
            obj(t, OBJ_TYPE_PAGE, v);
        }
        let mut got = Vec::new();
        let mut p = t.head_tab[OBJ_TYPE_PAGE];
        while p != 0 {
            got.push(t.obj_info(p));
            p = t.obj_link(p);
        }
        assert_eq!(got, [12, 9, 7, 5, 4, 3, 1]);
        // Any other type goes on the front.
        for v in [5, 9, 3] {
            obj(t, OBJ_TYPE_XFORM, v);
        }
        let head = t.head_tab[OBJ_TYPE_XFORM];
        assert_eq!(t.obj_info(head), 3);
        assert_eq!(t.obj_info(t.obj_link(head)), 9);
        Tex::dealloc(t);
    }

    #[test]
    fn tables_grow_by_a_fifth() {
        let t = pdf_machine();
        print_setup(t);
        t.obj_tab_size = 1000;
        t.dest_names_size = 1000;
        t.sys_obj_ptr = 999;
        t.pdf_new_objnum();
        assert_eq!(t.obj_tab_size, 1000);
        t.pdf_new_objnum();
        assert_eq!((t.obj_tab_size, t.sys_obj_ptr), (1200, 1001));
        t.pdf_dest_names_ptr = 1000;
        t.append_dest_name(300, 7);
        assert_eq!((t.dest_names_size, t.pdf_dest_names_ptr), (1200, 1001));
        assert_eq!((t.dest_names[1000].objname, t.dest_names[1000].objnum), (300, 7));
        Tex::dealloc(t);
    }

    fn entry(t: &mut Tex, f: impl FnOnce(&mut Tex)) -> String {
        f(t);
        taken(t)
    }

    #[test]
    fn entries_and_strings() {
        let t = pdf_machine();
        print_setup(t);
        assert_eq!(entry(t, |t| t.pdf_int_entry(b"Size", 12)), "/Size 12");
        assert_eq!(entry(t, |t| t.pdf_int_entry_ln(b"Size", -12)), "/Size -12\n");
        assert_eq!(entry(t, |t| t.pdf_indirect(b"Root", 3)), "/Root 3 0 R");
        assert_eq!(entry(t, |t| t.pdf_indirect_ln(b"Root", 3)), "/Root 3 0 R\n");
        assert_eq!(entry(t, |t| t.pdf_str_entry(b"T", 0)), "");
        assert_eq!(entry(t, |t| t.pdf_str_entry_ln(b"T", 0)), "");
        // tests/integration/pdfobj/strings.tex has the oracle on the same forms.
        for (s, want) in [
            (&b""[..], "()"),
            (b"x", "(x)"),
            (b"(x)", "(x)"),
            (b"()", "()"),
            (b"(x", "((x)"),
            (b"x)", "(x))"),
            (b")(", "()()"),
            (b"(", "(()"),
            (b"<>", "<>"),
            (b"<", "(<)"),
            (b">", "(>)"),
            (b"><", "(><)"),
            (b"<a>", "(<a>)"),
            (b"<ab>", "<ab>"),
            (b"<AB>", "<AB>"),
            (b"<aB09>", "<aB09>"),
            (b"<abc>", "(<abc>)"),
            (b"<g0>", "(<g0>)"),
            (b"<0g>", "(<0g>)"),
            (b"<000g>", "(<000g>)"),
            (b"<:0>", "(<:0>)"),
            (b"<0@>", "(<0@>)"),
            (b"<G0>", "(<G0>)"),
            (b"<0/>", "(<0/>)"),
            (b"<`0>", "(<`0>)"),
        ] {
            let v = string(t, s);
            assert_eq!(entry(t, |t| t.pdf_print_str(v)), want, "{:?}", String::from_utf8_lossy(s));
            assert_eq!(entry(t, |t| t.pdf_print_str_ln(v)), want.to_string() + "\n");
            assert_eq!(entry(t, |t| t.pdf_str_entry(b"T", v)), "/T ".to_string() + want);
            assert_eq!(entry(t, |t| t.pdf_str_entry_ln(b"T", v)), "/T ".to_string() + want + "\n");
        }
        Tex::dealloc(t);
    }

    #[test]
    fn append_ptr_and_lookup() {
        let t = crate::mem::memkit::mem_machine();
        let mut l = NULL;
        assert_eq!(t.pdf_lookup_list(l, 1), NULL);
        for i in [5, 6, 7] {
            l = t.append_ptr(l, i);
        }
        assert_eq!((t.info(l), t.info(t.link(l)), t.info(t.link(t.link(l)))), (5, 6, 7));
        assert_eq!(t.link(t.link(t.link(l))), NULL);
        assert_eq!(t.pdf_lookup_list(l, 5), l);
        assert_eq!(t.pdf_lookup_list(l, 7), t.link(t.link(l)));
        assert_eq!(t.pdf_lookup_list(l, 8), NULL);
        Tex::dealloc(t);
    }

    fn os_machine() -> &'static mut Tex {
        let t = pdf_machine();
        print_setup(t);
        t.obj_tab_size = 100000;
        t.pdf_os_enable = true;
        t.fixed_pdf_objcompresslevel = 2;
        t.set_pdf_compress_level(0);
        t
    }

    /// 100 objects fill an object stream.  The frame, the pairs in tens and /First against what the oracle
    /// writes for tests/integration/pdfobj/objstm.tex.
    #[test]
    fn a_full_object_stream_goes_out_with_its_pairs() {
        let t = os_machine();
        let mut nums = Vec::new();
        let mut offs = Vec::new();
        let mut body = Vec::new();
        for k in 0..100 {
            let i = t.pdf_new_objnum();
            nums.push(i);
            t.pdf_begin_obj(i, 2);
            offs.push(body.len());
            body.extend_from_slice(format!("% {i} 0 obj\n{k}\n").as_bytes());
            let v = k;
            t.pdf_print_int_ln(v);
            assert_eq!(t.obj_offset(i), 2, "the first object's preparation made the stream, object 2");
            assert_eq!(t.obj_os_idx(i), k);
            t.pdf_end_obj();
        }
        assert!(!t.pdf_os_mode, "the 100th object wrote the stream and left object stream mode");
        assert_eq!(t.pdf_os_cur_objnum, 0);
        assert_eq!(t.pdf_os_cntr, 1);
        let f = file(t);
        let mut pairs = String::new();
        for (k, (i, o)) in nums.iter().zip(&offs).enumerate() {
            pairs += &format!("{i} {o}");
            pairs.push(if (k + 1) % 10 == 0 { '\n' } else { ' ' });
        }
        let mut data = pairs.clone().into_bytes();
        data.extend_from_slice(&body);
        let head = format!("2 0 obj\n<<\n/Type /ObjStm\n/N 100\n/First {}\n/Length {:<10}\n>>\nstream\n", pairs.len(), data.len());
        let mut want = head.into_bytes();
        want.extend_from_slice(&data);
        want.extend_from_slice(b"\nendstream\nendobj\n");
        assert_eq!(String::from_utf8_lossy(&f), String::from_utf8_lossy(&want));
        assert_eq!(t.obj_offset(2), 0, "at the file offset where it began");
        assert_eq!(t.obj_os_idx(2), -1, "not in an object stream");
        Tex::dealloc(t);
    }

    /// Fewer than 100, with a count that does not end a group of ten: the trailing space becomes a newline.  The buffer is
    /// passed in pieces when the objects are bigger than the output buffer.
    #[test]
    fn a_short_object_stream_written_at_the_end() {
        let t = os_machine();
        let big = vec![b'x'; 40000];
        let i1 = t.pdf_new_objnum();
        t.pdf_begin_obj(i1, 1);
        t.pdf_print_s(&big);
        t.pdf_end_obj();
        // A level too high stays out, and writes around the buffered objects.
        let i2 = t.pdf_new_objnum();
        t.pdf_begin_obj(i2, 3);
        t.pdf_print_ln_s(b"plain");
        t.pdf_end_obj();
        assert!(!t.pdf_os_mode);
        assert_eq!(t.obj_os_idx(i2), -1);
        let i3 = t.pdf_new_objnum();
        t.pdf_begin_dict(i3, 2);
        t.pdf_end_dict();
        t.pdf_os_switch(true);
        t.pdf_os_write_objstream();
        t.pdf_flush();
        t.pdf_os_switch(false);
        let f = file(t);
        let text = String::from_utf8_lossy(&f).into_owned();
        assert!(text.starts_with(&format!("{i2} 0 obj\nplain\nendobj\n{} 0 obj\n<<\n/Type /ObjStm\n/N 2\n", t.obj_offset(i1))), "{}", &text[..60]);
        let c1 = format!("% {i1} 0 obj\n");
        let first = format!("{i1} 0 {i3} {}\n", c1.len() + 40000);
        assert!(text.contains(&format!("/First {}\n", first.len())));
        let at = text.find("stream\n").unwrap() + 7;
        assert_eq!(&text[at..at + first.len()], first);
        let tail = format!("{c1}{}% {i3} 0 obj\n<<\n>>\n\nendstream\nendobj\n", String::from_utf8_lossy(&big));
        assert_eq!(&text[at + first.len()..], tail);
        // Nothing started: nothing written.
        let n = f.len();
        t.pdf_os_switch(true);
        t.pdf_os_write_objstream();
        t.pdf_os_switch(false);
        assert_eq!(file(t).len(), n);
        Tex::dealloc(t);
    }

    #[test]
    fn plain_objects_when_object_streams_are_off() {
        let t = pdf_machine();
        print_setup(t);
        t.obj_tab_size = 1000;
        t.pdf_print_s(b"head\n");
        t.pdf_new_obj(OBJ_TYPE_OTHERS, 0, 1);
        t.pdf_print_ln_s(b"7");
        t.pdf_end_obj();
        t.pdf_new_dict(OBJ_TYPE_OTHERS, 0, 2);
        t.pdf_end_dict();
        assert_eq!(String::from_utf8(file(t)).unwrap(), "head\n1 0 obj\n7\nendobj\n2 0 obj\n<<\n>>\nendobj\n");
        assert_eq!((t.obj_offset(1), t.obj_os_idx(1), t.obj_offset(2)), (5, -1, 22));
        Tex::dealloc(t);
    }
}
