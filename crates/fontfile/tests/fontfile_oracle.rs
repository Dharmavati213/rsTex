//! The tables in testdata/, from `tests/oracle/gen_fontfile_tables.py`, against this crate.
//!
//! `fontfile_syn.tsv`, `fontfile_float.tsv` and `fontfile_map.tsv` need nothing installed: the synthetic
//! fonts are made here, byte for byte as the script made them for the oracle, and the digest of each says so.
//! `fontfile_tex.tsv` and `fontfile_uni.tsv` use TeX Live's fonts through `kpsewhich`: a row is skipped when
//! the file is not there or is not the one the table was made from, and the test says how many ran.
use rstex_fontfile::*;
use std::process::Command;

fn table(name: &str) -> Vec<Vec<String>> {
    let path = format!("{}/testdata/{}", env!("CARGO_MANIFEST_DIR"), name);
    let data = std::fs::read(&path).expect("the table");
    let text: String = data.iter().map(|&b| b as char).collect(); // latin-1
    text.lines().map(|l| l.split('\t').map(|s| s.to_string()).collect()).collect()
}

fn bytes(s: &str) -> Vec<u8> {
    s.chars().map(|c| c as u32 as u8).collect()
}

fn text(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

fn kpse(name: &str) -> Option<String> {
    let o = Command::new("kpsewhich").arg(name).output().ok()?;
    let p = String::from_utf8(o.stdout).ok()?;
    let p = p.trim();
    if p.is_empty() { None } else { Some(p.to_string()) }
}

/// Like the job in the script: what create_fontdictionary and writet1 do with one font.
struct One {
    length: (i32, i32, i32),
    out: Vec<u8>,
    name: String,
    flags: i32,
    charset: String,
    dims: FontDims,
    warnings: Vec<Vec<u8>>,
    builtin: Option<GlyphNames>,
}

fn run_t1(
    font: &[u8],
    fm: &FmEntry<'_>,
    enc: Option<&GlyphNames>,
    tfm: &[u8],
    codes: &[usize],
) -> Result<One, FontError> {
    let mut marked = CodeSet::new();
    for &c in codes {
        marked.insert(c);
    }
    let (first, last) = get_char_range(&marked, 0, 255);
    let mut gl = GlyphSet::new();
    let mut tx = None;
    if let Some(n) = enc {
        if fm.is_subsetted() {
            mark_reenc_glyphs(n, &marked, first, last, &mut gl);
        }
    } else {
        let mut t = CodeSet::new();
        mark_chars(&mut t, &marked, first, last);
        tx = Some(t);
    }
    let preset = fm.ps_name.unwrap_or(tfm);
    let mut dims = new_font_dims();
    let mut carry = T1Carry::default();
    let mut out = Vec::new();
    let inp = T1Input {
        font,
        subsetted: fm.is_subsetted(),
        all_glyphs: false,
        slant: fm.slant,
        extend: fm.extend,
        fontname: preset,
        tx_codes: tx.as_ref(),
        used_tags: &[],
    };
    let o = write_t1(&inp, &mut dims, &mut gl, &mut carry, &mut out)?;
    let mut name = Vec::new();
    if let Some(t) = o.subset_tag {
        name.extend_from_slice(&t);
        name.push(b'+');
    }
    name.extend_from_slice(&o.fontname);
    let mut charset = Vec::new();
    for g in gl.iter() {
        charset.push(b'/');
        charset.extend_from_slice(g);
    }
    let (flags, _) = font_descriptor_flags(fm.fd_flags, true, fm.ps_name, tfm, fm.is_std_t1font());
    Ok(One {
        length: (o.length1, o.length2, o.length3),
        out,
        name: text(&name),
        flags,
        charset: text(&charset),
        dims,
        warnings: o.warnings,
        builtin: o.builtin_glyph_names,
    })
}

// ---- synthetic Type 1 fonts: the same as synth_t1 in tests/oracle/fontfile_gate.py ----

fn eexec(b: &[u8], mut key: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    for &p in b {
        let c = p ^ (key >> 8) as u8;
        key = (c as u32 + key as u32).wrapping_mul(52845).wrapping_add(22719) as u16;
        out.push(c);
    }
    out
}

fn cs_enc(plain: &[u8], leniv: usize) -> Vec<u8> {
    let mut v = vec![0u8; leniv];
    v.extend_from_slice(plain);
    eexec(&v, 4330)
}

/// A charstring from words: numbers and operator names.
fn cs(words: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for w in words.split_whitespace() {
        if let Ok(v) = w.parse::<i32>() {
            if (-107..=107).contains(&v) {
                out.push((v + 139) as u8);
            } else if (108..=1131).contains(&v) {
                let x = v - 108;
                out.push((247 + (x >> 8)) as u8);
                out.push((x & 255) as u8);
            } else if (-1131..=-108).contains(&v) {
                let x = -v - 108;
                out.push((251 + (x >> 8)) as u8);
                out.push((x & 255) as u8);
            } else {
                out.push(255);
                out.extend_from_slice(&v.to_be_bytes());
            }
            continue;
        }
        let one = match w {
            "hstem" => Some(1),
            "vstem" => Some(3),
            "vmoveto" => Some(4),
            "rlineto" => Some(5),
            "hlineto" => Some(6),
            "vlineto" => Some(7),
            "rrcurveto" => Some(8),
            "closepath" => Some(9),
            "callsubr" => Some(10),
            "ret" => Some(11),
            "hsbw" => Some(13),
            "endchar" => Some(14),
            "rmoveto" => Some(21),
            "hmoveto" => Some(22),
            _ => None,
        };
        if let Some(b) = one {
            out.push(b);
            continue;
        }
        let esc = match w {
            "seac" => 6,
            "div" => 12,
            "callothersubr" => 16,
            "pop" => 17,
            "setcurrentpoint" => 33,
            other => panic!("no operator {}", other),
        };
        out.push(12);
        out.push(esc);
    }
    out
}

const SIMPLE: &str = "0 500 hsbw 100 100 rmoveto 200 hlineto closepath endchar";
const NOTDEF_CS: &str = "0 250 hsbw endchar";

struct Syn {
    name: Vec<u8>,
    encoding: Vec<u8>,
    glyphs: Vec<(Vec<u8>, Vec<u8>)>,
    subrs: Option<Vec<(i32, Vec<u8>)>>,
    leniv: Option<i32>,
    rd: Vec<u8>,
    np: Vec<u8>,
    nd: Vec<u8>,
    fonttype: i32,
    header_extra: Vec<u8>,
    private_extra: Vec<u8>,
    charstrings_line: Option<Vec<u8>>,
    subrs_line: Option<Vec<u8>>,
    fontname: bool,
    declared_glyphs: Option<i32>,
    fontmatrix: Vec<u8>,
    synthetic_prefix: bool,
    pfb: bool,
    cut: Option<usize>,
}

fn syn() -> Syn {
    Syn {
        name: b"Syn".to_vec(),
        encoding: b"/Encoding StandardEncoding def\n".to_vec(),
        glyphs: vec![
            (b".notdef".to_vec(), cs(NOTDEF_CS)),
            (b"A".to_vec(), cs(SIMPLE)),
            (b"B".to_vec(), cs(SIMPLE)),
        ],
        subrs: None,
        leniv: None,
        rd: b"RD".to_vec(),
        np: b"NP".to_vec(),
        nd: b"ND".to_vec(),
        fonttype: 1,
        header_extra: Vec::new(),
        private_extra: Vec::new(),
        charstrings_line: None,
        subrs_line: None,
        fontname: true,
        declared_glyphs: None,
        fontmatrix: b"[0.001 0 0 0.001 0 0]".to_vec(),
        synthetic_prefix: false,
        pfb: true,
        cut: None,
    }
}

impl Syn {
    fn subr_block(&self, items: &[(i32, Vec<u8>)], iv: usize) -> Vec<u8> {
        let mut b = match &self.subrs_line {
            Some(l) => l.clone(),
            None => format!("/Subrs {} array\n", items.len()).into_bytes(),
        };
        for (i, plain) in items {
            let e = cs_enc(plain, iv);
            b.extend_from_slice(format!("dup {} {} ", i, e.len()).as_bytes());
            b.extend_from_slice(&self.rd);
            b.push(b' ');
            b.extend_from_slice(&e);
            b.push(b' ');
            b.extend_from_slice(&self.np);
            b.push(b'\n');
        }
        b.extend_from_slice(b"ND\n");
        b
    }

    fn build(&self) -> Vec<u8> {
        let iv = self.leniv.unwrap_or(4).max(0) as usize;
        let mut h: Vec<u8> = b"%!PS-AdobeFont-1.0: ".to_vec();
        h.extend_from_slice(&self.name);
        h.extend_from_slice(b" 1.0\n%");
        h.extend_from_slice(&[b'x'; 600]);
        h.extend_from_slice(b"\n12 dict begin\n/FontInfo 3 dict dup begin\n/FullName (");
        h.extend_from_slice(&self.name);
        h.extend_from_slice(b") readonly def\n/ItalicAngle -12.5 def\nend readonly def\n");
        if self.fontname {
            h.extend_from_slice(b"/FontName /");
            h.extend_from_slice(&self.name);
            h.extend_from_slice(b" def\n");
        }
        h.extend_from_slice(format!("/PaintType 0 def\n/FontType {} def\n", self.fonttype).as_bytes());
        h.extend_from_slice(b"/FontMatrix ");
        h.extend_from_slice(&self.fontmatrix);
        h.extend_from_slice(b" readonly def\n");
        h.extend_from_slice(&self.encoding);
        h.extend_from_slice(b"/FontBBox {-10 -20 1000 900} readonly def\n/UniqueID 5000000 def\n");
        h.extend_from_slice(&self.header_extra);
        h.extend_from_slice(b"currentdict end\ncurrentfile eexec\n");

        let mut p: Vec<u8> = b"dup /Private 16 dict dup begin\n".to_vec();
        p.extend_from_slice(b"/RD {string currentfile exch readstring pop} executeonly def\n");
        p.extend_from_slice(b"/ND {noaccess def} executeonly def\n/NP {noaccess put} executeonly def\n");
        p.extend_from_slice(b"/BlueValues [] def\n/password 5839 def\n/UniqueID 5000000 def\n");
        if let Some(l) = self.leniv {
            p.extend_from_slice(format!("/lenIV {} def\n", l).as_bytes());
        }
        p.extend_from_slice(&self.private_extra);
        if self.synthetic_prefix {
            p.extend_from_slice(&self.subr_block(&[(0, cs("ret"))], iv));
            p.extend_from_slice(b"hires 1 def\nhires 2 def\nhires 3 def\nhires 4 def\nhires 5 def\nhires 6 def\n");
        }
        if let Some(s) = &self.subrs {
            p.extend_from_slice(&self.subr_block(s, iv));
        }
        match &self.charstrings_line {
            Some(l) => p.extend_from_slice(l),
            None => {
                let n = self.declared_glyphs.unwrap_or(self.glyphs.len() as i32);
                p.extend_from_slice(format!("2 index /CharStrings {} dict dup begin\n", n).as_bytes());
            }
        }
        for (g, plain) in &self.glyphs {
            let e = cs_enc(plain, iv);
            p.push(b'/');
            p.extend_from_slice(g);
            p.extend_from_slice(format!(" {} ", e.len()).as_bytes());
            p.extend_from_slice(&self.rd);
            p.push(b' ');
            p.extend_from_slice(&e);
            p.push(b' ');
            p.extend_from_slice(&self.nd);
            p.push(b'\n');
        }
        p.extend_from_slice(b"end\nend\nreadonly put\nnoaccess put\ndup /FontName get exch definefont pop\nmark currentfile closefile\n");
        let mut plain = vec![0u8; 4];
        plain.extend_from_slice(&p);
        let mut enc = eexec(&plain, 55665);
        if let Some(c) = self.cut {
            enc.truncate(c);
        }
        let mut tail = Vec::new();
        for _ in 0..8 {
            tail.extend_from_slice(&[b'0'; 64]);
            tail.push(b'\n');
        }
        tail.extend_from_slice(b"cleartomark\n");
        let mut out = Vec::new();
        if self.pfb {
            out.extend_from_slice(&[0x80, 1]);
            out.extend_from_slice(&(h.len() as u32).to_le_bytes());
            out.extend_from_slice(&h);
            out.extend_from_slice(&[0x80, 2]);
            out.extend_from_slice(&(enc.len() as u32).to_le_bytes());
            out.extend_from_slice(&enc);
            out.extend_from_slice(&[0x80, 1]);
            out.extend_from_slice(&(tail.len() as u32).to_le_bytes());
            out.extend_from_slice(&tail);
            out.extend_from_slice(&[0x80, 3]);
        } else {
            out.extend_from_slice(&h);
            let hex: Vec<u8> = enc.iter().flat_map(|b| format!("{:02x}", b).into_bytes()).collect();
            for (i, chunk) in hex.chunks(64).enumerate() {
                if i > 0 {
                    out.push(b'\n');
                }
                out.extend_from_slice(chunk);
            }
            out.push(b'\n');
            out.extend_from_slice(&tail);
        }
        out
    }
}

fn hint_subrs() -> Vec<(i32, Vec<u8>)> {
    vec![
        (0, cs("ret")),
        (1, cs("ret")),
        (2, cs("ret")),
        (3, cs("ret")),
        (4, cs("10 20 hstem ret")),
        (5, cs("30 40 vstem ret")),
        (6, cs("50 60 hstem ret")),
    ]
}

fn hint_glyphs() -> Vec<(Vec<u8>, Vec<u8>)> {
    vec![
        (b".notdef".to_vec(), cs(NOTDEF_CS)),
        (b"A".to_vec(), cs("0 500 hsbw 4 1 3 callothersubr pop callsubr 10 10 rmoveto endchar")),
        (b"B".to_vec(), cs(SIMPLE)),
    ]
}

fn glyph_a(words: &str) -> Vec<(Vec<u8>, Vec<u8>)> {
    vec![(b".notdef".to_vec(), cs(NOTDEF_CS)), (b"A".to_vec(), cs(words))]
}

fn glyph_a_bytes(prefix: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut v = prefix.to_vec();
    v.extend_from_slice(&cs(SIMPLE));
    vec![(b".notdef".to_vec(), cs(NOTDEF_CS)), (b"A".to_vec(), v)]
}

const DUP_ENC: &[u8] = b"/Encoding 256 array\n0 1 255 {1 index exch /.notdef put} for\ndup 65 /A put\ndup 66 /B put\ndup 67 /B put\nreadonly def\n";

/// The case of the given name in `synth_cases` of the script.
fn case(name: &str) -> Vec<u8> {
    let seac_glyphs = || {
        vec![
            (b".notdef".to_vec(), cs(NOTDEF_CS)),
            (b"A".to_vec(), cs(SIMPLE)),
            (b"B".to_vec(), cs(SIMPLE)),
            (b"acute".to_vec(), cs(SIMPLE)),
            (b"Aacute".to_vec(), cs("0 500 hsbw 0 0 0 65 194 seac endchar")),
        ]
    };
    let hints = || Syn { subrs: Some(hint_subrs()), glyphs: hint_glyphs(), ..syn() };
    let s = match name {
        "std" => syn(),
        "std_pfa" => Syn { pfb: false, ..syn() },
        "dupenc" | "notdef_code" => Syn { encoding: DUP_ENC.to_vec(), ..syn() },
        "arrayenc" => Syn { encoding: b"/Encoding [ /.notdef /A /B\n% comment\n/B /.notdef ] readonly def\n".to_vec(), ..syn() },
        "dupops" => Syn {
            encoding: b"/Encoding 256 array\n0 1 255 {1 index exch /.notdef put} for\ndup 65 /A put dup 66 /B put\ndup dup 67 exch 65 get put\ndup dup 65 1 getinterval 70 exch putinterval\ndup 0x48 /B put\ndup 010 /B put\nsome junk here\nreadonly def\n".to_vec(),
            ..syn()
        },
        "seac" => Syn {
            encoding: b"/Encoding 256 array\n0 1 255 {1 index exch /.notdef put} for\ndup 65 /A put\ndup 66 /Aacute put\nreadonly def\n".to_vec(),
            glyphs: seac_glyphs(),
            ..syn()
        },
        "seac_std" => Syn { glyphs: seac_glyphs(), ..syn() },
        "hints" | "hints_b" => hints(),
        "flex" => Syn {
            glyphs: glyph_a("0 500 hsbw 1 callsubr 10 10 rmoveto 2 callsubr 50 5 5 3 0 callothersubr pop pop setcurrentpoint endchar"),
            ..hints()
        },
        "noreturn" => {
            let mut sub = hint_subrs();
            sub.push((7, cs("1 2 hstem")));
            Syn { subrs: Some(sub), glyphs: glyph_a("0 500 hsbw 7 callsubr endchar"), ..syn() }
        }
        "emptysubr" => Syn { subrs: Some(vec![(0, Vec::new())]), glyphs: glyph_a("0 500 hsbw 0 callsubr endchar"), ..syn() },
        "div" => Syn { glyphs: glyph_a("0 1000 2 div hsbw 70000 2 div 0 rmoveto endchar"), ..syn() },
        "leniv0" => Syn { leniv: Some(0), ..hints() },
        "leniv1" => Syn { leniv: Some(1), ..hints() },
        "leniv7" => Syn { leniv: Some(7), ..hints() },
        "tokens_dash" => Syn {
            rd: b"-|".to_vec(),
            np: b"|".to_vec(),
            nd: b"|-".to_vec(),
            private_extra: b"/-| {string currentfile exch readstring pop} executeonly def\n".to_vec(),
            ..hints()
        },
        "tokens_put" => Syn { np: b"noaccess put".to_vec(), ..hints() },
        "tokens_dash_put" => Syn { rd: b"-|".to_vec(), np: b"noaccess put".to_vec(), nd: b"|-".to_vec(), ..hints() },
        "tokens_bad" => Syn { np: b"XX".to_vec(), ..hints() },
        "unusual_cs" => Syn { charstrings_line: Some(b"dup/CharStrings\n3 dict dup begin\n".to_vec()), ..syn() },
        "subrs0" => Syn { subrs: Some(Vec::new()), glyphs: glyph_a(SIMPLE), ..syn() },
        "synthetic" => Syn { synthetic_prefix: true, ..hints() },
        "nofontname" => Syn { fontname: false, ..syn() },
        "type3" => Syn { fonttype: 3, ..syn() },
        "leniv_neg" => Syn { leniv: Some(-1), ..syn() },
        "undefined_glyph" => Syn {
            encoding: b"/Encoding 256 array\n0 1 255 {1 index exch /.notdef put} for\ndup 65 /A put\ndup 66 /Nope put\nreadonly def\n".to_vec(),
            ..syn()
        },
        "subr_range" => Syn { glyphs: glyph_a("0 500 hsbw 99 callsubr endchar"), subrs: Some(hint_subrs()), ..syn() },
        "subr_store_range" => Syn { subrs_line: Some(b"/Subrs 2 array\n".to_vec()), ..hints() },
        "bad_command" => Syn { glyphs: glyph_a_bytes(&[2]), ..syn() },
        "bad_escape" => Syn { glyphs: glyph_a_bytes(&[12, 5]), ..syn() },
        "escape_range" => Syn { glyphs: glyph_a_bytes(&[12, 0x7f]), ..syn() },
        "more_args" => Syn { glyphs: glyph_a("1 0 500 hsbw endchar"), ..syn() },
        "less_args" => Syn { glyphs: glyph_a("500 hsbw endchar"), ..syn() },
        "stack_pop" => Syn { glyphs: glyph_a("div endchar"), ..syn() },
        "too_many_glyphs" => Syn { declared_glyphs: Some(1), ..syn() },
        "no_number" => Syn { charstrings_line: Some(b"2 index /CharStrings x dict dup begin\n".to_vec()), ..syn() },
        "bad_fontmatrix" => Syn { fontmatrix: b"0.001 0 0 0.001 0 0".to_vec(), ..syn() },
        "brace_fontmatrix" => Syn { fontmatrix: b"{0.001 0 0 0.001 0 0}".to_vec(), ..syn() },
        "unclosed_fontmatrix" => Syn { fontmatrix: b"[0.001 0 0 0.001 0 0".to_vec(), ..syn() },
        "pred_enc_bad" => Syn { encoding: b"/Encoding Foo def\n".to_vec(), ..syn() },
        "array_enc_bad" => Syn { encoding: b"/Encoding [ /A /B foo ] readonly def\n".to_vec(), ..syn() },
        "noname_font" => Syn { header_extra: b"/FontName 5 def\n".to_vec(), ..syn() },
        "uniqueid_forms" => Syn { header_extra: b"/UniqueID 77 def\n/UniqueID 77 noaccess\n".to_vec(), ..syn() },
        "stemv" => Syn {
            header_extra: b"/StdVW [85] def\n/XHeight 431 def\n/CapHeight 683 def\n/Ascender 694 def\n/Descender -194 def\n".to_vec(),
            ..syn()
        },
        "truncated" => Syn { cut: Some(60), ..syn() },
        _ => panic!("no case {}", name),
    };
    s.build()
}

fn parse_codes(s: &str) -> Vec<usize> {
    s.split(',').filter(|x| !x.is_empty()).map(|x| x.parse().unwrap()).collect()
}

fn list(s: &str) -> Vec<String> {
    if s == "-" {
        Vec::new()
    } else {
        s.split("|||").map(|x| x.to_string()).collect()
    }
}

/// A message can quote a line with binary bytes, which the log shows otherwise: up to there it must agree.
fn check_error(context: &str, port: &FontError, oracle: &str) {
    let m = text(&port.message);
    let n = m.chars().position(|c| !(' '..'\u{7f}').contains(&c)).unwrap_or(m.len());
    if n == m.len() {
        assert_eq!(m, oracle, "{}", context);
    } else {
        assert!(oracle.starts_with(&m[..n]), "{}: {} against {}", context, m, oracle);
    }
}

fn check_syn_row(r: &[String], font: &[u8]) {
    let (name, specials, codes) = (&r[0], &r[1], parse_codes(&r[2]));
    assert_eq!(md5_hex(font), r[3], "{}: the synthetic font is not the script's", name);
    let mut line = b"cmr10 Syn ".to_vec();
    if specials != "-" {
        line.extend_from_slice(&bytes(specials));
    }
    line.extend_from_slice(b" <syn.pfb");
    let scan = fm_scan_line(&line).unwrap();
    let want_warn: Vec<String> = list(&r[16]);
    let mut got_warn: Vec<String> = scan.warnings.iter().map(|w| text(w)).collect();
    let error = list(&r[17]);
    let fm = match scan.entry {
        Some(fm) => fm,
        None => {
            // A bad line leaves pdftex.map's own cmr10 in charge: there is nothing of this font to compare.
            assert_eq!(got_warn, want_warn, "{}", name);
            return;
        }
    };
    match run_t1(font, &fm, None, b"cmr10", &codes) {
        Err(e) => {
            assert_eq!(error.len(), 1, "{}: the port failed and the oracle did not: {}", name, e);
            check_error(name, &e, &error[0]);
        }
        Ok(o) => {
            assert!(error.is_empty(), "{}: the oracle failed and the port did not: {:?}", name, error);
            assert_eq!(o.length.0.to_string(), r[4], "{} Length1", name);
            assert_eq!(o.length.1.to_string(), r[5], "{} Length2", name);
            assert_eq!(o.length.2.to_string(), r[6], "{} Length3", name);
            assert_eq!(md5_hex(&o.out), r[7], "{} stream", name);
            assert_eq!(o.name, r[8], "{} name", name);
            if o.dims[ITALIC_ANGLE_CODE].set {
                assert_eq!(o.dims[ITALIC_ANGLE_CODE].val.to_string(), r[9], "{} ItalicAngle", name);
            }
            let bbox = format!(
                "{} {} {} {}",
                o.dims[FONTBBOX1_CODE].val, o.dims[FONTBBOX2_CODE].val, o.dims[FONTBBOX3_CODE].val, o.dims[FONTBBOX4_CODE].val
            );
            assert_eq!(bbox, r[10], "{} FontBBox", name);
            if o.dims[STEMV_CODE].set {
                assert_eq!(o.dims[STEMV_CODE].val.to_string(), r[11], "{} StemV", name);
            }
            assert_eq!(o.flags.to_string(), r[12], "{} Flags", name);
            let want_charset = if r[13] == "-" { "" } else { r[13].as_str() };
            assert_eq!(o.charset, want_charset, "{} CharSet", name);
            // The float arithmetic, in the clear text of the stream.
            let head = text(&o.out[..o.length.0 as usize]);
            let find = |key: &str| -> String {
                let at = head.find(key).expect(key) + key.len();
                head[at..].lines().next().unwrap().to_string()
            };
            assert_eq!(find("/FontMatrix "), r[14], "{} FontMatrix", name);
            assert_eq!(find("/ItalicAngle "), r[15], "{} ItalicAngle line", name);
            got_warn.extend(o.warnings.iter().map(|w| text(w)));
            assert_eq!(got_warn, want_warn, "{} warnings", name);
        }
    }
}

#[test]
fn synthetic_fonts_against_the_oracle() {
    let rows = table("fontfile_syn.tsv");
    assert!(rows.len() > 40);
    for r in &rows {
        check_syn_row(r, &case(&r[0]));
    }
}

#[test]
fn slant_and_extend_against_the_oracle() {
    let rows = table("fontfile_float.tsv");
    assert!(rows.len() > 20);
    let font = case("std");
    for r in &rows {
        check_syn_row(r, &font);
    }
}

#[test]
fn map_lines_against_the_oracle() {
    let rows = table("fontfile_map.tsv");
    assert!(rows.len() > 20);
    for r in &rows {
        let line = bytes(&r[0]);
        let want_warn = list(&r[3]);
        let item = {
            let mut with = b"=".to_vec();
            with.extend_from_slice(&line);
            with
        };
        let it = process_map_item(&item, MapType::MapLine);
        assert_eq!(it.mode, FmMode::Replace);
        let scan = fm_scan_line(it.item).unwrap();
        let got: Vec<String> = scan.warnings.iter().map(|w| text(w)).collect();
        let mut got_all = got.clone();
        if r[1] == "-" {
            // No font of this line was used, or it was bad.
            assert_eq!(got_all, want_warn, "{}", r[0]);
            continue;
        }
        let fm = scan.entry.expect("the oracle used it");
        let (flags, w) = font_descriptor_flags(
            fm.fd_flags,
            fm.ff_name.is_some(),
            fm.ps_name,
            fm.tfm_name,
            fm.is_std_t1font(),
        );
        got_all.extend(w.iter().map(|x| text(x)));
        if fm.is_builtin() && !fm.is_std_t1font() {
            got_all.push(format!(
                "font `{}' is not a standard font; I suppose it is available to your PDF viewer then",
                text(fm.ps_name.unwrap_or(b""))
            ));
        }
        assert_eq!(flags.to_string(), r[2], "{} flags", r[0]);
        let mut a = got_all.clone();
        let mut b = want_warn.clone();
        a.sort();
        b.sort();
        assert_eq!(a, b, "{}", r[0]);
        // The name, past any tag, says the slant and extension.
        let base = r[1].rsplit('+').next().unwrap();
        if fm.slant != 0 {
            assert!(base.contains(&format!("-Slant_{}", fm.slant)), "{} in {}", fm.slant, r[1]);
        }
        if fm.extend != 0 {
            assert!(base.contains(&format!("-Extend_{}", fm.extend)), "{} in {}", fm.extend, r[1]);
        }
    }
}

#[test]
fn the_installed_map_file_reads_without_surprises() {
    let path = match kpse("pdftex.map") {
        Some(p) => p,
        None => {
            eprintln!("skipped: no kpsewhich or no pdftex.map");
            return;
        }
    };
    let data = std::fs::read(path).unwrap();
    let mut pos = 0;
    let mut buf = [0u8; 1024];
    let mut n = 0;
    while let Some(line) = fm_read_line(&data, &mut pos, &mut buf).unwrap() {
        let scan = fm_scan_line(line).unwrap();
        if let Some(fm) = scan.entry {
            assert!(!fm.tfm_name.is_empty());
            n += 1;
        }
    }
    assert!(n > 100, "{} entries", n);
}

/// Reads a file through kpathsea if it is the one the table was made from.
fn matching(name: &str, digest: &str) -> Option<Vec<u8>> {
    let path = kpse(name)?;
    let data = std::fs::read(path).ok()?;
    if md5_hex(&data) == digest { Some(data) } else { None }
}

#[test]
fn tex_live_fonts_against_the_oracle() {
    if kpse("cmr10.tfm").is_none() {
        eprintln!("skipped: no TeX Live");
        return;
    }
    let (mut ran, mut skipped) = (0, 0);
    for r in table("fontfile_tex.tsv") {
        let label = format!("{} {}", r[1], r[2]);
        let font = match matching(&r[5], &r[6]) {
            Some(d) => d,
            None => {
                skipped += 1;
                continue;
            }
        };
        let names = if r[7] != "-" {
            match matching(&r[7], &r[8]) {
                Some(d) => Some(load_enc_file(&d).unwrap()),
                None => {
                    skipped += 1;
                    continue;
                }
            }
        } else {
            None
        };
        let line = bytes(&r[3]);
        let item = process_map_item(&line, MapType::MapLine);
        let scan = fm_scan_line(item.item).unwrap();
        let fm = scan.entry.expect("a good line");
        let codes = parse_codes(&r[4]);
        ran += 1;
        if fm.is_truetype() {
            let mut gl = GlyphSet::new();
            let mut tx = CodeSet::new();
            let mut marked = CodeSet::new();
            for &c in &codes {
                marked.insert(c);
            }
            let (first, last) = get_char_range(&marked, 0, 255);
            if let (Some(n), true) = (&names, fm.is_subsetted()) {
                mark_reenc_glyphs(n, &marked, first, last, &mut gl);
            }
            mark_chars(&mut tx, &marked, first, last);
            let mut dims = new_font_dims();
            let mut cmaps = TtfCmapCache::default();
            let mut out = Vec::new();
            let inp = TtfInput {
                font: &font,
                file_name: fm.ff_name.unwrap(),
                ff_name: fm.ff_name.unwrap(),
                tfm_name: b"cmr10",
                subsetted: fm.is_subsetted(),
                fontname: fm.ps_name.unwrap_or(b"cmr10"),
                fe_names: names.as_ref(),
                subfont: None,
                pid: fm.pid as i32,
                eid: fm.eid as i32,
                write_ttf_glyph_names: names.is_some(),
                used_tags: &[],
            };
            let o = write_ttf(&inp, &mut dims, &mut gl, &mut tx, &mut cmaps, &mut out).unwrap();
            assert_eq!(o.length.to_string(), r[9], "{}", label);
            assert_eq!(md5_hex(&out), r[12], "{} stream", label);
            continue;
        }
        if fm.is_opentype() {
            let mut dims = new_font_dims();
            let mut out = Vec::new();
            write_otf(&font, fm.is_subsetted(), &mut dims, &mut out).unwrap();
            assert_eq!(md5_hex(&out), r[12], "{} stream", label);
            continue;
        }
        let o = run_t1(&font, &fm, names.as_ref(), bytes(&r[1]).as_slice(), &codes).unwrap();
        assert_eq!(o.length.0.to_string(), r[9], "{} Length1", label);
        assert_eq!(o.length.1.to_string(), r[10], "{} Length2", label);
        assert_eq!(o.length.2.to_string(), r[11], "{} Length3", label);
        assert_eq!(md5_hex(&o.out), r[12], "{} stream", label);
        assert_eq!(o.name, r[13], "{} name", label);
        if o.dims[FONTBBOX1_CODE].set {
            let bbox = format!(
                "{} {} {} {}",
                o.dims[FONTBBOX1_CODE].val, o.dims[FONTBBOX2_CODE].val, o.dims[FONTBBOX3_CODE].val, o.dims[FONTBBOX4_CODE].val
            );
            assert_eq!(bbox, r[14], "{} FontBBox", label);
        }
        if o.dims[ITALIC_ANGLE_CODE].set {
            assert_eq!(o.dims[ITALIC_ANGLE_CODE].val.to_string(), r[15], "{} ItalicAngle", label);
        }
        assert_eq!(o.flags.to_string(), r[16], "{} Flags", label);
        assert_eq!(md5_hex(&bytes(&o.charset)), r[17], "{} CharSet", label);
    }
    eprintln!("TeX Live fonts: {} rows ran, {} skipped", ran, skipped);
    assert!(ran > 0 || skipped > 0);
}

#[test]
fn tounicode_against_the_oracle() {
    let path = match kpse("glyphtounicode.tex") {
        Some(p) => p,
        None => {
            eprintln!("skipped: no glyphtounicode.tex");
            return;
        }
    };
    let gtu = std::fs::read(path).unwrap();
    let mut map = GlyphUnicodeMap::new();
    for line in gtu.split(|&c| c == b'\n') {
        if let Some(rest) = line.strip_prefix(b"\\pdfglyphtounicode{") {
            let close = rest.iter().position(|&c| c == b'}').unwrap();
            let glyph = &rest[..close];
            let uni = &rest[close + 2..];
            let end = uni.iter().position(|&c| c == b'}').unwrap();
            assert!(map.deftounicode(glyph, &uni[..end]).unwrap().is_none());
        }
    }
    let mut ran = 0;
    for r in table("fontfile_uni.tsv") {
        if md5_hex(&gtu) != r[6] {
            continue;
        }
        let font = match matching(&r[2], &r[3]) {
            Some(d) => d,
            None => continue,
        };
        let line = bytes(&r[1]);
        let scan = fm_scan_line(&line).unwrap();
        let fm = scan.entry.expect("a good line");
        let mut out = Vec::new();
        let tfm = bytes(&r[0]);
        if r[4] != "-" {
            let enc = match matching(&r[4], &r[5]) {
                Some(d) => load_enc_file(&d).unwrap(),
                None => continue,
            };
            write_tounicode(&enc, &tfm, fm.encname, &mut map, &mut out).unwrap();
        } else {
            // the font's own encoding, from writet1
            let o = run_t1(&font, &fm, None, &tfm, &[65]).unwrap();
            let names = o.builtin.expect("subsetted");
            write_tounicode(&names, &tfm, None, &mut map, &mut out).unwrap();
        }
        assert_eq!(out.len().to_string(), r[7], "{}", r[0]);
        assert_eq!(md5_hex(&out), r[8], "{}", r[0]);
        ran += 1;
    }
    eprintln!("ToUnicode: {} rows ran", ran);
}

/// The C's collision loop, which real documents do not reach: the same glyphs, the same name, the tag taken.
#[test]
fn subset_tags_differ_when_taken() {
    let font = case("std");
    let mut used: Vec<[u8; 6]> = Vec::new();
    for _ in 0..3 {
        let mut dims = new_font_dims();
        let mut gl = GlyphSet::new();
        gl.insert(b"A");
        let mut carry = T1Carry::default();
        let mut out = Vec::new();
        let inp = T1Input {
            font: &font,
            subsetted: true,
            all_glyphs: false,
            slant: 0,
            extend: 0,
            fontname: b"Syn",
            tx_codes: None,
            used_tags: &used,
        };
        let o = write_t1(&inp, &mut dims, &mut gl, &mut carry, &mut out).unwrap();
        let t = o.subset_tag.unwrap();
        assert!(!used.contains(&t));
        used.push(t);
    }
}

/// `all_glyphs`, which only PDF inclusion sets: every glyph and subr stays.
#[test]
fn all_glyphs_keeps_everything() {
    let font = Syn { subrs: Some(hint_subrs()), glyphs: hint_glyphs(), ..syn() }.build();
    let mut dims = new_font_dims();
    let mut gl = GlyphSet::new();
    let mut carry = T1Carry::default();
    let mut out = Vec::new();
    let inp = T1Input {
        font: &font,
        subsetted: true,
        all_glyphs: true,
        slant: 0,
        extend: 0,
        fontname: b"Syn",
        tx_codes: None,
        used_tags: &[],
    };
    let o = write_t1(&inp, &mut dims, &mut gl, &mut carry, &mut out).unwrap();
    // Decrypt the private part again: glyph B and subr 6 are there though nothing asked for them.
    let plain = eexec_decrypt(&out[o.length1 as usize..(o.length1 + o.length2) as usize]);
    let s = text(&plain);
    assert!(s.contains("/B "));
    assert!(s.contains("/A "));
    assert!(s.contains("dup 6 "));
    assert!(s.contains("/Subrs 7 "), "the size of the array is subr_size");
}

fn eexec_decrypt(c: &[u8]) -> Vec<u8> {
    let mut key: u16 = 55665;
    let mut out = Vec::with_capacity(c.len());
    for &b in c {
        out.push(b ^ (key >> 8) as u8);
        key = (b as u32 + key as u32).wrapping_mul(52845).wrapping_add(22719) as u16;
    }
    out
}
