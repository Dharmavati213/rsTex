//! §1510-1519 Parts 51-52: the main program, debugging.
//!
//! `main_body` is §1512 and §1517 literally, in WEB's order: `initialize` runs on every run, and a
//! format load happens in the middle of §1517.  docs/PORTING.md, notes on `image.rs` and `main.rs`.
//! `init_prim` and §1649 are generated from the tangled WEB, one line per `primitive`.
//!
//! What web2c's texmfmp.c does before `main_body` is in `crates/rstex/src/main.rs`.
//!
//! DROPPED §1518-1519: `debug_help` is `debug` only, not compiled in the oracle.
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
    pub struct Vars {
        /// utils.c `pdftexbanner_init`: `make_pdftex_banner` makes its string once per run.
        pub pdftex_banner_init: bool,
    }
}

/// Non-POD state private to this module, in `self.host`: what texmfmp.c keeps in its statics and the
/// command line sets.  `crates/rstex/src/main.rs` fills it before `main_body`.
#[derive(Default)]
pub struct HVars {
    /// `dump_name`, §32: the format's name, for §61's ` (preloaded format=...)`.
    pub dump_name: Vec<u8>,
}

/// §1514 prints `hash_size`, 15000 in the oracle.  It is 0 here, every control sequence being above
/// `eqtb_size`, and the line has to read the same.  docs/PORTING.md, notes on `hash.rs`.
const ORACLE_HASH_SIZE: i32 = 15000;

/// §1512 `const_chk`, with what `state.rs` can hold: the capacities are fixed at compile time.
fn main_const_chk(x: i32, inf: i32, sup: i32, cap: usize) -> i32 {
    let x = if x < inf {
        inf
    } else if x > sup {
        sup
    } else {
        x
    };
    x.min(cap as i32 - 1)
}

impl Tex {
    /// §1512
    pub fn main_body(&mut self) {
        // Bounds that may be set from the configuration file.  `mem_bot`, `mem_top`, `mem_min` and
        // `mem_max` are constants, so §1512's lines on them have no Rust, and `main_memory` is read but
        // limits nothing: `mem` has its capacity.  `xmalloc_array` has no Rust either: the arrays are inline.
        self.main_memory = rstex_kpse::setup_bound_var(b"main_memory", 250000);
        self.extra_mem_top = rstex_kpse::setup_bound_var(b"extra_mem_top", 0);
        self.extra_mem_bot = rstex_kpse::setup_bound_var(b"extra_mem_bot", 0);
        self.pool_size = rstex_kpse::setup_bound_var(b"pool_size", 200000);
        self.string_vacancies = rstex_kpse::setup_bound_var(b"string_vacancies", 75000);
        self.pool_free = rstex_kpse::setup_bound_var(b"pool_free", 5000); // min pool avail after fmt
        self.max_strings = rstex_kpse::setup_bound_var(b"max_strings", 15000);
        self.strings_free = rstex_kpse::setup_bound_var(b"strings_free", 100);
        self.font_mem_size = rstex_kpse::setup_bound_var(b"font_mem_size", 100000);
        self.font_max = rstex_kpse::setup_bound_var(b"font_max", 500);
        self.trie_size = rstex_kpse::setup_bound_var(b"trie_size", 20000);
        self.hyph_size = rstex_kpse::setup_bound_var(b"hyph_size", 659);
        self.buf_size = rstex_kpse::setup_bound_var(b"buf_size", 3000);
        self.nest_size = rstex_kpse::setup_bound_var(b"nest_size", 50);
        self.max_in_open = rstex_kpse::setup_bound_var(b"max_in_open", 15);
        self.param_size = rstex_kpse::setup_bound_var(b"param_size", 60);
        self.save_size = rstex_kpse::setup_bound_var(b"save_size", 4000);
        self.stack_size = rstex_kpse::setup_bound_var(b"stack_size", 300);
        self.dvi_buf_size = rstex_kpse::setup_bound_var(b"dvi_buf_size", 16384);
        self.error_line = rstex_kpse::setup_bound_var(b"error_line", 79);
        self.half_error_line = rstex_kpse::setup_bound_var(b"half_error_line", 50);
        self.max_print_line = rstex_kpse::setup_bound_var(b"max_print_line", 79);
        self.hash_extra = rstex_kpse::setup_bound_var(b"hash_extra", 0);
        self.expand_depth = rstex_kpse::setup_bound_var(b"expand_depth", 10000);
        self.pk_dpi = rstex_kpse::setup_bound_var(b"pk_dpi", 72);
        // Said on stderr, which is no part of what a checkpoint brings back.
        self.host.world.print_kpse_warnings();
        self.main_memory = main_const_chk(self.main_memory, INF_MAIN_MEMORY, SUP_MAIN_MEMORY, MEM_CAP);
        if self.ini_version {
            self.extra_mem_top = 0;
            self.extra_mem_bot = 0;
        }
        if self.extra_mem_bot > SUP_MAIN_MEMORY {
            self.extra_mem_bot = SUP_MAIN_MEMORY;
        }
        if self.extra_mem_top > SUP_MAIN_MEMORY {
            self.extra_mem_top = SUP_MAIN_MEMORY;
        }
        // Check other constants against their sup and inf.
        self.trie_size = main_const_chk(self.trie_size, INF_TRIE_SIZE, SUP_TRIE_SIZE, TRIE_CAP);
        self.hyph_size = main_const_chk(self.hyph_size, INF_HYPH_SIZE, SUP_HYPH_SIZE, HYPH_CAP);
        self.buf_size = main_const_chk(self.buf_size, INF_BUF_SIZE, SUP_BUF_SIZE, BUF_CAP);
        self.nest_size = main_const_chk(self.nest_size, INF_NEST_SIZE, SUP_NEST_SIZE, NEST_CAP);
        self.max_in_open = main_const_chk(self.max_in_open, INF_MAX_IN_OPEN, SUP_MAX_IN_OPEN, MAX_IN_OPEN_CAP);
        self.param_size = main_const_chk(self.param_size, INF_PARAM_SIZE, SUP_PARAM_SIZE, PARAM_CAP);
        self.save_size = main_const_chk(self.save_size, INF_SAVE_SIZE, SUP_SAVE_SIZE, SAVE_CAP);
        self.stack_size = main_const_chk(self.stack_size, INF_STACK_SIZE, SUP_STACK_SIZE, STACK_CAP);
        self.dvi_buf_size = main_const_chk(self.dvi_buf_size, INF_DVI_BUF_SIZE, SUP_DVI_BUF_SIZE, DVI_BUF_CAP);
        self.pool_size = main_const_chk(self.pool_size, INF_POOL_SIZE, SUP_POOL_SIZE, POOL_CAP);
        self.string_vacancies = main_const_chk(self.string_vacancies, INF_STRING_VACANCIES, SUP_STRING_VACANCIES, POOL_CAP);
        self.pool_free = main_const_chk(self.pool_free, INF_POOL_FREE, SUP_POOL_FREE, POOL_CAP);
        self.max_strings = main_const_chk(self.max_strings, INF_MAX_STRINGS, SUP_MAX_STRINGS, STR_CAP - 1);
        self.strings_free = main_const_chk(self.strings_free, INF_STRINGS_FREE, SUP_STRINGS_FREE, STR_CAP - 1);
        self.font_mem_size = main_const_chk(self.font_mem_size, INF_FONT_MEM_SIZE, SUP_FONT_MEM_SIZE, FONT_MEM_CAP);
        self.font_max = main_const_chk(self.font_max, INF_FONT_MAX, SUP_FONT_MAX, FONT_CAP);
        self.hash_extra = main_const_chk(self.hash_extra, INF_HASH_EXTRA, SUP_HASH_EXTRA, i32::MAX as usize);
        self.obj_tab_size = main_const_chk(self.obj_tab_size, INF_OBJ_TAB_SIZE, SUP_OBJ_TAB_SIZE, OBJ_TAB_CAP);
        self.pdf_mem_size = main_const_chk(self.pdf_mem_size, INF_PDF_MEM_SIZE, SUP_PDF_MEM_SIZE, PDF_MEM_CAP);
        self.dest_names_size = main_const_chk(self.dest_names_size, INF_DEST_NAMES_SIZE, SUP_DEST_NAMES_SIZE, DEST_NAMES_CAP);
        self.pk_dpi = main_const_chk(self.pk_dpi, INF_PK_DPI, SUP_PK_DPI, i32::MAX as usize);
        if self.error_line > SSUP_ERROR_LINE {
            self.error_line = SSUP_ERROR_LINE;
        }

        // Every control sequence made at run time is above eqtb_size, whatever hash_extra says, and
        // the `next` and `text` arrays start as zero.  docs/PORTING.md, notes on `main.rs`, `hash.rs`.
        self.eqtb_top = EQTB_CAP as i32 - 1;
        self.hash_top = self.eqtb_top;

        self.history = FATAL_ERROR_STOP; // in case we quit during initialization
        if self.ready_already != 314159 {
            // `Check the "constant" values` is not ported: docs/PORTING.md section 8.
            self.initialize(); // set global variables to their starting values
            crate::prof_phase!("initialize");
            if self.ini_version {
                if !self.get_strings_started() {
                    self.do_final_end();
                }
                self.init_prim(); // call `primitive` for each primitive
                self.init_str_ptr = self.str_ptr;
                self.init_pool_ptr = self.pool_ptr;
                self.fix_date_and_time();
            }
            self.ready_already = 314159;
        }
        // start_of_TEX:
        // §55 Initialize the output routines
        self.selector = TERM_ONLY;
        self.tally = 0;
        self.term_offset = 0;
        self.file_offset = 0;
        // §61: banner_k is the same string as banner, and src_specials_p is always false.
        self.wterm_s(BANNER);
        self.wterm_s(VERSION_STRING);
        if self.format_ident == 0 {
            self.wterm_s(b" (preloaded format=");
            let name = core::mem::take(&mut self.host.hv.main.dump_name);
            self.wterm_s(&name);
            self.host.hv.main.dump_name = name;
            self.wterm_ln_s(b")");
        } else {
            self.slow_print(self.format_ident);
            self.print_ln();
        }
        if self.shellenabledp != 0 {
            self.wterm_s(b" ");
            if self.restrictedshell != 0 {
                self.wterm_s(b"restricted ");
            }
            self.wterm_ln_s(b"\\write18 enabled.");
        }
        if let Some(name) = self.host.translate_filename.take() {
            self.wterm_s(b" (");
            self.wterm_s(&name);
            self.wterm_ln_s(b")");
            self.host.translate_filename = Some(name);
        }
        self.update_terminal();
        // §554
        self.job_name = 0;
        self.name_in_progress = false;
        self.log_opened = false;
        // §559
        self.output_file_name = 0;
        // §1517 Get the first line of input and prepare to start
        // §353 Initialize the input routines
        self.input_ptr = 0;
        self.max_in_stack = 0;
        self.source_filename_stack[0] = 0;
        self.full_source_filename_stack[0] = 0;
        self.in_open = 0;
        self.open_parens = 0;
        self.max_buf_stack = 0;
        self.grp_stack[0] = 0;
        self.if_stack[0] = NULL;
        self.param_ptr = 0;
        self.max_param_stack = 0;
        // The loop that sets buffer[1..=buf_size] to 0 has no Rust: nothing has written there, in a new image, whose pages are zero until touched.
        self.first = 0;
        self.scanner_status = NORMAL;
        self.warning_index = NULL;
        self.first = 1;
        self.set_state(NEW_LINE);
        self.set_start(1);
        self.set_index(0);
        self.line = 0;
        self.set_name(0);
        self.force_eof = false;
        self.align_state = 1000000;
        if !self.init_terminal() {
            self.do_final_end();
        }
        self.set_limit(self.last);
        self.first = self.last + 1; // init_terminal has set loc and last
        // §1648 Enable eTeX, if requested
        if (self.etex_p || self.buffer.at(self.loc()) == b'*' as i32) && self.format_ident == S_INITEX_IDENT {
            self.no_new_control_sequence = false;
            self.main_etex_primitives();
            if self.buffer.at(self.loc()) == b'*' as i32 {
                self.set_loc(self.loc() + 1);
            }
            self.eTeX_mode = 1; // enter extended mode
            // §1813 Initialize variables for eTeX extended mode
            self.max_reg_num = 32767;
            self.max_reg_help_line = 1; // "A register number must be between 0 and 32767."
        }
        if !self.no_new_control_sequence {
            // just entered extended mode: §1648's `else` is on the next statement
            self.no_new_control_sequence = true;
        } else if self.format_ident == 0 || self.buffer.at(self.loc()) == b'&' as i32 || self.dump_line {
            if self.format_ident != 0 {
                self.initialize(); // erase preloaded format
            }
            if !self.open_fmt_file() {
                self.do_final_end();
            }
            crate::prof_phase!("format read");
            if !self.load_fmt_file() {
                self.w_close(FMT_FILE);
                self.do_final_end();
            }
            self.w_close(FMT_FILE);
            crate::prof_phase!("format load");
            crate::prof_after_format!(self);
            self.checkpoint_arm();
            while self.loc() < self.limit() && self.buffer.at(self.loc()) == b' ' as i32 {
                self.set_loc(self.loc() + 1);
            }
        }
        if self.pdf_output_option != 0 {
            self.set_pdf_output(self.pdf_output_value);
        }
        if self.pdf_draftmode_option != 0 {
            self.set_pdf_draftmode(self.pdf_draftmode_value);
        }
        self.pdf_init_map_file(b"pdftex.map");
        if self.eTeX_ex() {
            self.wterm_ln_s(b"entering extended mode");
        }
        if self.end_line_char_inactive() {
            self.set_limit(self.limit() - 1);
        } else {
            let c = self.end_line_char();
            self.buffer.put(self.limit(), c);
        }
        // dropped: MLTeX, `if mltex_enabled_p then wterm_ln('MLTeX v2.2 enabled')`
        self.fix_date_and_time();
        if self.trie_not_ready {
            // initex without format loaded
            self.set_trie_root(0);
            self.trie_c.put(0, 0);
            self.trie_ptr = 0;
            self.set_hyph_root(0);
            self.hyph_start = 0;
            // Allocate and initialize font arrays
            self.main_init_pdf_font_arrays();
            self.font_ptr = NULL_FONT;
            self.fmem_ptr = 7;
            self.make_pdftex_banner();
            self.font_name[NULL_FONT] = S_NULLFONT;
            self.font_area[NULL_FONT] = S_EMPTY;
            self.hyphen_char[NULL_FONT] = b'-' as i32;
            self.skew_char[NULL_FONT] = -1;
            self.bchar_label[NULL_FONT] = NON_ADDRESS;
            self.font_bchar[NULL_FONT] = NON_CHAR;
            self.font_false_bchar[NULL_FONT] = NON_CHAR;
            self.font_bc[NULL_FONT] = 1;
            self.font_ec[NULL_FONT] = 0;
            self.font_size[NULL_FONT] = 0;
            self.font_dsize[NULL_FONT] = 0;
            self.char_base[NULL_FONT] = 0;
            self.width_base[NULL_FONT] = 0;
            self.height_base[NULL_FONT] = 0;
            self.depth_base[NULL_FONT] = 0;
            self.italic_base[NULL_FONT] = 0;
            self.lig_kern_base[NULL_FONT] = 0;
            self.kern_base[NULL_FONT] = 0;
            self.exten_base[NULL_FONT] = 0;
            self.font_glue[NULL_FONT] = NULL;
            self.font_params[NULL_FONT] = 7;
            self.param_base[NULL_FONT] = -1;
            self.font_k = 0;
            while self.font_k <= 6 {
                self.font_info[self.font_k].set_sc(0);
                self.font_k += 1;
            }
        }
        // The loop that sets font_used[font_base..=font_max] to false has no Rust, as with buffer above.
        self.random_seed = (self.microseconds * 1000) + (self.epochseconds % 1000000);
        self.init_randoms(self.random_seed);
        // §941 Compute the magic offset.  `math_spacing` is the constant MATH_SPACING and not a
        // string in the pool, so the offset is an offset into that: docs/PORTING.md section 7.
        self.magic_offset = -9 * ORD_NOAD;
        // §75 Initialize the print selector based on interaction
        if self.interaction == BATCH_MODE {
            self.selector = NO_PRINT;
        } else {
            self.selector = TERM_ONLY;
        }
        if self.loc() < self.limit() && self.cat_code(self.buffer.at(self.loc())) != ESCAPE {
            self.start_input(); // \input assumed
        }
        self.history = SPOTLESS; // ready to go!
        // §1904 Initialize synctex primitive
        self.synctex_init_command();
        crate::prof_phase!("main_control entered");
        self.main_control(); // come to life
        crate::prof_phase!("main_control left");
        self.final_cleanup(); // prepare for death
        self.close_files_and_terminate();
        // final_end:
        self.do_final_end();
    }

