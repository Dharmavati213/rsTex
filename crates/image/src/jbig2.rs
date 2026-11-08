//! writejbig2.c: `read_jbig2_info`, `write_jbig2` and `flushjbig2page0objects`.
//!
//! The C keeps an AVL tree of `FILEINFO` by file name.  Here a `Jbig2File` is that record: the engine keeps one per file
//! name and, at the end of the run, calls `flush_page0` for each in `strcmp` order of the names.  The C opens the file
//! again to write; here it is given the same bytes each time.
//!
//! `checkseghdr` reads from wherever the file position is, not from the segment it was asked about, and `writeseghdr`
//! carries on from there.  That is what the C does, so the position is threaded through the same way.

use crate::out::{ObjNo, Op, Slot, SlotAlloc, Writer};
use crate::Fail;

const M_PAGE_INFORMATION: u32 = 48;
const M_END_OF_PAGE: u32 = 49;
const M_END_OF_STRIPE: u32 = 50;
const M_END_OF_FILE: u32 = 51;

const PREMATURE: &str = "getc() failed; premature end of JBIG2 image file";

#[derive(Clone, Debug, Default)]
struct SegInfo {
    segnum: u32,
    isrefered: bool,
    refers: bool,
    seghdrflags: u32,
    pageassocsizeflag: bool,
    countofrefered: u32,
    fieldlen: u32,
    segnumwidth: u32,
    segpage: i64,
    segdatalen: u32,
    hdrstart: i64,
    hdrend: i64,
    datastart: i64,
    dataend: i64,
    endofstripeflag: bool,
    endofpageflag: bool,
    pageinfoflag: bool,
    endoffileflag: bool,
}

#[derive(Clone, Debug, Default)]
struct PageInfo {
    segments: Vec<SegInfo>,
    pagenum: u32,
    width: u32,
    height: u32,
    xres: u32,
    yres: u32,
    stripinginfo: u32,
    stripedheight: u32,
}

/// `FILEINFO`.
#[derive(Clone, Debug, Default)]
pub struct Jbig2File {
    filesize: i64,
    pages: Vec<PageInfo>,
    page0: Option<PageInfo>,
    filehdrflags: u32,
    sequentialaccess: bool,
    numofpages: u32,
    pdfpage0: Option<Slot>,
}

/// What `read_jbig2_info` stores in the image entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Jbig2Info {
    /// `img_pages`.
    pub pages: i32,
    /// `img_width`.
    pub width: i32,
    /// `img_height`.
    pub height: i32,
    /// `img_xres`.
    pub x_res: i32,
    /// `img_yres`.
    pub y_res: i32,
}

struct Rd<'a> {
    d: &'a [u8],
    pos: i64,
}

impl Rd<'_> {
    /// `ygetc`.
    fn getc(&mut self) -> Result<u32, Fail> {
        if self.pos >= 0 && (self.pos as u64) < self.d.len() as u64 {
            let c = self.d[self.pos as usize];
            self.pos += 1;
            Ok(c as u32)
        } else {
            Err(Fail::new(PREMATURE))
        }
    }

    fn read2bytes(&mut self) -> Result<u32, Fail> {
        let c = self.getc()?;
        Ok((c << 8) + self.getc()?)
    }

    fn read4bytes(&mut self) -> Result<u32, Fail> {
        let l = self.read2bytes()?;
        Ok(l.wrapping_shl(16).wrapping_add(self.read2bytes()?))
    }

    /// `xfseeko`, which stops the run on a position before the start.
    fn seek(&mut self, p: i64) -> Result<(), Fail> {
        if p < 0 {
            return Err(Fail::new("fseeko failed"));
        }
        self.pos = p;
        Ok(())
    }
}

/// `getstreamlen`.
fn getstreamlen(segs: &[SegInfo], refer: bool) -> i64 {
    let mut len = 0;
    for sip in segs {
        if refer || sip.isrefered {
            len += (sip.hdrend - sip.hdrstart) + (sip.dataend - sip.datastart);
        }
    }
    len
}

