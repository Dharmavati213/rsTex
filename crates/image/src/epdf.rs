//! pdftoepdf.cc: `read_pdf_info` and `write_epdf`, the copy of a page of another PDF into a form XObject.
//!
//! The C keeps a linked list of open documents.  Here `EpdfDocs` is that list and `EpdfDoc` one record, with the list of
//! copied objects, which lives as long as the document does, so a second page of the same file shares what the first
//! copied.  `find_add_document`'s counting of `occurences` decides when a document goes, and with it those numbers.
//!
//! `copyFont` asks the engine's font map about each font it might replace.  `epdf_font_queries` lists the names to ask
//! about before `write_epdf` runs, and `write_epdf` takes the answers.  The objects the engine then has to set up come
//! out as `Op::EpdfFont` and `Op::EpdfFontName`.

use crate::out::{ObjNo, Op, Slot, SlotAlloc, Writer};
use crate::pdf::{Dict, Doc, Object, OpenError, Page};
use crate::pdffont::make_font_enc;
use crate::{Fail, zround};
use std::collections::HashMap;
use std::sync::Arc;

/// `pdfboxspecmedia` and friends: the `\pdfpagebox` values.
pub const PDF_BOX_SPEC_MEDIA: i32 = 1;
pub const PDF_BOX_SPEC_CROP: i32 = 2;
pub const PDF_BOX_SPEC_BLEED: i32 = 3;
pub const PDF_BOX_SPEC_TRIM: i32 = 4;
pub const PDF_BOX_SPEC_ART: i32 = 5;

const MASK_SUPPRESS_PTEX_FILENAME: i32 = 0x02;
const MASK_SUPPRESS_PTEX_PAGENUMBER: i32 = 0x04;
const MASK_SUPPRESS_PTEX_INFODICT: i32 = 0x08;

/// `one_hundred_bp` over 100.
const BP2INT: f64 = 6578176.0 / 100.0;

/// `InObjType`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum InObjType {
    Font,
    FontDesc,
    Other,
}

/// `InObj`.
#[derive(Clone, Debug)]
struct InObj {
    r: (i32, i32),
    ty: InObjType,
    /// `num`, the number in the output.
    slot: Slot,
    /// `fd`, by the slot of its object number.
    fd: Slot,
    enc: Slot,
    written: bool,
}

/// `PdfDocument`.
pub struct EpdfDoc {
    file_name: Vec<u8>,
    doc: Doc,
    in_obj_list: InObjList,
    occurences: i32,
}

/// `inObjList`: the C searches its linked list from the head for each reference, so it has a table beside it here.
#[derive(Default)]
struct InObjList {
    list: Vec<InObj>,
    index: HashMap<(i32, i32), usize>,
}

impl InObjList {
    fn find(&self, r: (i32, i32)) -> Option<&InObj> {
        self.index.get(&r).map(|&i| &self.list[i])
    }
}

/// `pdfDocuments`, the open embedded PDF files.  An index stays good until `epdf_delete` frees it: it is what the image
/// entry keeps, for the C's `pdf_ptr(img)->doc`.
#[derive(Default)]
pub struct EpdfDocs {
    docs: Vec<Option<EpdfDoc>>,
}

/// The inputs of `read_pdf_info`.
#[derive(Clone, Copy, Debug)]
pub struct PdfInfoParams {
    /// `page_num`.
    pub page_num: i32,
    /// `pagebox_spec`.
    pub page_box: i32,
    /// `major_pdf_version_wanted`.
    pub major_wanted: i32,
    /// `minor_pdf_version_wanted`.
    pub minor_wanted: i32,
    /// `pdf_inclusion_errorlevel`.
    pub inclusion_errorlevel: i32,
}

/// What `read_pdf_info` returns and `readimage` does with `epdf_width` and the rest.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfInfo {
    /// The return value: the page, by number.
    pub selected_page: i32,
    /// `img_width(img) = bp2int(epdf_width)`.
    pub width: i32,
    /// `img_height`.
    pub height: i32,
    /// `img_rotate`.
    pub rotate: i32,
    /// `img_pages`.
    pub pages: i32,
    /// `pdf_ptr(img)->orig_x`.
    pub orig_x: i32,
    /// `pdf_ptr(img)->orig_y`.
    pub orig_y: i32,
    /// `epdf_has_page_group`: `img_group_ref(img)` is -1 if set, and 0 if not.
    pub has_page_group: bool,
    /// `pdftex_warn` from the version check.
    pub warning: Option<Vec<u8>>,
}

/// `bp2int`.
fn bp2int(p: f32) -> i32 {
    zround(p as f64 * BP2INT)
}

/// `get_pagebox`.
fn get_pagebox(page: &Page, spec: i32) -> Result<[f64; 4], Fail> {
    match spec {
        PDF_BOX_SPEC_MEDIA => Ok(page.attrs.media_box),
        PDF_BOX_SPEC_CROP => Ok(page.attrs.crop_box),
        PDF_BOX_SPEC_BLEED => Ok(page.attrs.bleed_box),
        PDF_BOX_SPEC_TRIM => Ok(page.attrs.trim_box),
        PDF_BOX_SPEC_ART => Ok(page.attrs.art_box),
        n => Err(Fail(format!("PDF inclusion: unknown value of pagebox spec ({n})").into_bytes())),
    }
}