    /// §1517, and §1501 in `load_fmt_file`: the same loop over `font_base..font_max`.  WEB's values for `pdf_char_used`, `pdf_font_size`, `pdf_font_num`,
    /// `pdf_font_map`, `pdf_font_type`, `pdf_font_blink`, `pdf_font_elink`, `pdf_font_has_space_char`, `pdf_font_stretch`, `pdf_font_shrink`, `pdf_font_step`,
    /// `pdf_font_expand_ratio`, `pdf_font_auto_expand`, `pdf_font_lp_base`, `pdf_font_rp_base`, `pdf_font_ef_base`, `pdf_font_kn_bs_base`,
    /// `pdf_font_st_bs_base`, `pdf_font_sh_bs_base`, `pdf_font_kn_bc_base`, `pdf_font_kn_ac_base` and `pdf_font_nobuiltin_tounicode` are all zero, which is what a new
    /// image holds without touching 300 pages.  Only `pdf_font_attr` is not, and it is set here for the fonts that exist, and where a font is made for the rest.
    pub fn main_init_pdf_font_arrays(&mut self) {
        for f in FONT_BASE..=self.font_ptr {
            self.pdf_font_attr[f] = S_EMPTY;
        }
    }

    /// `wlog(n:1)`, §1514.  At the end of a run, once.
    fn main_wlog_int(&mut self, n: impl Into<i64>) {
        let s = n.into().to_string();
        self.wlog_s(s.as_bytes());
    }

