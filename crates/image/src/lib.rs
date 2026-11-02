//! The image readers of rstex: PNG, JPEG, JBIG2 and PDF inclusion.  This is writeimg.c's helpers, writepng.c (with as
//! much of libpng 1.6.59 as they reach), writejpg.c, writejbig2.c and pdftoepdf.cc (with as much of xpdf 4.06), for the
//! engine's `ext.rs` and `pdfship.rs` to call.  It never calls back into the engine and holds no global state.
//!
//! # Output: a list of operations
//!
//! A C writer calls `pdf_puts`, `pdfbeginstream`, `pdfnewobjnum`, ... as it goes.  Here it returns those calls as a
//! `Vec<out::Op>`, which the engine replays right away:
//!
//! ```text
//! for op in ops {
//!     match op {
//!         Op::Puts(b)               => pdf_out_bytes(&b),               // through the compressor in stream mode
//!         Op::CopyInput{start, len} => pdf_out_bytes(&file[start..start + len]),
//!         Op::NewObj(s)             => table[s] = pdf_new_objnum(),     // table indexed by Slot, grown to slots.len()
//!         Op::Num(s)                => pdf_print_int(table[s]),
//!         Op::BeginDict{obj, level} => pdf_begin_dict(number(obj), level),
//!         Op::BeginObj{obj, level}  => pdf_begin_obj(number(obj), level),
//!         Op::EndDict => pdf_end_dict(),          Op::EndObj => pdf_end_obj(),
//!         Op::BeginStream => pdf_begin_stream(),  Op::EndStream => pdf_end_stream(),
//!         Op::Flush => pdf_flush(),
//!         Op::TexPrint(b) => tex_printf,          Op::Warn(b) => pdftex_warn,
//!         Op::EpdfFont{..} | Op::EpdfFontName{..} => see those variants,
//!     }
//! }
//! ```
//!
//! Object numbers are slots because the font code allocates some of them in the engine: only the replay knows the
//! numbers, and it allocates them in the C's order.  One `out::SlotAlloc` and one table serve the whole run.
//!
//! A `pdftex_fail` is `Err(Fail(message))`, the message as the C formats it; the engine adds its prefix and stops.  The
//! ops of a failed call are dropped, since the run ends there anyway.  `pdftex_warn` and `tex_printf` keep their place in
//! the list, except `read_pdf_info`'s warning, which comes back in `epdf::PdfInfo`.
//!
//! All writers expect the engine to have called `pdf_begin_dict(n, 0)` and printed `obj_ximage_attr` first, as
//! `pdf_write_image` does (§778).
//!
//! # Entry points, by the C function each replaces
//!
//! | C | Rust |
//! |---|---|
//! | `checktypebyheader`, `checktypebyextension` | `check_type_by_header`, `check_type_by_extension` |
//! | `read_png_info` | `png::read_png_info`; the page-group part is `png::PngInfo::wants_page_group`, in the engine |
//! | `write_png` | `png::write_png` |
//! | `write_additional_png_objects` | the engine's test on `png::PngOutput::needs_page_group`, then `png::png_page_group_object` |
//! | `read_jpg_info`, `write_jpg` | `jpg::read_jpg_info`, `jpg::write_jpg` |
//! | `rd_jbig2_info`, `read_jbig2_info` | `jbig2::rd_jbig2_info`, once per file name, then `jbig2::read_jbig2_info` |
//! | `write_jbig2` | `jbig2::write_jbig2` |
//! | `flushjbig2page0objects` | `jbig2::flush_page0`, for each file in `strcmp` order of the names |
//! | `find_add_document`, `delete_document` | `epdf::EpdfDocs::find_document`, `add_document`, `epdf_delete` |
//! | `read_pdf_info` | `epdf::EpdfDoc::read_pdf_info`, which also does what `readimage` does with `epdf_width` etc. |
//! | `lookup_fontmap` in `copyFont` | `epdf::EpdfDoc::epdf_font_queries`, then `EpdfParams::fontmap_hits` |
//! | `write_epdf` | `epdf::EpdfDoc::write_epdf`; `Op::CopyInput` refers to `EpdfDoc::data()` |
//! | `epdf_delete` | `epdf::EpdfDocs::epdf_delete` |
//! | `imagecolordepth` | `PngInfo::bit_depth`, `JpgInfo::bits_per_component`; 0 for JBIG2 and PDF |
//! | `img_color` | `PngInfo::img_color`, `JpgInfo::img_color`; 0 for JBIG2 and PDF |
//!
//! `writeimage`'s `tex_printf(" <%s", img_name)` and `">"` around the writer are the engine's.  So is
//! `getenv("TEXMF_DEBUG_PNG_COPY")`, whose only effect is a message: it is not ported.
//!
//! `read_image`, `write_image` and the image table themselves are the engine's, in `ext.rs`, and the file reading goes
//! through its World: this crate gets byte slices.  `writeimg.c`'s `epdf_*` values are `f32` in the C, and so here, as
//! `docs/PORTING.md` asks: `epdf::EpdfDoc::read_pdf_info` converts through `f32` to the final integers.
//!
//! The oracle's PDF library is xpdf 4.06, built in (see `pdf.rs`), and its PNG library is the system's libpng 1.6.59
//! (see `png.rs`).  Differences from them are listed at the top of those two files.
#![forbid(unsafe_code)]