impl EpdfDocs {
    /// The first half of `find_add_document`: an open document under this name counts one more reference and its index
    /// comes back.  If there is none, the engine reads the file and calls `add_document`.
    pub fn find_document(&mut self, file_name: &[u8]) -> Option<usize> {
        let i = self.docs.iter().position(|d| d.as_ref().is_some_and(|d| d.file_name == file_name))?;
        self.docs[i].as_mut().unwrap().occurences += 1;
        Some(i)
    }

    /// The second half of `find_add_document`.  `file_name` is `img_name`, as kpathsea found it: it goes in
    /// `/PTEX.FileName`.
    pub fn add_document(&mut self, file_name: &[u8], data: Vec<u8>) -> Result<usize, Fail> {
        let doc = Doc::open(data).map_err(|_: OpenError| Fail::new("xpdf: reading PDF image failed"))?;
        let rec =
            EpdfDoc { file_name: file_name.to_vec(), doc, in_obj_list: InObjList::default(), occurences: 0 };
        self.docs.push(Some(rec));
        Ok(self.docs.len() - 1)
    }

    /// `epdf_delete`.  Returns whether the document went, which it does once no image still to be written refers to it.
    /// The engine does not call this in INITEX, as `deleteimage` does not.
    pub fn epdf_delete(&mut self, i: usize) -> bool {
        if self.docs[i].as_ref().is_some_and(|d| d.occurences < 0) {
            self.docs[i] = None;
            return true;
        }
        false
    }

    /// The document at an index from `find_document` or `add_document`.
    pub fn doc_mut(&mut self, i: usize) -> &mut EpdfDoc {
        self.docs[i].as_mut().expect("a document that epdf_delete freed")
    }
}

impl EpdfDoc {
    /// The file's bytes, which `Op::CopyInput` of `write_epdf` refers to.
    pub fn data(&self) -> &[u8] {
        self.doc.data()
    }

    /// `read_pdf_info`, after `find_add_document`.  `page_name` is the `named` argument.
    pub fn read_pdf_info(&mut self, page_name: Option<&[u8]>, p: &PdfInfoParams) -> Result<PdfInfo, Fail> {
        let doc = &mut self.doc;
        let mut warning = None;
        let found = doc.pdf_version() as f32;
        let wanted = (p.major_wanted as f64 + (p.minor_wanted as f64 * 0.1)) as f32;
        if found as f64 > wanted as f64 + 0.01 {
            let msg = format!(
                "PDF inclusion: found PDF version <{:.1}>, but at most version <{:.1}> allowed",
                found as f64, wanted as f64
            )
            .into_bytes();
            if p.inclusion_errorlevel > 0 {
                return Err(Fail(msg));
            } else if p.inclusion_errorlevel == 0 {
                warning = Some(msg);
            }
        }
        let num_pages = doc.num_pages();
        let mut page_num = p.page_num;
        if let Some(name) = page_name {
            let Some(dest) = doc.find_dest(name) else {
                let mut m = b"PDF inclusion: invalid destination <".to_vec();
                m.extend_from_slice(name);
                m.push(b'>');
                return Err(Fail(m));
            };
            let (num, gno) = dest.unwrap_or((-1, -1));
            page_num = doc.find_page(num, gno);
            if page_num == 0 {
                let mut m = b"PDF inclusion: destination is not a page <".to_vec();
                m.extend_from_slice(name);
                m.push(b'>');
                return Err(Fail(m));
            }
        } else if page_num <= 0 || page_num > num_pages {
            return Err(Fail(format!("PDF inclusion: required page does not exist <{num_pages}>").into_bytes()));
        }
        let page = doc.get_page(page_num);
        let b = get_pagebox(&page, p.page_box)?;
        let (orig_x, width) = if b[2] > b[0] { (b[0] as f32, (b[2] - b[0]) as f32) } else { (b[2] as f32, (b[0] - b[2]) as f32) };
        let (orig_y, height) = if b[3] > b[1] { (b[1] as f32, (b[3] - b[1]) as f32) } else { (b[3] as f32, (b[1] - b[3]) as f32) };
        // epdf_rotate is a float; getRotate() is 0..359.
        let rotate = (page.attrs.rotate % 360) as f32;
        Ok(PdfInfo {
            selected_page: page_num,
            width: bp2int(width),
            height: bp2int(height),
            rotate: rotate as i32,
            pages: num_pages,
            orig_x: bp2int(orig_x),
            orig_y: bp2int(orig_y),
            has_page_group: matches!(page.attrs.group, Object::Dict(_)),
            warning,
        })
    }