    /// §1513
    pub fn close_files_and_terminate(&mut self) {
        let (mut a, mut b, mut c, mut i, mut j, mut k, mut l): (i32, i32, i32, i32, i32, i32, i32);
        // §1626 Finish the extensions
        for k in 0..=15 {
            if self.write_open[k] {
                self.a_close(write_file(k));
            }
        }
        self.set_new_line_char(-1);
        if self.tracing_stats() > 0 {
            // §1514 Output statistics about this job
            if self.log_opened {
                self.wlog_ln_s(b" ");
                self.wlog_ln_s(b"Here is how much of TeX's memory you used:");
                self.wlog_s(b" ");
                self.main_wlog_int(self.str_ptr - self.init_str_ptr);
                self.wlog_s(b" string");
                if self.str_ptr != self.init_str_ptr + 1 {
                    self.wlog_s(b"s");
                }
                self.wlog_s(b" out of ");
                self.main_wlog_int(self.max_strings - self.init_str_ptr);
                self.wlog_cr();
                self.wlog_s(b" ");
                self.main_wlog_int(self.pool_ptr - self.init_pool_ptr);
                self.wlog_s(b" string characters out of ");
                self.main_wlog_int(self.pool_size - self.init_pool_ptr);
                self.wlog_cr();
                self.wlog_s(b" ");
                self.main_wlog_int(self.lo_mem_max - MEM_MIN + self.mem_end - self.hi_mem_min + 2);
                self.wlog_s(b" words of memory out of ");
                self.main_wlog_int(self.mem_end + 1 - MEM_MIN);
                self.wlog_cr();
                self.wlog_s(b" ");
                self.main_wlog_int(self.cs_count);
                self.wlog_s(b" multiletter control sequences out of ");
                self.main_wlog_int(ORACLE_HASH_SIZE);
                self.wlog_s(b"+");
                self.main_wlog_int(self.hash_extra);
                self.wlog_cr();
                self.wlog_s(b" ");
                self.main_wlog_int(self.fmem_ptr);
                self.wlog_s(b" words of font info for ");
                self.main_wlog_int(self.font_ptr - FONT_BASE);
                self.wlog_s(b" font");
                if self.font_ptr != FONT_BASE + 1 {
                    self.wlog_s(b"s");
                }
                self.wlog_s(b", out of ");
                self.main_wlog_int(self.font_mem_size);
                self.wlog_s(b" for ");
                self.main_wlog_int(self.font_max - FONT_BASE);
                self.wlog_cr();
                self.wlog_s(b" ");
                self.main_wlog_int(self.hyph_count);
                self.wlog_s(b" hyphenation exception");
                if self.hyph_count != 1 {
                    self.wlog_s(b"s");
                }
                self.wlog_s(b" out of ");
                self.main_wlog_int(self.hyph_size);
                self.wlog_cr();
                self.wlog_s(b" ");
                self.main_wlog_int(self.max_in_stack);
                self.wlog_s(b"i,");
                self.main_wlog_int(self.max_nest_stack);
                self.wlog_s(b"n,");
                self.main_wlog_int(self.max_param_stack);
                self.wlog_s(b"p,");
                self.main_wlog_int(self.max_buf_stack + 1);
                self.wlog_s(b"b,");
                self.main_wlog_int(self.max_save_stack + 6);
                self.wlog_s(b"s stack positions out of ");
                self.main_wlog_int(self.stack_size);
                self.wlog_s(b"i,");
                self.main_wlog_int(self.nest_size);
                self.wlog_s(b"n,");
                self.main_wlog_int(self.param_size);
                self.wlog_s(b"p,");
                self.main_wlog_int(self.buf_size);
                self.wlog_s(b"b,");
                self.main_wlog_int(self.save_size);
                self.wlog_s(b"s");
                self.wlog_cr();
            }
        }
        self.wake_up_terminal();
        if !self.fixed_pdfoutput_set {
            self.fix_pdfoutput();
        }
        if self.fixed_pdfoutput > 0 {
            if self.history == FATAL_ERROR_STOP {
                self.remove_pdffile();
                self.synctex_abort(self.log_opened);
                self.print_err_s(b" ==> Fatal error occurred, no output PDF file produced!");
            } else {
                // §794 Finish the PDF file
                if self.total_pages == 0 {
                    self.print_nl_s(b"No pages of output.");
                    if self.pdf_gone > 0 {
                        self.garbage_warning();
                    }
                } else {
                    if self.fixed_pdf_draftmode == 0 {
                        self.pdf_flush(); // to make sure that the output file name has been already created
                        if self.total_pages % PAGES_TREE_KIDS_MAX != 0 {
                            self.set_obj_info(self.pdf_last_pages, self.total_pages % PAGES_TREE_KIDS_MAX);
                        }
                        // last pages object may have less than pages_tree_kids_max children
                        self.flush_jbig2_page0_objects(); // flush page 0 objects from JBIG2 images, if any
                        // §799 Check for non-existing pages
                        k = self.head_tab[OBJ_TYPE_PAGE];
                        while self.obj_aux(k) == 0 {
                            self.pdf_warning(b"dest", b"Page ", true, false);
                            self.print_int(self.obj_info(k));
                            self.print_s(b" has been referenced but does not exist!");
                            self.print_ln();
                            self.print_ln();
                            k = self.obj_link(k);
                        }
                        self.head_tab[OBJ_TYPE_PAGE] = k;
                        // §800 Reverse the linked list of Page and Pages objects
                        k = self.head_tab[OBJ_TYPE_PAGE];
                        l = 0;
                        loop {
                            i = self.obj_link(k);
                            self.set_obj_link(k, l);
                            l = k;
                            k = i;
                            if k == 0 {
                                break;
                            }
                        }
                        self.head_tab[OBJ_TYPE_PAGE] = l;
                        k = self.head_tab[OBJ_TYPE_PAGES];
                        self.pages_tail = k;
                        l = 0;
                        loop {
                            i = self.obj_link(k);
                            self.set_obj_link(k, l);
                            l = k;
                            k = i;
                            if k == 0 {
                                break;
                            }
                        }
                        self.head_tab[OBJ_TYPE_PAGES] = l;
                        // §796 Check for non-existing destinations
                        k = self.head_tab[OBJ_TYPE_DEST];
                        while k != 0 {
                            self.pdf_fix_dest(k);
                            k = self.obj_link(k);
                        }
                        // §798 Check for non-existing structure destinations
                        k = self.head_tab[OBJ_TYPE_STRUCT_DEST];
                        while k != 0 {
                            self.pdf_fix_struct_dest(k);
                            k = self.obj_link(k);
                        }
                        // §801 Output fonts definition
                        for k in FONT_BASE + 1..=self.font_ptr {
                            if self.font_used[k] && self.hasfmentry(k) && self.pdf_font_num[k] < 0 {
                                let i = -self.pdf_font_num[k];
                                let ok = self.pdf_font_num[i] > 0;
                                self.pdfassert(ok);
                                for j in 0..=255 {
                                    if self.pdf_char_marked(k, j) {
                                        self.pdf_mark_char(i, j);
                                    }
                                }
                                if self.length(self.pdf_font_attr[i]) == 0 && self.length(self.pdf_font_attr[k]) != 0 {
                                    self.pdf_font_attr[i] = self.pdf_font_attr[k];
                                } else if self.length(self.pdf_font_attr[k]) == 0 && self.length(self.pdf_font_attr[i]) != 0 {
                                    self.pdf_font_attr[k] = self.pdf_font_attr[i];
                                } else if self.length(self.pdf_font_attr[i]) != 0
                                    && self.length(self.pdf_font_attr[k]) != 0
                                    && !self.str_eq_str(self.pdf_font_attr[i], self.pdf_font_attr[k])
                                {
                                    self.pdf_warning(b"\\pdffontattr", b"fonts ", true, false);
                                    self.print_font_identifier(i);
                                    self.print_s(b" and ");
                                    self.print_font_identifier(k);
                                    self.print_s(b" have conflicting attributes; I will ignore the attributes assigned to ");
                                    self.print_font_identifier(i);
                                    self.print_ln();
                                    self.print_ln();
                                }
                            }
                        }
                        self.fixed_gen_tounicode = self.pdf_gen_tounicode();
                        k = self.head_tab[OBJ_TYPE_FONT];
                        while k != 0 {
                            self.f = self.obj_info(k);
                            let ok = self.pdf_font_num[self.f] > 0;
                            self.pdfassert(ok);
                            self.do_pdf_font(k, self.f);
                            k = self.obj_link(k);
                        }
                        self.write_fontstuff();
                        crate::prof_phase!("fonts written");
                        // §802 Output pages tree
                        a = self.sys_obj_ptr + 1; // all Pages objects whose children are not Page objects should have index greater than a
                        l = self.head_tab[OBJ_TYPE_PAGES]; // the index of current Pages object which is being output
                        k = self.head_tab[OBJ_TYPE_PAGE]; // the index of current child of l
                        b = 0;
                        loop {
                            i = 0; // counter of Pages object in current level
                            c = 0; // first Pages object in previous level
                            // the only Pages object: total pages is not greater than pages_tree_kids_max
                            let is_root = self.obj_link(l) == 0;
                            loop {
                                if !is_root {
                                    if i % PAGES_TREE_KIDS_MAX == 0 {
                                        // create a new Pages object for next level
                                        self.pdf_last_pages = self.pdf_new_objnum();
                                        if c == 0 {
                                            c = self.pdf_last_pages;
                                        }
                                        self.set_obj_link(self.pages_tail, self.pdf_last_pages);
                                        self.pages_tail = self.pdf_last_pages;
                                        self.set_obj_link(self.pdf_last_pages, 0);
                                        self.set_obj_info(self.pdf_last_pages, self.obj_info(l));
                                    } else {
                                        self.set_obj_info(self.pdf_last_pages, self.obj_info(self.pdf_last_pages) + self.obj_info(l));
                                    }
                                }
                                // §803 Output the current Pages object in this level
                                self.pdf_begin_dict(l, 1);
                                self.pdf_print_ln_s(b"/Type /Pages");
                                self.pdf_int_entry_ln(b"Count", self.obj_info(l));
                                if !is_root {
                                    self.pdf_indirect_ln(b"Parent", self.pdf_last_pages);
                                }
                                self.pdf_print_s(b"/Kids [");
                                j = 0;
                                loop {
                                    self.pdf_print_int(k);
                                    self.pdf_print_s(b" 0 R ");
                                    k = self.obj_link(k);
                                    j += 1;
                                    if (l < a && j == self.obj_info(l)) || k == 0 || (k == b && b != 0) || j == PAGES_TREE_KIDS_MAX {
                                        break;
                                    }
                                }
                                self.remove_last_space();
                                self.pdf_print_ln_s(b"]");
                                if k == 0 {
                                    k = self.head_tab[OBJ_TYPE_PAGES];
                                    self.head_tab[OBJ_TYPE_PAGES] = 0;
                                }
                                if is_root && self.pdf_pages_attr() != NULL {
                                    self.pdf_print_toks_ln(self.pdf_pages_attr());
                                }
                                self.pdf_end_dict();
                                i += 1;
                                l = self.obj_link(l);
                                if l == c {
                                    break;
                                }
                            }
                            b = c;
                            if l == 0 {
                                break; // goto done
                            }
                        }
                        // §788 Output outlines
                        let outlines;
                        if self.pdf_first_outline != 0 {
                            self.pdf_new_dict(OBJ_TYPE_OTHERS, 0, 1);
                            outlines = self.obj_ptr;
                            l = self.pdf_first_outline;
                            k = 0;
                            loop {
                                k += 1;
                                a = self.open_subentries(l);
                                if self.obj_outline_count(l) > 0 {
                                    k = k + a;
                                }
                                self.set_obj_outline_parent(l, self.obj_ptr);
                                l = self.obj_outline_next(l);
                                if l == 0 {
                                    break;
                                }
                            }
                            self.pdf_print_ln_s(b"/Type /Outlines");
                            self.pdf_indirect_ln(b"First", self.pdf_first_outline);
                            self.pdf_indirect_ln(b"Last", self.pdf_last_outline);
                            self.pdf_int_entry_ln(b"Count", k);
                            self.pdf_end_dict();
                            // §789 Output PDF outline entries
                            k = self.head_tab[OBJ_TYPE_OUTLINE];
                            while k != 0 {
                                if self.obj_outline_parent(k) == self.pdf_parent_outline {
                                    if self.obj_outline_prev(k) == 0 {
                                        self.pdf_first_outline = k;
                                    }
                                    if self.obj_outline_next(k) == 0 {
                                        self.pdf_last_outline = k;
                                    }
                                }
                                self.pdf_begin_dict(k, 1);
                                self.pdf_indirect_ln(b"Title", self.obj_outline_title(k));
                                self.pdf_indirect_ln(b"A", self.obj_outline_action_objnum(k));
                                if self.obj_outline_parent(k) != 0 {
                                    self.pdf_indirect_ln(b"Parent", self.obj_outline_parent(k));
                                }
                                if self.obj_outline_prev(k) != 0 {
                                    self.pdf_indirect_ln(b"Prev", self.obj_outline_prev(k));
                                }
                                if self.obj_outline_next(k) != 0 {
                                    self.pdf_indirect_ln(b"Next", self.obj_outline_next(k));
                                }
                                if self.obj_outline_first(k) != 0 {
                                    self.pdf_indirect_ln(b"First", self.obj_outline_first(k));
                                }
                                if self.obj_outline_last(k) != 0 {
                                    self.pdf_indirect_ln(b"Last", self.obj_outline_last(k));
                                }
                                if self.obj_outline_count(k) != 0 {
                                    self.pdf_int_entry_ln(b"Count", self.obj_outline_count(k));
                                }
                                if self.obj_outline_attr(k) != 0 {
                                    self.pdf_print_toks_ln(self.obj_outline_attr(k));
                                    self.delete_token_ref(self.obj_outline_attr(k));
                                    self.set_obj_outline_attr(k, NULL);
                                }
                                self.pdf_end_dict();
                                k = self.obj_link(k);
                            }
                        } else {
                            outlines = 0;
                        }
                        // §804 Output name tree
                        let mut dests = 0;
                        if self.pdf_dest_names_ptr != 0 {
                            self.sort_dest_names(0, self.pdf_dest_names_ptr - 1);
                            let mut names_head = 0;
                            let mut names_tail = 0;
                            // the index of current child of l; if k<pdf_dest_names_ptr then this is a
                            // pointer to the dest_names array, otherwise it is an object number
                            k = 0;
                            let mut is_names = true; // flag whether Names or Kids
                            b = 0;
                            loop {
                                loop {
                                    self.pdf_create_obj(OBJ_TYPE_OTHERS, 0); // create a new node
                                    l = self.obj_ptr;
                                    if b == 0 {
                                        b = l; // first in this level
                                    }
                                    if names_head == 0 {
                                        names_head = l;
                                        names_tail = l;
                                    } else {
                                        self.set_obj_link(names_tail, l);
                                        names_tail = l;
                                    }
                                    self.set_obj_link(names_tail, 0);
                                    // §805 Output the current node in this level
                                    self.pdf_begin_dict(l, 1);
                                    j = 0;
                                    if is_names {
                                        self.set_obj_info(l, self.dest_names[k].objname);
                                        self.pdf_print_s(b"/Names [");
                                        loop {
                                            self.pdf_print_str(self.dest_names[k].objname);
                                            self.pdf_out(b' ' as i32);
                                            self.pdf_print_int(self.dest_names[k].objnum);
                                            self.pdf_print_s(b" 0 R ");
                                            j += 1;
                                            k += 1;
                                            if j == NAME_TREE_KIDS_MAX || k == self.pdf_dest_names_ptr {
                                                break;
                                            }
                                        }
                                        self.remove_last_space();
                                        self.pdf_print_ln_s(b"]");
                                        self.set_obj_aux(l, self.dest_names[k - 1].objname);
                                        if k == self.pdf_dest_names_ptr {
                                            is_names = false;
                                            k = names_head;
                                            b = 0;
                                        }
                                    } else {
                                        self.set_obj_info(l, self.obj_info(k));
                                        self.pdf_print_s(b"/Kids [");
                                        loop {
                                            self.pdf_print_int(k);
                                            self.pdf_print_s(b" 0 R ");
                                            j += 1;
                                            self.set_obj_aux(l, self.obj_aux(k));
                                            k = self.obj_link(k);
                                            if j == NAME_TREE_KIDS_MAX || k == b || self.obj_link(k) == 0 {
                                                break;
                                            }
                                        }
                                        self.remove_last_space();
                                        self.pdf_print_ln_s(b"]");
                                        if k == b {
                                            b = 0;
                                        }
                                    }
                                    self.pdf_print_s(b"/Limits [");
                                    self.pdf_print_str(self.obj_info(l));
                                    self.pdf_out(b' ' as i32);
                                    self.pdf_print_str(self.obj_aux(l));
                                    self.pdf_print_ln_s(b"]");
                                    self.pdf_end_dict();
                                    if b == 0 {
                                        break;
                                    }
                                }
                                if k == l {
                                    dests = l;
                                    break; // goto done1
                                }
                            }
                        }
                        // done1:
                        let names_tree;
                        if dests != 0 || self.pdf_names_toks != NULL {
                            self.pdf_new_dict(OBJ_TYPE_OTHERS, 0, 1);
                            if dests != 0 {
                                self.pdf_indirect_ln(b"Dests", dests);
                            }
                            if self.pdf_names_toks != NULL {
                                self.pdf_print_toks_ln(self.pdf_names_toks);
                                self.delete_token_ref(self.pdf_names_toks);
                                self.pdf_names_toks = NULL;
                            }
                            self.pdf_end_dict();
                            names_tree = self.obj_ptr;
                        } else {
                            names_tree = 0;
                        }
                        // §790 Output article threads
                        let threads;
                        if self.head_tab[OBJ_TYPE_THREAD] != 0 {
                            self.pdf_new_obj(OBJ_TYPE_OTHERS, 0, 1);
                            threads = self.obj_ptr;
                            self.pdf_out(b'[' as i32);
                            k = self.head_tab[OBJ_TYPE_THREAD];
                            while k != 0 {
                                self.pdf_print_int(k);
                                self.pdf_print_s(b" 0 R ");
                                k = self.obj_link(k);
                            }
                            self.remove_last_space();
                            self.pdf_print_ln_s(b"]");
                            self.pdf_end_obj();
                            k = self.head_tab[OBJ_TYPE_THREAD];
                            while k != 0 {
                                self.out_thread(k);
                                k = self.obj_link(k);
                            }
                        } else {
                            threads = 0;
                        }
                        // §806 Output the catalog object
                        self.pdf_new_dict(OBJ_TYPE_OTHERS, 0, 1);
                        let root = self.obj_ptr;
                        self.pdf_print_ln_s(b"/Type /Catalog");
                        self.pdf_indirect_ln(b"Pages", self.pdf_last_pages);
                        if threads != 0 {
                            self.pdf_indirect_ln(b"Threads", threads);
                        }
                        if outlines != 0 {
                            self.pdf_indirect_ln(b"Outlines", outlines);
                        }
                        if names_tree != 0 {
                            self.pdf_indirect_ln(b"Names", names_tree);
                        }
                        if self.pdf_catalog_toks != NULL {
                            self.pdf_print_toks_ln(self.pdf_catalog_toks);
                            self.delete_token_ref(self.pdf_catalog_toks);
                            self.pdf_catalog_toks = NULL;
                        }
                        if self.pdf_catalog_openaction != 0 {
                            self.pdf_indirect_ln(b"OpenAction", self.pdf_catalog_openaction);
                        }
                        self.pdf_end_dict();
                        if self.pdf_omit_info_dict() == 0 {
                            self.pdf_print_info(); // last candidate for object stream
                        }
                        if self.pdf_os_enable {
                            self.pdf_os_switch(true);
                            self.pdf_os_write_objstream();
                            self.pdf_flush();
                            self.pdf_os_switch(false);
                            self.pdfout_settle_for_good();
                            // §814 Output the cross-reference stream dictionary
                            self.pdf_new_dict(OBJ_TYPE_OTHERS, 0, 0);
                            let xref_offset_width;
                            if self.obj_offset(self.sys_obj_ptr) as f64 / 256.0 > 16777215.0 {
                                xref_offset_width = 5;
                            } else if self.obj_offset(self.sys_obj_ptr) > 16777215 {
                                xref_offset_width = 4;
                            } else if self.obj_offset(self.sys_obj_ptr) > 65535 {
                                xref_offset_width = 3;
                            } else {
                                xref_offset_width = 2;
                            }
                            self.main_free_objects();
                            self.pdf_print_ln_s(b"/Type /XRef");
                            self.pdf_print_s(b"/Index [0 ");
                            self.pdf_print_int(self.obj_ptr + 1);
                            self.pdf_print_ln_s(b"]");
                            self.pdf_int_entry_ln(b"Size", self.obj_ptr + 1);
                            self.pdf_print_s(b"/W [1 ");
                            self.pdf_print_int(xref_offset_width);
                            self.pdf_print_ln_s(b" 1]");
                            self.pdf_indirect_ln(b"Root", root);
                            if self.pdf_omit_info_dict() == 0 {
                                self.pdf_indirect_ln(b"Info", self.obj_ptr - 1);
                            }
                            if self.pdf_trailer_toks != NULL {
                                self.pdf_print_toks_ln(self.pdf_trailer_toks);
                                self.delete_token_ref(self.pdf_trailer_toks);
                                self.pdf_trailer_toks = NULL;
                            }
                            if self.pdf_trailer_id_toks != NULL {
                                self.print_ID_alt(self.pdf_trailer_id_toks);
                            } else {
                                self.print_ID(self.output_file_name);
                            }
                            self.pdf_print_nl();
                            self.pdf_begin_stream();
                            for k in 0..=self.sys_obj_ptr {
                                if !self.is_obj_written(k) {
                                    // a free object
                                    self.pdf_out(0);
                                    self.pdf_out_bytes(self.obj_link(k), xref_offset_width);
                                    self.pdf_out(255);
                                } else if self.obj_os_idx(k) == -1 {
                                    // object not in object stream
                                    self.pdf_out(1);
                                    self.pdf_out_bytes(self.obj_offset(k), xref_offset_width);
                                    self.pdf_out(0);
                                } else {
                                    // object in object stream
                                    self.pdf_out(2);
                                    self.pdf_out_bytes(self.obj_offset(k), xref_offset_width);
                                    self.pdf_out(self.obj_os_idx(k));
                                }
                            }
                            self.pdf_end_stream();
                            self.pdf_flush();
                        } else {
                            // §813 Output the obj_tab
                            self.pdfout_settle_for_good();
                            self.main_free_objects();
                            self.pdf_save_offset = self.pdf_offset();
                            self.pdf_print_ln_s(b"xref");
                            self.pdf_print_s(b"0 ");
                            self.pdf_print_int_ln(self.obj_ptr + 1);
                            self.pdf_print_fw_int(self.obj_link(0), 10);
                            self.pdf_print_ln_s(b" 65535 f ");
                            for k in 1..=self.obj_ptr {
                                if !self.is_obj_written(k) {
                                    self.pdf_print_fw_int(self.obj_link(k), 10);
                                    self.pdf_print_ln_s(b" 00000 f ");
                                } else {
                                    self.pdf_print_fw_int(self.obj_offset(k), 10);
                                    self.pdf_print_ln_s(b" 00000 n ");
                                }
                            }
                        }
                        // §815 Output the trailer
                        if !self.pdf_os_enable {
                            self.pdf_print_ln_s(b"trailer");
                            self.pdf_print_s(b"<< ");
                            self.pdf_int_entry_ln(b"Size", self.sys_obj_ptr + 1);
                            self.pdf_indirect_ln(b"Root", root);
                            if self.pdf_omit_info_dict() == 0 {
                                self.pdf_indirect_ln(b"Info", self.sys_obj_ptr);
                            }
                            if self.pdf_trailer_toks != NULL {
                                self.pdf_print_toks_ln(self.pdf_trailer_toks);
                                self.delete_token_ref(self.pdf_trailer_toks);
                                self.pdf_trailer_toks = NULL;
                            }
                            if self.pdf_trailer_id_toks != NULL {
                                self.print_ID_alt(self.pdf_trailer_id_toks);
                            } else {
                                self.print_ID(self.output_file_name);
                            }
                            self.pdf_print_ln_s(b" >>");
                        }
                        self.pdf_print_ln_s(b"startxref");
                        if self.pdf_os_enable {
                            self.pdf_print_int_ln(self.obj_offset(self.sys_obj_ptr));
                        } else {
                            self.pdf_print_int_ln(self.pdf_save_offset);
                        }
                        self.pdf_print_ln_s(b"%%EOF");
                        self.pdf_flush();
                        self.pdfout_write_file();
                        self.host.hv.outcache.publish();
                        self.print_nl_s(b"Output written on ");
                        self.print_file_name(0, self.output_file_name, 0);
                        self.print_s(b" (");
                        self.print_int(self.total_pages);
                        self.print_s(b" page");
                        if self.total_pages != 1 {
                            self.print_char(b's' as i32);
                        }
                        self.print_s(b", ");
                        self.print_int(self.pdf_offset());
                        self.print_s(b" bytes).");
                    }
                    self.libpdffinish();
                    if self.fixed_pdf_draftmode == 0 {
                        self.b_close(PDF_FILE);
                        crate::prof_phase!("PDF closed");
                    } else {
                        self.pdf_warning(b"", b"\\pdfdraftmode enabled, not changing output pdf", true, true);
                    }
                }
                if self.log_opened {
                    self.wlog_cr();
                    self.wlog_ln_s(b"PDF statistics:");
                    self.wlog_s(b" ");
                    self.main_wlog_int(self.obj_ptr);
                    self.wlog_s(b" PDF objects out of ");
                    self.main_wlog_int(self.obj_tab_size);
                    self.wlog_s(b" (max. ");
                    self.main_wlog_int(SUP_OBJ_TAB_SIZE);
                    self.wlog_s(b")");
                    self.wlog_cr();
                    if self.pdf_os_cntr > 0 {
                        self.wlog_s(b" ");
                        self.main_wlog_int((self.pdf_os_cntr - 1) * PDF_OS_MAX_OBJS + self.pdf_os_objidx + 1);
                        self.wlog_s(b" compressed objects within ");
                        self.main_wlog_int(self.pdf_os_cntr);
                        self.wlog_s(b" object stream");
                        if self.pdf_os_cntr > 1 {
                            self.wlog_s(b"s");
                        }
                        self.wlog_cr();
                    }
                    self.wlog_s(b" ");
                    self.main_wlog_int(self.pdf_dest_names_ptr);
                    self.wlog_s(b" named destinations out of ");
                    self.main_wlog_int(self.dest_names_size);
                    self.wlog_s(b" (max. ");
                    self.main_wlog_int(SUP_DEST_NAMES_SIZE);
                    self.wlog_s(b")");
                    self.wlog_cr();
                    self.wlog_s(b" ");
                    self.main_wlog_int(self.pdf_mem_ptr);
                    self.wlog_s(b" words of extra memory for PDF output out of ");
                    self.main_wlog_int(self.pdf_mem_size);
                    self.wlog_s(b" (max. ");
                    self.main_wlog_int(SUP_PDF_MEM_SIZE);
                    self.wlog_s(b")");
                    self.wlog_cr();
                }
            }
        } else {
            // §670 Finish the DVI file
            while self.cur_s > -1 {
                if self.cur_s > 0 {
                    self.dvi_out(POP);
                } else {
                    self.dvi_out(EOP);
                    self.total_pages += 1;
                }
                self.cur_s -= 1;
            }
            if self.total_pages == 0 {
                self.print_nl_s(b"No pages of output.");
            } else if self.cur_s != -2 {
                self.dvi_out(POST); // beginning of the postamble
                self.dvi_four(self.last_bop);
                self.last_bop = self.dvi_offset + self.dvi_ptr - 5; // post location
                self.dvi_four(25400000);
                self.dvi_four(473628672); // conversion ratio for sp
                self.prepare_mag();
                self.dvi_four(self.mag()); // magnification factor
                self.dvi_four(self.max_v);
                self.dvi_four(self.max_h);
                self.dvi_out(self.max_push / 256);
                self.dvi_out(self.max_push % 256);
                self.dvi_out((self.total_pages / 256) % 256);
                self.dvi_out(self.total_pages % 256);
                // §671 Output the font definitions for all fonts that were used
                while self.font_ptr > FONT_BASE {
                    if self.font_used[self.font_ptr] {
                        self.dvi_font_def(self.font_ptr);
                    }
                    self.font_ptr -= 1;
                }
                self.dvi_out(POST_POST);
                self.dvi_four(self.last_bop);
                self.dvi_out(ID_BYTE);
                // dropped: IPC, `k:=7-((3+dvi_offset+dvi_ptr) mod 4)`
                k = 4 + ((self.dvi_buf_size - self.dvi_ptr) % 4); // the number of 223's
                while k > 0 {
                    self.dvi_out(223);
                    k -= 1;
                }
                // §626 Empty the last bytes out of dvi_buf
                if self.dvi_limit == self.half_buf {
                    self.write_dvi(self.half_buf, self.dvi_buf_size - 1);
                }
                if self.dvi_ptr > 0x7FFFFFFF - self.dvi_offset {
                    self.cur_s = -2;
                    self.fatal_error(b"dvi length exceeds \"7FFFFFFF");
                }
                if self.dvi_ptr > 0 {
                    self.write_dvi(0, self.dvi_ptr - 1);
                }
                self.print_nl_s(b"Output written on ");
                self.print_file_name(0, self.output_file_name, 0);
                self.print_s(b" (");
                self.print_int(self.total_pages);
                if self.total_pages != 1 {
                    self.print_s(b" pages");
                } else {
                    self.print_s(b" page");
                }
                self.print_s(b", ");
                self.print_int(self.dvi_offset + self.dvi_ptr);
                self.print_s(b" bytes).");
                self.b_close(DVI_FILE);
            }
        }
        // §1912 Close SyncTeX file and write status
        self.synctex_terminate(self.log_opened); // Let the SyncTeX controller close its files.
        if self.log_opened {
            self.wlog_cr();
            self.a_close(LOG_FILE);
            self.selector -= 2;
            if self.selector == TERM_ONLY {
                self.print_nl_s(b"Transcript written on ");
                self.print_file_name(0, self.log_name, 0);
                self.print_char(b'.' as i32);
            }
        }
        self.print_ln();
        if self.edit_name_start != 0 && self.interaction > BATCH_MODE {
            self.call_edit(self.edit_name_start, self.edit_name_length, self.edit_line);
        }
    }

