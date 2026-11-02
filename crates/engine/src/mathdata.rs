//! §856-874 Part 34: data structures for math mode.
//!
//! The noads themselves are in `access.rs`; here are their constructors and the routines that display them.
//! `show_node_list`, in `show.rs`, holds the cases of the noads.
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
    /// §862
    pub fn new_noad(&mut self) -> i32 {
        let p = self.get_node(NOAD_SIZE);
        self.set_ty(p, ORD_NOAD);
        self.set_subtype(p, NORMAL);
        let e = self.empty_field;
        let n = self.nucleus(p);
        self.mem[n] = e;
        let n = self.subscr(p);
        self.mem[n] = e;
        let n = self.supscr(p);
        self.mem[n] = e;
        p
    }

    /// §864
    pub fn new_style(&mut self, s: i32) -> i32 {
        let p = self.get_node(STYLE_NODE_SIZE);
        self.set_ty(p, STYLE_NODE);
        self.set_subtype(p, s);
        // The width and depth are not used.
        self.set_width(p, 0);
        self.set_depth(p, 0);
        p
    }

    /// §865
    pub fn new_choice(&mut self) -> i32 {
        let p = self.get_node(STYLE_NODE_SIZE);
        self.set_ty(p, CHOICE_NODE);
        self.set_subtype(p, 0);
        self.set_display_mlist(p, NULL);
        self.set_text_mlist(p, NULL);
        self.set_script_mlist(p, NULL);
        self.set_script_script_mlist(p, NULL);
        p
    }

    /// §867
    pub fn print_fam_and_char(&mut self, p: i32) {
        self.print_esc_s(b"fam");
        self.print_int(self.fam(p));
        self.print_char(b' ' as i32);
        self.print(self.character(p));
    }

    /// §867
    pub fn print_delimiter(&mut self, p: i32) {
        let mut a = self.small_fam(p) * 256 + self.small_char(p);
        a = a * 0x1000 + self.large_fam(p) * 256 + self.large_char(p);
        if a < 0 {
            self.print_int(a); // this should never happen
        } else {
            self.print_hex(a);
        }
    }

    /// §868
    pub fn print_subsidiary_data(&mut self, p: i32, c: i32) {
        if self.cur_length() >= self.depth_threshold {
            if self.math_type(p) != EMPTY {
                self.print_s(b" []");
            }
        } else {
            self.append_char(c); // include c in the recursion history
            self.temp_ptr = p; // prepare for show_info if recursion is needed
            match self.math_type(p) {
                MATH_CHAR => {
                    self.print_ln();
                    self.print_current_string();
                    self.print_fam_and_char(p);
                }
                SUB_BOX => self.show_info(), // recursive call
                SUB_MLIST => {
                    if self.info(p) == NULL {
                        self.print_ln();
                        self.print_current_string();
                        self.print_s(b"{}");
                    } else {
                        self.show_info(); // recursive call
                    }
                }
                _ => {} // empty
            }
            self.flush_char(); // remove c from the recursion history
        }
    }

    /// §869
    pub fn show_info(&mut self) {
        self.show_node_list(self.info(self.temp_ptr));
    }

    /// §870
    pub fn print_style(&mut self, c: i32) {
        match c / 2 {
            0 => self.print_esc_s(b"displaystyle"),
            1 => self.print_esc_s(b"textstyle"),
            2 => self.print_esc_s(b"scriptstyle"),
            3 => self.print_esc_s(b"scriptscriptstyle"),
            _ => self.print_s(b"Unknown style!"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::memkit::*;
    use crate::show::showkit::*;
    use crate::toks::testkit::*;

    /// The rows of show_oracle.tsv that are noads and nothing else: with the noads built by show.rs's own builder,
    /// these need only the routines of this file and of show.rs.
    const NOAD_ROWS: &[&str] = &[
        "noads_kinds", "noads_limits", "noad_scripts", "noad_mlist", "noad_over", "noad_radical", "noad_accent",
        "noad_left_right", "noad_middle", "noad_left_small_fam", "fraction_default", "fraction_thickness",
        "fraction_atop", "fraction_delims", "fraction_right_only", "styles", "choice", "limit_depth_noad",
        "limit_depth_noad1",
    ];

    #[test]
    fn noads_display_as_the_oracle_shows_them() {
        let rows = show_rows();
        let mut bad = Vec::new();
        for name in NOAD_ROWS {
            let row = rows.iter().find(|r| r[0] == *name).unwrap_or_else(|| panic!("no row {}", name));
            let t = case_machine(name, true);
            let p = build(t, name);
            t.show_box(p);
            let got = printed(t);
            let want = format!("{}\n", row[1]);
            if got != want {
                bad.push(format!("--- {}\nwant {:?}\ngot  {:?}", name, want, got));
            }
            Tex::dealloc(t);
        }
        assert!(bad.is_empty(), "{} of {} differ:\n{}", bad.len(), NOAD_ROWS.len(), bad.join("\n"));
    }

    fn print_machine() -> &'static mut Tex {
        let t = mem_machine();
        print_setup(t);
        t
    }

    #[test]
    fn a_new_noad_has_three_empty_fields() {
        let t = mem_machine();
        let v = t.var_used;
        // A used block, so nothing here passes by being zero already.
        let junk = t.get_node(NOAD_SIZE);
        for k in 0..NOAD_SIZE {
            t.mem[junk + k] = Word::hh(-1, -1);
        }
        t.free_node(junk, NOAD_SIZE);
        let p = t.new_noad();
        assert_eq!(p, junk);
        assert_eq!(t.var_used, v + NOAD_SIZE);
        assert_eq!((t.ty(p), t.subtype(p)), (ORD_NOAD, NORMAL));
        for x in [t.nucleus(p), t.supscr(p), t.subscr(p)] {
            assert_eq!((t.math_type(x), t.fam(x), t.character(x), t.info(x)), (EMPTY, 0, 0, NULL));
        }
        Tex::dealloc(t);
    }

    #[test]
    fn a_style_node_and_a_choice_node_have_their_size_and_type() {
        let t = mem_machine();
        let v = t.var_used;
        let s = t.new_style(SCRIPT_STYLE + CRAMPED);
        assert_eq!((t.ty(s), t.subtype(s), t.width(s), t.depth(s)), (STYLE_NODE, SCRIPT_STYLE + CRAMPED, 0, 0));
        let c = t.new_choice();
        assert_eq!((t.ty(c), t.subtype(c)), (CHOICE_NODE, 0));
        assert_eq!(
            (t.display_mlist(c), t.text_mlist(c), t.script_mlist(c), t.script_script_mlist(c)),
            (NULL, NULL, NULL, NULL)
        );
        assert_eq!(t.var_used, v + 2 * STYLE_NODE_SIZE);
        Tex::dealloc(t);
    }

    #[test]
    fn print_style_takes_the_cramped_forms_and_says_when_it_is_lost() {
        let t = print_machine();
        for (c, want) in [
            (DISPLAY_STYLE, "\\displaystyle"),
            (DISPLAY_STYLE + CRAMPED, "\\displaystyle"),
            (TEXT_STYLE, "\\textstyle"),
            (TEXT_STYLE + CRAMPED, "\\textstyle"),
            (SCRIPT_STYLE, "\\scriptstyle"),
            (SCRIPT_STYLE + CRAMPED, "\\scriptstyle"),
            (SCRIPT_SCRIPT_STYLE, "\\scriptscriptstyle"),
            (SCRIPT_SCRIPT_STYLE + CRAMPED, "\\scriptscriptstyle"),
            (8, "Unknown style!"),
            (9, "Unknown style!"),
            (-1, "\\displaystyle"),
            (-2, "Unknown style!"),
        ] {
            t.print_style(c);
            assert_eq!(printed(t), want, "style {}", c);
        }
        Tex::dealloc(t);
    }

    #[test]
    fn print_delimiter_is_hex_of_the_four_fields() {
        let t = print_machine();
        let d = t.get_node(2);
        for (a, b, c, e, want) in [
            (0, 0, 0, 0, "\"0"),
            (2, 0x6A, 3, 0x0C, "\"26A30C"),
            (15, 255, 15, 255, "\"FFFFFF"),
            (0, 0x28, 0, 0, "\"28000"),
            (0, 0, 3, 0, "\"300"),
        ] {
            t.set_small_fam(d, a);
            t.set_small_char(d, b);
            t.set_large_fam(d, c);
            t.set_large_char(d, e);
            t.print_delimiter(d);
            assert_eq!(printed(t), want);
        }
        Tex::dealloc(t);
    }

    #[test]
    fn print_fam_and_char_prints_through_print() {
        let t = print_machine();
        let x = t.get_node(1);
        for (f, c, want) in [(1, b'a' as i32, "\\fam1 a"), (15, 255, "\\fam15 ^^ff"), (0, 13, "\\fam0 ^^M"), (7, 127, "\\fam7 ^^?")] {
            t.set_fam(x, f);
            t.set_character(x, c);
            t.print_fam_and_char(x);
            assert_eq!(printed(t), want);
        }
        Tex::dealloc(t);
    }

    #[test]
    fn print_subsidiary_data_stops_at_the_depth_threshold() {
        let t = print_machine();
        let x = t.get_node(1);
        t.set_math_type(x, MATH_CHAR);
        t.set_fam(x, 1);
        t.set_character(x, b'a' as i32);
        t.depth_threshold = 1;
        t.print_subsidiary_data(x, b'^' as i32);
        assert_eq!(printed(t), "\n^\\fam1 a");
        assert_eq!(t.cur_length(), 0, "the character is out of the history again");
        // At the threshold the field is only hinted at, and an empty one is not even that.
        t.depth_threshold = 0;
        t.print_subsidiary_data(x, b'^' as i32);
        assert_eq!(printed(t), " []");
        t.set_math_type(x, EMPTY);
        t.print_subsidiary_data(x, b'^' as i32);
        assert_eq!(printed(t), "");
        // Empty, under the threshold, prints nothing either; an empty sub_mlist does.
        t.depth_threshold = 1;
        t.print_subsidiary_data(x, b'^' as i32);
        assert_eq!(printed(t), "");
        t.set_math_type(x, SUB_MLIST);
        t.set_info(x, NULL);
        t.print_subsidiary_data(x, b'^' as i32);
        assert_eq!(printed(t), "\n^{}");
        Tex::dealloc(t);
    }
}