    /// The names `copyFont` would pass to `lookup_fontmap` for this page: the engine answers each, and passes the
    /// answers to `write_epdf` in the same order.  Empty under `\pdfinclusioncopyfonts=1`, which the engine knows.
    pub fn epdf_font_queries(&mut self, selected_page: i32) -> Vec<Vec<u8>> {
        let page = self.doc.get_page(selected_page);
        let mut out = Vec::new();
        let Object::Dict(res) = &page.attrs.resources else {
            return out;
        };
        let Object::Dict(fonts) = self.doc.lookup(res, b"Font") else {
            return out;
        };
        for i in 0..fonts.len() {
            if let Object::Ref(n, g) = fonts.val_nf(i) {
                if let Some(c) = font_candidate(&mut self.doc, n, g) {
                    if !out.contains(&c.basefont) {
                        out.push(c.basefont);
                    }
                }
            }
        }
        out
    }

    /// `write_epdf`.  The engine has called `pdf_begin_dict` and printed the attributes.  See `EpdfParams`.
    pub fn write_epdf(&mut self, p: &EpdfParams<'_>, slots: &mut SlotAlloc) -> Result<EpdfOutput, Fail> {
        self.occurences -= 1;
        let mut c = Copier {
            w: Writer::new(slots),
            doc: &mut self.doc,
            list: &mut self.in_obj_list,
            encodings: Vec::new(),
            fm: p.fontmap_hits,
            copy_font: p.inclusion_copy_font,
        };
        let sep: &[u8] = if p.use_underscore { b"_" } else { b"." };
        let page = c.doc.get_page(p.selected_page);
        let page_ref = c.doc.get_page_ref(p.selected_page);
        let page_obj = c.doc.fetch(page_ref.0, page_ref.1, 0);
        let Object::Dict(page_dict) = page_obj else {
            return Err(Fail::new("PDF inclusion: invalid page"));
        };
        let rotate = page.attrs.rotate;
        c.w.puts(b"/Type /XObject\n");
        c.w.puts(b"/Subtype /Form\n");
        c.w.puts(b"/FormType 1\n");
        if p.suppress_ptex_info & MASK_SUPPRESS_PTEX_FILENAME == 0 {
            let name = convert_string_to_pdf_string(&self.file_name)?;
            let mut pre = b"/PTEX".to_vec();
            pre.extend_from_slice(sep);
            pre.extend_from_slice(b"FileName (");
            c.w.printf_bytes(&pre, &name, b")\n");
        }
        if p.suppress_ptex_info & MASK_SUPPRESS_PTEX_PAGENUMBER == 0 {
            c.w.printf(format_args!("/PTEX{}PageNumber {}\n", sep[0] as char, p.selected_page));
        }
        if p.suppress_ptex_info & MASK_SUPPRESS_PTEX_INFODICT == 0 {
            if let Object::Ref(n, g) = c.doc.doc_info_nf() {
                c.w.printf(format_args!("/PTEX{}InfoDict ", sep[0] as char));
                let s = c.add_in_obj(InObjType::Other, (n, g), 0, 0)?;
                c.w.num(s);
                c.w.puts(b" 0 R\n");
            }
        }
        let pagebox = get_pagebox(&page, p.page_box)?;
        if rotate != 0 && rotate % 90 == 0 {
            c.w.op(Op::TexPrint(format!(", page is rotated {rotate} degrees").into_bytes()));
            let mut scale = [0.0f64; 6];
            let mut writematrix = false;
            match rotate {
                90 => {
                    scale[1] = -1.0;
                    scale[2] = 1.0;
                    scale[4] = pagebox[0] - pagebox[1];
                    scale[5] = pagebox[1] + pagebox[2];
                    writematrix = true;
                }
                180 => {
                    scale[0] = -1.0;
                    scale[3] = -1.0;
                    scale[4] = pagebox[0] + pagebox[2];
                    scale[5] = pagebox[1] + pagebox[3];
                    writematrix = true;
                }
                270 => {
                    scale[1] = 1.0;
                    scale[2] = -1.0;
                    scale[4] = pagebox[0] + pagebox[3];
                    scale[5] = pagebox[1] - pagebox[0];
                    writematrix = true;
                }
                _ => {}
            }
            if writematrix {
                let s = format!(
                    "/Matrix [{:.8} {:.8} {:.8} {:.8} {:.8} {:.8}]\n",
                    scale[0], scale[1], scale[2], scale[3], scale[4], scale[5]
                );
                c.w.puts(&stripzeros(s.as_bytes()));
            }
        }
        let s = format!("/BBox [{:.8} {:.8} {:.8} {:.8}]\n", pagebox[0], pagebox[1], pagebox[2], pagebox[3]);
        c.w.puts(&stripzeros(s.as_bytes()));

        let meta = page_dict.get_nf(b"Metadata");
        if !meta.is_null() && !matches!(meta, Object::Ref(..)) {
            c.w.op(Op::Warn(b"PDF inclusion: /Metadata must be indirect object".to_vec()));
        }
        for key in [&b"LastModified"[..], b"Metadata", b"PieceInfo", b"SeparationInfo"] {
            let o = page_dict.get_nf(key);
            if !o.is_null() {
                c.w.newline();
                c.w.printf_bytes(b"/", key, b" ");
                c.copy_object(&o)?;
            }
        }
        // handle page group
        let mut page_group_val = p.page_group_val;
        let mut group_dict: Option<Arc<Dict>> = None;
        if !page_dict.get_nf(b"Group").is_null() {
            if page_group_val == 0 {
                if !p.suppress_warning_page_group {
                    c.w.op(Op::Warn(b"PDF inclusion: multiple pdfs with page group included in a single page".to_vec()));
                }
                c.w.newline();
                c.w.puts(b"/Group ");
                let o = page_dict.get_nf(b"Group");
                c.copy_object(&o)?;
            } else {
                let g = c.doc.lookup(&page_dict, b"Group");
                if !matches!(g, Object::Dict(_)) {
                    return Err(Fail::new("PDF inclusion: /Group dict missing"));
                }
                // initDictFromDict(groupDict, page->getGroup())
                match &page.attrs.group {
                    Object::Dict(d) => group_dict = Some(d.clone()),
                    _ => return Err(Fail::new("PDF inclusion: /Group dict missing")),
                }
                c.w.printf(format_args!("/Group {page_group_val} 0 R\n"));
            }
        }
        // write the Resources dictionary
        if let Object::Dict(res) = &page.attrs.resources {
            c.w.newline();
            c.w.puts(b"/Resources <<\n");
            for i in 0..res.len() {
                let v = res.val_nf(i);
                let obj2 = c.doc.resolve(&v, 0);
                let key = res.key(i);
                if key == b"Font" {
                    c.copy_font_resources(&obj2)?;
                } else if key == b"ProcSet" {
                    c.copy_proc_set(&obj2)?;
                } else {
                    c.copy_other_resources(&obj2, key)?;
                }
            }
            c.w.puts(b">>\n");
        } else {
            c.w.op(Op::Warn(
                b"PDF inclusion: /Resources missing. 'This practice is not recommended' (PDF Ref)".to_vec(),
            ));
        }
        // write the page contents
        let contents = c.doc.resolve(&page.contents, 0);
        match &contents {
            Object::Stream(s) => {
                let f = c.doc.lookup(&s.dict, b"F");
                if !f.is_null() {
                    return Err(Fail::new("PDF inclusion: Unsupported external stream"));
                }
                c.w.puts(b"/Length ");
                let o = c.doc.lookup(&s.dict, b"Length");
                if o.is_null() {
                    return Err(Fail::new("PDF inclusion: content stream without /Length")); // an assert in the C
                }
                c.copy_object(&o)?;
                c.w.puts(b"\n");
                let o = c.doc.lookup(&s.dict, b"Filter");
                if !o.is_null() {
                    c.w.puts(b"/Filter ");
                    c.copy_object(&o)?;
                    c.w.puts(b"\n");
                    let o = c.doc.lookup(&s.dict, b"DecodeParms");
                    if !o.is_null() {
                        c.w.puts(b"/DecodeParms ");
                        c.copy_object(&o)?;
                        c.w.puts(b"\n");
                    }
                }
                c.w.puts(b">>\nstream\n");
                c.copy_raw_stream(s);
                c.w.op(Op::EndStream);
            }
            Object::Array(a) => {
                c.w.op(Op::BeginStream);
                let l = a.len();
                for i in 0..l {
                    let e = c.doc.array_get(a, i);
                    // A non-stream here is a null pointer in the C.
                    let Object::Stream(s) = e else {
                        return Err(Fail::new("PDF inclusion: invalid contents array"));
                    };
                    let d = c.doc.decode_stream(&s, 0);
                    if let Some(name) = d.unsupported {
                        let mut m = b"PDF inclusion: content stream filter not supported: ".to_vec();
                        m.extend_from_slice(&name);
                        return Err(Fail(m));
                    }
                    c.w.last_byte = d.data.last().copied().unwrap_or(0);
                    if !d.data.is_empty() {
                        c.w.raw(d.data);
                    }
                    if i + 1 < l {
                        c.w.newline();
                    }
                }
                c.w.op(Op::EndStream);
            }
            _ => {
                c.w.op(Op::BeginStream);
                c.w.op(Op::EndStream);
            }
        }
        // write out all used encodings
        c.write_encodings()?;
        // write the Group dict if needed
        if let Some(d) = group_dict {
            c.w.op(Op::BeginObj { obj: ObjNo::Known(page_group_val), level: 2 });
            let o = Object::Dict(d);
            c.copy_object(&o)?;
            c.w.puts(b"\n");
            c.w.op(Op::EndObj);
            page_group_val = 0;
        }
        c.write_refs()?;
        Ok(EpdfOutput { ops: c.w.ops, page_group_val })
    }
}

