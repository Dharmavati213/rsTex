//! The Rust side of `tests/oracle/gen_pdfio_tables.py --check`.  Each line of standard input is a request,
//! and each line of output its answer.
//!
//! ```text
//! D LEVEL PIECE HEX    the hex of that data deflated with Deflater, in pieces of PIECE bytes, or with
//!                      deflate if PIECE is 0
//! I HEX                `TOOK HEX`: inflate, the bytes of the input it took and what came out; or `ERROR why`
//! ```
use std::io::{BufRead, Write};

fn main() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut w = std::io::BufWriter::new(stdout.lock());
    let unhex = |s: &str| -> Vec<u8> {
        let v = |c: u8| (c as char).to_digit(16).unwrap() as u8;
        s.as_bytes().chunks_exact(2).map(|p| v(p[0]) * 16 + v(p[1])).collect()
    };
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let mut f = line.split(' ');
        let mut text = Vec::new();
        match f.next().unwrap() {
            "D" => {
                let level: i32 = f.next().unwrap().parse().unwrap();
                let piece: usize = f.next().unwrap().parse().unwrap();
                let data = unhex(f.next().unwrap_or(""));
                let mut out = Vec::new();
                if piece == 0 {
                    rstex_pdfio::deflate(&data, level, &mut out);
                } else {
                    let mut d = rstex_pdfio::Deflater::new(level);
                    let mut chunks = data.chunks(piece).peekable();
                    while let Some(c) = chunks.next() {
                        if chunks.peek().is_some() {
                            d.write(c, &mut out);
                        } else {
                            d.finish(c, &mut out);
                        }
                    }
                    if data.is_empty() {
                        d.finish(b"", &mut out);
                    }
                }
                rstex_pdfio::write_hex(&mut text, &out);
            }
            "I" => {
                let data = unhex(f.next().unwrap_or(""));
                let mut out = Vec::new();
                match rstex_pdfio::inflate(&data, &mut out) {
                    Ok(took) => {
                        rstex_pdfio::write_int(&mut text, took as i64);
                        text.push(b' ');
                        rstex_pdfio::write_hex(&mut text, &out);
                    }
                    Err(e) => text.extend_from_slice(format!("ERROR {e}").as_bytes()),
                }
            }
            other => panic!("unknown request {other}"),
        }
        text.push(b'\n');
        w.write_all(&text).unwrap();
    }
}
