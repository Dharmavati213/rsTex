//! A PDF reader, following xpdf 4.06: Object, Dict, Array, Lexer, Parser, XRef, Catalog, Page and PDFDoc.
//!
//! The oracle's binary contains xpdf 4.06 statically (`ldd` shows no poppler; `pdftex --version` says "Compiled with
//! xpdf version 4.06"; the strings carry `libxpdf:`), from `libs/xpdf` of the same TeX Live tag.  So this follows that
//! source, damaged-file recovery included, because what it finds in a file is what gets copied to the output.
//!
//! What differs from it:
//! - Objects are not cached by number.  xpdf's 16-entry cache only saves parsing; object streams are cached.
//! - Streams decode eagerly and only when asked (`Doc::decode_stream`), and filters are looked up then.  xpdf's decoders
//!   never fail, so the bytes are the same.  CCITTFax, DCT, JBIG2 and JPX are not decoded: `Decoded::unsupported` names
//!   the filter.  They can only matter inside a page's content array or a font file; every other stream is copied raw.
//! - Encrypted files are refused: `OpenError::Encrypted`.
//! - Warnings on stderr, which do not reach the log, are not produced.

use crate::filters;
use std::collections::HashMap;
use std::sync::Arc;

const OBJECT_RECURSION_LIMIT: i32 = 500;
const XREF_SEARCH_SIZE: usize = 1024;
const HEADER_SEARCH_SIZE: usize = 1024;
const TOK_BUF_SIZE: usize = 128;

/// `specialChars`: 1 is white space; 1 or 2 ends a name or command.
const SPECIAL: [u8; 256] = {
    let mut t = [0u8; 256];
    t[0] = 1;
    t[9] = 1;
    t[10] = 1;
    t[12] = 1;
    t[13] = 1;
    t[32] = 1;
    t[b'%' as usize] = 2;
    t[b'(' as usize] = 2;
    t[b')' as usize] = 2;
    t[b'/' as usize] = 2;
    t[b'<' as usize] = 2;
    t[b'>' as usize] = 2;
    t[b'[' as usize] = 2;
    t[b']' as usize] = 2;
    t[b'{' as usize] = 2;
    t[b'}' as usize] = 2;
    t
};

/// `Object`.
#[derive(Clone, Debug)]
pub enum Object {
    Bool(bool),
    Int(i32),
    Real(f64),
    Str(Arc<Vec<u8>>),
    Name(Arc<Vec<u8>>),
    Null,
    Array(Arc<Vec<Object>>),
    Dict(Arc<Dict>),
    Stream(Arc<StreamObj>),
    Ref(i32, i32),
    Cmd(Vec<u8>),
    Error,
    Eof,
    /// `objNone`, an uninitialised object: the trailer before one is found.
    None,
}

impl Object {
    fn is_cmd(&self, s: &[u8]) -> bool {
        matches!(self, Object::Cmd(c) if c == s)
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Object::Null)
    }

    pub fn is_name(&self, s: &[u8]) -> bool {
        matches!(self, Object::Name(n) if n.as_slice() == s)
    }

    pub fn as_num(&self) -> Option<f64> {
        match self {
            Object::Int(i) => Some(*i as f64),
            Object::Real(r) => Some(*r),
            _ => None,
        }
    }

    /// `getTypeName`, for the messages of `pdftex_fail`.
    pub fn type_name(&self) -> &'static str {
        match self {
            Object::Bool(_) => "boolean",
            Object::Int(_) => "integer",
            Object::Real(_) => "real",
            Object::Str(_) => "string",
            Object::Name(_) => "name",
            Object::Null => "null",
            Object::Array(_) => "array",
            Object::Dict(_) => "dictionary",
            Object::Stream(_) => "stream",
            Object::Ref(..) => "ref",
            Object::Cmd(_) => "cmd",
            Object::Error => "error",
            Object::Eof => "eof",
            Object::None => "none",
        }
    }
}

/// `Dict`: entries in file order; a repeated key keeps its place and takes the later value, as `Dict::add` does.
#[derive(Clone, Debug, Default)]
pub struct Dict {
    entries: Vec<(Vec<u8>, Object)>,
    index: HashMap<Vec<u8>, usize>,
}

/// Below this a dictionary is searched.  Above, it has a hash table: xpdf hashes all of them.
const DICT_INDEX_MIN: usize = 32;

impl Dict {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn position(&self, key: &[u8]) -> Option<usize> {
        if self.entries.len() > DICT_INDEX_MIN {
            self.index.get(key).copied()
        } else {
            self.entries.iter().position(|(k, _)| k == key)
        }
    }

    pub(crate) fn add(&mut self, key: Vec<u8>, val: Object) {
        if let Some(i) = self.position(&key) {
            self.entries[i].1 = val;
            return;
        }
        let n = self.entries.len();
        if n == DICT_INDEX_MIN {
            for (i, (k, _)) in self.entries.iter().enumerate() {
                self.index.insert(k.clone(), i);
            }
        }
        if n >= DICT_INDEX_MIN {
            self.index.insert(key.clone(), n);
        }
        self.entries.push((key, val));
    }

    /// `lookupNF`.
    pub fn get_nf(&self, key: &[u8]) -> Object {
        match self.position(key) {
            Some(i) => self.entries[i].1.clone(),
            None => Object::Null,
        }
    }

    /// `getKey`.
    pub fn key(&self, i: usize) -> &[u8] {
        &self.entries[i].0
    }

    /// `getValNF`.
    pub fn val_nf(&self, i: usize) -> Object {
        self.entries[i].1.clone()
    }
}

/// A stream object: its dictionary, and where the raw bytes are in the file.
#[derive(Debug)]
pub struct StreamObj {
    pub dict: Arc<Dict>,
    /// `pos`, an absolute position in the file.
    pub start: usize,
    /// `length`.  The bytes stop at the end of the file if that comes first.
    pub length: u64,
    /// Whether the dictionary's references can be followed: not for one read before the xref table exists.
    has_xref: bool,
}

/// A stream through its filters.
#[derive(Debug)]
pub struct Decoded {
    pub data: Vec<u8>,
    /// The first filter of the chain this crate cannot decode.
    pub unsupported: Option<Vec<u8>>,
}

//------------------------------------------------------------------------
// Lexer
//------------------------------------------------------------------------

struct Lexer {
    data: Arc<[u8]>,
    pos: u64,
    end: u64,
    /// `curStr.isNone()`: a `getChar` has run off the end.
    exhausted: bool,
}

impl Lexer {
    fn new(data: Arc<[u8]>, pos: u64, end: u64) -> Self {
        Lexer { data, pos, end, exhausted: false }
    }

    fn get_char(&mut self) -> i32 {
        if self.exhausted {
            return -1;
        }
        if self.pos < self.end {
            let c = self.data[self.pos as usize];
            self.pos += 1;
            c as i32
        } else {
            self.exhausted = true;
            -1
        }
    }

    fn look_char(&self) -> i32 {
        if !self.exhausted && self.pos < self.end { self.data[self.pos as usize] as i32 } else { -1 }
    }

    fn skip_to_next_line(&mut self) {
        loop {
            let c = self.get_char();
            if c == -1 || c == b'\n' as i32 {
                return;
            }
            if c == b'\r' as i32 {
                if self.look_char() == b'\n' as i32 {
                    self.get_char();
                }
                return;
            }
        }
    }

    /// `Lexer::getObj`.
    fn get_obj(&mut self) -> Object {
        let mut comment = false;
        let mut c;
        loop {
            c = self.get_char();
            if c == -1 {
                return Object::Eof;
            }
            if comment {
                if c == b'\r' as i32 || c == b'\n' as i32 {
                    comment = false;
                }
            } else if c == b'%' as i32 {
                comment = true;
            } else if SPECIAL[c as usize] != 1 {
                break;
            }
        }
        let ch = c as u8;
        match ch {
            b'0'..=b'9' | b'+' | b'-' | b'.' => self.number(ch),
            b'(' => self.string(),
            b'/' => self.name(),
            b'[' | b']' => Object::Cmd(vec![ch]),
            b'<' => {
                if self.look_char() == b'<' as i32 {
                    self.get_char();
                    Object::Cmd(b"<<".to_vec())
                } else {
                    self.hex_string()
                }
            }
            b'>' => {
                if self.look_char() == b'>' as i32 {
                    self.get_char();
                    Object::Cmd(b">>".to_vec())
                } else {
                    Object::Error
                }
            }
            b')' | b'{' | b'}' => Object::Error,
            _ => {
                let mut tok = vec![ch];
                let mut n = 1;
                loop {
                    let c = self.look_char();
                    if c == -1 || SPECIAL[c as usize] != 0 {
                        break;
                    }
                    self.get_char();
                    n += 1;
                    if n == TOK_BUF_SIZE {
                        break;
                    }
                    tok.push(c as u8);
                }
                match tok.as_slice() {
                    b"true" => Object::Bool(true),
                    b"false" => Object::Bool(false),
                    b"null" => Object::Null,
                    _ => Object::Cmd(tok),
                }
            }
        }
    }

