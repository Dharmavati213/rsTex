//! A stand-in for the engine: it replays this crate's ops the way `read_image`, `write_image` and the PDF output
//! procedures of the WEB would, with `\pdfcompresslevel=0` and `\pdfobjcompresslevel=0`, into objects keyed by number.
//!
//! A case is a sequence of `\immediate\pdfximage`.  That writes the object at once, so a run of pdfTeX and a run of this
//! start from the same `obj_ptr` and allocate in the same order.

use rstex_image::epdf::{EpdfDocs, EpdfParams, PDF_BOX_SPEC_CROP, PdfInfoParams};
use rstex_image::jbig2::{self, Jbig2File};
use rstex_image::out::{ObjNo, Op, SlotAlloc};
use rstex_image::{ImageType, jpg, png};
use std::collections::{BTreeMap, HashMap};

/// The engine parameters a case can set.
#[derive(Clone, Debug)]
pub struct Params {
    pub major: i32,
    pub minor: i32,
    pub hicolor: i32,
    pub apply_gamma: i32,
    pub gamma: i32,
    pub image_gamma: i32,
    pub resolution: i32,
    pub errorlevel: i32,
    pub copy_fonts: bool,
    pub suppress_info: i32,
    pub underscore: bool,
    pub suppress_group_warning: bool,
    /// The number `\pdfximage` gets: `obj_ptr + 1` when the run starts.
    pub start_obj: i32,
    /// For an image written at shipout: the next free number by then, and `pdf_page_group_val`.  The page has taken
    /// the numbers in between.  0 for `\immediate`.
    pub next_at_write: i32,
    pub page_group: i32,
    /// `fixed_pdf_objcompresslevel`.  Above 0 an object stream takes a number when its first object comes.
    pub obj_level: i32,
    /// All the images are read, then a page is shipped out with them on it, which writes them.
    pub deferred: bool,
    /// One of `super::kernels`.
    pub kernels: png::Kernels,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            major: 1,
            minor: 5,
            hicolor: 1,
            apply_gamma: 0,
            gamma: 1000,
            image_gamma: 2200,
            resolution: 0,
            errorlevel: 0,
            copy_fonts: false,
            suppress_info: 0,
            underscore: false,
            suppress_group_warning: false,
            start_obj: 1,
            next_at_write: 0,
            page_group: 0,
            obj_level: 0,
            deferred: false,
            kernels: png::Kernels::PLAIN,
        }
    }
}

impl Params {
    /// Reads `key=value` words.
    pub fn parse(words: &str) -> Params {
        let mut p = Params::default();
        for w in words.split_whitespace() {
            let (k, v) = w.split_once('=').unwrap_or((w, "1"));
            let n: i32 = v.parse().unwrap_or(0);
            match k {
                "major" => p.major = n,
                "minor" => p.minor = n,
                "hicolor" => p.hicolor = n,
                "applygamma" => p.apply_gamma = n,
                "gamma" => p.gamma = n,
                "imagegamma" => p.image_gamma = n,
                "resolution" => p.resolution = n,
                "errorlevel" => p.errorlevel = n,
                "copyfonts" => p.copy_fonts = n != 0,
                "suppressinfo" => p.suppress_info = n,
                "underscore" => p.underscore = n != 0,
                "suppressgroupwarning" => p.suppress_group_warning = n != 0,
                "start" => p.start_obj = n,
                "next" => p.next_at_write = n,
                "group" => p.page_group = n,
                "objlevel" => p.obj_level = n,
                "deferred" => p.deferred = true,
                _ => panic!("unknown parameter {k}"),
            }
        }
        p
    }
}

/// One `\pdfximage`.
#[derive(Clone, Debug)]
pub struct Image {
    /// The name the file goes by in the PDF, as kpathsea found it.
    pub name: String,
    pub page: i32,
    pub named: Option<String>,
    pub colorspace: i32,
    pub page_box: i32,
}