    /// §812 Build a linked list of free objects
    fn main_free_objects(&mut self) {
        let mut l = 0;
        self.set_obj_fresh(l); // null object at begin of list of free objects
        for k in 1..=self.sys_obj_ptr {
            if !self.is_obj_written(k) {
                self.set_obj_link(l, k);
                l = k;
            }
        }
        self.set_obj_link(l, 0);
    }

    /// §1515
    pub fn final_cleanup(&mut self) {
        let c = self.cur_chr; // 0 for \end, 1 for \dump
        if c != 1 {
            self.set_new_line_char(-1);
        }
        if self.job_name == 0 {
            self.open_log_file();
        }
        while self.input_ptr > 0 {
            if self.state() == TOKEN_LIST {
                self.end_token_list();
            } else {
                self.end_file_reading();
            }
        }
        while self.open_parens > 0 {
            self.print_s(b" )");
            self.open_parens -= 1;
        }
        if self.cur_level > LEVEL_ONE {
            self.print_nl(b'(' as i32);
            self.print_esc_s(b"end occurred ");
            self.print_s(b"inside a group at level ");
            self.print_int(self.cur_level - LEVEL_ONE);
            self.print_char(b')' as i32);
            if self.eTeX_ex() {
                self.show_save_groups();
            }
        }
        while self.cond_ptr != NULL {
            self.print_nl(b'(' as i32);
            self.print_esc_s(b"end occurred ");
            self.print_s(b"when ");
            self.print_cmd_chr(IF_TEST, self.cur_if);
            if self.if_line != 0 {
                self.print_s(b" on line ");
                self.print_int(self.if_line);
            }
            self.print_s(b" was incomplete)");
            self.if_line = self.if_line_field(self.cond_ptr);
            self.cur_if = self.subtype(self.cond_ptr);
            self.temp_ptr = self.cond_ptr;
            self.cond_ptr = self.link(self.cond_ptr);
            self.free_node(self.temp_ptr, IF_NODE_SIZE);
        }
        if self.history != SPOTLESS {
            if self.history == WARNING_ISSUED || self.interaction < ERROR_STOP_MODE {
                if self.selector == TERM_AND_LOG {
                    self.selector = TERM_ONLY;
                    self.print_nl_s(b"(see the transcript file for additional information)");
                    self.selector = TERM_AND_LOG;
                }
            }
        }
        if c == 1 {
            if self.ini_version {
                for c in TOP_MARK_CODE..=SPLIT_BOT_MARK_CODE {
                    if self.cur_mark[c] != NULL {
                        self.delete_token_ref(self.cur_mark[c]);
                    }
                }
                if self.sa_mark() != NULL {
                    if self.do_marks(DESTROY_MARKS, 0, self.sa_mark()) {
                        self.set_sa_mark(NULL);
                    }
                }
                for c in LAST_BOX_CODE..=VSPLIT_CODE {
                    self.flush_node_list(self.disc_ptr[c]);
                }
                if self.last_glue != MAX_HALFWORD {
                    self.delete_glue_ref(self.last_glue);
                }
                self.store_fmt_file();
                return;
            }
            self.print_nl_s(b"(\\dump is performed only by INITEX)");
        }
    }

