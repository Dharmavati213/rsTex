//! pdfio against what the oracle and system zlib produce: tables in `testdata/`, made by
//! `tests/oracle/gen_pdfio_tables.py`.  Its `--check` mode goes further, with streams too long to commit.
use rstex_pdfio::md5::md5;
use rstex_pdfio::*;

fn unhex(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let v = |c: u8| (c as char).to_digit(16).unwrap() as u8;
    b.chunks_exact(2).map(|p| v(p[0]) * 16 + v(p[1])).collect()
}

fn upper(d: &[u8]) -> String {
    let mut v = Vec::new();
    write_hex(&mut v, d);
    String::from_utf8(v).unwrap()
}

/// The same bytes as the generator makes: all byte values, with hardly any structure.
fn pattern(seed: u32, n: usize) -> Vec<u8> {
    let mut x = seed;
    (0..n)
        .map(|_| {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            (x >> 24) as u8
        })
        .collect()
}

/// The input a table row describes, and the fields after it.  See `expand_spec` in the generator.
fn spec<'a>(f: &'a [&'a str]) -> (Vec<u8>, &'a [&'a str]) {
    match f[0] {
        "hex" => (unhex(f[1]), &f[2..]),
        "repeat" => (vec![f[1].parse().unwrap(); f[2].parse().unwrap()], &f[3..]),
        "range" => ((f[1].parse::<u16>().unwrap()..=f[2].parse().unwrap()).map(|b| b as u8).collect(), &f[3..]),
        "pattern" => (pattern(f[1].parse().unwrap(), f[2].parse().unwrap()), &f[3..]),
        other => panic!("unknown input {other}"),
    }
}

/// `\pdfmdfivesum{...}` by pdfTeX itself, on text it can be given as tokens.
#[test]
fn md5_of_strings_as_pdftex() {
    let mut n = 0;
    for line in include_str!("../testdata/md5_oracle.tsv").lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f[0] == "file" {
            continue;
        }
        let (data, rest) = spec(&f);
        assert_eq!(upper(&md5(&data)), rest[0], "{line}");
        n += 1;
    }
    assert!(n >= 30, "{n} cases");
}

/// `\pdfmdfivesum file{...}` by pdfTeX itself, on files holding every byte value, around each length
/// where the padding of MD5 changes.
#[test]
fn md5_of_files_as_pdftex() {
    let mut n = 0;
    for line in include_str!("../testdata/md5_oracle.tsv").lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f[0] != "file" {
            continue;
        }
        let (seed, len): (u32, usize) = (f[1].parse().unwrap(), f[2].parse().unwrap());
        let data = pattern(seed, len);
        assert_eq!(upper(&md5(&data)), f[3], "seed {seed}, length {len}");
        // And given in pieces, as getmd5sum reads a file 1024 bytes at a time.
        let mut m = Md5::new();
        for c in data.chunks(1024) {
            m.update(c);
        }
        assert_eq!(upper(&m.finish()), f[3]);
        n += 1;
    }
    assert!(n >= 20, "{n} cases");
}

/// Streams made by system zlib, which is what pdfTeX compresses with and what other programs write.
#[test]
fn inflate_what_zlib_made() {
    let mut n = 0;
    for line in include_str!("../testdata/inflate_oracle.tsv").lines() {
        let f: Vec<&str> = line.split('\t').collect();
        let level = f[0];
        let (plain, rest) = spec(&f[1..]);
        let z = unhex(rest[0]);
        let mut out = Vec::new();
        assert_eq!(inflate(&z, &mut out), Ok(z.len()), "level {level}");
        assert_eq!(out, plain, "level {level}");
        // In pieces too.
        let mut i = Inflater::new();
        let mut out = Vec::new();
        for c in z.chunks(3) {
            i.push(c, &mut out).unwrap();
        }
        assert!(i.finished());
        assert_eq!(out, plain);
        n += 1;
    }
    assert!(n >= 20, "{n} cases");
}