impl Image {
    /// `name[,page=N][,named=X][,colorspace=N][,box=N]`.
    pub fn parse(s: &str) -> Image {
        let mut parts = s.split(',');
        let mut im = Image {
            name: parts.next().unwrap().to_string(),
            page: 1,
            named: None,
            colorspace: 0,
            page_box: PDF_BOX_SPEC_CROP,
        };
        for w in parts {
            let (k, v) = w.split_once('=').unwrap();
            match k {
                "page" => im.page = v.parse().unwrap(),
                "named" => im.named = Some(v.to_string()),
                "colorspace" => im.colorspace = v.parse().unwrap(),
                "box" => im.page_box = v.parse().unwrap(),
                _ => panic!("unknown image option {k}"),
            }
        }
        im
    }
}

/// What the TeX side can see of an image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    pub pages: i32,
    pub depth: i32,
    pub wd: i32,
    pub ht: i32,
}

/// The result of a case.
#[derive(Debug, Default)]
pub struct RunOutput {
    /// Objects in the order they were written.
    pub objects: Vec<(i32, Vec<u8>)>,
    /// The `pdf_os_level` each was begun with, for the objects above.
    pub levels: Vec<i32>,
    pub seen: Vec<Seen>,
    pub warnings: Vec<Vec<u8>>,
    pub printed: Vec<u8>,
    /// How many font descriptors the font replacement made: the oracle writes objects for them at the end.
    pub fonts: usize,
    /// `pdftex_fail`'s message, and nothing after it.
    pub failed: Option<Vec<u8>>,
}

fn ext_xn_over_d(x: i32, n: i32, d: i32) -> i32 {
    let mut r = x as f64 * n as f64 / d as f64;
    if r > f64::EPSILON {
        r += 0.5;
    } else {
        r -= 0.5;
    }
    r as i32
}

/// §1552 `scale_image`, for an image with neither width nor height given.
fn scale_image(is_pdf: bool, rotate: i32, width: i32, height: i32, xres: i32, yres: i32, default_res: i32) -> (i32, i32) {
    const ONE_HUNDRED_INCH: i32 = 473628672;
    const ONE_INCH: f64 = 226.0;
    let (x, y, mut xr, mut yr) =
        if rotate == 90 || rotate == 270 { (height, width, yres, xres) } else { (width, height, xres, yres) };
    if xr > 65535 || yr > 65535 {
        xr = 0;
        yr = 0;
    }
    if (xr != 0 || yr != 0) && (x as f64 / ONE_INCH >= xr as f64 || y as f64 / ONE_INCH >= yr as f64) {
        xr = 0;
        yr = 0;
    }
    if is_pdf {
        return (x, y);
    }
    if default_res > 0 && (xr == 0 || yr == 0) {
        xr = default_res;
        yr = default_res;
    }
    if xr > 0 && yr > 0 {
        (ext_xn_over_d(ONE_HUNDRED_INCH, x, 100 * xr), ext_xn_over_d(ONE_HUNDRED_INCH, y, 100 * yr))
    } else {
        (ext_xn_over_d(ONE_HUNDRED_INCH, x, 7200), ext_xn_over_d(ONE_HUNDRED_INCH, y, 7200))
    }
}

/// `lookup_fontmap`'s treatment of a name with a subset tag.
pub fn strip_subset_tag(name: &[u8]) -> &[u8] {
    if name.len() > 7 && name[..6].iter().all(|c| c.is_ascii_uppercase()) && name[6] == b'+' { &name[7..] } else { name }
}

/// An image read and not yet written.
pub enum Pending {
    Png { n: i32, info: png::PngInfo, data: Vec<u8>, colorspace: i32 },
    Jpg { n: i32, info: jpg::JpgInfo, data: Vec<u8>, colorspace: i32 },
    Jbig2 { n: i32, name: String, page: i32 },
    Pdf { n: i32, doc: usize, page: i32, page_box: i32 },
}