/// `readfilehdr`.
fn readfilehdr(fip: &mut Jbig2File, r: &mut Rd<'_>) -> Result<(), Fail> {
    const JBIG2_ID: [u8; 8] = [0x97, b'J', b'B', b'2', 0x0d, 0x0a, 0x1a, 0x0a];
    r.seek(0)?;
    for id in JBIG2_ID {
        if r.getc()? != id as u32 {
            return Err(Fail::new("readfilehdr(): reading JBIG2 image file failed: ID string missing"));
        }
    }
    fip.filehdrflags = r.getc()?;
    fip.sequentialaccess = fip.filehdrflags & 0x01 != 0;
    if fip.sequentialaccess {
        fip.filesize = r.d.len() as i64;
        r.seek(9)?;
    }
    // `if (!(fip->filehdrflags >> 1) & 0x01)`: the `!` binds first, so any higher flag means no count.
    if fip.filehdrflags >> 1 == 0 {
        fip.numofpages = r.read4bytes()?;
    }
    Ok(())
}

/// `checkseghdrflags`.
fn checkseghdrflags(sip: &mut SegInfo) -> Result<(), Fail> {
    sip.endofstripeflag = false;
    sip.endofpageflag = false;
    sip.pageinfoflag = false;
    sip.endoffileflag = false;
    match sip.seghdrflags & 0x3f {
        0 | 4 | 6 | 7 | 16 | 20 | 22 | 23 | 36 | 38 | 39 | 40 | 42 | 43 | 52 | 53 | 62 => {}
        M_PAGE_INFORMATION => sip.pageinfoflag = true,
        M_END_OF_PAGE => sip.endofpageflag = true,
        M_END_OF_STRIPE => sip.endofstripeflag = true,
        M_END_OF_FILE => sip.endoffileflag = true,
        _ => return Err(Fail::new("checkseghdrflags(): unknown segment type in JBIG2 image file")),
    }
    Ok(())
}

/// `readseghdr`: the return value tells if a header has been read.
fn readseghdr(fip: &Jbig2File, r: &mut Rd<'_>, sip: &mut SegInfo) -> Result<bool, Fail> {
    sip.hdrstart = r.pos;
    if fip.sequentialaccess && sip.hdrstart == fip.filesize {
        return Ok(false); // no endoffileflag is ok for sequentialaccess
    }
    sip.segnum = r.read4bytes()?;
    sip.seghdrflags = r.getc()?;
    checkseghdrflags(sip)?;
    if fip.sequentialaccess && sip.endoffileflag {
        return Ok(true); // accept shorter segment
    }
    sip.pageassocsizeflag = (sip.seghdrflags >> 6) & 0x01 != 0;
    let reftosegcount = r.getc()?;
    sip.countofrefered = reftosegcount >> 5;
    if sip.countofrefered < 5 {
        sip.fieldlen = 1;
    } else {
        sip.fieldlen = 5 + sip.countofrefered / 8;
        r.seek(r.pos + sip.fieldlen as i64 - 1)?;
    }
    sip.segnumwidth = if sip.segnum <= 256 {
        1
    } else if sip.segnum <= 65536 {
        2
    } else {
        4
    };
    for _ in 0..sip.countofrefered {
        match sip.segnumwidth {
            1 => {
                r.getc()?;
            }
            2 => {
                r.read2bytes()?;
            }
            _ => {
                r.read4bytes()?;
            }
        }
    }
    sip.segpage = if sip.pageassocsizeflag { r.read4bytes()? as i64 } else { r.getc()? as i64 };
    sip.segdatalen = r.read4bytes()?;
    sip.hdrend = r.pos;
    Ok(true)
}

