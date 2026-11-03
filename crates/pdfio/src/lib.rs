//! Bytes of a PDF: deflate and inflate, MD5, the chunk writer, and number formatting.
//!
//! This is a leaf crate (DESIGN.md sections 6 and 9).  Everything here is a stateless function or a
//! plain value that the engine owns; nothing calls back into the engine, opens a file or reads the
//! environment.  It is pure Rust: `zlib-rs` does the compression.
//!
//! | Module | What it holds | C it replaces |
//! |---|---|---|
//! | [`flate`] | [`deflate`], [`Deflater`], [`inflate`], [`Inflater`], the levels of DESIGN.md section 9 | `writezip.c`, zlib |
//! | [`md5`] | [`md5::md5`], [`Md5`] | `md5.c` |
//! | [`chunk`] | [`ChunkWriter`], [`Deferred`] | the `pdffile` stream, its seeks in `writestreamlength` |
//! | [`workers`] | [`DeflatePool`] | nothing: DESIGN.md section 9's whole streams on other threads |
//! | [`numfmt`] | [`write_int`], [`write_real`], [`write_g`], ... | `pdf_print_int` and friends, `%g`, `%02X` |
//!
//! # Writing a PDF
//!
//! The output is a list of chunks in memory and nothing ever seeks.  A field whose value is not yet known
//! is either reserved with [`ChunkWriter::defer`] and filled through the [`Deferred`] it returns, or
//! patched by absolute offset with [`ChunkWriter::patch`], which is what WEB's `write_stream_length`
//! needs since it remembers `pdf_stream_length_offset` as a number.  At the end the owner of the file
//! writes every chunk out, and publishes it.
//!
//! ```
//! use rstex_pdfio::{ChunkWriter, deflate, write_int, LEVEL_CONTENT};
//!
//! let mut pdf = ChunkWriter::new();
//! pdf.write(b"1 0 obj\n<< /Length ");
//! let length = pdf.defer(10);                  // ten blanks, as pdf_begin_stream writes
//! pdf.write(b" /Filter /FlateDecode >>\nstream\n");
//! let start = pdf.offset();
//!
//! let mut z = Vec::new();
//! deflate(b"BT /F1 10 Tf (hello) Tj ET", LEVEL_CONTENT, &mut z);
//! pdf.write(&z);
//!
//! let mut digits = Vec::new();
//! write_int(&mut digits, pdf.offset() - start);
//! pdf.fill(length, &digits);                   // padded on the right, as the blanks stay
//! pdf.write(b"\nendstream\nendobj\n");
//!
//! let mut file = Vec::new();                   // a File, in the engine's Host
//! pdf.write_to(&mut file).unwrap();
//! assert!(file.starts_with(b"1 0 obj\n<< /Length "));
//! ```
//!
//! # Compressing a stream
//!
//! WEB flushes `pdf_buf` piecemeal, calling `write_zip(false)` each time and `write_zip(true)` at the
//! end; [`Deflater`] has that shape, with one value per stream or one kept and reset, as `writezip.c`
//! keeps its `z_stream`.
//!
//! **The bytes depend on where the pieces begin**, unlike those of system zlib: `tests/oracle/gen_pdfio_tables.py
//! --check` measures it for zlib-rs 0.6.8 at levels 0, 1, 5 and 6, so at both levels used here.  All of it in one
//! [`Deflater::finish`] is what [`deflate`] gives.  The same pieces always give the same bytes, and WEB's
//! pieces come from `pdf_buf`, so a run is reproducible.  But a path that compresses whole streams, like a
//! worker of DESIGN.md section 9, writes other bytes than one that goes piecemeal, valid as both are, so the engine
//! writes every stream whole, with a worker or without.
//!
//! Levels are those of DESIGN.md section 9, not `\pdfcompresslevel`: see [`level_for`].
//! The output does not match the bytes of system zlib, and is not meant to: oracle comparisons of
//! compressed streams inflate them first, and L6 runs with `\pdfcompresslevel=0`.
#![forbid(unsafe_code)]

pub mod chunk;
pub mod flate;
pub mod md5;
pub mod numfmt;
pub mod workers;

pub use chunk::{ChunkWriter, Deferred};
pub use flate::{Deflater, InflateError, Inflater, LEVEL_CONTENT, LEVEL_FONTFILE, Stream, deflate, inflate, level_for};
pub use md5::Md5;
pub use numfmt::{write_fw_int, write_g, write_hex, write_hex_width, write_int, write_out_bytes, write_real};
pub use workers::DeflatePool;