/// The engine: everything that outlives one image.
pub struct Sim {
    pub p: Params,
    pub next_obj: i32,
    /// PostScript name to font file name, the part of the font map `lookup_fontmap` and `lookup_fd_entry` use.
    pub fontmap: HashMap<Vec<u8>, Vec<u8>>,
    slots: SlotAlloc,
    table: Vec<i32>,
    objects: Vec<(i32, Vec<u8>)>,
    levels: Vec<i32>,
    current: Option<(i32, Vec<u8>)>,
    current_level: i32,
    os_open: bool,
    os_count: i32,
    stream_length_at: Option<usize>,
    stream_start: usize,
    transparent_page_group: i32,
    group_written: bool,
    page_group_val: i32,
    docs: EpdfDocs,
    jb2: BTreeMap<String, (Jbig2File, Vec<u8>)>,
    fd_by_file: HashMap<Vec<u8>, usize>,
    fds: Vec<(i32, i32)>,
    fd_of_slot: HashMap<u32, usize>,
    warnings: Vec<Vec<u8>>,
    printed: Vec<u8>,
}

impl Sim {
    pub fn new(p: Params) -> Sim {
        Sim {
            next_obj: p.start_obj,
            p,
            fontmap: HashMap::new(),
            slots: SlotAlloc::default(),
            table: Vec::new(),
            objects: Vec::new(),
            levels: Vec::new(),
            current: None,
            current_level: 0,
            os_open: false,
            os_count: 0,
            stream_length_at: None,
            stream_start: 0,
            transparent_page_group: 0,
            group_written: false,
            page_group_val: 0,
            docs: EpdfDocs::default(),
            jb2: BTreeMap::new(),
            fd_by_file: HashMap::new(),
            fds: Vec::new(),
            fd_of_slot: HashMap::new(),
            warnings: Vec::new(),
            printed: Vec::new(),
        }
    }

    fn new_objnum(&mut self) -> i32 {
        let n = self.next_obj;
        self.next_obj += 1;
        n
    }

    /// `pdf_begin_obj`, with the numbering part of `pdf_os_prepare_obj`.
    fn begin(&mut self, n: i32, level: i32) {
        if level > 0 && self.p.obj_level >= level && !self.os_open {
            self.new_objnum();
            self.os_open = true;
        }
        self.current = Some((n, format!("{n} 0 obj\n").into_bytes()));
        self.current_level = level;
    }

    fn cur(&mut self) -> &mut Vec<u8> {
        &mut self.current.as_mut().expect("output outside an object").1
    }

    /// `pdf_end_obj`.  An object stream is written out after its 100th object.
    fn finish(&mut self) {
        if let Some(o) = self.current.take() {
            self.objects.push(o);
            self.levels.push(self.current_level);
            if self.current_level > 0 && self.p.obj_level >= self.current_level {
                self.os_count += 1;
                if self.os_count == 100 {
                    self.os_count = 0;
                    self.os_open = false;
                }
            }
        }
    }

    fn number(&self, o: ObjNo) -> i32 {
        match o {
            ObjNo::Slot(s) => self.table[s as usize],
            ObjNo::Known(n) => n,
        }
    }

    fn set_slot(&mut self, s: u32, v: i32) {
        let need = (s as usize + 1).max(self.slots.len());
        if self.table.len() < need {
            self.table.resize(need, 0);
        }
        self.table[s as usize] = v;
    }