/// `rd_jbig2_info`, with `pages_maketree` and `segments_maketree`, which only build the search trees.
pub fn rd_jbig2_info(data: &[u8]) -> Result<Jbig2File, Fail> {
    let mut fip = Jbig2File::default();
    let mut r = Rd { d: data, pos: 0 };
    let mut streampos: i64 = 0;
    let mut currentpage: i64 = 0;
    readfilehdr(&mut fip, &mut r)?;
    if !fip.sequentialaccess {
        // findstreamstart: D.2 random-access organisation
        let mut tmp = SegInfo::default();
        loop {
            readseghdr(&fip, &mut r, &mut tmp)?;
            if tmp.endoffileflag {
                break;
            }
        }
        streampos = tmp.hdrend;
        readfilehdr(&mut fip, &mut r)?;
    }
    loop {
        let mut sip = SegInfo::default();
        if !readseghdr(&fip, &mut r, &mut sip)? || sip.endoffileflag {
            break;
        }
        let in_pages = sip.segpage > 0;
        if in_pages {
            if sip.segpage > currentpage {
                fip.pages.push(PageInfo::default());
                currentpage = sip.segpage;
            }
        } else if fip.page0.is_none() {
            fip.page0 = Some(PageInfo::default());
        }
        sip.datastart = if !fip.sequentialaccess { streampos } else { sip.hdrend };
        sip.dataend = sip.datastart + sip.segdatalen as i64;
        if !fip.sequentialaccess && (sip.pageinfoflag || sip.endofstripeflag) {
            r.seek(sip.datastart)?;
        }
        let mut seekdist = sip.segdatalen as i64;
        // `pip` is the last page: a segment going back to an earlier page number still lands there.
        let last_page = if in_pages { fip.pages.len() - 1 } else { 0 };
        {
            let pip = if in_pages { &mut fip.pages[last_page] } else { fip.page0.as_mut().unwrap() };
            if sip.pageinfoflag {
                pip.pagenum = sip.segpage as u32;
                pip.width = r.read4bytes()?;
                pip.height = r.read4bytes()?;
                pip.xres = r.read4bytes()?;
                pip.yres = r.read4bytes()?;
                r.getc()?; // pagesegmentflags
                pip.stripinginfo = r.read2bytes()?;
                seekdist -= 19;
            }
            if sip.endofstripeflag {
                pip.stripedheight = r.read4bytes()?;
                seekdist -= 4;
            }
        }
        if !fip.sequentialaccess && (sip.pageinfoflag || sip.endofstripeflag) {
            r.seek(sip.hdrend)?;
        }
        if !fip.sequentialaccess {
            streampos += sip.segdatalen as i64;
        }
        if fip.sequentialaccess {
            r.seek(r.pos + seekdist)?;
        }
        let pip = if in_pages { &mut fip.pages[last_page] } else { fip.page0.as_mut().unwrap() };
        if sip.endofpageflag && currentpage != 0 && (pip.stripinginfo >> 15) != 0 {
            pip.height = pip.stripedheight;
        }
        if !sip.endofpageflag {
            pip.segments.push(sip);
        }
    }
    Ok(fip)
}

/// `find_pageinfo`: the AVL tree keeps the first of equal keys.
fn find_page(pages: &[PageInfo], pagenum: u32) -> Option<usize> {
    pages.iter().position(|p| p.pagenum == pagenum)
}

/// `read_jbig2_info`, given the file's record.
pub fn read_jbig2_info(fip: &Jbig2File, selected_page: i32) -> Result<Jbig2Info, Fail> {
    if selected_page < 1 {
        return Err(Fail::msg(
            format!("read_jbig2_info(): page {selected_page} not in JBIG2 image file; page must be > 0").into_bytes(),
        ));
    }
    let Some(i) = find_page(&fip.pages, selected_page as u32) else {
        return Err(Fail::msg(format!("read_jbig2_info(): page {selected_page} not found in JBIG2 image file").into_bytes()));
    };
    let pip = &fip.pages[i];
    Ok(Jbig2Info {
        pages: fip.numofpages as i32,
        width: pip.width as i32,
        height: pip.height as i32,
        x_res: (pip.xres as f64 * 0.0254 + 0.5) as i32,
        y_res: (pip.yres as f64 * 0.0254 + 0.5) as i32,
    })
}