    /// §1516
    pub fn init_prim(&mut self) {
        self.no_new_control_sequence = false;
        self.first = 0;
        // §244
        self.primitive(b"lineskip", ASSIGN_GLUE, GLUE_BASE + LINE_SKIP_CODE);
        self.primitive(b"baselineskip", ASSIGN_GLUE, GLUE_BASE + BASELINE_SKIP_CODE);
        self.primitive(b"parskip", ASSIGN_GLUE, GLUE_BASE + PAR_SKIP_CODE);
        self.primitive(b"abovedisplayskip", ASSIGN_GLUE, GLUE_BASE + ABOVE_DISPLAY_SKIP_CODE);
        self.primitive(b"belowdisplayskip", ASSIGN_GLUE, GLUE_BASE + BELOW_DISPLAY_SKIP_CODE);
        self.primitive(b"abovedisplayshortskip", ASSIGN_GLUE, GLUE_BASE + ABOVE_DISPLAY_SHORT_SKIP_CODE);
        self.primitive(b"belowdisplayshortskip", ASSIGN_GLUE, GLUE_BASE + BELOW_DISPLAY_SHORT_SKIP_CODE);
        self.primitive(b"leftskip", ASSIGN_GLUE, GLUE_BASE + LEFT_SKIP_CODE);
        self.primitive(b"rightskip", ASSIGN_GLUE, GLUE_BASE + RIGHT_SKIP_CODE);
        self.primitive(b"topskip", ASSIGN_GLUE, GLUE_BASE + TOP_SKIP_CODE);
        self.primitive(b"splittopskip", ASSIGN_GLUE, GLUE_BASE + SPLIT_TOP_SKIP_CODE);
        self.primitive(b"tabskip", ASSIGN_GLUE, GLUE_BASE + TAB_SKIP_CODE);
        self.primitive(b"spaceskip", ASSIGN_GLUE, GLUE_BASE + SPACE_SKIP_CODE);
        self.primitive(b"xspaceskip", ASSIGN_GLUE, GLUE_BASE + XSPACE_SKIP_CODE);
        self.primitive(b"parfillskip", ASSIGN_GLUE, GLUE_BASE + PAR_FILL_SKIP_CODE);
        self.primitive(b"thinmuskip", ASSIGN_MU_GLUE, GLUE_BASE + THIN_MU_SKIP_CODE);
        self.primitive(b"medmuskip", ASSIGN_MU_GLUE, GLUE_BASE + MED_MU_SKIP_CODE);
        self.primitive(b"thickmuskip", ASSIGN_MU_GLUE, GLUE_BASE + THICK_MU_SKIP_CODE);
        // §248
        self.primitive(b"output", ASSIGN_TOKS, OUTPUT_ROUTINE_LOC);
        self.primitive(b"everypar", ASSIGN_TOKS, EVERY_PAR_LOC);
        self.primitive(b"everymath", ASSIGN_TOKS, EVERY_MATH_LOC);
        self.primitive(b"everydisplay", ASSIGN_TOKS, EVERY_DISPLAY_LOC);
        self.primitive(b"everyhbox", ASSIGN_TOKS, EVERY_HBOX_LOC);
        self.primitive(b"everyvbox", ASSIGN_TOKS, EVERY_VBOX_LOC);
        self.primitive(b"everyjob", ASSIGN_TOKS, EVERY_JOB_LOC);
        self.primitive(b"everycr", ASSIGN_TOKS, EVERY_CR_LOC);
        self.primitive(b"errhelp", ASSIGN_TOKS, ERR_HELP_LOC);
        self.primitive(b"pdfpagesattr", ASSIGN_TOKS, PDF_PAGES_ATTR_LOC);
        self.primitive(b"pdfpageattr", ASSIGN_TOKS, PDF_PAGE_ATTR_LOC);
        self.primitive(b"pdfpageresources", ASSIGN_TOKS, PDF_PAGE_RESOURCES_LOC);
        self.primitive(b"pdfpkmode", ASSIGN_TOKS, PDF_PK_MODE_LOC);
        // §256
        self.primitive(b"pretolerance", ASSIGN_INT, INT_BASE + PRETOLERANCE_CODE);
        self.primitive(b"tolerance", ASSIGN_INT, INT_BASE + TOLERANCE_CODE);
        self.primitive(b"linepenalty", ASSIGN_INT, INT_BASE + LINE_PENALTY_CODE);
        self.primitive(b"hyphenpenalty", ASSIGN_INT, INT_BASE + HYPHEN_PENALTY_CODE);
        self.primitive(b"exhyphenpenalty", ASSIGN_INT, INT_BASE + EX_HYPHEN_PENALTY_CODE);
        self.primitive(b"clubpenalty", ASSIGN_INT, INT_BASE + CLUB_PENALTY_CODE);
        self.primitive(b"widowpenalty", ASSIGN_INT, INT_BASE + WIDOW_PENALTY_CODE);
        self.primitive(b"displaywidowpenalty", ASSIGN_INT, INT_BASE + DISPLAY_WIDOW_PENALTY_CODE);
        self.primitive(b"brokenpenalty", ASSIGN_INT, INT_BASE + BROKEN_PENALTY_CODE);
        self.primitive(b"binoppenalty", ASSIGN_INT, INT_BASE + BIN_OP_PENALTY_CODE);
        self.primitive(b"relpenalty", ASSIGN_INT, INT_BASE + REL_PENALTY_CODE);
        self.primitive(b"predisplaypenalty", ASSIGN_INT, INT_BASE + PRE_DISPLAY_PENALTY_CODE);
        self.primitive(b"postdisplaypenalty", ASSIGN_INT, INT_BASE + POST_DISPLAY_PENALTY_CODE);
        self.primitive(b"interlinepenalty", ASSIGN_INT, INT_BASE + INTER_LINE_PENALTY_CODE);
        self.primitive(b"doublehyphendemerits", ASSIGN_INT, INT_BASE + DOUBLE_HYPHEN_DEMERITS_CODE);
        self.primitive(b"finalhyphendemerits", ASSIGN_INT, INT_BASE + FINAL_HYPHEN_DEMERITS_CODE);
        self.primitive(b"adjdemerits", ASSIGN_INT, INT_BASE + ADJ_DEMERITS_CODE);
        self.primitive(b"mag", ASSIGN_INT, INT_BASE + MAG_CODE);
        self.primitive(b"delimiterfactor", ASSIGN_INT, INT_BASE + DELIMITER_FACTOR_CODE);
        self.primitive(b"looseness", ASSIGN_INT, INT_BASE + LOOSENESS_CODE);
        self.primitive(b"time", ASSIGN_INT, INT_BASE + TIME_CODE);
        self.primitive(b"day", ASSIGN_INT, INT_BASE + DAY_CODE);
        self.primitive(b"month", ASSIGN_INT, INT_BASE + MONTH_CODE);
        self.primitive(b"year", ASSIGN_INT, INT_BASE + YEAR_CODE);
        self.primitive(b"showboxbreadth", ASSIGN_INT, INT_BASE + SHOW_BOX_BREADTH_CODE);
        self.primitive(b"showboxdepth", ASSIGN_INT, INT_BASE + SHOW_BOX_DEPTH_CODE);
        self.primitive(b"hbadness", ASSIGN_INT, INT_BASE + HBADNESS_CODE);
        self.primitive(b"vbadness", ASSIGN_INT, INT_BASE + VBADNESS_CODE);
        self.primitive(b"pausing", ASSIGN_INT, INT_BASE + PAUSING_CODE);
        self.primitive(b"tracingonline", ASSIGN_INT, INT_BASE + TRACING_ONLINE_CODE);
        self.primitive(b"tracingmacros", ASSIGN_INT, INT_BASE + TRACING_MACROS_CODE);
        self.primitive(b"tracingstats", ASSIGN_INT, INT_BASE + TRACING_STATS_CODE);
        self.primitive(b"tracingparagraphs", ASSIGN_INT, INT_BASE + TRACING_PARAGRAPHS_CODE);
        self.primitive(b"tracingpages", ASSIGN_INT, INT_BASE + TRACING_PAGES_CODE);
        self.primitive(b"tracingoutput", ASSIGN_INT, INT_BASE + TRACING_OUTPUT_CODE);
        self.primitive(b"tracinglostchars", ASSIGN_INT, INT_BASE + TRACING_LOST_CHARS_CODE);
        self.primitive(b"tracingcommands", ASSIGN_INT, INT_BASE + TRACING_COMMANDS_CODE);
        self.primitive(b"tracingrestores", ASSIGN_INT, INT_BASE + TRACING_RESTORES_CODE);
        self.primitive(b"uchyph", ASSIGN_INT, INT_BASE + UC_HYPH_CODE);
        self.primitive(b"outputpenalty", ASSIGN_INT, INT_BASE + OUTPUT_PENALTY_CODE);
        self.primitive(b"maxdeadcycles", ASSIGN_INT, INT_BASE + MAX_DEAD_CYCLES_CODE);
        self.primitive(b"hangafter", ASSIGN_INT, INT_BASE + HANG_AFTER_CODE);
        self.primitive(b"floatingpenalty", ASSIGN_INT, INT_BASE + FLOATING_PENALTY_CODE);
        self.primitive(b"globaldefs", ASSIGN_INT, INT_BASE + GLOBAL_DEFS_CODE);
        self.primitive(b"fam", ASSIGN_INT, INT_BASE + CUR_FAM_CODE);
        self.primitive(b"escapechar", ASSIGN_INT, INT_BASE + ESCAPE_CHAR_CODE);
        self.primitive(b"defaulthyphenchar", ASSIGN_INT, INT_BASE + DEFAULT_HYPHEN_CHAR_CODE);
        self.primitive(b"defaultskewchar", ASSIGN_INT, INT_BASE + DEFAULT_SKEW_CHAR_CODE);
        self.primitive(b"endlinechar", ASSIGN_INT, INT_BASE + END_LINE_CHAR_CODE);
        self.primitive(b"newlinechar", ASSIGN_INT, INT_BASE + NEW_LINE_CHAR_CODE);
        self.primitive(b"language", ASSIGN_INT, INT_BASE + LANGUAGE_CODE);
        self.primitive(b"lefthyphenmin", ASSIGN_INT, INT_BASE + LEFT_HYPHEN_MIN_CODE);
        self.primitive(b"righthyphenmin", ASSIGN_INT, INT_BASE + RIGHT_HYPHEN_MIN_CODE);
        self.primitive(b"holdinginserts", ASSIGN_INT, INT_BASE + HOLDING_INSERTS_CODE);
        self.primitive(b"errorcontextlines", ASSIGN_INT, INT_BASE + ERROR_CONTEXT_LINES_CODE);
        // dropped: MLTeX, `if mltex_p then ...`
        self.primitive(b"tracingstacklevels", ASSIGN_INT, INT_BASE + TRACING_STACK_LEVELS_CODE);
        self.primitive(b"partokenname", PARTOKEN_NAME, 0);
        self.primitive(b"partokencontext", ASSIGN_INT, INT_BASE + PARTOKEN_CONTEXT_CODE);
        self.primitive(b"showstream", ASSIGN_INT, INT_BASE + SHOW_STREAM_CODE);
        self.primitive(b"pdfoutput", ASSIGN_INT, INT_BASE + PDF_OUTPUT_CODE);
        self.primitive(b"pdfcompresslevel", ASSIGN_INT, INT_BASE + PDF_COMPRESS_LEVEL_CODE);
        self.primitive(b"pdfobjcompresslevel", ASSIGN_INT, INT_BASE + PDF_OBJCOMPRESSLEVEL_CODE);
        self.primitive(b"pdfdecimaldigits", ASSIGN_INT, INT_BASE + PDF_DECIMAL_DIGITS_CODE);
        self.primitive(b"pdfmovechars", ASSIGN_INT, INT_BASE + PDF_MOVE_CHARS_CODE);
        self.primitive(b"pdfimageresolution", ASSIGN_INT, INT_BASE + PDF_IMAGE_RESOLUTION_CODE);
        self.primitive(b"pdfpkresolution", ASSIGN_INT, INT_BASE + PDF_PK_RESOLUTION_CODE);
        self.primitive(b"pdfuniqueresname", ASSIGN_INT, INT_BASE + PDF_UNIQUE_RESNAME_CODE);
        self.primitive(b"pdfoptionpdfminorversion", ASSIGN_INT, INT_BASE + PDF_MINOR_VERSION_CODE);
        self.primitive(b"pdfoptionalwaysusepdfpagebox", ASSIGN_INT, INT_BASE + PDF_OPTION_ALWAYS_USE_PDFPAGEBOX_CODE);
        self.primitive(b"pdfoptionpdfinclusionerrorlevel", ASSIGN_INT, INT_BASE + PDF_OPTION_PDF_INCLUSION_ERRORLEVEL_CODE);
        self.primitive(b"pdfmajorversion", ASSIGN_INT, INT_BASE + PDF_MAJOR_VERSION_CODE);
        self.primitive(b"pdfminorversion", ASSIGN_INT, INT_BASE + PDF_MINOR_VERSION_CODE);
        self.primitive(b"pdfforcepagebox", ASSIGN_INT, INT_BASE + PDF_FORCE_PAGEBOX_CODE);
        self.primitive(b"pdfpagebox", ASSIGN_INT, INT_BASE + PDF_PAGEBOX_CODE);
        self.primitive(b"pdfinclusionerrorlevel", ASSIGN_INT, INT_BASE + PDF_INCLUSION_ERRORLEVEL_CODE);
        self.primitive(b"pdfgamma", ASSIGN_INT, INT_BASE + PDF_GAMMA_CODE);
        self.primitive(b"pdfimagegamma", ASSIGN_INT, INT_BASE + PDF_IMAGE_GAMMA_CODE);
        self.primitive(b"pdfimagehicolor", ASSIGN_INT, INT_BASE + PDF_IMAGE_HICOLOR_CODE);
        self.primitive(b"pdfimageapplygamma", ASSIGN_INT, INT_BASE + PDF_IMAGE_APPLY_GAMMA_CODE);
        self.primitive(b"pdfadjustspacing", ASSIGN_INT, INT_BASE + PDF_ADJUST_SPACING_CODE);
        self.primitive(b"pdfprotrudechars", ASSIGN_INT, INT_BASE + PDF_PROTRUDE_CHARS_CODE);
        self.primitive(b"pdftracingfonts", ASSIGN_INT, INT_BASE + PDF_TRACING_FONTS_CODE);
        self.primitive(b"pdfadjustinterwordglue", ASSIGN_INT, INT_BASE + PDF_ADJUST_INTERWORD_GLUE_CODE);
        self.primitive(b"pdfprependkern", ASSIGN_INT, INT_BASE + PDF_PREPEND_KERN_CODE);
        self.primitive(b"pdfappendkern", ASSIGN_INT, INT_BASE + PDF_APPEND_KERN_CODE);
        self.primitive(b"pdfgentounicode", ASSIGN_INT, INT_BASE + PDF_GEN_TOUNICODE_CODE);
        self.primitive(b"pdfdraftmode", ASSIGN_INT, INT_BASE + PDF_DRAFTMODE_CODE);
        self.primitive(b"pdfinclusioncopyfonts", ASSIGN_INT, INT_BASE + PDF_INCLUSION_COPY_FONT_CODE);
        self.primitive(b"pdfsuppresswarningdupdest", ASSIGN_INT, INT_BASE + PDF_SUPPRESS_WARNING_DUP_DEST_CODE);
        self.primitive(b"pdfsuppresswarningdupmap", ASSIGN_INT, INT_BASE + PDF_SUPPRESS_WARNING_DUP_MAP_CODE);
        self.primitive(b"pdfsuppresswarningpagegroup", ASSIGN_INT, INT_BASE + PDF_SUPPRESS_WARNING_PAGE_GROUP_CODE);
        self.primitive(b"pdfinfoomitdate", ASSIGN_INT, INT_BASE + PDF_INFO_OMIT_DATE_CODE);
        self.primitive(b"pdfsuppressptexinfo", ASSIGN_INT, INT_BASE + PDF_SUPPRESS_PTEX_INFO_CODE);
        self.primitive(b"pdfomitcharset", ASSIGN_INT, INT_BASE + PDF_OMIT_CHARSET_CODE);
        self.primitive(b"pdfomitinfodict", ASSIGN_INT, INT_BASE + PDF_OMIT_INFO_DICT_CODE);
        self.primitive(b"pdfomitprocset", ASSIGN_INT, INT_BASE + PDF_OMIT_PROCSET_CODE);
        self.primitive(b"pdfptexuseunderscore", ASSIGN_INT, INT_BASE + PDF_PTEX_USE_UNDERSCORE_CODE);
        // §266
        self.primitive(b"parindent", ASSIGN_DIMEN, DIMEN_BASE + PAR_INDENT_CODE);
        self.primitive(b"mathsurround", ASSIGN_DIMEN, DIMEN_BASE + MATH_SURROUND_CODE);
        self.primitive(b"lineskiplimit", ASSIGN_DIMEN, DIMEN_BASE + LINE_SKIP_LIMIT_CODE);
        self.primitive(b"hsize", ASSIGN_DIMEN, DIMEN_BASE + HSIZE_CODE);
        self.primitive(b"vsize", ASSIGN_DIMEN, DIMEN_BASE + VSIZE_CODE);
        self.primitive(b"maxdepth", ASSIGN_DIMEN, DIMEN_BASE + MAX_DEPTH_CODE);
        self.primitive(b"splitmaxdepth", ASSIGN_DIMEN, DIMEN_BASE + SPLIT_MAX_DEPTH_CODE);
        self.primitive(b"boxmaxdepth", ASSIGN_DIMEN, DIMEN_BASE + BOX_MAX_DEPTH_CODE);
        self.primitive(b"hfuzz", ASSIGN_DIMEN, DIMEN_BASE + HFUZZ_CODE);
        self.primitive(b"vfuzz", ASSIGN_DIMEN, DIMEN_BASE + VFUZZ_CODE);
        self.primitive(b"delimitershortfall", ASSIGN_DIMEN, DIMEN_BASE + DELIMITER_SHORTFALL_CODE);
        self.primitive(b"nulldelimiterspace", ASSIGN_DIMEN, DIMEN_BASE + NULL_DELIMITER_SPACE_CODE);
        self.primitive(b"scriptspace", ASSIGN_DIMEN, DIMEN_BASE + SCRIPT_SPACE_CODE);
        self.primitive(b"predisplaysize", ASSIGN_DIMEN, DIMEN_BASE + PRE_DISPLAY_SIZE_CODE);
        self.primitive(b"displaywidth", ASSIGN_DIMEN, DIMEN_BASE + DISPLAY_WIDTH_CODE);
        self.primitive(b"displayindent", ASSIGN_DIMEN, DIMEN_BASE + DISPLAY_INDENT_CODE);
        self.primitive(b"overfullrule", ASSIGN_DIMEN, DIMEN_BASE + OVERFULL_RULE_CODE);
        self.primitive(b"hangindent", ASSIGN_DIMEN, DIMEN_BASE + HANG_INDENT_CODE);
        self.primitive(b"hoffset", ASSIGN_DIMEN, DIMEN_BASE + H_OFFSET_CODE);
        self.primitive(b"voffset", ASSIGN_DIMEN, DIMEN_BASE + V_OFFSET_CODE);
        self.primitive(b"emergencystretch", ASSIGN_DIMEN, DIMEN_BASE + EMERGENCY_STRETCH_CODE);
        self.primitive(b"pdfhorigin", ASSIGN_DIMEN, DIMEN_BASE + PDF_H_ORIGIN_CODE);
        self.primitive(b"pdfvorigin", ASSIGN_DIMEN, DIMEN_BASE + PDF_V_ORIGIN_CODE);
        self.primitive(b"pdfpagewidth", ASSIGN_DIMEN, DIMEN_BASE + PDF_PAGE_WIDTH_CODE);
        self.primitive(b"pdfpageheight", ASSIGN_DIMEN, DIMEN_BASE + PDF_PAGE_HEIGHT_CODE);
        self.primitive(b"pdflinkmargin", ASSIGN_DIMEN, DIMEN_BASE + PDF_LINK_MARGIN_CODE);
        self.primitive(b"pdfdestmargin", ASSIGN_DIMEN, DIMEN_BASE + PDF_DEST_MARGIN_CODE);
        self.primitive(b"pdfthreadmargin", ASSIGN_DIMEN, DIMEN_BASE + PDF_THREAD_MARGIN_CODE);
        self.primitive(b"pdffirstlineheight", ASSIGN_DIMEN, DIMEN_BASE + PDF_FIRST_LINE_HEIGHT_CODE);
        self.primitive(b"pdflastlinedepth", ASSIGN_DIMEN, DIMEN_BASE + PDF_LAST_LINE_DEPTH_CODE);
        self.primitive(b"pdfeachlineheight", ASSIGN_DIMEN, DIMEN_BASE + PDF_EACH_LINE_HEIGHT_CODE);
        self.primitive(b"pdfeachlinedepth", ASSIGN_DIMEN, DIMEN_BASE + PDF_EACH_LINE_DEPTH_CODE);
        self.primitive(b"pdfignoreddimen", ASSIGN_DIMEN, DIMEN_BASE + PDF_IGNORED_DIMEN_CODE);
        self.primitive(b"pdfpxdimen", ASSIGN_DIMEN, DIMEN_BASE + PDF_PX_DIMEN_CODE);
        // §287
        self.primitive(b" ", EX_SPACE, 0);
        self.primitive(b"/", ITAL_CORR, 0);
        self.primitive(b"accent", ACCENT, 0);
        self.primitive(b"advance", ADVANCE, 0);
        self.primitive(b"afterassignment", AFTER_ASSIGNMENT, 0);
        self.primitive(b"aftergroup", AFTER_GROUP, 0);
        self.primitive(b"begingroup", BEGIN_GROUP, 0);
        self.primitive(b"char", CHAR_NUM, 0);
        self.primitive(b"csname", CS_NAME, 0);
        self.primitive(b"delimiter", DELIM_NUM, 0);
        self.primitive(b"divide", DIVIDE, 0);
        self.primitive(b"endcsname", END_CS_NAME, 0);
        self.primitive(b"endgroup", END_GROUP, 0);
        let s = self.text(self.cur_val); // "endgroup", which `primitive` just made
        self.set_text(FROZEN_END_GROUP, s);
        self.eqtb[FROZEN_END_GROUP] = self.eqtb[self.cur_val];
        self.primitive(b"expandafter", EXPAND_AFTER, 0);
        self.primitive(b"font", DEF_FONT, 0);
        self.primitive(b"letterspacefont", LETTERSPACE_FONT, 0);
        self.primitive(b"pdfcopyfont", PDF_COPY_FONT, 0);
        self.primitive(b"fontdimen", ASSIGN_FONT_DIMEN, 0);
        self.primitive(b"halign", HALIGN, 0);
        self.primitive(b"hrule", HRULE, 0);
        self.primitive(b"ignorespaces", IGNORE_SPACES, 0);
        self.primitive(b"insert", INSERT, 0);
        self.primitive(b"mark", MARK, 0);
        self.primitive(b"mathaccent", MATH_ACCENT, 0);
        self.primitive(b"mathchar", MATH_CHAR_NUM, 0);
        self.primitive(b"mathchoice", MATH_CHOICE, 0);
        self.primitive(b"multiply", MULTIPLY, 0);
        self.primitive(b"noalign", NO_ALIGN, 0);
        self.primitive(b"noboundary", NO_BOUNDARY, 0);
        self.primitive(b"noexpand", NO_EXPAND, 0);
        self.primitive(b"pdfprimitive", NO_EXPAND, 1);
        self.primitive(b"nonscript", NON_SCRIPT, 0);
        self.primitive(b"omit", OMIT, 0);
        self.primitive(b"parshape", SET_SHAPE, PAR_SHAPE_LOC);
        self.primitive(b"penalty", BREAK_PENALTY, 0);
        self.primitive(b"prevgraf", SET_PREV_GRAF, 0);
        self.primitive(b"radical", RADICAL, 0);
        self.primitive(b"read", READ_TO_CS, 0);
        self.primitive(b"relax", RELAX, 256);
        let s = self.text(self.cur_val); // "relax", which `primitive` just made
        self.set_text(FROZEN_RELAX, s);
        self.eqtb[FROZEN_RELAX] = self.eqtb[self.cur_val];
        self.primitive(b"setbox", SET_BOX, 0);
        self.primitive(b"the", THE, 0);
        self.primitive(b"toks", TOKS_REGISTER, MEM_BOT);
        self.primitive(b"vadjust", VADJUST, 0);
        self.primitive(b"valign", VALIGN, 0);
        self.primitive(b"vcenter", VCENTER, 0);
        self.primitive(b"vrule", VRULE, 0);
        // §356
        self.primitive(b"par", PAR_END, 256);
        self.par_loc = self.cur_val;
        self.par_token = CS_TOKEN_FLAG + self.par_loc;
        // §402
        self.primitive(b"input", INPUT, 0);
        self.primitive(b"endinput", INPUT, 1);
        // §410
        self.primitive(b"topmark", TOP_BOT_MARK, TOP_MARK_CODE);
        self.primitive(b"firstmark", TOP_BOT_MARK, FIRST_MARK_CODE);
        self.primitive(b"botmark", TOP_BOT_MARK, BOT_MARK_CODE);
        self.primitive(b"splitfirstmark", TOP_BOT_MARK, SPLIT_FIRST_MARK_CODE);
        self.primitive(b"splitbotmark", TOP_BOT_MARK, SPLIT_BOT_MARK_CODE);
        // §437
        self.primitive(b"count", REGISTER, MEM_BOT + INT_VAL);
        self.primitive(b"dimen", REGISTER, MEM_BOT + DIMEN_VAL);
        self.primitive(b"skip", REGISTER, MEM_BOT + GLUE_VAL);
        self.primitive(b"muskip", REGISTER, MEM_BOT + MU_VAL);
        // §442
        self.primitive(b"spacefactor", SET_AUX, HMODE);
        self.primitive(b"prevdepth", SET_AUX, VMODE);
        self.primitive(b"deadcycles", SET_PAGE_INT, 0);
        self.primitive(b"insertpenalties", SET_PAGE_INT, 1);
        self.primitive(b"wd", SET_BOX_DIMEN, WIDTH_OFFSET);
        self.primitive(b"ht", SET_BOX_DIMEN, HEIGHT_OFFSET);
        self.primitive(b"dp", SET_BOX_DIMEN, DEPTH_OFFSET);
        self.primitive(b"lastpenalty", LAST_ITEM, INT_VAL);
        self.primitive(b"lastkern", LAST_ITEM, DIMEN_VAL);
        self.primitive(b"lastskip", LAST_ITEM, GLUE_VAL);
        self.primitive(b"inputlineno", LAST_ITEM, INPUT_LINE_NO_CODE);
        self.primitive(b"badness", LAST_ITEM, BADNESS_CODE);
        self.primitive(b"pdftexversion", LAST_ITEM, PDFTEX_VERSION_CODE);
        self.primitive(b"pdflastobj", LAST_ITEM, PDF_LAST_OBJ_CODE);
        self.primitive(b"pdflastxform", LAST_ITEM, PDF_LAST_XFORM_CODE);
        self.primitive(b"pdflastximage", LAST_ITEM, PDF_LAST_XIMAGE_CODE);
        self.primitive(b"pdflastximagepages", LAST_ITEM, PDF_LAST_XIMAGE_PAGES_CODE);
        self.primitive(b"pdflastannot", LAST_ITEM, PDF_LAST_ANNOT_CODE);
        self.primitive(b"pdflastxpos", LAST_ITEM, PDF_LAST_X_POS_CODE);
        self.primitive(b"pdflastypos", LAST_ITEM, PDF_LAST_Y_POS_CODE);
        self.primitive(b"pdfretval", LAST_ITEM, PDF_RETVAL_CODE);
        self.primitive(b"pdflastximagecolordepth", LAST_ITEM, PDF_LAST_XIMAGE_COLORDEPTH_CODE);
        self.primitive(b"pdfelapsedtime", LAST_ITEM, ELAPSED_TIME_CODE);
        self.primitive(b"pdfshellescape", LAST_ITEM, PDF_SHELL_ESCAPE_CODE);
        self.primitive(b"pdfrandomseed", LAST_ITEM, RANDOM_SEED_CODE);
        self.primitive(b"pdflastlink", LAST_ITEM, PDF_LAST_LINK_CODE);
        // §494
        self.primitive(b"number", CONVERT, NUMBER_CODE);
        self.primitive(b"romannumeral", CONVERT, ROMAN_NUMERAL_CODE);
        self.primitive(b"string", CONVERT, STRING_CODE);
        self.primitive(b"meaning", CONVERT, MEANING_CODE);
        self.primitive(b"fontname", CONVERT, FONT_NAME_CODE);
        self.primitive(b"expanded", CONVERT, EXPANDED_CODE);
        self.primitive(b"pdftexrevision", CONVERT, PDFTEX_REVISION_CODE);
        self.primitive(b"pdftexbanner", CONVERT, PDFTEX_BANNER_CODE);
        self.primitive(b"pdffontname", CONVERT, PDF_FONT_NAME_CODE);
        self.primitive(b"pdffontobjnum", CONVERT, PDF_FONT_OBJNUM_CODE);
        self.primitive(b"pdffontsize", CONVERT, PDF_FONT_SIZE_CODE);
        self.primitive(b"pdfpageref", CONVERT, PDF_PAGE_REF_CODE);
        self.primitive(b"leftmarginkern", CONVERT, LEFT_MARGIN_KERN_CODE);
        self.primitive(b"rightmarginkern", CONVERT, RIGHT_MARGIN_KERN_CODE);
        self.primitive(b"pdfxformname", CONVERT, PDF_XFORM_NAME_CODE);
        self.primitive(b"pdfescapestring", CONVERT, PDF_ESCAPE_STRING_CODE);
        self.primitive(b"pdfescapename", CONVERT, PDF_ESCAPE_NAME_CODE);
        self.primitive(b"pdfescapehex", CONVERT, PDF_ESCAPE_HEX_CODE);
        self.primitive(b"pdfunescapehex", CONVERT, PDF_UNESCAPE_HEX_CODE);
        self.primitive(b"pdfcreationdate", CONVERT, PDF_CREATION_DATE_CODE);
        self.primitive(b"pdffilemoddate", CONVERT, PDF_FILE_MOD_DATE_CODE);
        self.primitive(b"pdffilesize", CONVERT, PDF_FILE_SIZE_CODE);
        self.primitive(b"pdfmdfivesum", CONVERT, PDF_MDFIVE_SUM_CODE);
        self.primitive(b"pdffiledump", CONVERT, PDF_FILE_DUMP_CODE);
        self.primitive(b"pdfmatch", CONVERT, PDF_MATCH_CODE);
        self.primitive(b"pdflastmatch", CONVERT, PDF_LAST_MATCH_CODE);
        self.primitive(b"pdfstrcmp", CONVERT, PDF_STRCMP_CODE);
        self.primitive(b"pdfcolorstackinit", CONVERT, PDF_COLORSTACK_INIT_CODE);
        self.primitive(b"pdfuniformdeviate", CONVERT, UNIFORM_DEVIATE_CODE);
        self.primitive(b"pdfnormaldeviate", CONVERT, NORMAL_DEVIATE_CODE);
        self.primitive(b"jobname", CONVERT, JOB_NAME_CODE);
        self.primitive(b"pdfinsertht", CONVERT, PDF_INSERT_HT_CODE);
        self.primitive(b"pdfximagebbox", CONVERT, PDF_XIMAGE_BBOX_CODE);
        // §513
        self.primitive(b"if", IF_TEST, IF_CHAR_CODE);
        self.primitive(b"ifcat", IF_TEST, IF_CAT_CODE);
        self.primitive(b"ifnum", IF_TEST, IF_INT_CODE);
        self.primitive(b"ifdim", IF_TEST, IF_DIM_CODE);
        self.primitive(b"ifodd", IF_TEST, IF_ODD_CODE);
        self.primitive(b"ifvmode", IF_TEST, IF_VMODE_CODE);
        self.primitive(b"ifhmode", IF_TEST, IF_HMODE_CODE);
        self.primitive(b"ifmmode", IF_TEST, IF_MMODE_CODE);
        self.primitive(b"ifinner", IF_TEST, IF_INNER_CODE);
        self.primitive(b"ifvoid", IF_TEST, IF_VOID_CODE);
        self.primitive(b"ifhbox", IF_TEST, IF_HBOX_CODE);
        self.primitive(b"ifvbox", IF_TEST, IF_VBOX_CODE);
        self.primitive(b"ifx", IF_TEST, IFX_CODE);
        self.primitive(b"ifeof", IF_TEST, IF_EOF_CODE);
        self.primitive(b"iftrue", IF_TEST, IF_TRUE_CODE);
        self.primitive(b"iffalse", IF_TEST, IF_FALSE_CODE);
        self.primitive(b"ifcase", IF_TEST, IF_CASE_CODE);
        self.primitive(b"ifpdfprimitive", IF_TEST, IF_PDFPRIMITIVE_CODE);
        // §517
        self.primitive(b"fi", FI_OR_ELSE, FI_CODE);
        let s = self.text(self.cur_val); // "fi", which `primitive` just made
        self.set_text(FROZEN_FI, s);
        self.eqtb[FROZEN_FI] = self.eqtb[self.cur_val];
        self.primitive(b"or", FI_OR_ELSE, OR_CODE);
        self.primitive(b"else", FI_OR_ELSE, ELSE_CODE);
        // §579
        self.primitive(b"nullfont", SET_FONT, NULL_FONT);
        let s = self.text(self.cur_val); // "nullfont", which `primitive` just made
        self.set_text(FROZEN_NULL_FONT, s);
        self.eqtb[FROZEN_NULL_FONT] = self.eqtb[self.cur_val];
        // §956
        self.primitive(b"span", TAB_MARK, SPAN_CODE);
        self.primitive(b"cr", CAR_RET, CR_CODE);
        let s = self.text(self.cur_val); // "cr", which `primitive` just made
        self.set_text(FROZEN_CR, s);
        self.eqtb[FROZEN_CR] = self.eqtb[self.cur_val];
        self.primitive(b"crcr", CAR_RET, CR_CR_CODE);
        self.set_text(FROZEN_END_TEMPLATE, S_ENDTEMPLATE);
        self.set_text(FROZEN_ENDV, S_ENDTEMPLATE);
        self.set_eq_type(FROZEN_ENDV, ENDV);
        self.set_equiv(FROZEN_ENDV, NULL_LIST);
        self.set_eq_level(FROZEN_ENDV, LEVEL_ONE);
        self.eqtb[FROZEN_END_TEMPLATE] = self.eqtb[FROZEN_ENDV];
        self.set_eq_type(FROZEN_END_TEMPLATE, END_TEMPLATE);
        // §1160
        self.primitive(b"pagegoal", SET_PAGE_DIMEN, 0);
        self.primitive(b"pagetotal", SET_PAGE_DIMEN, 1);
        self.primitive(b"pagestretch", SET_PAGE_DIMEN, 2);
        self.primitive(b"pagefilstretch", SET_PAGE_DIMEN, 3);
        self.primitive(b"pagefillstretch", SET_PAGE_DIMEN, 4);
        self.primitive(b"pagefilllstretch", SET_PAGE_DIMEN, 5);
        self.primitive(b"pageshrink", SET_PAGE_DIMEN, 6);
        self.primitive(b"pagedepth", SET_PAGE_DIMEN, 7);
        // §1230
        self.primitive(b"end", STOP, 0);
        self.primitive(b"dump", STOP, 1);
        // §1236
        self.primitive(b"hskip", HSKIP, SKIP_CODE);
        self.primitive(b"hfil", HSKIP, FIL_CODE);
        self.primitive(b"hfill", HSKIP, FILL_CODE);
        self.primitive(b"hss", HSKIP, SS_CODE);
        self.primitive(b"hfilneg", HSKIP, FIL_NEG_CODE);
        self.primitive(b"vskip", VSKIP, SKIP_CODE);
        self.primitive(b"vfil", VSKIP, FIL_CODE);
        self.primitive(b"vfill", VSKIP, FILL_CODE);
        self.primitive(b"vss", VSKIP, SS_CODE);
        self.primitive(b"vfilneg", VSKIP, FIL_NEG_CODE);
        self.primitive(b"mskip", MSKIP, MSKIP_CODE);
        self.primitive(b"kern", KERN, EXPLICIT);
        self.primitive(b"mkern", MKERN, MU_GLUE);
        // §1249
        self.primitive(b"moveleft", HMOVE, 1);
        self.primitive(b"moveright", HMOVE, 0);
        self.primitive(b"raise", VMOVE, 1);
        self.primitive(b"lower", VMOVE, 0);
        self.primitive(b"box", MAKE_BOX, BOX_CODE);
        self.primitive(b"copy", MAKE_BOX, COPY_CODE);
        self.primitive(b"lastbox", MAKE_BOX, LAST_BOX_CODE);
        self.primitive(b"vsplit", MAKE_BOX, VSPLIT_CODE);
        self.primitive(b"vtop", MAKE_BOX, VTOP_CODE);
        self.primitive(b"vbox", MAKE_BOX, VTOP_CODE + VMODE);
        self.primitive(b"hbox", MAKE_BOX, VTOP_CODE + HMODE);
        self.primitive(b"shipout", LEADER_SHIP, A_LEADERS - 1);
        self.primitive(b"leaders", LEADER_SHIP, A_LEADERS);
        self.primitive(b"cleaders", LEADER_SHIP, C_LEADERS);
        self.primitive(b"xleaders", LEADER_SHIP, X_LEADERS);
        // §1266
        self.primitive(b"indent", START_PAR, 1);
        self.primitive(b"noindent", START_PAR, 0);
        self.primitive(b"quitvmode", START_PAR, 2);
        // §1285
        self.primitive(b"unpenalty", REMOVE_ITEM, PENALTY_NODE);
        self.primitive(b"unkern", REMOVE_ITEM, KERN_NODE);
        self.primitive(b"unskip", REMOVE_ITEM, GLUE_NODE);
        self.primitive(b"unhbox", UN_HBOX, BOX_CODE);
        self.primitive(b"unhcopy", UN_HBOX, COPY_CODE);
        self.primitive(b"unvbox", UN_VBOX, BOX_CODE);
        self.primitive(b"unvcopy", UN_VBOX, COPY_CODE);
        // §1292
        self.primitive(b"-", DISCRETIONARY, 1);
        self.primitive(b"discretionary", DISCRETIONARY, 0);
        // §1319
        self.primitive(b"eqno", EQ_NO, 0);
        self.primitive(b"leqno", EQ_NO, 1);
        // §1334
        self.primitive(b"mathord", MATH_COMP, ORD_NOAD);
        self.primitive(b"mathop", MATH_COMP, OP_NOAD);
        self.primitive(b"mathbin", MATH_COMP, BIN_NOAD);
        self.primitive(b"mathrel", MATH_COMP, REL_NOAD);
        self.primitive(b"mathopen", MATH_COMP, OPEN_NOAD);
        self.primitive(b"mathclose", MATH_COMP, CLOSE_NOAD);
        self.primitive(b"mathpunct", MATH_COMP, PUNCT_NOAD);
        self.primitive(b"mathinner", MATH_COMP, INNER_NOAD);
        self.primitive(b"underline", MATH_COMP, UNDER_NOAD);
        self.primitive(b"overline", MATH_COMP, OVER_NOAD);
        self.primitive(b"displaylimits", LIMIT_SWITCH, NORMAL);
        self.primitive(b"limits", LIMIT_SWITCH, LIMITS);
        self.primitive(b"nolimits", LIMIT_SWITCH, NO_LIMITS);
        // §1347
        self.primitive(b"displaystyle", MATH_STYLE, DISPLAY_STYLE);
        self.primitive(b"textstyle", MATH_STYLE, TEXT_STYLE);
        self.primitive(b"scriptstyle", MATH_STYLE, SCRIPT_STYLE);
        self.primitive(b"scriptscriptstyle", MATH_STYLE, SCRIPT_SCRIPT_STYLE);
        // §1356
        self.primitive(b"above", ABOVE, ABOVE_CODE);
        self.primitive(b"over", ABOVE, OVER_CODE);
        self.primitive(b"atop", ABOVE, ATOP_CODE);
        self.primitive(b"abovewithdelims", ABOVE, DELIMITED_CODE + ABOVE_CODE);
        self.primitive(b"overwithdelims", ABOVE, DELIMITED_CODE + OVER_CODE);
        self.primitive(b"atopwithdelims", ABOVE, DELIMITED_CODE + ATOP_CODE);
        // §1366
        self.primitive(b"left", LEFT_RIGHT, LEFT_NOAD);
        self.primitive(b"right", LEFT_RIGHT, RIGHT_NOAD);
        let s = self.text(self.cur_val); // "right", which `primitive` just made
        self.set_text(FROZEN_RIGHT, s);
        self.eqtb[FROZEN_RIGHT] = self.eqtb[self.cur_val];
        // §1386
        self.primitive(b"long", PREFIX, 1);
        self.primitive(b"outer", PREFIX, 2);
        self.primitive(b"global", PREFIX, 4);
        self.primitive(b"def", DEF, 0);
        self.primitive(b"gdef", DEF, 1);
        self.primitive(b"edef", DEF, 2);
        self.primitive(b"xdef", DEF, 3);
        // §1397
        self.primitive(b"let", LET, NORMAL);
        self.primitive(b"futurelet", LET, NORMAL + 1);
        // §1400
        self.primitive(b"chardef", SHORTHAND_DEF, CHAR_DEF_CODE);
        self.primitive(b"mathchardef", SHORTHAND_DEF, MATH_CHAR_DEF_CODE);
        self.primitive(b"countdef", SHORTHAND_DEF, COUNT_DEF_CODE);
        self.primitive(b"dimendef", SHORTHAND_DEF, DIMEN_DEF_CODE);
        self.primitive(b"skipdef", SHORTHAND_DEF, SKIP_DEF_CODE);
        self.primitive(b"muskipdef", SHORTHAND_DEF, MU_SKIP_DEF_CODE);
        self.primitive(b"toksdef", SHORTHAND_DEF, TOKS_DEF_CODE);
        // dropped: MLTeX, `if mltex_p then ...`
        // §1408
        self.primitive(b"catcode", DEF_CODE, CAT_CODE_BASE);
        self.primitive(b"mathcode", DEF_CODE, MATH_CODE_BASE);
        self.primitive(b"lccode", DEF_CODE, LC_CODE_BASE);
        self.primitive(b"uccode", DEF_CODE, UC_CODE_BASE);
        self.primitive(b"sfcode", DEF_CODE, SF_CODE_BASE);
        self.primitive(b"delcode", DEF_CODE, DEL_CODE_BASE);
        self.primitive(b"textfont", DEF_FAMILY, MATH_FONT_BASE);
        self.primitive(b"scriptfont", DEF_FAMILY, MATH_FONT_BASE + SCRIPT_SIZE);
        self.primitive(b"scriptscriptfont", DEF_FAMILY, MATH_FONT_BASE + SCRIPT_SCRIPT_SIZE);
        // §1428
        self.primitive(b"hyphenation", HYPH_DATA, 0);
        self.primitive(b"patterns", HYPH_DATA, 1);
        // §1432
        self.primitive(b"hyphenchar", ASSIGN_FONT_INT, 0);
        self.primitive(b"skewchar", ASSIGN_FONT_INT, 1);
        self.primitive(b"lpcode", ASSIGN_FONT_INT, LP_CODE_BASE);
        self.primitive(b"rpcode", ASSIGN_FONT_INT, RP_CODE_BASE);
        self.primitive(b"efcode", ASSIGN_FONT_INT, EF_CODE_BASE);
        self.primitive(b"tagcode", ASSIGN_FONT_INT, TAG_CODE);
        self.primitive(b"knbscode", ASSIGN_FONT_INT, KN_BS_CODE_BASE);
        self.primitive(b"stbscode", ASSIGN_FONT_INT, ST_BS_CODE_BASE);
        self.primitive(b"shbscode", ASSIGN_FONT_INT, SH_BS_CODE_BASE);
        self.primitive(b"knbccode", ASSIGN_FONT_INT, KN_BC_CODE_BASE);
        self.primitive(b"knaccode", ASSIGN_FONT_INT, KN_AC_CODE_BASE);
        self.primitive(b"pdfnoligatures", ASSIGN_FONT_INT, NO_LIG_CODE);
        // §1440
        self.primitive(b"batchmode", SET_INTERACTION, BATCH_MODE);
        self.primitive(b"nonstopmode", SET_INTERACTION, NONSTOP_MODE);
        self.primitive(b"scrollmode", SET_INTERACTION, SCROLL_MODE);
        self.primitive(b"errorstopmode", SET_INTERACTION, ERROR_STOP_MODE);
        // §1450
        self.primitive(b"openin", IN_STREAM, 1);
        self.primitive(b"closein", IN_STREAM, 0);
        // §1455
        self.primitive(b"message", MESSAGE, 0);
        self.primitive(b"errmessage", MESSAGE, 1);
        // §1464
        self.primitive(b"lowercase", CASE_SHIFT, LC_CODE_BASE);
        self.primitive(b"uppercase", CASE_SHIFT, UC_CODE_BASE);
        // §1469
        self.primitive(b"show", XRAY, SHOW_CODE);
        self.primitive(b"showbox", XRAY, SHOW_BOX_CODE);
        self.primitive(b"showthe", XRAY, SHOW_THE_CODE);
        self.primitive(b"showlists", XRAY, SHOW_LISTS_CODE);
        // §1524
        self.primitive(b"openout", EXTENSION, OPEN_NODE);
        self.primitive(b"write", EXTENSION, WRITE_NODE);
        self.write_loc = self.cur_val;
        self.primitive(b"closeout", EXTENSION, CLOSE_NODE);
        self.primitive(b"special", EXTENSION, SPECIAL_NODE);
        let s = self.text(self.cur_val); // "special", which `primitive` just made
        self.set_text(FROZEN_SPECIAL, s);
        self.eqtb[FROZEN_SPECIAL] = self.eqtb[self.cur_val];
        self.primitive(b"immediate", EXTENSION, IMMEDIATE_CODE);
        self.primitive(b"setlanguage", EXTENSION, SET_LANGUAGE_CODE);
        self.primitive(b"pdfliteral", EXTENSION, PDF_LITERAL_NODE);
        self.primitive(b"pdfcolorstack", EXTENSION, PDF_COLORSTACK_NODE);
        self.primitive(b"pdfsetmatrix", EXTENSION, PDF_SETMATRIX_NODE);
        self.primitive(b"pdfsave", EXTENSION, PDF_SAVE_NODE);
        self.primitive(b"pdfrestore", EXTENSION, PDF_RESTORE_NODE);
        self.primitive(b"pdfobj", EXTENSION, PDF_OBJ_CODE);
        self.primitive(b"pdfrefobj", EXTENSION, PDF_REFOBJ_NODE);
        self.primitive(b"pdfxform", EXTENSION, PDF_XFORM_CODE);
        self.primitive(b"pdfrefxform", EXTENSION, PDF_REFXFORM_NODE);
        self.primitive(b"pdfximage", EXTENSION, PDF_XIMAGE_CODE);
        self.primitive(b"pdfrefximage", EXTENSION, PDF_REFXIMAGE_NODE);
        self.primitive(b"pdfannot", EXTENSION, PDF_ANNOT_NODE);
        self.primitive(b"pdfstartlink", EXTENSION, PDF_START_LINK_NODE);
        self.primitive(b"pdfendlink", EXTENSION, PDF_END_LINK_NODE);
        self.primitive(b"pdfoutline", EXTENSION, PDF_OUTLINE_CODE);
        self.primitive(b"pdfdest", EXTENSION, PDF_DEST_NODE);
        self.primitive(b"pdfthread", EXTENSION, PDF_THREAD_NODE);
        self.primitive(b"pdfstartthread", EXTENSION, PDF_START_THREAD_NODE);
        self.primitive(b"pdfendthread", EXTENSION, PDF_END_THREAD_NODE);
        self.primitive(b"pdfsavepos", EXTENSION, PDF_SAVE_POS_NODE);
        self.primitive(b"pdfsnaprefpoint", EXTENSION, PDF_SNAP_REF_POINT_NODE);
        self.primitive(b"pdfsnapy", EXTENSION, PDF_SNAPY_NODE);
        self.primitive(b"pdfsnapycomp", EXTENSION, PDF_SNAPY_COMP_NODE);
        self.primitive(b"pdfinfo", EXTENSION, PDF_INFO_CODE);
        self.primitive(b"pdfcatalog", EXTENSION, PDF_CATALOG_CODE);
        self.primitive(b"pdfnames", EXTENSION, PDF_NAMES_CODE);
        self.primitive(b"pdfincludechars", EXTENSION, PDF_INCLUDE_CHARS_CODE);
        self.primitive(b"pdffontattr", EXTENSION, PDF_FONT_ATTR_CODE);
        self.primitive(b"pdfmapfile", EXTENSION, PDF_MAP_FILE_CODE);
        self.primitive(b"pdfmapline", EXTENSION, PDF_MAP_LINE_CODE);
        self.primitive(b"pdftrailer", EXTENSION, PDF_TRAILER_CODE);
        self.primitive(b"pdftrailerid", EXTENSION, PDF_TRAILER_ID_CODE);
        self.primitive(b"pdfresettimer", EXTENSION, RESET_TIMER_CODE);
        self.primitive(b"pdfsetrandomseed", EXTENSION, SET_RANDOM_SEED_CODE);
        self.primitive(b"pdffontexpand", EXTENSION, PDF_FONT_EXPAND_CODE);
        self.primitive(b"pdfglyphtounicode", EXTENSION, PDF_GLYPH_TO_UNICODE_CODE);
        self.primitive(b"pdfnobuiltintounicode", EXTENSION, PDF_NOBUILTIN_TOUNICODE_CODE);
        self.primitive(b"pdfinterwordspaceon", EXTENSION, PDF_INTERWORD_SPACE_ON_NODE);
        self.primitive(b"pdfinterwordspaceoff", EXTENSION, PDF_INTERWORD_SPACE_OFF_NODE);
        self.primitive(b"pdffakespace", EXTENSION, PDF_FAKE_SPACE_NODE);
        self.primitive(b"pdfrunninglinkoff", EXTENSION, PDF_RUNNING_LINK_OFF_NODE);
        self.primitive(b"pdfrunninglinkon", EXTENSION, PDF_RUNNING_LINK_ON_NODE);
        self.primitive(b"pdfspacefont", EXTENSION, PDF_SPACE_FONT_CODE);
        // §1899
        self.primitive(b"synctex", ASSIGN_INT, INT_BASE + SYNCTEX_CODE);
        self.no_new_control_sequence = true;
    }

