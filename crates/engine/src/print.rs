//! §1-37 Parts 1-3: introduction, character set, input and output; §54-71 Part 5: on-line and off-line printing.
//!
//! `initialize` holds every `@<Set initial values of key variables@>` and `@<Initialize table entries@>`,
//! from all parts, in tangle's order.  It is split in three only so that a long run of assignments reads
//! better, and so that tests can run the parts that do not touch the clock.
//!
//! DROPPED §35: `input_ln` is in host.rs; the module's other use is in `pseudo_input`, in etex.rs.
//! DROPPED §78: forward declarations; all procedures are methods of `Tex`.
//! DROPPED §184: `debug` only, not compiled in the oracle.
//! DROPPED §966: `info(omit_template):=end_template_token` is done by `toks_init`: docs/PORTING.md section 15.
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
    /// §4
    pub fn initialize(&mut self) {
        // §19, §181, §1104 Local variables for initialization: `i`, `k` and `z`, local to each loop.
        // §8 Initialize whatever TeX might access
        self.print_init_key_variables_to_1571();
        // §1584 Set initial values of key variables
        self.seconds_and_micros();
        self.init_start_time();
        self.print_init_key_variables_from_1629();
        if self.ini_version {
            self.print_init_table_entries();
        }
        // §1903 Initialize whatever TeX might access
        self.synctexoffset = INT_BASE + SYNCTEX_CODE;
    }

    /// §21-§1571 Set initial values of key variables, the part before the clock is read in §1584.
    pub fn print_init_key_variables_to_1571(&mut self) {
        // §21 Set initial values of key variables: the printable part of `xchr` is the identity
        for i in 0o40..=0o176 {
            self.xchr.put(i, i);
        }
        // §23 Initialize xchr to the identity mapping
        for i in 0..=0o37 {
            self.xchr.put(i, i);
        }
        for i in 0o177..=0o377 {
            self.xchr.put(i, i);
        }
        self.message_printing = false;
        // §24
        for i in FIRST_TEXT_CHAR..=LAST_TEXT_CHAR {
            self.xord.put(i, INVALID_CODE);
        }
        for i in 0o200..=0o377 {
            let c = self.xchr.at(i);
            self.xord.put(c, i);
        }
        for i in 0..=0o176 {
            let c = self.xchr.at(i);
            self.xord.put(c, i);
        }
        // Set xprn for printable ASCII, unless eight_bit_p is set.
        for i in 0..=255 {
            let p = self.eight_bit_p != 0 || (i >= b' ' as i32 && i <= b'~' as i32);
            self.xprn.put(i, p as i32);
        }
        // Updates the xchr, xord and xprn arrays from the provided translate_filename.
        if self.host.translate_filename.is_some() {
            self.read_tcx_file();
        }
        // §74
        if self.interaction_option == UNSPECIFIED_MODE {
            self.interaction = ERROR_STOP_MODE;
        } else {
            self.interaction = self.interaction_option;
        }
        // §77
        self.deletions_allowed = true;
        self.set_box_allowed = true;
        self.error_count = 0;
        // §80
        self.help_ptr = 0;
        self.use_err_help = false;
        // §97
        self.interrupt = 0;
        self.OK_to_interrupt = true;
        // §118
        self.two_to_the[0] = 1;
        for k in 1..=30 {
            self.two_to_the[k] = 2 * self.two_to_the[k - 1];
        }
        self.spec_log[1] = 93032640;
        self.spec_log[2] = 38612034;
        self.spec_log[3] = 17922280;
        self.spec_log[4] = 8662214;
        self.spec_log[5] = 4261238;
        self.spec_log[6] = 2113709;
        self.spec_log[7] = 1052693;
        self.spec_log[8] = 525315;
        self.spec_log[9] = 262400;
        self.spec_log[10] = 131136;
        self.spec_log[11] = 65552;
        self.spec_log[12] = 32772;
        self.spec_log[13] = 16385;
        for k in 14..=27 {
            self.spec_log[k] = self.two_to_the[27 - k];
        }
        self.spec_log[28] = 1;
        // §233
        self.nest_ptr = 0;
        self.max_nest_stack = 0;
        self.set_mode(VMODE);
        self.set_head(CONTRIB_HEAD);
        self.set_tail(CONTRIB_HEAD);
        self.set_eTeX_aux(NULL);
        self.save_tail = NULL;
        self.set_prev_depth(IGNORE_DEPTH);
        self.set_mode_line(0);
        self.set_prev_graf(0);
        self.shown_mode = 0;
        // The following piece of code is a copy of module 991:
        self.page_contents = EMPTY;
        self.page_tail = PAGE_HEAD;
        self.last_glue = MAX_HALFWORD;
        self.last_penalty = 0;
        self.last_kern = 0;
        self.last_node_type = -1;
        self.set_page_depth(0);
        self.page_max_depth = 0;
        // §272
        for k in INT_BASE..=EQTB_SIZE {
            self.xeq_level[k] = LEVEL_ONE;
        }
        // §276
        self.no_new_control_sequence = true;
        self.set_prim_next(0, 0);
        self.set_prim_text(0, 0);
        for k in 1..=PRIM_SIZE {
            self.prim[k] = self.prim[0];
        }
        // §294
        self.save_ptr = 0;
        self.cur_level = LEVEL_ONE;
        self.cur_group = BOTTOM_LEVEL;
        self.cur_boundary = 0;
        self.max_save_stack = 0;
        // §309
        self.mag_set = 0;
        // §390
        self.is_in_csname = false;
        // §409
        self.set_top_mark(NULL);
        self.set_first_mark(NULL);
        self.set_bot_mark(NULL);
        self.set_split_first_mark(NULL);
        self.set_split_bot_mark(NULL);
        // §465
        self.cur_val = 0;
        self.cur_val_level = INT_VAL;
        self.radix = 0;
        self.cur_order = NORMAL;
        // §507
        for k in 0..=16 {
            self.read_open[k] = CLOSED;
        }
        // §516
        self.cond_ptr = NULL;
        self.if_limit = NORMAL;
        self.cur_if = 0;
        self.if_line = 0;
        // §582
        self.null_character.b0 = MIN_QUARTERWORD;
        self.null_character.b1 = MIN_QUARTERWORD;
        self.null_character.b2 = MIN_QUARTERWORD;
        self.null_character.b3 = MIN_QUARTERWORD;
        // §620
        self.total_pages = 0;
        self.max_v = 0;
        self.max_h = 0;
        self.max_push = 0;
        self.last_bop = -1;
        self.doing_leaders = false;
        self.dead_cycles = 0;
        self.cur_s = -1;
        // §623
        self.half_buf = self.dvi_buf_size / 2;
        self.dvi_limit = self.dvi_buf_size;
        self.dvi_ptr = 0;
        self.dvi_offset = 0;
        self.dvi_gone = 0;
        // §633
        self.down_ptr = NULL;
        self.right_ptr = NULL;
        // §677
        self.pdf_mem_ptr = 1;
        self.pdf_mem_size = INF_PDF_MEM_SIZE;
        // §681
        self.pdf_gone = 0;
        self.pdf_os_mode = false;
        self.pdf_ptr = 0;
        self.pdf_op_ptr = 0;
        self.pdf_os_ptr = 0;
        self.pdf_os_cur_objnum = 0;
        self.pdf_os_cntr = 0;
        self.pdf_buf_size = PDF_OP_BUF_SIZE;
        self.pdf_os_buf_size = INF_PDF_OS_BUF_SIZE;
        self.pdf_buf_is_os = false;
        self.pdf_seek_write_length = false;
        self.zip_write_state = NO_ZIP;
        self.pdf_version_written = false;
        self.fixed_pdfoutput_set = false;
        self.fixed_pdf_draftmode_set = false;
        // §688
        self.one_bp = 65782;
        self.one_hundred_bp = 6578176;
        self.one_hundred_inch = 473628672;
        self.one_inch = 226;
        self.ten_pow[0] = 1;
        for i in 1..=9 {
            self.ten_pow[i] = 10 * self.ten_pow[i - 1];
        }
        self.init_pdf_output = false;
        // §697
        self.obj_ptr = 0;
        self.sys_obj_ptr = 0;
        self.obj_tab_size = INF_OBJ_TAB_SIZE;
        self.dest_names_size = INF_DEST_NAMES_SIZE;
        for k in 1..=HEAD_TAB_MAX {
            self.head_tab[k] = 0;
        }
        self.pdf_box_spec_media = 1;
        self.pdf_box_spec_crop = 2;
        self.pdf_box_spec_bleed = 3;
        self.pdf_box_spec_trim = 4;
        self.pdf_box_spec_art = 5;
        self.pdf_dummy_font = NULL_FONT;
        // §709
        self.pdf_resname_prefix = 0;
        self.last_tokens_string = 0;
        // §711
        self.vf_nf = 0;
        // §724
        self.vf_cur_s = 0;
        self.vf_stack_ptr = 0;
        // §820
        self.adjust_tail = NULL;
        self.last_badness = 0;
        // §830
        self.pre_adjust_tail = NULL;
        // §838
        self.pack_begin_line = 0;
        // §861
        self.empty_field = Word::hh(EMPTY, NULL);
        self.null_delimiter.b0 = 0;
        self.null_delimiter.b1 = MIN_QUARTERWORD;
        self.null_delimiter.b2 = 0;
        self.null_delimiter.b3 = MIN_QUARTERWORD;
        // §947
        self.align_ptr = NULL;
        self.cur_align = NULL;
        self.cur_span = NULL;
        self.cur_loop = NULL;
        self.cur_head = NULL;
        self.cur_tail = NULL;
        self.cur_pre_head = NULL;
        self.cur_pre_tail = NULL;
        // §1105
        for z in 0..=self.hyph_size {
            self.hyph_word[z] = 0;
            self.hyph_list[z] = NULL;
            self.hyph_link[z] = 0;
        }
        self.hyph_count = 0;
        self.hyph_next = HYPH_PRIME + 1;
        if self.hyph_next > self.hyph_size {
            self.hyph_next = HYPH_PRIME;
        }
        // §1167
        self.output_active = false;
        self.output_can_end = false;
        self.insert_penalties = 0;
        // §1210
        self.ligature_present = false;
        self.cancel_boundary = false;
        self.lft_hit = false;
        self.rt_hit = false;
        self.ins_disc = false;
        // §1445
        self.after_token = 0;
        // §1460
        self.long_help_seen = false;
        // §1478
        self.format_ident = 0;
        // §1523
        for k in 0..=17 {
            self.write_open[k] = false;
        }
        // §1551
        self.alt_rule = NULL;
        self.warn_pdfpagebox = true;
        // §1571
        self.count_do_snapy = 0;
    }

    /// §1629-§1879 Set initial values of key variables, the part after the clock is read in §1584.
    pub fn print_init_key_variables_from_1629(&mut self) {
        // §1629
        self.pdf_first_outline = 0;
        self.pdf_last_outline = 0;
        self.pdf_parent_outline = 0;
        self.pdf_obj_count = 0;
        self.pdf_xform_count = 0;
        self.pdf_ximage_count = 0;
        self.pdf_dest_names_ptr = 0;
        self.pdf_info_toks = NULL;
        self.pdf_catalog_toks = NULL;
        self.pdf_names_toks = NULL;
        self.pdf_catalog_openaction = 0;
        self.pdf_trailer_toks = NULL;
        self.pdf_trailer_id_toks = NULL;
        self.gen_faked_interword_space = false;
        self.gen_running_link = true;
        self.pdf_space_font_name = S_PDFTEXSPACE;
        // §1634
        self.pdf_link_stack_ptr = 0;
        // §1706
        self.LR_ptr = NULL;
        self.LR_problems = 0;
        self.cur_dir = LEFT_TO_RIGHT;
        // §1751
        self.pseudo_files = NULL;
        // §1817
        self.set_sa_mark(NULL);
        self.sa_null = Word::hh(NULL, NULL);
        // §1836
        self.sa_chain = NULL;
        self.sa_level = LEVEL_ZERO;
        // §1860
        self.set_page_disc(NULL);
        self.set_split_disc(NULL);
        // §1868
        self.edit_name_start = 0;
        self.stop_at_space = true;
        self.halting_on_error_p = false;
        // §1879
        self.expand_depth_count = 0;
        // dropped: MLTeX, §1883
    }

    /// `Init ... Tini` in §8: §182-§1854 Initialize table entries (done by INITEX only).
    pub fn print_init_table_entries(&mut self) {
        // docs/PORTING.md section 20: the token heap's reserved lists come first.
        self.toks_init();
        // §182
        for k in MEM_BOT + 1..=LO_MEM_STAT_MAX {
            self.mem[k].set_sc(0);
        }
        let mut k = MEM_BOT;
        while k <= LO_MEM_STAT_MAX {
            self.set_glue_ref_count(k, NULL + 1);
            self.set_stretch_order(k, NORMAL);
            self.set_shrink_order(k, NORMAL);
            k = k + GLUE_SPEC_SIZE;
        }
        self.set_stretch(FIL_GLUE, UNITY);
        self.set_stretch_order(FIL_GLUE, FIL);
        self.set_stretch(FILL_GLUE, UNITY);
        self.set_stretch_order(FILL_GLUE, FILL);
        self.set_stretch(SS_GLUE, UNITY);
        self.set_stretch_order(SS_GLUE, FIL);
        self.set_shrink(SS_GLUE, UNITY);
        self.set_shrink_order(SS_GLUE, FIL);
        self.set_stretch(FIL_NEG_GLUE, -UNITY);
        self.set_stretch_order(FIL_NEG_GLUE, FIL);
        self.rover = LO_MEM_STAT_MAX + 1;
        self.set_link(self.rover, EMPTY_FLAG);
        self.set_node_size(self.rover, 1000);
        self.set_llink(self.rover, self.rover);
        self.set_rlink(self.rover, self.rover);
        self.lo_mem_max = self.rover + 1000;
        self.set_link(self.lo_mem_max, NULL);
        self.set_info(self.lo_mem_max, NULL);
        for k in HI_MEM_STAT_MIN..=MEM_TOP {
            self.mem[k] = self.mem[self.lo_mem_max];
        }
        // §973 Initialize the special list heads and constant nodes
        self.set_link(END_SPAN, WEB_MAX_QUARTERWORD + 1);
        self.set_info(END_SPAN, NULL);
        // §996
        self.set_ty(LAST_ACTIVE, HYPHENATED);
        self.set_line_number(LAST_ACTIVE, MAX_HALFWORD);
        self.set_subtype(LAST_ACTIVE, 0);
        // §1158
        self.set_subtype(PAGE_INS_HEAD, 255);
        self.set_ty(PAGE_INS_HEAD, SPLIT_UP);
        self.set_link(PAGE_INS_HEAD, PAGE_INS_HEAD);
        // §1165
        self.set_ty(PAGE_HEAD, GLUE_NODE);
        self.set_subtype(PAGE_HEAD, NORMAL);
        self.avail = NULL;
        self.mem_end = MEM_TOP;
        self.hi_mem_min = HI_MEM_STAT_MIN;
        self.var_used = LO_MEM_STAT_MAX + 1 - MEM_BOT;
        self.dyn_used = HI_MEM_STAT_USAGE;
        // §240.  The loop stops at EQTB_SIZE: run-time entries above it start as §279 makes them.
        self.set_eq_type(UNDEFINED_CONTROL_SEQUENCE, UNDEFINED_CS);
        self.set_equiv(UNDEFINED_CONTROL_SEQUENCE, NULL);
        self.set_eq_level(UNDEFINED_CONTROL_SEQUENCE, LEVEL_ZERO);
        for k in ACTIVE_BASE..=EQTB_SIZE {
            self.eqtb[k] = self.eqtb[UNDEFINED_CONTROL_SEQUENCE];
        }
        // §246
        self.set_equiv(GLUE_BASE, ZERO_GLUE);
        self.set_eq_level(GLUE_BASE, LEVEL_ONE);
        self.set_eq_type(GLUE_BASE, GLUE_REF);
        for k in GLUE_BASE + 1..=LOCAL_BASE - 1 {
            self.eqtb[k] = self.eqtb[GLUE_BASE];
        }
        self.set_glue_ref_count(ZERO_GLUE, self.glue_ref_count(ZERO_GLUE) + LOCAL_BASE - GLUE_BASE);
        // §250
        self.set_par_shape_ptr(NULL);
        self.set_eq_type(PAR_SHAPE_LOC, SHAPE_REF);
        self.set_eq_level(PAR_SHAPE_LOC, LEVEL_ONE);
        for k in ETEX_PEN_BASE..=ETEX_PENS - 1 {
            self.eqtb[k] = self.eqtb[PAR_SHAPE_LOC];
        }
        for k in OUTPUT_ROUTINE_LOC..=TOKS_BASE + 255 {
            self.eqtb[k] = self.eqtb[UNDEFINED_CONTROL_SEQUENCE];
        }
        self.set_box_reg(0, NULL);
        self.set_eq_type(BOX_BASE, BOX_REF);
        self.set_eq_level(BOX_BASE, LEVEL_ONE);
        for k in BOX_BASE + 1..=BOX_BASE + 255 {
            self.eqtb[k] = self.eqtb[BOX_BASE];
        }
        self.set_cur_font(NULL_FONT);
        self.set_eq_type(CUR_FONT_LOC, DATA);
        self.set_eq_level(CUR_FONT_LOC, LEVEL_ONE);
        for k in MATH_FONT_BASE..=MATH_FONT_BASE + 47 {
            self.eqtb[k] = self.eqtb[CUR_FONT_LOC];
        }
        self.set_equiv(CAT_CODE_BASE, 0);
        self.set_eq_type(CAT_CODE_BASE, DATA);
        self.set_eq_level(CAT_CODE_BASE, LEVEL_ONE);
        for k in CAT_CODE_BASE + 1..=INT_BASE - 1 {
            self.eqtb[k] = self.eqtb[CAT_CODE_BASE];
        }
        for k in 0..=255 {
            self.set_cat_code(k, OTHER_CHAR);
            self.set_math_code(k, k);
            self.set_sf_code(k, 1000);
        }
        self.set_cat_code(CARRIAGE_RETURN, CAR_RET);
        self.set_cat_code(b' ' as i32, SPACER);
        self.set_cat_code(b'\\' as i32, ESCAPE);
        self.set_cat_code(b'%' as i32, COMMENT);
        self.set_cat_code(INVALID_CODE, INVALID_CHAR);
        self.set_cat_code(NULL_CODE, IGNORE);
        for k in b'0' as i32..=b'9' as i32 {
            self.set_math_code(k, k + VAR_CODE);
        }
        let case = b'a' as i32 - b'A' as i32;
        for k in b'A' as i32..=b'Z' as i32 {
            self.set_cat_code(k, LETTER);
            self.set_cat_code(k + case, LETTER);
            self.set_math_code(k, k + VAR_CODE + 0x100);
            self.set_math_code(k + case, k + case + VAR_CODE + 0x100);
            self.set_lc_code(k, k + case);
            self.set_lc_code(k + case, k + case);
            self.set_uc_code(k, k);
            self.set_uc_code(k + case, k);
            self.set_sf_code(k, 999);
        }
        // §258
        for k in INT_BASE..=DEL_CODE_BASE - 1 {
            self.eqtb[k].set_int(0);
        }
        self.set_char_sub_def_min(256);
        self.set_char_sub_def_max(-1);
        self.set_mag(1000);
        self.set_tolerance(10000);
        self.set_hang_after(1);
        self.set_max_dead_cycles(25);
        self.set_escape_char(b'\\' as i32);
        self.set_end_line_char(CARRIAGE_RETURN);
        for k in 0..=255 {
            self.set_del_code(k, -1);
        }
        self.set_del_code(b'.' as i32, 0);
        self.set_show_stream(-1);
        // §268
        for k in DIMEN_BASE..=EQTB_SIZE {
            self.eqtb[k].set_sc(0);
        }
        // §277
        self.prim_used = PRIM_SIZE;
        self.hash_used = FROZEN_CONTROL_SEQUENCE;
        self.hash_high = 0;
        self.cs_count = 0;
        self.set_eq_type(FROZEN_DONT_EXPAND, DONT_EXPAND);
        self.set_text(FROZEN_DONT_EXPAND, S_NOTEXPANDED);
        self.set_eq_type(FROZEN_PRIMITIVE, IGNORE_SPACES);
        self.set_equiv(FROZEN_PRIMITIVE, 1);
        self.set_eq_level(FROZEN_PRIMITIVE, LEVEL_ONE);
        self.set_text(FROZEN_PRIMITIVE, S_PDFPRIMITIVE);
        // §672
        self.set_pdf_h_origin((self.one_hundred_inch + 50) / 100);
        self.set_pdf_v_origin((self.one_hundred_inch + 50) / 100);
        self.set_pdf_compress_level(9);
        self.set_pdf_objcompresslevel(0);
        self.set_pdf_decimal_digits(3);
        self.set_pdf_image_resolution(72);
        self.set_pdf_major_version(1);
        self.set_pdf_minor_version(4);
        self.set_pdf_gamma(1000);
        self.set_pdf_image_gamma(2200);
        self.set_pdf_image_hicolor(1);
        self.set_pdf_image_apply_gamma(0);
        self.set_pdf_px_dimen(self.one_bp);
        self.set_pdf_draftmode(0);
        // §1064
        self.set_pdf_ignored_dimen(IGNORE_DEPTH);
        self.set_pdf_each_line_height(self.pdf_ignored_dimen());
        self.set_pdf_each_line_depth(self.pdf_ignored_dimen());
        self.set_pdf_first_line_height(self.pdf_ignored_dimen());
        self.set_pdf_last_line_depth(self.pdf_ignored_dimen());
        // §1123
        for k in -TRIE_OP_SIZE..=TRIE_OP_SIZE {
            self.trie_op_hash[k] = 0;
        }
        for k in 0..=255 {
            self.trie_used[k] = MIN_TRIE_OP;
        }
        self.max_op_used = MIN_TRIE_OP;
        self.trie_op_ptr = 0;
        // §1128
        self.trie_not_ready = true;
        // §1394
        self.set_text(FROZEN_PROTECTION, S_INACCESSIBLE);
        // §1479
        if self.ini_version {
            self.format_ident = S_INITEX_IDENT;
        }
        // §1616
        self.set_text(END_WRITE, S_ENDWRITE);
        self.set_eq_level(END_WRITE, LEVEL_ONE);
        self.set_eq_type(END_WRITE, OUTER_CALL);
        self.set_equiv(END_WRITE, NULL);
        // §1653
        self.eTeX_mode = 0; // initially we are in compatibility mode
        // §1812 Initialize variables for eTeX compatibility mode
        self.max_reg_num = 255;
        self.max_reg_help_line = 0; // "A register number must be between 0 and 255."
        // §1818
        for i in INT_VAL..=TOK_VAL {
            self.sa_root[i] = NULL;
        }
    }

    /// §37
    pub fn init_terminal(&mut self) -> bool {
        self.t_open_in();
        if self.last > self.first {
            self.set_loc(self.first);
            while self.loc() < self.last && self.buffer.at(self.loc()) == b' ' as i32 {
                self.set_loc(self.loc() + 1);
            }
            if self.loc() < self.last {
                return true;
            }
        }
        loop {
            self.wake_up_terminal();
            self.wterm_s(b"**");
            self.update_terminal();
            if !self.input_ln(TERM_IN, true) {
                // this shouldn't happen
                self.wterm_cr();
                self.wterm_ln_s(b"! End of file on the terminal... why?");
                return false;
            }
            self.set_loc(self.first);
            while self.loc() < self.last && self.buffer.at(self.loc()) == b' ' as i32 {
                self.set_loc(self.loc() + 1);
            }
            if self.loc() < self.last {
                return true; // return unless the line was all blank
            }
            self.wterm_ln_s(b"Please type the name of your input file.");
        }
    }

    /// §57
    pub fn print_ln(&mut self) {
        match self.selector {
            TERM_AND_LOG => {
                self.wterm_cr();
                self.wlog_cr();
                self.term_offset = 0;
                self.file_offset = 0;
            }
            LOG_ONLY => {
                self.wlog_cr();
                self.file_offset = 0;
            }
            TERM_ONLY => {
                self.wterm_cr();
                self.term_offset = 0;
            }
            NO_PRINT | PSEUDO | NEW_STRING => {}
            _ => self.write_file_ln(self.selector),
        }
    }

    /// §58.  A string in progress is what most characters go to, from `\\string`, `\\number` and their like.
    #[inline]
    pub fn print_char(&mut self, s: i32) {
        if self.selector == NEW_STRING {
            if self.pool_ptr < self.pool_size {
                self.append_char(s);
            } // we drop characters if the string space is full
            self.tally += 1;
        } else {
            self.print_char_to_file(s);
        }
    }

    /// §58 for any other selector.
    #[inline(never)]
    fn print_char_to_file(&mut self, s: i32) {
        // §262 Character s is the current new-line character
        if s == self.new_line_char() && self.selector < PSEUDO {
            self.print_ln();
            return;
        }
        match self.selector {
            TERM_AND_LOG => {
                let c = self.xchr.at(s);
                self.wterm_char(c);
                self.wlog_char(c);
                self.term_offset += 1;
                self.file_offset += 1;
                if self.term_offset == self.max_print_line {
                    self.wterm_cr();
                    self.term_offset = 0;
                }
                if self.file_offset == self.max_print_line {
                    self.wlog_cr();
                    self.file_offset = 0;
                }
            }
            LOG_ONLY => {
                let c = self.xchr.at(s);
                self.wlog_char(c);
                self.file_offset += 1;
                if self.file_offset == self.max_print_line {
                    self.print_ln();
                }
            }
            TERM_ONLY => {
                let c = self.xchr.at(s);
                self.wterm_char(c);
                self.term_offset += 1;
                if self.term_offset == self.max_print_line {
                    self.print_ln();
                }
            }
            NO_PRINT => {}
            PSEUDO => {
                if self.tally < self.trick_count {
                    let k = self.tally % self.error_line;
                    self.trick_buf[k] = s;
                }
            }
            _ => {
                let c = self.xchr.at(s);
                self.write_file_char(self.selector, c);
            }
        }
        self.tally += 1;
    }

    /// §59's and §60's loop over a string, if what they print goes to a string in progress: what each
    /// `print_char` would do, in one go.
    #[inline]
    fn print_string_to_string(&mut self, s: i32) {
        let (a, b) = (self.str_start[s], self.str_start[s + 1]);
        self.tally += b - a;
        let n = (b - a).min(self.pool_size - self.pool_ptr).max(0); // we drop characters if the string space is full
        self.str_pool.copy_within(a, self.pool_ptr, n);
        self.pool_ptr += n;
    }

    /// §59
    pub fn print(&mut self, s: i32) {
        let mut s = s;
        if s >= self.str_ptr {
            s = S_QUESTIONS; // this can't happen
        } else if s < 256 {
            if s < 0 {
                s = S_QUESTIONS; // can't happen
            } else {
                if self.selector > PSEUDO && !self.message_printing {
                    self.print_char(s); // internal strings are not expanded
                    return;
                }
                // §262 Character s is the current new-line character
                if s == self.new_line_char() {
                    if self.selector < PSEUDO {
                        self.print_ln();
                        return;
                    } else if self.message_printing {
                        self.print_char(s);
                        return;
                    }
                }
                if self.xprn.at(s) != 0 {
                    self.print_char(s);
                    return;
                }
                let nl = self.new_line_char();
                self.set_new_line_char(-1); // temporarily disable new-line character
                let mut j = self.str_start[s];
                while j < self.str_start[s + 1] {
                    self.print_char(self.str_pool.at(j));
                    j += 1;
                }
                self.set_new_line_char(nl);
                return;
            }
        }
        if self.selector == NEW_STRING {
            self.print_string_to_string(s);
            return;
        }
        let mut j = self.str_start[s];
        while j < self.str_start[s + 1] {
            self.print_char(self.str_pool.at(j));
            j += 1;
        }
    }

    /// §60
    pub fn slow_print(&mut self, s: i32) {
        if s >= self.str_ptr || s < 256 {
            self.print(s);
        } else if self.selector == NEW_STRING && !self.message_printing {
            self.print_string_to_string(s); // internal strings are not expanded
        } else {
            let mut j = self.str_start[s];
            while j < self.str_start[s + 1] {
                self.print(self.str_pool.at(j));
                j += 1;
            }
        }
    }

    /// §62
    pub fn print_nl(&mut self, s: i32) {
        if self.selector < NO_PRINT
            || (self.term_offset > 0 && odd(self.selector))
            || (self.file_offset > 0 && self.selector >= LOG_ONLY)
        {
            self.print_ln();
        }
        self.print(s);
    }

    /// §63
    pub fn print_esc(&mut self, s: i32) {
        // §261 Set variable c to the current escape character
        let c = self.escape_char();
        if c >= 0 && c < 256 {
            self.print(c);
        }
        self.slow_print(s);
    }

    /// §64
    pub fn print_the_digs(&mut self, k: i32) {
        let mut k = k;
        while k > 0 {
            k -= 1;
            if self.dig[k] < 10 {
                self.print_char(b'0' as i32 + self.dig[k]);
            } else {
                self.print_char(b'A' as i32 - 10 + self.dig[k]);
            }
        }
    }

    /// §65
    pub fn print_int(&mut self, n: impl Into<i64>) {
        let mut n: i64 = n.into();
        let mut k = 0;
        if n < 0 {
            self.print_char(b'-' as i32);
            if n > -100000000 {
                n = -n;
            } else {
                let mut m = -1 - n;
                n = m / 10;
                m = (m % 10) + 1;
                k = 1;
                if m < 10 {
                    self.dig[0] = m as i32;
                } else {
                    self.dig[0] = 0;
                    n += 1;
                }
            }
        }
        loop {
            self.dig[k] = (n % 10) as i32;
            n = n / 10;
            k += 1;
            if n == 0 {
                break;
            }
        }
        self.print_the_digs(k);
    }

    /// §66
    pub fn print_two(&mut self, n: i32) {
        let n = abs(n) % 100;
        self.print_char(b'0' as i32 + (n / 10));
        self.print_char(b'0' as i32 + (n % 10));
    }

    /// §67
    pub fn print_hex(&mut self, n: i32) {
        let mut n = n;
        let mut k = 0;
        self.print_char(b'"' as i32);
        loop {
            self.dig[k] = n % 16;
            n = n / 16;
            k += 1;
            if n == 0 {
                break;
            }
        }
        self.print_the_digs(k);
    }

    /// §69
    pub fn print_roman_int(&mut self, n: i32) {
        // The pool string "m2d5c2l5x2v5i"; `j` and `k` index it.
        const ROMAN: &[u8] = b"m2d5c2l5x2v5i";
        let digit = |i: usize| ROMAN[i] as i32;
        let mut n = n;
        let mut j = 0;
        let mut v = 1000;
        loop {
            while n >= v {
                self.print_char(digit(j));
                n = n - v;
            }
            if n <= 0 {
                return; // nonpositive input produces no output
            }
            let mut k = j + 2;
            let mut u = v / (digit(k - 1) - b'0' as i32);
            if ROMAN[k - 1] == b'2' {
                k = k + 2;
                u = u / (digit(k - 1) - b'0' as i32);
            }
            if n + u >= v {
                self.print_char(digit(k));
                n = n + u;
            } else {
                j = j + 2;
                v = v / (digit(j - 1) - b'0' as i32);
            }
        }
    }

    /// §70
    pub fn print_current_string(&mut self) {
        let mut j = self.str_start[self.str_ptr];
        while j < self.pool_ptr {
            self.print_char(self.str_pool.at(j));
            j += 1;
        }
    }

    /// §71
    pub fn term_input(&mut self) {
        self.update_terminal(); // now the user sees the prompt for sure
        if !self.input_ln(TERM_IN, true) {
            self.set_limit(0);
            self.fatal_error(b"End of file on the terminal!");
        }
        self.term_offset = 0; // the user's line ended with <return>
        self.selector -= 1; // prepare to echo the input
        if self.last != self.first {
            for k in self.first..=self.last - 1 {
                self.print(self.buffer.at(k));
            }
        }
        self.print_ln();
        self.selector += 1; // restore previous status
    }
}