/// `markpage0seg`.
fn markpage0seg(page0: &mut PageInfo, r: &mut Rd<'_>, referedseg: u32) -> Result<(), Fail> {
    if let Some(i) = page0.segments.iter().position(|s| s.segnum == referedseg) {
        if !page0.segments[i].refers && page0.segments[i].countofrefered > 0 {
            checkseghdr(page0, r, i)?;
        }
        page0.segments[i].isrefered = true;
    }
    Ok(())
}

/// `checkseghdr`, for recursive marking of referred page 0 segments.
fn checkseghdr(page0: &mut PageInfo, r: &mut Rd<'_>, i: usize) -> Result<(), Fail> {
    let (fieldlen, count, width, assoc) = {
        let s = &page0.segments[i];
        (s.fieldlen, s.countofrefered, s.segnumwidth, s.pageassocsizeflag)
    };
    r.seek(r.pos + fieldlen as i64 + 5)?;
    for _ in 0..count {
        let referedseg = match width {
            1 => r.getc()?,
            2 => r.read2bytes()?,
            _ => r.read4bytes()?,
        };
        if !page0.segments[i].refers {
            markpage0seg(page0, r, referedseg)?;
        }
    }
    if count > 0 {
        page0.segments[i].refers = true;
    }
    r.seek(r.pos + if assoc { 8 } else { 5 })?;
    Ok(())
}

/// `wr_jbig2`.
fn wr_jbig2(fip: &mut Jbig2File, data: &[u8], page: u32, slots: &mut SlotAlloc) -> Result<Vec<Op>, Fail> {
    let mut w = Writer::new(slots);
    let page_index = if page > 0 {
        let i = find_page(&fip.pages, page).ok_or_else(|| Fail::new("write_jbig2(): page not found"))?;
        let (width, height, len) = {
            let pip = &fip.pages[i];
            (pip.width as i32, pip.height as i32, getstreamlen(&pip.segments, true))
        };
        w.puts(b"/Type /XObject\n");
        w.puts(b"/Subtype /Image\n");
        w.printf(format_args!("/Width {width}\n"));
        w.printf(format_args!("/Height {height}\n"));
        w.puts(b"/ColorSpace /DeviceGray\n");
        w.puts(b"/BitsPerComponent 1\n");
        w.printf(format_args!("/Length {len}\n"));
        w.puts(b"/Filter [/JBIG2Decode]\n");
        if fip.page0.is_some() {
            let slot = match fip.pdfpage0 {
                Some(s) => s,
                None => {
                    let s = w.new_obj();
                    fip.pdfpage0 = Some(s);
                    s
                }
            };
            w.puts(b"/DecodeParms [<< /JBIG2Globals ");
            w.num(slot);
            w.puts(b" 0 R >>]\n");
        }
        i
    } else {
        // A file read but never written has no object, and the C opens object 0.
        let obj = match fip.pdfpage0 {
            Some(s) => ObjNo::Slot(s),
            None => ObjNo::Known(0),
        };
        w.op(Op::BeginDict { obj, level: 0 });
        let len = getstreamlen(&fip.page0.as_ref().unwrap().segments, false);
        w.printf(format_args!("/Length {len}\n"));
        0
    };
    w.puts(b">>\n");
    w.puts(b"stream\n");
    let mut r = Rd { d: data, pos: 0 };
    let nsegs = if page > 0 { fip.pages[page_index].segments.len() } else { fip.page0.as_ref().unwrap().segments.len() };
    for s in 0..nsegs {
        let isrefered = if page > 0 {
            fip.pages[page_index].segments[s].isrefered
        } else {
            fip.page0.as_ref().unwrap().segments[s].isrefered
        };
        if isrefered || page > 0 {
            let hdrstart = if page > 0 {
                fip.pages[page_index].segments[s].hdrstart
            } else {
                fip.page0.as_ref().unwrap().segments[s].hdrstart
            };
            r.seek(hdrstart)?;
            writeseghdr(fip, &mut r, &mut w, page, page_index, s)?;
            let sip =
                if page > 0 { &fip.pages[page_index].segments[s] } else { &fip.page0.as_ref().unwrap().segments[s] };
            let (datastart, dataend) = (sip.datastart, sip.dataend);
            r.seek(datastart)?;
            let mut i = datastart;
            while i < dataend {
                let c = r.getc()?;
                w.out(c as u8);
                i += 1;
            }
        }
    }
    w.op(Op::EndStream);
    Ok(w.ops)
}