    /// §1649-§1864 Generate all eTeX primitives, in §1648.
    fn main_etex_primitives(&mut self) {
        // §1649
        self.primitive(b"lastnodetype", LAST_ITEM, LAST_NODE_TYPE_CODE);
        self.primitive(b"eTeXversion", LAST_ITEM, ETEX_VERSION_CODE);
        self.primitive(b"eTeXrevision", CONVERT, ETEX_REVISION_CODE);
        // §1657
        self.primitive(b"everyeof", ASSIGN_TOKS, EVERY_EOF_LOC);
        self.primitive(b"tracingassigns", ASSIGN_INT, INT_BASE + TRACING_ASSIGNS_CODE);
        self.primitive(b"tracinggroups", ASSIGN_INT, INT_BASE + TRACING_GROUPS_CODE);
        self.primitive(b"tracingifs", ASSIGN_INT, INT_BASE + TRACING_IFS_CODE);
        self.primitive(b"tracingscantokens", ASSIGN_INT, INT_BASE + TRACING_SCAN_TOKENS_CODE);
        self.primitive(b"tracingnesting", ASSIGN_INT, INT_BASE + TRACING_NESTING_CODE);
        self.primitive(b"predisplaydirection", ASSIGN_INT, INT_BASE + PRE_DISPLAY_DIRECTION_CODE);
        self.primitive(b"lastlinefit", ASSIGN_INT, INT_BASE + LAST_LINE_FIT_CODE);
        self.primitive(b"savingvdiscards", ASSIGN_INT, INT_BASE + SAVING_VDISCARDS_CODE);
        self.primitive(b"savinghyphcodes", ASSIGN_INT, INT_BASE + SAVING_HYPH_CODES_CODE);
        self.primitive(b"ignoreprimitiveerror", ASSIGN_INT, INT_BASE + IGNORE_PRIMITIVE_ERROR_CODE);
        // §1663
        self.primitive(b"currentgrouplevel", LAST_ITEM, CURRENT_GROUP_LEVEL_CODE);
        self.primitive(b"currentgrouptype", LAST_ITEM, CURRENT_GROUP_TYPE_CODE);
        // §1666
        self.primitive(b"currentiflevel", LAST_ITEM, CURRENT_IF_LEVEL_CODE);
        self.primitive(b"currentiftype", LAST_ITEM, CURRENT_IF_TYPE_CODE);
        self.primitive(b"currentifbranch", LAST_ITEM, CURRENT_IF_BRANCH_CODE);
        // §1669
        self.primitive(b"fontcharwd", LAST_ITEM, FONT_CHAR_WD_CODE);
        self.primitive(b"fontcharht", LAST_ITEM, FONT_CHAR_HT_CODE);
        self.primitive(b"fontchardp", LAST_ITEM, FONT_CHAR_DP_CODE);
        self.primitive(b"fontcharic", LAST_ITEM, FONT_CHAR_IC_CODE);
        // §1672
        self.primitive(b"parshapelength", LAST_ITEM, PAR_SHAPE_LENGTH_CODE);
        self.primitive(b"parshapeindent", LAST_ITEM, PAR_SHAPE_INDENT_CODE);
        self.primitive(b"parshapedimen", LAST_ITEM, PAR_SHAPE_DIMEN_CODE);
        // §1675
        self.primitive(b"showgroups", XRAY, SHOW_GROUPS);
        // §1684
        self.primitive(b"showtokens", XRAY, SHOW_TOKENS);
        // §1686
        self.primitive(b"unexpanded", THE, 1);
        self.primitive(b"detokenize", THE, SHOW_TOKENS);
        // §1689
        self.primitive(b"showifs", XRAY, SHOW_IFS);
        // §1692
        self.primitive(b"interactionmode", SET_PAGE_INT, 2);
        // §1697
        self.primitive(b"middle", LEFT_RIGHT, MIDDLE_NOAD);
        // §1701
        self.primitive(b"TeXXeTstate", ASSIGN_INT, ETEX_STATE_BASE + TEXXET_CODE);
        self.primitive(b"beginL", VALIGN, BEGIN_L_CODE);
        self.primitive(b"endL", VALIGN, END_L_CODE);
        self.primitive(b"beginR", VALIGN, BEGIN_R_CODE);
        self.primitive(b"endR", VALIGN, END_R_CODE);
        // §1747
        self.primitive(b"scantokens", INPUT, 2);
        // §1759
        self.primitive(b"readline", READ_TO_CS, 1);
        // §1762
        self.primitive(b"unless", EXPAND_AFTER, 1);
        self.primitive(b"ifdefined", IF_TEST, IF_DEF_CODE);
        self.primitive(b"ifcsname", IF_TEST, IF_CS_CODE);
        self.primitive(b"iffontchar", IF_TEST, IF_FONT_CHAR_CODE);
        self.primitive(b"ifincsname", IF_TEST, IF_IN_CSNAME_CODE);
        self.primitive(b"ifpdfabsnum", IF_TEST, IF_PDFABS_NUM_CODE);
        self.primitive(b"ifpdfabsdim", IF_TEST, IF_PDFABS_DIM_CODE);
        // §1770
        self.primitive(b"protected", PREFIX, 8);
        // §1778
        self.primitive(b"numexpr", LAST_ITEM, ETEX_EXPR - INT_VAL + INT_VAL);
        self.primitive(b"dimexpr", LAST_ITEM, ETEX_EXPR - INT_VAL + DIMEN_VAL);
        self.primitive(b"glueexpr", LAST_ITEM, ETEX_EXPR - INT_VAL + GLUE_VAL);
        self.primitive(b"muexpr", LAST_ITEM, ETEX_EXPR - INT_VAL + MU_VAL);
        // §1801
        self.primitive(b"gluestretchorder", LAST_ITEM, GLUE_STRETCH_ORDER_CODE);
        self.primitive(b"glueshrinkorder", LAST_ITEM, GLUE_SHRINK_ORDER_CODE);
        self.primitive(b"gluestretch", LAST_ITEM, GLUE_STRETCH_CODE);
        self.primitive(b"glueshrink", LAST_ITEM, GLUE_SHRINK_CODE);
        // §1805
        self.primitive(b"mutoglue", LAST_ITEM, MU_TO_GLUE_CODE);
        self.primitive(b"gluetomu", LAST_ITEM, GLUE_TO_MU_CODE);
        // §1809
        self.primitive(b"marks", MARK, MARKS_CODE);
        self.primitive(b"topmarks", TOP_BOT_MARK, TOP_MARK_CODE + MARKS_CODE);
        self.primitive(b"firstmarks", TOP_BOT_MARK, FIRST_MARK_CODE + MARKS_CODE);
        self.primitive(b"botmarks", TOP_BOT_MARK, BOT_MARK_CODE + MARKS_CODE);
        self.primitive(b"splitfirstmarks", TOP_BOT_MARK, SPLIT_FIRST_MARK_CODE + MARKS_CODE);
        self.primitive(b"splitbotmarks", TOP_BOT_MARK, SPLIT_BOT_MARK_CODE + MARKS_CODE);
        // §1861
        self.primitive(b"pagediscards", UN_VBOX, LAST_BOX_CODE);
        self.primitive(b"splitdiscards", UN_VBOX, VSPLIT_CODE);
        // §1864
        self.primitive(b"interlinepenalties", SET_SHAPE, INTER_LINE_PENALTIES_LOC);
        self.primitive(b"clubpenalties", SET_SHAPE, CLUB_PENALTIES_LOC);
        self.primitive(b"widowpenalties", SET_SHAPE, WIDOW_PENALTIES_LOC);
        self.primitive(b"displaywidowpenalties", SET_SHAPE, DISPLAY_WIDOW_PENALTIES_LOC);
    }