    /// The replay loop of the crate documentation.
    pub fn replay(&mut self, ops: Vec<Op>, input: &[u8]) {
        for op in ops {
            match op {
                Op::Puts(b) => self.cur().extend_from_slice(&b),
                Op::CopyInput { start, len } => {
                    let d = &input[start..start + len];
                    self.cur().extend_from_slice(d);
                }
                Op::NewObj(s) => {
                    let n = self.new_objnum();
                    self.set_slot(s, n);
                }
                Op::Num(s) => {
                    let n = self.table[s as usize];
                    self.cur().extend_from_slice(n.to_string().as_bytes());
                }
                Op::BeginDict { obj, level } => {
                    let n = self.number(obj);
                    self.begin(n, level);
                    self.cur().extend_from_slice(b"<<\n");
                }
                Op::BeginObj { obj, level } => {
                    let n = self.number(obj);
                    self.begin(n, level);
                }
                Op::EndDict => {
                    self.cur().extend_from_slice(b">>\nendobj\n");
                    self.finish();
                }
                Op::EndObj => {
                    self.cur().extend_from_slice(b"endobj\n");
                    self.finish();
                }
                Op::BeginStream => {
                    // pdf_begin_stream, compress level 0: the length goes over the spaces at pdf_end_stream.
                    let at = self.cur().len() + b"/Length ".len();
                    self.stream_length_at = Some(at);
                    self.cur().extend_from_slice(b"/Length           \n>>\nstream\n");
                    self.stream_start = self.cur().len();
                }
                Op::EndStream => {
                    if let Some(at) = self.stream_length_at.take() {
                        let len = self.cur().len() - self.stream_start;
                        let digits = len.to_string();
                        self.cur()[at..at + digits.len()].copy_from_slice(digits.as_bytes());
                    }
                    self.cur().extend_from_slice(b"\nendstream\nendobj\n");
                    self.finish();
                }
                Op::Flush => {}
                Op::TexPrint(b) => self.printed.extend_from_slice(&b),
                Op::Warn(b) => self.warnings.push(b),
                Op::EpdfFont { fd_slot, basefont, .. } => {
                    let ps = strip_subset_tag(&basefont).to_vec();
                    let file = self.fontmap.get(&ps).expect("a font the map answered for").clone();
                    let fd = match self.fd_by_file.get(&file) {
                        Some(&f) => f,
                        None => {
                            let num = self.new_objnum();
                            self.fds.push((num, 0));
                            let f = self.fds.len() - 1;
                            self.fd_by_file.insert(file, f);
                            f
                        }
                    };
                    self.fd_of_slot.insert(fd_slot, fd);
                    self.set_slot(fd_slot, self.fds[fd].0);
                }
                Op::EpdfFontName { fd_slot, fn_slot } => {
                    let fd = self.fd_of_slot[&fd_slot];
                    if self.fds[fd].1 == 0 {
                        let n = self.new_objnum();
                        self.fds[fd].1 = n;
                    }
                    self.set_slot(fn_slot, self.fds[fd].1);
                }
                Op::Precompressed(_) => unreachable!("only the engine's output cache gives this, not a writer"),
            }
        }
    }