/// The inputs of `write_epdf`.
#[derive(Clone, Copy, Debug)]
pub struct EpdfParams<'a> {
    /// `epdf_selected_page`.
    pub selected_page: i32,
    /// `epdf_page_box`.
    pub page_box: i32,
    /// `pdfpagegroupval`.
    pub page_group_val: i32,
    /// `getpdfsuppressptexinfo()`.
    pub suppress_ptex_info: i32,
    /// `getptexuseunderscore()`.
    pub use_underscore: bool,
    /// `getpdfsuppresswarningpagegroup()`.
    pub suppress_warning_page_group: bool,
    /// `fixedinclusioncopyfont`.
    pub inclusion_copy_font: bool,
    /// The names of `epdf_font_queries` for which `lookup_fontmap` gave an entry.
    pub fontmap_hits: &'a [Vec<u8>],
}

/// The result of `write_epdf`.
#[derive(Clone, Debug)]
pub struct EpdfOutput {
    pub ops: Vec<Op>,
    /// `pdfpagegroupval` afterwards: 0 if this image's group took the page's.
    pub page_group_val: i32,
}

/// The tests in `copyFont` up to `lookup_fontmap`.
struct FontCandidate {
    basefont: Vec<u8>,
    desc_ref: (i32, i32),
    stem_v: i32,
    charset: Option<Vec<u8>>,
    font_dict: Arc<Dict>,
}