    fn number(&mut self, first: u8) -> Object {
        let is_digit = |c: i32| (b'0' as i32..=b'9' as i32).contains(&c);
        let mut neg = false;
        let mut double_minus = false;
        let mut xf = 0.0f64;
        let mut xi = 0i32;
        let mut real = false;
        if first == b'+' {
            // just ignore it
        } else if first == b'-' {
            neg = true;
            if self.look_char() == b'-' as i32 {
                double_minus = true;
                loop {
                    self.get_char();
                    if self.look_char() != b'-' as i32 {
                        break;
                    }
                }
            }
        } else if first == b'.' {
            real = true;
        } else {
            xi = (first - b'0') as i32;
            xf = xi as f64;
        }
        if !real {
            loop {
                let c = self.look_char();
                if is_digit(c) {
                    self.get_char();
                    xi = xi.wrapping_mul(10).wrapping_add(c - b'0' as i32);
                    if xf < 1e20 {
                        xf = xf * 10.0 + (c - b'0' as i32) as f64;
                    }
                } else if c == b'.' as i32 {
                    self.get_char();
                    real = true;
                    break;
                } else {
                    break;
                }
            }
        }
        if !real {
            loop {
                let c = self.look_char();
                if c == b'-' as i32 || is_digit(c) {
                    self.get_char();
                } else {
                    break;
                }
            }
            if neg {
                xi = xi.wrapping_neg();
            }
            if double_minus {
                xi = 0;
            }
            return Object::Int(xi);
        }
        let mut scale = 0.1f64;
        loop {
            let c = self.look_char();
            if c == b'-' as i32 {
                self.get_char();
                continue;
            }
            if !is_digit(c) {
                break;
            }
            self.get_char();
            xf += scale * (c - b'0' as i32) as f64;
            scale *= 0.1;
        }
        loop {
            let c = self.look_char();
            if c == b'-' as i32 || is_digit(c) {
                self.get_char();
            } else {
                break;
            }
        }
        if neg {
            xf = -xf;
        }
        Object::Real(xf)
    }

    fn string(&mut self) -> Object {
        let mut s = Vec::new();
        let mut num_paren = 1;
        let mut done = false;
        while !done {
            let mut c2 = -1;
            let mut c = self.get_char();
            match c {
                -1 => done = true,
                0x28 => {
                    num_paren += 1;
                    c2 = c;
                }
                0x29 => {
                    num_paren -= 1;
                    if num_paren == 0 {
                        done = true;
                    } else {
                        c2 = c;
                    }
                }
                0x0d => {
                    if self.look_char() == b'\n' as i32 {
                        self.get_char();
                    }
                    c2 = b'\n' as i32;
                }
                0x5c => {
                    c = self.get_char();
                    match c {
                        0x6e => c2 = b'\n' as i32,
                        0x72 => c2 = b'\r' as i32,
                        0x74 => c2 = b'\t' as i32,
                        0x62 => c2 = 8,
                        0x66 => c2 = 12,
                        0x5c | 0x28 | 0x29 => c2 = c,
                        0x30..=0x37 => {
                            c2 = c - 0x30;
                            let c3 = self.look_char();
                            if (0x30..=0x37).contains(&c3) {
                                self.get_char();
                                c2 = (c2 << 3) + (c3 - 0x30);
                                let c4 = self.look_char();
                                if (0x30..=0x37).contains(&c4) {
                                    self.get_char();
                                    c2 = (c2 << 3) + (c4 - 0x30);
                                }
                            }
                        }
                        0x0d => {
                            if self.look_char() == b'\n' as i32 {
                                self.get_char();
                            }
                        }
                        0x0a => {}
                        -1 => done = true,
                        _ => c2 = c,
                    }
                }
                _ => c2 = c,
            }
            if c2 != -1 {
                s.push(c2 as u8);
            }
        }
        Object::Str(Arc::new(s))
    }

    fn name(&mut self) -> Object {
        fn hex(c: i32) -> Option<i32> {
            match c {
                0x30..=0x39 => Some(c - 0x30),
                0x41..=0x46 => Some(c - 0x41 + 10),
                0x61..=0x66 => Some(c - 0x61 + 10),
                _ => None,
            }
        }
        let mut s = Vec::new();
        let mut invalid = false;
        loop {
            let mut c = self.look_char();
            if c == -1 || SPECIAL[c as usize] != 0 {
                break;
            }
            self.get_char();
            if c == b'#' as i32 {
                if let Some(h1) = hex(self.look_char()) {
                    c = h1;
                    self.get_char();
                    if let Some(h2) = hex(self.look_char()) {
                        c = (c << 4) + h2;
                        self.get_char();
                        if c == 0 {
                            invalid = true;
                        }
                    }
                }
            }
            s.push(c as u8);
        }
        if invalid { Object::Error } else { Object::Name(Arc::new(s)) }
    }

    fn hex_string(&mut self) -> Object {
        let mut s = Vec::new();
        let (mut m, mut c2) = (0, 0i32);
        let mut n_errors = 0;
        while n_errors < 100 {
            let c = self.get_char();
            if c == b'>' as i32 || c == -1 {
                break;
            }
            if SPECIAL[c as usize] != 1 {
                c2 = c2.wrapping_shl(4);
                match c {
                    0x30..=0x39 => c2 += c - 0x30,
                    0x41..=0x46 => c2 += c - 0x41 + 10,
                    0x61..=0x66 => c2 += c - 0x61 + 10,
                    _ => n_errors += 1,
                }
                m += 1;
                if m == 2 {
                    s.push(c2 as u8);
                    c2 = 0;
                    m = 0;
                }
            }
        }
        if m == 1 {
            s.push((c2 << 4) as u8);
        }
        Object::Str(Arc::new(s))
    }
}

//------------------------------------------------------------------------
// Parser
//------------------------------------------------------------------------

struct Parser {
    lexer: Lexer,
    buf1: Object,
    buf2: Object,
    inline_img: i32,
    allow_streams: bool,
}

impl Parser {
    fn new(mut lexer: Lexer, allow_streams: bool) -> Self {
        let buf1 = lexer.get_obj();
        let buf2 = lexer.get_obj();
        Parser { lexer, buf1, buf2, inline_img: 0, allow_streams }
    }

    fn shift(&mut self) {
        if self.inline_img > 0 {
            if self.inline_img < 2 {
                self.inline_img += 1;
            } else {
                self.inline_img = 0;
            }
        } else if self.buf2.is_cmd(b"ID") {
            self.lexer.get_char();
            self.inline_img = 1;
        }
        self.buf1 = std::mem::replace(&mut self.buf2, Object::Null);
        if self.inline_img <= 0 {
            self.buf2 = self.lexer.get_obj();
        }
    }

    /// `Parser::getObj`.  `doc` is the `xref` argument, absent while the xref itself is being read.
    fn get_obj(&mut self, mut doc: Option<&mut Doc>, simple_only: bool, recursion: i32) -> Object {
        if self.inline_img == 2 {
            self.buf1 = self.lexer.get_obj();
            self.buf2 = self.lexer.get_obj();
            self.inline_img = 0;
        }
        if !simple_only && recursion < OBJECT_RECURSION_LIMIT && self.buf1.is_cmd(b"[") {
            self.shift();
            let mut a = Vec::new();
            while !self.buf1.is_cmd(b"]") && !matches!(self.buf1, Object::Eof) {
                a.push(self.get_obj(doc.as_deref_mut(), false, recursion + 1));
            }
            self.shift();
            Object::Array(Arc::new(a))
        } else if !simple_only && recursion < OBJECT_RECURSION_LIMIT && self.buf1.is_cmd(b"<<") {
            self.shift();
            let mut d = Dict::default();
            while !self.buf1.is_cmd(b">>") && !matches!(self.buf1, Object::Eof) {
                if let Object::Name(key) = &self.buf1 {
                    let key = key.as_ref().clone();
                    self.shift();
                    if matches!(self.buf1, Object::Eof | Object::Error) {
                        break;
                    }
                    let v = self.get_obj(doc.as_deref_mut(), false, recursion + 1);
                    d.add(key, v);
                } else {
                    self.shift();
                }
            }
            let dict = Arc::new(d);
            if self.allow_streams && self.buf2.is_cmd(b"stream") {
                match self.make_stream(doc, &dict, recursion + 1) {
                    Some(s) => Object::Stream(Arc::new(s)),
                    None => Object::Error,
                }
            } else {
                self.shift();
                Object::Dict(dict)
            }
        } else if let Object::Int(num) = self.buf1 {
            self.shift();
            if let Object::Int(gno) = self.buf1 {
                if self.buf2.is_cmd(b"R") {
                    let r = if num >= 0 && gno >= 0 { Object::Ref(num, gno) } else { Object::Error };
                    self.shift();
                    self.shift();
                    return r;
                }
            }
            Object::Int(num)
        } else {
            let o = self.buf1.clone();
            self.shift();
            o
        }
    }

