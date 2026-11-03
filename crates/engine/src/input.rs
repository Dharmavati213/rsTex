//! §319-353 Parts 21-23: syntactic routines, input stacks and states.
//!
//! `print_cmd_chr` carries all the `@<Cases of |print_cmd_chr|...@>` of the program.  Token lists are
//! in `toks`, not `mem`: docs/PORTING.md, "Token lists".
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
    /// §320
    pub fn print_cmd_chr(&mut self, cmd: i32, chr_code: i32) {
        // §1832 assigns both parameters.
        let (mut cmd, mut chr_code) = (cmd, chr_code);
        match cmd {
            LEFT_BRACE => self.chr_cmd(b"begin-group character ", chr_code),
            RIGHT_BRACE => self.chr_cmd(b"end-group character ", chr_code),
            MATH_SHIFT => self.chr_cmd(b"math shift character ", chr_code),
            MAC_PARAM => self.chr_cmd(b"macro parameter character ", chr_code),
            SUP_MARK => self.chr_cmd(b"superscript character ", chr_code),
            SUB_MARK => self.chr_cmd(b"subscript character ", chr_code),
            ENDV => self.print_s(b"end of alignment template"),
            SPACER => self.chr_cmd(b"blank space ", chr_code),
            LETTER => self.chr_cmd(b"the letter ", chr_code),
            OTHER_CHAR => self.chr_cmd(b"the character ", chr_code),

            // §245
            ASSIGN_GLUE | ASSIGN_MU_GLUE => {
                if chr_code < SKIP_BASE {
                    self.print_skip_param(chr_code - GLUE_BASE);
                } else if chr_code < MU_SKIP_BASE {
                    self.print_esc_s(b"skip");
                    self.print_int(chr_code - SKIP_BASE);
                } else {
                    self.print_esc_s(b"muskip");
                    self.print_int(chr_code - MU_SKIP_BASE);
                }
            }

            // §249
            ASSIGN_TOKS => {
                if chr_code >= TOKS_BASE {
                    self.print_esc_s(b"toks");
                    self.print_int(chr_code - TOKS_BASE);
                } else {
                    match chr_code {
                        OUTPUT_ROUTINE_LOC => self.print_esc_s(b"output"),
                        EVERY_PAR_LOC => self.print_esc_s(b"everypar"),
                        EVERY_MATH_LOC => self.print_esc_s(b"everymath"),
                        EVERY_DISPLAY_LOC => self.print_esc_s(b"everydisplay"),
                        EVERY_HBOX_LOC => self.print_esc_s(b"everyhbox"),
                        EVERY_VBOX_LOC => self.print_esc_s(b"everyvbox"),
                        EVERY_JOB_LOC => self.print_esc_s(b"everyjob"),
                        EVERY_CR_LOC => self.print_esc_s(b"everycr"),
                        // §1658
                        EVERY_EOF_LOC => self.print_esc_s(b"everyeof"),
                        PDF_PAGES_ATTR_LOC => self.print_esc_s(b"pdfpagesattr"),
                        PDF_PAGE_ATTR_LOC => self.print_esc_s(b"pdfpageattr"),
                        PDF_PAGE_RESOURCES_LOC => self.print_esc_s(b"pdfpageresources"),
                        PDF_PK_MODE_LOC => self.print_esc_s(b"pdfpkmode"),
                        _ => self.print_esc_s(b"errhelp"),
                    }
                }
            }

            // §257
            ASSIGN_INT => {
                if chr_code < COUNT_BASE {
                    self.print_param(chr_code - INT_BASE);
                } else {
                    self.print_esc_s(b"count");
                    self.print_int(chr_code - COUNT_BASE);
                }
            }

            // §267
            ASSIGN_DIMEN => {
                if chr_code < SCALED_BASE {
                    self.print_length_param(chr_code - DIMEN_BASE);
                } else {
                    self.print_esc_s(b"dimen");
                    self.print_int(chr_code - SCALED_BASE);
                }
            }

            // §288
            ACCENT => self.print_esc_s(b"accent"),
            ADVANCE => self.print_esc_s(b"advance"),
            AFTER_ASSIGNMENT => self.print_esc_s(b"afterassignment"),
            AFTER_GROUP => self.print_esc_s(b"aftergroup"),
            ASSIGN_FONT_DIMEN => self.print_esc_s(b"fontdimen"),
            BEGIN_GROUP => self.print_esc_s(b"begingroup"),
            BREAK_PENALTY => self.print_esc_s(b"penalty"),
            CHAR_NUM => self.print_esc_s(b"char"),
            CS_NAME => self.print_esc_s(b"csname"),
            DEF_FONT => self.print_esc_s(b"font"),
            LETTERSPACE_FONT => self.print_esc_s(b"letterspacefont"),
            PDF_COPY_FONT => self.print_esc_s(b"pdfcopyfont"),
            DELIM_NUM => self.print_esc_s(b"delimiter"),
            DIVIDE => self.print_esc_s(b"divide"),
            END_CS_NAME => self.print_esc_s(b"endcsname"),
            END_GROUP => self.print_esc_s(b"endgroup"),
            EX_SPACE => self.print_esc(b' ' as i32),
            EXPAND_AFTER => {
                if chr_code == 0 {
                    self.print_esc_s(b"expandafter");
                } else {
                    // §1763
                    self.print_esc_s(b"unless");
                }
            }
            HALIGN => self.print_esc_s(b"halign"),
            HRULE => self.print_esc_s(b"hrule"),
            IGNORE_SPACES => {
                if chr_code == 0 {
                    self.print_esc_s(b"ignorespaces");
                } else {
                    self.print_esc_s(b"pdfprimitive");
                }
            }
            INSERT => self.print_esc_s(b"insert"),
            ITAL_CORR => self.print_esc(b'/' as i32),
            MARK => {
                self.print_esc_s(b"mark");
                if chr_code > 0 {
                    self.print_char(b's' as i32);
                }
            }
            MATH_ACCENT => self.print_esc_s(b"mathaccent"),
            MATH_CHAR_NUM => self.print_esc_s(b"mathchar"),
            MATH_CHOICE => self.print_esc_s(b"mathchoice"),
            MULTIPLY => self.print_esc_s(b"multiply"),
            NO_ALIGN => self.print_esc_s(b"noalign"),
            NO_BOUNDARY => self.print_esc_s(b"noboundary"),
            NO_EXPAND => {
                if chr_code == 0 {
                    self.print_esc_s(b"noexpand");
                } else {
                    self.print_esc_s(b"pdfprimitive");
                }
            }
            NON_SCRIPT => self.print_esc_s(b"nonscript"),
            OMIT => self.print_esc_s(b"omit"),
            RADICAL => self.print_esc_s(b"radical"),
            READ_TO_CS => {
                if chr_code == 0 {
                    self.print_esc_s(b"read");
                } else {
                    // §1760
                    self.print_esc_s(b"readline");
                }
            }
            RELAX => self.print_esc_s(b"relax"),
            SET_BOX => self.print_esc_s(b"setbox"),
            SET_PREV_GRAF => self.print_esc_s(b"prevgraf"),
            SET_SHAPE => match chr_code {
                PAR_SHAPE_LOC => self.print_esc_s(b"parshape"),
                // §1865
                INTER_LINE_PENALTIES_LOC => self.print_esc_s(b"interlinepenalties"),
                CLUB_PENALTIES_LOC => self.print_esc_s(b"clubpenalties"),
                WIDOW_PENALTIES_LOC => self.print_esc_s(b"widowpenalties"),
                DISPLAY_WIDOW_PENALTIES_LOC => self.print_esc_s(b"displaywidowpenalties"),
                _ => {}
            },
            THE => {
                if chr_code == 0 {
                    self.print_esc_s(b"the");
                } else if chr_code == 1 {
                    // §1687
                    self.print_esc_s(b"unexpanded");
                } else {
                    self.print_esc_s(b"detokenize");
                }
            }
            TOKS_REGISTER => {
                // §1833
                self.print_esc_s(b"toks");
                if chr_code != MEM_BOT {
                    self.print_sa_num(chr_code);
                }
            }
            VADJUST => self.print_esc_s(b"vadjust"),
            VALIGN => {
                if chr_code == 0 {
                    self.print_esc_s(b"valign");
                } else {
                    // §1702
                    match chr_code {
                        BEGIN_L_CODE => self.print_esc_s(b"beginL"),
                        END_L_CODE => self.print_esc_s(b"endL"),
                        BEGIN_R_CODE => self.print_esc_s(b"beginR"),
                        _ => self.print_esc_s(b"endR"),
                    }
                }
            }
            VCENTER => self.print_esc_s(b"vcenter"),
            VRULE => self.print_esc_s(b"vrule"),
            PARTOKEN_NAME => self.print_esc_s(b"partokenname"),

            // §357
            PAR_END => self.print_esc_s(b"par"),

            // §403
            INPUT => {
                if chr_code == 0 {
                    self.print_esc_s(b"input");
                } else if chr_code == 2 {
                    // §1748
                    self.print_esc_s(b"scantokens");
                } else {
                    self.print_esc_s(b"endinput");
                }
            }

            // §411
            TOP_BOT_MARK => {
                match chr_code % MARKS_CODE {
                    FIRST_MARK_CODE => self.print_esc_s(b"firstmark"),
                    BOT_MARK_CODE => self.print_esc_s(b"botmark"),
                    SPLIT_FIRST_MARK_CODE => self.print_esc_s(b"splitfirstmark"),
                    SPLIT_BOT_MARK_CODE => self.print_esc_s(b"splitbotmark"),
                    _ => self.print_esc_s(b"topmark"),
                }
                if chr_code >= MARKS_CODE {
                    self.print_char(b's' as i32);
                }
            }

            // §438
            REGISTER => {
                // §1832
                if chr_code < MEM_BOT || chr_code > LO_MEM_STAT_MAX {
                    cmd = self.sa_type(chr_code);
                } else {
                    cmd = chr_code - MEM_BOT;
                    chr_code = NULL;
                }
                if cmd == INT_VAL {
                    self.print_esc_s(b"count");
                } else if cmd == DIMEN_VAL {
                    self.print_esc_s(b"dimen");
                } else if cmd == GLUE_VAL {
                    self.print_esc_s(b"skip");
                } else {
                    self.print_esc_s(b"muskip");
                }
                if chr_code != NULL {
                    self.print_sa_num(chr_code);
                }
            }

            // §443
            SET_AUX => {
                if chr_code == VMODE {
                    self.print_esc_s(b"prevdepth");
                } else {
                    self.print_esc_s(b"spacefactor");
                }
            }
            SET_PAGE_INT => {
                if chr_code == 0 {
                    self.print_esc_s(b"deadcycles");
                } else if chr_code == 2 {
                    // §1693
                    self.print_esc_s(b"interactionmode");
                } else {
                    self.print_esc_s(b"insertpenalties");
                }
            }
            SET_BOX_DIMEN => {
                if chr_code == WIDTH_OFFSET {
                    self.print_esc_s(b"wd");
                } else if chr_code == HEIGHT_OFFSET {
                    self.print_esc_s(b"ht");
                } else {
                    self.print_esc_s(b"dp");
                }
            }
            LAST_ITEM => {
                // §1779's labels: eTeX_expr-int_val+int_val and so on.
                const NUMEXPR_CODE: i32 = ETEX_EXPR - INT_VAL + INT_VAL;
                const DIMEXPR_CODE: i32 = ETEX_EXPR - INT_VAL + DIMEN_VAL;
                const GLUEEXPR_CODE: i32 = ETEX_EXPR - INT_VAL + GLUE_VAL;
                const MUEXPR_CODE: i32 = ETEX_EXPR - INT_VAL + MU_VAL;
                match chr_code {
                    INT_VAL => self.print_esc_s(b"lastpenalty"),
                    DIMEN_VAL => self.print_esc_s(b"lastkern"),
                    GLUE_VAL => self.print_esc_s(b"lastskip"),
                    INPUT_LINE_NO_CODE => self.print_esc_s(b"inputlineno"),
                    // §1650
                    LAST_NODE_TYPE_CODE => self.print_esc_s(b"lastnodetype"),
                    ETEX_VERSION_CODE => self.print_esc_s(b"eTeXversion"),
                    // §1664
                    CURRENT_GROUP_LEVEL_CODE => self.print_esc_s(b"currentgrouplevel"),
                    CURRENT_GROUP_TYPE_CODE => self.print_esc_s(b"currentgrouptype"),
                    // §1667
                    CURRENT_IF_LEVEL_CODE => self.print_esc_s(b"currentiflevel"),
                    CURRENT_IF_TYPE_CODE => self.print_esc_s(b"currentiftype"),
                    CURRENT_IF_BRANCH_CODE => self.print_esc_s(b"currentifbranch"),
                    // §1670
                    FONT_CHAR_WD_CODE => self.print_esc_s(b"fontcharwd"),
                    FONT_CHAR_HT_CODE => self.print_esc_s(b"fontcharht"),
                    FONT_CHAR_DP_CODE => self.print_esc_s(b"fontchardp"),
                    FONT_CHAR_IC_CODE => self.print_esc_s(b"fontcharic"),
                    // §1673
                    PAR_SHAPE_LENGTH_CODE => self.print_esc_s(b"parshapelength"),
                    PAR_SHAPE_INDENT_CODE => self.print_esc_s(b"parshapeindent"),
                    PAR_SHAPE_DIMEN_CODE => self.print_esc_s(b"parshapedimen"),
                    // §1779
                    NUMEXPR_CODE => self.print_esc_s(b"numexpr"),
                    DIMEXPR_CODE => self.print_esc_s(b"dimexpr"),
                    GLUEEXPR_CODE => self.print_esc_s(b"glueexpr"),
                    MUEXPR_CODE => self.print_esc_s(b"muexpr"),
                    // §1802
                    GLUE_STRETCH_ORDER_CODE => self.print_esc_s(b"gluestretchorder"),
                    GLUE_SHRINK_ORDER_CODE => self.print_esc_s(b"glueshrinkorder"),
                    GLUE_STRETCH_CODE => self.print_esc_s(b"gluestretch"),
                    GLUE_SHRINK_CODE => self.print_esc_s(b"glueshrink"),
                    // §1806
                    MU_TO_GLUE_CODE => self.print_esc_s(b"mutoglue"),
                    GLUE_TO_MU_CODE => self.print_esc_s(b"gluetomu"),
                    PDFTEX_VERSION_CODE => self.print_esc_s(b"pdftexversion"),
                    PDF_LAST_OBJ_CODE => self.print_esc_s(b"pdflastobj"),
                    PDF_LAST_XFORM_CODE => self.print_esc_s(b"pdflastxform"),
                    PDF_LAST_XIMAGE_CODE => self.print_esc_s(b"pdflastximage"),
                    PDF_LAST_XIMAGE_PAGES_CODE => self.print_esc_s(b"pdflastximagepages"),
                    PDF_LAST_ANNOT_CODE => self.print_esc_s(b"pdflastannot"),
                    PDF_LAST_X_POS_CODE => self.print_esc_s(b"pdflastxpos"),
                    PDF_LAST_Y_POS_CODE => self.print_esc_s(b"pdflastypos"),
                    PDF_RETVAL_CODE => self.print_esc_s(b"pdfretval"),
                    PDF_LAST_XIMAGE_COLORDEPTH_CODE => self.print_esc_s(b"pdflastximagecolordepth"),
                    ELAPSED_TIME_CODE => self.print_esc_s(b"pdfelapsedtime"),
                    PDF_SHELL_ESCAPE_CODE => self.print_esc_s(b"pdfshellescape"),
                    RANDOM_SEED_CODE => self.print_esc_s(b"pdfrandomseed"),
                    PDF_LAST_LINK_CODE => self.print_esc_s(b"pdflastlink"),
                    _ => self.print_esc_s(b"badness"),
                }
            }

            // §495
            CONVERT => match chr_code {
                NUMBER_CODE => self.print_esc_s(b"number"),
                ROMAN_NUMERAL_CODE => self.print_esc_s(b"romannumeral"),
                STRING_CODE => self.print_esc_s(b"string"),
                MEANING_CODE => self.print_esc_s(b"meaning"),
                FONT_NAME_CODE => self.print_esc_s(b"fontname"),
                ETEX_REVISION_CODE => self.print_esc_s(b"eTeXrevision"),
                EXPANDED_CODE => self.print_esc_s(b"expanded"),
                PDFTEX_REVISION_CODE => self.print_esc_s(b"pdftexrevision"),
                PDFTEX_BANNER_CODE => self.print_esc_s(b"pdftexbanner"),
                PDF_FONT_NAME_CODE => self.print_esc_s(b"pdffontname"),
                PDF_FONT_OBJNUM_CODE => self.print_esc_s(b"pdffontobjnum"),
                PDF_FONT_SIZE_CODE => self.print_esc_s(b"pdffontsize"),
                PDF_PAGE_REF_CODE => self.print_esc_s(b"pdfpageref"),
                LEFT_MARGIN_KERN_CODE => self.print_esc_s(b"leftmarginkern"),
                RIGHT_MARGIN_KERN_CODE => self.print_esc_s(b"rightmarginkern"),
                PDF_XFORM_NAME_CODE => self.print_esc_s(b"pdfxformname"),
                PDF_ESCAPE_STRING_CODE => self.print_esc_s(b"pdfescapestring"),
                PDF_ESCAPE_NAME_CODE => self.print_esc_s(b"pdfescapename"),
                PDF_ESCAPE_HEX_CODE => self.print_esc_s(b"pdfescapehex"),
                PDF_UNESCAPE_HEX_CODE => self.print_esc_s(b"pdfunescapehex"),
                PDF_CREATION_DATE_CODE => self.print_esc_s(b"pdfcreationdate"),
                PDF_FILE_MOD_DATE_CODE => self.print_esc_s(b"pdffilemoddate"),
                PDF_FILE_SIZE_CODE => self.print_esc_s(b"pdffilesize"),
                PDF_MDFIVE_SUM_CODE => self.print_esc_s(b"pdfmdfivesum"),
                PDF_FILE_DUMP_CODE => self.print_esc_s(b"pdffiledump"),
                PDF_MATCH_CODE => self.print_esc_s(b"pdfmatch"),
                PDF_LAST_MATCH_CODE => self.print_esc_s(b"pdflastmatch"),
                PDF_STRCMP_CODE => self.print_esc_s(b"pdfstrcmp"),
                PDF_COLORSTACK_INIT_CODE => self.print_esc_s(b"pdfcolorstackinit"),
                UNIFORM_DEVIATE_CODE => self.print_esc_s(b"pdfuniformdeviate"),
                NORMAL_DEVIATE_CODE => self.print_esc_s(b"pdfnormaldeviate"),
                PDF_INSERT_HT_CODE => self.print_esc_s(b"pdfinsertht"),
                PDF_XIMAGE_BBOX_CODE => self.print_esc_s(b"pdfximagebbox"),
                _ => self.print_esc_s(b"jobname"),
            },

            // §514
            IF_TEST => {
                if chr_code >= UNLESS_CODE {
                    self.print_esc_s(b"unless");
                }
                match chr_code % UNLESS_CODE {
                    IF_CAT_CODE => self.print_esc_s(b"ifcat"),
                    IF_INT_CODE => self.print_esc_s(b"ifnum"),
                    IF_DIM_CODE => self.print_esc_s(b"ifdim"),
                    IF_ODD_CODE => self.print_esc_s(b"ifodd"),
                    IF_VMODE_CODE => self.print_esc_s(b"ifvmode"),
                    IF_HMODE_CODE => self.print_esc_s(b"ifhmode"),
                    IF_MMODE_CODE => self.print_esc_s(b"ifmmode"),
                    IF_INNER_CODE => self.print_esc_s(b"ifinner"),
                    IF_VOID_CODE => self.print_esc_s(b"ifvoid"),
                    IF_HBOX_CODE => self.print_esc_s(b"ifhbox"),
                    IF_VBOX_CODE => self.print_esc_s(b"ifvbox"),
                    IFX_CODE => self.print_esc_s(b"ifx"),
                    IF_EOF_CODE => self.print_esc_s(b"ifeof"),
                    IF_TRUE_CODE => self.print_esc_s(b"iftrue"),
                    IF_FALSE_CODE => self.print_esc_s(b"iffalse"),
                    IF_CASE_CODE => self.print_esc_s(b"ifcase"),
                    IF_PDFPRIMITIVE_CODE => self.print_esc_s(b"ifpdfprimitive"),
                    // §1764
                    IF_DEF_CODE => self.print_esc_s(b"ifdefined"),
                    IF_CS_CODE => self.print_esc_s(b"ifcsname"),
                    IF_FONT_CHAR_CODE => self.print_esc_s(b"iffontchar"),
                    IF_IN_CSNAME_CODE => self.print_esc_s(b"ifincsname"),
                    IF_PDFABS_NUM_CODE => self.print_esc_s(b"ifpdfabsnum"),
                    IF_PDFABS_DIM_CODE => self.print_esc_s(b"ifpdfabsdim"),
                    _ => self.print_esc_s(b"if"),
                }
            }

            // §518
            FI_OR_ELSE => {
                if chr_code == FI_CODE {
                    self.print_esc_s(b"fi");
                } else if chr_code == OR_CODE {
                    self.print_esc_s(b"or");
                } else {
                    self.print_esc_s(b"else");
                }
            }

            // §957
            TAB_MARK => {
                if chr_code == SPAN_CODE {
                    self.print_esc_s(b"span");
                } else {
                    self.chr_cmd(b"alignment tab character ", chr_code);
                }
            }
            CAR_RET => {
                if chr_code == CR_CODE {
                    self.print_esc_s(b"cr");
                } else {
                    self.print_esc_s(b"crcr");
                }
            }

            // §1161
            SET_PAGE_DIMEN => match chr_code {
                0 => self.print_esc_s(b"pagegoal"),
                1 => self.print_esc_s(b"pagetotal"),
                2 => self.print_esc_s(b"pagestretch"),
                3 => self.print_esc_s(b"pagefilstretch"),
                4 => self.print_esc_s(b"pagefillstretch"),
                5 => self.print_esc_s(b"pagefilllstretch"),
                6 => self.print_esc_s(b"pageshrink"),
                _ => self.print_esc_s(b"pagedepth"),
            },

            // §1231
            STOP => {
                if chr_code == 1 {
                    self.print_esc_s(b"dump");
                } else {
                    self.print_esc_s(b"end");
                }
            }

            // §1237
            HSKIP => match chr_code {
                SKIP_CODE => self.print_esc_s(b"hskip"),
                FIL_CODE => self.print_esc_s(b"hfil"),
                FILL_CODE => self.print_esc_s(b"hfill"),
                SS_CODE => self.print_esc_s(b"hss"),
                _ => self.print_esc_s(b"hfilneg"),
            },
            VSKIP => match chr_code {
                SKIP_CODE => self.print_esc_s(b"vskip"),
                FIL_CODE => self.print_esc_s(b"vfil"),
                FILL_CODE => self.print_esc_s(b"vfill"),
                SS_CODE => self.print_esc_s(b"vss"),
                _ => self.print_esc_s(b"vfilneg"),
            },
            MSKIP => self.print_esc_s(b"mskip"),
            KERN => self.print_esc_s(b"kern"),
            MKERN => self.print_esc_s(b"mkern"),

            // §1250
            HMOVE => {
                if chr_code == 1 {
                    self.print_esc_s(b"moveleft");
                } else {
                    self.print_esc_s(b"moveright");
                }
            }
            VMOVE => {
                if chr_code == 1 {
                    self.print_esc_s(b"raise");
                } else {
                    self.print_esc_s(b"lower");
                }
            }
            MAKE_BOX => {
                const VBOX_CODE: i32 = VTOP_CODE + VMODE;
                match chr_code {
                    BOX_CODE => self.print_esc_s(b"box"),
                    COPY_CODE => self.print_esc_s(b"copy"),
                    LAST_BOX_CODE => self.print_esc_s(b"lastbox"),
                    VSPLIT_CODE => self.print_esc_s(b"vsplit"),
                    VTOP_CODE => self.print_esc_s(b"vtop"),
                    VBOX_CODE => self.print_esc_s(b"vbox"),
                    _ => self.print_esc_s(b"hbox"),
                }
            }
            LEADER_SHIP => {
                if chr_code == A_LEADERS {
                    self.print_esc_s(b"leaders");
                } else if chr_code == C_LEADERS {
                    self.print_esc_s(b"cleaders");
                } else if chr_code == X_LEADERS {
                    self.print_esc_s(b"xleaders");
                } else {
                    self.print_esc_s(b"shipout");
                }
            }

            // §1267
            START_PAR => {
                if chr_code == 0 {
                    self.print_esc_s(b"noindent");
                } else if chr_code == 1 {
                    self.print_esc_s(b"indent");
                } else {
                    self.print_esc_s(b"quitvmode");
                }
            }

            // §1286
            REMOVE_ITEM => {
                if chr_code == GLUE_NODE {
                    self.print_esc_s(b"unskip");
                } else if chr_code == KERN_NODE {
                    self.print_esc_s(b"unkern");
                } else {
                    self.print_esc_s(b"unpenalty");
                }
            }
            UN_HBOX => {
                if chr_code == COPY_CODE {
                    self.print_esc_s(b"unhcopy");
                } else {
                    self.print_esc_s(b"unhbox");
                }
            }
            UN_VBOX => {
                if chr_code == COPY_CODE {
                    self.print_esc_s(b"unvcopy");
                } else if chr_code == LAST_BOX_CODE {
                    // §1862
                    self.print_esc_s(b"pagediscards");
                } else if chr_code == VSPLIT_CODE {
                    self.print_esc_s(b"splitdiscards");
                } else {
                    self.print_esc_s(b"unvbox");
                }
            }

            // §1293
            DISCRETIONARY => {
                if chr_code == 1 {
                    self.print_esc(b'-' as i32);
                } else {
                    self.print_esc_s(b"discretionary");
                }
            }

            // §1321
            EQ_NO => {
                if chr_code == 1 {
                    self.print_esc_s(b"leqno");
                } else {
                    self.print_esc_s(b"eqno");
                }
            }

            // §1335
            MATH_COMP => match chr_code {
                ORD_NOAD => self.print_esc_s(b"mathord"),
                OP_NOAD => self.print_esc_s(b"mathop"),
                BIN_NOAD => self.print_esc_s(b"mathbin"),
                REL_NOAD => self.print_esc_s(b"mathrel"),
                OPEN_NOAD => self.print_esc_s(b"mathopen"),
                CLOSE_NOAD => self.print_esc_s(b"mathclose"),
                PUNCT_NOAD => self.print_esc_s(b"mathpunct"),
                INNER_NOAD => self.print_esc_s(b"mathinner"),
                UNDER_NOAD => self.print_esc_s(b"underline"),
                _ => self.print_esc_s(b"overline"),
            },
            LIMIT_SWITCH => {
                if chr_code == LIMITS {
                    self.print_esc_s(b"limits");
                } else if chr_code == NO_LIMITS {
                    self.print_esc_s(b"nolimits");
                } else {
                    self.print_esc_s(b"displaylimits");
                }
            }

            // §1348
            MATH_STYLE => self.print_style(chr_code),

            // §1357
            ABOVE => {
                const ABOVE_DELIMS: i32 = DELIMITED_CODE + ABOVE_CODE;
                const OVER_DELIMS: i32 = DELIMITED_CODE + OVER_CODE;
                const ATOP_DELIMS: i32 = DELIMITED_CODE + ATOP_CODE;
                match chr_code {
                    OVER_CODE => self.print_esc_s(b"over"),
                    ATOP_CODE => self.print_esc_s(b"atop"),
                    ABOVE_DELIMS => self.print_esc_s(b"abovewithdelims"),
                    OVER_DELIMS => self.print_esc_s(b"overwithdelims"),
                    ATOP_DELIMS => self.print_esc_s(b"atopwithdelims"),
                    _ => self.print_esc_s(b"above"),
                }
            }

            // §1367
            LEFT_RIGHT => {
                if chr_code == LEFT_NOAD {
                    self.print_esc_s(b"left");
                } else if chr_code == MIDDLE_NOAD {
                    // §1698
                    self.print_esc_s(b"middle");
                } else {
                    self.print_esc_s(b"right");
                }
            }

            // §1387
            PREFIX => {
                if chr_code == 1 {
                    self.print_esc_s(b"long");
                } else if chr_code == 2 {
                    self.print_esc_s(b"outer");
                } else if chr_code == 8 {
                    // §1771
                    self.print_esc_s(b"protected");
                } else {
                    self.print_esc_s(b"global");
                }
            }
            DEF => {
                if chr_code == 0 {
                    self.print_esc_s(b"def");
                } else if chr_code == 1 {
                    self.print_esc_s(b"gdef");
                } else if chr_code == 2 {
                    self.print_esc_s(b"edef");
                } else {
                    self.print_esc_s(b"xdef");
                }
            }

            // §1398
            LET => {
                if chr_code != NORMAL {
                    self.print_esc_s(b"futurelet");
                } else {
                    self.print_esc_s(b"let");
                }
            }

            // §1401
            SHORTHAND_DEF => match chr_code {
                CHAR_DEF_CODE => self.print_esc_s(b"chardef"),
                MATH_CHAR_DEF_CODE => self.print_esc_s(b"mathchardef"),
                COUNT_DEF_CODE => self.print_esc_s(b"countdef"),
                DIMEN_DEF_CODE => self.print_esc_s(b"dimendef"),
                SKIP_DEF_CODE => self.print_esc_s(b"skipdef"),
                MU_SKIP_DEF_CODE => self.print_esc_s(b"muskipdef"),
                // MLTeX's primitive is not defined here, so this arm is never taken.
                CHAR_SUB_DEF_CODE => self.print_esc_s(b"charsubdef"),
                _ => self.print_esc_s(b"toksdef"),
            },
            CHAR_GIVEN => {
                self.print_esc_s(b"char");
                self.print_hex(chr_code);
            }
            MATH_GIVEN => {
                self.print_esc_s(b"mathchar");
                self.print_hex(chr_code);
            }

            // §1409
            DEF_CODE => {
                if chr_code == CAT_CODE_BASE {
                    self.print_esc_s(b"catcode");
                } else if chr_code == MATH_CODE_BASE {
                    self.print_esc_s(b"mathcode");
                } else if chr_code == LC_CODE_BASE {
                    self.print_esc_s(b"lccode");
                } else if chr_code == UC_CODE_BASE {
                    self.print_esc_s(b"uccode");
                } else if chr_code == SF_CODE_BASE {
                    self.print_esc_s(b"sfcode");
                } else {
                    self.print_esc_s(b"delcode");
                }
            }
            DEF_FAMILY => self.print_size(chr_code - MATH_FONT_BASE),

            // §1429
            HYPH_DATA => {
                if chr_code == 1 {
                    self.print_esc_s(b"patterns");
                } else {
                    self.print_esc_s(b"hyphenation");
                }
            }

            // §1433
            ASSIGN_FONT_INT => match chr_code {
                0 => self.print_esc_s(b"hyphenchar"),
                1 => self.print_esc_s(b"skewchar"),
                LP_CODE_BASE => self.print_esc_s(b"lpcode"),
                RP_CODE_BASE => self.print_esc_s(b"rpcode"),
                EF_CODE_BASE => self.print_esc_s(b"efcode"),
                TAG_CODE => self.print_esc_s(b"tagcode"),
                KN_BS_CODE_BASE => self.print_esc_s(b"knbscode"),
                ST_BS_CODE_BASE => self.print_esc_s(b"stbscode"),
                SH_BS_CODE_BASE => self.print_esc_s(b"shbscode"),
                KN_BC_CODE_BASE => self.print_esc_s(b"knbccode"),
                KN_AC_CODE_BASE => self.print_esc_s(b"knaccode"),
                NO_LIG_CODE => self.print_esc_s(b"pdfnoligatures"),
                _ => {}
            },

            // §1439
            SET_FONT => {
                self.print_s(b"select font ");
                self.slow_print(self.font_name[chr_code]);
                if self.font_size[chr_code] != self.font_dsize[chr_code] {
                    self.print_s(b" at ");
                    self.print_scaled(self.font_size[chr_code]);
                    self.print_s(b"pt");
                }
            }

            // §1441
            SET_INTERACTION => match chr_code {
                BATCH_MODE => self.print_esc_s(b"batchmode"),
                NONSTOP_MODE => self.print_esc_s(b"nonstopmode"),
                SCROLL_MODE => self.print_esc_s(b"scrollmode"),
                _ => self.print_esc_s(b"errorstopmode"),
            },

            // §1451
            IN_STREAM => {
                if chr_code == 0 {
                    self.print_esc_s(b"closein");
                } else {
                    self.print_esc_s(b"openin");
                }
            }

            // §1456
            MESSAGE => {
                if chr_code == 0 {
                    self.print_esc_s(b"message");
                } else {
                    self.print_esc_s(b"errmessage");
                }
            }

            // §1465
            CASE_SHIFT => {
                if chr_code == LC_CODE_BASE {
                    self.print_esc_s(b"lowercase");
                } else {
                    self.print_esc_s(b"uppercase");
                }
            }

            // §1470
            XRAY => match chr_code {
                SHOW_BOX_CODE => self.print_esc_s(b"showbox"),
                SHOW_THE_CODE => self.print_esc_s(b"showthe"),
                SHOW_LISTS_CODE => self.print_esc_s(b"showlists"),
                // §1676
                SHOW_GROUPS => self.print_esc_s(b"showgroups"),
                // §1685
                SHOW_TOKENS => self.print_esc_s(b"showtokens"),
                // §1690
                SHOW_IFS => self.print_esc_s(b"showifs"),
                _ => self.print_esc_s(b"show"),
            },

            // §1473
            UNDEFINED_CS => self.print_s(b"undefined"),
            CALL | LONG_CALL | OUTER_CALL | LONG_OUTER_CALL => {
                let mut n = cmd - CALL;
                if self.toks[chr_code + 1] == PROTECTED_TOKEN {
                    n += 4;
                }
                if odd(n / 4) {
                    self.print_esc_s(b"protected");
                }
                if odd(n) {
                    self.print_esc_s(b"long");
                }
                if odd(n / 2) {
                    self.print_esc_s(b"outer");
                }
                if n > 0 {
                    self.print_char(b' ' as i32);
                }
                self.print_s(b"macro");
            }
            END_TEMPLATE => self.print_esc_s(b"outer endtemplate"),

            // §1526
            EXTENSION => match chr_code {
                OPEN_NODE => self.print_esc_s(b"openout"),
                WRITE_NODE => self.print_esc_s(b"write"),
                CLOSE_NODE => self.print_esc_s(b"closeout"),
                SPECIAL_NODE => self.print_esc_s(b"special"),
                IMMEDIATE_CODE => self.print_esc_s(b"immediate"),
                SET_LANGUAGE_CODE => self.print_esc_s(b"setlanguage"),
                PDF_ANNOT_NODE => self.print_esc_s(b"pdfannot"),
                PDF_CATALOG_CODE => self.print_esc_s(b"pdfcatalog"),
                PDF_DEST_NODE => self.print_esc_s(b"pdfdest"),
                PDF_END_LINK_NODE => self.print_esc_s(b"pdfendlink"),
                PDF_END_THREAD_NODE => self.print_esc_s(b"pdfendthread"),
                PDF_FONT_ATTR_CODE => self.print_esc_s(b"pdffontattr"),
                PDF_FONT_EXPAND_CODE => self.print_esc_s(b"pdffontexpand"),
                PDF_INCLUDE_CHARS_CODE => self.print_esc_s(b"pdfincludechars"),
                PDF_INFO_CODE => self.print_esc_s(b"pdfinfo"),
                PDF_LITERAL_NODE => self.print_esc_s(b"pdfliteral"),
                PDF_COLORSTACK_NODE => self.print_esc_s(b"pdfcolorstack"),
                PDF_SETMATRIX_NODE => self.print_esc_s(b"pdfsetmatrix"),
                PDF_SAVE_NODE => self.print_esc_s(b"pdfsave"),
                PDF_RESTORE_NODE => self.print_esc_s(b"pdfrestore"),
                PDF_MAP_FILE_CODE => self.print_esc_s(b"pdfmapfile"),
                PDF_MAP_LINE_CODE => self.print_esc_s(b"pdfmapline"),
                PDF_NAMES_CODE => self.print_esc_s(b"pdfnames"),
                PDF_OBJ_CODE => self.print_esc_s(b"pdfobj"),
                PDF_OUTLINE_CODE => self.print_esc_s(b"pdfoutline"),
                PDF_REFOBJ_NODE => self.print_esc_s(b"pdfrefobj"),
                PDF_REFXFORM_NODE => self.print_esc_s(b"pdfrefxform"),
                PDF_REFXIMAGE_NODE => self.print_esc_s(b"pdfrefximage"),
                PDF_SAVE_POS_NODE => self.print_esc_s(b"pdfsavepos"),
                PDF_SNAP_REF_POINT_NODE => self.print_esc_s(b"pdfsnaprefpoint"),
                PDF_SNAPY_COMP_NODE => self.print_esc_s(b"pdfsnapycomp"),
                PDF_SNAPY_NODE => self.print_esc_s(b"pdfsnapy"),
                PDF_START_LINK_NODE => self.print_esc_s(b"pdfstartlink"),
                PDF_START_THREAD_NODE => self.print_esc_s(b"pdfstartthread"),
                PDF_THREAD_NODE => self.print_esc_s(b"pdfthread"),
                PDF_TRAILER_CODE => self.print_esc_s(b"pdftrailer"),
                PDF_TRAILER_ID_CODE => self.print_esc_s(b"pdftrailerid"),
                PDF_XFORM_CODE => self.print_esc_s(b"pdfxform"),
                PDF_XIMAGE_CODE => self.print_esc_s(b"pdfximage"),
                RESET_TIMER_CODE => self.print_esc_s(b"pdfresettimer"),
                SET_RANDOM_SEED_CODE => self.print_esc_s(b"pdfsetrandomseed"),
                PDF_NOBUILTIN_TOUNICODE_CODE => self.print_esc_s(b"pdfnobuiltintounicode"),
                PDF_GLYPH_TO_UNICODE_CODE => self.print_esc_s(b"pdfglyphtounicode"),
                PDF_INTERWORD_SPACE_ON_NODE => self.print_esc_s(b"pdfinterwordspaceon"),
                PDF_INTERWORD_SPACE_OFF_NODE => self.print_esc_s(b"pdfinterwordspaceoff"),
                PDF_FAKE_SPACE_NODE => self.print_esc_s(b"pdffakespace"),
                PDF_RUNNING_LINK_OFF_NODE => self.print_esc_s(b"pdfrunninglinkoff"),
                PDF_RUNNING_LINK_ON_NODE => self.print_esc_s(b"pdfrunninglinkon"),
                PDF_SPACE_FONT_CODE => self.print_esc_s(b"pdfspacefont"),
                _ => self.print_s(b"[unknown extension!]"),
            },

            _ => self.print_s(b"[unknown command code!]"),
        }
    }

    /// §321
    pub fn show_cur_cmd_chr(&mut self) {
        self.begin_diagnostic();
        self.print_nl(b'{' as i32);
        if self.mode() != self.shown_mode {
            self.print_mode(self.mode());
            self.print_s(b": ");
            self.shown_mode = self.mode();
        }
        self.print_cmd_chr(self.cur_cmd, self.cur_chr);
        if self.tracing_ifs() > 0 {
            if self.cur_cmd >= IF_TEST && self.cur_cmd <= FI_OR_ELSE {
                self.print_s(b": ");
                let mut n;
                let l;
                if self.cur_cmd == FI_OR_ELSE {
                    self.print_cmd_chr(IF_TEST, self.cur_if);
                    self.print_char(b' ' as i32);
                    n = 0;
                    l = self.if_line;
                } else {
                    n = 1;
                    l = self.line;
                }
                let mut p = self.cond_ptr;
                while p != NULL {
                    n += 1;
                    p = self.link(p);
                }
                self.print_s(b"(level ");
                self.print_int(n);
                self.print_char(b')' as i32);
                self.print_if_line(l);
            }
        }
        self.print_char(b'}' as i32);
        self.end_diagnostic(false);
    }

    /// §328
    pub fn runaway(&mut self) {
        if self.scanner_status > SKIPPING {
            // WEB's p is the head of the list in flight: a mark on the builder here.  The list on top
            // of tb is the one being scanned, since \csname and the like use tx (toks.rs).
            let m = match self.scanner_status {
                DEFINING => {
                    self.print_nl_s(b"Runaway definition");
                    self.tb_def_mark
                }
                MATCHING => {
                    self.print_nl_s(b"Runaway argument");
                    self.tb_match_mark
                }
                ALIGNING => {
                    self.print_nl_s(b"Runaway preamble");
                    self.tb_align_mark
                }
                _ /* ABSORBING */ => {
                    self.print_nl_s(b"Runaway text");
                    self.tb_def_mark
                }
            };
            self.print_char(b'?' as i32);
            self.print_ln();
            let p = self.tb_snapshot(m);
            self.show_token_list(p + 1, NULL, self.error_line - 10);
            self.flush_list(p);
        }
    }

    /// §333
    pub fn show_context(&mut self) {
        self.base_ptr = self.input_ptr;
        self.input_stack[self.base_ptr] = self.cur_input;
        let mut nn = -1;
        let mut bottom_line = false;
        loop {
            self.cur_input = self.input_stack[self.base_ptr];
            if self.state() != TOKEN_LIST {
                if self.name() > 19 || self.base_ptr == 0 {
                    bottom_line = true;
                }
            }
            if self.base_ptr == self.input_ptr || bottom_line || nn < self.error_context_lines() {
                // §334
                if self.base_ptr == self.input_ptr
                    || self.state() != TOKEN_LIST
                    || self.token_type() != BACKED_UP
                    || !self.at_list_end()
                {
                    self.tally = 0;
                    let old_setting = self.selector;
                    // §337: l is the only one of its locals living across a module.
                    let l;
                    if self.state() != TOKEN_LIST {
                        // §335
                        if self.name() <= 17 {
                            if self.terminal_input() {
                                if self.base_ptr == 0 {
                                    self.print_nl_s(b"<*>");
                                } else {
                                    self.print_nl_s(b"<insert> ");
                                }
                            } else {
                                self.print_nl_s(b"<read ");
                                if self.name() == 17 {
                                    self.print_char(b'*' as i32);
                                } else {
                                    self.print_int(self.name() - 1);
                                }
                                self.print_char(b'>' as i32);
                            }
                        } else {
                            self.print_nl_s(b"l.");
                            if self.index() == self.in_open {
                                self.print_int(self.line);
                            } else {
                                self.print_int(self.line_stack[self.index() + 1]);
                            }
                        }
                        self.print_char(b' ' as i32);
                        // §340
                        l = self.begin_pseudoprint();
                        let j = if self.buffer.at(self.limit()) == self.end_line_char() {
                            self.limit()
                        } else {
                            self.limit() + 1
                        };
                        if j > 0 {
                            for i in self.start()..j {
                                if i == self.loc() {
                                    self.set_trick_count();
                                }
                                self.print(self.buffer.at(i));
                            }
                        }
                    } else {
                        // §336
                        match self.token_type() {
                            PARAMETER => self.print_nl_s(b"<argument> "),
                            U_TEMPLATE | V_TEMPLATE => self.print_nl_s(b"<template> "),
                            BACKED_UP => {
                                if self.at_list_end() {
                                    self.print_nl_s(b"<recently read> ");
                                } else {
                                    self.print_nl_s(b"<to be read again> ");
                                }
                            }
                            INSERTED => self.print_nl_s(b"<inserted text> "),
                            MACRO => {
                                self.print_ln();
                                self.print_cs(self.name());
                            }
                            OUTPUT_TEXT => self.print_nl_s(b"<output> "),
                            EVERY_PAR_TEXT => self.print_nl_s(b"<everypar> "),
                            EVERY_MATH_TEXT => self.print_nl_s(b"<everymath> "),
                            EVERY_DISPLAY_TEXT => self.print_nl_s(b"<everydisplay> "),
                            EVERY_HBOX_TEXT => self.print_nl_s(b"<everyhbox> "),
                            EVERY_VBOX_TEXT => self.print_nl_s(b"<everyvbox> "),
                            EVERY_JOB_TEXT => self.print_nl_s(b"<everyjob> "),
                            EVERY_CR_TEXT => self.print_nl_s(b"<everycr> "),
                            MARK_TEXT => self.print_nl_s(b"<mark> "),
                            EVERY_EOF_TEXT => self.print_nl_s(b"<everyeof> "),
                            WRITE_TEXT => self.print_nl_s(b"<write> "),
                            _ => self.print_nl(b'?' as i32),
                        }
                        // §341: start is a header in every case.
                        l = self.begin_pseudoprint();
                        self.show_token_list(self.start() + 1, self.loc(), 100000);
                    }
                    self.selector = old_setting;
                    // §339
                    if self.trick_count == 1000000 {
                        self.set_trick_count();
                    }
                    let m = if self.tally < self.trick_count {
                        self.tally - self.first_count
                    } else {
                        self.trick_count - self.first_count
                    };
                    let mut p;
                    let n;
                    if l + self.first_count <= self.half_error_line {
                        p = 0;
                        n = l + self.first_count;
                    } else {
                        self.print_s(b"...");
                        p = l + self.first_count - self.half_error_line + 3;
                        n = self.half_error_line;
                    }
                    for q in p..self.first_count {
                        self.print_char(self.trick_buf[q % self.error_line]);
                    }
                    self.print_ln();
                    for _ in 1..=n {
                        self.print_char(b' ' as i32);
                    }
                    if m + n <= self.error_line {
                        p = self.first_count + m;
                    } else {
                        p = self.first_count + (self.error_line - n - 3);
                    }
                    for q in self.first_count..p {
                        self.print_char(self.trick_buf[q % self.error_line]);
                    }
                    if m + n > self.error_line {
                        self.print_s(b"...");
                    }
                    nn += 1;
                }
            } else if nn == self.error_context_lines() {
                self.print_nl_s(b"...");
                nn += 1;
            }
            if bottom_line {
                break;
            }
            self.base_ptr -= 1;
        }
        self.cur_input = self.input_stack[self.input_ptr];
    }

    /// §338 `begin_pseudoprint`; returns show_context's local `l`.
    fn begin_pseudoprint(&mut self) -> i32 {
        let l = self.tally;
        self.tally = 0;
        self.selector = PSEUDO;
        self.trick_count = 1000000;
        l
    }

    /// §345
    /// p is a list header in toks, or 0.  Sets loc=p+1 unless t=MACRO.
    pub fn begin_token_list(&mut self, p: i32, t: i32) {
        self.push_input();
        self.set_state(TOKEN_LIST);
        self.set_start(p);
        self.set_token_type(t);
        if t >= MACRO {
            self.add_token_ref(p);
            if t == MACRO {
                self.set_param_start(self.param_ptr);
            } else {
                self.set_loc(p + 1);
                if self.tracing_macros() > 1 {
                    self.begin_diagnostic();
                    self.print_nl_s(b"");
                    match t {
                        MARK_TEXT => self.print_esc_s(b"mark"),
                        WRITE_TEXT => self.print_esc_s(b"write"),
                        _ => self.print_cmd_chr(ASSIGN_TOKS, t - OUTPUT_TEXT + OUTPUT_ROUTINE_LOC),
                    }
                    self.print_s(b"->");
                    self.token_show(p);
                    self.end_diagnostic(false);
                }
            }
        } else {
            // WEB's loc:=p: a list of these types had no header there.
            self.set_loc(p + 1);
        }
    }

    /// §345 for t=parameter, §381's: as much as `begin_token_list` needs when it knows that.
    #[inline(always)]
    pub fn begin_parameter_list(&mut self, p: i32) {
        self.push_input();
        self.set_state(TOKEN_LIST);
        self.set_start(p);
        self.set_token_type(PARAMETER);
        self.set_loc(p + 1);
    }

    /// §345 for t=macro, §390's.
    #[inline(always)]
    pub fn begin_macro_list(&mut self, p: i32) {
        self.push_input();
        self.set_state(TOKEN_LIST);
        self.set_start(p);
        self.set_token_type(MACRO);
        self.add_token_ref(p);
        self.set_param_start(self.param_ptr);
    }

    /// §346
    pub fn end_token_list(&mut self) {
        self.end_token_list_unchecked();
        self.check_interrupt();
    }

    /// §346 without its `check_interrupt`, for `get_next_slow`, which can then do without a frame.
    #[inline(always)]
    pub fn end_token_list_unchecked(&mut self) {
        if self.token_type() >= BACKED_UP {
            if self.token_type() <= INSERTED {
                self.flush_list(self.start());
            } else {
                self.delete_token_ref(self.start());
                if self.token_type() == MACRO {
                    while self.param_ptr > self.param_start() {
                        self.param_ptr -= 1;
                        self.flush_list(self.param_stack[self.param_ptr]);
                    }
                } else if self.token_type() == OUTPUT_TEXT && !self.output_can_end {
                    self.fatal_error(b"Unbalanced output routine");
                }
            }
        } else if self.token_type() == U_TEMPLATE {
            if self.align_state > 500000 {
                self.align_state = 0;
            } else {
                self.fatal_error(b"(interwoven alignment preambles are not allowed)");
            }
        }
        self.pop_input();
    }

    /// §347
    pub fn back_input(&mut self) {
        while self.at_list_end() && self.token_type() != V_TEMPLATE && self.token_type() != OUTPUT_TEXT {
            self.end_token_list();
        }
        let p = self.tok_one(self.cur_tok);
        if self.cur_tok < RIGHT_BRACE_LIMIT {
            if self.cur_tok < LEFT_BRACE_LIMIT {
                self.align_state -= 1;
            } else {
                self.align_state += 1;
            }
        }
        self.push_input();
        self.set_state(TOKEN_LIST);
        self.set_start(p);
        self.set_token_type(BACKED_UP);
        self.set_loc(p + 1);
    }

    /// §349
    pub fn back_error(&mut self) {
        self.OK_to_interrupt = false;
        self.back_input();
        self.OK_to_interrupt = true;
        self.error();
    }

    /// §349
    pub fn ins_error(&mut self) {
        self.OK_to_interrupt = false;
        self.back_input();
        self.set_token_type(INSERTED);
        self.OK_to_interrupt = true;
        self.error();
    }

    /// §350
    pub fn begin_file_reading(&mut self) {
        if self.in_open == self.max_in_open {
            self.overflow(b"text input levels", self.max_in_open);
        }
        if self.first == self.buf_size {
            self.overflow(b"buffer size", self.buf_size);
        }
        self.in_open += 1;
        self.push_input();
        self.set_index(self.in_open);
        let k = self.index();
        self.source_filename_stack[k] = 0;
        self.full_source_filename_stack[k] = 0;
        self.eof_seen[k] = false;
        self.grp_stack[k] = self.cur_boundary;
        self.if_stack[k] = self.cond_ptr;
        self.line_stack[k] = self.line;
        self.set_start(self.first);
        self.set_state(MID_LINE);
        self.set_name(0);
        // §1910
        self.set_synctex_tag(0);
    }

    /// §351
    pub fn end_file_reading(&mut self) {
        self.first = self.start();
        self.line = self.line_stack[self.index()];
        if self.name() == 18 || self.name() == 19 {
            self.pseudo_close();
        } else if self.name() > 17 {
            self.a_close(self.cur_file());
        }
        self.pop_input();
        self.in_open -= 1;
    }

    /// §352
    pub fn clear_for_error_prompt(&mut self) {
        while self.state() != TOKEN_LIST && self.terminal_input() && self.input_ptr > 0 && self.loc() > self.limit() {
            self.end_file_reading();
        }
        self.print_ln();
        self.clear_terminal();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toks::testkit::*;

    #[test]
    fn begin_token_list_conventions() {
        let t = machine();
        // Below macro: no reference taken, loc is the first token.
        let p = t.tok_two(300, 301);
        t.begin_token_list(p, BACKED_UP);
        assert_eq!((t.state(), t.start(), t.token_type(), t.loc()), (TOKEN_LIST, p, BACKED_UP, p + 1));
        assert_eq!(t.token_ref_count(p), 0);
        assert_eq!(t.input_ptr, 1);
        // The null list is an empty level.
        t.begin_token_list(NULL, PARAMETER);
        assert_eq!((t.start(), t.loc()), (NULL, 1));
        assert!(t.at_list_end());
        // Above macro: a reference, and loc is set.
        let q = t.tok_one(400);
        t.begin_token_list(q, EVERY_PAR_TEXT);
        assert_eq!((t.loc(), t.token_ref_count(q)), (q + 1, 1));
        // Macro: a reference, and the caller sets loc and name.
        t.param_ptr = 7;
        let r = t.tok_one(500);
        t.begin_token_list(r, MACRO);
        assert_eq!((t.param_start(), t.token_ref_count(r)), (7, 1));
        Tex::dealloc(t);
    }

    /// `begin_parameter_list` and `begin_macro_list` are `begin_token_list` for those types.
    #[test]
    fn begin_lists_for_one_type_agree() {
        let (a, b) = (machine(), machine());
        for t in [&mut *a, &mut *b] {
            t.set_name(9);
            t.param_ptr = 7;
            t.tok_one(300);
        }
        let (p, q) = (a.tok_one(300), b.tok_one(300));
        assert_eq!(p, q);
        a.begin_token_list(p, PARAMETER);
        b.begin_parameter_list(q);
        let same = |a: &Tex, b: &Tex| {
            let (x, y) = (a.cur_input, b.cur_input);
            assert_eq!((x.state_field, x.index_field, x.start_field, x.loc_field, x.limit_field, x.name_field), (y.state_field, y.index_field, y.start_field, y.loc_field, y.limit_field, y.name_field));
            assert_eq!((a.input_ptr, a.max_in_stack, a.toks[p]), (b.input_ptr, b.max_in_stack, b.toks[q]));
            let (x, y) = (a.input_stack[a.input_ptr - 1], b.input_stack[b.input_ptr - 1]);
            assert_eq!((x.state_field, x.name_field), (y.state_field, y.name_field));
        };
        same(a, b);
        a.begin_token_list(p, MACRO);
        b.begin_macro_list(q);
        same(a, b);
        Tex::dealloc(a);
        Tex::dealloc(b);
    }

    #[test]
    fn end_token_list_frees_by_type() {
        let t = machine();
        // backed_up and inserted: the whole list goes, whatever its count.
        let p = t.tok_one(300);
        t.begin_token_list(p, INSERTED);
        t.end_token_list();
        assert_eq!(t.tok_one(301), p, "freed");
        // Others above macro lose one reference.
        let q = t.tok_two(400, 401);
        t.begin_token_list(q, MARK_TEXT);
        t.end_token_list();
        assert_eq!(t.token_ref_count(q), 0, "alive: the mark itself still refers to it");
        assert_eq!(list(t, q), vec![400, 401]);
        // parameter and the templates free nothing.
        t.begin_token_list(q, PARAMETER);
        t.end_token_list();
        assert_eq!(list(t, q), vec![400, 401]);
        t.begin_token_list(q, V_TEMPLATE);
        t.end_token_list();
        assert_eq!(list(t, q), vec![400, 401]);
        // A u_template that ends resets align_state.
        t.align_state = 1000000;
        t.begin_token_list(q, U_TEMPLATE);
        t.end_token_list();
        assert_eq!(t.align_state, 0);
        assert_eq!(list(t, q), vec![400, 401]);
        Tex::dealloc(t);
    }

    #[test]
    fn end_of_macro_flushes_its_parameters() {
        let t = machine();
        let a = t.tok_one(300);
        t.param_stack[3] = a;
        t.param_stack[4] = NULL;
        t.param_ptr = 5;
        let h = def_macro(t, cs(b'm'), CALL, &[], &[]);
        t.begin_token_list(h, MACRO);
        t.set_param_start(3);
        t.end_token_list();
        assert_eq!(t.param_ptr, 3);
        assert_eq!(t.tok_one(301), a, "the argument was freed");
        assert_eq!(t.token_ref_count(h), 0, "the macro's own reference remains");
        Tex::dealloc(t);
    }

    #[test]
    fn back_input_rereads_and_undoes_align_state() {
        let t = machine();
        feed(t, &[LEFT_BRACE_TOKEN + 123, RIGHT_BRACE_TOKEN + 125, LETTER_TOKEN + 97]);
        t.get_token();
        assert_eq!(t.align_state, 1000001);
        t.back_input();
        assert_eq!(t.align_state, 1000000, "a left brace backed up");
        assert_eq!(t.token_type(), BACKED_UP);
        t.get_token();
        assert_eq!(t.align_state, 1000001);
        assert_eq!(t.cur_tok, LEFT_BRACE_TOKEN + 123);
        t.get_token();
        assert_eq!(t.align_state, 1000000);
        t.back_input();
        assert_eq!(t.align_state, 1000001, "a right brace backed up");
        t.get_token();
        assert_eq!(t.align_state, 1000000);
        assert_eq!(t.cur_tok, RIGHT_BRACE_TOKEN + 125);
        t.cur_tok = LETTER_TOKEN + 122;
        t.back_input();
        assert_eq!(t.align_state, 1000000, "other tokens do not count");
        t.get_token();
        assert_eq!(t.cur_tok, LETTER_TOKEN + 122);
        Tex::dealloc(t);
    }

    #[test]
    fn back_input_conserves_the_stack() {
        let t = machine();
        feed(t, &[LETTER_TOKEN + 97]);
        t.get_token();
        assert_eq!(t.input_ptr, 1);
        // That list is exhausted, so back_input removes it first: the stack does not grow.
        t.back_input();
        assert_eq!(t.input_ptr, 1);
        t.get_token();
        t.back_input();
        assert_eq!(t.input_ptr, 1);
        assert_eq!(drain(t, 0), vec![LETTER_TOKEN + 97]);
        assert_eq!(t.input_ptr, 0);
        // A v_template and an output routine stay on the stack, even when exhausted.
        for ty in [V_TEMPLATE, OUTPUT_TEXT] {
            let p = t.tok_one(LETTER_TOKEN + 98);
            t.begin_token_list(p, ty);
            t.get_token();
            assert!(t.at_list_end());
            let before = t.input_ptr;
            t.back_input();
            assert_eq!(t.input_ptr, before + 1, "not removed: {}", ty);
            t.get_token();
            assert_eq!(t.input_ptr, before + 1, "the backed-up list ended, nothing popped yet");
            t.cur_input.state_field = TOKEN_LIST;
            t.output_can_end = true;
            t.end_token_list();
            t.end_token_list();
            assert_eq!(t.input_ptr, before - 1);
        }
        Tex::dealloc(t);
    }

    #[test]
    fn a_file_level_is_never_at_list_end() {
        let t = machine();
        set_line(t, b"abc", MID_LINE);
        // Whatever toks holds at that index.
        let l = t.loc();
        t.toks[l] = 0;
        assert!(!t.at_list_end());
        t.cur_tok = LETTER_TOKEN + 97;
        t.back_input();
        assert_eq!(t.input_ptr, 1, "back_input must not pop a file level");
        Tex::dealloc(t);
    }

    #[test]
    fn file_levels_nest() {
        let t = machine();
        set_line(t, b"abc", MID_LINE);
        t.get_token();
        let (start, first) = (t.start(), t.first);
        t.line = 12;
        t.in_open = 0;
        t.begin_file_reading();
        assert_eq!((t.in_open, t.index(), t.name(), t.state()), (1, 1, 0, MID_LINE));
        assert_eq!(t.start(), first, "a new level's line begins at first");
        assert_eq!(t.line_stack[1], 12);
        t.line = 3;
        t.first = first + 20;
        t.set_name(1);
        t.end_file_reading();
        assert_eq!(t.first, first);
        assert_eq!(t.line, 12);
        assert_eq!(t.in_open, 0);
        assert_eq!(t.start(), start);
        assert_eq!(t.loc(), start + 1, "the outer line is where it was");
        Tex::dealloc(t);
    }

    fn run_printed(t: &mut Tex, f: impl FnOnce(&mut Tex)) -> String {
        printed(t);
        f(t);
        printed(t)
    }

    #[test]
    fn show_token_list_prints_a_macro() {
        let t = machine();
        print_setup(t);
        // From §313: \def\mac a#1#2 \b {#1\-a ##1#2 #2}
        let list_ = [
            LETTER_TOKEN + 97, MATCH_TOKEN + 35, MATCH_TOKEN + 35, SPACE_TOKEN, cs_tok(b'b'), END_MATCH_TOKEN,
            OUT_PARAM_TOKEN + 1, cs_tok(b'-'), LETTER_TOKEN + 97, SPACE_TOKEN, MAC_PARAM * 256 + 35, OTHER_TOKEN + 49,
            OUT_PARAM_TOKEN + 2, SPACE_TOKEN, OUT_PARAM_TOKEN + 2,
        ];
        let m = t.tb_mark();
        for x in list_ {
            t.tb_push(x);
        }
        let p = t.tb_commit_ref(m);
        let s = run_printed(t, |t| t.token_show(p));
        assert_eq!(s, "a#1#2 \\b ->#1\\-a ##1#2 #2", "§314's example");
        let s = run_printed(t, |t| t.show_token_list(p + 1, NULL, 3));
        assert_eq!(s, "a#1\\ETC.", "the limit l");
        let s = run_printed(t, |t| t.show_token_list(-5, NULL, 100));
        assert_eq!(s, "\\CLOBBERED.");
        let s = run_printed(t, |t| t.show_token_list(t.toks_top + 5, NULL, 100));
        assert_eq!(s, "\\CLOBBERED.");
        Tex::dealloc(t);
    }

    /// Each primitive's meaning, from WEB's `primitive` calls: `print_cmd_chr` is its inverse (§320).
    /// A few classes go to printing routines in other modules, so they are not checked here.
    #[test]
    fn print_cmd_chr_inverts_primitive() {
        let t = machine();
        print_setup(t);
        let data = include_str!("../testdata/primitives.tsv");
        let mut n = 0;
        for l in data.lines() {
            let f: Vec<&str> = l.split('\t').collect();
            let (name, cmd, chr): (&str, i32, i32) = (f[0], f[1].parse().unwrap(), f[2].parse().unwrap());
            let s = run_printed(t, |t| t.print_cmd_chr(cmd, chr));
            assert_eq!(s, format!("\\{}", name), "cmd {} chr {}", cmd, chr);
            n += 1;
        }
        assert!(n > 300, "{} primitives checked", n);
        Tex::dealloc(t);
    }

    #[test]
    fn print_cmd_chr_other_forms() {
        let t = machine();
        print_setup(t);
        for (cmd, chr, want) in [
            (LEFT_BRACE, 123, "begin-group character {"),
            (RIGHT_BRACE, 125, "end-group character }"),
            (MATH_SHIFT, 36, "math shift character $"),
            (TAB_MARK, 38, "alignment tab character &"),
            (TAB_MARK, SPAN_CODE, "\\span"),
            (MAC_PARAM, 35, "macro parameter character #"),
            (SUP_MARK, 94, "superscript character ^"),
            (SUB_MARK, 95, "subscript character _"),
            (ENDV, 0, "end of alignment template"),
            (SPACER, 32, "blank space  "),
            (LETTER, 97, "the letter a"),
            (OTHER_CHAR, 49, "the character 1"),
            (ASSIGN_TOKS, TOKS_BASE + 7, "\\toks7"),
            (ASSIGN_INT, COUNT_BASE + 7, "\\count7"),
            (ASSIGN_DIMEN, SCALED_BASE + 7, "\\dimen7"),
            (ASSIGN_GLUE, SKIP_BASE + 7, "\\skip7"),
            (ASSIGN_MU_GLUE, MU_SKIP_BASE + 7, "\\muskip7"),
            (CHAR_GIVEN, 65, "\\char\"41"),
            (MATH_GIVEN, 65, "\\mathchar\"41"),
            (UNDEFINED_CS, 0, "undefined"),
            (END_TEMPLATE, 0, "\\outer endtemplate"),
            (TOP_BOT_MARK, MARKS_CODE + BOT_MARK_CODE, "\\botmarks"),
            (IF_TEST, UNLESS_CODE + IF_TRUE_CODE, "\\unless\\iftrue"),
            (REGISTER, MEM_BOT + INT_VAL, "\\count"),
            (REGISTER, MEM_BOT + MU_VAL, "\\muskip"),
            (TOKS_REGISTER, MEM_BOT, "\\toks"),
            (EXTENSION, 9999, "[unknown extension!]"),
            (9999, 0, "[unknown command code!]"),
        ] {
            let s = run_printed(t, |t| t.print_cmd_chr(cmd, chr));
            assert_eq!(s, want, "cmd {} chr {}", cmd, chr);
        }
        // Macros: long, outer, protected.
        let plain = def_macro(t, cs(b'a'), CALL, &[], &[]);
        let prot = def_macro(t, cs(b'b'), CALL, &[PROTECTED_TOKEN], &[]);
        for (cmd, chr, want) in [
            (CALL, plain, "macro"),
            (LONG_CALL, plain, "\\long macro"),
            (OUTER_CALL, plain, "\\outer macro"),
            (LONG_OUTER_CALL, plain, "\\long\\outer macro"),
            (CALL, prot, "\\protected macro"),
            (LONG_CALL, prot, "\\protected\\long macro"),
            (LONG_OUTER_CALL, prot, "\\protected\\long\\outer macro"),
        ] {
            let s = run_printed(t, |t| t.print_cmd_chr(cmd, chr));
            assert_eq!(s, want);
        }
        Tex::dealloc(t);
    }

    /// The display of §333-§341, against the oracle's log for
    /// `\def\m#1{x#1\b y}` then `\m{ab} z`, with `\b` undefined; and for the same with `\b` backed up.
    #[test]
    fn show_context_against_the_oracle() {
        let t = machine();
        print_setup(t);
        let h = def_macro(t, cs(b'm'), CALL, &[MATCH_TOKEN + 35], &[LETTER_TOKEN + 120, OUT_PARAM_TOKEN + 1, cs_tok(b'b'), LETTER_TOKEN + 121]);
        t.set_end_line_char(13);
        t.line = 7;
        t.in_open = 1;
        t.set_index(1);
        t.set_name(20);
        set_line(t, b"\\m{ab} z\r", NEW_LINE);
        t.get_x_token(); // x
        t.get_x_token(); // a
        t.get_x_token(); // b
        t.get_token(); // \b
        assert_eq!(t.cur_tok, cs_tok(b'b'));
        let s = run_printed(t, |t| t.show_context());
        assert_eq!(s, "\n\\m #1->x#1\\b \n             y\nl.7 \\m{ab}\n           z");
        t.back_input();
        let s = run_printed(t, |t| t.show_context());
        assert_eq!(s, "<to be read again> \n                   \\b \n\\m #1->x#1\\b \n             y\nl.7 \\m{ab}\n           z");
        // Once it has been read again it is shown as recently read, since it is the top level.  The
        // second line is all spaces, as in any log.
        t.get_token();
        let s = run_printed(t, |t| t.show_context());
        assert_eq!(s, "<recently read> \\b \n                   \n\\m #1->x#1\\b \n             y\nl.7 \\m{ab}\n           z");
        // A backed-up list already read is left out when it is not the top: §334.
        feed(t, &[LETTER_TOKEN + 97]);
        let s = run_printed(t, |t| t.show_context());
        assert_eq!(s, "<inserted text> \n                a\n\\m #1->x#1\\b \n             y\nl.7 \\m{ab}\n           z");
        let _ = h;
        Tex::dealloc(t);
    }

    #[test]
    fn show_context_labels_and_cropping() {
        let t = machine();
        print_setup(t);
        t.set_end_line_char(13);
        t.line = 1;
        t.in_open = 1;
        t.set_index(1);
        t.set_name(20);
        set_line(t, b"x\r", NEW_LINE);
        t.get_token();
        for (ty, label) in [
            (PARAMETER, "<argument> "),
            (U_TEMPLATE, "<template> "),
            (V_TEMPLATE, "<template> "),
            (INSERTED, "<inserted text> "),
            (OUTPUT_TEXT, "<output> "),
            (EVERY_PAR_TEXT, "<everypar> "),
            (EVERY_EOF_TEXT, "<everyeof> "),
            (MARK_TEXT, "<mark> "),
            (WRITE_TEXT, "<write> "),
        ] {
            let p = t.tok_one(LETTER_TOKEN + 97);
            t.begin_token_list(p, ty);
            let s = run_printed(t, |t| t.show_context());
            assert!(s.starts_with(&format!("{}\n", label)), "type {}: got {:?}", ty, s);
            t.set_token_type(PARAMETER);
            t.end_token_list();
        }
        // A list too long for the two lines is cropped with dots on both sides.
        let m = t.tb_mark();
        for _ in 0..200 {
            t.tb_push(LETTER_TOKEN + 97);
        }
        let p = t.tb_commit(m);
        t.begin_token_list(p, INSERTED);
        t.set_loc(p + 1 + 100);
        let s = run_printed(t, |t| t.show_context());
        let lines: Vec<&str> = s.split('\n').collect();
        assert_eq!(lines[0], format!("<inserted text> ...{}", "a".repeat(50 - 16 - 3)));
        assert_eq!(lines[1], format!("{}{}...", " ".repeat(50), "a".repeat(79 - 50 - 3)));
        Tex::dealloc(t);
    }

    #[test]
    fn begin_token_list_traces_above_macro() {
        let t = machine();
        print_setup(t);
        t.set_tracing_macros(2);
        let p = t.tok_one(LETTER_TOKEN + 97);
        let s = run_printed(t, |t| t.begin_token_list(p, MARK_TEXT));
        assert_eq!(s, "\\mark->a\n");
        t.end_token_list();
        t.delete_token_ref(p);
        Tex::dealloc(t);
    }

    #[test]
    fn show_cur_cmd_chr_with_tracing_ifs_against_the_oracle() {
        let t = machine();
        print_setup(t);
        define(t, b"tracingifs", ASSIGN_INT, INT_BASE); // only so that the name exists
        t.set_tracing_ifs(1);
        t.line = 6;
        t.cur_cmd = IF_TEST;
        t.cur_chr = IF_TRUE_CODE;
        let s = run_printed(t, |t| t.show_cur_cmd_chr());
        assert_eq!(s, "{\\iftrue: (level 1) entered on line 6}\n");
        t.cond_ptr = 100;
        t.set_link(100, NULL);
        t.cur_if = IF_TRUE_CODE;
        t.if_line = 6;
        t.cur_cmd = FI_OR_ELSE;
        t.cur_chr = FI_CODE;
        let s = run_printed(t, |t| t.show_cur_cmd_chr());
        assert_eq!(s, "{\\fi: \\iftrue (level 1) entered on line 6}\n");
        Tex::dealloc(t);
    }

    #[test]
    fn print_meaning_of_a_macro_and_a_mark() {
        let t = machine();
        print_setup(t);
        let h = def_macro(t, cs(b'm'), LONG_CALL, &[MATCH_TOKEN + 35], &[LETTER_TOKEN + 122]);
        t.cur_cmd = LONG_CALL;
        t.cur_chr = h;
        let s = run_printed(t, |t| t.print_meaning());
        assert_eq!(s, "\\long macro:\n#1->z");
        let p = t.tok_one(LETTER_TOKEN + 97);
        t.cur_mark[BOT_MARK_CODE] = p;
        t.cur_cmd = TOP_BOT_MARK;
        t.cur_chr = BOT_MARK_CODE;
        let s = run_printed(t, |t| t.print_meaning());
        assert_eq!(s, "\\botmark:\na");
        t.cur_chr = BOT_MARK_CODE + MARKS_CODE;
        let s = run_printed(t, |t| t.print_meaning());
        assert_eq!(s, "\\botmarks", "a class of marks has no fixed text to show");
        Tex::dealloc(t);
    }

    #[test]
    fn clear_for_error_prompt_removes_finished_insertions() {
        let t = machine();
        print_setup(t);
        set_line(t, b"x", MID_LINE);
        t.set_name(0);
        t.max_in_open = 15;
        for _ in 0..2 {
            t.begin_file_reading();
            t.set_limit(t.start() - 1);
            t.set_loc(t.start());
        }
        assert_eq!(t.input_ptr, 2);
        t.clear_for_error_prompt();
        assert_eq!(t.input_ptr, 0, "stops at the bottom level");
        Tex::dealloc(t);
    }
}