fn font_candidate(doc: &mut Doc, num: i32, gno: i32) -> Option<FontCandidate> {
    let Object::Dict(font_dict) = doc.fetch(num, gno, 0) else {
        return None;
    };
    if !doc.lookup(&font_dict, b"Subtype").is_name(b"Type1") {
        return None;
    }
    let Object::Name(basefont) = doc.lookup(&font_dict, b"BaseFont") else {
        return None;
    };
    let Object::Ref(dn, dg) = font_dict.get_nf(b"FontDescriptor") else {
        return None;
    };
    let Object::Dict(desc) = doc.fetch(dn, dg, 0) else {
        return None;
    };
    let has_file = if matches!(doc.lookup(&desc, b"FontFile"), Object::Stream(_)) {
        true
    } else if let Object::Stream(s) = doc.lookup(&desc, b"FontFile3") {
        doc.lookup(&s.dict, b"Subtype").is_name(b"Type1C")
    } else {
        false
    };
    if !has_file {
        return None;
    }
    // round /StemV value, since the PDF input is a float.  With no /StemV the C reads an uninitialised double.
    let stem_v = zround(doc.lookup(&desc, b"StemV").as_num().unwrap_or(0.0));
    let charset = match doc.lookup(&desc, b"CharSet") {
        Object::Str(s) => Some(s.as_ref().clone()),
        _ => None,
    };
    Some(FontCandidate { basefont: basefont.as_ref().clone(), desc_ref: (dn, dg), stem_v, charset, font_dict })
}

struct UsedEncoding {
    enc: Slot,
    is_cid: bool,
    names: Vec<Option<Vec<u8>>>,
}

struct Copier<'a, 's> {
    w: Writer<'s>,
    doc: &'a mut Doc,
    list: &'a mut InObjList,
    encodings: Vec<UsedEncoding>,
    fm: &'a [Vec<u8>],
    copy_font: bool,
}

impl Copier<'_, '_> {
    /// `addInObj`.  For `FontDesc`, `fd` is the slot that already holds the descriptor's number.
    fn add_in_obj(&mut self, ty: InObjType, r: (i32, i32), fd: Slot, enc: Slot) -> Result<Slot, Fail> {
        if r.0 == 0 {
            return Err(Fail::new("PDF inclusion: invalid reference"));
        }
        if let Some(p) = self.list.find(r) {
            return Ok(p.slot);
        }
        let slot = if ty == InObjType::FontDesc { fd } else { self.w.new_obj() };
        self.list.index.insert(r, self.list.list.len());
        self.list.list.push(InObj { r, ty, slot, fd, enc, written: false });
        Ok(slot)
    }

    /// `copyName`.
    fn copy_name(&mut self, s: &[u8]) {
        self.w.puts(b"/");
        for &c in s {
            if c.is_ascii_digit() || c.is_ascii_uppercase() || c.is_ascii_lowercase() || matches!(c, b'_' | b'.' | b'-' | b'+') {
                self.w.out(c);
            } else {
                self.w.printf(format_args!("#{c:02X}"));
            }
        }
    }

    /// `copyDictEntry`.
    fn copy_dict_entry(&mut self, d: &Dict, i: usize) -> Result<(), Fail> {
        self.copy_name(d.key(i));
        self.w.puts(b" ");
        let v = d.val_nf(i);
        self.copy_object(&v)?;
        self.w.puts(b"\n");
        Ok(())
    }

    /// `copyDict`.
    fn copy_dict(&mut self, d: &Dict) -> Result<(), Fail> {
        for i in 0..d.len() {
            self.copy_dict_entry(d, i)?;
        }
        Ok(())
    }

    /// `copyFontDict`.
    fn copy_font_dict(&mut self, d: &Dict, r: &InObj) -> Result<(), Fail> {
        self.w.puts(b"<<\n");
        for i in 0..d.len() {
            let key = d.key(i);
            if key.starts_with(b"FontDescriptor") || key.starts_with(b"BaseFont") || key.starts_with(b"Encoding") {
                continue;
            }
            self.copy_dict_entry(d, i)?;
        }
        self.w.puts(b"/FontDescriptor ");
        self.w.num(r.fd);
        self.w.puts(b" 0 R\n");
        let fn_slot = self.w.slots.next();
        self.w.op(Op::EpdfFontName { fd_slot: r.fd, fn_slot });
        self.w.puts(b"/BaseFont ");
        self.w.num(fn_slot);
        self.w.puts(b" 0 R\n");
        self.w.puts(b"/Encoding ");
        self.w.num(r.enc);
        self.w.puts(b" 0 R\n");
        self.w.puts(b">>");
        Ok(())
    }

