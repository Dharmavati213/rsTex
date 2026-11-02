//! §565-609 Part 30: font metric data.
//!
//! `getc` gives -1 past the end of the file, as C does, and nothing here checks for that until §602: a
//! short file reads as what the -1s make of the fields.  `tfm_temp` keeps the -1, while `a`, `b`, `c` and
//! `d` are `eight_bits`, `unsigned char` in the oracle's C, so there it is 255: that is the `& 255` below.
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
    /// §590 `fget`
    #[inline(always)]
    fn fget(&mut self) {
        self.tfm_temp = self.getc(TFM_FILE);
    }

    /// §590 `store_four_quarters`, up to its assignment: `a`, `b`, `c`, `d` are in the result.
    #[inline]
    fn tfm_four_quarters(&mut self) -> FourQuarters {
        self.fget();
        let b0 = self.tfm_temp & 255;
        self.fget();
        let b1 = self.tfm_temp & 255;
        self.fget();
        let b2 = self.tfm_temp & 255;
        self.fget();
        let b3 = self.tfm_temp & 255;
        FourQuarters { b0, b1, b2, b3 }
    }

    /// §571 `store_scaled`, up to its assignment: `None` is `abort`.
    #[inline]
    fn tfm_scaled(&mut self, z: i32, alpha: i32, beta: i32) -> Option<i32> {
        let q = self.tfm_four_quarters();
        let (a, b, c, d) = (q.b0, q.b1, q.b2, q.b3);
        let sw = (((d * z) / 0o400 + c * z) / 0o400 + b * z) / beta;
        if a == 0 {
            Some(sw)
        } else if a == 255 {
            Some(sw - alpha)
        } else {
            None
        }
    }

    /// §587 `start_font_error_message`
    fn start_font_error_message(&mut self, u: i32, nom: i32, aire: i32, s: i32) {
        self.print_err_s(b"Font ");
        self.sprint_cs(u);
        self.print_char(b'=' as i32);
        self.print_file_name(nom, aire, S_EMPTY);
        if s >= 0 {
            self.print_s(b" at ");
            self.print_scaled(s);
            self.print_s(b"pt");
        } else if s != -1000 {
            self.print_s(b" scaled ");
            self.print_int(-s);
        }
    }

    /// §586
    pub fn read_font_info(&mut self, u: i32, nom: i32, aire: i32, s: i32) -> i32 {
        // §590 `read_sixteen`, with `abort` at the first byte.
        macro_rules! read_sixteen {
            ($x:ident, $bad:lifetime) => {
                $x = self.tfm_temp;
                if $x > 127 {
                    break $bad;
                }
                self.fget();
                $x = $x * 0o400 + self.tfm_temp;
            };
        }
        let mut g = NULL_FONT;
        let mut file_opened = false;
        let name_too_long = self.length(nom) > 255 || self.length(aire) > 255;
        'done: {
            'bad_tfm: {
                // §588 Read and check the font data; abort if the TFM file is malformed
                // §589 Open tfm_file for input
                if name_too_long {
                    break 'bad_tfm;
                }
                // kpse_find_file will append the ".tfm", and avoid searching the disk before the font
                // alias files as well.
                self.pack_file_name(nom, aire, S_EMPTY);
                if !self.b_open_in(TFM_FILE) {
                    break 'bad_tfm;
                }
                file_opened = true;
                // §591 Read the TFM size fields
                let (mut lf, mut lh, mut bc, mut ec, mut nw, mut nh, mut nd, mut ni, mut nl, mut nk, mut ne, mut np);
                read_sixteen!(lf, 'bad_tfm);
                self.fget();
                read_sixteen!(lh, 'bad_tfm);
                self.fget();
                read_sixteen!(bc, 'bad_tfm);
                self.fget();
                read_sixteen!(ec, 'bad_tfm);
                if bc > ec + 1 || ec > 255 {
                    break 'bad_tfm;
                }
                if bc > 255 {
                    // bc=256 and ec=255
                    bc = 1;
                    ec = 0;
                }
                self.fget();
                read_sixteen!(nw, 'bad_tfm);
                self.fget();
                read_sixteen!(nh, 'bad_tfm);
                self.fget();
                read_sixteen!(nd, 'bad_tfm);
                self.fget();
                read_sixteen!(ni, 'bad_tfm);
                self.fget();
                read_sixteen!(nl, 'bad_tfm);
                self.fget();
                read_sixteen!(nk, 'bad_tfm);
                self.fget();
                read_sixteen!(ne, 'bad_tfm);
                self.fget();
                read_sixteen!(np, 'bad_tfm);
                if lf != 6 + lh + (ec - bc + 1) + nw + nh + nd + ni + nl + nk + ne + np {
                    break 'bad_tfm;
                }
                if nw == 0 || nh == 0 || nd == 0 || ni == 0 {
                    break 'bad_tfm;
                }
                // §592 Use size fields to allocate font information
                lf = lf - 6 - lh; // lf words should be loaded into font_info
                if np < 7 {
                    lf = lf + 7 - np; // at least seven parameters will appear
                }
                if self.font_ptr == self.font_max || self.fmem_ptr + lf > self.font_mem_size {
                    // §593 Apologize for not loading the font, goto done
                    self.start_font_error_message(u, nom, aire, s);
                    self.print_s(b" not loaded: Not enough room left");
                    self.help(&[
                        b"I'm afraid I won't be able to make use of this font,",
                        b"because my memory for character-size data is too small.",
                        b"If you're really stuck, ask a wizard to enlarge me.",
                        b"Or maybe try `I\\font<same font id>=<name of loaded font>'.",
                    ]);
                    self.error();
                    break 'done;
                }
                let f = self.font_ptr + 1;
                self.char_base[f] = self.fmem_ptr - bc;
                self.width_base[f] = self.char_base[f] + ec + 1;
                self.height_base[f] = self.width_base[f] + nw;
                self.depth_base[f] = self.height_base[f] + nh;
                self.italic_base[f] = self.depth_base[f] + nd;
                self.lig_kern_base[f] = self.italic_base[f] + ni;
                self.kern_base[f] = self.lig_kern_base[f] + nl - KERN_BASE_OFFSET;
                self.exten_base[f] = self.kern_base[f] + KERN_BASE_OFFSET + nk;
                self.param_base[f] = self.exten_base[f] + ne;
                // §594 Read the TFM header
                if lh < 2 {
                    break 'bad_tfm;
                }
                self.font_check[f] = self.tfm_four_quarters();
                self.fget();
                let mut z;
                read_sixteen!(z, 'bad_tfm); // this rejects a negative design size
                self.fget();
                z = z * 0o400 + self.tfm_temp;
                self.fget();
                z = z * 0o20 + self.tfm_temp / 0o20;
                if z < UNITY {
                    break 'bad_tfm;
                }
                while lh > 2 {
                    self.fget();
                    self.fget();
                    self.fget();
                    self.fget();
                    lh -= 1; // ignore the rest of the header
                }
                self.font_dsize[f] = z;
                if s != -1000 {
                    if s >= 0 {
                        z = s;
                    } else {
                        self.save_arith_error = self.arith_error;
                        let sw = z;
                        z = self.xn_over_d(z, -s, 1000);
                        if self.arith_error || z >= 0o1000000000 {
                            self.start_font_error_message(u, nom, aire, s);
                            self.print_s(b" scaled to 2048pt or higher");
                            self.help(&[b"I will ignore the scaling factor."]);
                            self.error();
                            z = sw;
                        }
                        self.arith_error = self.save_arith_error;
                    }
                }
                self.font_size[f] = z;
                // §595 Read character data
                for k in self.fmem_ptr..self.width_base[f] {
                    let mut qw = self.tfm_four_quarters();
                    self.font_info[k].set_qqqq(qw);
                    let (a, b, c, mut d) = (qw.b0, qw.b1, qw.b2, qw.b3);
                    if a >= nw || b / 0o20 >= nh || b % 0o20 >= nd || c / 4 >= ni {
                        break 'bad_tfm;
                    }
                    match c % 4 {
                        LIG_TAG => {
                            if d >= nl {
                                break 'bad_tfm;
                            }
                        }
                        EXT_TAG => {
                            if d >= ne {
                                break 'bad_tfm;
                            }
                        }
                        LIST_TAG => {
                            // §596 Check for charlist cycle
                            // check_byte_range(d)
                            if d < bc || d > ec {
                                break 'bad_tfm;
                            }
                            let current_character_being_worked_on = k + bc - self.fmem_ptr;
                            'not_found: {
                                while d < current_character_being_worked_on {
                                    qw = self.orig_char_info(f, d);
                                    // N.B.: not qi(d), since char_base[f] hasn't been adjusted yet
                                    if self.char_tag(qw) != LIST_TAG {
                                        break 'not_found;
                                    }
                                    d = self.rem_byte(qw); // next character on the list
                                }
                                if d == current_character_being_worked_on {
                                    break 'bad_tfm; // yes, there's a cycle
                                }
                            }
                        }
                        _ => {} // no_tag
                    }
                }
                // §598 Read box dimensions
                // §599 Replace z by z' and compute alpha, beta
                let mut alpha = 16;
                if z >= 0o1000000000 {
                    self.pdf_error(b"font", b"size is too large");
                }
                while z >= 0o40000000 {
                    z = z / 2;
                    alpha = alpha + alpha;
                }
                let beta = 256 / alpha;
                alpha = alpha * z;
                for k in self.width_base[f]..self.lig_kern_base[f] {
                    let Some(sw) = self.tfm_scaled(z, alpha, beta) else {
                        break 'bad_tfm;
                    };
                    self.font_info[k].set_sc(sw);
                }
                if self.font_info[self.width_base[f]].sc() != 0 {
                    break 'bad_tfm; // width[0] must be zero
                }
                if self.font_info[self.height_base[f]].sc() != 0 {
                    break 'bad_tfm; // height[0] must be zero
                }
                if self.font_info[self.depth_base[f]].sc() != 0 {
                    break 'bad_tfm; // depth[0] must be zero
                }
                if self.font_info[self.italic_base[f]].sc() != 0 {
                    break 'bad_tfm; // italic[0] must be zero
                }
                // §600 Read ligature/kern program
                // check_existence(x)
                macro_rules! check_existence {
                    ($x:expr) => {
                        let x = $x;
                        if x < bc || x > ec {
                            break 'bad_tfm;
                        }
                        let qw = self.orig_char_info(f, x); // N.B.: not qi(x)
                        if !self.char_exists(qw) {
                            break 'bad_tfm;
                        }
                    };
                }
                let mut bch_label = 0o77777;
                let mut bchar = 256;
                if nl > 0 {
                    // a, c and d after the loop are those of the last command.
                    let (mut a, mut c, mut d) = (0, 0, 0);
                    for k in self.lig_kern_base[f]..self.kern_base[f] + KERN_BASE_OFFSET {
                        let qw = self.tfm_four_quarters();
                        self.font_info[k].set_qqqq(qw);
                        let b;
                        (a, b, c, d) = (qw.b0, qw.b1, qw.b2, qw.b3);
                        if a > 128 {
                            if 256 * c + d >= nl {
                                break 'bad_tfm;
                            }
                            if a == 255 && k == self.lig_kern_base[f] {
                                bchar = b;
                            }
                        } else {
                            if b != bchar {
                                check_existence!(b);
                            }
                            if c < 128 {
                                check_existence!(d); // check ligature
                            } else if 256 * (c - 128) + d >= nk {
                                break 'bad_tfm; // check kern
                            }
                            if a < 128 && k - self.lig_kern_base[f] + a + 1 >= nl {
                                break 'bad_tfm;
                            }
                        }
                    }
                    if a == 255 {
                        bch_label = 256 * c + d;
                    }
                }
                for k in self.kern_base[f] + KERN_BASE_OFFSET..self.exten_base[f] {
                    let Some(sw) = self.tfm_scaled(z, alpha, beta) else {
                        break 'bad_tfm;
                    };
                    self.font_info[k].set_sc(sw);
                }
                // §601 Read extensible character recipes
                for k in self.exten_base[f]..self.param_base[f] {
                    let qw = self.tfm_four_quarters();
                    self.font_info[k].set_qqqq(qw);
                    let (a, b, c, d) = (qw.b0, qw.b1, qw.b2, qw.b3);
                    if a != 0 {
                        check_existence!(a);
                    }
                    if b != 0 {
                        check_existence!(b);
                    }
                    if c != 0 {
                        check_existence!(c);
                    }
                    check_existence!(d);
                }
                // §602 Read font parameters
                for k in 1..=np {
                    if k == 1 {
                        // the slant parameter is a pure number
                        self.fget();
                        let mut sw = self.tfm_temp;
                        if sw > 127 {
                            sw = sw - 256;
                        }
                        self.fget();
                        sw = sw * 0o400 + self.tfm_temp;
                        self.fget();
                        sw = sw * 0o400 + self.tfm_temp;
                        self.fget();
                        let v = sw * 0o20 + self.tfm_temp / 0o20;
                        self.font_info[self.param_base[f]].set_sc(v);
                    } else {
                        let Some(sw) = self.tfm_scaled(z, alpha, beta) else {
                            break 'bad_tfm;
                        };
                        self.font_info[self.param_base[f] + k - 1].set_sc(sw);
                    }
                }
                // `feof(tfm_file)`: C's feof, true once a read has failed, so true only if the last read did.
                // `self.feof` is web2c's `eof`, which is already true after the last byte of a good file.
                if self.tfm_temp < 0 {
                    break 'bad_tfm;
                }
                for k in np + 1..=7 {
                    self.font_info[self.param_base[f] + k - 1].set_sc(0);
                }
                // §603 Make final adjustments and goto done
                self.font_params[f] = if np >= 7 { np } else { 7 };
                self.hyphen_char[f] = self.default_hyphen_char();
                self.skew_char[f] = self.default_skew_char();
                self.bchar_label[f] = if bch_label < nl { bch_label + self.lig_kern_base[f] } else { NON_ADDRESS };
                self.font_bchar[f] = bchar;
                self.font_false_bchar[f] = bchar;
                if bchar <= ec && bchar >= bc {
                    let qw = self.orig_char_info(f, bchar); // N.B.: not qi(bchar)
                    if self.char_exists(qw) {
                        self.font_false_bchar[f] = NON_CHAR;
                    }
                }
                self.font_name[f] = nom;
                self.font_area[f] = aire;
                self.font_bc[f] = bc;
                self.font_ec[f] = ec;
                self.font_glue[f] = NULL;
                // adjust(char_base), adjust(width_base), adjust(lig_kern_base), adjust(kern_base),
                // adjust(exten_base): the identity at min_quarterword=0.
                self.param_base[f] -= 1;
                self.fmem_ptr = self.fmem_ptr + lf;
                self.font_ptr = f;
                g = f;
                break 'done;
            }
            // bad_tfm: §587 Report that the font won't be loaded
            self.start_font_error_message(u, nom, aire, s);
            if file_opened {
                self.print_s(b" not loadable: Bad metric (TFM) file");
            } else if name_too_long {
                self.print_s(b" not loadable: Metric (TFM) file name too long");
            } else {
                self.print_s(b" not loadable: Metric (TFM) file not found");
            }
            self.help(&[
                b"I wasn't able to read the size data for this font,",
                b"so I will ignore the font specification.",
                b"[Wizards can fix TFM files using TFtoPL/PLtoTF.]",
                b"You might try inserting a different font spec;",
                b"e.g., type `I\\font<same font id>=<substitute font name>'.",
            ]);
            self.error();
        }
        // done:
        if file_opened {
            self.b_close(TFM_FILE);
        }
        g
    }

    /// §597
    pub fn store_scaled_f(&mut self, sq: i32, z: i32) -> i32 {
        let mut sq = sq;
        let mut z = z;
        let mut alpha = 16;
        if z >= 0o1000000000 {
            self.pdf_error(b"font", b"size is too large");
        }
        while z >= 0o40000000 {
            z = z / 2;
            alpha = alpha + alpha;
        }
        let beta = 256 / alpha;
        alpha = alpha * z;
        let (a, b, c, d);
        if sq >= 0 {
            d = sq % 256;
            sq = sq / 256;
            c = sq % 256;
            sq = sq / 256;
            b = sq % 256;
            sq = sq / 256;
            a = sq % 256;
        } else {
            sq = (sq + 1073741824) + 1073741824; // braces for optimizing compiler
            d = sq % 256;
            sq = sq / 256;
            c = sq % 256;
            sq = sq / 256;
            b = sq % 256;
            sq = sq / 256;
            a = (sq + 128) % 256;
        }
        let sw = (((d * z) / 0o400 + c * z) / 0o400 + b * z) / beta;
        if a == 0 {
            sw
        } else if a == 255 {
            sw - alpha
        } else {
            self.pdf_error(b"store_scaled_f", b"vf scaling")
        }
    }

    /// §604
    pub fn test_no_ligatures(&mut self, f: i32) -> i32 {
        for c in self.font_bc[f]..=self.font_ec[f] {
            if self.char_exists(self.orig_char_info(f, c)) && odd(self.char_tag(self.orig_char_info(f, c))) {
                return 0;
            }
        }
        1
    }

    /// §604
    pub fn get_tag_code(&mut self, f: i32, c: i32) -> i32 {
        if self.is_valid_char(f, c) {
            let i = self.char_tag(self.orig_char_info(f, c));
            if i == LIG_TAG {
                1
            } else if i == LIST_TAG {
                2
            } else if i == EXT_TAG {
                4
            } else {
                0
            }
        } else {
            -1
        }
    }

    /// §604
    pub fn scan_font_ident(&mut self) {
        // §432 Get the next non-blank non-call token
        loop {
            self.get_x_token();
            if self.cur_cmd != SPACER {
                break;
            }
        }
        let f;
        if self.cur_cmd == DEF_FONT || self.cur_cmd == LETTERSPACE_FONT || self.cur_cmd == PDF_COPY_FONT {
            f = self.cur_font();
        } else if self.cur_cmd == SET_FONT {
            f = self.cur_chr;
        } else if self.cur_cmd == DEF_FAMILY {
            let m = self.cur_chr;
            self.scan_four_bit_int();
            f = self.equiv(m + self.cur_val);
        } else {
            self.print_err_s(b"Missing font identifier");
            self.help(&[
                b"I was looking for a control sequence whose",
                b"current meaning has been defined by \\font.",
            ]);
            self.back_error();
            f = NULL_FONT;
        }
        self.cur_val = f;
    }

    /// §605 Sets `cur_val` to a `font_info` location.
    pub fn find_font_dimen(&mut self, writing: bool) {
        self.scan_int();
        let n = self.cur_val;
        self.scan_font_ident();
        let f = self.cur_val;
        if n <= 0 {
            self.cur_val = self.fmem_ptr;
        } else {
            if writing && n <= SPACE_SHRINK_CODE && n >= SPACE_CODE && self.font_glue[f] != NULL {
                self.delete_glue_ref(self.font_glue[f]);
                self.font_glue[f] = NULL;
            }
            if n > self.font_params[f] {
                if f < self.font_ptr {
                    self.cur_val = self.fmem_ptr;
                } else {
                    // §607 Increase the number of parameters in the last font
                    loop {
                        if self.fmem_ptr == self.font_mem_size {
                            self.overflow(b"font memory", self.font_mem_size);
                        }
                        self.font_info[self.fmem_ptr].set_sc(0);
                        self.fmem_ptr += 1;
                        self.font_params[f] += 1;
                        if n == self.font_params[f] {
                            break;
                        }
                    }
                    self.cur_val = self.fmem_ptr - 1; // this equals param_base[f]+font_params[f]
                }
            } else {
                self.cur_val = n + self.param_base[f];
            }
        }
        // §606 Issue an error message if cur_val=fmem_ptr
        if self.cur_val == self.fmem_ptr {
            self.print_err_s(b"Font ");
            self.print_esc(self.font_id_text(f));
            self.print_s(b" has only ");
            self.print_int(self.font_params[f]);
            self.print_s(b" fontdimen parameters");
            self.help(&[
                b"To increase the number of font parameters, you must",
                b"use \\fontdimen immediately after the \\font is loaded.",
            ]);
            self.error();
        }
    }

    /// §608
    pub fn char_warning(&mut self, f: i32, c: i32) {
        if self.tracing_lost_chars() > 0 {
            let old_setting = self.tracing_online(); // saved value of tracing_online
            if self.eTeX_ex() && self.tracing_lost_chars() > 1 {
                self.set_tracing_online(1);
            }
            if self.tracing_lost_chars() > 2 {
                self.print_err_s(b"Missing character: There is no ");
            } else {
                self.begin_diagnostic();
                self.print_nl_s(b"Missing character: There is no ");
            }
            self.print(c); // print_ASCII
            if self.tracing_lost_chars() > 2 {
                self.print_s(b" (");
                self.print_hex(c);
                self.print(b')' as i32);
            }
            self.print_s(b" in font ");
            self.slow_print(self.font_name[f]);
            if self.tracing_lost_chars() < 3 {
                self.print_char(b'!' as i32);
            }
            self.set_tracing_online(old_setting);
            if self.tracing_lost_chars() > 2 {
                self.help(&[]);
                self.error();
            } else {
                self.end_diagnostic(false);
            }
        }
    }

    /// §609
    pub fn new_character(&mut self, f: i32, c: i32) -> i32 {
        let ec = self.effective_char(false, f, c);
        if self.font_bc[f] <= ec && self.font_ec[f] >= ec && self.char_exists(self.orig_char_info(f, ec)) {
            // N.B.: not char_info
            let p = self.get_avail();
            self.set_font(p, f);
            self.set_character(p, c);
            return p;
        }
        self.char_warning(f, c);
        NULL
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testfs;
    use crate::toks::testkit::*;

    /// tests/oracle/gen_tfm_tables.py: what the oracle made of each file.
    const TABLE: &str = include_str!("../testdata/tfm_oracle.tsv");

    const ORDER: &[u8] = b"hcwtdilkep";

    /// The table writes a long run of one letter as {a*255}.
    fn runs(line: &str) -> String {
        let mut out = String::new();
        let mut rest = line;
        while let Some(k) = rest.find('{') {
            let end = rest[k..].find('}').map(|e| k + e);
            let spec = end.and_then(|e| rest[k + 1..e].split_once('*'));
            match (end, spec) {
                (Some(e), Some((c, n))) if c.len() == 1 => {
                    out.push_str(&rest[..k]);
                    out.push_str(&c.repeat(n.parse().unwrap()));
                    rest = &rest[e + 1..];
                }
                _ => {
                    out.push_str(&rest[..=k]);
                    rest = &rest[k + 1..];
                }
            }
        }
        out.push_str(rest);
        out
    }

    /// The same font and the same operations as the generator.
    fn build(ops: &str) -> Vec<u8> {
        let mut sec: Vec<Vec<u32>> = vec![
            vec![0x01020304, 0x00A00000],
            vec![0x01110400, 0x02200101, 0x03100244, 0x01000241, 0x01110300, 0x03210000],
            vec![0x00000000, 0x00080000, 0x00099999, 0x000C0000],
            vec![0x00000000, 0x000B3333, 0x000CCCCC],
            vec![0x00000000, 0x00033333],
            vec![0x00000000, 0x00019999],
            vec![0xFF460002, 0x00418000, 0x80420043, 0xFF000001],
            vec![0x00019999, 0xFFFE6667],
            vec![0x41004243],
            vec![0x00040000, 0x00055555, 0x0002AAAA, 0x0001C71C, 0x0006E147, 0x00100000, 0x00019999],
        ];
        let section = |c: &str| ORDER.iter().position(|&x| x == c.as_bytes()[0]).unwrap();
        let (mut bc, mut ec, mut dlf) = (65i64, 70i64, 0i64);
        let mut head: Vec<(usize, i64)> = Vec::new();
        let (mut cut, mut pad) = (None, 0usize);
        for op in ops.split_whitespace() {
            let (key, val) = op.split_once('=').unwrap();
            if let Some(rest) = key.strip_prefix("set:") {
                let (s, i) = rest.split_once(':').unwrap();
                sec[section(s)][i.parse::<usize>().unwrap()] = u32::from_str_radix(val, 16).unwrap();
            } else if let Some(s) = key.strip_prefix("len:") {
                sec[section(s)].resize(val.parse().unwrap(), 0);
            } else if let Some(f) = key.strip_prefix("head:") {
                head.push((f.parse().unwrap(), val.parse().unwrap()));
            } else {
                let v: i64 = val.parse().unwrap();
                match key {
                    "bc" => bc = v,
                    "ec" => ec = v,
                    "lf" => dlf = v,
                    "cut" => cut = Some(v as usize),
                    "pad" => pad = v as usize,
                    _ => panic!("{op}"),
                }
            }
        }
        let sizes: Vec<i64> = sec.iter().map(|s| s.len() as i64).collect();
        let lf = 6 + sizes.iter().sum::<i64>() + dlf;
        let mut fields = vec![lf, sizes[0], bc, ec];
        fields.extend_from_slice(&sizes[2..]);
        for (f, v) in head {
            fields[f] = v;
        }
        let mut out = Vec::new();
        for v in fields {
            out.extend_from_slice(&((v & 0xFFFF) as u16).to_be_bytes());
        }
        for s in &sec {
            for w in s {
                out.extend_from_slice(&w.to_be_bytes());
            }
        }
        out.resize(out.len() + pad, 0);
        if let Some(n) = cut {
            out.truncate(n);
        }
        out
    }

    fn font_machine() -> &'static mut Tex {
        let t = machine();
        print_setup(t);
        t.stop_at_space = true;
        t.max_strings = 200000;
        t.pool_size = 5000000;
        t.init_str_ptr = t.str_ptr;
        t.max_print_line = 4000; // as the generator: no message is broken across lines
        t.interaction = SCROLL_MODE;
        t.font_max = 9000;
        t.font_mem_size = 8000000;
        t.font_ptr = NULL_FONT;
        t.fmem_ptr = 7;
        t.set_default_hyphen_char(b'-' as i32);
        t.set_default_skew_char(-1);
        t.eTeX_mode = 1;
        t
    }

    fn make_str(t: &mut Tex, s: &[u8]) -> i32 {
        for &c in s {
            t.append_char(c as i32);
        }
        t.make_string()
    }

    /// `\font\x=name`, as new_font calls it.  `name` goes through the file name scanner, and `dir` is put in
    /// front as an area, which the messages then have to lose again.
    fn load(t: &mut Tex, name: &[u8], dir: &[u8], s: i32) -> (i32, String) {
        t.begin_name();
        for &c in name {
            if !t.more_name(c as i32) {
                break;
            }
        }
        t.end_name();
        let nom = t.cur_name;
        let aire = if dir.is_empty() { t.cur_area } else { make_str(t, dir) };
        printed(t);
        t.error_count = 0;
        let u = cs(b'x');
        let f = t.read_font_info(u, nom, aire, s);
        let text = printed(t).replace(std::str::from_utf8(dir).unwrap(), "");
        (f, text)
    }

    fn font_messages(text: &str, prefix: &str) -> Vec<String> {
        text.lines().filter(|l| l.starts_with(prefix)).map(str::to_string).collect()
    }

    fn check_loaded(t: &mut Tex, f: i32, row: &[&str], name: &str) {
        let params: i32 = row[5].parse().unwrap();
        assert_eq!(t.font_params[f], params, "{name}: font_params");
        for (k, d) in row[6].split(',').enumerate() {
            let k = k as i32 + 1;
            if k <= params {
                assert_eq!(t.font_info[t.param_base[f] + k].sc(), d.parse::<i32>().unwrap(), "{name}: fontdimen{k}");
            }
        }
        let mut chars_seen = 0;
        if !row[7].is_empty() {
            for c in row[7].split(';') {
                let (code, m) = c.split_once(':').unwrap();
                let code: i32 = code.parse().unwrap();
                let m: Vec<i32> = m.split(',').map(|x| x.parse().unwrap()).collect();
                let q = t.char_info(f, code);
                assert!(t.char_exists(q), "{name}: char {code} exists");
                let hd = t.height_depth(q);
                let got = [t.char_width(f, q), t.char_height(f, hd), t.char_depth(f, hd), t.char_italic(f, q)];
                assert_eq!(&got[..], &m[..4], "{name}: char {code}");
                assert_eq!(t.get_tag_code(f, code), m[4], "{name}: tag of char {code}");
                chars_seen += 1;
            }
        }
        // Nothing else exists.
        let mut exist = 0;
        for c in t.font_bc[f]..=t.font_ec[f] {
            if t.char_exists(t.orig_char_info(f, c)) {
                exist += 1;
            }
        }
        assert_eq!(exist, chars_seen, "{name}: how many chars");
        assert_eq!(t.test_no_ligatures(f), row[8].parse::<i32>().unwrap(), "{name}: pdfnoligatures");
        assert_eq!(t.get_tag_code(f, 64), row[9].parse::<i32>().unwrap(), "{name}: tagcode of 64");
    }

    /// Every row but the lost characters: the error messages, and for a font that loads what \fontcharwd and
    /// friends report.  Each file is read by the real read_font_info.
    #[test]
    fn read_font_info_against_the_oracle() {
        let dir = testfs::scratch("tfm-oracle");
        let t = font_machine();
        let mut n = 0;
        for line in TABLE.lines() {
            let line = runs(line);
            let row: Vec<&str> = line.split('\t').collect();
            let name = row[0];
            if name.starts_with("lost:") {
                continue;
            }
            n += 1;
            // Each case starts over: only what WEB would have after the fonts before it matters.
            t.font_ptr = NULL_FONT;
            t.fmem_ptr = 7;
            t.font_max = 9000;
            t.font_mem_size = 8000000;
            match name {
                "room_memory" => t.font_mem_size = 20000,
                "room_fonts" => {
                    t.font_max = 50;
                    t.font_ptr = 50;
                }
                _ => {}
            }
            let s: i32 = row[2].parse().unwrap();
            let (f, text) = match row[3].strip_prefix('@') {
                Some(spec) => load(t, spec.as_bytes(), b"", s),
                None => {
                    testfs::write(&dir, &format!("{name}.tfm"), &build(row[3]));
                    load(t, name.as_bytes(), &dir, s)
                }
            };
            let got = font_messages(&text, "! Font \\x=");
            match row[4] {
                "BAD" => {
                    let want: Vec<String> = row[5].split('\x0b').map(str::to_string).collect();
                    assert_eq!(got, want, "{name}");
                    assert_eq!(f, NULL_FONT, "{name}");
                }
                "OK" => {
                    let want: Vec<String> = row[10].split('\x0b').filter(|m| !m.is_empty()).map(str::to_string).collect();
                    assert_eq!(got, want, "{name}");
                    assert_eq!(f, 1, "{name}");
                    check_loaded(t, f, &row, name);
                }
                other => panic!("{name}: {other}"),
            }
        }
        assert!(n > 150, "the table was not read");
        testfs::remove(&dir);
        Tex::dealloc(t);
    }

    /// new_character on the font of the table: a char node, or char_warning's message.
    #[test]
    fn new_character_and_char_warning_against_the_oracle() {
        let dir = testfs::scratch("tfm-lost");
        testfs::write(&dir, "lost.tfm", &build(""));
        let t = font_machine();
        t.mem_end = MEM_TOP;
        t.hi_mem_min = MEM_TOP;
        t.lo_mem_max = MEM_BOT;
        let (f, _) = load(t, b"lost", &dir, -1000);
        assert_eq!(f, 1);
        // The name without its directory, as the oracle had it.
        t.font_name[f] = make_str(t, b"lost");
        let mut n = 0;
        for line in TABLE.lines() {
            let row: Vec<&str> = line.split('\t').collect();
            let Some(spec) = row[0].strip_prefix("lost:") else {
                continue;
            };
            n += 1;
            let (level, c) = spec.split_once(':').unwrap();
            let (level, c): (i32, i32) = (level.parse().unwrap(), c.parse().unwrap());
            t.set_tracing_lost_chars(level);
            t.set_tracing_online(0);
            t.error_count = 0;
            printed(t);
            let p = t.new_character(f, c);
            let text = printed(t);
            let got: Vec<&str> = text.lines().filter(|l| l.contains("Missing character")).collect();
            let want: Vec<&str> = row[5].split('\x0b').filter(|m| !m.is_empty()).collect();
            assert_eq!(got, want, "level {level} char {c}");
            assert_eq!(t.tracing_online(), 0, "tracing_online comes back");
            if c == 65 {
                assert!(t.is_char_node(p));
                assert_eq!((t.font(p), t.character(p)), (f, c));
            } else {
                assert_eq!(p, NULL);
            }
        }
        assert_eq!(n, 20);
        testfs::remove(&dir);
        Tex::dealloc(t);
    }

    fn loaded(ops: &str, name: &str) -> (&'static mut Tex, i32) {
        let dir = testfs::scratch(&format!("tfm-{name}"));
        testfs::write(&dir, "f.tfm", &build(ops));
        let t = font_machine();
        let (f, _) = load(t, b"f", &dir, -1000);
        assert_eq!(f, 1, "{name}");
        testfs::remove(&dir);
        (t, f)
    }

    /// What the oracle has no primitive to show: the tables as they are stored.
    #[test]
    fn the_program_and_the_boundary_chars_are_stored() {
        let (t, f) = loaded("", "stored");
        assert_eq!(t.font_check[f], FourQuarters { b0: 1, b1: 2, b2: 3, b3: 4 });
        assert_eq!(t.font_dsize[f], 10 * UNITY);
        assert_eq!(t.font_size[f], 10 * UNITY);
        assert_eq!((t.font_bc[f], t.font_ec[f]), (65, 70));
        // The first command has a=255, so its b is the right boundary char, which exists.
        assert_eq!(t.font_bchar[f], 70);
        assert_eq!(t.font_false_bchar[f], NON_CHAR);
        // The last has a=255: the left boundary program starts at 256c+d.
        assert_eq!(t.bchar_label[f], t.lig_kern_base[f] + 1);
        assert_eq!(t.font_info[t.lig_kern_base[f] + 2].qqqq(), FourQuarters { b0: 0x80, b1: 0x42, b2: 0, b3: 0x43 });
        assert_eq!(t.font_info[t.exten_base[f]].qqqq(), FourQuarters { b0: 0x41, b1: 0, b2: 0x42, b3: 0x43 });
        // kern_base and the words after it: a kern's fix_word in design-size units.
        assert_eq!(t.kern_base[f] + KERN_BASE_OFFSET, t.lig_kern_base[f] + 4);
        assert_eq!(t.font_info[t.kern_base[f] + KERN_BASE_OFFSET].sc(), 65535);
        assert_eq!(t.font_info[t.kern_base[f] + KERN_BASE_OFFSET + 1].sc(), -65536);
        assert_eq!(t.font_glue[f], NULL);
        assert_eq!(t.hyphen_char[f], b'-' as i32);
        assert_eq!(t.skew_char[f], -1);
        // Six chars, the 4+3+2+2 box dimensions, 4 commands, 2 kerns, one recipe and 7 params.
        assert_eq!(t.fmem_ptr, 7 + 6 + 11 + 4 + 2 + 1 + 7);
        assert_eq!(t.font_ptr, f);
        assert_eq!(t.param_base[f] + 7, t.fmem_ptr - 1);
        Tex::dealloc(t);
    }

    #[test]
    fn the_boundary_cases_of_the_final_adjustments() {
        // Right boundary char outside the font: it stays as both bchar and false bchar.
        let (t, f) = loaded("set:l:0=FF500002 set:l:1=00508000", "bchar_out");
        assert_eq!((t.font_bchar[f], t.font_false_bchar[f]), (0x50, 0x50));
        Tex::dealloc(t);
        // In range, but not a char.
        let (t, f) = loaded("set:c:5=00000000 set:l:1=00468000", "bchar_missing");
        assert_eq!((t.font_bchar[f], t.font_false_bchar[f]), (70, 70));
        Tex::dealloc(t);
        // No boundary char, and no left boundary program.
        let (t, f) = loaded("set:l:0=80460043 set:l:3=80410043", "no_boundary");
        assert_eq!((t.font_bchar[f], t.font_false_bchar[f]), (NON_CHAR, NON_CHAR));
        assert_eq!(t.bchar_label[f], NON_ADDRESS);
        Tex::dealloc(t);
        // Fewer than 7 params: room for 7, read as zero.  More are kept.
        let (t, f) = loaded("len:p=2", "np2");
        assert_eq!(t.font_params[f], 7);
        assert_eq!(t.font_info[t.param_base[f] + 3].sc(), 0);
        assert_eq!(t.font_info[t.param_base[f] + 7].sc(), 0);
        assert_eq!(t.param_base[f] + 7, t.fmem_ptr - 1);
        Tex::dealloc(t);
        let (t, f) = loaded("len:p=9", "np9");
        assert_eq!(t.font_params[f], 9);
        assert_eq!(t.param_base[f] + 9, t.fmem_ptr - 1);
        Tex::dealloc(t);
        // bc=256, ec=255 is the empty font, stored as 1 and 0.
        let (t, f) = loaded("bc=256 ec=255 len:c=0 len:l=0 len:e=0", "empty");
        assert_eq!((t.font_bc[f], t.font_ec[f]), (1, 0));
        Tex::dealloc(t);
    }

    /// A second font follows the first, and a file with a bad end leaves nothing behind but what it wrote.
    #[test]
    fn a_failed_load_changes_no_counts() {
        let dir = testfs::scratch("tfm-counts");
        testfs::write(&dir, "good.tfm", &build(""));
        testfs::write(&dir, "bad.tfm", &build("cut=155"));
        let t = font_machine();
        let d = dir.clone();
        let (f, _) = load(t, b"good", &d, -1000);
        assert_eq!(f, 1);
        let (p, m) = (t.font_ptr, t.fmem_ptr);
        let (g, text) = load(t, b"bad", &d, -1000);
        assert_eq!(g, NULL_FONT);
        assert!(text.contains("Bad metric"));
        assert_eq!((t.font_ptr, t.fmem_ptr), (p, m));
        let (h, _) = load(t, b"good", &d, 5 * UNITY);
        assert_eq!(h, 2);
        assert_eq!(t.char_base[2], m - 65);
        assert_eq!(t.font_size[2], 5 * UNITY);
        assert_eq!(t.font_dsize[2], 10 * UNITY);
        // An open file does not stay open.
        assert!(matches!(t.host.files[TFM_FILE as usize], Slot::Closed));
        testfs::remove(&dir);
        Tex::dealloc(t);
    }

    /// A fix_word scaled as in VF files.  The values come from `store_scaled` itself on the same word, whose results
    /// the table above checks against the oracle.
    #[test]
    fn store_scaled_f_agrees_with_read_font_info() {
        let t = font_machine();
        let sizes = [UNITY, 10 * UNITY, 127 * UNITY, 128 * UNITY, 1000 * UNITY, 134217727];
        let words: [u32; 7] = [0, 1, 0x00100000, 0x00FFFFFF, 0xFFFFFFFF, 0xFF000000, 0xFFF00000];
        for z in sizes {
            let (mut zz, mut alpha) = (z, 16);
            while zz >= 0o40000000 {
                zz /= 2;
                alpha += alpha;
            }
            let beta = 256 / alpha;
            let alpha = alpha * zz;
            for w in words {
                let [a, b, c, d] = w.to_be_bytes().map(|x| x as i32);
                let sw = (((d * zz) / 0o400 + c * zz) / 0o400 + b * zz) / beta;
                let want = if a == 0 { sw } else { sw - alpha };
                assert_eq!(t.store_scaled_f(w as i32, z), want, "z={z} word={w:x}");
            }
        }
        Tex::dealloc(t);
    }
}
