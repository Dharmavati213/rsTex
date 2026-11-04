//! §1520-1647 Part 53: extensions, and the C behind them: the matrix code of utils.c and all of writeimg.c.
//!
//! `\openout`, `\write`, `\closeout`, `\special`, `\immediate`, `\setlanguage` and every `\pdf...` primitive that is an
//! `extension` command, with what they do at shipout that is not in `pdfship.rs`.  The node accessors are in `access.rs`.
//!
//! **The image table.**  `writeimg.c`'s `image_entry` is `ImageEntry`, in `v_ext.images`, a `PodVec` in the arena, with the
//! file names in `v_ext.image_names`, so a format and a checkpoint carry it.  What the C keeps behind the entry's pointers
//! (a `png_struct`, an open file, a PDF document) is plain Rust data in `HVars`, which neither holds.  So `HVars.loaded`
//! says for each image whether this process has that data.  `ext_image_restore` makes it from `load_fmt_file`, as
//! `undumpimagemeta` does, see `ext_undump_images`, and `ext_images_after_restore` first thing after a checkpoint.
//!
//! **Files.**  `crates/image` gets byte slices.  They come through `self.host.world`, so an image read is an observation.
#![forbid(unsafe_code)]
#![deny(unreachable_patterns, unused_variables)]
#![allow(non_snake_case, unused_mut, unused_imports, dead_code)]

use std::collections::BTreeMap;

use rstex_image::epdf::{EpdfDocs, EpdfParams, PdfInfoParams};
use rstex_image::jbig2::{self, Jbig2File};
use rstex_image::out::{ObjNo, Op, SlotAlloc};
use rstex_image::{self, Fail, ImageType, jpg, png};

use crate::access::*;
use crate::consts::*;
use crate::host::*;
use crate::raw::*;
use crate::state::*;

// `IMAGE_TYPE_*` in image.h.
const IMAGE_TYPE_PDF: i32 = 1;
const IMAGE_TYPE_PNG: i32 = 2;
const IMAGE_TYPE_JPG: i32 = 3;
const IMAGE_TYPE_JBIG2: i32 = 5;

/// `PRINTF_BUF_SIZE` in ptexmac.h: every message of the C side goes through `vsnprintf` into a buffer this big.
const PRINT_BUF_SIZE: usize = 1024;

crate::pod_struct! {
    /// writeimg.c's `image_entry`.  `width` and `height` are `integer` in the C, so 64 bits: JBIG2 can give values
    /// from 2^31 to 2^32-1, and `scale_image` must see them.  `orig_x` to `page_box` are the `pdf_image_struct`,
    /// `selected_page` also the `JBIG2_IMAGE_INFO`, and `doc` is an index in `HVars.docs`.
    pub struct ImageEntry {
        pub image_type: i32,
        pub color_type: i32,
        pub width: i64,
        pub height: i64,
        pub rotate: i32,
        pub x_res: i32,
        pub y_res: i32,
        pub num_pages: i32,
        pub colorspace_ref: i32,
        pub group_ref: i32,
        pub color_depth: i32,
        pub orig_x: i32,
        pub orig_y: i32,
        pub selected_page: i32,
        pub page_box: i32,
        pub doc: i32,
        pub name_start: i32,
        pub name_len: i32,
    }
}

crate::pod_struct! {
    /// POD state private to this module.  Add fields here, not to state.rs.
    pub struct Vars {
        /// utils.c `ret_llx` and so on: what `getllx` and the others read.
        pub ret_llx: i32,
        pub ret_lly: i32,
        pub ret_urx: i32,
        pub ret_ury: i32,
        /// utils.c `last_llx` and so on, for `matrixrecalculate`.
        pub last_llx: i32,
        pub last_lly: i32,
        pub last_urx: i32,
        pub last_ury: i32,
        /// writeimg.c `image_array`, and the bytes of the `img_name`s.
        pub images: PodVec<ImageEntry>,
        pub image_names: PodVec<u8>,
        /// writepng.c.  Statics, and the C does not dump them.
        pub transparent_page_group: i32,
        pub transparent_page_group_was_written: bool,
    }
}

/// An open PNG or JPEG: the C keeps the file open from `read_image` to `delete_image`.
enum HostImage {
    Png { info: png::PngInfo, data: Vec<u8> },
    Jpg { info: jpg::JpgInfo, data: Vec<u8> },
}

/// Non-POD state private to this module, in `self.host`.
#[derive(Default)]
pub struct HVars {
    /// For each image: this process has read its file.
    loaded: Vec<bool>,
    /// For each image, if it is a PNG or JPEG not yet deleted.
    files: Vec<Option<HostImage>>,
    /// pdftoepdf.cc's `pdfDocuments`.
    docs: EpdfDocs,
    /// writejbig2.c's `file_tree`, in `strcmp` order of the names, with the bytes of each file.
    jbig2: BTreeMap<Vec<u8>, (Jbig2File, Vec<u8>)>,
    /// The object numbers of `crates/image`'s slots.
    slots: SlotAlloc,
    table: Vec<i32>,
    /// `fd`, by the slot of its object number: `Op::EpdfFont` and `Op::EpdfFontName`.
    epdf_fds: Vec<(u32, i32)>,
}