    /// §1518
    /// `debug` only, not compiled in the oracle.
    pub fn debug_help(&mut self) {}

    // ---- External functions: the C side of web2c and pdfTeX, called by WEB. ----

    /// External: utils.c `makepdftexbanner`, which sets pdftex_banner, once.  `setjobid` in pdfship.rs builds on the same
    /// constants.
    pub fn make_pdftex_banner(&mut self) {
        if self.v_main.pdftex_banner_init {
            return;
        }
        // "%s%s %s", ptexbanner, versionstring, kpathsea_version_string
        let (n, m, k) = (BANNER.len() as i32, VERSION_STRING.len() as i32, KPATHSEA_VERSION_STRING.len() as i32);
        self.str_room(n + m + 1 + k);
        for &c in BANNER {
            self.append_char(c as i32);
        }
        for &c in VERSION_STRING {
            self.append_char(c as i32);
        }
        self.append_char(b' ' as i32);
        for &c in KPATHSEA_VERSION_STRING {
            self.append_char(c as i32);
        }
        self.pdftex_banner = self.make_string();
        self.v_main.pdftex_banner_init = true;
    }

    /// External: texmfmp.c `readtcxfile`.  The TCX reader is dropped, docs/PORTING.md section 19: this
    /// finds the file as C does, since §61 and §562 print the name found, and installs the table of
    /// `cp227.tcx`, the one every TeX Live format is built with, whatever the file says.
    /// The CLI warns about any other name.
    pub fn read_tcx_file(&mut self) {
        let Some(name) = self.host.translate_filename.take() else {
            return;
        };
        let name = if rstex_kpse::names::find_suffix(&name).is_none() { [&name[..], b".tcx"].concat() } else { name };
        let Some(found) = self.host.world.lookup(&name, rstex_kpse::KPSE_WEB2C_FORMAT, true) else {
            // WARNING1 ("Could not open char translation file `%s'"): the CLI has checked, on its way
            // here, and said so; translate_filename stays NULL as in C.
            return;
        };
        self.host.translate_filename = Some(found);
        self.main_install_cp227();
    }