    /// `\pdfximage`: `read_image` and `scale_image`.
    pub fn read(&mut self, im: &Image, data: Vec<u8>) -> Result<(Seen, Pending), Vec<u8>> {
        let n = self.new_objnum();
        let ty = {
            let t = rstex_image::check_type_by_header(&data);
            if t == ImageType::None { rstex_image::check_type_by_extension(im.name.as_bytes()) } else { t }
        };
        let p = self.p.clone();
        match ty {
            ImageType::Png => {
                let info = png::read_png_info(&data).map_err(|e| e.0)?;
                if info.wants_page_group(p.major, p.minor) {
                    if self.transparent_page_group == 0 {
                        self.transparent_page_group = self.new_objnum();
                    }
                    if self.page_group_val == 0 {
                        self.page_group_val = self.transparent_page_group;
                    }
                }
                let (wd, ht) =
                    scale_image(false, 0, info.width as i32, info.height as i32, info.x_res, info.y_res, p.resolution);
                let seen = Seen { pages: 1, depth: info.bit_depth as i32, wd, ht };
                Ok((seen, Pending::Png { n, info, data, colorspace: im.colorspace }))
            }
            ImageType::Jpg => {
                let info = jpg::read_jpg_info(&data, p.major, p.minor).map_err(|e| e.0)?;
                let (wd, ht) = scale_image(false, 0, info.width, info.height, info.x_res, info.y_res, p.resolution);
                let seen = Seen { pages: 1, depth: info.bits_per_component, wd, ht };
                Ok((seen, Pending::Jpg { n, info, data, colorspace: im.colorspace }))
            }
            ImageType::Jbig2 => {
                if p.major == 1 && p.minor < 4 {
                    return Err(format!(
                        "JBIG2 images only possible with at least PDF 1.4; you are generating PDF 1.{}",
                        p.minor
                    )
                    .into_bytes());
                }
                if !self.jb2.contains_key(&im.name) {
                    let f = jbig2::rd_jbig2_info(&data).map_err(|e| e.0)?;
                    self.jb2.insert(im.name.clone(), (f, data));
                }
                let (file, _) = self.jb2.get_mut(&im.name).unwrap();
                let info = jbig2::read_jbig2_info(file, im.page).map_err(|e| e.0)?;
                let (wd, ht) = scale_image(false, 0, info.width, info.height, info.x_res, info.y_res, p.resolution);
                let seen = Seen { pages: info.pages, depth: 0, wd, ht };
                Ok((seen, Pending::Jbig2 { n, name: im.name.clone(), page: im.page }))
            }
            ImageType::Pdf => {
                // What kpathsea makes of a name in the current directory.
                let found = if im.name.starts_with('/') { im.name.clone() } else { format!("./{}", im.name) };
                let name = found.as_bytes();
                let i = match self.docs.find_document(name) {
                    Some(i) => i,
                    None => self.docs.add_document(name, data).map_err(|e| e.0)?,
                };
                let params = PdfInfoParams {
                    page_num: im.page,
                    page_box: im.page_box,
                    major_wanted: p.major,
                    minor_wanted: p.minor,
                    inclusion_errorlevel: p.errorlevel,
                };
                let doc = self.docs.doc_mut(i);
                let info = doc.read_pdf_info(im.named.as_deref().map(str::as_bytes), &params).map_err(|e| e.0)?;
                if let Some(w) = &info.warning {
                    self.warnings.push(w.clone());
                }
                let (wd, ht) = scale_image(true, info.rotate, info.width, info.height, 0, 0, p.resolution);
                let seen = Seen { pages: info.pages, depth: 0, wd, ht };
                Ok((seen, Pending::Pdf { n, doc: i, page: info.selected_page, page_box: im.page_box }))
            }
            ImageType::None => Err(b"unknown type of image".to_vec()),
        }
    }

    /// `pdf_write_image`.
    pub fn write(&mut self, pending: Pending) -> Result<(), Vec<u8>> {
        let p = self.p.clone();
        match pending {
            Pending::Png { n, info, data, colorspace } => {
                self.begin(n, 0);
                self.cur().extend_from_slice(b"<<\n");
                let params = png::PngParams {
                    pdf_major: p.major,
                    pdf_minor: p.minor,
                    image_hicolor: p.hicolor,
                    image_apply_gamma: p.apply_gamma,
                    gamma: p.gamma,
                    image_gamma: p.image_gamma,
                    colorspace_ref: colorspace,
                    kernels: p.kernels,
                };
                let out = png::write_png(&info, &data, &params, &mut self.slots).map_err(|e| e.0)?;
                self.replay(out.ops, &data);
                if out.needs_page_group && !self.group_written && self.transparent_page_group > 0 {
                    self.group_written = true;
                    let ops = png::png_page_group_object(self.transparent_page_group, 0);
                    self.replay(ops, &data);
                }
            }
            Pending::Jpg { n, info, data, colorspace } => {
                self.begin(n, 0);
                self.cur().extend_from_slice(b"<<\n");
                let ops = jpg::write_jpg(&info, colorspace, &mut self.slots).map_err(|e| e.0)?;
                self.replay(ops, &data);
            }
            Pending::Jbig2 { n, name, page } => {
                let (file, bytes) = self.jb2.get_mut(&name).unwrap();
                let ops = jbig2::write_jbig2(file, bytes, page, &mut self.slots).map_err(|e| e.0)?;
                let bytes = bytes.clone();
                self.begin(n, 0);
                self.cur().extend_from_slice(b"<<\n");
                self.replay(ops, &bytes);
            }
            Pending::Pdf { n, doc, page, page_box } => {
                let d = self.docs.doc_mut(doc);
                let queries = if p.copy_fonts { Vec::new() } else { d.epdf_font_queries(page) };
                let hits: Vec<Vec<u8>> =
                    queries.into_iter().filter(|q| self.fontmap.contains_key(strip_subset_tag(q))).collect();
                self.begin(n, 0);
                self.cur().extend_from_slice(b"<<\n");
                let e = EpdfParams {
                    selected_page: page,
                    page_box,
                    page_group_val: self.page_group_val,
                    suppress_ptex_info: p.suppress_info,
                    use_underscore: p.underscore,
                    suppress_warning_page_group: p.suppress_group_warning,
                    inclusion_copy_font: p.copy_fonts,
                    fontmap_hits: &hits,
                };
                let d = self.docs.doc_mut(doc);
                let out = d.write_epdf(&e, &mut self.slots).map_err(|err| err.0)?;
                self.page_group_val = out.page_group_val;
                let bytes = self.docs.doc_mut(doc).data().to_vec();
                self.replay(out.ops, &bytes);
                self.docs.epdf_delete(doc);
            }
        }
        Ok(())
    }