    /// `copyStream` of `getUndecodedStream`.
    fn copy_raw_stream(&mut self, s: &crate::pdf::StreamObj) {
        if let Some(d) = self.doc.decrypted_raw(s) {
            self.w.last_byte = d.last().copied().unwrap_or(0);
            if !d.is_empty() {
                self.w.raw(d);
            }
            return;
        }
        let (start, n) = self.doc.raw_range(s);
        self.w.last_byte = if n > 0 { self.doc.data()[start + n - 1] } else { 0 };
        if n > 0 {
            self.w.copy_input(start, n);
        }
    }

    /// `copyProcSet`.
    fn copy_proc_set(&mut self, obj: &Object) -> Result<(), Fail> {
        let Object::Array(a) = obj else {
            return Err(Fail(format!("PDF inclusion: invalid ProcSet array type <{}>", obj.type_name()).into_bytes()));
        };
        self.w.puts(b"/ProcSet [ ");
        for o in a.iter() {
            let Object::Name(n) = o else {
                return Err(Fail(format!("PDF inclusion: invalid ProcSet entry type <{}>", o.type_name()).into_bytes()));
            };
            self.copy_name(n);
            self.w.puts(b" ");
        }
        self.w.puts(b"]\n");
        Ok(())
    }

    /// `copyFont`.
    fn copy_font(&mut self, tag: &[u8], font_ref: (i32, i32)) -> Result<(), Fail> {
        if let Some(p) = self.list.find(font_ref) {
            let slot = p.slot;
            self.copy_name(tag);
            self.w.puts(b" ");
            self.w.num(slot);
            self.w.puts(b" 0 R ");
            return Ok(());
        }
        let cand = if self.copy_font { None } else { font_candidate(self.doc, font_ref.0, font_ref.1) };
        if let Some(c) = cand.filter(|c| self.fm.contains(&c.basefont)) {
            let fd_slot = self.w.slots.next();
            self.w.op(Op::EpdfFont {
                fd_slot,
                basefont: c.basefont.clone(),
                stem_v: c.stem_v,
                charset: c.charset.clone(),
            });
            self.add_in_obj(InObjType::FontDesc, c.desc_ref, fd_slot, 0)?;
            self.copy_name(tag);
            let e = make_font_enc(self.doc, &c.font_dict);
            // addEncoding, the argument of addFont, comes first.
            let enc = self.w.new_obj();
            self.encodings.push(UsedEncoding { enc, is_cid: e.is_cid, names: e.names });
            let font = self.add_in_obj(InObjType::Font, font_ref, fd_slot, enc)?;
            self.w.puts(b" ");
            self.w.num(font);
            self.w.puts(b" 0 R ");
        } else {
            self.copy_name(tag);
            self.w.puts(b" ");
            self.copy_object(&Object::Ref(font_ref.0, font_ref.1))?;
        }
        Ok(())
    }

    /// `copyFontResources`.
    fn copy_font_resources(&mut self, obj: &Object) -> Result<(), Fail> {
        let Object::Dict(d) = obj else {
            return Err(Fail(format!("PDF inclusion: invalid font resources dict type <{}>", obj.type_name()).into_bytes()));
        };
        self.w.puts(b"/Font << ");
        for i in 0..d.len() {
            let f = d.val_nf(i);
            match &f {
                Object::Ref(n, g) => self.copy_font(d.key(i), (*n, *g))?,
                Object::Dict(_) => {
                    // some programs generate pdf with embedded font object
                    self.copy_name(d.key(i));
                    self.w.puts(b" ");
                    self.copy_object(&f)?;
                }
                o => {
                    return Err(Fail(format!("PDF inclusion: invalid font in reference type <{}>", o.type_name()).into_bytes()));
                }
            }
        }
        self.w.puts(b">>\n");
        Ok(())
    }

    /// `copyOtherResources`.
    fn copy_other_resources(&mut self, obj: &Object, key: &[u8]) -> Result<(), Fail> {
        if key == b"Subtype" {
            if !matches!(obj, Object::Name(_)) {
                let mut m = b"PDF inclusion: Subtype in Resources dict is not a name (key '".to_vec();
                m.extend_from_slice(key);
                m.extend_from_slice(format!("', type <{}>); ignored.", obj.type_name()).as_bytes());
                self.w.op(Op::Warn(m));
                return Ok(());
            }
        } else if !matches!(obj, Object::Dict(_)) {
            let mut m = b"PDF inclusion: invalid other resource which is no dict (key '".to_vec();
            m.extend_from_slice(key);
            m.extend_from_slice(format!("', type <{}>); ignored.", obj.type_name()).as_bytes());
            self.w.op(Op::Warn(m));
            return Ok(());
        }
        self.copy_name(key);
        self.w.puts(b" ");
        self.copy_object(obj)
    }

