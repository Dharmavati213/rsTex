//! §225-228 Part 15: the command codes; §238-273 Part 17: the table of equivalents.
//!
//! The regions' layout is in consts.rs.  §240's region-2 loop lands in `initialize` and stops at
//! `EQTB_SIZE`; run-time control sequences above it are initialised by `id_lookup`, in hash.rs.
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
    /// §243
    pub fn print_skip_param(&mut self, n: i32) {
        match n {
            LINE_SKIP_CODE => self.print_esc_s(b"lineskip"),
            BASELINE_SKIP_CODE => self.print_esc_s(b"baselineskip"),
            PAR_SKIP_CODE => self.print_esc_s(b"parskip"),
            ABOVE_DISPLAY_SKIP_CODE => self.print_esc_s(b"abovedisplayskip"),
            BELOW_DISPLAY_SKIP_CODE => self.print_esc_s(b"belowdisplayskip"),
            ABOVE_DISPLAY_SHORT_SKIP_CODE => self.print_esc_s(b"abovedisplayshortskip"),
            BELOW_DISPLAY_SHORT_SKIP_CODE => self.print_esc_s(b"belowdisplayshortskip"),
            LEFT_SKIP_CODE => self.print_esc_s(b"leftskip"),
            RIGHT_SKIP_CODE => self.print_esc_s(b"rightskip"),
            TOP_SKIP_CODE => self.print_esc_s(b"topskip"),
            SPLIT_TOP_SKIP_CODE => self.print_esc_s(b"splittopskip"),
            TAB_SKIP_CODE => self.print_esc_s(b"tabskip"),
            SPACE_SKIP_CODE => self.print_esc_s(b"spaceskip"),
            XSPACE_SKIP_CODE => self.print_esc_s(b"xspaceskip"),
            PAR_FILL_SKIP_CODE => self.print_esc_s(b"parfillskip"),
            THIN_MU_SKIP_CODE => self.print_esc_s(b"thinmuskip"),
            MED_MU_SKIP_CODE => self.print_esc_s(b"medmuskip"),
            THICK_MU_SKIP_CODE => self.print_esc_s(b"thickmuskip"),
            _ => self.print_s(b"[unknown glue parameter!]"),
        }
    }

    /// §255
    pub fn print_param(&mut self, n: i32) {
        // §1700's case is an expression.
        const TEXXET_STATE_CODE: i32 = ETEX_STATE_CODE + TEXXET_CODE;
        match n {
            PRETOLERANCE_CODE => self.print_esc_s(b"pretolerance"),
            TOLERANCE_CODE => self.print_esc_s(b"tolerance"),
            LINE_PENALTY_CODE => self.print_esc_s(b"linepenalty"),
            HYPHEN_PENALTY_CODE => self.print_esc_s(b"hyphenpenalty"),
            EX_HYPHEN_PENALTY_CODE => self.print_esc_s(b"exhyphenpenalty"),
            CLUB_PENALTY_CODE => self.print_esc_s(b"clubpenalty"),
            WIDOW_PENALTY_CODE => self.print_esc_s(b"widowpenalty"),
            DISPLAY_WIDOW_PENALTY_CODE => self.print_esc_s(b"displaywidowpenalty"),
            BROKEN_PENALTY_CODE => self.print_esc_s(b"brokenpenalty"),
            BIN_OP_PENALTY_CODE => self.print_esc_s(b"binoppenalty"),
            REL_PENALTY_CODE => self.print_esc_s(b"relpenalty"),
            PRE_DISPLAY_PENALTY_CODE => self.print_esc_s(b"predisplaypenalty"),
            POST_DISPLAY_PENALTY_CODE => self.print_esc_s(b"postdisplaypenalty"),
            INTER_LINE_PENALTY_CODE => self.print_esc_s(b"interlinepenalty"),
            DOUBLE_HYPHEN_DEMERITS_CODE => self.print_esc_s(b"doublehyphendemerits"),
            FINAL_HYPHEN_DEMERITS_CODE => self.print_esc_s(b"finalhyphendemerits"),
            ADJ_DEMERITS_CODE => self.print_esc_s(b"adjdemerits"),
            MAG_CODE => self.print_esc_s(b"mag"),
            DELIMITER_FACTOR_CODE => self.print_esc_s(b"delimiterfactor"),
            LOOSENESS_CODE => self.print_esc_s(b"looseness"),
            TIME_CODE => self.print_esc_s(b"time"),
            DAY_CODE => self.print_esc_s(b"day"),
            MONTH_CODE => self.print_esc_s(b"month"),
            YEAR_CODE => self.print_esc_s(b"year"),
            SHOW_BOX_BREADTH_CODE => self.print_esc_s(b"showboxbreadth"),
            SHOW_BOX_DEPTH_CODE => self.print_esc_s(b"showboxdepth"),
            HBADNESS_CODE => self.print_esc_s(b"hbadness"),
            VBADNESS_CODE => self.print_esc_s(b"vbadness"),
            PAUSING_CODE => self.print_esc_s(b"pausing"),
            TRACING_ONLINE_CODE => self.print_esc_s(b"tracingonline"),
            TRACING_MACROS_CODE => self.print_esc_s(b"tracingmacros"),
            TRACING_STATS_CODE => self.print_esc_s(b"tracingstats"),
            TRACING_PARAGRAPHS_CODE => self.print_esc_s(b"tracingparagraphs"),
            TRACING_PAGES_CODE => self.print_esc_s(b"tracingpages"),
            TRACING_OUTPUT_CODE => self.print_esc_s(b"tracingoutput"),
            TRACING_LOST_CHARS_CODE => self.print_esc_s(b"tracinglostchars"),
            TRACING_COMMANDS_CODE => self.print_esc_s(b"tracingcommands"),
            TRACING_RESTORES_CODE => self.print_esc_s(b"tracingrestores"),
            UC_HYPH_CODE => self.print_esc_s(b"uchyph"),
            OUTPUT_PENALTY_CODE => self.print_esc_s(b"outputpenalty"),
            MAX_DEAD_CYCLES_CODE => self.print_esc_s(b"maxdeadcycles"),
            HANG_AFTER_CODE => self.print_esc_s(b"hangafter"),
            FLOATING_PENALTY_CODE => self.print_esc_s(b"floatingpenalty"),
            GLOBAL_DEFS_CODE => self.print_esc_s(b"globaldefs"),
            CUR_FAM_CODE => self.print_esc_s(b"fam"),
            ESCAPE_CHAR_CODE => self.print_esc_s(b"escapechar"),
            DEFAULT_HYPHEN_CHAR_CODE => self.print_esc_s(b"defaulthyphenchar"),
            DEFAULT_SKEW_CHAR_CODE => self.print_esc_s(b"defaultskewchar"),
            END_LINE_CHAR_CODE => self.print_esc_s(b"endlinechar"),
            NEW_LINE_CHAR_CODE => self.print_esc_s(b"newlinechar"),
            LANGUAGE_CODE => self.print_esc_s(b"language"),
            LEFT_HYPHEN_MIN_CODE => self.print_esc_s(b"lefthyphenmin"),
            RIGHT_HYPHEN_MIN_CODE => self.print_esc_s(b"righthyphenmin"),
            HOLDING_INSERTS_CODE => self.print_esc_s(b"holdinginserts"),
            ERROR_CONTEXT_LINES_CODE => self.print_esc_s(b"errorcontextlines"),
            CHAR_SUB_DEF_MIN_CODE => self.print_esc_s(b"charsubdefmin"),
            CHAR_SUB_DEF_MAX_CODE => self.print_esc_s(b"charsubdefmax"),
            TRACING_CHAR_SUB_DEF_CODE => self.print_esc_s(b"tracingcharsubdef"),
            TRACING_STACK_LEVELS_CODE => self.print_esc_s(b"tracingstacklevels"),
            PARTOKEN_CONTEXT_CODE => self.print_esc_s(b"partokencontext"),
            SHOW_STREAM_CODE => self.print_esc_s(b"showstream"),
            PDF_OUTPUT_CODE => self.print_esc_s(b"pdfoutput"),
            PDF_COMPRESS_LEVEL_CODE => self.print_esc_s(b"pdfcompresslevel"),
            PDF_OBJCOMPRESSLEVEL_CODE => self.print_esc_s(b"pdfobjcompresslevel"),
            PDF_DECIMAL_DIGITS_CODE => self.print_esc_s(b"pdfdecimaldigits"),
            PDF_MOVE_CHARS_CODE => self.print_esc_s(b"pdfmovechars"),
            PDF_IMAGE_RESOLUTION_CODE => self.print_esc_s(b"pdfimageresolution"),
            PDF_PK_RESOLUTION_CODE => self.print_esc_s(b"pdfpkresolution"),
            PDF_UNIQUE_RESNAME_CODE => self.print_esc_s(b"pdfuniqueresname"),
            PDF_OPTION_ALWAYS_USE_PDFPAGEBOX_CODE => self.print_esc_s(b"pdfoptionalwaysusepdfpagebox"),
            PDF_OPTION_PDF_INCLUSION_ERRORLEVEL_CODE => self.print_esc_s(b"pdfoptionpdfinclusionerrorlevel"),
            PDF_MAJOR_VERSION_CODE => self.print_esc_s(b"pdfmajorversion"),
            PDF_MINOR_VERSION_CODE => self.print_esc_s(b"pdfminorversion"),
            PDF_FORCE_PAGEBOX_CODE => self.print_esc_s(b"pdfforcepagebox"),
            PDF_PAGEBOX_CODE => self.print_esc_s(b"pdfpagebox"),
            PDF_INCLUSION_ERRORLEVEL_CODE => self.print_esc_s(b"pdfinclusionerrorlevel"),
            PDF_GAMMA_CODE => self.print_esc_s(b"pdfgamma"),
            PDF_IMAGE_GAMMA_CODE => self.print_esc_s(b"pdfimagegamma"),
            PDF_IMAGE_HICOLOR_CODE => self.print_esc_s(b"pdfimagehicolor"),
            PDF_IMAGE_APPLY_GAMMA_CODE => self.print_esc_s(b"pdfimageapplygamma"),
            PDF_ADJUST_SPACING_CODE => self.print_esc_s(b"pdfadjustspacing"),
            PDF_PROTRUDE_CHARS_CODE => self.print_esc_s(b"pdfprotrudechars"),
            PDF_TRACING_FONTS_CODE => self.print_esc_s(b"pdftracingfonts"),
            PDF_ADJUST_INTERWORD_GLUE_CODE => self.print_esc_s(b"pdfadjustinterwordglue"),
            PDF_PREPEND_KERN_CODE => self.print_esc_s(b"pdfprependkern"),
            PDF_APPEND_KERN_CODE => self.print_esc_s(b"pdfappendkern"),
            PDF_GEN_TOUNICODE_CODE => self.print_esc_s(b"pdfgentounicode"),
            PDF_DRAFTMODE_CODE => self.print_esc_s(b"pdfdraftmode"),
            PDF_INCLUSION_COPY_FONT_CODE => self.print_esc_s(b"pdfinclusioncopyfonts"),
            PDF_SUPPRESS_WARNING_DUP_DEST_CODE => self.print_esc_s(b"pdfsuppresswarningdupdest"),
            PDF_SUPPRESS_WARNING_DUP_MAP_CODE => self.print_esc_s(b"pdfsuppresswarningdupmap"),
            PDF_SUPPRESS_WARNING_PAGE_GROUP_CODE => self.print_esc_s(b"pdfsuppresswarningpagegroup"),
            PDF_INFO_OMIT_DATE_CODE => self.print_esc_s(b"pdfinfoomitdate"),
            PDF_SUPPRESS_PTEX_INFO_CODE => self.print_esc_s(b"pdfsuppressptexinfo"),
            PDF_OMIT_CHARSET_CODE => self.print_esc_s(b"pdfomitcharset"),
            PDF_OMIT_INFO_DICT_CODE => self.print_esc_s(b"pdfomitinfodict"),
            PDF_OMIT_PROCSET_CODE => self.print_esc_s(b"pdfomitprocset"),
            PDF_PTEX_USE_UNDERSCORE_CODE => self.print_esc_s(b"pdfptexuseunderscore"),
            // §1900
            SYNCTEX_CODE => self.print_esc_s(b"synctex"),
            // §1659
            TRACING_ASSIGNS_CODE => self.print_esc_s(b"tracingassigns"),
            TRACING_GROUPS_CODE => self.print_esc_s(b"tracinggroups"),
            TRACING_IFS_CODE => self.print_esc_s(b"tracingifs"),
            TRACING_SCAN_TOKENS_CODE => self.print_esc_s(b"tracingscantokens"),
            TRACING_NESTING_CODE => self.print_esc_s(b"tracingnesting"),
            PRE_DISPLAY_DIRECTION_CODE => self.print_esc_s(b"predisplaydirection"),
            LAST_LINE_FIT_CODE => self.print_esc_s(b"lastlinefit"),
            SAVING_VDISCARDS_CODE => self.print_esc_s(b"savingvdiscards"),
            SAVING_HYPH_CODES_CODE => self.print_esc_s(b"savinghyphcodes"),
            IGNORE_PRIMITIVE_ERROR_CODE => self.print_esc_s(b"ignoreprimitiveerror"),
            // §1700
            TEXXET_STATE_CODE => self.print_esc_s(b"TeXXeTstate"),
            _ => self.print_s(b"[unknown integer parameter!]"),
        }
    }

    /// §259
    pub fn fix_date_and_time(&mut self) {
        let (t, d, m, y) = self.date_and_time();
        self.sys_time = t;
        self.sys_day = d;
        self.sys_month = m;
        self.sys_year = y;
        self.set_time(self.sys_time);
        self.set_day(self.sys_day);
        self.set_month(self.sys_month);
        self.set_year(self.sys_year);
    }

    /// §263
    pub fn begin_diagnostic(&mut self) {
        self.old_setting = self.selector;
        if self.tracing_online() <= 0 && self.selector == TERM_AND_LOG {
            self.selector -= 1;
            if self.history == SPOTLESS {
                self.history = WARNING_ISSUED;
            }
        }
    }

    /// §263
    pub fn end_diagnostic(&mut self, blank_line: bool) {
        self.print_nl_s(b"");
        if blank_line {
            self.print_ln();
        }
        self.selector = self.old_setting;
    }

    /// §265
    pub fn print_length_param(&mut self, n: i32) {
        match n {
            PAR_INDENT_CODE => self.print_esc_s(b"parindent"),
            MATH_SURROUND_CODE => self.print_esc_s(b"mathsurround"),
            LINE_SKIP_LIMIT_CODE => self.print_esc_s(b"lineskiplimit"),
            HSIZE_CODE => self.print_esc_s(b"hsize"),
            VSIZE_CODE => self.print_esc_s(b"vsize"),
            MAX_DEPTH_CODE => self.print_esc_s(b"maxdepth"),
            SPLIT_MAX_DEPTH_CODE => self.print_esc_s(b"splitmaxdepth"),
            BOX_MAX_DEPTH_CODE => self.print_esc_s(b"boxmaxdepth"),
            HFUZZ_CODE => self.print_esc_s(b"hfuzz"),
            VFUZZ_CODE => self.print_esc_s(b"vfuzz"),
            DELIMITER_SHORTFALL_CODE => self.print_esc_s(b"delimitershortfall"),
            NULL_DELIMITER_SPACE_CODE => self.print_esc_s(b"nulldelimiterspace"),
            SCRIPT_SPACE_CODE => self.print_esc_s(b"scriptspace"),
            PRE_DISPLAY_SIZE_CODE => self.print_esc_s(b"predisplaysize"),
            DISPLAY_WIDTH_CODE => self.print_esc_s(b"displaywidth"),
            DISPLAY_INDENT_CODE => self.print_esc_s(b"displayindent"),
            OVERFULL_RULE_CODE => self.print_esc_s(b"overfullrule"),
            HANG_INDENT_CODE => self.print_esc_s(b"hangindent"),
            H_OFFSET_CODE => self.print_esc_s(b"hoffset"),
            V_OFFSET_CODE => self.print_esc_s(b"voffset"),
            EMERGENCY_STRETCH_CODE => self.print_esc_s(b"emergencystretch"),
            PDF_H_ORIGIN_CODE => self.print_esc_s(b"pdfhorigin"),
            PDF_V_ORIGIN_CODE => self.print_esc_s(b"pdfvorigin"),
            PDF_PAGE_WIDTH_CODE => self.print_esc_s(b"pdfpagewidth"),
            PDF_PAGE_HEIGHT_CODE => self.print_esc_s(b"pdfpageheight"),
            PDF_LINK_MARGIN_CODE => self.print_esc_s(b"pdflinkmargin"),
            PDF_DEST_MARGIN_CODE => self.print_esc_s(b"pdfdestmargin"),
            PDF_THREAD_MARGIN_CODE => self.print_esc_s(b"pdfthreadmargin"),
            PDF_FIRST_LINE_HEIGHT_CODE => self.print_esc_s(b"pdffirstlineheight"),
            PDF_LAST_LINE_DEPTH_CODE => self.print_esc_s(b"pdflastlinedepth"),
            PDF_EACH_LINE_HEIGHT_CODE => self.print_esc_s(b"pdfeachlineheight"),
            PDF_EACH_LINE_DEPTH_CODE => self.print_esc_s(b"pdfeachlinedepth"),
            PDF_IGNORED_DIMEN_CODE => self.print_esc_s(b"pdfignoreddimen"),
            PDF_PX_DIMEN_CODE => self.print_esc_s(b"pdfpxdimen"),
            _ => self.print_s(b"[unknown dimen parameter!]"),
        }
    }

    /// §270
    pub fn show_eqtb(&mut self, n: i32) {
        if n < ACTIVE_BASE {
            self.print_char(b'?' as i32); // this can't happen
        } else if n < GLUE_BASE || (n > EQTB_SIZE && n <= self.eqtb_top) {
            // §241 regions 1 and 2
            self.sprint_cs(n);
            self.print_char(b'=' as i32);
            let t = self.eq_type(n) & !crate::native::ARMED_BITS;
            self.print_cmd_chr(t, self.equiv(n));
            if t >= CALL {
                self.print_char(b':' as i32);
                self.show_token_list(self.equiv(n) + 1, NULL, 32);
            }
        } else if n < LOCAL_BASE {
            // §247 region 3
            if n < SKIP_BASE {
                self.print_skip_param(n - GLUE_BASE);
                self.print_char(b'=' as i32);
                if n < GLUE_BASE + THIN_MU_SKIP_CODE {
                    self.print_spec(self.equiv(n), b"pt");
                } else {
                    self.print_spec(self.equiv(n), b"mu");
                }
            } else if n < MU_SKIP_BASE {
                self.print_esc_s(b"skip");
                self.print_int(n - SKIP_BASE);
                self.print_char(b'=' as i32);
                self.print_spec(self.equiv(n), b"pt");
            } else {
                self.print_esc_s(b"muskip");
                self.print_int(n - MU_SKIP_BASE);
                self.print_char(b'=' as i32);
                self.print_spec(self.equiv(n), b"mu");
            }
        } else if n < INT_BASE {
            // §251 region 4
            if n == PAR_SHAPE_LOC || (n >= ETEX_PEN_BASE && n < ETEX_PENS) {
                self.print_cmd_chr(SET_SHAPE, n);
                self.print_char(b'=' as i32);
                if self.equiv(n) == NULL {
                    self.print_char(b'0' as i32);
                } else if n > PAR_SHAPE_LOC {
                    self.print_int(self.penalty(self.equiv(n)));
                    self.print_char(b' ' as i32);
                    self.print_int(self.penalty(self.equiv(n) + 1));
                    if self.penalty(self.equiv(n)) > 1 {
                        self.print_esc_s(b"ETC.");
                    }
                } else {
                    self.print_int(self.info(self.par_shape_ptr()));
                }
            } else if n < TOKS_BASE {
                self.print_cmd_chr(ASSIGN_TOKS, n);
                self.print_char(b'=' as i32);
                if self.equiv(n) != NULL {
                    self.show_token_list(self.equiv(n) + 1, NULL, 32);
                }
            } else if n < BOX_BASE {
                self.print_esc_s(b"toks");
                self.print_int(n - TOKS_BASE);
                self.print_char(b'=' as i32);
                if self.equiv(n) != NULL {
                    self.show_token_list(self.equiv(n) + 1, NULL, 32);
                }
            } else if n < CUR_FONT_LOC {
                self.print_esc_s(b"box");
                self.print_int(n - BOX_BASE);
                self.print_char(b'=' as i32);
                if self.equiv(n) == NULL {
                    self.print_s(b"void");
                } else {
                    self.depth_threshold = 0;
                    self.breadth_max = 1;
                    self.show_node_list(self.equiv(n));
                }
            } else if n < CAT_CODE_BASE {
                // §252 the font identifier
                if n == CUR_FONT_LOC {
                    self.print_s(b"current font");
                } else if n < MATH_FONT_BASE + 16 {
                    self.print_esc_s(b"textfont");
                    self.print_int(n - MATH_FONT_BASE);
                } else if n < MATH_FONT_BASE + 32 {
                    self.print_esc_s(b"scriptfont");
                    self.print_int(n - MATH_FONT_BASE - 16);
                } else {
                    self.print_esc_s(b"scriptscriptfont");
                    self.print_int(n - MATH_FONT_BASE - 32);
                }
                self.print_char(b'=' as i32);
                self.print_esc(self.font_id_text(self.equiv(n)));
            } else if n < MATH_CODE_BASE {
                // §253 the halfword code
                if n < LC_CODE_BASE {
                    self.print_esc_s(b"catcode");
                    self.print_int(n - CAT_CODE_BASE);
                } else if n < UC_CODE_BASE {
                    self.print_esc_s(b"lccode");
                    self.print_int(n - LC_CODE_BASE);
                } else if n < SF_CODE_BASE {
                    self.print_esc_s(b"uccode");
                    self.print_int(n - UC_CODE_BASE);
                } else {
                    self.print_esc_s(b"sfcode");
                    self.print_int(n - SF_CODE_BASE);
                }
                self.print_char(b'=' as i32);
                self.print_int(self.equiv(n));
            } else {
                self.print_esc_s(b"mathcode");
                self.print_int(n - MATH_CODE_BASE);
                self.print_char(b'=' as i32);
                self.print_int(self.equiv(n));
            }
        } else if n < DIMEN_BASE {
            // §260 region 5
            if n < COUNT_BASE {
                self.print_param(n - INT_BASE);
            } else if n < DEL_CODE_BASE {
                self.print_esc_s(b"count");
                self.print_int(n - COUNT_BASE);
            } else {
                self.print_esc_s(b"delcode");
                self.print_int(n - DEL_CODE_BASE);
            }
            self.print_char(b'=' as i32);
            self.print_int(self.eqtb[n].int());
        } else if n <= EQTB_SIZE {
            // §269 region 6
            if n < SCALED_BASE {
                self.print_length_param(n - DIMEN_BASE);
            } else {
                self.print_esc_s(b"dimen");
                self.print_int(n - SCALED_BASE);
            }
            self.print_char(b'=' as i32);
            self.print_scaled(self.eqtb[n].sc());
            self.print_s(b"pt");
        } else {
            self.print_char(b'?' as i32); // this can't happen either
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::testkit::*;
    use crate::toks::testkit::*;

    #[test]
    fn date_and_time_reach_the_parameters() {
        let t = machine();
        t.fix_date_and_time();
        assert_eq!((t.time(), t.day(), t.month(), t.year()), (t.sys_time, t.sys_day, t.sys_month, t.sys_year));
        assert!((0..24 * 60).contains(&t.time()), "minutes since midnight");
        assert!((1..=31).contains(&t.day()));
        assert!((1..=12).contains(&t.month()));
        assert!(t.year() >= 1970);
        Tex::dealloc(t);
    }

    #[test]
    fn begin_diagnostic_leaves_the_terminal_unless_tracing_online() {
        let t = machine();
        t.selector = TERM_AND_LOG;
        t.history = SPOTLESS;
        t.begin_diagnostic();
        assert_eq!((t.selector, t.old_setting, t.history), (LOG_ONLY, TERM_AND_LOG, WARNING_ISSUED));
        t.selector = TERM_AND_LOG;
        t.history = ERROR_MESSAGE_ISSUED;
        t.begin_diagnostic();
        assert_eq!((t.selector, t.history), (LOG_ONLY, ERROR_MESSAGE_ISSUED), "a worse history stays");
        t.selector = TERM_AND_LOG;
        t.history = SPOTLESS;
        t.set_tracing_online(1);
        t.begin_diagnostic();
        assert_eq!((t.selector, t.old_setting, t.history), (TERM_AND_LOG, TERM_AND_LOG, SPOTLESS));
        t.set_tracing_online(0);
        t.selector = LOG_ONLY;
        t.begin_diagnostic();
        assert_eq!((t.selector, t.old_setting, t.history), (LOG_ONLY, LOG_ONLY, SPOTLESS), "only term_and_log changes");
        Tex::dealloc(t);
    }

    #[test]
    fn end_diagnostic_ends_the_line_and_restores_the_selector() {
        let t = machine();
        print_setup(t);
        t.begin_diagnostic();
        t.selector = NO_PRINT;
        t.end_diagnostic(false);
        assert_eq!(t.selector, TERM_ONLY);
        t.print_char(b'x' as i32);
        t.begin_diagnostic();
        t.end_diagnostic(false);
        assert_eq!(printed(t), "x\n");
        t.print_char(b'y' as i32);
        t.begin_diagnostic();
        t.end_diagnostic(true);
        assert_eq!(printed(t), "y\n\n");
        Tex::dealloc(t);
    }

    /// Every parameter, printed as the oracle's `\show` prints it.
    #[test]
    fn parameter_names_against_the_oracle() {
        let rows = table(include_str!("../testdata/eqtb_names_oracle.tsv"));
        assert_eq!(rows.len(), 163);
        let t = machine();
        print_setup(t);
        for r in rows {
            let n: i32 = r[2].parse().unwrap();
            match r[1].as_str() {
                "assign_int" => t.print_param(n),
                "assign_dimen" => t.print_length_param(n),
                _ => t.print_skip_param(n),
            }
            assert_eq!(printed(t), r[3], "{}", r[0]);
        }
        Tex::dealloc(t);
    }

    #[test]
    fn unknown_parameters() {
        let t = machine();
        print_setup(t);
        t.print_skip_param(GLUE_PARS);
        assert_eq!(printed(t), "[unknown glue parameter!]");
        t.print_param(INT_PARS);
        assert_eq!(printed(t), "[unknown integer parameter!]");
        t.print_length_param(DIMEN_PARS);
        assert_eq!(printed(t), "[unknown dimen parameter!]");
        Tex::dealloc(t);
    }

    /// The regions' edges.  The oracle's tracing goes through the middles: `save.rs` has those tests.
    #[test]
    fn show_eqtb_at_the_edges_of_the_regions() {
        let t = table_machine();
        print_setup(t);
        t.no_new_control_sequence = false;
        let p = cs_named(t, "brandnew");
        for (n, want) in [
            (ACTIVE_BASE - 1, "?"),
            (ACTIVE_BASE + b'a' as i32, "a=undefined"),
            (SINGLE_BASE + b'a' as i32, "\\a=undefined"),
            (NULL_CS, "\\csname\\endcsname=undefined"),
            (p, "\\brandnew=undefined"),
            (GLUE_BASE, "\\lineskip=0.0pt"),
            (GLUE_BASE + THIN_MU_SKIP_CODE - 1, "\\parfillskip=0.0pt"),
            (GLUE_BASE + THIN_MU_SKIP_CODE, "\\thinmuskip=0.0mu"),
            (SKIP_BASE, "\\skip0=0.0pt"),
            (MU_SKIP_BASE, "\\muskip0=0.0mu"),
            (LOCAL_BASE - 1, "\\muskip255=0.0mu"),
            (PAR_SHAPE_LOC, "\\parshape=0"),
            (ETEX_PEN_BASE, "\\interlinepenalties=0"),
            (ETEX_PENS - 1, "\\displaywidowpenalties=0"),
            (OUTPUT_ROUTINE_LOC, "\\output="),
            (TOKS_BASE, "\\toks0="),
            (BOX_BASE, "\\box0=void"),
            (CUR_FONT_LOC, "current font=\\nullfont"),
            (MATH_FONT_BASE, "\\textfont0=\\nullfont"),
            (MATH_FONT_BASE + 16, "\\scriptfont0=\\nullfont"),
            (MATH_FONT_BASE + 32, "\\scriptscriptfont0=\\nullfont"),
            (CAT_CODE_BASE + b'a' as i32, "\\catcode97=11"),
            (LC_CODE_BASE + b'A' as i32, "\\lccode65=97"),
            (UC_CODE_BASE + b'a' as i32, "\\uccode97=65"),
            (SF_CODE_BASE + b'A' as i32, "\\sfcode65=999"),
            (MATH_CODE_BASE + b'0' as i32, "\\mathcode48=28720"),
            (INT_BASE + TOLERANCE_CODE, "\\tolerance=10000"),
            (COUNT_BASE, "\\count0=0"),
            (DEL_CODE_BASE, "\\delcode0=-1"),
            (DIMEN_BASE, "\\parindent=0.0pt"),
            (SCALED_BASE, "\\dimen0=0.0pt"),
            (EQTB_SIZE, "\\dimen255=0.0pt"),
            (t.eqtb_top + 1, "?"),
        ] {
            t.show_eqtb(n);
            assert_eq!(printed(t), want, "eqtb[{}]", n);
        }
        Tex::dealloc(t);
    }
}