/// `writeseghdr`: marks referred page 0 segments and sets segment pages above 0 to 1.
fn writeseghdr(
    fip: &mut Jbig2File,
    r: &mut Rd<'_>,
    w: &mut Writer<'_>,
    page: u32,
    page_index: usize,
    s: usize,
) -> Result<(), Fail> {
    let sip = if page > 0 { &fip.pages[page_index].segments[s] } else { &fip.page0.as_ref().unwrap().segments[s] };
    let (fieldlen, count, width, assoc, segpage) =
        (sip.fieldlen, sip.countofrefered, sip.segnumwidth, sip.pageassocsizeflag, sip.segpage);
    for _ in 0..5 + fieldlen {
        let c = r.getc()?;
        w.out(c as u8);
    }
    for _ in 0..count {
        let referedseg = match width {
            1 => {
                let v = r.getc()?;
                w.out(v as u8);
                v
            }
            2 => {
                let v = r.read2bytes()?;
                w.out((v >> 8) as u8);
                w.out(v as u8);
                v
            }
            _ => {
                let v = r.read4bytes()?;
                w.out((v >> 24) as u8);
                w.out((v >> 16) as u8);
                w.out((v >> 8) as u8);
                w.out(v as u8);
                v
            }
        };
        let refers = if page > 0 {
            fip.pages[page_index].segments[s].refers
        } else {
            fip.page0.as_ref().unwrap().segments[s].refers
        };
        if fip.page0.is_some() && !refers {
            markpage0seg(fip.page0.as_mut().unwrap(), r, referedseg)?;
        }
    }
    if count > 0 {
        if page > 0 {
            fip.pages[page_index].segments[s].refers = true;
        } else {
            fip.page0.as_mut().unwrap().segments[s].refers = true;
        }
    }
    if assoc {
        for _ in 0..3 {
            r.getc()?;
            w.out(0);
        }
    }
    r.getc()?;
    w.out(if segpage > 0 { 1 } else { 0 });
    for _ in 0..4 {
        let c = r.getc()?;
        w.out(c as u8);
    }
    Ok(())
}

/// `write_jbig2`.  The engine has called `pdf_begin_dict` and printed the attributes.
pub fn write_jbig2(fip: &mut Jbig2File, data: &[u8], selected_page: i32, slots: &mut SlotAlloc) -> Result<Vec<Op>, Fail> {
    let i = find_page(&fip.pages, selected_page as u32).ok_or_else(|| Fail::new("write_jbig2(): page not found"))?;
    let pagenum = fip.pages[i].pagenum;
    wr_jbig2(fip, data, pagenum, slots)
}

/// `flushjbig2page0objects`, for one file: empty when the file has no page 0.
pub fn flush_page0(fip: &mut Jbig2File, data: &[u8], slots: &mut SlotAlloc) -> Result<Vec<Op>, Fail> {
    if fip.page0.is_some() { wr_jbig2(fip, data, 0, slots) } else { Ok(Vec::new()) }
}
