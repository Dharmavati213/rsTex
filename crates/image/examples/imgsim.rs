//! The port's side of `tests/oracle/gen_image_tables.py`.
//!
//! - `imgsim gen DIR`: writes the generated input files to DIR and prints the cases, one per line:
//!   `name <TAB> params <TAB> image <TAB> image ...`, then the font map lines as `font <TAB> ps <TAB> file`.
//! - `imgsim run DIR PARAMS FONTMAP IMAGE...`: runs a case on files in DIR, reading names as given, and prints each
//!   object as `obj NUM LEN LEVEL` followed by its bytes and a newline, then `seen ...`, `warn ...`, `printed ...`,
//!   `fail ...`.  FONTMAP is a file of `ps <TAB> file` lines, or `-`.
//! - `imgsim queries DIR IMAGE`: the names `copyFont` would look up for that page of a PDF.
//!
//! Test tooling only: it reads and writes files, which the crate itself never does.

#[path = "../tests/common/mod.rs"]
mod common;

use common::sim;
use rstex_image::epdf::{EpdfDocs, PdfInfoParams};
use std::collections::HashMap;
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args[0].as_str() {
        "gen" => gen_files(&args[1]),
        "run" => run(&args[1], &args[2], &args[3], &args[4..]),
        "queries" => queries(&args[1], &args[2]),
        o => panic!("unknown command {o}"),
    }
}

fn gen_files(dir: &str) {
    let set = common::gen_inputs::all();
    std::fs::create_dir_all(dir).unwrap();
    for (name, data) in &set.files {
        std::fs::write(format!("{dir}/{name}"), data).unwrap();
    }
    let mut o = std::io::stdout().lock();
    for c in &set.cases {
        writeln!(o, "{}\t{}\t{}", c.name, c.params, c.images.join("\t")).unwrap();
    }
    for (ps, f) in &set.fontmap {
        writeln!(o, "font\t{ps}\t{f}").unwrap();
    }
}

fn read_file(dir: &str, name: &str) -> Vec<u8> {
    let n = name.strip_prefix("./").unwrap_or(name);
    let p = if n.starts_with('/') { n.to_string() } else { format!("{dir}/{n}") };
    std::fs::read(p).unwrap()
}

fn run(dir: &str, params: &str, fontmap: &str, images: &[String]) {
    let p = sim::Params::parse(params);
    let mut fm: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
    if fontmap != "-" {
        let text = std::fs::read(fontmap).unwrap();
        for line in text.split(|&c| c == b'\n') {
            if let Some(t) = line.iter().position(|&c| c == b'\t') {
                fm.insert(line[..t].to_vec(), line[t + 1..].to_vec());
            }
        }
    }
    let out = sim::run_case(&p, images, fm, |name| read_file(dir, name));
    let mut o = std::io::stdout().lock();
    for (i, (n, b)) in out.objects.iter().enumerate() {
        write!(o, "obj {n} {} {}\n", b.len(), out.levels[i]).unwrap();
        o.write_all(b).unwrap();
        o.write_all(b"\n").unwrap();
    }
    for s in &out.seen {
        writeln!(o, "seen {} {} {} {}", s.pages, s.depth, s.wd, s.ht).unwrap();
    }
    for w in &out.warnings {
        o.write_all(b"warn ").unwrap();
        o.write_all(w).unwrap();
        o.write_all(b"\n").unwrap();
    }
    writeln!(o, "fonts {}", out.fonts).unwrap();
    o.write_all(b"printed ").unwrap();
    o.write_all(&out.printed).unwrap();
    o.write_all(b"\n").unwrap();
    if let Some(m) = &out.failed {
        o.write_all(b"fail ").unwrap();
        o.write_all(m).unwrap();
        o.write_all(b"\n").unwrap();
    }
}

fn queries(dir: &str, image: &str) {
    let im = sim::Image::parse(image);
    let data = read_file(dir, &im.name);
    let mut docs = EpdfDocs::default();
    let Ok(i) = docs.add_document(im.name.as_bytes(), data) else {
        return;
    };
    let doc = docs.doc_mut(i);
    let p = PdfInfoParams { page_num: im.page, page_box: im.page_box, major_wanted: 1, minor_wanted: 9, inclusion_errorlevel: -1 };
    let Ok(info) = doc.read_pdf_info(im.named.as_deref().map(str::as_bytes), &p) else {
        return;
    };
    let mut o = std::io::stdout().lock();
    for q in doc.epdf_font_queries(info.selected_page) {
        o.write_all(&q).unwrap();
        o.write_all(b"\n").unwrap();
    }
}