mod enc_tables;
pub mod epdf;
mod filters;
pub mod jbig2;
pub mod jpg;
pub mod out;
pub mod pdf;
mod pdffont;
pub mod png;

/// `pdftex_fail`: the message, as the C formats it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fail(pub Vec<u8>);

impl Fail {
    pub(crate) fn new(s: &str) -> Fail {
        Fail(s.as_bytes().to_vec())
    }
}

impl std::fmt::Display for Fail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&String::from_utf8_lossy(&self.0))
    }
}

/// `IMAGE_COLOR_B`, for `\pdfimageprocset`'s `/ImageB`.
pub const IMAGE_COLOR_B: i32 = 1;
/// `IMAGE_COLOR_C`.
pub const IMAGE_COLOR_C: i32 = 2;
/// `IMAGE_COLOR_I`.
pub const IMAGE_COLOR_I: i32 = 4;

/// `IMAGE_TYPE_NONE` and the others.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageType {
    None,
    Pdf,
    Png,
    Jpg,
    Jbig2,
}

/// `checktypebyheader`.  The C fails with "reading image file failed" on a file shorter than 8 bytes; the engine tests
/// the length, since it holds the file.
pub fn check_type_by_header(header: &[u8]) -> ImageType {
    if header.starts_with(b"\xFF\xD8") {
        ImageType::Jpg
    } else if header.starts_with(b"\x89PNG\r\n\x1A\n") {
        ImageType::Png
    } else if header.starts_with(b"\x97\x4A\x42\x32\x0D\x0A\x1A\x0A") {
        ImageType::Jbig2
    } else if header.starts_with(b"%PDF-1.") {
        ImageType::Pdf
    } else {
        ImageType::None
    }
}

/// `checktypebyextension`, on `cur_file_name`.
pub fn check_type_by_extension(name: &[u8]) -> ImageType {
    let Some(i) = name.iter().rposition(|&c| c == b'.') else {
        return ImageType::None;
    };
    let suffix = name[i..].to_ascii_lowercase();
    match suffix.as_slice() {
        b".png" => ImageType::Png,
        b".jpg" | b".jpeg" => ImageType::Jpg,
        b".jbig2" | b".jb2" => ImageType::Jbig2,
        b".pdf" => ImageType::Pdf,
        _ => ImageType::None,
    }
}

/// web2c's `zround`, called `round` in the C of this crate's sources.
pub(crate) fn zround(r: f64) -> i32 {
    if r > 2147483647.0 {
        2147483647
    } else if r < -2147483647.0 {
        -2147483647
    } else if r >= 0.0 {
        (r + 0.5) as i32
    } else {
        (r - 0.5) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_types() {
        assert_eq!(check_type_by_header(b"\xFF\xD8\xFF\xE0xxxx"), ImageType::Jpg);
        assert_eq!(check_type_by_header(b"\x89PNG\r\n\x1A\nxx"), ImageType::Png);
        assert_eq!(check_type_by_header(b"\x97JB2\r\n\x1A\n"), ImageType::Jbig2);
        assert_eq!(check_type_by_header(b"%PDF-1.5"), ImageType::Pdf);
        assert_eq!(check_type_by_header(b"%PDF-2.0"), ImageType::None);
        assert_eq!(check_type_by_extension(b"a.b/c.PNG"), ImageType::Png);
        assert_eq!(check_type_by_extension(b"c.JpEg"), ImageType::Jpg);
        assert_eq!(check_type_by_extension(b"c.jb2"), ImageType::Jbig2);
        assert_eq!(check_type_by_extension(b"c.pdf"), ImageType::Pdf);
        assert_eq!(check_type_by_extension(b"c.txt"), ImageType::None);
        assert_eq!(check_type_by_extension(b"c"), ImageType::None);
    }

    #[test]
    fn rounding() {
        assert_eq!(zround(1.5), 2);
        assert_eq!(zround(-1.5), -2);
        assert_eq!(zround(0.4), 0);
        assert_eq!(zround(3e10), 2147483647);
        assert_eq!(zround(-3e10), -2147483647);
    }
}