impl Tex {
    /// §1528
    pub fn do_extension(&mut self) {
        let (mut i, mut j, mut k): (i32, i32, i32);
        let (mut p, mut q, mut r): (i32, i32, i32);
        match self.cur_chr {
            OPEN_NODE => {
                // §1531 Implement \openout
                self.new_write_whatsit(OPEN_NODE_SIZE);
                self.scan_optional_equals();
                self.scan_file_name();
                self.set_open_name(self.tail(), self.cur_name);
                self.set_open_area(self.tail(), self.cur_area);
                self.set_open_ext(self.tail(), self.cur_ext);
            }
            WRITE_NODE => {
                // §1532 Implement \write
                k = self.cur_cs;
                self.new_write_whatsit(WRITE_NODE_SIZE);
                self.cur_cs = k;
                self.scan_toks(false, false);
                self.set_write_tokens(self.tail(), self.def_ref);
            }
            CLOSE_NODE => {
                // §1533 Implement \closeout
                self.new_write_whatsit(WRITE_NODE_SIZE);
                self.set_write_tokens(self.tail(), NULL);
            }
            SPECIAL_NODE => {
                // §1534 Implement \special
                if self.scan_keyword(b"shipout") {
                    self.new_whatsit(LATESPECIAL_NODE, WRITE_NODE_SIZE);
                    self.set_write_stream(self.tail(), NULL);
                    self.scan_toks(false, false);
                    self.set_write_tokens(self.tail(), self.def_ref);
                } else {
                    self.new_whatsit(SPECIAL_NODE, WRITE_NODE_SIZE);
                    self.set_write_stream(self.tail(), NULL);
                    self.scan_toks(false, true);
                    self.set_write_tokens(self.tail(), self.def_ref);
                }
            }
            IMMEDIATE_CODE => {
                // §1623 Implement \immediate
                self.get_x_token();
                if self.cur_cmd == EXTENSION {
                    if self.cur_chr <= CLOSE_NODE {
                        p = self.tail();
                        self.do_extension(); // append a whatsit node
                        self.out_what(self.tail()); // do the action immediately
                        self.flush_node_list(self.tail());
                        self.set_tail(p);
                        self.set_link(p, NULL);
                    } else {
                        match self.cur_chr {
                            PDF_OBJ_CODE => {
                                self.do_extension(); // scan object and set pdf_last_obj
                                if self.obj_data_ptr(self.pdf_last_obj) == 0 {
                                    self.pdf_error(b"ext1", b"`\\pdfobj reserveobjnum' cannot be used with \\immediate");
                                }
                                self.pdf_write_obj(self.pdf_last_obj);
                            }
                            PDF_XFORM_CODE => {
                                self.do_extension(); // scan form and set pdf_last_xform
                                self.pdf_cur_form = self.pdf_last_xform;
                                self.pdf_ship_out(self.obj_xform_box(self.pdf_last_xform), false);
                            }
                            PDF_XIMAGE_CODE => {
                                self.do_extension(); // scan image and set pdf_last_ximage
                                self.pdf_write_image(self.pdf_last_ximage);
                            }
                            _ => self.back_input(),
                        }
                    }
                } else {
                    self.back_input();
                }
            }
            SET_LANGUAGE_CODE => {
                // §1625 Implement \setlanguage
                if abs(self.mode()) != HMODE {
                    self.report_illegal_case();
                } else {
                    self.new_whatsit(LANGUAGE_NODE, SMALL_NODE_SIZE);
                    self.scan_int();
                    if self.cur_val <= 0 {
                        self.set_clang(0);
                    } else if self.cur_val > 255 {
                        self.set_clang(0);
                    } else {
                        self.set_clang(self.cur_val);
                    }
                    self.set_what_lang(self.tail(), self.clang());
                    let v = self.norm_min(self.left_hyphen_min());
                    self.set_what_lhm(self.tail(), v);
                    let v = self.norm_min(self.right_hyphen_min());
                    self.set_what_rhm(self.tail(), v);
                }
            }
            PDF_ANNOT_NODE => {
                // §1558 Implement \pdfannot
                self.check_pdfoutput(b"\\pdfannot", true);
                if self.scan_keyword(b"reserveobjnum") {
                    self.pdf_last_annot = self.pdf_new_objnum();
                    self.ext_scan_optional_space();
                } else {
                    if self.scan_keyword(b"useobjnum") {
                        self.scan_int();
                        k = self.cur_val;
                        if k <= 0 || k > self.obj_ptr || self.obj_annot_ptr(k) != 0 {
                            self.pdf_error(b"ext1", b"invalid object number");
                        }
                    } else {
                        k = self.pdf_new_objnum();
                    }
                    self.new_annot_whatsit(PDF_ANNOT_NODE, PDF_ANNOT_NODE_SIZE);
                    self.set_pdf_annot_objnum(self.tail(), k);
                    self.scan_pdf_ext_toks();
                    self.set_pdf_annot_data(self.tail(), self.def_ref);
                    self.pdf_last_annot = k;
                }
            }
            PDF_CATALOG_CODE => {
                // §1579 Implement \pdfcatalog
                self.check_pdfoutput(b"\\pdfcatalog", false);
                self.scan_pdf_ext_toks();
                if self.pdf_output() > 0 {
                    self.pdf_catalog_toks = self.concat_tokens(self.pdf_catalog_toks, self.def_ref);
                }
                if self.scan_keyword(b"openaction") {
                    if self.pdf_catalog_openaction != 0 {
                        self.pdf_error(b"ext1", b"duplicate of openaction");
                    } else {
                        p = self.scan_action();
                        self.pdf_new_obj(OBJ_TYPE_OTHERS, 0, 1);
                        if self.pdf_output() > 0 {
                            self.pdf_catalog_openaction = self.obj_ptr;
                        }
                        self.write_action(p);
                        self.pdf_end_obj();
                        self.delete_action_ref(p);
                    }
                }
            }
            PDF_DEST_NODE => {
                // §1565 Implement \pdfdest
                self.check_pdfoutput(b"\\pdfdest", true);
                q = self.tail();
                self.new_whatsit(PDF_DEST_NODE, PDF_DEST_NODE_SIZE);
                if self.scan_keyword(b"struct") {
                    self.scan_int();
                    if self.cur_val <= 0 {
                        self.pdf_error(b"ext1", b"struct identifier must be positive");
                    }
                    self.set_pdf_dest_objnum(self.tail(), self.cur_val);
                    j = OBJ_TYPE_STRUCT_DEST;
                } else {
                    self.set_pdf_dest_objnum(self.tail(), NULL);
                    j = OBJ_TYPE_DEST;
                }
                if self.scan_keyword(b"num") {
                    self.scan_int();
                    if self.cur_val <= 0 {
                        self.pdf_error(b"ext1", b"num identifier must be positive");
                    }
                    if self.cur_val > MAX_HALFWORD {
                        self.pdf_error(b"ext1", b"number too big");
                    }
                    self.set_pdf_dest_id(self.tail(), self.cur_val);
                    self.set_pdf_dest_named_id(self.tail(), 0);
                } else if self.scan_keyword(b"name") {
                    self.scan_pdf_ext_toks();
                    self.set_pdf_dest_id(self.tail(), self.def_ref);
                    self.set_pdf_dest_named_id(self.tail(), 1);
                } else {
                    self.pdf_error(b"ext1", b"identifier type missing");
                }
                if self.scan_keyword(b"xyz") {
                    self.set_pdf_dest_type(self.tail(), PDF_DEST_XYZ);
                    if self.scan_keyword(b"zoom") {
                        self.scan_int();
                        if self.cur_val > MAX_HALFWORD {
                            self.pdf_error(b"ext1", b"number too big");
                        }
                        self.set_pdf_dest_xyz_zoom(self.tail(), self.cur_val);
                    } else {
                        // zoom 0 is a number here: docs/PORTING.md section 6.
                        self.set_pdf_dest_xyz_zoom(self.tail(), PDF_ZOOM_NULL);
                    }
                } else if self.scan_keyword(b"fitbh") {
                    self.set_pdf_dest_type(self.tail(), PDF_DEST_FITBH);
                } else if self.scan_keyword(b"fitbv") {
                    self.set_pdf_dest_type(self.tail(), PDF_DEST_FITBV);
                } else if self.scan_keyword(b"fitb") {
                    self.set_pdf_dest_type(self.tail(), PDF_DEST_FITB);
                } else if self.scan_keyword(b"fith") {
                    self.set_pdf_dest_type(self.tail(), PDF_DEST_FITH);
                } else if self.scan_keyword(b"fitv") {
                    self.set_pdf_dest_type(self.tail(), PDF_DEST_FITV);
                } else if self.scan_keyword(b"fitr") {
                    self.set_pdf_dest_type(self.tail(), PDF_DEST_FITR);
                } else if self.scan_keyword(b"fit") {
                    self.set_pdf_dest_type(self.tail(), PDF_DEST_FIT);
                } else {
                    self.pdf_error(b"ext1", b"destination type missing");
                }
                self.ext_scan_optional_space();
                if self.pdf_dest_type(self.tail()) == PDF_DEST_FITR {
                    self.scan_alt_rule(); // scans <rule spec> to alt_rule
                    self.set_pdf_width(self.tail(), self.width(self.alt_rule));
                    self.set_pdf_height(self.tail(), self.height(self.alt_rule));
                    self.set_pdf_depth(self.tail(), self.depth(self.alt_rule));
                }
                if self.pdf_dest_named_id(self.tail()) != 0 {
                    i = self.tokens_to_string(self.pdf_dest_id(self.tail()));
                    k = self.find_obj(j, i, true);
                    self.flush_str(i);
                } else {
                    k = self.find_obj(j, self.pdf_dest_id(self.tail()), false);
                }
                if k != 0 && self.obj_dest_ptr(k) != NULL {
                    self.warn_dest_dup(
                        self.pdf_dest_id(self.tail()),
                        self.pdf_dest_named_id(self.tail()),
                        b"ext4",
                        b"has been already used, duplicate ignored",
                    );
                    self.flush_node_list(self.tail());
                    self.set_tail(q);
                    self.set_link(q, NULL);
                }
            }
            PDF_END_LINK_NODE => {
                // §1561 Implement \pdfendlink
                self.check_pdfoutput(b"\\pdfendlink", true);
                if abs(self.mode()) == VMODE {
                    self.pdf_error(b"ext1", b"\\pdfendlink cannot be used in vertical mode");
                }
                self.new_whatsit(PDF_END_LINK_NODE, SMALL_NODE_SIZE);
            }
            PDF_END_THREAD_NODE => {
                // §1569 Implement \pdfendthread
                self.check_pdfoutput(b"\\pdfendthread", true);
                self.new_whatsit(PDF_END_THREAD_NODE, SMALL_NODE_SIZE);
            }
            PDF_FONT_ATTR_CODE => {
                // §1589 Implement \pdffontattr
                self.check_pdfoutput(b"\\pdffontattr", true);
                self.scan_font_ident();
                k = self.cur_val;
                if k == NULL_FONT {
                    self.pdf_error(b"font", b"invalid font identifier");
                }
                self.scan_pdf_ext_toks();
                let s = self.tokens_to_string(self.def_ref);
                self.pdf_font_attr[k] = s;
            }
            PDF_FONT_EXPAND_CODE => {
                // §1535 Implement \pdffontexpand
                self.read_expand_font();
            }
            PDF_INCLUDE_CHARS_CODE => {
                // §1588 Implement \pdfincludechars
                self.check_pdfoutput(b"\\pdfincludechars", true);
                self.pdf_include_chars();
            }
            PDF_INFO_CODE => {
                // §1578 Implement \pdfinfo
                self.check_pdfoutput(b"\\pdfinfo", false);
                self.scan_pdf_ext_toks();
                if self.pdf_output() > 0 {
                    self.pdf_info_toks = self.concat_tokens(self.pdf_info_toks, self.def_ref);
                }
            }
            PDF_LITERAL_NODE => {
                // §1538 Implement \pdfliteral
                self.check_pdfoutput(b"\\pdfliteral", true);
                if self.scan_keyword(b"shipout") {
                    k = PDF_LATELITERAL_NODE;
                } else {
                    k = PDF_LITERAL_NODE;
                }
                self.new_whatsit(k, WRITE_NODE_SIZE);
                if self.scan_keyword(b"direct") {
                    self.set_pdf_literal_mode(self.tail(), DIRECT_ALWAYS);
                } else if self.scan_keyword(b"page") {
                    self.set_pdf_literal_mode(self.tail(), DIRECT_PAGE);
                } else {
                    self.set_pdf_literal_mode(self.tail(), SET_ORIGIN);
                }
                if k == PDF_LITERAL_NODE {
                    self.scan_pdf_ext_toks();
                } else {
                    self.scan_pdf_ext_late_toks();
                }
                self.set_pdf_literal_data(self.tail(), self.def_ref);
            }
            PDF_COLORSTACK_NODE => {
                // §1539 Implement \pdfcolorstack
                self.check_pdfoutput(b"\\pdfcolorstack", true);
                // Scan and check the stack number and store in cur_val
                self.scan_int();
                if self.cur_val >= self.colorstackused() {
                    self.print_err_s(b"Unknown color stack number ");
                    self.print_int(self.cur_val);
                    self.help(&[
                        b"Allocate and initialize a color stack with \\\\pdfcolorstackinit.",
                        b"I'll use default color stack 0 here.",
                        b"Proceed, with fingers crossed.",
                    ]);
                    self.error();
                    self.cur_val = 0;
                }
                if self.cur_val < 0 {
                    self.print_err_s(b"Invalid negative color stack number");
                    self.help(&[b"I'll use default color stack 0 here.", b"Proceed, with fingers crossed."]);
                    self.error();
                    self.cur_val = 0;
                }
                // Scan the command and store in i, j holds the node size
                j = 0;
                if self.scan_keyword(b"set") {
                    i = COLORSTACK_SET;
                    j = PDF_COLORSTACK_SETTER_NODE_SIZE;
                } else if self.scan_keyword(b"push") {
                    i = COLORSTACK_PUSH;
                    j = PDF_COLORSTACK_SETTER_NODE_SIZE;
                } else if self.scan_keyword(b"pop") {
                    i = COLORSTACK_POP;
                    j = PDF_COLORSTACK_GETTER_NODE_SIZE;
                } else if self.scan_keyword(b"current") {
                    i = COLORSTACK_CURRENT;
                    j = PDF_COLORSTACK_GETTER_NODE_SIZE;
                } else {
                    i = -1; // error
                }
                if i >= 0 {
                    self.new_whatsit(PDF_COLORSTACK_NODE, j);
                    self.set_pdf_colorstack_stack(self.tail(), self.cur_val);
                    self.set_pdf_colorstack_cmd(self.tail(), i);
                    if i <= COLORSTACK_DATA {
                        self.scan_pdf_ext_toks();
                        self.set_pdf_colorstack_data(self.tail(), self.def_ref);
                    }
                } else {
                    self.print_err_s(b"Color stack action is missing");
                    self.help(&[
                        b"The expected actions for \\pdfcolorstack:",
                        b"    set, push, pop, current",
                        b"I'll ignore the color stack command.",
                    ]);
                    self.error();
                }
            }
            PDF_SETMATRIX_NODE => {
                // §1540 Implement \pdfsetmatrix
                self.check_pdfoutput(b"\\pdfsetmatrix", true);
                self.new_whatsit(PDF_SETMATRIX_NODE, PDF_SETMATRIX_NODE_SIZE);
                self.scan_pdf_ext_toks();
                self.set_pdf_setmatrix_data(self.tail(), self.def_ref);
            }
            PDF_SAVE_NODE => {
                // §1541 Implement \pdfsave
                self.check_pdfoutput(b"\\pdfsave", true);
                self.new_whatsit(PDF_SAVE_NODE, PDF_SAVE_NODE_SIZE);
            }
            PDF_RESTORE_NODE => {
                // §1542 Implement \pdfrestore
                self.check_pdfoutput(b"\\pdfrestore", true);
                self.new_whatsit(PDF_RESTORE_NODE, PDF_RESTORE_NODE_SIZE);
            }
            PDF_MAP_FILE_CODE => {
                // §1590 Implement \pdfmapfile
                self.check_pdfoutput(b"\\pdfmapfile", true);
                self.scan_pdf_ext_toks();
                self.pdfmapfile(self.def_ref);
                self.delete_token_ref(self.def_ref);
            }
            PDF_MAP_LINE_CODE => {
                // §1591 Implement \pdfmapline
                self.check_pdfoutput(b"\\pdfmapline", true);
                self.scan_pdf_ext_toks();
                self.pdfmapline(self.def_ref);
                self.delete_token_ref(self.def_ref);
            }
            PDF_NAMES_CODE => {
                // §1580 Implement \pdfnames
                self.check_pdfoutput(b"\\pdfnames", true);
                self.scan_pdf_ext_toks();
                self.pdf_names_toks = self.concat_tokens(self.pdf_names_toks, self.def_ref);
            }
            PDF_OBJ_CODE => {
                // §1544 Implement \pdfobj
                self.check_pdfoutput(b"\\pdfobj", true);
                if self.scan_keyword(b"reserveobjnum") {
                    self.ext_scan_optional_space();
                    self.pdf_obj_count += 1;
                    self.pdf_create_obj(OBJ_TYPE_OBJ, self.pdf_obj_count);
                    self.pdf_last_obj = self.obj_ptr;
                } else {
                    k = -1;
                    if self.scan_keyword(b"useobjnum") {
                        self.scan_int();
                        k = self.cur_val;
                        if k <= 0 || k > self.obj_ptr || self.obj_data_ptr(k) != 0 {
                            self.pdf_warning(b"\\pdfobj", b"invalid object number being ignored", true, true);
                            self.pdf_retval = -1; // signal the problem
                            k = -1; // will be generated again
                        }
                    }
                    if k < 0 {
                        self.pdf_obj_count += 1;
                        self.pdf_create_obj(OBJ_TYPE_OBJ, self.pdf_obj_count);
                        k = self.obj_ptr;
                    }
                    let m = self.pdf_get_mem(PDFMEM_OBJ_SIZE);
                    self.set_obj_data_ptr(k, m);
                    if self.scan_keyword(b"stream") {
                        self.set_obj_obj_is_stream(k, 1);
                        if self.scan_keyword(b"attr") {
                            self.scan_pdf_ext_toks();
                            self.set_obj_obj_stream_attr(k, self.def_ref);
                        } else {
                            self.set_obj_obj_stream_attr(k, NULL);
                        }
                    } else {
                        self.set_obj_obj_is_stream(k, 0);
                    }
                    if self.scan_keyword(b"file") {
                        self.set_obj_obj_is_file(k, 1);
                    } else {
                        self.set_obj_obj_is_file(k, 0);
                    }
                    self.scan_pdf_ext_toks();
                    self.set_obj_obj_data(k, self.def_ref);
                    self.pdf_last_obj = k;
                }
            }
            PDF_OUTLINE_CODE => {
                // §1563 Implement \pdfoutline
                self.check_pdfoutput(b"\\pdfoutline", true);
                if self.scan_keyword(b"attr") {
                    self.scan_pdf_ext_toks();
                    r = self.def_ref;
                } else {
                    r = 0;
                }
                p = self.scan_action();
                if self.scan_keyword(b"count") {
                    self.scan_int();
                    i = self.cur_val;
                } else {
                    i = 0;
                }
                self.scan_pdf_ext_toks();
                q = self.def_ref;
                self.pdf_new_obj(OBJ_TYPE_OTHERS, 0, 1);
                j = self.obj_ptr;
                self.write_action(p);
                self.pdf_end_obj();
                self.delete_action_ref(p);
                self.pdf_create_obj(OBJ_TYPE_OUTLINE, 0);
                k = self.obj_ptr;
                let m = self.pdf_get_mem(PDFMEM_OUTLINE_SIZE);
                self.set_obj_outline_ptr(k, m);
                self.set_obj_outline_action_objnum(k, j);
                self.set_obj_outline_count(k, i);
                self.pdf_new_obj(OBJ_TYPE_OTHERS, 0, 1);
                let s = self.tokens_to_string(q);
                self.pdf_print_str_ln(s);
                self.flush_str(self.last_tokens_string);
                self.delete_token_ref(q);
                self.pdf_end_obj();
                self.set_obj_outline_title(k, self.obj_ptr);
                self.set_obj_outline_prev(k, 0);
                self.set_obj_outline_next(k, 0);
                self.set_obj_outline_first(k, 0);
                self.set_obj_outline_last(k, 0);
                self.set_obj_outline_parent(k, self.pdf_parent_outline);
                self.set_obj_outline_attr(k, r);
                if self.pdf_first_outline == 0 {
                    self.pdf_first_outline = k;
                }
                if self.pdf_last_outline == 0 {
                    if self.pdf_parent_outline != 0 {
                        self.set_obj_outline_first(self.pdf_parent_outline, k);
                    }
                } else {
                    self.set_obj_outline_next(self.pdf_last_outline, k);
                    self.set_obj_outline_prev(k, self.pdf_last_outline);
                }
                self.pdf_last_outline = k;
                if self.obj_outline_count(k) != 0 {
                    self.pdf_parent_outline = k;
                    self.pdf_last_outline = 0;
                } else if self.pdf_parent_outline != 0
                    && self.outline_list_count(k) == abs(self.obj_outline_count(self.pdf_parent_outline))
                {
                    j = self.pdf_last_outline;
                    loop {
                        self.set_obj_outline_last(self.pdf_parent_outline, j);
                        j = self.pdf_parent_outline;
                        self.pdf_parent_outline = self.obj_outline_parent(self.pdf_parent_outline);
                        if self.pdf_parent_outline == 0
                            || self.outline_list_count(j) < abs(self.obj_outline_count(self.pdf_parent_outline))
                        {
                            break;
                        }
                    }
                    if self.pdf_parent_outline == 0 {
                        self.pdf_last_outline = self.pdf_first_outline;
                    } else {
                        self.pdf_last_outline = self.obj_outline_first(self.pdf_parent_outline);
                    }
                    while self.obj_outline_next(self.pdf_last_outline) != 0 {
                        self.pdf_last_outline = self.obj_outline_next(self.pdf_last_outline);
                    }
                }
            }
            PDF_REFOBJ_NODE => {
                // §1546 Implement \pdfrefobj
                self.check_pdfoutput(b"\\pdfrefobj", true);
                self.scan_int();
                self.pdf_check_obj(OBJ_TYPE_OBJ, self.cur_val);
                self.new_whatsit(PDF_REFOBJ_NODE, PDF_REFOBJ_NODE_SIZE);
                self.set_pdf_obj_objnum(self.tail(), self.cur_val);
            }
            PDF_REFXFORM_NODE => {
                // §1549 Implement \pdfrefxform
                self.check_pdfoutput(b"\\pdfrefxform", true);
                self.scan_int();
                self.pdf_check_obj(OBJ_TYPE_XFORM, self.cur_val);
                self.new_whatsit(PDF_REFXFORM_NODE, PDF_REFXFORM_NODE_SIZE);
                self.set_pdf_xform_objnum(self.tail(), self.cur_val);
                self.set_pdf_width(self.tail(), self.obj_xform_width(self.cur_val));
                self.set_pdf_height(self.tail(), self.obj_xform_height(self.cur_val));
                self.set_pdf_depth(self.tail(), self.obj_xform_depth(self.cur_val));
            }
            PDF_REFXIMAGE_NODE => {
                // §1554 Implement \pdfrefximage
                self.check_pdfoutput(b"\\pdfrefximage", true);
                self.scan_int();
                self.pdf_check_obj(OBJ_TYPE_XIMAGE, self.cur_val);
                self.new_whatsit(PDF_REFXIMAGE_NODE, PDF_REFXIMAGE_NODE_SIZE);
                self.set_pdf_ximage_objnum(self.tail(), self.cur_val);
                self.set_pdf_width(self.tail(), self.obj_ximage_width(self.cur_val));
                self.set_pdf_height(self.tail(), self.obj_ximage_height(self.cur_val));
                self.set_pdf_depth(self.tail(), self.obj_ximage_depth(self.cur_val));
            }
            PDF_SAVE_POS_NODE => {
                // §1576 Implement \pdfsavepos
                self.new_whatsit(PDF_SAVE_POS_NODE, SMALL_NODE_SIZE);
            }
            PDF_SNAP_REF_POINT_NODE => {
                // §1572 Implement \pdfsnaprefpoint
                self.check_pdfoutput(b"\\pdfsnaprefpoint", true);
                self.new_whatsit(PDF_SNAP_REF_POINT_NODE, SMALL_NODE_SIZE);
            }
            PDF_SNAPY_COMP_NODE => {
                // §1575 Implement \pdfsnapycomp
                self.check_pdfoutput(b"\\pdfsnapycomp", true);
                self.new_whatsit(PDF_SNAPY_COMP_NODE, SMALL_NODE_SIZE);
                self.scan_int();
                let v = self.fix_int(self.cur_val, 0, 1000);
                self.set_snapy_comp_ratio(self.tail(), v);
            }
            PDF_SNAPY_NODE => {
                // §1574 Implement \pdfsnapy
                self.check_pdfoutput(b"\\pdfsnapy", true);
                let n = self.new_snap_node(PDF_SNAPY_NODE);
                self.tail_append(n);
            }
            PDF_START_LINK_NODE => {
                // §1560 Implement \pdfstartlink
                self.check_pdfoutput(b"\\pdfstartlink", true);
                if abs(self.mode()) == VMODE {
                    self.pdf_error(b"ext1", b"\\pdfstartlink cannot be used in vertical mode");
                }
                k = self.pdf_new_objnum();
                self.new_annot_whatsit(PDF_START_LINK_NODE, PDF_ANNOT_NODE_SIZE);
                let a = self.scan_action();
                self.set_pdf_link_action(self.tail(), a);
                self.set_pdf_link_objnum(self.tail(), k);
                self.pdf_last_link = k;
            }
            PDF_START_THREAD_NODE => {
                // §1568 Implement \pdfstartthread
                self.check_pdfoutput(b"\\pdfstartthread", true);
                self.new_annot_whatsit(PDF_START_THREAD_NODE, PDF_THREAD_NODE_SIZE);
                self.scan_thread_id();
            }
            PDF_THREAD_NODE => {
                // §1567 Implement \pdfthread
                self.check_pdfoutput(b"\\pdfthread", true);
                self.new_annot_whatsit(PDF_THREAD_NODE, PDF_THREAD_NODE_SIZE);
                self.scan_thread_id();
            }
            PDF_TRAILER_CODE => {
                // §1581 Implement \pdftrailer
                self.check_pdfoutput(b"\\pdftrailer", false);
                self.scan_pdf_ext_toks();
                if self.pdf_output() > 0 {
                    self.pdf_trailer_toks = self.concat_tokens(self.pdf_trailer_toks, self.def_ref);
                }
            }
            PDF_TRAILER_ID_CODE => {
                // §1582 Implement \pdftrailerid
                self.check_pdfoutput(b"\\pdftrailerid", false);
                self.scan_pdf_ext_toks();
                if self.pdf_output() > 0 {
                    self.pdf_trailer_id_toks = self.concat_tokens(self.pdf_trailer_id_toks, self.def_ref);
                }
            }
            PDF_XFORM_CODE => {
                // §1548 Implement \pdfxform
                self.check_pdfoutput(b"\\pdfxform", true);
                self.pdf_xform_count += 1;
                self.pdf_create_obj(OBJ_TYPE_XFORM, self.pdf_xform_count);
                k = self.obj_ptr;
                let m = self.pdf_get_mem(PDFMEM_XFORM_SIZE);
                self.set_obj_data_ptr(k, m);
                if self.scan_keyword(b"attr") {
                    self.scan_pdf_ext_toks();
                    self.set_obj_xform_attr(k, self.def_ref);
                } else {
                    self.set_obj_xform_attr(k, NULL);
                }
                if self.scan_keyword(b"resources") {
                    self.scan_pdf_ext_toks();
                    self.set_obj_xform_resources(k, self.def_ref);
                } else {
                    self.set_obj_xform_resources(k, NULL);
                }
                self.scan_register_num();
                p = self.fetch_box();
                if p == NULL {
                    self.pdf_error(b"ext1", b"\\pdfxform cannot be used with a void box");
                }
                self.set_obj_xform_width(k, self.width(p));
                self.set_obj_xform_height(k, self.height(p));
                self.set_obj_xform_depth(k, self.depth(p));
                self.set_obj_xform_box(k, p); // save pointer to the box
                self.change_box(NULL);
                self.pdf_last_xform = k;
            }
            PDF_XIMAGE_CODE => {
                // §1553 Implement \pdfximage
                self.check_pdfoutput(b"\\pdfximage", true);
                self.check_pdfversion();
                self.scan_image();
            }
            RESET_TIMER_CODE => {
                // §1586 Implement \pdfresettimer
                self.seconds_and_micros();
            }
            SET_RANDOM_SEED_CODE => {
                // §1585 Implement \pdfsetrandomseed
                self.v_image.ck.seed_pinned = true;
                self.scan_int();
                if self.cur_val < 0 {
                    self.cur_val = -self.cur_val;
                }
                self.random_seed = self.cur_val;
                self.init_randoms(self.random_seed);
            }
            PDF_GLYPH_TO_UNICODE_CODE => {
                // §1592 Implement \pdfglyphtounicode
                self.glyph_to_unicode();
            }
            PDF_NOBUILTIN_TOUNICODE_CODE => {
                // §1593 Implement \pdfnobuiltintounicode
                self.check_pdfoutput(b"\\pdfnobuiltintounicode", true);
                self.scan_font_ident();
                k = self.cur_val;
                if k == NULL_FONT {
                    self.pdf_error(b"font", b"invalid font identifier");
                }
                self.pdf_font_nobuiltin_tounicode[k] = true;
            }
            PDF_INTERWORD_SPACE_ON_NODE => {
                // §1594 Implement \pdfinterwordspaceon
                self.check_pdfoutput(b"\\pdfinterwordspaceon", true);
                self.new_whatsit(PDF_INTERWORD_SPACE_ON_NODE, SMALL_NODE_SIZE);
            }
            PDF_INTERWORD_SPACE_OFF_NODE => {
                // §1595 Implement \pdfinterwordspaceoff
                self.check_pdfoutput(b"\\pdfinterwordspaceoff", true);
                self.new_whatsit(PDF_INTERWORD_SPACE_OFF_NODE, SMALL_NODE_SIZE);
            }
            PDF_FAKE_SPACE_NODE => {
                // §1596 Implement \pdffakespace
                self.check_pdfoutput(b"\\pdffakespace", true);
                self.new_whatsit(PDF_FAKE_SPACE_NODE, SMALL_NODE_SIZE);
            }
            PDF_RUNNING_LINK_OFF_NODE => {
                // §1597 Implement \pdfrunninglinkoff
                self.check_pdfoutput(b"\\pdfrunninglinkoff", true);
                self.new_whatsit(PDF_RUNNING_LINK_OFF_NODE, SMALL_NODE_SIZE);
            }
            PDF_RUNNING_LINK_ON_NODE => {
                // §1598 Implement \pdfrunninglinkon
                self.check_pdfoutput(b"\\pdfrunninglinkon", true);
                self.new_whatsit(PDF_RUNNING_LINK_ON_NODE, SMALL_NODE_SIZE);
            }
            PDF_SPACE_FONT_CODE => {
                // §1599 Implement \pdfspacefont
                self.check_pdfoutput(b"\\pdfspacefont", true);
                self.scan_pdf_ext_toks();
                self.pdf_space_font_name = self.tokens_to_string(self.def_ref);
                self.delete_token_ref(self.def_ref);
            }
            _ => self.confusion(b"ext1"),
        }
    }

    /// §469 Scan an optional space
    fn ext_scan_optional_space(&mut self) {
        self.get_x_token();
        if self.cur_cmd != SPACER {
            self.back_input();
        }
    }

    /// §1529
    pub fn new_whatsit(&mut self, s: i32, w: i32) {
        let p = self.get_node(w);
        self.set_ty(p, WHATSIT_NODE);
        self.set_subtype(p, s);
        self.set_link(self.tail(), p);
        self.set_tail(p);
    }