    /// `copyObject`.
    fn copy_object(&mut self, obj: &Object) -> Result<(), Fail> {
        match obj {
            Object::Bool(b) => self.w.puts(if *b { b"true" } else { b"false" }),
            Object::Int(i) => self.w.printf(format_args!("{i}")),
            Object::Real(r) => {
                let s = convert_num_to_pdf(*r);
                self.w.puts(&s);
            }
            Object::Str(s) => self.copy_string(s),
            Object::Name(n) => self.copy_name(n),
            Object::Null => self.w.puts(b"null"),
            Object::Array(a) => {
                self.w.puts(b"[");
                for o in a.iter() {
                    if !matches!(o, Object::Name(_)) {
                        self.w.puts(b" ");
                    }
                    self.copy_object(o)?;
                }
                self.w.puts(b"]");
            }
            Object::Dict(d) => {
                self.w.puts(b"<<\n");
                self.copy_dict(d)?;
                self.w.puts(b">>");
            }
            Object::Stream(s) => {
                self.w.puts(b"<<\n");
                self.copy_dict(&s.dict)?;
                self.w.puts(b">>\n");
                self.w.puts(b"stream\n");
                self.copy_raw_stream(s);
                self.w.puts(b"\nendstream");
            }
            Object::Ref(n, g) => {
                if *n == 0 {
                    return Err(Fail::new("PDF inclusion: reference to invalid object (is the included pdf broken?)"));
                }
                let s = self.add_in_obj(InObjType::Other, (*n, *g), 0, 0)?;
                self.w.num(s);
                self.w.puts(b" 0 R");
            }
            o => {
                return Err(Fail(format!("PDF inclusion: type <{}> cannot be copied", o.type_name()).into_bytes()));
            }
        }
        Ok(())
    }

    /// The string case of `copyObject`.
    fn copy_string(&mut self, s: &[u8]) {
        if !s.contains(&0) {
            self.w.puts(b"(");
            for &c in s {
                if c == b'(' || c == b')' || c == b'\\' {
                    self.w.printf(format_args!("\\{}", c as char));
                } else if c < 0x20 || c > 0x7f {
                    self.w.printf(format_args!("\\{c:03o}"));
                } else {
                    self.w.out(c);
                }
            }
            self.w.puts(b")");
        } else {
            self.w.puts(b"<");
            for &c in s {
                self.w.printf(format_args!("{c:02x}"));
            }
            self.w.puts(b">");
        }
    }

    /// `writeRefs`.  The list grows while it is being written.
    fn write_refs(&mut self) -> Result<(), Fail> {
        let mut i = 0;
        while i < self.list.list.len() {
            if !self.list.list[i].written {
                self.list.list[i].written = true;
                let r = self.list.list[i].clone();
                let obj = self.doc.fetch(r.r.0, r.r.1, 0);
                let num = ObjNo::Slot(r.slot);
                match r.ty {
                    InObjType::Font => {
                        let Object::Dict(d) = &obj else {
                            return Err(Fail(format!("PDF inclusion: invalid dict type <{}>", obj.type_name()).into_bytes()));
                        };
                        self.w.op(Op::BeginObj { obj: num, level: 2 });
                        self.copy_font_dict(d, &r)?;
                        self.w.puts(b"\n");
                        self.w.op(Op::EndObj);
                    }
                    InObjType::Other => {
                        let level = if matches!(obj, Object::Stream(_)) { 0 } else { 2 };
                        self.w.op(Op::BeginObj { obj: num, level });
                        self.copy_object(&obj)?;
                        self.w.puts(b"\n");
                        self.w.op(Op::EndObj);
                    }
                    InObjType::FontDesc => {}
                }
            }
            i += 1;
        }
        Ok(())
    }

    /// `writeEncodings`, with `epdf_write_enc`.  The list is built at the head, so the last font comes first.
    fn write_encodings(&mut self) -> Result<(), Fail> {
        while let Some(r) = self.encodings.pop() {
            if r.is_cid {
                return Err(Fail::new(
                    "PDF inclusion: CID fonts are not supported (try to disable font replacement to fix this)",
                ));
            }
            self.w.op(Op::BeginDict { obj: ObjNo::Slot(r.enc), level: 1 });
            self.w.puts(b"/Type /Encoding\n");
            self.w.puts(b"/Differences [");
            let mut i_old = -2;
            for (i, n) in r.names.iter().enumerate() {
                if let Some(name) = n {
                    let i = i as i32;
                    if i == i_old + 1 {
                        self.w.printf_bytes(b"/", name, b"");
                    } else if i_old == -2 {
                        self.w.printf_bytes(format!("{i}/").as_bytes(), name, b"");
                    } else {
                        self.w.printf_bytes(format!(" {i}/").as_bytes(), name, b"");
                    }
                    i_old = i;
                }
            }
            self.w.puts(b"]\n");
            self.w.op(Op::EndDict);
        }
        Ok(())
    }
}