    /// The contents of `cp227.tcx`: tab, line feed and vertical tab and all of 128-255 are printable,
    /// and each maps to itself.
    fn main_install_cp227(&mut self) {
        let mut one = |t: &mut Tex, c: i32| {
            t.xord.put(c, c);
            t.xchr.put(c, c);
            t.xprn.put(c, 1);
        };
        for c in 0x09..=0x0b {
            one(self, c);
        }
        for c in 0x80..=0xff {
            one(self, c);
        }
    }

    /// External: debug only: nothing reaches it, since `debug_help` is not compiled.
    pub fn dump_core(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toks::testkit::*;

    /// A machine after `get_strings_started`, ready for `primitive`.
    fn prim_machine() -> &'static mut Tex {
        let t = machine();
        t.pool_ptr = 0;
        t.str_ptr = 0;
        t.pool_size = 1 << 20;
        t.string_vacancies = 8000;
        t.max_strings = 100000;
        t.buf_size = 10000;
        t.eqtb_top = EQTB_CAP as i32 - 1;
        t.prim_used = PRIM_SIZE; // section 277
        assert!(t.get_strings_started());
        t
    }

    fn lookup(t: &mut Tex, name: &[u8]) -> i32 {
        if let [c] = *name {
            return c as i32 + SINGLE_BASE;
        }
        for (k, &c) in name.iter().enumerate() {
            t.buffer.put(k as i32, c as i32);
        }
        t.no_new_control_sequence = true;
        t.id_lookup(0, name.len() as i32)
    }

    /// primitives.tsv comes from the WEB, by an evaluator that is not this file's: every line is here.
    #[test]
    fn init_prim_agrees_with_the_web() {
        let t = prim_machine();
        t.init_prim();
        t.no_new_control_sequence = false;
        t.first = 0;
        t.main_etex_primitives();
        let data = include_str!("../testdata/primitives.tsv");
        let mut n = 0;
        for l in data.lines() {
            let f: Vec<&str> = l.split('\t').collect();
            let (name, cmd, chr): (&str, i32, i32) = (f[0], f[1].parse().unwrap(), f[2].parse().unwrap());
            let p = lookup(t, name.as_bytes());
            assert_ne!(p, UNDEFINED_CONTROL_SEQUENCE, "{} is missing", name);
            assert_eq!(t.eq_type(p), cmd, "cmd of {}", name);
            assert_eq!(t.equiv(p), chr, "chr of {}", name);
            assert_eq!(t.eq_level(p), LEVEL_ONE);
            n += 1;
        }
        assert!(n > 350, "{} primitives", n);
        Tex::dealloc(t);
    }

    /// Every primitive the oracle has, tests/primitives.json, but the encTeX and MLTeX ones; and no more:
    /// the count is the hash's, so an extra one shows.
    #[test]
    fn init_prim_has_the_oracles_primitives() {
        let t = prim_machine();
        t.init_prim();
        t.no_new_control_sequence = false;
        t.first = 0;
        t.main_etex_primitives();
        let json = include_str!("../../../tests/primitives.json");
        let mut names = 0;
        let mut rest = json;
        let mut multi = 0;
        while let Some(i) = rest.find("\"name\": \"") {
            rest = &rest[i + 9..];
            let e = rest.find('"').unwrap();
            let name = rest[..e].replace("\\\\", "\\");
            rest = &rest[e..];
            if ["endmubyte", "mubyte", "mubytein", "mubytelog", "mubyteout", "noconvert", "specialout", "xchrcode", "xordcode", "xprncode"].contains(&name.as_str()) {
                continue; // encTeX
            }
            if ["charsubdef", "charsubdefmin", "charsubdefmax", "tracingcharsubdef"].contains(&name.as_str()) {
                continue; // MLTeX
            }
            names += 1;
            let p = lookup(t, name.as_bytes());
            assert_ne!(p, UNDEFINED_CONTROL_SEQUENCE, "\\{} is missing", name);
            if name.len() != 1 {
                multi += 1;
            } else {
                assert_ne!(t.eq_type(p), UNDEFINED_CS, "\\{} is missing", name);
            }
        }
        assert!(names > 500, "{} names", names);
        assert_eq!(t.hash_high, multi, "control sequences made, against the oracle's list");
        Tex::dealloc(t);
    }

    #[test]
    fn init_prim_frozen_locations() {
        let t = prim_machine();
        t.init_prim();
        let same = |t: &Tex, frozen: i32, name: &[u8]| {
            let s = t.text(frozen);
            assert_eq!(t.str_pool.range(t.str_start[s], t.str_start[s + 1]), name);
        };
        for (frozen, name) in [
            (FROZEN_END_GROUP, &b"endgroup"[..]),
            (FROZEN_RELAX, b"relax"),
            (FROZEN_FI, b"fi"),
            (FROZEN_NULL_FONT, b"nullfont"),
            (FROZEN_CR, b"cr"),
            (FROZEN_RIGHT, b"right"),
            (FROZEN_SPECIAL, b"special"),
            (FROZEN_END_TEMPLATE, b"endtemplate"),
            (FROZEN_ENDV, b"endtemplate"),
        ] {
            same(t, frozen, name);
        }
        assert_eq!(t.eq_type(FROZEN_RELAX), RELAX);
        assert_eq!(t.equiv(FROZEN_RELAX), 256);
        assert_eq!(t.eq_type(FROZEN_END_TEMPLATE), END_TEMPLATE);
        assert_eq!(t.equiv(FROZEN_END_TEMPLATE), NULL_LIST);
        assert_eq!(t.eq_type(FROZEN_ENDV), ENDV);
        assert_eq!(t.par_token, CS_TOKEN_FLAG + t.par_loc);
        assert_eq!(t.eq_type(t.par_loc), PAR_END);
        assert_eq!(t.eq_type(t.write_loc), EXTENSION);
        assert_eq!(t.equiv(t.write_loc), WRITE_NODE);
        assert!(t.no_new_control_sequence);
        Tex::dealloc(t);
    }

    #[test]
    fn const_chk_stops_at_inf_sup_and_the_capacity() {
        assert_eq!(main_const_chk(5, 10, 100, 1000), 10);
        assert_eq!(main_const_chk(50, 10, 100, 1000), 50);
        assert_eq!(main_const_chk(500, 10, 100, 1000), 100);
        assert_eq!(main_const_chk(500, 10, 1000, 101), 100, "an array of 101 has 100 as its last index");
        assert_eq!(main_const_chk(SUP_BUF_SIZE, INF_BUF_SIZE, SUP_BUF_SIZE, BUF_CAP), BUF_CAP as i32 - 1);
    }

    #[test]
    fn the_banner_is_made_once() {
        let t = prim_machine();
        t.make_pdftex_banner();
        let s = t.pdftex_banner;
        assert!(s > 0);
        let text = t.str_pool.range(t.str_start[s], t.str_start[s + 1]).to_vec();
        assert!(text.starts_with(b"This is pdfTeX, Version 3.141592653-2.6-1.40.29"), "{:?}", String::from_utf8_lossy(&text));
        assert!(text.ends_with(b" kpathsea version 6.4.2"));
        let n = t.str_ptr;
        t.make_pdftex_banner();
        assert_eq!((t.pdftex_banner, t.str_ptr), (s, n));
        Tex::dealloc(t);
    }

    #[test]
    fn cp227_makes_8bit_and_three_controls_printable() {
        let t = machine();
        for i in 0..256 {
            t.xchr.put(i, i);
            t.xord.put(i, i);
            t.xprn.put(i, 0);
        }
        t.main_install_cp227();
        let printable: Vec<i32> = (0..256).filter(|&i| t.xprn.at(i) == 1).collect();
        let want: Vec<i32> = [9, 10, 11].into_iter().chain(0x80..=0xff).collect();
        assert_eq!(printable, want);
        Tex::dealloc(t);
    }
}
