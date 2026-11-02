//! The output of every writer in this crate: a flat list of operations the engine replays.
//!
//! Each `Op` is one call the C wrote against the engine's PDF output: `pdf_puts`, `pdfout`, `pdfbeginstream`,
//! `pdfcreateobj(0,0)`, ...  The writers make no call themselves, so they stay free of engine state, and the list is
//! plain data.  See the crate documentation for the replay loop.

use std::fmt::Write as _;

/// An object number that does not exist yet.  `Op::NewObj(slot)` allocates it when the list is replayed and every later
/// use of the slot means that number.  A slot lives as long as the engine keeps its table, across calls: the PDF
/// inclusion's copied objects and a JBIG2 file's globals object are referred to by later images.
pub type Slot = u32;

/// The source of slots.  One per run, kept with the engine's image table.
#[derive(Clone, Copy, Debug, Default)]
pub struct SlotAlloc {
    next: Slot,
}

impl SlotAlloc {
    /// Number of slots handed out, so a table indexed by `Slot` needs this many entries.
    pub fn len(&self) -> usize {
        self.next as usize
    }

    /// True when no slot has been handed out.
    pub fn is_empty(&self) -> bool {
        self.next == 0
    }

    pub(crate) fn next(&mut self) -> Slot {
        let s = self.next;
        self.next += 1;
        s
    }
}

/// The object an `Op::BeginDict` or `Op::BeginObj` opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjNo {
    /// Allocated earlier by `Op::NewObj`, or filled by the engine for `Op::EpdfFont`.
    Slot(Slot),
    /// A number the engine passed in, such as `pdf_page_group_val`.
    Known(i32),
}

/// One step of output.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// `pdf_puts`, `pdf_printf` or `pdfout`: these bytes go to the PDF buffer, in stream mode through the compressor.
    Puts(Vec<u8>),
    /// As `Puts`, the bytes being `input[start..start + len]` of the file the writer was given, so no copy is made.
    CopyInput { start: usize, len: usize },
    /// `pdfcreateobj(0, 0)` or `pdfnewobjnum()`, storing `obj_ptr` in the slot.
    NewObj(Slot),
    /// The slot's number, in decimal, as `Puts`.
    Num(Slot),
    /// `pdfbegindict(obj, level)`.
    BeginDict { obj: ObjNo, level: i32 },
    /// `pdfbeginobj(obj, level)`.
    BeginObj { obj: ObjNo, level: i32 },
    /// `pdfenddict()`.
    EndDict,
    /// `pdfendobj()`.
    EndObj,
    /// `pdfbeginstream()`.
    BeginStream,
    /// `pdfendstream()`.
    EndStream,
    /// `pdfflush()`.
    Flush,
    /// `tex_printf`: to the terminal and log.
    TexPrint(Vec<u8>),
    /// `pdftex_warn`, with the text the C formats.  The engine adds the `pdfTeX warning: ... (file ...): ` prefix.
    Warn(Vec<u8>),
    /// `copyFont` found a font it replaces.  The engine runs
    /// `fm = lookup_fontmap(basefont); fd = epdf_create_fontdescriptor(fm, stem_v)`, then, if `charset` is given and
    /// `is_subsetable(fm)`, `epdf_mark_glyphs(fd, charset)`, otherwise `embed_whole_font(fd)`.  It stores
    /// `get_fd_objnum(fd)` in `fd_slot` and remembers `fd` under that slot for `EpdfFontName`.
    EpdfFont { fd_slot: Slot, basefont: Vec<u8>, stem_v: i32, charset: Option<Vec<u8>> },
    /// `get_fn_objnum(fd)`, for the `fd` remembered under `fd_slot`, stored in `fn_slot`.  It allocates on first use.
    EpdfFontName { fd_slot: Slot, fn_slot: Slot },
}

/// Printing into an op list, with the C's view of the last byte written.
pub(crate) struct Writer<'s> {
    pub ops: Vec<Op>,
    pub slots: &'s mut SlotAlloc,
    /// `pdflastbyte`.  `pdf_puts` and `copyStream` set it; `pdfout` does not, as in the C.
    pub last_byte: u8,
}

/// `PRINTF_BUF_SIZE` in ptexmac.h: `pdf_printf` goes through `vsnprintf` into a buffer this big.
const PRINTF_BUF_SIZE: usize = 1024;

impl<'s> Writer<'s> {
    pub fn new(slots: &'s mut SlotAlloc) -> Self {
        Writer { ops: Vec::new(), slots, last_byte: 0 }
    }

    fn append(&mut self, s: &[u8]) {
        if let Some(Op::Puts(v)) = self.ops.last_mut() {
            v.extend_from_slice(s);
        } else {
            self.ops.push(Op::Puts(s.to_vec()));
        }
    }

    /// `pdf_puts`.  The C stops at a NUL; nothing here passes one.
    pub fn puts(&mut self, s: &[u8]) {
        self.append(s);
        if let Some(&c) = s.last() {
            self.last_byte = c;
        }
    }

    /// `pdf_printf`, with its truncation to 1023 bytes.
    pub fn printf(&mut self, args: std::fmt::Arguments<'_>) {
        let mut s = String::new();
        let _ = s.write_fmt(args);
        let mut b = s.into_bytes();
        b.truncate(PRINTF_BUF_SIZE - 1);
        self.puts(&b);
    }

    /// `pdf_printf` of an argument that holds raw bytes: `prefix`, `arg`, `suffix`, truncated like `printf`.
    pub fn printf_bytes(&mut self, prefix: &[u8], arg: &[u8], suffix: &[u8]) {
        let mut b = Vec::with_capacity(prefix.len() + arg.len() + suffix.len());
        b.extend_from_slice(prefix);
        b.extend_from_slice(arg);
        b.extend_from_slice(suffix);
        b.truncate(PRINTF_BUF_SIZE - 1);
        self.puts(&b);
    }

    /// `pdfout`.
    pub fn out(&mut self, c: u8) {
        self.append(&[c]);
    }

    /// `pdf_newline`.
    pub fn newline(&mut self) {
        if self.last_byte != b'\n' {
            self.puts(b"\n");
        }
    }

    pub fn op(&mut self, op: Op) {
        self.ops.push(op);
    }

    /// `pdfcreateobj(0, 0)`: a new number, in a new slot.
    pub fn new_obj(&mut self) -> Slot {
        let s = self.slots.next();
        self.ops.push(Op::NewObj(s));
        s
    }

    /// `pdf_printf("%d", slot's number)`.  The last byte is a digit, so it cannot be the newline `pdf_newline` tests for.
    pub fn num(&mut self, s: Slot) {
        self.ops.push(Op::Num(s));
        self.last_byte = b'0';
    }

    /// Raw bytes of the input file, which leave `last_byte` alone like the C's direct `pdfbuf` stores and `fread`s.
    pub fn copy_input(&mut self, start: usize, len: usize) {
        self.ops.push(Op::CopyInput { start, len });
    }

    /// A stream's bytes written directly into `pdfbuf`, leaving `last_byte` alone.
    pub fn raw(&mut self, data: Vec<u8>) {
        self.ops.push(Op::Puts(data));
    }
}
