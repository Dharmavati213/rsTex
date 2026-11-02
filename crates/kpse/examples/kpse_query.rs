//! The Rust side of `tests/oracle/kpse_compare.py`.  It reads requests from standard input, one to a
//! line, tab separated, with names in hex so that any byte can pass:
//!
//! ```text
//! find FORMAT MUST_EXIST NAME          kpse_find_file
//! all FORMAT MUST_EXIST NAME           kpse_find_file_generic, all
//! pathsearch MUST_EXIST PATH NAME      kpsewhich -path=PATH NAME
//! allpath PATH NAME                    kpsewhich -all -path=PATH NAME
//! var NAME                             kpsewhich -var-value=NAME
//! showpath FORMAT                      kpsewhich -show-path=FORMAT
//! expandvar S | expandbraces S | expandpath S
//! safeout S | safeoutext S | safein S | safeinext S
//! scans                                directories opened so far, as the hex of its decimal digits
//! ```
//!
//! and prints one line per request: comma-separated hex results, or `-` for none.
use rstex_kpse::{Config, Kpse};
use std::io::{BufRead, Write};

fn unhex(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let v = |c: u8| (c as char).to_digit(16).expect("hex") as u8;
    b.chunks_exact(2).map(|p| v(p[0]) * 16 + v(p[1])).collect()
}

fn hex(s: &[u8], out: &mut String) {
    const H: &[u8; 16] = b"0123456789abcdef";
    for &c in s {
        out.push(H[(c >> 4) as usize] as char);
        out.push(H[(c & 15) as usize] as char);
    }
}

fn results(list: &[Vec<u8>]) -> String {
    let mut out = String::new();
    for (i, r) in list.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        hex(r, &mut out);
    }
    if list.is_empty() {
        out.push('-');
    }
    out
}

fn main() {
    let mut cfg = Config::default();
    let mut cnf_lines = Vec::new();
    for a in std::env::args().skip(1) {
        if let Some(p) = a.strip_prefix("--progname=") {
            cfg.program_name = p.as_bytes().to_vec();
        } else if let Some(e) = a.strip_prefix("--engine=") {
            cfg.engine = Some(e.as_bytes().to_vec());
        } else if a == "--no-engine" {
            cfg.engine = None;
        } else if let Some(l) = a.strip_prefix("--cnf-line=") {
            cnf_lines.push(l.as_bytes().to_vec());
        } else {
            panic!("unknown argument {a}");
        }
    }
    let mut k = Kpse::new(cfg);
    for l in &cnf_lines {
        k.cnf_line_env_progname(l);
    }
    // As kpsewhich does: its default resolution is 600, and it has no mode unless told.
    let upper = k.program_name().to_ascii_uppercase();
    k.init_prog(&upper, 600, None);
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    for line in stdin.lock().lines() {
        let line = line.expect("stdin");
        let f: Vec<&str> = line.split('\t').collect();
        let one = |v: Option<Vec<u8>>| results(&v.into_iter().collect::<Vec<_>>());
        let flag = |b: bool| if b { "31".to_string() } else { "30".to_string() };
        let r = match f[0] {
            "find" => one(k.find_file(&unhex(f[3]), f[1].parse().unwrap(), f[2] == "1")),
            "all" => results(&k.find_file_generic(&unhex(f[3]), f[1].parse().unwrap(), f[2] == "1", true)),
            "pathsearch" => {
                let p = k.path_expand(&unhex(f[2]));
                one(k.path_search(&p, &unhex(f[3]), f[1] == "1"))
            }
            "allpath" => {
                let p = k.path_expand(&unhex(f[1]));
                results(&k.all_path_search(&p, &unhex(f[2])))
            }
            "var" => one(k.var_value(&unhex(f[1]))),
            "showpath" => one(k.format_path(f[1].parse().unwrap())),
            "expandvar" => one(Some(k.var_expand(&unhex(f[1])))),
            "expandbraces" => one(Some(k.brace_expand(&unhex(f[1])))),
            "expandpath" => one(Some(k.path_expand(&unhex(f[1])))),
            "safeout" => flag(k.out_name_ok_silent(&unhex(f[1]))),
            "safeoutext" => flag(k.out_name_ok_silent_extended(&unhex(f[1]))),
            "safein" => flag(k.in_name_ok(&unhex(f[1]))),
            "safeinext" => flag(k.in_name_ok_extended(&unhex(f[1]))),
            "scans" => one(Some(k.dir_scans().to_string().into_bytes())),
            other => panic!("unknown request {other}"),
        };
        writeln!(out, "{r}").unwrap();
    }
}