    /// §1530
    pub fn new_write_whatsit(&mut self, w: i32) {
        self.new_whatsit(self.cur_chr, w);
        if w != WRITE_NODE_SIZE {
            self.scan_four_bit_int();
        } else {
            self.scan_int();
            if self.cur_val < 0 {
                self.cur_val = 17;
            } else if self.cur_val > 15 && self.cur_val != 18 {
                self.cur_val = 16;
            }
        }
        self.set_write_stream(self.tail(), self.cur_val);
    }

    /// §1537
    pub fn check_pdfoutput(&mut self, s: &[u8], is_error: bool) {
        if self.pdf_output() <= 0 {
            if is_error {
                self.pdf_error(s, b"not allowed in DVI mode (\\pdfoutput <= 0)");
            } else {
                self.pdf_warning(s, b"not allowed in DVI mode (\\pdfoutput <= 0); ignoring it", true, true);
            }
        }
    }

    /// §1537
    pub fn scan_pdf_ext_toks(&mut self) {
        self.scan_toks(false, true); // like \special
    }

    /// §1537
    pub fn scan_pdf_ext_late_toks(&mut self) {
        self.scan_toks(false, false); // like \special, but doesn't expand
    }

    /// §1537 to implement \pdfstrcmp
    pub fn compare_strings(&mut self) {
        let save_cur_cs = self.cur_cs;
        self.scan_toks(false, true);
        let s1 = self.tokens_to_string(self.def_ref);
        self.delete_token_ref(self.def_ref);
        self.cur_cs = save_cur_cs;
        self.scan_toks(false, true);
        let s2 = self.tokens_to_string(self.def_ref);
        self.delete_token_ref(self.def_ref);
        let mut i1 = self.str_start[s1];
        let j1 = self.str_start[s1 + 1];
        let mut i2 = self.str_start[s2];
        let j2 = self.str_start[s2 + 1];
        'done: {
            while i1 < j1 && i2 < j2 {
                if self.str_pool.at(i1) < self.str_pool.at(i2) {
                    self.cur_val = -1;
                    break 'done;
                }
                if self.str_pool.at(i1) > self.str_pool.at(i2) {
                    self.cur_val = 1;
                    break 'done;
                }
                i1 += 1;
                i2 += 1;
            }
            if i1 == j1 && i2 == j2 {
                self.cur_val = 0;
            } else if i1 < j1 {
                self.cur_val = 1;
            } else {
                self.cur_val = -1;
            }
        }
        self.flush_str(s2);
        self.flush_str(s1);
        self.cur_val_level = INT_VAL;
    }

    /// §1545 finds the node preceding the rightmost node `e`; `s` is some node before `e`
    pub fn prev_rightmost(&mut self, s: i32, e: i32) -> i32 {
        let mut p = s;
        if p == NULL {
            return NULL;
        }
        while self.link(p) != e {
            p = self.link(p);
            if p == NULL {
                return NULL;
            }
        }
        p
    }

    /// §1545
    pub fn pdf_check_obj(&mut self, t: i32, n: i32) {
        let mut k = self.head_tab[t];
        while k != 0 && k != n {
            k = self.obj_link(k);
        }
        if k == 0 {
            self.pdf_error(b"ext1", b"cannot find referenced object");
        }
    }

    /// §1552.  `x` and `y` are 64 bits, as the oracle's `integer`: docs/PORTING.md section 11.
    pub fn scale_image(&mut self, n: i32) {
        let image = self.obj_ximage_data(n);
        let e = self.ext_img(image);
        let (x, y, mut xr, mut yr);
        if e.rotate == 90 || e.rotate == 270 {
            y = e.width;
            x = e.height;
            yr = e.x_res;
            xr = e.y_res;
        } else {
            x = e.width;
            y = e.height;
            xr = e.x_res;
            yr = e.y_res;
        }
        if xr > 65535 || yr > 65535 {
            xr = 0;
            yr = 0;
            self.pdf_warning(b"ext1", b"too large image resolution ignored", true, true);
        }
        if x <= 0 || y <= 0 || xr < 0 || yr < 0 {
            self.pdf_error(b"ext1", b"invalid image dimensions");
        }
        if xr == 0 && yr == 0 {
            // If both resolutions are zero, that suggests that the resolution data is missing, so we should never
            // issue a warning.
        } else if x as f64 / self.one_inch as f64 >= xr as f64 || y as f64 / self.one_inch as f64 >= yr as f64 {
            // It's better to warn and ignore too-small resolutions given in the image file, usually 1dpi x 1dpi, to
            // avoid arithmetic overflow.
            xr = 0;
            yr = 0;
            self.pdf_warning(b"ext1", b"too small image resolution ignored", true, true);
        }
        let is_pdf = self.is_pdf_image(image);
        let default_res = if is_pdf { 0 } else { self.fix_int(self.pdf_image_resolution(), 0, 65535) };
        self.ext_scale_image_dims(n, x, y, xr, yr, is_pdf, default_res);
    }

    /// §1552, its second half: all the arithmetic, on the resolutions the first half has checked.
    fn ext_scale_image_dims(&mut self, n: i32, x: i64, y: i64, xr: i32, yr: i32, is_pdf: bool, default_res: i32) {
        let (mut xr, mut yr) = (xr, yr);
        let (mut w, mut h): (i64, i64) = (0, 0);
        if is_pdf {
            w = x;
            h = y;
        } else {
            if default_res > 0 && (xr == 0 || yr == 0) {
                xr = default_res;
                yr = default_res;
            }
            if self.is_running(self.obj_ximage_width(n)) && self.is_running(self.obj_ximage_height(n)) {
                let hundred_inch = self.one_hundred_inch as i64;
                if xr > 0 && yr > 0 {
                    w = self.ext_xn_over_d_wide(hundred_inch, x, 100 * xr as i64);
                    h = self.ext_xn_over_d_wide(hundred_inch, y, 100 * yr as i64);
                } else {
                    w = self.ext_xn_over_d_wide(hundred_inch, x, 7200);
                    h = self.ext_xn_over_d_wide(hundred_inch, y, 7200);
                }
            }
        }
        let (run_w, run_h, run_d) = (
            self.is_running(self.obj_ximage_width(n)),
            self.is_running(self.obj_ximage_height(n)),
            self.is_running(self.obj_ximage_depth(n)),
        );
        let (ht, dp) = (self.obj_ximage_height(n) as i64, self.obj_ximage_depth(n) as i64);
        let wd = self.obj_ximage_width(n) as i64;
        if run_w && run_h && run_d {
            self.set_obj_ximage_width(n, sat32(w));
            self.set_obj_ximage_height(n, sat32(h));
            self.set_obj_ximage_depth(n, 0);
        } else if run_w {
            // image depth or height is explicitly specified
            if run_h {
                // image depth is explicitly specified
                let v = self.ext_xn_over_d_wide(h, x, y);
                self.set_obj_ximage_width(n, sat32(v));
                self.set_obj_ximage_height(n, sat32(h - dp));
            } else if run_d {
                // image height is explicitly specified
                let v = self.ext_xn_over_d_wide(ht, x, y);
                self.set_obj_ximage_width(n, sat32(v));
                self.set_obj_ximage_depth(n, 0);
            } else {
                // both image depth and height are explicitly specified
                let v = self.ext_xn_over_d_wide(ht + dp, x, y);
                self.set_obj_ximage_width(n, sat32(v));
            }
        } else {
            // image width is explicitly specified
            if run_h && run_d {
                // both image depth and height are not specified
                let v = self.ext_xn_over_d_wide(wd, y, x);
                self.set_obj_ximage_height(n, sat32(v));
                self.set_obj_ximage_depth(n, 0);
            } else if run_h {
                // image depth is explicitly specified
                let v = self.ext_xn_over_d_wide(wd, y, x);
                self.set_obj_ximage_height(n, sat32(v - dp));
            } else if run_d {
                // image height is explicitly specified
                self.set_obj_ximage_depth(n, 0);
            }
            // both image depth and height are explicitly specified: nothing to do
        }
    }

    /// utils.c `extxnoverd`, for §1552: as `ext_xn_over_d`, on and to 64 bits.
    fn ext_xn_over_d_wide(&mut self, x: i64, n: i64, d: i64) -> i64 {
        let mut r = x as f64 * n as f64 / d as f64;
        if r > f64::EPSILON {
            r += 0.5;
        } else {
            r -= 0.5;
        }
        if r >= MAX_INTEGER as f64 || r <= -(MAX_INTEGER as f64) {
            self.ext_pdftex_warn(None, b"arithmetic: number too big");
        }
        r as i64
    }

    /// §1552 scans PDF pagebox specification
    pub fn scan_pdf_box_spec(&mut self) -> i32 {
        if self.scan_keyword(b"mediabox") {
            self.pdf_box_spec_media
        } else if self.scan_keyword(b"cropbox") {
            self.pdf_box_spec_crop
        } else if self.scan_keyword(b"bleedbox") {
            self.pdf_box_spec_bleed
        } else if self.scan_keyword(b"trimbox") {
            self.pdf_box_spec_trim
        } else if self.scan_keyword(b"artbox") {
            self.pdf_box_spec_art
        } else {
            0
        }
    }

    /// §1552 scans rule spec to alt_rule
    pub fn scan_alt_rule(&mut self) {
        if self.alt_rule == NULL {
            self.alt_rule = self.new_rule();
        }
        self.set_width(self.alt_rule, NULL_FLAG);
        self.set_height(self.alt_rule, NULL_FLAG);
        self.set_depth(self.alt_rule, NULL_FLAG);
        loop {
            if self.scan_keyword(b"width") {
                self.scan_normal_dimen();
                self.set_width(self.alt_rule, self.cur_val);
                continue;
            }
            if self.scan_keyword(b"height") {
                self.scan_normal_dimen();
                self.set_height(self.alt_rule, self.cur_val);
                continue;
            }
            if self.scan_keyword(b"depth") {
                self.scan_normal_dimen();
                self.set_depth(self.alt_rule, self.cur_val);
                continue;
            }
            break;
        }
    }

    /// §1552
    pub fn scan_image(&mut self) {
        self.pdf_ximage_count += 1;
        self.pdf_create_obj(OBJ_TYPE_XIMAGE, self.pdf_ximage_count);
        let k = self.obj_ptr;
        let m = self.pdf_get_mem(PDFMEM_XIMAGE_SIZE);
        self.set_obj_data_ptr(k, m);
        self.scan_alt_rule(); // scans <rule spec> to alt_rule
        self.set_obj_ximage_width(k, self.width(self.alt_rule));
        self.set_obj_ximage_height(k, self.height(self.alt_rule));
        self.set_obj_ximage_depth(k, self.depth(self.alt_rule));
        if self.scan_keyword(b"attr") {
            self.scan_pdf_ext_toks();
            self.set_obj_ximage_attr(k, self.def_ref);
        } else {
            self.set_obj_ximage_attr(k, NULL);
        }
        let mut named = 0;
        // Pascal leaves `page` unset after `named`.  The C's `readimage` then reads it only for a JBIG2 file.
        let mut page = 0;
        if self.scan_keyword(b"named") {
            self.scan_pdf_ext_toks();
            named = self.tokens_to_string(self.def_ref);
            self.delete_token_ref(self.def_ref);
        } else if self.scan_keyword(b"page") {
            self.scan_int();
            page = self.cur_val;
        } else {
            page = 1;
        }
        let colorspace;
        if self.scan_keyword(b"colorspace") {
            self.scan_int();
            colorspace = self.cur_val;
        } else {
            colorspace = 0;
        }
        let mut pagebox = self.scan_pdf_box_spec();
        if pagebox == 0 {
            pagebox = self.pdf_pagebox();
        }
        self.scan_pdf_ext_toks();
        let s = self.tokens_to_string(self.def_ref);
        self.delete_token_ref(self.def_ref);
        if self.pdf_option_always_use_pdfpagebox() != 0 {
            self.pdf_warning(
                b"PDF inclusion",
                b"Primitive \\pdfoptionalwaysusepdfpagebox is obsolete; use \\pdfpagebox instead.",
                true,
                true,
            );
            self.set_pdf_force_pagebox(self.pdf_option_always_use_pdfpagebox());
            self.set_pdf_option_always_use_pdfpagebox(0); // warn once
            self.warn_pdfpagebox = false;
        }
        if self.pdf_option_pdf_inclusion_errorlevel() != 0 {
            self.pdf_warning(
                b"PDF inclusion",
                b"Primitive \\pdfoptionpdfinclusionerrorlevel is obsolete; use \\pdfinclusionerrorlevel instead.",
                true,
                true,
            );
            self.set_pdf_inclusion_errorlevel(self.pdf_option_pdf_inclusion_errorlevel());
            self.set_pdf_option_pdf_inclusion_errorlevel(0); // warn once
        }
        if self.pdf_force_pagebox() > 0 {
            if self.warn_pdfpagebox {
                self.pdf_warning(
                    b"PDF inclusion",
                    b"Primitive \\pdfforcepagebox is obsolete; use \\pdfpagebox instead.",
                    true,
                    true,
                );
                self.warn_pdfpagebox = false;
            }
            pagebox = self.pdf_force_pagebox();
        }
        if pagebox == 0 {
            // no pagebox specification given
            pagebox = self.pdf_box_spec_crop;
        }
        let d = self.read_image(
            s,
            page,
            named,
            colorspace,
            pagebox,
            self.pdf_major_version(),
            self.pdf_minor_version(),
            self.pdf_inclusion_errorlevel(),
        );
        self.set_obj_ximage_data(k, d);
        if named != 0 {
            self.flush_str(named);
        }
        self.flush_str(s);
        self.scale_image(k);
        self.pdf_last_ximage = k;
        self.pdf_last_ximage_pages = self.image_pages(self.obj_ximage_data(k));
        self.pdf_last_ximage_colordepth = self.image_colordepth(self.obj_ximage_data(k));
    }

    /// §1555
    pub fn find_obj(&mut self, t: i32, i: i32, byname: bool) -> i32 {
        self.avl_find_obj(t, i, byname as i32)
    }

    /// §1555 flush a string if possible
    pub fn flush_str(&mut self, s: i32) {
        if self.flushable(s) {
            self.flush_string();
        }
    }

    /// §1555
    pub fn get_obj(&mut self, t: i32, i: i32, byname: bool) -> i32 {
        let mut s;
        let mut r;
        if byname {
            s = self.tokens_to_string(i);
            r = self.find_obj(t, s, true);
        } else {
            s = 0;
            r = self.find_obj(t, i, false);
        }
        if r == 0 {
            if byname {
                self.pdf_create_obj(t, -s);
                s = 0;
            } else {
                self.pdf_create_obj(t, i);
            }
            r = self.obj_ptr;
            if t == OBJ_TYPE_DEST || t == OBJ_TYPE_STRUCT_DEST {
                self.set_obj_dest_ptr(r, NULL);
            }
        }
        if s != 0 {
            self.flush_str(s);
        }
        r
    }

    /// §1555.  `/` is real division, as the oracle's code has it.  A read of the clock: hard taint, DESIGN section 7.
    pub fn get_microinterval(&mut self) -> i32 {
        self.host.world.mark_taint_because(Taint::Hard, "\\pdfelapsedtime was read");
        let (s, m) = self.host.world.clock();
        // `epochseconds` is an `i32` here and 64 bits in the oracle: only the difference counts.
        let elapsed = (s as i32).wrapping_sub(self.epochseconds) as i64;
        let micro = self.microseconds;
        if elapsed > 32767 {
            MAX_INTEGER
        } else if micro > m {
            (((elapsed - 1) * 65536) as f64 + ((m + 1000000 - micro) as f64 / 100.0 * 65536.0) / 10000.0) as i32
        } else {
            ((elapsed * 65536) as f64 + ((m - micro) as f64 / 100.0 * 65536.0) / 10000.0) as i32
        }
    }

    /// §1556 read an action specification
    pub fn scan_action(&mut self) -> i32 {
        let p = self.get_node(PDF_ACTION_SIZE);
        self.set_pdf_action_file(p, NULL);
        self.set_pdf_action_refcount(p, NULL);
        if self.scan_keyword(b"user") {
            self.set_pdf_action_type(p, PDF_ACTION_USER);
        } else if self.scan_keyword(b"goto") {
            self.set_pdf_action_type(p, PDF_ACTION_GOTO);
        } else if self.scan_keyword(b"thread") {
            self.set_pdf_action_type(p, PDF_ACTION_THREAD);
        } else {
            self.pdf_error(b"ext1", b"action type missing");
        }
        if self.pdf_action_type(p) == PDF_ACTION_USER {
            self.scan_pdf_ext_toks();
            self.set_pdf_action_user_tokens(p, self.def_ref);
            return p;
        }
        self.set_pdf_action_named_id(p, 0);
        if self.scan_keyword(b"file") {
            self.scan_pdf_ext_toks();
            self.set_pdf_action_file(p, self.def_ref);
        }
        if self.scan_keyword(b"struct") {
            if self.pdf_action_type(p) != PDF_ACTION_GOTO {
                self.pdf_error(b"ext1", b"only GoTo action can be used with `struct'");
            }
            if self.pdf_action_file(p) != NULL {
                self.scan_pdf_ext_toks();
                self.set_pdf_action_named_id(p, self.pdf_action_named_id(p) + 2);
                self.set_pdf_action_struct_id(p, self.def_ref);
            } else if self.scan_keyword(b"name") {
                self.scan_pdf_ext_toks();
                self.set_pdf_action_named_id(p, self.pdf_action_named_id(p) + 2);
                self.set_pdf_action_struct_id(p, self.def_ref);
            } else if self.scan_keyword(b"num") {
                self.scan_int();
                // So a number here is never NULL, which is 0 here and negative in the oracle.
                if self.cur_val <= 0 {
                    self.pdf_error(b"ext1", b"num identifier must be positive");
                }
                self.set_pdf_action_struct_id(p, self.cur_val);
            } else {
                self.pdf_error(b"ext1", b"identifier type missing");
            }
        } else {
            self.set_pdf_action_struct_id(p, NULL);
        }
        if self.scan_keyword(b"page") {
            if self.pdf_action_type(p) != PDF_ACTION_GOTO {
                self.pdf_error(b"ext1", b"only GoTo action can be used with `page'");
            }
            self.set_pdf_action_type(p, PDF_ACTION_PAGE);
            self.scan_int();
            if self.cur_val <= 0 {
                self.pdf_error(b"ext1", b"page number must be positive");
            }
            self.set_pdf_action_id(p, self.cur_val);
            self.scan_pdf_ext_toks();
            self.set_pdf_action_page_tokens(p, self.def_ref);
        } else if self.scan_keyword(b"name") {
            self.scan_pdf_ext_toks();
            self.set_pdf_action_named_id(p, self.pdf_action_named_id(p) + 1);
            self.set_pdf_action_id(p, self.def_ref);
        } else if self.scan_keyword(b"num") {
            if self.pdf_action_type(p) == PDF_ACTION_GOTO && self.pdf_action_file(p) != NULL {
                self.pdf_error(b"ext1", b"`goto' option cannot be used with both `file' and `num'");
            }
            self.scan_int();
            if self.cur_val <= 0 {
                self.pdf_error(b"ext1", b"num identifier must be positive");
            }
            self.set_pdf_action_id(p, self.cur_val);
        } else {
            self.pdf_error(b"ext1", b"identifier type missing");
        }
        if self.scan_keyword(b"newwindow") {
            self.set_pdf_action_new_window(p, 1);
            self.ext_scan_optional_space();
        } else if self.scan_keyword(b"nonewwindow") {
            self.set_pdf_action_new_window(p, 2);
            self.ext_scan_optional_space();
        } else {
            self.set_pdf_action_new_window(p, 0);
        }
        if self.pdf_action_new_window(p) > 0
            && ((self.pdf_action_type(p) != PDF_ACTION_GOTO && self.pdf_action_type(p) != PDF_ACTION_PAGE)
                || self.pdf_action_file(p) == NULL)
        {
            self.pdf_error(b"ext1", b"`newwindow'/`nonewwindow' must be used with `goto' and `file' option");
        }
        p
    }

    /// §1556 create a new whatsit node for annotation
    pub fn new_annot_whatsit(&mut self, w: i32, s: i32) {
        self.new_whatsit(w, s);
        self.scan_alt_rule(); // scans <rule spec> to alt_rule
        self.set_pdf_width(self.tail(), self.width(self.alt_rule));
        self.set_pdf_height(self.tail(), self.height(self.alt_rule));
        self.set_pdf_depth(self.tail(), self.depth(self.alt_rule));
        if w == PDF_START_LINK_NODE {
            if self.scan_keyword(b"attr") {
                self.scan_pdf_ext_toks();
                self.set_pdf_link_attr(self.tail(), self.def_ref);
            } else {
                self.set_pdf_link_attr(self.tail(), NULL);
            }
        }
        if w == PDF_THREAD_NODE || w == PDF_START_THREAD_NODE {
            if self.scan_keyword(b"attr") {
                self.scan_pdf_ext_toks();
                self.set_pdf_thread_attr(self.tail(), self.def_ref);
            } else {
                self.set_pdf_thread_attr(self.tail(), NULL);
            }
        }
    }

    /// §1562 return number of outline entries in the same level with `p`
    pub fn outline_list_count(&mut self, p: i32) -> i32 {
        let mut p = p;
        let mut k = 1;
        while self.obj_outline_prev(p) != 0 {
            k += 1;
            p = self.obj_outline_prev(p);
        }
        k
    }

    /// §1564
    pub fn warn_dest_dup(&mut self, id: i32, byname: i32, s1: &[u8], s2: &[u8]) {
        if self.pdf_suppress_warning_dup_dest() > 0 {
            return;
        }
        self.pdf_warning(s1, b"destination with the same identifier (", true, false);
        if byname > 0 {
            self.print_s(b"name");
            self.print_mark(id);
        } else {
            self.print_s(b"num");
            self.print_int(id);
        }
        self.print_s(b") ");
        self.print_s(s2);
        self.print_ln();
        self.show_context();
    }

    /// §1566
    pub fn scan_thread_id(&mut self) {
        if self.scan_keyword(b"num") {
            self.scan_int();
            if self.cur_val <= 0 {
                self.pdf_error(b"ext1", b"num identifier must be positive");
            }
            if self.cur_val > MAX_HALFWORD {
                self.pdf_error(b"ext1", b"number too big");
            }
            self.set_pdf_thread_id(self.tail(), self.cur_val);
            self.set_pdf_thread_named_id(self.tail(), 0);
        } else if self.scan_keyword(b"name") {
            self.scan_pdf_ext_toks();
            self.set_pdf_thread_id(self.tail(), self.def_ref);
            self.set_pdf_thread_named_id(self.tail(), 1);
        } else {
            self.pdf_error(b"ext1", b"identifier type missing");
        }
    }

    /// §1573
    pub fn new_snap_node(&mut self, s: i32) -> i32 {
        self.scan_glue(GLUE_VAL);
        if self.width(self.cur_val) < 0 {
            self.pdf_error(b"ext1", b"negative snap glue");
        }
        let p = self.get_node(SNAP_NODE_SIZE);
        self.set_ty(p, WHATSIT_NODE);
        self.set_subtype(p, s);
        self.set_link(p, NULL);
        self.set_snap_glue_ptr(p, self.cur_val);
        self.set_final_skip(p, 0);
        p
    }

    /// §1577 concat `q` and `r` and returns the result tokens list
    pub fn concat_tokens(&mut self, q: i32, r: i32) -> i32 {
        self.tok_concat(q, r)
    }

    /// §1587
    pub fn pdf_include_chars(&mut self) {
        self.scan_font_ident();
        let f = self.cur_val;
        if f == NULL_FONT {
            self.pdf_error(b"font", b"invalid font identifier");
        }
        self.pdf_check_vf_cur_val();
        if !self.font_used[f] {
            self.pdf_init_font(f);
        }
        self.scan_pdf_ext_toks();
        let s = self.tokens_to_string(self.def_ref);
        self.delete_token_ref(self.def_ref);
        let mut k = self.str_start[s];
        while k < self.str_start[s + 1] {
            self.pdf_mark_char(f, self.str_pool.at(k));
            k += 1;
        }
        self.flush_str(s);
    }

    /// §1587
    pub fn glyph_to_unicode(&mut self) {
        self.scan_pdf_ext_toks();
        let s1 = self.tokens_to_string(self.def_ref);
        self.delete_token_ref(self.def_ref);
        self.scan_pdf_ext_toks();
        let s2 = self.tokens_to_string(self.def_ref);
        self.delete_token_ref(self.def_ref);
        self.def_tounicode(s1, s2);
        self.flush_str(s2);
        self.flush_str(s1);
    }

    /// §1600
    pub fn thread_title(&mut self, thread: i32) {
        self.pdf_print_s(b"/Title (");
        if self.obj_info(thread) < 0 {
            self.pdf_print(-self.obj_info(thread));
        } else {
            self.pdf_print_int(self.obj_info(thread));
        }
        self.pdf_print_ln_s(b")");
    }

    /// §1600
    pub fn pdf_fix_thread(&mut self, thread: i32) {
        self.pdf_warning(b"thread", b"destination ", true, false);
        if self.obj_info(thread) < 0 {
            self.print_s(b"name{");
            self.print(-self.obj_info(thread));
            self.print(b'}' as i32);
        } else {
            self.print_s(b"num");
            self.print_int(self.obj_info(thread));
        }
        self.print_s(b" has been referenced but does not exist, replaced by a fixed one");
        self.print_ln();
        self.print_ln();
        self.pdf_new_dict(OBJ_TYPE_OTHERS, 0, 0);
        let a = self.obj_ptr;
        self.pdf_indirect_ln(b"T", thread);
        self.pdf_indirect_ln(b"V", a);
        self.pdf_indirect_ln(b"N", a);
        self.pdf_indirect_ln(b"P", self.head_tab[OBJ_TYPE_PAGE]);
        self.pdf_print_s(b"/R [0 0 ");
        self.pdf_print_bp(self.pdf_page_width());
        self.pdf_out(b' ' as i32);
        self.pdf_print_bp(self.pdf_page_height());
        self.pdf_print_ln_s(b"]");
        self.pdf_end_dict();
        self.pdf_begin_dict(thread, 1);
        self.pdf_print_ln_s(b"/I << ");
        self.thread_title(thread);
        self.pdf_print_ln_s(b">>");
        self.pdf_indirect_ln(b"F", a);
        self.pdf_end_dict();
    }

    /// §1600
    pub fn out_thread(&mut self, thread: i32) {
        if self.obj_thread_first(thread) == 0 {
            self.pdf_fix_thread(thread);
            return;
        }
        self.pdf_begin_dict(thread, 1);
        let mut a = self.obj_thread_first(thread);
        let b = a;
        let mut last_attr = 0;
        loop {
            if self.obj_bead_attr(a) != 0 {
                last_attr = self.obj_bead_attr(a);
            }
            a = self.obj_bead_next(a);
            if a == b {
                break;
            }
        }
        if last_attr != 0 {
            self.pdf_print_ln(last_attr);
        } else {
            self.pdf_print_ln_s(b"/I << ");
            self.thread_title(thread);
            self.pdf_print_ln_s(b">>");
        }
        self.pdf_indirect_ln(b"F", a);
        self.pdf_end_dict();
        loop {
            self.pdf_begin_dict(a, 1);
            if a == b {
                self.pdf_indirect_ln(b"T", thread);
            }
            self.pdf_indirect_ln(b"V", self.obj_bead_prev(a));
            self.pdf_indirect_ln(b"N", self.obj_bead_next(a));
            self.pdf_indirect_ln(b"P", self.obj_bead_page(a));
            self.pdf_indirect_ln(b"R", self.obj_bead_rect(a));
            self.pdf_end_dict();
            a = self.obj_bead_next(a);
            if a == b {
                break;
            }
        }
    }

    /// §1602
    pub fn print_write_whatsit(&mut self, s: &[u8], p: i32) {
        self.print_esc_s(s);
        if self.write_stream(p) < 16 {
            self.print_int(self.write_stream(p));
        } else if self.write_stream(p) == 16 {
            self.print_char(b'*' as i32);
        } else {
            self.print_char(b'-' as i32);
        }
    }

    /// §1615
    pub fn special_out(&mut self, p: i32) {
        self.synch_h();
        self.synch_v();
        let old_setting = self.selector;
        let h;
        if self.subtype(p) == LATESPECIAL_NODE {
            self.ext_expand_write_tokens(p);
            h = self.def_ref;
        } else {
            h = self.write_tokens(p);
        }
        self.selector = NEW_STRING;
        self.show_token_list(h + 1, NULL, self.pool_size - self.pool_ptr);
        self.selector = old_setting;
        self.str_room(1);
        if self.cur_length() < 256 {
            self.dvi_out(XXX1);
            self.dvi_out(self.cur_length());
        } else {
            self.dvi_out(XXX4);
            self.dvi_four(self.cur_length());
        }
        let mut k = self.str_start[self.str_ptr];
        let e = self.pool_ptr;
        while k < e {
            self.dvi_out(self.str_pool.at(k));
            k += 1;
        }
        self.pool_ptr = self.str_start[self.str_ptr]; // erase the string
        if self.subtype(p) == LATESPECIAL_NODE {
            self.flush_list(self.def_ref);
        }
    }

    /// §1618 Expand macros in the token list and make `def_ref` point to the result.
    fn ext_expand_write_tokens(&mut self, p: i32) {
        let q = self.tok_two(RIGHT_BRACE_TOKEN + b'}' as i32, END_WRITE_TOKEN);
        self.ins_list(q);
        self.begin_token_list(self.write_tokens(p), WRITE_TEXT);
        let q = self.tok_one(LEFT_BRACE_TOKEN + b'{' as i32);
        self.ins_list(q);
        // now we're ready to scan `{<token list>} \endwrite'
        let old_mode = self.mode();
        self.set_mode(0);
        // disable \prevdepth, \spacefactor, \lastskip, \prevgraf
        self.cur_cs = self.write_loc;
        self.scan_toks(false, true); // expand macros, etc.
        self.set_mode(old_mode);
        self.get_token();
        if self.cur_tok != END_WRITE_TOKEN {
            // §1619 Recover from an unbalanced write command
            self.print_err_s(b"Unbalanced write command");
            self.help(&[
                b"On this page there's a \\write with fewer real {'s than }'s.",
                b"I can't handle that very well; good luck.",
            ]);
            self.error();
            loop {
                self.get_token();
                if self.cur_tok == END_WRITE_TOKEN {
                    break;
                }
            }
        }
        self.end_token_list(); // conserve stack space
    }

    /// §1617
    pub fn write_out(&mut self, p: i32) {
        self.ext_expand_write_tokens(p);
        let old_setting = self.selector;
        let j = self.write_stream(p);
        if j == 18 {
            self.selector = NEW_STRING;
        } else if self.write_open[j] {
            self.selector = j;
        } else {
            // write to the terminal if file isn't open
            if j == 17 && self.selector == TERM_AND_LOG {
                self.selector = LOG_ONLY;
            }
            self.print_nl_s(b"");
        }
        self.token_show(self.def_ref);
        self.print_ln();
        self.flush_list(self.def_ref);
        if j == 18 {
            if self.tracing_online() <= 0 {
                self.selector = LOG_ONLY; // Show what we're doing in the log file.
            } else {
                self.selector = TERM_AND_LOG; // Show what we're doing.
            }
            // If the log file isn't open yet, we can only send output to the terminal.  Calling open_log_file from
            // here seems to result in bad data in the log.
            if !self.log_opened {
                self.selector = TERM_ONLY;
            }
            self.print_nl_s(b"runsystem(");
            for d in 0..self.cur_length() {
                // print gives up if passed str_ptr, so do it by hand.
                self.print(self.str_pool.at(self.str_start[self.str_ptr] + d)); // N.B.: not print_char
            }
            self.print_s(b")...");
            if self.shellenabledp != 0 {
                self.str_room(1);
                self.append_char(0); // Append a null byte to the expansion.
                let mut clobbered = false;
                let start = self.str_start[self.str_ptr];
                let e = self.cur_length();
                for d in 0..e {
                    // Convert to external character set.
                    let c = self.xchr.at(self.str_pool.at(start + d));
                    self.str_pool.put(start + d, c);
                    // minimal checking: NUL not allowed in argument string of system()
                    if self.str_pool.at(start + d) == NULL_CODE && d < e - 1 {
                        clobbered = true;
                    }
                }
                if clobbered {
                    self.print_s(b"clobbered");
                } else {
                    // We have the command.  See if we're allowed to execute it, and report in the log.  We don't check
                    // the actual exit status of the command, or do anything with the output.
                    let cmd = self.str_pool.range(start, start + e - 1).to_vec();
                    let runsystem_ret = self.runsystem(&cmd);
                    if runsystem_ret == -1 {
                        self.print_s(b"quotation error in system command");
                    } else if runsystem_ret == 0 {
                        self.print_s(b"disabled (restricted)");
                    } else if runsystem_ret == 1 {
                        self.print_s(b"executed");
                    } else if runsystem_ret == 2 {
                        self.print_s(b"executed safely (allowed)");
                    }
                }
            } else {
                self.print_s(b"disabled"); // shellenabledp false
            }
            self.print_char(b'.' as i32);
            self.print_nl_s(b"");
            self.print_ln();
            self.pool_ptr = self.str_start[self.str_ptr]; // erase the string
        }
        self.selector = old_setting;
    }

    /// §1620
    pub fn out_what(&mut self, p: i32) {
        match self.subtype(p) {
            OPEN_NODE | WRITE_NODE | CLOSE_NODE => {
                // §1622 Do some work that has been queued up for \write
                if !self.doing_leaders {
                    let j = self.write_stream(p);
                    if self.subtype(p) == WRITE_NODE {
                        self.write_out(p);
                    } else {
                        if self.write_open[j] {
                            self.a_close(write_file(j));
                            self.write_open[j] = false;
                        }
                        if self.subtype(p) == CLOSE_NODE {
                            // already closed
                        } else if j < 16 {
                            self.cur_name = self.open_name(p);
                            self.cur_area = self.open_area(p);
                            self.cur_ext = self.open_ext(p);
                            if self.cur_ext == S_EMPTY {
                                self.cur_ext = S_DOT_TEX;
                            }
                            self.pack_cur_name();
                            while !self.kpse_out_name_ok() || !self.a_open_out(write_file(j)) {
                                self.prompt_file_name(b"output file name", S_DOT_TEX);
                            }
                            self.write_open[j] = true;
                            // If on first line of input, log file is not ready yet, so don't log.
                            if self.log_opened && self.texmf_yesno(b"log_openout") {
                                let old_setting = self.selector;
                                if self.tracing_online() <= 0 {
                                    self.selector = LOG_ONLY; // Show what we're doing in the log file.
                                } else {
                                    self.selector = TERM_AND_LOG; // Show what we're doing.
                                }
                                self.print_nl_s(b"\\openout");
                                self.print_int(j);
                                self.print_s(b" = `");
                                self.print_file_name(self.cur_name, self.cur_area, self.cur_ext);
                                self.print_s(b"'.");
                                self.print_nl_s(b"");
                                self.print_ln();
                                self.selector = old_setting;
                            }
                        }
                    }
                }
            }
            SPECIAL_NODE | LATESPECIAL_NODE => self.special_out(p),
            LANGUAGE_NODE => {}
            PDF_SAVE_POS_NODE => {
                // §1621 Save current position in DVI mode.  4736286 = 1in, the funny DVI origin offset
                self.pdf_last_x_pos = self.cur_h + 4736286;
                self.pdf_last_y_pos = self.cur_page_height - self.cur_v - 4736286;
            }
            _ => {
                if PDFTEX_FIRST_EXTENSION_CODE <= self.subtype(p) && self.subtype(p) <= PDFTEX_LAST_EXTENSION_CODE {
                    self.pdf_error(b"ext4", b"pdf node ended up in DVI mode");
                } else {
                    self.confusion(b"ext4");
                }
            }
        }
    }

    /// §1624
    pub fn fix_language(&mut self) {
        let l;
        if self.language() <= 0 {
            l = 0;
        } else if self.language() > 255 {
            l = 0;
        } else {
            l = self.language();
        }
        if l != self.clang() {
            self.new_whatsit(LANGUAGE_NODE, SMALL_NODE_SIZE);
            self.set_what_lang(self.tail(), l);
            self.set_clang(l);
            let v = self.norm_min(self.left_hyphen_min());
            self.set_what_lhm(self.tail(), v);
            let v = self.norm_min(self.right_hyphen_min());
            self.set_what_rhm(self.tail(), v);
        }
    }

    /// §1630 write an action specification
    pub fn write_action(&mut self, p: i32) {
        let mut d = 0;
        if self.pdf_action_type(p) == PDF_ACTION_USER {
            self.pdf_print_toks_ln(self.pdf_action_user_tokens(p));
            return;
        }
        self.pdf_print_s(b"<< ");
        if self.pdf_action_file(p) != NULL {
            self.pdf_print_s(b"/F ");
            let s = self.tokens_to_string(self.pdf_action_file(p));
            if self.str_pool.at(self.str_start[s]) == 40 && self.str_pool.at(self.str_start[s] + self.length(s) - 1) == 41 {
                self.pdf_print(s);
            } else {
                self.pdf_print_str(s);
            }
            self.flush_str(s);
            self.pdf_print_s(b" ");
            if self.pdf_action_new_window(p) > 0 {
                self.pdf_print_s(b"/NewWindow ");
                if self.pdf_action_new_window(p) == 1 {
                    self.pdf_print_s(b"true ");
                } else {
                    self.pdf_print_s(b"false ");
                }
            }
        }
        let named_mod_2 = self.pdf_action_named_id(p) % 2 == 1;
        match self.pdf_action_type(p) {
            PDF_ACTION_PAGE => {
                if self.pdf_action_file(p) == NULL {
                    self.pdf_print_s(b"/S /GoTo /D [");
                    let n = self.get_obj(OBJ_TYPE_PAGE, self.pdf_action_id(p), false);
                    self.pdf_print_int(n);
                    self.pdf_print_s(b" 0 R");
                } else {
                    self.pdf_print_s(b"/S /GoToR /D [");
                    self.pdf_print_int(self.pdf_action_id(p) - 1);
                }
                self.pdf_out(b' ' as i32);
                let s = self.tokens_to_string(self.pdf_action_page_tokens(p));
                self.pdf_print(s);
                self.flush_str(self.last_tokens_string);
                self.pdf_out(b']' as i32);
            }
            PDF_ACTION_GOTO => {
                if self.pdf_action_file(p) == NULL {
                    self.pdf_print_s(b"/S /GoTo ");
                    d = self.get_obj(OBJ_TYPE_DEST, self.pdf_action_id(p), named_mod_2);
                } else {
                    self.pdf_print_s(b"/S /GoToR ");
                }
                if named_mod_2 {
                    let s = self.tokens_to_string(self.pdf_action_id(p));
                    self.pdf_str_entry(b"D", s);
                    self.flush_str(self.last_tokens_string);
                } else if self.pdf_action_file(p) == NULL {
                    self.pdf_indirect(b"D", d);
                } else {
                    self.pdf_error(b"ext4", b"`goto' option cannot be used with both `file' and `num'");
                }
            }
            PDF_ACTION_THREAD => {
                self.pdf_print_s(b"/S /Thread ");
                if self.pdf_action_file(p) == NULL {
                    d = self.get_obj(OBJ_TYPE_THREAD, self.pdf_action_id(p), named_mod_2);
                }
                if named_mod_2 {
                    let s = self.tokens_to_string(self.pdf_action_id(p));
                    self.pdf_str_entry(b"D", s);
                    self.flush_str(self.last_tokens_string);
                } else if self.pdf_action_file(p) == NULL {
                    self.pdf_indirect(b"D", d);
                } else {
                    self.pdf_int_entry(b"D", self.pdf_action_id(p));
                }
            }
            _ => {}
        }
        if self.pdf_action_struct_id(p) != NULL {
            self.pdf_out(b' ' as i32);
            if self.pdf_action_file(p) == NULL {
                let n = self.get_obj(
                    OBJ_TYPE_STRUCT_DEST,
                    self.pdf_action_struct_id(p),
                    (self.pdf_action_named_id(p) / 2) % 2 == 1,
                );
                self.pdf_indirect(b"SD", n);
            } else {
                self.pdf_print_s(b"/SD ");
                let s = self.tokens_to_string(self.pdf_action_struct_id(p));
                self.pdf_print(s);
                self.flush_str(self.last_tokens_string);
            }
        }
        self.pdf_print_ln_s(b" >>");
    }

    /// §1630
    pub fn set_rect_dimens(&mut self, p: i32, parent_box: i32, x: i32, y: i32, w: i32, h: i32, d: i32, margin: i32) {
        self.set_pdf_left(p, self.cur_h);
        if self.is_running(w) {
            self.set_pdf_right(p, x + self.width(parent_box));
        } else {
            self.set_pdf_right(p, self.cur_h + w);
        }
        if self.is_running(h) {
            self.set_pdf_top(p, y - self.height(parent_box));
        } else {
            self.set_pdf_top(p, self.cur_v - h);
        }
        if self.is_running(d) {
            self.set_pdf_bottom(p, y + self.depth(parent_box));
        } else {
            self.set_pdf_bottom(p, self.cur_v + d);
        }
        if self.is_shipping_page && self.matrixused() {
            self.matrixtransformrect(
                self.pdf_left(p),
                self.cur_page_height - self.pdf_bottom(p),
                self.pdf_right(p),
                self.cur_page_height - self.pdf_top(p),
            );
            let v = self.getllx();
            self.set_pdf_left(p, v);
            let v = self.getlly();
            self.set_pdf_bottom(p, self.cur_page_height - v);
            let v = self.geturx();
            self.set_pdf_right(p, v);
            let v = self.getury();
            self.set_pdf_top(p, self.cur_page_height - v);
        }
        self.set_pdf_left(p, self.pdf_left(p) - margin);
        self.set_pdf_top(p, self.pdf_top(p) - margin);
        self.set_pdf_right(p, self.pdf_right(p) + margin);
        self.set_pdf_bottom(p, self.pdf_bottom(p) + margin);
    }

    /// §1630
    pub fn do_annot(&mut self, p: i32, parent_box: i32, x: i32, y: i32) {
        if !self.is_shipping_page {
            self.pdf_error(b"ext4", b"annotations cannot be inside an XForm");
        }
        if self.doing_leaders {
            return;
        }
        if self.is_obj_scheduled(self.pdf_annot_objnum(p)) {
            let n = self.pdf_new_objnum();
            self.set_pdf_annot_objnum(p, n);
        }
        self.set_rect_dimens(p, parent_box, x, y, self.pdf_width(p), self.pdf_height(p), self.pdf_depth(p), 0);
        self.set_obj_annot_ptr(self.pdf_annot_objnum(p), p);
        self.pdf_annot_list = self.append_ptr(self.pdf_annot_list, self.pdf_annot_objnum(p));
        self.set_obj_scheduled(self.pdf_annot_objnum(p));
    }

    /// §1635
    pub fn push_link_level(&mut self, p: i32) {
        if self.pdf_link_stack_ptr >= PDF_MAX_LINK_LEVEL {
            self.overflow(b"pdf link stack size", PDF_MAX_LINK_LEVEL);
        }
        self.pdfassert(self.ty(p) == WHATSIT_NODE && self.subtype(p) == PDF_START_LINK_NODE);
        self.pdf_link_stack_ptr += 1;
        let c = self.copy_node_list(p);
        self.set_pdf_link_stack_top(PdfLinkStackRecord { nesting_level: self.cur_s, link_node: c, ref_link_node: p });
    }

    /// §1635
    pub fn pop_link_level(&mut self) {
        self.pdfassert(self.pdf_link_stack_ptr > 0);
        self.flush_node_list(self.pdf_link_stack_top().link_node);
        self.pdf_link_stack_ptr -= 1;
    }

    /// §1635
    pub fn do_link(&mut self, p: i32, parent_box: i32, x: i32, y: i32) {
        if !self.is_shipping_page {
            self.pdf_error(b"ext4", b"link annotations cannot be inside an XForm");
        }
        self.pdfassert(self.ty(parent_box) == HLIST_NODE);
        if self.is_obj_scheduled(self.pdf_link_objnum(p)) {
            let n = self.pdf_new_objnum();
            self.set_pdf_link_objnum(p, n);
        }
        self.push_link_level(p);
        self.set_rect_dimens(p, parent_box, x, y, self.pdf_width(p), self.pdf_height(p), self.pdf_depth(p), self.pdf_link_margin());
        // the reference for the pdf annot object must be set here
        self.set_obj_annot_ptr(self.pdf_link_objnum(p), p);
        self.pdf_link_list = self.append_ptr(self.pdf_link_list, self.pdf_link_objnum(p));
        self.set_obj_scheduled(self.pdf_link_objnum(p));
    }

    /// §1635
    pub fn end_link(&mut self) {
        if self.pdf_link_stack_ptr < 1 {
            self.pdf_error(b"ext4", b"pdf_link_stack empty, \\pdfendlink used without \\pdfstartlink?");
        }
        if self.pdf_link_stack_top().nesting_level != self.cur_s {
            self.pdf_warning(b"", b"\\pdfendlink ended up in different nesting level than \\pdfstartlink", true, true);
        }
        // N.B.: test for running link must be done on link_node and not ref_link_node, as ref_link_node can be set by
        // do_link or append_link already
        if self.is_running(self.pdf_width(self.pdf_link_stack_top().link_node)) {
            let p = self.pdf_link_stack_top().ref_link_node;
            if self.is_shipping_page && self.matrixused() {
                self.matrixrecalculate(self.cur_h + self.pdf_link_margin());
                let v = self.getllx();
                self.set_pdf_left(p, v - self.pdf_link_margin());
                let v = self.getury();
                self.set_pdf_top(p, self.cur_page_height - v - self.pdf_link_margin());
                let v = self.geturx();
                self.set_pdf_right(p, v + self.pdf_link_margin());
                let v = self.getlly();
                self.set_pdf_bottom(p, self.cur_page_height - v + self.pdf_link_margin());
            } else {
                self.set_pdf_right(p, self.cur_h + self.pdf_link_margin());
            }
        }
        self.pop_link_level();
    }

    /// §1636 append a new pdf annot to pdf_link_list
    pub fn append_link(&mut self, parent_box: i32, x: i32, y: i32, i: i32) {
        self.pdfassert(self.ty(parent_box) == HLIST_NODE);
        let p = self.copy_node_list(self.pdf_link_stack[i].link_node);
        self.pdf_link_stack[i].ref_link_node = p;
        self.set_info(p, MAX_HALFWORD); // mark that this node is not a whatsit node
        self.set_link(p, NULL); // this node is not linked in any list
        self.set_rect_dimens(p, parent_box, x, y, self.pdf_width(p), self.pdf_height(p), self.pdf_depth(p), self.pdf_link_margin());
        self.pdf_create_obj(OBJ_TYPE_OTHERS, 0);
        self.set_obj_annot_ptr(self.obj_ptr, p);
        self.pdf_link_list = self.append_ptr(self.pdf_link_list, self.obj_ptr);
    }

    /// §1637
    pub fn append_bead(&mut self, p: i32) {
        if !self.is_shipping_page {
            self.pdf_error(b"ext4", b"threads cannot be inside an XForm");
        }
        let t = self.get_obj(OBJ_TYPE_THREAD, self.pdf_thread_id(p), self.pdf_thread_named_id(p) > 0);
        let b = self.pdf_new_objnum();
        let m = self.pdf_get_mem(PDFMEM_BEAD_SIZE);
        self.set_obj_bead_ptr(b, m);
        self.set_obj_bead_page(b, self.pdf_last_page);
        self.set_obj_bead_data(b, p);
        if self.pdf_thread_attr(p) != NULL {
            let s = self.tokens_to_string(self.pdf_thread_attr(p));
            self.set_obj_bead_attr(b, s);
        } else {
            self.set_obj_bead_attr(b, 0);
        }
        if self.obj_thread_first(t) == 0 {
            self.set_obj_thread_first(t, b);
            self.set_obj_bead_next(b, b);
            self.set_obj_bead_prev(b, b);
        } else {
            let a = self.obj_thread_first(t);
            let c = self.obj_bead_prev(a);
            self.set_obj_bead_prev(b, c);
            self.set_obj_bead_next(b, a);
            self.set_obj_bead_prev(a, b);
            self.set_obj_bead_next(c, b);
        }
        self.pdf_bead_list = self.append_ptr(self.pdf_bead_list, b);
    }

    /// §1637
    pub fn do_thread(&mut self, p: i32, parent_box: i32, x: i32, y: i32) {
        if self.doing_leaders {
            return;
        }
        if self.subtype(p) == PDF_START_THREAD_NODE {
            self.pdf_thread_wd = self.pdf_width(p);
            self.pdf_thread_ht = self.pdf_height(p);
            self.pdf_thread_dp = self.pdf_depth(p);
            self.pdf_last_thread_id = self.pdf_thread_id(p);
            self.pdf_last_thread_named_id = self.pdf_thread_named_id(p) > 0;
            if self.pdf_last_thread_named_id {
                self.add_token_ref(self.pdf_thread_id(p));
            }
            self.pdf_thread_level = self.cur_s;
        }
        self.set_rect_dimens(p, parent_box, x, y, self.pdf_width(p), self.pdf_height(p), self.pdf_depth(p), self.pdf_thread_margin());
        self.append_bead(p);
        self.last_thread = p;
    }

    /// §1637
    pub fn append_thread(&mut self, parent_box: i32, x: i32, y: i32) {
        let p = self.get_node(PDF_THREAD_NODE_SIZE);
        self.set_info(p, MAX_HALFWORD); // this is not a whatsit node
        self.set_link(p, NULL); // this node will be destroyed separately
        self.set_pdf_width(p, self.pdf_thread_wd);
        self.set_pdf_height(p, self.pdf_thread_ht);
        self.set_pdf_depth(p, self.pdf_thread_dp);
        self.set_pdf_thread_attr(p, NULL);
        self.set_pdf_thread_id(p, self.pdf_last_thread_id);
        if self.pdf_last_thread_named_id {
            self.add_token_ref(self.pdf_thread_id(p));
            self.set_pdf_thread_named_id(p, 1);
        } else {
            self.set_pdf_thread_named_id(p, 0);
        }
        self.set_rect_dimens(p, parent_box, x, y, self.pdf_width(p), self.pdf_height(p), self.pdf_depth(p), self.pdf_thread_margin());
        self.append_bead(p);
        self.last_thread = p;
    }

    /// §1637
    pub fn end_thread(&mut self) {
        if self.pdf_thread_level != self.cur_s {
            self.pdf_error(b"ext4", b"\\pdfendthread ended up in different nesting level than \\pdfstartthread");
        }
        if self.is_running(self.pdf_thread_dp) && self.last_thread != NULL {
            self.set_pdf_bottom(self.last_thread, self.cur_v + self.pdf_thread_margin());
        }
        if self.pdf_last_thread_named_id {
            self.delete_token_ref(self.pdf_last_thread_id);
        }
        self.last_thread = NULL;
    }

    /// §1637
    pub fn open_subentries(&mut self, p: i32) -> i32 {
        let mut k = 0;
        if self.obj_outline_first(p) != 0 {
            let mut l = self.obj_outline_first(p);
            loop {
                k += 1;
                let c = self.open_subentries(l);
                if self.obj_outline_count(l) > 0 {
                    k += c;
                }
                self.set_obj_outline_parent(l, p);
                let r = self.obj_outline_next(l);
                if r == 0 {
                    self.set_obj_outline_last(p, l);
                }
                l = r;
                if l == 0 {
                    break;
                }
            }
        }
        if self.obj_outline_count(p) > 0 {
            self.set_obj_outline_count(p, k);
        } else {
            self.set_obj_outline_count(p, -k);
        }
        k
    }

    /// §1637
    pub fn do_dest(&mut self, p: i32, parent_box: i32, x: i32, y: i32) {
        if !self.is_shipping_page {
            self.pdf_error(b"ext4", b"destinations cannot be inside an XForm");
        }
        if self.doing_leaders {
            return;
        }
        let byname = self.pdf_dest_named_id(p) > 0;
        let k;
        if self.pdf_dest_objnum(p) == NULL {
            k = self.get_obj(OBJ_TYPE_DEST, self.pdf_dest_id(p), byname);
        } else {
            k = self.get_obj(OBJ_TYPE_STRUCT_DEST, self.pdf_dest_id(p), byname);
        }
        if self.obj_dest_ptr(k) != NULL {
            self.warn_dest_dup(self.pdf_dest_id(p), self.pdf_dest_named_id(p), b"ext4", b"has been already used, duplicate ignored");
            return;
        }
        self.set_obj_dest_ptr(k, p);
        self.pdf_dest_list = self.append_ptr(self.pdf_dest_list, k);
        match self.pdf_dest_type(p) {
            PDF_DEST_XYZ => {
                if self.matrixused() {
                    self.set_rect_dimens(p, parent_box, x, y, self.pdf_width(p), self.pdf_height(p), self.pdf_depth(p), self.pdf_dest_margin());
                } else {
                    self.set_pdf_left(p, self.cur_h);
                    self.set_pdf_top(p, self.cur_v);
                }
            }
            PDF_DEST_FITH | PDF_DEST_FITBH => {
                if self.matrixused() {
                    self.set_rect_dimens(p, parent_box, x, y, self.pdf_width(p), self.pdf_height(p), self.pdf_depth(p), self.pdf_dest_margin());
                } else {
                    self.set_pdf_top(p, self.cur_v);
                }
            }
            PDF_DEST_FITV | PDF_DEST_FITBV => {
                if self.matrixused() {
                    self.set_rect_dimens(p, parent_box, x, y, self.pdf_width(p), self.pdf_height(p), self.pdf_depth(p), self.pdf_dest_margin());
                } else {
                    self.set_pdf_left(p, self.cur_h);
                }
            }
            PDF_DEST_FIT | PDF_DEST_FITB => {}
            PDF_DEST_FITR => {
                self.set_rect_dimens(p, parent_box, x, y, self.pdf_width(p), self.pdf_height(p), self.pdf_depth(p), self.pdf_dest_margin());
            }
            _ => {}
        }
    }

    /// §1637
    pub fn out_form(&mut self, p: i32) {
        self.synctexpdfrefxform(self.pdf_xform_objnum(p));
        self.pdf_end_text();
        self.pdf_print_ln_s(b"q");
        if self.pdf_lookup_list(self.pdf_xform_list, self.pdf_xform_objnum(p)) == NULL {
            self.pdf_xform_list = self.append_ptr(self.pdf_xform_list, self.pdf_xform_objnum(p));
        }
        self.cur_v += self.obj_xform_depth(self.pdf_xform_objnum(p));
        self.pdf_print_s(b"1 0 0 1 ");
        self.pdf_print_bp(self.pdf_x(self.cur_h));
        self.pdf_out(b' ' as i32);
        self.pdf_print_bp(self.pdf_y(self.cur_v));
        self.pdf_print_ln_s(b" cm");
        self.pdf_print_s(b"/Fm");
        self.pdf_print_int(self.obj_info(self.pdf_xform_objnum(p)));
        self.pdf_print_resname_prefix();
        self.pdf_print_ln_s(b" Do");
        self.pdf_print_ln_s(b"Q");
    }

    /// §1637
    pub fn out_image(&mut self, p: i32) {
        let image = self.obj_ximage_data(self.pdf_ximage_objnum(p));
        let (img_w, img_h);
        if self.image_rotate(image) == 90 || self.image_rotate(image) == 270 {
            img_h = self.image_width(image);
            img_w = self.image_height(image);
        } else {
            img_w = self.image_width(image);
            img_h = self.image_height(image);
        }
        self.pdf_end_text();
        self.pdf_print_ln_s(b"q");
        if self.pdf_lookup_list(self.pdf_ximage_list, self.pdf_ximage_objnum(p)) == NULL {
            self.pdf_ximage_list = self.append_ptr(self.pdf_ximage_list, self.pdf_ximage_objnum(p));
        }
        if !self.is_pdf_image(image) {
            if self.is_png_image(image) {
                let groupref = self.get_image_group_ref(image);
                if groupref > 0 && self.pdf_page_group_val == 0 {
                    self.pdf_page_group_val = groupref;
                }
            }
            let v = self.ext_xn_over_d(self.pdf_width(p), self.ten_pow[6], self.one_hundred_bp);
            self.pdf_print_real(v, 4);
            self.pdf_print_s(b" 0 0 ");
            let v = self.ext_xn_over_d(self.pdf_height(p) + self.pdf_depth(p), self.ten_pow[6], self.one_hundred_bp);
            self.pdf_print_real(v, 4);
            self.pdf_out(b' ' as i32);
            self.pdf_print_bp(self.pdf_x(self.cur_h));
            self.pdf_out(b' ' as i32);
            self.pdf_print_bp(self.pdf_y(self.cur_v));
        } else {
            // for pdf images we generate the page group object number here
            let groupref = self.get_image_group_ref(image); // 0: no group, -1: to be generated; >0: already written
            if groupref != 0 && self.pdf_page_group_val == 0 {
                if groupref == -1 {
                    self.pdf_page_group_val = self.pdf_new_objnum();
                    self.set_image_group_ref(image, self.pdf_page_group_val);
                } else {
                    // groupref > 0
                    self.pdf_page_group_val = groupref;
                }
            }
            let v = self.ext_xn_over_d(self.pdf_width(p), self.ten_pow[6], img_w);
            self.pdf_print_real(v, 6);
            self.pdf_print_s(b" 0 0 ");
            let v = self.ext_xn_over_d(self.pdf_height(p) + self.pdf_depth(p), self.ten_pow[6], img_h);
            self.pdf_print_real(v, 6);
            self.pdf_out(b' ' as i32);
            let o = self.epdf_orig_x(image);
            let v = self.ext_xn_over_d(self.pdf_width(p), o, img_w);
            self.pdf_print_bp(self.pdf_x(self.cur_h) - v);
            self.pdf_out(b' ' as i32);
            let o = self.epdf_orig_y(image);
            let v = self.ext_xn_over_d(self.pdf_height(p) + self.pdf_depth(p), o, img_h);
            self.pdf_print_bp(self.pdf_y(self.cur_v) - v);
        }
        self.pdf_print_ln_s(b" cm");
        self.pdf_print_s(b"/Im");
        self.pdf_print_int(self.obj_info(self.pdf_ximage_objnum(p)));
        self.pdf_print_resname_prefix();
        self.pdf_print_ln_s(b" Do");
        self.pdf_print_ln_s(b"Q");
    }

    /// §1637 find the gap between the position of the current snap node `p` and the nearest point on the grid
    pub fn gap_amount(&mut self, p: i32, cur_pos: i32) -> i32 {
        let snap_unit = self.width(self.snap_glue_ptr(p));
        let stretch_amount;
        if self.stretch_order(self.snap_glue_ptr(p)) > NORMAL {
            stretch_amount = MAX_DIMEN;
        } else {
            stretch_amount = self.stretch(self.snap_glue_ptr(p));
        }
        let shrink_amount;
        if self.shrink_order(self.snap_glue_ptr(p)) > NORMAL {
            shrink_amount = MAX_DIMEN;
        } else {
            shrink_amount = self.shrink(self.snap_glue_ptr(p));
        }
        let last_pos;
        if self.subtype(p) == PDF_SNAPY_NODE {
            last_pos = self.pdf_snapy_refpos + snap_unit * ((cur_pos - self.pdf_snapy_refpos) / snap_unit);
        } else {
            self.pdf_error(b"snapping", b"invalid parameter value for gap_amount");
        }
        let next_pos = last_pos + snap_unit;
        let mut g = MAX_DIMEN;
        let mut g2 = MAX_DIMEN;
        if cur_pos - last_pos < shrink_amount {
            g = cur_pos - last_pos;
        }
        if next_pos - cur_pos < stretch_amount {
            g2 = next_pos - cur_pos;
        }
        if g == MAX_DIMEN && g2 == MAX_DIMEN {
            return 0; // unable to snap
        }
        if g2 <= g {
            g2 // skip forward
        } else {
            -g // skip backward
        }
    }

    /// §1637 find the vertical position of node `q` in the output PDF page; this function is called when the current
    /// node is `p` and current position is `cur_v`; `b` is the parent box
    pub fn get_vpos(&mut self, p: i32, q: i32, b: i32) -> i32 {
        let mut p = p;
        let mut tmp_v = self.cur_v;
        let this_box = b;
        let mut cur_g = 0;
        let mut cur_glue = 0.0f64;
        let g_order = self.glue_order(this_box);
        let g_sign = self.glue_sign(this_box);
        while p != q && p != NULL {
            if self.is_char_node(p) {
                self.confusion(b"get_vpos");
            } else {
                match self.ty(p) {
                    HLIST_NODE | VLIST_NODE | RULE_NODE => {
                        tmp_v += self.height(p) + self.depth(p);
                    }
                    WHATSIT_NODE => {
                        if self.subtype(p) == PDF_REFXFORM_NODE || self.subtype(p) == PDF_REFXIMAGE_NODE {
                            tmp_v += self.pdf_height(p) + self.pdf_depth(p);
                        }
                    }
                    GLUE_NODE => {
                        // §1638 Move down without outputting leaders
                        self.g = self.glue_ptr(p);
                        self.rule_ht = self.width(self.g) - cur_g;
                        if g_sign != NORMAL {
                            if g_sign == STRETCHING {
                                if self.stretch_order(self.g) == g_order {
                                    cur_glue += self.stretch(self.g) as f64;
                                    let glue_temp = vet_glue(self.glue_set(this_box) * cur_glue);
                                    cur_g = round(glue_temp);
                                }
                            } else if self.shrink_order(self.g) == g_order {
                                cur_glue -= self.shrink(self.g) as f64;
                                let glue_temp = vet_glue(self.glue_set(this_box) * cur_glue);
                                cur_g = round(glue_temp);
                            }
                        }
                        self.rule_ht += cur_g;
                        tmp_v += self.rule_ht;
                    }
                    KERN_NODE => {
                        tmp_v += self.width(p);
                    }
                    _ => {}
                }
            }
            p = self.link(p);
        }
        tmp_v
    }

    /// §1637 do snapping compensation in vertical direction; search for the next snap node and do the compensation if
    /// found
    pub fn do_snapy_comp(&mut self, p: i32, b: i32) {
        if !(!self.is_char_node(p) && self.ty(p) == WHATSIT_NODE && self.subtype(p) == PDF_SNAPY_COMP_NODE) {
            self.pdf_error(b"snapping", b"invalid parameter value for do_snapy_comp");
        }
        let mut q = p;
        while q != NULL {
            if !self.is_char_node(q) && self.ty(q) == WHATSIT_NODE && self.subtype(q) == PDF_SNAPY_NODE {
                let tmp_v = self.get_vpos(p, q, b); // get the position of q
                let g = self.gap_amount(q, tmp_v); // get the gap to the grid
                let g2 = self.round_xn_over_d(g, self.snapy_comp_ratio(p), 1000); // adjustment for p
                self.cur_v += g2;
                self.set_final_skip(q, g - g2); // adjustment for q
                if self.final_skip(q) == 0 {
                    self.set_final_skip(q, 1); // use 1sp as the magic value to record that final_skip has been set here
                }
                return;
            }
            q = self.link(q);
        }
    }

    /// §1637
    pub fn do_snapy(&mut self, p: i32) {
        self.count_do_snapy += 1;
        if self.final_skip(p) != 0 {
            self.cur_v += self.final_skip(p);
        } else {
            self.cur_v += self.gap_amount(p, self.cur_v);
        }
    }

    // ---- External functions: the C side of web2c and pdfTeX, called by WEB. ----
    //
    // The first group is the half of utils.c's matrix code that WEB's annotations call.  The other half, which fills
    // the stack, is in pdfship.rs, which keeps the stack and `page_mode`: `v_pdfship.matrix_stack`.

    /// utils.c `do_matrixtransform`.  `scaled` is 64 bits in the C, and the callers store in an `i32`.
    fn ext_do_matrixtransform(m: &crate::pdfship::MatrixEntry, x: i32, y: i32) -> (i32, i32) {
        let (x_old, y_old) = (x as f64, y as f64);
        let x_new = x_old * m.a + y_old * m.c + m.e;
        let y_new = x_old * m.b + y_old * m.d + m.f;
        let do_round = |v: f64| if v > 0.0 { v + 0.5 } else { v - 0.5 };
        (sat32(do_round(x_new) as i64), sat32(do_round(y_new) as i64))
    }

    /// External: utils.c
    pub fn matrixtransformrect(&mut self, llx: i32, lly: i32, urx: i32, ury: i32) {
        if let Some(m) = self.pdfship_matrix_top() {
            self.v_ext.last_llx = llx;
            self.v_ext.last_lly = lly;
            self.v_ext.last_urx = urx;
            self.v_ext.last_ury = ury;
            let (x1, y1) = Self::ext_do_matrixtransform(&m, llx, lly);
            let (x2, y2) = Self::ext_do_matrixtransform(&m, llx, ury);
            let (x3, y3) = Self::ext_do_matrixtransform(&m, urx, lly);
            let (x4, y4) = Self::ext_do_matrixtransform(&m, urx, ury);
            self.v_ext.ret_llx = x1.min(x2).min(x3.min(x4));
            self.v_ext.ret_lly = y1.min(y2).min(y3.min(y4));
            self.v_ext.ret_urx = x1.max(x2).max(x3.max(x4));
            self.v_ext.ret_ury = y1.max(y2).max(y3.max(y4));
        } else {
            self.v_ext.ret_llx = llx;
            self.v_ext.ret_lly = lly;
            self.v_ext.ret_urx = urx;
            self.v_ext.ret_ury = ury;
        }
    }

    /// External: utils.c
    pub fn matrixrecalculate(&mut self, urx: i32) {
        let (llx, lly, ury) = (self.v_ext.last_llx, self.v_ext.last_lly, self.v_ext.last_ury);
        self.matrixtransformrect(llx, lly, urx, ury);
    }

    /// External: utils.c
    pub fn matrixused(&mut self) -> bool {
        self.v_pdfship.matrix_stack.len > 0
    }

    /// External: utils.c
    pub fn getllx(&mut self) -> i32 {
        self.v_ext.ret_llx
    }

    /// External: utils.c
    pub fn getlly(&mut self) -> i32 {
        self.v_ext.ret_lly
    }

    /// External: utils.c
    pub fn geturx(&mut self) -> i32 {
        self.v_ext.ret_urx
    }

    /// External: utils.c
    pub fn getury(&mut self) -> i32 {
        self.v_ext.ret_ury
    }

    /// External: texmfmp.c `initstarttime`, called from §1584 on every run.  No taint: a run that never asks for the
    /// time does not depend on it.
    pub fn init_start_time(&mut self) {
        self.host.world.init_start_time();
    }


    // ---- writeimg.c ----

    /// `checkimageb`.
    pub fn check_image_b(&mut self, procset: i32) -> bool {
        procset & rstex_image::IMAGE_COLOR_B != 0
    }

    /// `checkimagec`.
    pub fn check_image_c(&mut self, procset: i32) -> bool {
        procset & rstex_image::IMAGE_COLOR_C != 0
    }

    /// `checkimagei`.
    pub fn check_image_i(&mut self, procset: i32) -> bool {
        procset & rstex_image::IMAGE_COLOR_I != 0
    }

    /// `ispdfimage`.
    pub fn is_pdf_image(&mut self, img: i32) -> bool {
        self.ext_img(img).image_type == IMAGE_TYPE_PDF
    }

    /// `ispngimage`.
    pub fn is_png_image(&mut self, img: i32) -> bool {
        self.ext_img(img).image_type == IMAGE_TYPE_PNG
    }

    /// `epdforigx`.
    pub fn epdf_orig_x(&mut self, img: i32) -> i32 {
        self.ext_img(img).orig_x
    }

    /// `epdforigy`.
    pub fn epdf_orig_y(&mut self, img: i32) -> i32 {
        self.ext_img(img).orig_y
    }

    /// `imagewidth`.
    pub fn image_width(&mut self, img: i32) -> i32 {
        sat32(self.ext_img(img).width)
    }

    /// `imageheight`.
    pub fn image_height(&mut self, img: i32) -> i32 {
        sat32(self.ext_img(img).height)
    }

    /// `imagerotate`.
    pub fn image_rotate(&mut self, img: i32) -> i32 {
        self.ext_img(img).rotate
    }

    /// `imagepages`.
    pub fn image_pages(&mut self, img: i32) -> i32 {
        self.ext_img(img).num_pages
    }

    /// `imagexres`.
    pub fn image_x_res(&mut self, img: i32) -> i32 {
        self.ext_img(img).x_res
    }

    /// `imageyres`.
    pub fn image_y_res(&mut self, img: i32) -> i32 {
        self.ext_img(img).y_res
    }

    /// `imagecolordepth`.  The bit depth is kept from `read_image`, since `png_get_bit_depth` only reads it back.
    pub fn image_colordepth(&mut self, img: i32) -> i32 {
        self.ext_img(img).color_depth
    }

    /// `getimagegroupref`.
    pub fn get_image_group_ref(&mut self, img: i32) -> i32 {
        self.ext_img(img).group_ref
    }

    /// `setimagegroupref`.
    pub fn set_image_group_ref(&mut self, img: i32, r: i32) {
        let mut e = self.ext_img(img);
        e.group_ref = r;
        self.ext_set_img(img, e);
    }

    /// `readimage`.
    pub fn read_image(
        &mut self,
        s: i32,
        page_num: i32,
        page_name: i32,
        colorspace: i32,
        page_box: i32,
        pdfmajorversion: i32,
        pdfminorversion: i32,
        inclusion_errorlevel: i32,
    ) -> i32 {
        let img = self.ext_new_image_entry();
        let dest = if page_name != 0 { Some(self.ext_make_cstring(page_name)) } else { None };
        let Some(name) = self.ext_find_input_file(s) else {
            let mut m = b"cannot find image file ".to_vec();
            m.extend_from_slice(&self.ext_make_cstring(s));
            self.ext_pdftex_fail(None, &m)
        };
        self.ext_set_img_name(img, &name);
        // recorder_record_input(cur_file_name): the Read observation below carries it.
        let data = self.ext_read_image_file(&name);
        // checktypebyheader
        if data.len() < 8 {
            self.ext_pdftex_fail(Some(&name), b"reading image file failed");
        }
        let mut e = self.ext_img(img);
        e.colorspace_ref = colorspace;
        e.selected_page = page_num;
        e.page_box = page_box;
        self.ext_load_image(img, &mut e, &name, data, dest.as_deref(), pdfmajorversion, pdfminorversion, inclusion_errorlevel);
        self.ext_set_img(img, e);
        img
    }

    /// What `readimage` and `undumpimagemeta` have in common: the type checks and the `switch`.  `e.selected_page` is
    /// `page_num` on entry, and `e.page_box` the box.
    #[allow(clippy::too_many_arguments)]
    fn ext_load_image(
        &mut self,
        img: i32,
        e: &mut ImageEntry,
        name: &[u8],
        data: Vec<u8>,
        dest: Option<&[u8]>,
        pdfmajorversion: i32,
        pdfminorversion: i32,
        inclusion_errorlevel: i32,
    ) {
        let mut ty = rstex_image::check_type_by_header(&data);
        if ty == ImageType::None {
            ty = rstex_image::check_type_by_extension(name);
        }
        match ty {
            ImageType::Pdf => {
                e.image_type = IMAGE_TYPE_PDF;
                let params = PdfInfoParams {
                    page_num: e.selected_page,
                    page_box: e.page_box,
                    major_wanted: pdfmajorversion,
                    minor_wanted: pdfminorversion,
                    inclusion_errorlevel,
                };
                let docs = &mut self.host.hv.ext.docs;
                let i = match docs.find_document(name) {
                    Some(i) => i,
                    None => match docs.add_document(name, data) {
                        Ok(i) => i,
                        Err(Fail(m)) => self.ext_pdftex_fail(Some(name), &m),
                    },
                };
                let info = match self.host.hv.ext.docs.doc_mut(i).read_pdf_info(dest, &params) {
                    Ok(info) => info,
                    Err(Fail(m)) => self.ext_pdftex_fail(Some(name), &m),
                };
                if let Some(w) = &info.warning {
                    self.ext_pdftex_warn(Some(name), w);
                }
                e.width = info.width as i64;
                e.height = info.height as i64;
                e.rotate = info.rotate;
                e.num_pages = info.pages;
                e.orig_x = info.orig_x;
                e.orig_y = info.orig_y;
                e.selected_page = info.selected_page;
                e.doc = i as i32;
                // page group present, but new object number not set yet; or no page group present
                e.group_ref = if info.has_page_group { -1 } else { 0 };
            }
            ImageType::Png => {
                e.image_type = IMAGE_TYPE_PNG;
                e.num_pages = 1;
                let info = match png::read_png_info(&data) {
                    Ok(info) => info,
                    Err(Fail(m)) => self.ext_pdftex_fail(Some(name), &m),
                };
                e.width = info.width as i64;
                e.height = info.height as i64;
                e.x_res = info.x_res;
                e.y_res = info.y_res;
                e.color_type = info.img_color;
                e.color_depth = info.bit_depth as i32;
                if info.wants_page_group(self.fixed_pdf_major_version, self.fixed_pdf_minor_version) {
                    // png with alpha channel in device colours; we have to add a Page Group to make Adobe happy, so we
                    // have to create a dummy group object
                    if self.v_ext.transparent_page_group == 0 {
                        self.v_ext.transparent_page_group = self.pdf_new_objnum();
                    }
                    if self.pdf_page_group_val == 0 {
                        self.pdf_page_group_val = self.v_ext.transparent_page_group;
                    }
                    e.group_ref = self.pdf_page_group_val;
                }
                self.ext_set_host_file(img, HostImage::Png { info, data });
            }
            ImageType::Jpg => {
                e.image_type = IMAGE_TYPE_JPG;
                e.num_pages = 1;
                let info = match jpg::read_jpg_info(&data, self.fixed_pdf_major_version, self.fixed_pdf_minor_version) {
                    Ok(info) => info,
                    Err(Fail(m)) => self.ext_pdftex_fail(Some(name), &m),
                };
                e.width = info.width as i64;
                e.height = info.height as i64;
                e.x_res = info.x_res;
                e.y_res = info.y_res;
                e.color_type = info.img_color;
                e.color_depth = info.bits_per_component;
                self.ext_set_host_file(img, HostImage::Jpg { info, data });
            }
            ImageType::Jbig2 => {
                if pdfmajorversion == 1 && pdfminorversion < 4 {
                    let m = format!(
                        "JBIG2 images only possible with at least PDF 1.4; you are generating PDF 1.{}",
                        pdfminorversion
                    );
                    self.ext_pdftex_fail(Some(name), m.as_bytes());
                }
                e.image_type = IMAGE_TYPE_JBIG2;
                // The C tests the page before it opens the file, so a bad page wins over a bad file.
                let r = if e.selected_page < 1 {
                    jbig2::read_jbig2_info(&Jbig2File::default(), e.selected_page)
                } else {
                    if !self.host.hv.ext.jbig2.contains_key(name) {
                        match jbig2::rd_jbig2_info(&data) {
                            Ok(f) => {
                                self.host.hv.ext.jbig2.insert(name.to_vec(), (f, data));
                            }
                            Err(Fail(m)) => self.ext_pdftex_fail(Some(name), &m),
                        }
                    }
                    jbig2::read_jbig2_info(&self.host.hv.ext.jbig2[name].0, e.selected_page)
                };
                let info = match r {
                    Ok(info) => info,
                    Err(Fail(m)) => self.ext_pdftex_fail(Some(name), &m),
                };
                e.num_pages = info.pages;
                // The crate's `i32`s hold the C's `unsigned long`s, which are read from 32 bits.
                e.width = info.width as u32 as i64;
                e.height = info.height as u32 as i64;
                e.x_res = info.x_res;
                e.y_res = info.y_res;
            }
            ImageType::None => self.ext_pdftex_fail(Some(name), b"unknown type of image"),
        }
        self.ext_mark_loaded(img);
    }

    /// `deleteimage`.
    pub fn delete_image(&mut self, img: i32) {
        if self.ini_version {
            return; // The image may be \dump{}ed to a format
        }
        let e = self.ext_img(img);
        match e.image_type {
            IMAGE_TYPE_PDF => {
                self.host.hv.ext.docs.epdf_delete(e.doc as usize);
            }
            IMAGE_TYPE_PNG | IMAGE_TYPE_JPG => {
                self.host.hv.ext.files[img as usize] = None;
            }
            IMAGE_TYPE_JBIG2 => {}
            _ => {
                let name = self.ext_img_name(img);
                self.ext_pdftex_fail(Some(&name), b"unknown type of image");
            }
        }
    }

    /// `updateimageprocset`.
    pub fn update_image_procset(&mut self, img: i32) {
        self.pdf_image_procset |= self.ext_img(img).color_type;
    }

    /// `writeimage`.  The writers return their calls as a list, replayed here: the top of `crates/image/src/lib.rs`.
    pub fn write_image(&mut self, img: i32) {
        let e = self.ext_img(img);
        let name = self.ext_img_name(img);
        // Written before a checkpoint: what a PDF file or a JBIG2 file has copied stays in Host for the next image of that file, and no restore brings it back.
        if e.image_type == IMAGE_TYPE_PDF || e.image_type == IMAGE_TYPE_JBIG2 {
            self.host.world.mark_taint_because(Taint::Hard, "an included file was written");
        }
        let mut m = b" <".to_vec();
        m.extend_from_slice(&name);
        self.ext_tex_printf(&m);
        match e.image_type {
            IMAGE_TYPE_PNG => {
                let Some(HostImage::Png { info, data }) = self.host.hv.ext.files[img as usize].take() else {
                    self.confusion(b"ext image")
                };
                let params = png::PngParams {
                    pdf_major: self.fixed_pdf_major_version,
                    pdf_minor: self.fixed_pdf_minor_version,
                    image_hicolor: self.fixed_image_hicolor as i32,
                    image_apply_gamma: self.fixed_image_apply_gamma,
                    gamma: self.fixed_gamma,
                    image_gamma: self.fixed_image_gamma,
                    colorspace_ref: e.colorspace_ref,
                };
                let mut slots = self.host.hv.ext.slots;
                let r = png::write_png(&info, &data, &params, &mut slots);
                self.host.hv.ext.slots = slots;
                let out = match r {
                    Ok(out) => out,
                    Err(Fail(m)) => self.ext_pdftex_fail(Some(&name), &m),
                };
                self.ext_replay(out.ops, &data, &name);
                // write_additional_png_objects
                if out.needs_page_group
                    && !self.v_ext.transparent_page_group_was_written
                    && self.v_ext.transparent_page_group > 0
                {
                    self.v_ext.transparent_page_group_was_written = true;
                    let level = self.get_pdf_compress_level();
                    let ops = png::png_page_group_object(self.v_ext.transparent_page_group, level);
                    self.ext_replay(ops, &data, &name);
                }
                self.host.hv.ext.files[img as usize] = Some(HostImage::Png { info, data });
            }
            IMAGE_TYPE_JPG => {
                let Some(HostImage::Jpg { info, data }) = self.host.hv.ext.files[img as usize].take() else {
                    self.confusion(b"ext image")
                };
                let mut slots = self.host.hv.ext.slots;
                let r = jpg::write_jpg(&info, e.colorspace_ref, &mut slots);
                self.host.hv.ext.slots = slots;
                let ops = match r {
                    Ok(ops) => ops,
                    Err(Fail(m)) => self.ext_pdftex_fail(Some(&name), &m),
                };
                self.ext_replay(ops, &data, &name);
                self.host.hv.ext.files[img as usize] = Some(HostImage::Jpg { info, data });
            }
            IMAGE_TYPE_JBIG2 => {
                let Some((mut file, bytes)) = self.host.hv.ext.jbig2.remove(&name) else {
                    self.confusion(b"ext image")
                };
                let mut slots = self.host.hv.ext.slots;
                let r = jbig2::write_jbig2(&mut file, &bytes, e.selected_page, &mut slots);
                self.host.hv.ext.slots = slots;
                let ops = match r {
                    Ok(ops) => ops,
                    Err(Fail(m)) => self.ext_pdftex_fail(Some(&name), &m),
                };
                self.ext_replay(ops, &bytes, &name);
                self.host.hv.ext.jbig2.insert(name.clone(), (file, bytes));
            }
            IMAGE_TYPE_PDF => self.ext_write_epdf(&e, &name),
            _ => self.ext_pdftex_fail(Some(&name), b"unknown type of image"),
        }
        self.ext_tex_printf(b">");
    }

    /// `write_epdf`, and the questions `copyFont` asks the font map.
    fn ext_write_epdf(&mut self, e: &ImageEntry, name: &[u8]) {
        let doc = e.doc as usize;
        let copy_font = self.fixed_inclusion_copy_font != 0;
        let queries = if copy_font { Vec::new() } else { self.host.hv.ext.docs.doc_mut(doc).epdf_font_queries(e.selected_page) };
        let mut hits = Vec::new();
        for q in queries {
            if self.ext_epdf_fontmap_hit(&q) {
                hits.push(q);
            }
        }
        let suppress_ptex_info = self.get_pdf_suppress_ptex_info();
        let use_underscore = self.get_ptex_use_underscore();
        let suppress_warning_page_group = self.get_pdf_suppress_warning_page_group() != 0;
        let p = EpdfParams {
            selected_page: e.selected_page,
            page_box: e.page_box,
            page_group_val: self.pdf_page_group_val,
            suppress_ptex_info,
            use_underscore,
            suppress_warning_page_group,
            inclusion_copy_font: copy_font,
            fontmap_hits: &hits,
        };
        let mut slots = self.host.hv.ext.slots;
        let r = self.host.hv.ext.docs.doc_mut(doc).write_epdf(&p, &mut slots);
        self.host.hv.ext.slots = slots;
        let out = match r {
            Ok(out) => out,
            Err(Fail(m)) => self.ext_pdftex_fail(Some(name), &m),
        };
        self.pdf_page_group_val = out.page_group_val;
        // The ops refer to the document's bytes, and nothing a replay calls opens a document.
        let mut docs = core::mem::take(&mut self.host.hv.ext.docs);
        self.ext_replay(out.ops, docs.doc_mut(doc).data(), name);
        self.host.hv.ext.docs = docs;
    }

    /// `flushjbig2page0objects`, for each file in `strcmp` order of the names.  For `flush_jbig2_page0_objects`, §794.
    pub fn ext_flush_jbig2_page0(&mut self) {
        let mut files = core::mem::take(&mut self.host.hv.ext.jbig2);
        for (name, (file, bytes)) in files.iter_mut() {
            let mut slots = self.host.hv.ext.slots;
            let r = jbig2::flush_page0(file, bytes, &mut slots);
            self.host.hv.ext.slots = slots;
            match r {
                Ok(ops) => self.ext_replay(ops, bytes, name),
                Err(Fail(m)) => self.ext_pdftex_fail(Some(name), &m),
            }
        }
        self.host.hv.ext.jbig2 = files;
    }

    /// `undumpimagemeta`, for `undump_image_meta`: gives each image of a format what it had behind its pointers, in
    /// the C's order and with its object numbers.  A checkpoint has no such step: `write_image` does this on first use.
    pub fn ext_undump_images(&mut self, pdfmajorversion: i32, pdfminorversion: i32, pdfinclusionerrorlevel: i32) {
        for img in 0..self.v_ext.images.len {
            self.ext_image_restore(img, pdfmajorversion, pdfminorversion, pdfinclusionerrorlevel);
        }
    }

    /// A restored run comes here at once, before it can do anything else with an image.  The table came back with the image; what stands behind it is Host's, and the run must have
    /// it as a cold run has it, each file as many times as the table names it and in that order: the count of a PDF file's uses decides how long the document stays open, and with it
    /// what it has already copied.  All this was said and checked when the preamble named them, so this time nothing is said, or recorded, or checked against the parameters of now.
    /// An image written with `\\immediate` is read again for nothing, and only a PNG or JPEG gets that far: `write_image`.
    pub fn ext_images_after_restore(&mut self) {
        for img in 0..self.v_ext.images.len {
            if self.ext_is_loaded(img) {
                continue;
            }
            let mut e = self.ext_img(img);
            let name = self.ext_img_name(img);
            let Some(data) = self.host.world.read_quietly(&name) else {
                let mut m = b"reading image file failed: ".to_vec();
                m.extend_from_slice(&name);
                self.ext_pdftex_fail(None, &m)
            };
            let group_ref = e.group_ref;
            self.ext_load_image(img, &mut e, &name, data, None, 1, 7, -1);
            if e.image_type == IMAGE_TYPE_PDF {
                e.group_ref = group_ref;
            }
            self.ext_set_img(img, e);
        }
    }

    /// One image of `undumpimagemeta`, if this process does not have it yet.
    fn ext_image_restore(&mut self, img: i32, pdfmajorversion: i32, pdfminorversion: i32, pdfinclusionerrorlevel: i32) {
        if self.ext_is_loaded(img) {
            return;
        }
        let mut e = self.ext_img(img);
        let name = self.ext_img_name(img);
        if self.host.world.lookup(&name, KPSE_TEX_FORMAT, true).is_none() {
            let mut m = b"cannot find image file ".to_vec();
            m.extend_from_slice(&name);
            self.ext_pdftex_fail(None, &m);
        }
        let data = self.ext_read_image_file(&name);
        let group_ref = e.group_ref;
        self.ext_load_image(img, &mut e, &name, data, None, pdfmajorversion, pdfminorversion, pdfinclusionerrorlevel);
        // `undumpimagemeta` keeps what it read for a PDF file, and for the others `read_png_info` may set it.
        if e.image_type == IMAGE_TYPE_PDF {
            e.group_ref = group_ref;
        }
        self.ext_set_img(img, e);
    }

    /// The replay loop of `crates/image/src/lib.rs`.  `input` is what the ops' `CopyInput` refer to, and `file` the name
    /// that a `pdftex_warn` prints: `cur_file_name`.
    fn ext_replay(&mut self, ops: Vec<Op>, input: &[u8], file: &[u8]) {
        for op in ops {
            match op {
                Op::Puts(b) => self.pdfout_bytes_owned(b),
                Op::CopyInput { start, len } => self.pdfout_bytes(&input[start..start + len]),
                Op::NewObj(s) => {
                    let n = self.pdf_new_objnum();
                    self.ext_set_slot(s, n);
                }
                Op::Num(s) => {
                    let n = self.host.hv.ext.table[s as usize];
                    self.pdf_print_int(n);
                }
                Op::BeginDict { obj, level } => {
                    let n = self.ext_obj_no(obj);
                    self.pdf_begin_dict(n, level);
                }
                Op::BeginObj { obj, level } => {
                    let n = self.ext_obj_no(obj);
                    self.pdf_begin_obj(n, level);
                }
                Op::EndDict => self.pdf_end_dict(),
                Op::EndObj => self.pdf_end_obj(),
                Op::BeginStream => self.pdf_begin_stream(),
                Op::EndStream => self.pdf_end_stream(),
                Op::Flush => self.pdf_flush(),
                Op::TexPrint(b) => self.ext_tex_printf(&b),
                Op::Warn(b) => self.ext_pdftex_warn(Some(file), &b),
                Op::EpdfFont { fd_slot, basefont, stem_v, charset } => {
                    let (fd, objnum) = self.ext_epdf_font_descriptor(&basefont, stem_v, charset.as_deref());
                    self.host.hv.ext.epdf_fds.push((fd_slot, fd));
                    self.ext_set_slot(fd_slot, objnum);
                }
                Op::EpdfFontName { fd_slot, fn_slot } => {
                    let fd = self.host.hv.ext.epdf_fds.iter().rev().find(|x| x.0 == fd_slot).map_or(0, |x| x.1);
                    let objnum = self.ext_epdf_font_name(fd);
                    self.ext_set_slot(fn_slot, objnum);
                }
            }
        }
    }

    // What `copyFont` needs of mapfile.c and writefont.c, which are in pdffont.rs.  The C's `lookup_fontmap` also ends the
    // caller's string before `-Slant_` or `-Extend_`.  That string is `dictLookup`'s copy of the name, which nothing else
    // reads, so a slanted font's own dictionary is unchanged; `pdffont_lookup_fontmap` works on the same cut name.

    /// `lookup_fontmap(name) != NULL`.
    fn ext_epdf_fontmap_hit(&mut self, name: &[u8]) -> bool {
        self.pdffont_lookup_fontmap(name) >= 0
    }

    /// `fm = lookup_fontmap(basefont); fd = epdf_create_fontdescriptor(fm, stem_v)`, then, if `charset` is given and
    /// `is_subsetable(fm)`, `epdf_mark_glyphs(fd, charset)`, otherwise `embed_whole_font(fd)`.  Returns `fd` and
    /// `get_fd_objnum(fd)`.  `basefont` is one `ext_epdf_fontmap_hit` said yes to.
    fn ext_epdf_font_descriptor(&mut self, basefont: &[u8], stem_v: i32, charset: Option<&[u8]>) -> (i32, i32) {
        let fm = self.pdffont_lookup_fontmap(basefont);
        let fd = self.pdffont_epdf_create_fontdescriptor(fm, stem_v);
        match charset {
            Some(charset) if self.pdffont_is_subsetable(fm) => self.pdffont_epdf_mark_glyphs(fd, charset),
            _ => self.pdffont_embed_whole_font(fd),
        }
        let objnum = self.pdffont_get_fd_objnum(fd);
        (fd, objnum)
    }

    /// `get_fn_objnum(fd)`.
    fn ext_epdf_font_name(&mut self, fd: i32) -> i32 {
        self.pdffont_get_fn_objnum(fd)
    }

    fn ext_obj_no(&self, o: ObjNo) -> i32 {
        match o {
            ObjNo::Slot(s) => self.host.hv.ext.table[s as usize],
            ObjNo::Known(n) => n,
        }
    }

    fn ext_set_slot(&mut self, s: u32, v: i32) {
        let x = &mut self.host.hv.ext;
        let need = (s as usize + 1).max(x.slots.len());
        if x.table.len() < need {
            x.table.resize(need, 0);
        }
        x.table[s as usize] = v;
    }

    /// `new_image_entry`.
    fn ext_new_image_entry(&mut self) -> i32 {
        let e = ImageEntry {
            image_type: 0,
            color_type: 0,
            width: 0,
            height: 0,
            rotate: 0,
            x_res: 0,
            y_res: 0,
            num_pages: 0,
            colorspace_ref: 0,
            group_ref: 0,
            color_depth: 0,
            orig_x: 0,
            orig_y: 0,
            selected_page: 0,
            page_box: 0,
            doc: 0,
            name_start: 0,
            name_len: 0,
        };
        let mut v = self.v_ext.images;
        self.arena_push(&mut v, e);
        self.v_ext.images = v;
        v.len - 1
    }

    #[inline]
    fn ext_img(&mut self, img: i32) -> ImageEntry {
        let v = self.v_ext.images;
        v.get(&mut self.arena, img)
    }

    #[inline]
    fn ext_set_img(&mut self, img: i32, e: ImageEntry) {
        let v = self.v_ext.images;
        v.set(&mut self.arena, img, e);
    }

    /// `img_name(img) = name`.
    fn ext_set_img_name(&mut self, img: i32, name: &[u8]) {
        let mut v = self.v_ext.image_names;
        let start = v.len;
        for &c in name {
            self.arena_push(&mut v, c);
        }
        self.v_ext.image_names = v;
        let mut e = self.ext_img(img);
        e.name_start = start;
        e.name_len = name.len() as i32;
        self.ext_set_img(img, e);
    }

    /// `img_name(img)`.
    fn ext_img_name(&mut self, img: i32) -> Vec<u8> {
        let e = self.ext_img(img);
        let v = self.v_ext.image_names;
        (0..e.name_len).map(|i| v.get(&mut self.arena, e.name_start + i)).collect()
    }

    fn ext_set_host_file(&mut self, img: i32, f: HostImage) {
        let x = &mut self.host.hv.ext;
        if x.files.len() <= img as usize {
            x.files.resize_with(img as usize + 1, || None);
        }
        x.files[img as usize] = Some(f);
    }

    fn ext_is_loaded(&self, img: i32) -> bool {
        self.host.hv.ext.loaded.get(img as usize).copied().unwrap_or(false)
    }

    fn ext_mark_loaded(&mut self, img: i32) {
        let x = &mut self.host.hv.ext;
        if x.loaded.len() <= img as usize {
            x.loaded.resize(img as usize + 1, false);
        }
        x.loaded[img as usize] = true;
    }

    /// `makecstring`: the bytes of a string, up to a NUL.
    fn ext_make_cstring(&self, s: i32) -> Vec<u8> {
        let b = self.str_pool.range(self.str_start[s], self.str_start[s + 1]);
        let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        b[..n].to_vec()
    }

    /// texmfmp.c `find_input_file`.
    fn ext_find_input_file(&mut self, s: i32) -> Option<Vec<u8>> {
        // makecfilename: without quotes
        let mut filename = self.ext_make_cstring(s);
        filename.retain(|&c| c != b'"');
        // Look in -output-directory first, if the filename is not absolute.  This is because we want the pdf* functions
        // to be able to find the same files as \openin.
        if let Some(dir) = self.host.world.output_directory.clone() {
            if !crate::platform::is_absolute(&filename) {
                let mut pathname = dir;
                pathname.push(b'/');
                pathname.extend_from_slice(&filename);
                // If there happens to be a directory by that name, never mind.  A read of a directory fails.
                if self.host.world.read(&pathname).is_some() {
                    return Some(pathname);
                }
            }
        }
        if !rstex_kpse::in_name_ok(&filename) {
            return None; // no permission
        }
        self.host.world.lookup(&filename, KPSE_TEX_FORMAT, true)
    }

    /// `xfopen`: which stops the run if it cannot open what the search found.
    fn ext_read_image_file(&mut self, name: &[u8]) -> Vec<u8> {
        match self.host.world.read(name) {
            Some(d) => d,
            None => {
                let mut m = b"reading image file failed: ".to_vec();
                m.extend_from_slice(name);
                self.ext_pdftex_fail(None, &m)
            }
        }
    }

    /// `tex_printf`.
    fn ext_tex_printf(&mut self, b: &[u8]) {
        self.print_s(&b[..b.len().min(PRINT_BUF_SIZE - 1)]);
        self.update_terminal();
    }

    /// utils.c `pdftex_warn`.  `file` is `cur_file_name`.
    pub fn ext_pdftex_warn(&mut self, file: Option<&[u8]>, msg: &[u8]) {
        self.print_ln();
        self.print_ln();
        let inv = rstex_kpse::invocation_name();
        self.ext_tex_printf(b"pdfTeX warning: ");
        self.ext_tex_printf(&inv);
        if let Some(f) = file {
            self.ext_tex_printf(b" (file ");
            self.ext_tex_printf(f);
            self.ext_tex_printf(b")");
        }
        self.ext_tex_printf(b": ");
        self.print_s(&msg[..msg.len().min(PRINT_BUF_SIZE - 1)]);
        self.print_ln();
    }

    /// utils.c `pdftex_fail`.  `file` is `cur_file_name`.
    pub fn ext_pdftex_fail(&mut self, file: Option<&[u8]>, msg: &[u8]) -> ! {
        self.print_ln();
        self.ext_safe_print(b"!pdfTeX error: ");
        let inv = rstex_kpse::invocation_name();
        self.ext_safe_print(&inv);
        if let Some(f) = file {
            self.ext_safe_print(b" (file ");
            self.ext_safe_print(f);
            self.ext_safe_print(b")");
        }
        self.ext_safe_print(b": ");
        self.ext_safe_print(&msg[..msg.len().min(PRINT_BUF_SIZE - 1)]);
        self.print_ln();
        self.remove_pdffile();
        self.ext_safe_print(b" ==> Fatal error occurred, no output PDF file produced!");
        self.print_ln();
        self.uexit(1)
    }

    /// utils.c `safe_print`.  It passes a `char`, which is signed on the oracle's machine: a byte above 127 is negative,
    /// so `print` makes it `???`.  Checked on the oracle, and kept on every platform.
    fn ext_safe_print(&mut self, s: &[u8]) {
        for &c in s {
            self.print(c as i8 as i32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::memkit::*;

    /// The arithmetic of `scale_image`, against the oracle: tests/oracle/gen_ext_tables.py.  Each row is a JBIG2 image
    /// of known size and resolution, with `\pdfimageresolution`, and what the rule spec gives.
    #[test]
    fn scale_image_as_the_oracle() {
        let tsv = include_str!("../testdata/ext_scale_oracle.tsv");
        let mut n = 0;
        for line in tsv.lines() {
            if line.starts_with('#') {
                continue;
            }
            let f: Vec<i64> = line.split('\t').map(|x| x.parse().unwrap()).collect();
            let (x, y, xr, yr, dflt) = (f[0], f[1], f[2] as i32, f[3] as i32, f[4] as i32);
            let t = crate::toks::testkit::machine();
            t.one_hundred_inch = 473628672;
            t.selector = NO_PRINT;
            // Three words of pdf_mem, as obj_ximage_width and its neighbours read them.
            t.set_obj_aux(1, 1);
            t.set_obj_ximage_width(1, f[5] as i32);
            t.set_obj_ximage_height(1, f[6] as i32);
            t.set_obj_ximage_depth(1, f[7] as i32);
            t.ext_scale_image_dims(1, x, y, xr, yr, false, dflt);
            // The oracle's numbers are those of an hbox holding the image, which has no negative height or depth.
            let got = (t.obj_ximage_width(1), t.obj_ximage_height(1).max(0), t.obj_ximage_depth(1).max(0));
            let want = (sat32(f[8]), sat32(f[9]), sat32(f[10]));
            assert_eq!(got, want, "{}", line);
            Tex::dealloc(t);
            n += 1;
        }
        assert!(n >= 300, "the table has {} rows", n);
    }

    #[test]
    fn a_pdf_image_takes_its_size_as_it_is() {
        let t = crate::toks::testkit::machine();
        t.set_obj_aux(1, 1);
        t.set_obj_ximage_width(1, NULL_FLAG);
        t.set_obj_ximage_height(1, NULL_FLAG);
        t.set_obj_ximage_depth(1, NULL_FLAG);
        t.ext_scale_image_dims(1, 6578176, 3289088, 0, 0, true, 0);
        assert_eq!((t.obj_ximage_width(1), t.obj_ximage_height(1), t.obj_ximage_depth(1)), (6578176, 3289088, 0));
        Tex::dealloc(t);
    }

    /// `\pdfsetmatrix` as pdfship.rs does it, at the origin: the stack these functions read.
    fn set_matrix(t: &mut Tex, text: &[u8]) {
        let b = t.pool_ptr;
        for &c in text {
            t.append_char(c as i32);
        }
        t.append_char(0);
        assert_eq!(t.pdfsetmatrix(b, 0, 0), 1);
        t.pool_ptr = b;
    }

    /// utils.c: the matrix `a b c d` at the origin, with an annotation's four corners.
    #[test]
    fn matrix_transforms_the_four_corners() {
        let t = crate::toks::testkit::machine();
        t.v_pdfship.page_mode = true;
        assert!(!t.matrixused());
        // Without a matrix the rectangle comes back as it is.
        t.matrixtransformrect(1, 2, 3, 4);
        assert_eq!((t.getllx(), t.getlly(), t.geturx(), t.getury()), (1, 2, 3, 4));
        // Quarter turn: (x, y) to (-y, x).
        set_matrix(t, b"0 1 -1 0");
        assert!(t.matrixused());
        t.matrixtransformrect(10, 20, 30, 50);
        assert_eq!((t.getllx(), t.getlly(), t.geturx(), t.getury()), (-50, 10, -20, 30));
        t.matrixrecalculate(40);
        assert_eq!((t.getllx(), t.getlly(), t.geturx(), t.getury()), (-50, 10, -20, 40));
        // Half away from zero, in doubles, and past 2^31 it clamps.  \pdfsave and \pdfrestore restore the stack.
        t.checkpdfsave(0, 0);
        t.v_pdfship.matrix_stack.len = 0;
        set_matrix(t, b"0.5 0 0 0.5");
        t.matrixtransformrect(3, -3, 5, -5);
        assert_eq!((t.getllx(), t.getlly(), t.geturx(), t.getury()), (2, -3, 3, -2));
        set_matrix(t, b"2000000 0 0 2");
        t.matrixtransformrect(1000000, 0, 1000000, 0);
        assert_eq!((t.getllx(), t.geturx()), (2147483647, 2147483647));
        // Outside a page the stack is not used, though it is still there.
        t.v_pdfship.page_mode = false;
        assert!(t.matrixused());
        t.matrixtransformrect(3, -3, 5, -5);
        assert_eq!((t.getllx(), t.getlly(), t.geturx(), t.getury()), (3, -3, 5, -5));
        Tex::dealloc(t);
    }

    #[test]
    fn the_image_table_keeps_each_entry_and_its_name() {
        let t = crate::toks::testkit::machine();
        let a = t.ext_new_image_entry();
        let b = t.ext_new_image_entry();
        assert_eq!((a, b), (0, 1));
        t.ext_set_img_name(a, b"./one.png");
        t.ext_set_img_name(b, b"/x/two.pdf");
        let mut e = t.ext_img(b);
        e.image_type = IMAGE_TYPE_PDF;
        e.width = 5_000_000_000;
        e.height = -7;
        e.rotate = 90;
        e.group_ref = -1;
        t.ext_set_img(b, e);
        t.set_image_group_ref(a, 12);
        assert_eq!(t.ext_img_name(a), b"./one.png");
        assert_eq!(t.ext_img_name(b), b"/x/two.pdf");
        assert_eq!(t.get_image_group_ref(a), 12);
        assert_eq!(t.get_image_group_ref(b), -1);
        assert!(t.is_pdf_image(b));
        assert!(!t.is_pdf_image(a));
        assert_eq!(t.image_width(b), 2147483647, "past 2^31 a caller of image_width sees the clamp");
        assert_eq!(t.image_height(b), -7);
        assert_eq!(t.image_rotate(b), 90);
        assert!(!t.ext_is_loaded(b), "the table says nothing of the host's side");
        t.ext_mark_loaded(b);
        assert!(t.ext_is_loaded(b));
        assert!(!t.ext_is_loaded(a));
        Tex::dealloc(t);
    }

    #[test]
    fn image_procset_bits_are_the_c_ones() {
        let t = crate::toks::testkit::machine();
        assert!(t.check_image_b(1) && !t.check_image_c(1) && !t.check_image_i(1));
        assert!(!t.check_image_b(6) && t.check_image_c(6) && t.check_image_i(6));
        let a = t.ext_new_image_entry();
        let mut e = t.ext_img(a);
        e.color_type = 6;
        t.ext_set_img(a, e);
        t.pdf_image_procset = 1;
        t.update_image_procset(a);
        assert_eq!(t.pdf_image_procset, 7);
        Tex::dealloc(t);
    }

    #[test]
    fn make_cstring_stops_at_a_nul() {
        let t = crate::toks::testkit::machine();
        t.str_ptr = 300;
        t.pool_ptr = 1000;
        t.str_start[300] = 1000;
        for &c in b"a\"b\0c" {
            t.append_char(c as i32);
        }
        t.str_ptr += 1;
        t.str_start[t.str_ptr] = t.pool_ptr;
        assert_eq!(t.ext_make_cstring(300), b"a\"b");
        Tex::dealloc(t);
    }

    #[test]
    fn flush_str_flushes_only_the_last_string() {
        let t = crate::toks::testkit::machine();
        t.str_ptr = 300;
        t.pool_ptr = 1000;
        t.str_start[300] = 1000;
        t.append_char(b'a' as i32);
        t.append_char(b'b' as i32);
        t.str_ptr += 1;
        t.str_start[t.str_ptr] = t.pool_ptr;
        t.append_char(b'c' as i32);
        t.str_ptr += 1;
        t.str_start[t.str_ptr] = t.pool_ptr;
        t.flush_str(300);
        assert_eq!((t.str_ptr, t.pool_ptr), (302, 1003), "not the last one");
        t.flush_str(301);
        assert_eq!((t.str_ptr, t.pool_ptr), (301, 1002));
        t.flush_str(300);
        assert_eq!((t.str_ptr, t.pool_ptr), (300, 1000));
        Tex::dealloc(t);
    }

    #[test]
    fn new_whatsit_links_at_the_tail() {
        let t = mem_machine();
        let h = t.get_node(SMALL_NODE_SIZE);
        t.set_link(h, NULL);
        t.set_tail(h);
        t.new_whatsit(PDF_SAVE_NODE, PDF_SAVE_NODE_SIZE);
        let p = t.tail();
        assert_ne!(p, h);
        assert_eq!(t.link(h), p);
        assert_eq!((t.ty(p), t.subtype(p)), (WHATSIT_NODE, PDF_SAVE_NODE));
        Tex::dealloc(t);
    }

    /// The stream as `\\showbox` prints it: a number, `*` for 16 and `-` for anything above it.
    #[test]
    fn print_write_whatsit_shows_the_stream() {
        use crate::toks::testkit::printed;
        let t = mem_machine();
        t.selector = TERM_ONLY;
        t.max_print_line = 1000;
        t.set_escape_char(-1);
        let p = t.get_node(WRITE_NODE_SIZE);
        for (j, want) in [(0, "write0"), (15, "write15"), (16, "write*"), (17, "write-"), (18, "write-")] {
            t.set_write_stream(p, j);
            let _ = printed(t);
            t.print_write_whatsit(b"write", p);
            assert_eq!(printed(t), want);
        }
        Tex::dealloc(t);
    }

    /// A \\pdfsnapy with neither way in reach does nothing, and do_snapy takes the final skip when there is one.
    #[test]
    fn do_snapy_uses_final_skip_or_the_gap() {
        let t = mem_machine();
        let p = snap_node(t, 100, 50, 50);
        t.pdf_snapy_refpos = 0;
        t.cur_v = 130;
        t.do_snapy(p);
        assert_eq!((t.cur_v, t.count_do_snapy), (100, 1), "back to the line");
        t.cur_v = 170;
        t.do_snapy(p);
        assert_eq!(t.cur_v, 200, "on to the line");
        t.cur_v = 150;
        t.do_snapy(p);
        assert_eq!(t.cur_v, 150, "out of reach");
        t.set_final_skip(p, 7);
        t.cur_v = 130;
        t.do_snapy(p);
        assert_eq!(t.cur_v, 137);
        Tex::dealloc(t);
    }

    #[test]
    fn prev_rightmost_finds_the_node_before() {
        let t = mem_machine();
        let (a, b, c) = (t.get_node(SMALL_NODE_SIZE), t.get_node(SMALL_NODE_SIZE), t.get_node(SMALL_NODE_SIZE));
        t.set_link(a, b);
        t.set_link(b, c);
        t.set_link(c, NULL);
        assert_eq!(t.prev_rightmost(a, c), b);
        assert_eq!(t.prev_rightmost(a, b), a);
        assert_eq!(t.prev_rightmost(b, a), NULL, "e is not after s");
        assert_eq!(t.prev_rightmost(NULL, a), NULL);
        assert_eq!(t.prev_rightmost(a, NULL), c, "as WEB's loop: the last node's link is null");
        Tex::dealloc(t);
    }

    fn outline(t: &mut Tex, k: i32, m: i32, prev: i32, next: i32, first: i32, count: i32) {
        t.set_obj_outline_ptr(k, m);
        t.set_obj_outline_prev(k, prev);
        t.set_obj_outline_next(k, next);
        t.set_obj_outline_first(k, first);
        t.set_obj_outline_last(k, 0);
        t.set_obj_outline_count(k, count);
    }

    /// open_subentries and outline_list_count on a tree of pdf_mem records: an open entry counts its open descendants,
    /// a closed one does not pass its own on, and its count goes negative.
    #[test]
    fn outline_counts() {
        let t = crate::toks::testkit::machine();
        // 1 (open) with children 2 (closed, child 4) and 3; 5 follows 1 at the top.
        let m = |k: i32| k * PDFMEM_OUTLINE_SIZE + 1;
        outline(t, 1, m(1), 0, 5, 2, 1);
        outline(t, 2, m(2), 0, 3, 4, -1);
        outline(t, 3, m(3), 2, 0, 0, 0);
        outline(t, 4, m(4), 0, 0, 0, 0);
        outline(t, 5, m(5), 1, 0, 0, 0);
        assert_eq!(t.outline_list_count(5), 2);
        assert_eq!(t.outline_list_count(3), 2);
        assert_eq!(t.outline_list_count(4), 1);
        assert_eq!(t.open_subentries(1), 2);
        assert_eq!(t.obj_outline_count(1), 2);
        assert_eq!(t.obj_outline_count(2), -1, "closed");
        assert_eq!(t.obj_outline_count(4), 0);
        assert_eq!(t.obj_outline_parent(2), 1);
        assert_eq!(t.obj_outline_parent(4), 2);
        assert_eq!(t.obj_outline_last(1), 3);
        Tex::dealloc(t);
    }

    fn snap_node(t: &mut Tex, unit: i32, stretch: i32, shrink: i32) -> i32 {
        let g = t.get_node(GLUE_SPEC_SIZE);
        t.set_stretch_order(g, NORMAL);
        t.set_shrink_order(g, NORMAL);
        t.set_glue_ref_count(g, NULL);
        t.set_width(g, unit);
        t.set_stretch(g, stretch);
        t.set_shrink(g, shrink);
        let p = t.get_node(SNAP_NODE_SIZE);
        t.set_ty(p, WHATSIT_NODE);
        t.set_subtype(p, PDF_SNAPY_NODE);
        t.set_snap_glue_ptr(p, g);
        t.set_final_skip(p, 0);
        p
    }

    #[test]
    fn gap_amount_goes_to_the_nearer_grid_line_it_may_reach() {
        let t = mem_machine();
        let p = snap_node(t, 100, 50, 50);
        t.pdf_snapy_refpos = 10;
        assert_eq!(t.gap_amount(p, 130), -20, "back: 20 up to the line, 80 down");
        assert_eq!(t.gap_amount(p, 190), 20, "forward");
        assert_eq!(t.gap_amount(p, 160), 0, "unable to snap: 50 either way is not below the limits");
        assert_eq!(t.gap_amount(p, 110), 0, "on a line, moving forward 100 is out of reach and back 0 is -0");
        Tex::dealloc(t);
    }

    #[test]
    fn get_vpos_adds_what_lies_between() {
        let t = mem_machine();
        let b = t.get_node(BOX_NODE_SIZE);
        t.set_glue_order(b, NORMAL);
        t.set_glue_sign(b, NORMAL);
        t.set_glue_set(b, 0.0);
        let k = t.get_node(SMALL_NODE_SIZE);
        t.set_ty(k, KERN_NODE);
        t.set_width(k, 100);
        let r = t.get_node(RULE_NODE_SIZE);
        t.set_ty(r, RULE_NODE);
        t.set_height(r, 20);
        t.set_depth(r, 3);
        let q = t.get_node(SMALL_NODE_SIZE);
        t.set_ty(q, WHATSIT_NODE);
        t.set_subtype(q, PDF_SNAPY_NODE);
        t.set_link(k, r);
        t.set_link(r, q);
        t.set_link(q, NULL);
        t.cur_v = 1000;
        assert_eq!(t.get_vpos(k, q, b), 1123);
        assert_eq!(t.get_vpos(k, NULL, b), 1123, "to the end of the list");
        Tex::dealloc(t);
    }
}