    /// Between `read_image` and `pdf_write_image`, for an image written at shipout.
    pub fn to_write_time(&mut self) {
        if self.p.next_at_write != 0 {
            self.next_obj = self.p.next_at_write;
            self.page_group_val = self.p.page_group;
        }
    }

    /// The end of the run: `flushjbig2page0objects`.  `images` have all been written.
    pub fn finish_run(&mut self) -> Result<(), Vec<u8>> {
        let names: Vec<String> = self.jb2.keys().cloned().collect();
        for name in names {
            let (file, bytes) = self.jb2.get_mut(&name).unwrap();
            let ops = jbig2::flush_page0(file, bytes, &mut self.slots).map_err(|e| e.0)?;
            let bytes = bytes.clone();
            self.replay(ops, &bytes);
        }
        Ok(())
    }

    pub fn into_output(self) -> RunOutput {
        RunOutput {
            objects: self.objects,
            levels: self.levels,
            seen: Vec::new(),
            warnings: self.warnings,
            printed: self.printed,
            fonts: self.fds.len(),
            failed: None,
        }
    }
}

/// Runs the images of a case in order, each given as its option string, with `read` supplying the bytes.
pub fn run_case<F>(params: &Params, images: &[String], fontmap: HashMap<Vec<u8>, Vec<u8>>, mut read: F) -> RunOutput
where
    F: FnMut(&str) -> Vec<u8>,
{
    let mut sim = Sim::new(params.clone());
    sim.fontmap = fontmap;
    let mut seen = Vec::new();
    let mut failed = None;
    let mut waiting = Vec::new();
    for spec in images {
        let im = Image::parse(spec);
        let data = read(&im.name);
        match sim.read(&im, data) {
            Ok((s, pending)) => {
                // TeX gets to look at the image after \immediate has written it.
                if params.deferred {
                    seen.push(s);
                    waiting.push(pending);
                } else if let Err(m) = sim.write(pending) {
                    failed = Some(m);
                    break;
                } else {
                    seen.push(s);
                }
            }
            Err(m) => {
                failed = Some(m);
                break;
            }
        }
    }
    if failed.is_none() && params.deferred {
        sim.to_write_time();
        for pending in waiting {
            if let Err(m) = sim.write(pending) {
                failed = Some(m);
                break;
            }
        }
    }
    if failed.is_none() {
        if let Err(m) = sim.finish_run() {
            failed = Some(m);
        }
    }
    let mut out = sim.into_output();
    out.seen = seen;
    out.failed = failed;
    out
}