/// `convertNumToPDF`.
pub(crate) fn convert_num_to_pdf(mut n: f64) -> Vec<u8> {
    const PRECISION: usize = 6;
    const FACT: f64 = 1000000.0;
    const EPSILON: f64 = 0.5e-6;
    if n.abs() < EPSILON {
        return b"0".to_vec();
    }
    let mut buf = Vec::with_capacity(24);
    if n < 0.0 {
        buf.push(b'-');
        n = -n;
    }
    n += EPSILON;
    // (int) floor(n): the conversion of a value out of range is the x86 "indefinite", INT_MIN.
    let fl = n.floor();
    let ival: i32 = if (-2147483649.0..2147483648.0).contains(&fl) { fl as i32 } else { i32::MIN };
    n -= ival as f64;
    buf.extend_from_slice(ival.to_string().as_bytes());
    let ff = (n * FACT).floor();
    let mut fval: i32 = if (-2147483649.0..2147483648.0).contains(&ff) { ff as i32 } else { i32::MIN };
    if fval != 0 {
        buf.push(b'.');
        let at = buf.len();
        let mut digits = [0u8; PRECISION];
        let mut len = PRECISION;
        while fval % 10 == 0 && len > 0 {
            len -= 1;
            fval /= 10;
        }
        let mut i = len;
        while i > 0 {
            i -= 1;
            digits[i] = (fval % 10 + 48) as u8;
            fval /= 10;
        }
        buf.truncate(at);
        buf.extend_from_slice(&digits[..len]);
    }
    buf
}

/// `stripzeros`.
pub(crate) fn stripzeros(a: &[u8]) -> Vec<u8> {
    #[derive(Clone, Copy, PartialEq)]
    enum S {
        NoNum,
        DotNoNum,
        Int,
        Dot,
        LeadDot,
        Frac,
    }
    let mut out = a.to_vec();
    let (mut s, mut t) = (S::NoNum, S::NoNum);
    let (mut p, mut q, mut r) = (0usize, 0usize, 0usize);
    while p < a.len() {
        let c = a[p];
        match s {
            S::NoNum => {
                if c.is_ascii_digit() {
                    s = S::Int;
                } else if c == b'.' {
                    s = S::LeadDot;
                }
            }
            S::DotNoNum => {
                if c != b'.' && !c.is_ascii_digit() {
                    s = S::NoNum;
                }
            }
            S::Int => {
                if c == b'.' {
                    s = S::Dot;
                } else if !c.is_ascii_digit() {
                    s = S::NoNum;
                }
            }
            S::Dot | S::LeadDot => {
                if c.is_ascii_digit() {
                    s = S::Frac;
                } else if c == b'.' {
                    s = S::DotNoNum;
                } else {
                    s = S::NoNum;
                }
            }
            S::Frac => {
                if c == b'.' {
                    s = S::DotNoNum;
                } else if !c.is_ascii_digit() {
                    s = S::NoNum;
                }
            }
        }
        match s {
            S::Dot => r = q,
            S::LeadDot => r = q + 1,
            S::Frac => {
                if c > b'0' {
                    r = q + 1;
                }
            }
            S::NoNum => {
                if (t == S::Frac || t == S::Dot) && r != 0 {
                    q = r;
                    r -= 1;
                    if out[r] == b'.' {
                        out[r] = b'0';
                    }
                    r = 0;
                }
            }
            _ => {}
        }
        out[q] = c;
        q += 1;
        p += 1;
        t = s;
    }
    out.truncate(q);
    out
}

/// `convertStringToPDFString`.
pub(crate) fn convert_string_to_pdf_string(input: &[u8]) -> Result<Vec<u8>, Fail> {
    const MAX_PSTRING_LEN: usize = 1024;
    let mut out = Vec::with_capacity(input.len() + 8);
    for &c in input {
        if out.len() + 5 > MAX_PSTRING_LEN {
            return Err(Fail::new("buffer overflow in convertStringToPDFString"));
        }
        if !(b'!'..=b'~').contains(&c) {
            out.extend_from_slice(format!("\\{c:03o}").as_bytes());
        } else if c == b'(' || c == b')' || c == b'\\' {
            out.push(b'\\');
            out.push(c);
        } else {
            out.push(c);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(n: f64) -> String {
        String::from_utf8(convert_num_to_pdf(n)).unwrap()
    }

    #[test]
    fn numbers_as_convert_num_to_pdf_prints() {
        assert_eq!(num(0.0), "0");
        assert_eq!(num(1.0), "1");
        assert_eq!(num(-1.5), "-1.5");
        assert_eq!(num(0.1), "0.1");
        assert_eq!(num(595.276), "595.276");
        assert_eq!(num(0.000001), "0.000001");
        assert_eq!(num(0.0000004), "0");
        assert_eq!(num(1.9999999), "2");
        assert_eq!(num(0.25), "0.25");
    }

    fn strip(s: &str) -> String {
        String::from_utf8(stripzeros(s.as_bytes())).unwrap()
    }

    #[test]
    fn stripzeros_cases() {
        assert_eq!(strip("/BBox [0.00000000 0.00000000 612.00000000 792.50000000]\n"), "/BBox [0 0 612 792.5]\n");
        assert_eq!(strip("[-1.00000000 0.25000000]\n"), "[-1 0.25]\n");
        assert_eq!(strip("[.50000000]\n"), "[.5]\n");
        assert_eq!(strip("[.00000000]\n"), "[0]\n");
    }

    #[test]
    fn pdf_strings() {
        assert_eq!(convert_string_to_pdf_string(b"a b(c)\\.pdf").unwrap(), b"a\\040b\\(c\\)\\\\.pdf");
        assert_eq!(convert_string_to_pdf_string(&[0xe4]).unwrap(), b"\\344");
    }
}