    /// `Parser::makeStream`.
    fn make_stream(&mut self, doc: Option<&mut Doc>, dict: &Arc<Dict>, recursion: i32) -> Option<StreamObj> {
        self.lexer.skip_to_next_line();
        if self.lexer.exhausted {
            return None;
        }
        let pos = self.lexer.pos;
        let has_xref = doc.is_some();
        let mut have_length = false;
        let mut length: u64 = 0;
        let stream_end = doc.as_ref().and_then(|d| d.get_stream_end(pos));
        if let Some(e) = stream_end {
            length = e - pos;
            have_length = true;
        } else {
            let obj = match doc {
                Some(d) => d.lookup_r(dict, b"Length", recursion),
                None => dict.get_nf(b"Length"),
            };
            if let Object::Int(n) = obj {
                length = n as u32 as u64;
                have_length = true;
            }
        }
        if self.lexer.exhausted {
            return None;
        }
        let end = self.lexer.end;
        if !have_length {
            // 'Length' attribute is missing: search for 'endstream', as curStr->getChar and getBlock do.
            let mut p = pos;
            let mut found = false;
            while p < end {
                let c = self.lexer.data[p as usize];
                p += 1;
                if c == b'e' {
                    let n = (end - p).min(8);
                    let block = &self.lexer.data[p as usize..(p + n) as usize];
                    let hit = block == b"ndstream";
                    p += n;
                    if hit {
                        length = p - 9 - pos;
                        found = true;
                        break;
                    }
                }
            }
            if !found {
                return None;
            }
        } else {
            // Check for 'endstream' directly, skipping up to 100 white space characters.
            let mut p = pos.saturating_add(length);
            let mut c = -1;
            for _ in 0..100 {
                c = if p < end {
                    let v = self.lexer.data[p as usize] as i32;
                    p += 1;
                    v
                } else {
                    -1
                };
                if !filters::lexer_is_space(c) {
                    break;
                }
            }
            let mut found = false;
            if c == b'e' as i32 && p.saturating_add(8) <= end {
                found = &self.lexer.data[p as usize..(p + 8) as usize] == b"ndstream";
            }
            if !found {
                // kludge for broken PDF files: just add 5k to the length, and hope it's enough
                length += 5000;
            }
        }
        Some(StreamObj { dict: dict.clone(), start: pos as usize, length, has_xref })
    }
}

//------------------------------------------------------------------------
// XRef and PDFDoc
//------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EntryType {
    Free,
    Uncompressed,
    Compressed,
}

#[derive(Clone, Copy, Debug)]
struct XRefEntry {
    offset: i64,
    gno: i32,
    ty: EntryType,
}

const FREE_ENTRY: XRefEntry = XRefEntry { offset: -1, gno: 0, ty: EntryType::Free };

struct ObjectStream {
    nums: Vec<i32>,
    objs: Vec<Object>,
}

/// Why a file could not be opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// `!isOk()`: `pdftex_fail("xpdf: reading PDF image failed")`.
    Bad,
    /// A file with `/Encrypt`.  xpdf opens one when the empty password works; this crate does not.
    Encrypted,
}

/// A page's attributes: `PageAttrs`.
#[derive(Clone, Debug)]
pub struct PageAttrs {
    pub media_box: [f64; 4],
    pub crop_box: [f64; 4],
    have_crop_box: bool,
    pub bleed_box: [f64; 4],
    pub trim_box: [f64; 4],
    pub art_box: [f64; 4],
    pub rotate: i32,
    pub group: Object,
    pub resources: Object,
}

impl Default for PageAttrs {
    /// `PageAttrs::PageAttrs()`, of a page that could not be read.
    fn default() -> Self {
        let b = [0.0, 0.0, 50.0, 50.0];
        PageAttrs {
            media_box: b,
            crop_box: b,
            have_crop_box: false,
            bleed_box: b,
            trim_box: b,
            art_box: b,
            rotate: 0,
            group: Object::Null,
            resources: Object::Null,
        }
    }
}

fn clip_to(b: &mut [f64; 4], r: &[f64; 4]) {
    if b[0] < r[0] {
        b[0] = r[0];
    } else if b[0] > r[2] {
        b[0] = r[2];
    }
    if b[2] < r[0] {
        b[2] = r[0];
    } else if b[2] > r[2] {
        b[2] = r[2];
    }
    if b[1] < r[1] {
        b[1] = r[1];
    } else if b[1] > r[3] {
        b[1] = r[3];
    }
    if b[3] < r[1] {
        b[3] = r[1];
    } else if b[3] > r[3] {
        b[3] = r[3];
    }
}

/// `Page`.
#[derive(Clone, Debug)]
pub struct Page {
    pub attrs: PageAttrs,
    /// `/Contents`, not fetched.
    pub contents: Object,
}

struct PageTreeNode {
    r: (i32, i32),
    count: i32,
    parent: Option<usize>,
    kids: Option<Vec<usize>>,
    attrs: Option<PageAttrs>,
}

/// `PDFDoc`, `XRef` and `Catalog` in one.
pub struct Doc {
    data: Arc<[u8]>,
    start: u64,
    entries: Vec<XRefEntry>,
    last: i32,
    trailer: Object,
    root: (i32, i32),
    stream_ends: Vec<u64>,
    objstrs: HashMap<i32, Arc<ObjectStream>>,
    pdf_version: f64,
    nodes: Vec<PageTreeNode>,
    num_pages: i32,
    pages: Vec<Option<Page>>,
    page_refs: Vec<(i32, i32)>,
    dests: Object,
    name_tree: Object,
}

impl Doc {
    /// `new PDFDoc(...)` with `isOk()`.
    pub fn open(data: Vec<u8>) -> Result<Doc, OpenError> {
        let mut doc = Doc {
            data: data.into(),
            start: 0,
            entries: Vec::new(),
            last: -1,
            trailer: Object::None,
            root: (0, 0),
            stream_ends: Vec::new(),
            objstrs: HashMap::new(),
            pdf_version: 0.0,
            nodes: Vec::new(),
            num_pages: 0,
            pages: Vec::new(),
            page_refs: Vec::new(),
            dests: Object::Null,
            name_tree: Object::Null,
        };
        doc.check_header();
        match doc.setup2(false) {
            Ok(()) => {}
            Err(OpenError::Encrypted) => return Err(OpenError::Encrypted),
            Err(OpenError::Bad) => {
                doc.reset_xref();
                doc.setup2(true)?;
            }
        }
        Ok(doc)
    }

    /// `getPDFVersion`.
    pub fn pdf_version(&self) -> f64 {
        self.pdf_version
    }

    /// `getCatalog()->getNumPages()`.
    pub fn num_pages(&self) -> i32 {
        self.num_pages
    }

    /// The whole file, for `CopyInput`.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    fn file_len(&self) -> u64 {
        self.data.len() as u64
    }

    fn reset_xref(&mut self) {
        self.entries.clear();
        self.last = -1;
        self.trailer = Object::None;
        self.root = (0, 0);
        self.stream_ends.clear();
        self.objstrs.clear();
        self.nodes.clear();
        self.num_pages = 0;
        self.pages.clear();
        self.page_refs.clear();
    }

    /// `PDFDoc::checkHeader`.
    fn check_header(&mut self) {
        let mut hdr = [0u8; HEADER_SEARCH_SIZE + 1];
        let n = self.data.len().min(HEADER_SEARCH_SIZE);
        hdr[..n].copy_from_slice(&self.data[..n]);
        let mut i = 0;
        while i < HEADER_SEARCH_SIZE - 5 {
            if &hdr[i..i + 5] == b"%PDF-" {
                break;
            }
            i += 1;
        }
        if i >= HEADER_SEARCH_SIZE - 5 {
            return;
        }
        self.start = i as u64;
        // strtok(&hdrBuf[i+5], " \t\n\r"), then atof.
        let rest = &hdr[i + 5..];
        let rest = &rest[..rest.iter().position(|&c| c == 0).unwrap_or(rest.len())];
        let is_delim = |c: u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r');
        let Some(s) = rest.iter().position(|&c| !is_delim(c)) else {
            return;
        };
        let rest = &rest[s..];
        let e = rest.iter().position(|&c| is_delim(c)).unwrap_or(rest.len());
        self.pdf_version = atof(&rest[..e]);
    }

    /// `PDFDoc::setup2`: the xref, encryption, the catalog.
    fn setup2(&mut self, repair: bool) -> Result<(), OpenError> {
        if !self.xref_init(repair) {
            return Err(OpenError::Bad);
        }
        let encrypt = self.trailer_lookup(b"Encrypt");
        if matches!(encrypt, Object::Dict(_)) {
            return Err(OpenError::Encrypted);
        }
        // Catalog::Catalog
        let cat = self.fetch(self.root.0, self.root.1, 0);
        let Object::Dict(cat) = cat else {
            return Err(OpenError::Bad);
        };
        if !self.read_page_tree(&cat) {
            return Err(OpenError::Bad);
        }
        self.dests = self.lookup(&cat, b"Dests");
        self.name_tree = match self.lookup(&cat, b"Names") {
            Object::Dict(n) => self.lookup(&n, b"Dests"),
            _ => Object::Null,
        };
        Ok(())
    }

    /// `XRef::XRef`.
    fn xref_init(&mut self, repair: bool) -> bool {
        if repair {
            if !self.construct_xref() {
                return false;
            }
        } else {
            let mut pos = match self.get_start_xref() {
                0 => return false,
                p => p,
            };
            let mut pos_set = Vec::new();
            let mut ok = true;
            while self.read_xref(&mut pos, &mut pos_set, false, &mut ok) {}
            if !ok {
                return false;
            }
        }
        let root = match &self.trailer {
            Object::Dict(d) => d.get_nf(b"Root"),
            _ => Object::Null,
        };
        if let Object::Ref(n, g) = root {
            self.root = (n, g);
        } else if !self.construct_xref() {
            return false;
        }
        true
    }

    /// `XRef::getStartXref`.
    fn get_start_xref(&self) -> u64 {
        let size = self.data.len();
        let from = size.saturating_sub(XREF_SEARCH_SIZE);
        let buf = &self.data[from..];
        let n = buf.len();
        let mut i = n as i64 - 9;
        while i >= 0 {
            if &buf[i as usize..i as usize + 9] == b"startxref" {
                break;
            }
            i -= 1;
        }
        if i < 0 {
            return 0;
        }
        let mut p = i as usize + 9;
        while p < n && matches!(buf[p], 0x20 | 0x09..=0x0d) {
            p += 1;
        }
        // strToFileOffset
        let mut x: u64 = 0;
        while p < n && buf[p].is_ascii_digit() {
            let d = (buf[p] - b'0') as u64;
            if x > (i64::MAX as u64 - d) / 10 {
                break;
            }
            x = 10 * x + d;
            p += 1;
        }
        x
    }

    fn file_lexer(&self, pos: u64) -> Lexer {
        Lexer::new(self.data.clone(), pos, self.file_len())
    }

    /// `XRef::readXRef`.
    fn read_xref(&mut self, pos: &mut u64, pos_set: &mut Vec<u64>, hybrid: bool, ok: &mut bool) -> bool {
        if pos_set.contains(pos) {
            return false;
        }
        pos_set.push(*pos);
        let at = self.start.saturating_add(*pos);
        let n = self.file_len().saturating_sub(at).min(100) as usize;
        let buf: &[u8] = if n > 0 { &self.data[at as usize..at as usize + n] } else { &[] };
        let mut i = 0;
        while i < n && filters::lexer_is_space(buf[i] as i32) {
            i += 1;
        }
        if !hybrid && i + 4 < n && &buf[i..i + 4] == b"xref" && filters::lexer_is_space(buf[i + 4] as i32) {
            return self.read_xref_table(pos, i + 5, pos_set, ok);
        }
        let mut parser = Parser::new(self.file_lexer(at), true);
        let fail = |ok: &mut bool| {
            if !hybrid {
                *ok = false;
            }
            false
        };
        if !matches!(parser.get_obj(None, true, 0), Object::Int(_)) {
            return fail(ok);
        }
        if !matches!(parser.get_obj(None, true, 0), Object::Int(_)) {
            return fail(ok);
        }
        if !parser.get_obj(None, true, 0).is_cmd(b"obj") {
            return fail(ok);
        }
        let Object::Stream(s) = parser.get_obj(None, false, 0) else {
            return fail(ok);
        };
        self.read_xref_stream(&s, pos, ok)
    }

    fn grow_to(&mut self, n: usize) {
        if n > self.entries.len() {
            self.entries.resize(n, FREE_ENTRY);
        }
    }

    /// `XRef::readXRefTable`.
    fn read_xref_table(&mut self, pos: &mut u64, offset: usize, pos_set: &mut Vec<u64>, ok: &mut bool) -> bool {
        let mut p = self.start.saturating_add(*pos).saturating_add(offset as u64);
        let len = self.file_len();
        let data = self.data.clone();
        let getc = |p: &mut u64| -> i32 {
            if *p < len {
                let c = data[*p as usize] as i32;
                *p += 1;
                c
            } else {
                -1
            }
        };
        let sp = filters::lexer_is_space;
        macro_rules! err1 {
            () => {{
                *ok = false;
                return false;
            }};
        }
        loop {
            let mut c;
            loop {
                c = getc(&mut p);
                if !sp(c) {
                    break;
                }
            }
            if c == b't' as i32 {
                let mut word = [0u8; 6];
                for w in word.iter_mut() {
                    let v = getc(&mut p);
                    if v < 0 {
                        err1!();
                    }
                    *w = v as u8;
                }
                if &word != b"railer" {
                    err1!();
                }
                break;
            }
            if !(b'0' as i32..=b'9' as i32).contains(&c) {
                err1!();
            }
            let mut first: i32 = 0;
            loop {
                let digit = c - b'0' as i32;
                if first > (i32::MAX - digit) / 10 {
                    err1!();
                }
                first = first * 10 + digit;
                c = getc(&mut p);
                if !(b'0' as i32..=b'9' as i32).contains(&c) {
                    break;
                }
            }
            if !sp(c) {
                err1!();
            }
            loop {
                c = getc(&mut p);
                if !sp(c) {
                    break;
                }
            }
            let mut n: i32 = 0;
            loop {
                let digit = c - b'0' as i32;
                if digit < -(i32::MAX / 10) || n > i32::MAX.wrapping_sub(digit) / 10 {
                    err1!();
                }
                n = n.wrapping_mul(10).wrapping_add(digit);
                c = getc(&mut p);
                if !(b'0' as i32..=b'9' as i32).contains(&c) {
                    break;
                }
            }
            if !sp(c) {
                err1!();
            }
            if first > i32::MAX - n {
                err1!();
            }
            if first + n > self.entries.len() as i32 {
                let mut new_size: i64 = if self.entries.is_empty() { 512 } else { self.entries.len() as i64 };
                loop {
                    if new_size > (i32::MAX / 2) as i64 {
                        err1!();
                    }
                    new_size <<= 1;
                    if (first + n) as i64 <= new_size {
                        break;
                    }
                }
                self.grow_to(new_size as usize);
            }
            let mut i = first;
            while i < first + n {
                loop {
                    c = getc(&mut p);
                    if !sp(c) {
                        break;
                    }
                }
                let mut off: i64 = 0;
                loop {
                    off = off.wrapping_mul(10).wrapping_add((c - b'0' as i32) as i64);
                    c = getc(&mut p);
                    if !(b'0' as i32..=b'9' as i32).contains(&c) {
                        break;
                    }
                }
                if !sp(c) {
                    err1!();
                }
                loop {
                    c = getc(&mut p);
                    if !sp(c) {
                        break;
                    }
                }
                let mut gno: i32 = 0;
                loop {
                    gno = gno.wrapping_mul(10).wrapping_add(c - b'0' as i32);
                    c = getc(&mut p);
                    if !(b'0' as i32..=b'9' as i32).contains(&c) {
                        break;
                    }
                }
                if !sp(c) {
                    err1!();
                }
                loop {
                    c = getc(&mut p);
                    if !sp(c) {
                        break;
                    }
                }
                let ty = if c == b'n' as i32 {
                    EntryType::Uncompressed
                } else if c == b'f' as i32 {
                    EntryType::Free
                } else {
                    err1!();
                };
                c = getc(&mut p);
                if !sp(c) {
                    err1!();
                }
                if i >= 0 && self.entries[i as usize].offset == -1 {
                    self.entries[i as usize] = XRefEntry { offset: off, gno, ty };
                    // PDF files of patents from the IBM Intellectual Property Network have a bug: the xref table
                    // claims to start at 1 instead of 0.
                    if i == 1
                        && first == 1
                        && self.entries[1].offset == 0
                        && self.entries[1].gno == 65535
                        && self.entries[1].ty == EntryType::Free
                    {
                        i = 0;
                        first = 0;
                        self.entries[0] = self.entries[1];
                        self.entries[1].offset = -1;
                    }
                    if i > self.last {
                        self.last = i;
                    }
                }
                i += 1;
            }
        }
        // read the trailer dictionary
        let mut parser = Parser::new(self.file_lexer(p), true);
        let obj = parser.get_obj(None, false, 0);
        let Object::Dict(d) = obj else {
            err1!();
        };
        let more = match d.get_nf(b"Prev") {
            Object::Int(n) => {
                *pos = n as u32 as u64;
                true
            }
            Object::Ref(n, _) => {
                *pos = n as u32 as u64;
                true
            }
            _ => false,
        };
        if matches!(self.trailer, Object::None) {
            self.trailer = Object::Dict(d.clone());
        }
        if let Object::Int(n) = d.get_nf(b"XRefStm") {
            let mut pos2 = n as u32 as u64;
            self.read_xref(&mut pos2, pos_set, true, ok);
            if !*ok {
                err1!();
            }
        }
        more
    }

    /// `XRef::readXRefStream`.
    fn read_xref_stream(&mut self, s: &Arc<StreamObj>, pos: &mut u64, ok: &mut bool) -> bool {
        let dict = s.dict.clone();
        macro_rules! fail {
            () => {{
                *ok = false;
                return false;
            }};
        }
        let Object::Int(new_size) = dict.get_nf(b"Size") else {
            fail!();
        };
        if new_size < 0 {
            fail!();
        }
        self.grow_to(new_size as usize);
        let Object::Array(wa) = dict.get_nf(b"W") else {
            fail!();
        };
        if wa.len() < 3 {
            fail!();
        }
        let mut w = [0i32; 3];
        for i in 0..3 {
            let Object::Int(v) = wa[i] else {
                fail!();
            };
            w[i] = v;
        }
        if w.iter().any(|&v| !(0..=8).contains(&v)) {
            fail!();
        }
        let decoded = self.decode_stream(s, 0);
        let mut cursor = 0usize;
        match dict.get_nf(b"Index") {
            Object::Array(idx) => {
                let mut i = 0;
                while i + 1 < idx.len() {
                    let (Object::Int(first), Object::Int(n)) = (&idx[i], &idx[i + 1]) else {
                        fail!();
                    };
                    let (first, n) = (*first, *n);
                    if first < 0 || n < 0 || !self.read_xref_stream_section(&decoded.data, &mut cursor, &w, first, n) {
                        fail!();
                    }
                    i += 2;
                }
            }
            _ => {
                if !self.read_xref_stream_section(&decoded.data, &mut cursor, &w, 0, new_size) {
                    fail!();
                }
            }
        }
        let more = match dict.get_nf(b"Prev") {
            Object::Int(n) => {
                *pos = n as u32 as u64;
                true
            }
            _ => false,
        };
        if matches!(self.trailer, Object::None) {
            self.trailer = Object::Dict(dict);
        }
        more
    }

    /// `XRef::readXRefStreamSection`.
    fn read_xref_stream_section(&mut self, d: &[u8], cur: &mut usize, w: &[i32; 3], first: i32, n: i32) -> bool {
        if first.checked_add(n).is_none() {
            return false;
        }
        let top = first + n;
        if top > self.entries.len() as i32 {
            let mut new_size: i64 = if self.entries.is_empty() { 1024 } else { 2 * self.entries.len() as i64 };
            while (top as i64) > new_size {
                new_size <<= 1;
            }
            if new_size > i32::MAX as i64 {
                return false;
            }
            self.grow_to(new_size as usize);
        }
        let read = |cur: &mut usize, k: i32| -> Option<i64> {
            let mut v: i64 = 0;
            for _ in 0..k {
                let c = *d.get(*cur)?;
                *cur += 1;
                v = (v << 8).wrapping_add(c as i64);
            }
            Some(v)
        };
        for i in first..top {
            let ty = if w[0] == 0 {
                1
            } else {
                match read(cur, w[0]) {
                    Some(v) => v,
                    None => return false,
                }
            };
            let Some(offset) = read(cur, w[1]) else {
                return false;
            };
            if offset < 0 {
                return false;
            }
            let Some(gno) = read(cur, w[2]) else {
                return false;
            };
            if (gno < 0 || gno > i32::MAX as i64) && ty != 0 {
                return false;
            }
            if self.entries[i as usize].offset == -1 {
                let t = match ty {
                    0 => EntryType::Free,
                    1 => EntryType::Uncompressed,
                    2 => EntryType::Compressed,
                    _ => return false,
                };
                self.entries[i as usize] = XRefEntry { offset, gno: gno as i32, ty: t };
                if i > self.last {
                    self.last = i;
                }
            }
        }
        true
    }

    /// `XRef::constructXRef`: scan a damaged file for objects.
    fn construct_xref(&mut self) -> bool {
        let mut stream_obj_nums: Vec<i32> = Vec::new();
        let mut last_obj_num = -1;
        self.root = (-1, 0);
        self.stream_ends.clear();
        let data = self.data.clone();
        let d: &[u8] = &data;
        let at = |p: usize| -> u8 { d.get(p).copied().unwrap_or(0) };
        let starts = |p: usize, s: &[u8]| -> bool { d.len() >= p + s.len() && &d[p..p + s.len()] == s };
        let mut p = self.start as usize;
        let mut start_of_line = true;
        let mut space = true;
        while p < d.len() {
            if start_of_line && starts(p, b"trailer") {
                self.construct_trailer_dict((p + 7) as u64);
                p += 7;
                start_of_line = false;
                space = false;
            } else if start_of_line && starts(p, b"endstream") {
                self.stream_ends.push(p as u64);
                p += 9;
                start_of_line = false;
                space = false;
            } else if space && at(p).is_ascii_digit() {
                p = self.construct_object_entry(d, p, &mut last_obj_num);
                start_of_line = false;
                space = false;
            } else if at(p) == b'>' && at(p + 1) == b'>' {
                p += 2;
                start_of_line = false;
                space = false;
                // skip any PDF whitespace except for '\0'
                while matches!(at(p), b'\t' | b'\n' | 0x0c | b'\r' | b' ') && p < d.len() {
                    if matches!(at(p), b'\n' | b'\r') {
                        start_of_line = true;
                    }
                    space = true;
                    p += 1;
                }
                if starts(p, b"stream") {
                    if last_obj_num >= 0 {
                        stream_obj_nums.push(last_obj_num);
                    }
                    p += 6;
                    start_of_line = false;
                    space = false;
                }
            } else {
                let c = at(p);
                if c == b'\n' || c == b'\r' {
                    start_of_line = true;
                    space = true;
                } else if filters::lexer_is_space(c as i32) {
                    space = true;
                } else {
                    start_of_line = false;
                    space = false;
                }
                p += 1;
            }
        }
        for &num in &stream_obj_nums {
            let gno = self.entries[num as usize].gno;
            let Object::Stream(s) = self.fetch(num, gno, 0) else {
                continue;
            };
            let ty = self.lookup(&s.dict, b"Type");
            if ty.is_name(b"XRef") {
                self.save_trailer_dict(&s.dict, true);
            } else if ty.is_name(b"ObjStm") {
                self.construct_object_stream_entries(&s, num);
            }
        }
        self.root.0 >= 0
    }

    /// `XRef::constructTrailerDict`.
    fn construct_trailer_dict(&mut self, pos: u64) {
        let mut parser = Parser::new(self.file_lexer(pos), false);
        if let Object::Dict(d) = parser.get_obj(None, false, 0) {
            self.save_trailer_dict(&d, false);
        }
    }

    /// `XRef::saveTrailerDict`.
    fn save_trailer_dict(&mut self, dict: &Arc<Dict>, is_xref_stream: bool) {
        if let Object::Ref(num, gno) = dict.get_nf(b"Root") {
            if !is_xref_stream || num <= self.last {
                self.root = (num, gno);
                self.trailer = Object::Dict(dict.clone());
            }
        }
    }

    /// `XRef::constructObjectEntry`.
    fn construct_object_entry(&mut self, d: &[u8], start: usize, obj_num: &mut i32) -> usize {
        let at = |p: usize| -> u8 { d.get(p).copied().unwrap_or(0) };
        let mut p = start;
        let mut num: i32 = 0;
        loop {
            num = num * 10 + (at(p) - b'0') as i32;
            p += 1;
            if !(at(p).is_ascii_digit() && num < 100000000) {
                break;
            }
        }
        let blank = |c: u8| c == b'\t' || c == 0x0c || c == b' ';
        if !blank(at(p)) {
            return p;
        }
        loop {
            p += 1;
            if !blank(at(p)) {
                break;
            }
        }
        if !at(p).is_ascii_digit() {
            return p;
        }
        let mut gno: i32 = 0;
        loop {
            gno = gno * 10 + (at(p) - b'0') as i32;
            p += 1;
            if !(at(p).is_ascii_digit() && gno < 100000000) {
                break;
            }
        }
        if !blank(at(p)) {
            return p;
        }
        loop {
            p += 1;
            if !blank(at(p)) {
                break;
            }
        }
        if d.len() < p + 3 || &d[p..p + 3] != b"obj" {
            return p;
        }
        if self.construct_xref_entry(num, gno, start as i64 - self.start as i64, EntryType::Uncompressed) {
            *obj_num = num;
        }
        p
    }

    /// `XRef::constructObjectStreamEntries`.
    fn construct_object_stream_entries(&mut self, s: &Arc<StreamObj>, obj_str_num: i32) {
        let Object::Int(n) = self.lookup(&s.dict, b"N") else {
            return;
        };
        if n <= 0 || n > 1000000 {
            return;
        }
        let decoded = self.decode_stream(s, 0);
        let bytes: Arc<[u8]> = decoded.data.into();
        let len = bytes.len() as u64;
        let mut parser = Parser::new(Lexer::new(bytes, 0, len), false);
        for i in 0..n {
            let o1 = parser.get_obj(None, true, 0);
            let o2 = parser.get_obj(None, true, 0);
            if let (Object::Int(num), Object::Int(_)) = (o1, o2) {
                if (0..1000000).contains(&num) {
                    self.construct_xref_entry(num, i, obj_str_num as i64, EntryType::Compressed);
                }
            }
        }
    }

    /// `XRef::constructXRefEntry`.
    fn construct_xref_entry(&mut self, num: i32, gno: i32, pos: i64, ty: EntryType) -> bool {
        if num as usize >= self.entries.len() {
            self.grow_to(((num as usize) + 1 + 255) & !255);
        }
        let e = &mut self.entries[num as usize];
        if e.ty == EntryType::Free || gno >= e.gno {
            *e = XRefEntry { offset: pos, gno, ty };
            if num > self.last {
                self.last = num;
            }
        }
        true
    }

    /// `XRef::getStreamEnd`.
    fn get_stream_end(&self, stream_start: u64) -> Option<u64> {
        if self.stream_ends.is_empty() || stream_start > *self.stream_ends.last().unwrap() {
            return None;
        }
        let i = self.stream_ends.partition_point(|&e| e < stream_start);
        Some(self.stream_ends[i])
    }

    fn num_objects(&self) -> i32 {
        self.last + 1
    }

    /// `XRef::fetch`.
    pub fn fetch(&mut self, num: i32, gno: i32, recursion: i32) -> Object {
        if num < 0 || num as usize >= self.entries.len() {
            return Object::Null;
        }
        let e = self.entries[num as usize];
        match e.ty {
            EntryType::Uncompressed => {
                if e.gno != gno {
                    return Object::Null;
                }
                let at = (self.start as i64).wrapping_add(e.offset);
                if at < 0 {
                    return Object::Null;
                }
                let mut parser = Parser::new(self.file_lexer(at as u64), true);
                let o1 = parser.get_obj(Some(self), true, 0);
                let o2 = parser.get_obj(Some(self), true, 0);
                let o3 = parser.get_obj(Some(self), true, 0);
                if !matches!(o1, Object::Int(n) if n == num)
                    || !matches!(o2, Object::Int(g) if g == gno)
                    || !o3.is_cmd(b"obj")
                {
                    return Object::Null;
                }
                parser.get_obj(Some(self), false, recursion)
            }
            EntryType::Compressed => {
                if e.offset >= self.entries.len() as i64
                    || e.offset < 0
                    || self.entries[e.offset as usize].ty != EntryType::Uncompressed
                {
                    return Object::Null;
                }
                self.get_object_stream_object(e.offset as i32, e.gno, num, recursion)
            }
            EntryType::Free => Object::Null,
        }
    }

    /// `XRef::getObjectStreamObject`, with the `ObjectStream` constructor.
    fn get_object_stream_object(&mut self, obj_str_num: i32, obj_idx: i32, obj_num: i32, recursion: i32) -> Object {
        if recursion >= OBJECT_RECURSION_LIMIT {
            return Object::Null;
        }
        let os = match self.objstrs.get(&obj_str_num) {
            Some(o) => o.clone(),
            None => match self.load_object_stream(obj_str_num, recursion + 1) {
                Some(o) => {
                    let a = Arc::new(o);
                    self.objstrs.insert(obj_str_num, a.clone());
                    a
                }
                None => return Object::Null,
            },
        };
        if obj_idx < 0 || obj_idx as usize >= os.nums.len() || obj_num != os.nums[obj_idx as usize] {
            return Object::Null;
        }
        os.objs[obj_idx as usize].clone()
    }

    fn load_object_stream(&mut self, obj_str_num: i32, recursion: i32) -> Option<ObjectStream> {
        let Object::Stream(s) = self.fetch(obj_str_num, 0, recursion) else {
            return None;
        };
        let Object::Int(n) = self.lookup_r(&s.dict, b"N", recursion) else {
            return None;
        };
        if n <= 0 {
            return None;
        }
        let Object::Int(first) = self.lookup_r(&s.dict, b"First", recursion) else {
            return None;
        };
        if first < 0 || n > 1000000 {
            return None;
        }
        let decoded = self.decode_stream(&s, recursion);
        let bytes: Arc<[u8]> = decoded.data.into();
        let len = bytes.len() as u64;
        let first = first as u64;
        let mut nums = Vec::with_capacity(n as usize);
        let mut offsets: Vec<i32> = Vec::with_capacity(n as usize);
        {
            let mut parser = Parser::new(Lexer::new(bytes.clone(), 0, first.min(len)), false);
            for i in 0..n as usize {
                let o1 = parser.get_obj(Some(self), true, 0);
                let o2 = parser.get_obj(Some(self), true, 0);
                let (Object::Int(a), Object::Int(b)) = (o1, o2) else {
                    return None;
                };
                if a < 0 || b < 0 || (i > 0 && b < offsets[i - 1]) {
                    return None;
                }
                nums.push(a);
                offsets.push(b);
            }
        }
        let mut objs = Vec::with_capacity(n as usize);
        for i in 0..n as usize {
            let from = first + offsets[i] as u64;
            let to = if i + 1 < n as usize { first + offsets[i + 1] as u64 } else { len };
            let mut parser = Parser::new(Lexer::new(bytes.clone(), from.min(len), to.min(len)), false);
            objs.push(parser.get_obj(Some(self), false, 0));
        }
        Some(ObjectStream { nums, objs })
    }

    /// `Object::fetch`.
    pub fn resolve(&mut self, o: &Object, recursion: i32) -> Object {
        match o {
            Object::Ref(n, g) => self.fetch(*n, *g, recursion),
            _ => o.clone(),
        }
    }

    /// `Dict::lookup` with a recursion count.
    fn lookup_r(&mut self, d: &Dict, key: &[u8], recursion: i32) -> Object {
        let v = d.get_nf(key);
        self.resolve(&v, recursion)
    }

    /// `Dict::lookup`.
    pub fn lookup(&mut self, d: &Dict, key: &[u8]) -> Object {
        self.lookup_r(d, key, 0)
    }

    /// `Array::get`.
    pub fn array_get(&mut self, a: &[Object], i: usize) -> Object {
        match a.get(i) {
            Some(o) => self.resolve(o, 0),
            None => Object::Null,
        }
    }

    fn trailer_lookup(&mut self, key: &[u8]) -> Object {
        let t = self.trailer.clone();
        match t {
            Object::Dict(d) => self.lookup(&d, key),
            _ => Object::Null,
        }
    }

    /// `getDocInfoNF`.
    pub fn doc_info_nf(&self) -> Object {
        match &self.trailer {
            Object::Dict(d) => d.get_nf(b"Info"),
            _ => Object::Null,
        }
    }

    /// The raw bytes of a stream: start and length within the file, stopping at its end.
    pub fn raw_range(&self, s: &StreamObj) -> (usize, usize) {
        let len = self.data.len() as u64;
        let start = (s.start as u64).min(len);
        let end = (s.start as u64).saturating_add(s.length).min(len);
        (start as usize, (end - start) as usize)
    }

    /// `Stream::addFilters` with `makeFilter`, and reading the result to the end.
    pub fn decode_stream(&mut self, s: &StreamObj, recursion: i32) -> Decoded {
        let (start, n) = self.raw_range(s);
        let data = self.data.clone();
        let raw = &data[start..start + n];
        let dict = s.dict.clone();
        let get = |doc: &mut Doc, key: &[u8]| -> Object {
            if s.has_xref { doc.lookup_r(&dict, key, recursion) } else { dict.get_nf(key) }
        };
        let mut obj = get(self, b"Filter");
        if obj.is_null() {
            obj = get(self, b"F");
        }
        let mut params = get(self, b"DecodeParms");
        if params.is_null() {
            params = get(self, b"DP");
        }
        let mut cur: Option<Vec<u8>> = None;
        let mut out = Decoded { data: Vec::new(), unsupported: None };
        match &obj {
            Object::Name(name) => {
                let input = cur.as_deref().unwrap_or(raw);
                match self.make_filter(name, input, &params, s.has_xref, recursion) {
                    FilterResult::Data(v) => cur = Some(v),
                    FilterResult::Same => {}
                    FilterResult::Eof => cur = Some(Vec::new()),
                    FilterResult::Unsupported => {
                        out.unsupported = Some(name.as_ref().clone());
                        cur = Some(Vec::new());
                    }
                }
            }
            Object::Array(a) => {
                for i in 0..a.len() {
                    let f = if s.has_xref { self.resolve(&a[i], recursion) } else { a[i].clone() };
                    let p = match &params {
                        Object::Array(pa) if i < pa.len() => {
                            if s.has_xref { self.resolve(&pa[i], recursion) } else { pa[i].clone() }
                        }
                        _ => Object::Null,
                    };
                    let Object::Name(name) = &f else {
                        cur = Some(Vec::new());
                        break;
                    };
                    let input = cur.as_deref().unwrap_or(raw);
                    match self.make_filter(name, input, &p, s.has_xref, recursion) {
                        FilterResult::Data(v) => cur = Some(v),
                        FilterResult::Same => {}
                        FilterResult::Eof => {
                            cur = Some(Vec::new());
                            break;
                        }
                        FilterResult::Unsupported => {
                            out.unsupported = Some(name.as_ref().clone());
                            cur = Some(Vec::new());
                            break;
                        }
                    }
                }
            }
            _ => {}
        }
        out.data = cur.unwrap_or_else(|| raw.to_vec());
        out
    }

    /// `Stream::makeFilter`.
    fn make_filter(&mut self, name: &[u8], input: &[u8], params: &Object, xref: bool, recursion: i32) -> FilterResult {
        let int_param = |doc: &mut Doc, key: &[u8], default: i32| -> i32 {
            if let Object::Dict(d) = params {
                let v = if xref { doc.lookup_r(d, key, recursion) } else { d.get_nf(key) };
                if let Object::Int(i) = v {
                    return i;
                }
            }
            default
        };
        match name {
            b"ASCIIHexDecode" | b"AHx" => FilterResult::Data(filters::ascii_hex(input)),
            b"ASCII85Decode" | b"A85" => FilterResult::Data(filters::ascii85(input)),
            b"LZWDecode" | b"LZW" => {
                let pred = int_param(self, b"Predictor", 1);
                let columns = int_param(self, b"Columns", 1);
                let colors = int_param(self, b"Colors", 1);
                let bits = int_param(self, b"BitsPerComponent", 8);
                let early = int_param(self, b"EarlyChange", 1);
                let v = filters::lzw(input, early);
                FilterResult::Data(apply_predictor(v, pred, columns, colors, bits))
            }
            b"RunLengthDecode" | b"RL" => FilterResult::Data(filters::run_length(input)),
            b"FlateDecode" | b"Fl" => {
                let pred = int_param(self, b"Predictor", 1);
                let columns = int_param(self, b"Columns", 1);
                let colors = int_param(self, b"Colors", 1);
                let bits = int_param(self, b"BitsPerComponent", 8);
                let v = filters::flate(input);
                FilterResult::Data(apply_predictor(v, pred, columns, colors, bits))
            }
            b"CCITTFaxDecode" | b"CCF" | b"DCTDecode" | b"DCT" | b"JBIG2Decode" | b"JPXDecode" => FilterResult::Unsupported,
            b"Crypt" => FilterResult::Same,
            _ => FilterResult::Eof,
        }
    }

    //------------------------------------------------------------------------
    // Catalog
    //------------------------------------------------------------------------

    /// `Catalog::readPageTree`.
    fn read_page_tree(&mut self, cat: &Dict) -> bool {
        let top_ref = cat.get_nf(b"Pages");
        let Object::Ref(tn, tg) = top_ref else {
            return false;
        };
        let top = self.fetch(tn, tg, 0);
        let Object::Dict(top) = top else {
            return false;
        };
        let mut num_pages = match self.lookup(&top, b"Count") {
            Object::Int(n) => {
                if n == 0 || n > 50000 {
                    let mut touched = vec![false; self.num_objects().max(0) as usize];
                    self.count_page_tree(&top_ref, &mut touched)
                } else {
                    n
                }
            }
            // assume we got a Page node instead of a Pages node
            _ => 1,
        };
        if num_pages < 0 {
            num_pages = 0;
            self.num_pages = num_pages;
            return false;
        }
        self.num_pages = num_pages;
        self.nodes.clear();
        self.nodes.push(PageTreeNode { r: (tn, tg), count: num_pages, parent: None, kids: None, attrs: None });
        self.pages = vec![None; num_pages as usize];
        self.page_refs = vec![(-1, -1); num_pages as usize];
        true
    }

    /// `Catalog::countPageTree`.
    fn count_page_tree(&mut self, node_ref: &Object, touched: &mut [bool]) -> i32 {
        let node = if let Object::Ref(n, g) = node_ref {
            if *n < 0 || *n >= self.num_objects() {
                return 0;
            }
            if touched[*n as usize] {
                return 0;
            }
            touched[*n as usize] = true;
            self.fetch(*n, *g, 0)
        } else {
            node_ref.clone()
        };
        let mut n = 0i32;
        if let Object::Dict(d) = node {
            let kids_ref = d.get_nf(b"Kids");
            let kids = if let Object::Ref(kn, kg) = kids_ref {
                if kn >= 0 && kn < self.num_objects() {
                    if touched[kn as usize] {
                        return 0;
                    }
                    touched[kn as usize] = true;
                    self.fetch(kn, kg, 0)
                } else {
                    kids_ref.clone()
                }
            } else {
                kids_ref.clone()
            };
            if let Object::Array(a) = kids {
                for k in a.iter() {
                    let n2 = self.count_page_tree(k, touched);
                    n = if n2 < i32::MAX - n { n + n2 } else { i32::MAX };
                }
            } else {
                n = 1;
            }
        }
        n
    }

    /// `Catalog::loadPage` and `loadPage2`.
    fn load_page(&mut self, pg: i32) {
        self.load_page2(pg, pg - 1, 0);
    }

    fn load_page2(&mut self, pg: i32, rel_pg: i32, node: usize) {
        let idx = (pg - 1) as usize;
        if rel_pg >= self.nodes[node].count {
            self.pages[idx] = Some(Page { attrs: PageAttrs::default(), contents: Object::Null });
            return;
        }
        if self.nodes[node].kids.is_none() {
            let r = self.nodes[node].r;
            let mut p = self.nodes[node].parent;
            while let Some(i) = p {
                if self.nodes[i].r == r {
                    self.pages[idx] = Some(Page { attrs: PageAttrs::default(), contents: Object::Null });
                    return;
                }
                p = self.nodes[i].parent;
            }
            let Object::Dict(page_obj) = self.fetch(r.0, r.1, 0) else {
                self.pages[idx] = Some(Page { attrs: PageAttrs::default(), contents: Object::Null });
                return;
            };
            let parent_attrs = self.nodes[node].parent.and_then(|i| self.nodes[i].attrs.clone());
            let attrs = self.page_attrs(parent_attrs.as_ref(), &page_obj);
            if let Object::Array(kids) = self.lookup(&page_obj, b"Kids") {
                self.nodes[node].attrs = Some(attrs);
                let mut list = Vec::new();
                for k in kids.iter() {
                    if let Object::Ref(kn, kg) = k {
                        if let Object::Dict(kd) = self.fetch(*kn, *kg, 0) {
                            let count = match self.lookup(&kd, b"Count") {
                                Object::Int(c) => c,
                                _ => 1,
                            };
                            self.nodes.push(PageTreeNode {
                                r: (*kn, *kg),
                                count,
                                parent: Some(node),
                                kids: None,
                                attrs: None,
                            });
                            list.push(self.nodes.len() - 1);
                        }
                    }
                }
                self.nodes[node].kids = Some(list);
            } else {
                self.page_refs[idx] = r;
                self.pages[idx] = Some(make_page(&page_obj, attrs));
            }
        }
        if let Some(kids) = self.nodes[node].kids.clone() {
            let mut rel = rel_pg;
            for k in kids {
                if rel < self.nodes[k].count {
                    self.load_page2(pg, rel, k);
                    return;
                }
                rel -= self.nodes[k].count;
            }
            // this will only happen if the page tree is invalid
            self.pages[idx] = Some(Page { attrs: PageAttrs::default(), contents: Object::Null });
        }
    }

    /// `PageAttrs::PageAttrs`.
    fn page_attrs(&mut self, attrs: Option<&PageAttrs>, dict: &Dict) -> PageAttrs {
        let mut media_box = [0.0, 0.0, 612.0, 792.0];
        let mut crop_box = [0.0; 4];
        let mut have_crop_box = false;
        let mut rotate = 0;
        if let Some(a) = attrs {
            media_box = a.media_box;
            crop_box = a.crop_box;
            have_crop_box = a.have_crop_box;
            rotate = a.rotate;
        }
        if let Some(b) = self.read_box(dict, b"MediaBox") {
            media_box = b;
        }
        if let Some(b) = self.read_box(dict, b"CropBox") {
            crop_box = b;
            have_crop_box = true;
        }
        if !have_crop_box {
            crop_box = media_box;
        }
        let bleed_box = self.read_box(dict, b"BleedBox").unwrap_or(crop_box);
        let trim_box = self.read_box(dict, b"TrimBox").unwrap_or(crop_box);
        let art_box = self.read_box(dict, b"ArtBox").unwrap_or(crop_box);
        if let Object::Int(r) = self.lookup(dict, b"Rotate") {
            rotate = r;
        }
        rotate = rotate.rem_euclid(360);
        let group = self.lookup(dict, b"Group");
        let child = self.lookup(dict, b"Resources");
        let parent_res = attrs.map(|a| a.resources.clone());
        let resources = match (&parent_res, &child) {
            (Some(Object::Dict(parent)), Object::Dict(child)) => {
                // merge this node's resources into the parent's resources
                let mut res = Dict::default();
                for i in 0..parent.len() {
                    let v = parent.val_nf(i);
                    if let Object::Dict(sub1) = self.resolve(&v, 0) {
                        let mut sub2 = Dict::default();
                        for j in 0..sub1.len() {
                            sub2.add(sub1.key(j).to_vec(), sub1.val_nf(j));
                        }
                        res.add(parent.key(i).to_vec(), Object::Dict(Arc::new(sub2)));
                    }
                }
                for i in 0..child.len() {
                    let v = child.val_nf(i);
                    if let Object::Dict(sub1) = self.resolve(&v, 0) {
                        let key = child.key(i);
                        if let Object::Dict(mut sub2) = res.get_nf(key) {
                            let m = Arc::make_mut(&mut sub2);
                            for j in 0..sub1.len() {
                                m.add(sub1.key(j).to_vec(), sub1.val_nf(j));
                            }
                            res.add(key.to_vec(), Object::Dict(sub2));
                        } else {
                            res.add(key.to_vec(), Object::Dict(sub1));
                        }
                    }
                }
                Object::Dict(Arc::new(res))
            }
            (Some(Object::Dict(p)), _) => Object::Dict(p.clone()),
            (_, Object::Dict(c)) => Object::Dict(c.clone()),
            _ => Object::Null,
        };
        PageAttrs { media_box, crop_box, have_crop_box, bleed_box, trim_box, art_box, rotate, group, resources }
    }

    /// `PageAttrs::readBox`.
    fn read_box(&mut self, dict: &Dict, key: &[u8]) -> Option<[f64; 4]> {
        let Object::Array(a) = self.lookup(dict, key) else {
            return None;
        };
        if a.len() != 4 {
            return None;
        }
        let mut t = [0.0; 4];
        let mut ok = true;
        for (i, slot) in t.iter_mut().enumerate() {
            match self.array_get(&a, i).as_num() {
                Some(v) => *slot = v,
                None => ok = false,
            }
        }
        if !ok {
            return None;
        }
        for v in t.iter_mut() {
            if *v < -1e9 {
                *v = -1e9;
            } else if *v > 1e9 {
                *v = 1e9;
            }
        }
        if t[0] > t[2] {
            t.swap(0, 2);
        }
        if t[1] > t[3] {
            t.swap(1, 3);
        }
        Some(t)
    }

    /// `Catalog::getPage`.
    pub fn get_page(&mut self, i: i32) -> Page {
        let idx = (i - 1) as usize;
        if self.pages[idx].is_none() {
            self.load_page(i);
        }
        match &self.pages[idx] {
            Some(p) => p.clone(),
            // A tree that gives no page: xpdf follows a null pointer.
            None => Page { attrs: PageAttrs::default(), contents: Object::Null },
        }
    }

    /// `Catalog::getPageRef`.
    pub fn get_page_ref(&mut self, i: i32) -> (i32, i32) {
        let idx = (i - 1) as usize;
        if self.pages[idx].is_none() {
            self.load_page(i);
        }
        self.page_refs[idx]
    }

    /// `Catalog::findPage`.
    pub fn find_page(&mut self, num: i32, gno: i32) -> i32 {
        for i in 0..self.num_pages {
            if self.pages[i as usize].is_none() {
                self.load_page(i + 1);
            }
            if self.page_refs[i as usize] == (num, gno) {
                return i + 1;
            }
        }
        0
    }

    /// `Catalog::findDest`: the destination's page reference, or `None` for no valid destination.  The inner `None` is
    /// a destination that names its page by number: pdfTeX then reads an unset `Ref`, which finds no page.
    pub fn find_dest(&mut self, name: &[u8]) -> Option<Option<(i32, i32)>> {
        let mut found: Option<Object> = None;
        let dests = self.dests.clone();
        if let Object::Dict(d) = &dests {
            let o = self.lookup(d, name);
            if !o.is_null() {
                found = Some(o);
            }
        }
        if found.is_none() {
            let tree = self.name_tree.clone();
            if matches!(tree, Object::Dict(_)) {
                let mut touched = vec![false; self.num_objects().max(0) as usize];
                let o = self.find_dest_in_tree(&tree, &tree, name, &mut touched);
                if !o.is_null() {
                    found = Some(o);
                }
            }
        }
        let obj = found?;
        let arr = match obj {
            Object::Array(a) => a,
            Object::Dict(d) => match self.lookup(&d, b"D") {
                Object::Array(a) => a,
                _ => return None,
            },
            _ => return None,
        };
        self.link_dest(&arr)
    }

    /// `LinkDest::LinkDest`: only whether it is ok, and the page.
    fn link_dest(&mut self, a: &[Object]) -> Option<Option<(i32, i32)>> {
        if a.len() < 2 {
            return None;
        }
        let page = match &a[0] {
            Object::Int(_) => None,
            Object::Ref(n, g) => Some((*n, *g)),
            _ => return None,
        };
        let kind = self.array_get(a, 1);
        let Object::Name(kind) = kind else {
            return None;
        };
        let num_or_null = |o: &Object| o.is_null() || o.as_num().is_some();
        match kind.as_slice() {
            b"XYZ" => {
                for i in 2..5 {
                    if a.len() > i {
                        let o = self.array_get(a, i);
                        if !num_or_null(&o) {
                            return None;
                        }
                    }
                }
            }
            b"Fit" | b"FitB" => {}
            b"FitH" | b"FitV" | b"FitBH" | b"FitBV" => {
                if a.len() < 3 {
                    return None;
                }
            }
            b"FitR" => {
                if a.len() < 6 {
                    return None;
                }
            }
            _ => return None,
        }
        Some(page)
    }

    /// `Catalog::findDestInTree`.
    fn find_dest_in_tree(&mut self, tree_ref: &Object, tree: &Object, name: &[u8], touched: &mut [bool]) -> Object {
        if let Object::Ref(n, _) = tree_ref {
            if *n < 0 || *n >= self.num_objects() {
                return Object::Null;
            }
            if touched[*n as usize] {
                return Object::Null;
            }
            touched[*n as usize] = true;
        }
        let Object::Dict(d) = tree else {
            return Object::Null;
        };
        if let Object::Array(names) = self.lookup(d, b"Names") {
            let mut i = 0;
            while i < names.len() {
                if let Object::Str(n1) = self.array_get(&names, i) {
                    match gstring_cmp(name, &n1) {
                        std::cmp::Ordering::Equal => return self.array_get(&names, i + 1),
                        std::cmp::Ordering::Less => return Object::Null,
                        std::cmp::Ordering::Greater => {}
                    }
                }
                i += 2;
            }
            return Object::Null;
        }
        if let Object::Array(kids) = self.lookup(d, b"Kids") {
            for k in kids.iter() {
                let kid = self.resolve(k, 0);
                if let Object::Dict(kd) = &kid {
                    if let Object::Array(limits) = self.lookup(kd, b"Limits") {
                        if let Object::Str(low) = self.array_get(&limits, 0) {
                            if gstring_cmp(name, &low) != std::cmp::Ordering::Less {
                                if let Object::Str(high) = self.array_get(&limits, 1) {
                                    if gstring_cmp(name, &high) != std::cmp::Ordering::Greater {
                                        return self.find_dest_in_tree(k, &kid, name, touched);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Object::Null
    }
}

enum FilterResult {
    Data(Vec<u8>),
    Same,
    Eof,
    Unsupported,
}

fn apply_predictor(v: Vec<u8>, pred: i32, columns: i32, colors: i32, bits: i32) -> Vec<u8> {
    if pred != 1 {
        if let Some(p) = filters::predict(&v, pred, columns, colors, bits) {
            return p;
        }
    }
    v
}

/// `Page::Page`.
fn make_page(page_dict: &Dict, mut attrs: PageAttrs) -> Page {
    let media = attrs.media_box;
    clip_to(&mut attrs.crop_box, &media);
    clip_to(&mut attrs.bleed_box, &media);
    clip_to(&mut attrs.trim_box, &media);
    clip_to(&mut attrs.art_box, &media);
    let annots = page_dict.get_nf(b"Annots");
    let contents = page_dict.get_nf(b"Contents");
    if !matches!(annots, Object::Ref(..) | Object::Array(_) | Object::Null)
        || !matches!(contents, Object::Ref(..) | Object::Array(_) | Object::Null)
    {
        return Page { attrs: PageAttrs::default(), contents: Object::Null };
    }
    Page { attrs, contents }
}

/// `GString::cmp`: unsigned bytes, then the shorter is less.
fn gstring_cmp(a: &[u8], b: &[u8]) -> std::cmp::Ordering {
    a.cmp(b)
}

/// C's `atof` on a token: optional sign, digits, fraction and exponent.  Anything else ends the number.
pub(crate) fn atof(s: &[u8]) -> f64 {
    let mut i = 0;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        i += 1;
    }
    let digits_start = i;
    while i < s.len() && s[i].is_ascii_digit() {
        i += 1;
    }
    let mut seen_digit = i > digits_start;
    if i < s.len() && s[i] == b'.' {
        i += 1;
        let frac_start = i;
        while i < s.len() && s[i].is_ascii_digit() {
            i += 1;
        }
        seen_digit |= i > frac_start;
    }
    if !seen_digit {
        return 0.0;
    }
    if i < s.len() && (s[i] == b'e' || s[i] == b'E') {
        let mut j = i + 1;
        if j < s.len() && (s[j] == b'+' || s[j] == b'-') {
            j += 1;
        }
        let exp_start = j;
        while j < s.len() && s[j].is_ascii_digit() {
            j += 1;
        }
        if j > exp_start {
            i = j;
        }
    }
    std::str::from_utf8(&s[..i]).ok().and_then(|t| t.parse::<f64>().ok()).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(s: &[u8]) -> Object {
        let d: Arc<[u8]> = s.to_vec().into();
        let n = d.len() as u64;
        Lexer::new(d, 0, n).get_obj()
    }

    #[test]
    fn numbers_as_xpdf_reads_them() {
        assert!(matches!(lex(b"123"), Object::Int(123)));
        assert!(matches!(lex(b"--123"), Object::Int(0)));
        assert!(matches!(lex(b"50-100"), Object::Int(50)));
        assert!(matches!(lex(b"-50-100"), Object::Int(-50)));
        assert!(matches!(lex(b"-"), Object::Int(0)));
        assert!(matches!(lex(b"+7"), Object::Int(7)));
        assert!(matches!(lex(b"2147483648"), Object::Int(i32::MIN)));
        match lex(b"--123.4") {
            Object::Real(r) => assert!((r + 123.4).abs() < 1e-9),
            o => panic!("{o:?}"),
        }
        match lex(b"-.5") {
            Object::Real(r) => assert!((r + 0.5).abs() < 1e-12),
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn strings_and_names() {
        match lex(b"(a\\(b\\)\\n\\101\\7(c)\\\r\nd\re)") {
            Object::Str(s) => assert_eq!(s.as_slice(), b"a(b)\nA\x07(c)d\ne"),
            o => panic!("{o:?}"),
        }
        match lex(b"<48 65 6c6C 6f7>") {
            Object::Str(s) => assert_eq!(s.as_slice(), b"Hellop"),
            o => panic!("{o:?}"),
        }
        match lex(b"/A#42C#zz") {
            Object::Name(s) => assert_eq!(s.as_slice(), b"ABC#zz"),
            o => panic!("{o:?}"),
        }
        assert!(matches!(lex(b"/A#00B"), Object::Error));
        assert!(matches!(lex(b"true"), Object::Bool(true)));
        assert!(matches!(lex(b"null"), Object::Null));
        assert!(lex(b"obj").is_cmd(b"obj"));
    }

    #[test]
    fn dict_keeps_first_place_and_last_value() {
        let mut d = Dict::default();
        d.add(b"A".to_vec(), Object::Int(1));
        d.add(b"B".to_vec(), Object::Int(2));
        d.add(b"A".to_vec(), Object::Int(3));
        assert_eq!(d.len(), 2);
        assert_eq!(d.key(0), b"A");
        assert!(matches!(d.get_nf(b"A"), Object::Int(3)));
        for i in 0..100 {
            d.add(format!("K{i}").into_bytes(), Object::Int(i));
        }
        assert!(matches!(d.get_nf(b"K99"), Object::Int(99)));
        assert!(matches!(d.get_nf(b"B"), Object::Int(2)));
        d.add(b"K5".to_vec(), Object::Int(-5));
        assert!(matches!(d.get_nf(b"K5"), Object::Int(-5)));
        assert_eq!(d.len(), 102);
    }

    #[test]
    fn atof_stops_at_junk() {
        assert_eq!(atof(b"1.5"), 1.5);
        assert_eq!(atof(b"1.7%xx"), 1.7);
        assert_eq!(atof(b"abc"), 0.0);
        assert_eq!(atof(b"2e3x"), 2000.0);
        assert_eq!(atof(b"2e"), 2.0);
    }
}
