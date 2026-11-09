//! The gate's Rust half: replays what `writefont.c` does with this crate, for each job in a manifest
//! written by `tests/oracle/fontfile_gate.py`, and compares with what the oracle put in its PDF.
//!
//!     fontfile_gate MANIFEST [GLYPHTOUNICODE_PAIRS]
//!
//! One line per record, fields separated by a tab, `-` for nothing, byte strings in hex where they
//! could hold a tab:
//!
//!     job ID
//!     font TFM CODES ENCPATH FONTPATH SFDPATH KIND HEXMAPLINE    KIND: file or line, how the C reads it
//!     tounicode 0|1
//!     desc HEXFONTNAME FLAGS BBOX ITALIC ASCENT CAPHEIGHT DESCENT STEMV XHEIGHT LEN1 LEN2 LEN3 LENGTH STREAM HEXCHARSET
//!     enc HEXBODY
//!     unicode TFM HEXNAME STREAM
//!     warn HEX
//!     error HEX
//!     end
//!
//! A line of output per job: `PASS id ...` or `FAIL id what`.
use rstex_fontfile::*;
use std::collections::BTreeMap;
use std::fs;

fn unhex(s: &str) -> Vec<u8> {
    if s == "-" {
        return Vec::new();
    }
    let b = s.as_bytes();
    let mut v = Vec::with_capacity(b.len() / 2);
    for c in b.chunks(2) {
        let h = (c[0] as char).to_digit(16).unwrap() * 16 + (c[1] as char).to_digit(16).unwrap();
        v.push(h as u8);
    }
    v
}

fn show(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[derive(Default, Clone)]
struct FontUse {
    tfm: String,
    codes: Vec<usize>,
    enc: String,
    path: String,
    sfd: String,
    file_line: bool,
    line: Vec<u8>,
}

#[derive(Default, Clone)]
struct Desc {
    name: Vec<u8>,
    flags: String,
    bbox: String,
    italic: String,
    ascent: String,
    capheight: String,
    descent: String,
    stemv: String,
    xheight: String,
    len1: String,
    len2: String,
    len3: String,
    length: String,
    stream: String,
    charset: Vec<u8>,
}

#[derive(Default, Clone)]
struct Job {
    id: String,
    fonts: Vec<FontUse>,
    tounicode: bool,
    descs: Vec<Desc>,
    encs: Vec<Vec<u8>>,
    unicode: Vec<(String, Vec<u8>, String)>,
    warns: Vec<Vec<u8>>,
    errors: Vec<Vec<u8>>,
    crash: Option<String>,
}

/// `fd_entry`.
struct Fd {
    ff_name: Vec<u8>,
    slant: i64,
    extend: i64,
    subsetted: bool,
    fontname: Vec<u8>,
    gl: GlyphSet,
    tx: Option<CodeSet>,
    dims: FontDims,
    path: String,
    fm_flags: i32,
    tfm: String,
    is_std: bool,
    ps: Option<Vec<u8>>,
    builtin: Option<GlyphNames>,
}

/// `fo_entry`, what the font dictionaries need afterwards.
struct Fo {
    tfm: String,
    fd: Option<usize>,
    enc: Option<Vec<u8>>,
}

#[derive(Default)]
struct Tally {
    seac: usize,
    t1_fonts: usize,
    t1_in: usize,
    t1_out: usize,
    t1_time: std::time::Duration,
}

fn run(job: &Job, gtu: &mut GlyphUnicodeMap, gtu_warnings: &[Vec<u8>], tally: &mut Tally) -> Result<String, String> {
    if let Some(c) = &job.crash {
        return Ok(format!("SKIP the oracle crashed: {}", c));
    }
    let mut warnings: Vec<Vec<u8>> = Vec::new();
    if job.tounicode {
        warnings.extend_from_slice(gtu_warnings); // the document's \\pdfglyphtounicode
    }
    match replay(job, gtu, &mut warnings, tally) {
        Ok(()) => {}
        Err(e) => {
            if e.starts_with(b"MISMATCH") {
                return Err(show(&e));
            }
            // A fatal error in the port has to be the oracle's.
            if job.errors.is_empty() {
                return Err(format!("the port failed and the oracle did not: {}", show(&e)));
            }
            if !same_error(&e, &job.errors[0]) {
                return Err(format!("errors differ: `{}' against `{}'", show(&e), show(&job.errors[0])));
            }
            // An assertion aborts, and what was still in the log's buffer is lost: nothing to compare.
            if e.windows(10).any(|x| x == b"Assertion ") {
                return Ok("fatal error, the same one".to_string());
            }
            return compare_warnings(job, warnings).map(|_| "fatal error, the same one".to_string());
        }
    }
    if !job.errors.is_empty() {
        return Err(format!("the oracle failed and the port did not: {}", show(&job.errors[0])));
    }
    compare_warnings(job, warnings)?;
    Ok(String::new())
}

/// A message can quote a line with binary bytes, which the log shows in `^^` notation and as `?`: up to
/// there they must agree, and how the engine prints the rest is the engine's.
fn same_error(port: &[u8], oracle: &[u8]) -> bool {
    let n = port.iter().position(|&c| !(32..127).contains(&c)).unwrap_or(port.len());
    if n == port.len() {
        return port == oracle;
    }
    oracle.len() >= n && port[..n] == oracle[..n]
}

fn compare_warnings(job: &Job, mut got: Vec<Vec<u8>>) -> Result<(), String> {
    let mut want = job.warns.clone();
    want.sort();
    got.sort();
    if want != got {
        let a: Vec<String> = got.iter().map(|x| show(x)).collect();
        let b: Vec<String> = want.iter().map(|x| show(x)).collect();
        return Err(format!("warnings differ: port {:?}, oracle {:?}", a, b));
    }
    Ok(())
}

/// `pdftex_fail`: what the C has printed before it, then the message.
fn fatal(e: FontError, w: &mut Vec<Vec<u8>>) -> Vec<u8> {
    w.extend(e.warnings);
    e.message
}

fn replay(job: &Job, gtu: &mut GlyphUnicodeMap, w: &mut Vec<Vec<u8>>, tally: &mut Tally) -> Result<(), Vec<u8>> {
    let mut fds: Vec<Fd> = Vec::new();
    let mut fos: Vec<Fo> = Vec::new();
    let mut encs: BTreeMap<Vec<u8>, (GlyphNames, CodeSet)> = BTreeMap::new();
    let mut used_tags: Vec<[u8; 6]> = Vec::new();
    let mut carry = T1Carry::default();
    let mut cmaps = TtfCmapCache::default();
    let mut desc_index = 0;
    let mut problems: Vec<String> = Vec::new();

    // First the map lines, as \pdfmapline does while the document is read.
    let mut scanned = Vec::new();
    let mut linebuf = [0u8; 1024];
    for f in job.fonts.iter() {
        let owned: Vec<u8> = if f.file_line {
            let mut pos = 0;
            match fm_read_line(&f.line, &mut pos, &mut linebuf).map_err(|e| fatal(e, w))? {
                Some(l) => l.to_vec(),
                None => Vec::new(),
            }
        } else {
            process_map_item(&f.line, MapType::MapLine).item.to_vec()
        };
        scanned.push(owned);
    }
    let mut entries = Vec::new();
    for (n, s) in scanned.iter().enumerate() {
        let r = fm_scan_line(s).map_err(|e| fatal(e, w))?;
        // The document says each \\pdfmapline once.
        if !scanned[..n].contains(s) {
            w.extend(r.warnings.iter().cloned());
        }
        entries.push(r.entry);
    }
    // Then dopdffont for each font: head_tab[obj_type_font] is a list with the last used font first.
    for (n, f) in job.fonts.iter().enumerate().rev() {
        let fm = match &entries[n] {
            Some(fm) => *fm,
            None => continue,
        };
        let mut marked = CodeSet::new();
        for &c in &f.codes {
            marked.insert(c);
        }
        let (first, last) = get_char_range(&marked, 0, 255);
        if first > last {
            continue;
        }
        let subfont_parts_found = subfont_parts(fm.tfm_name);
        let mut subfont: Option<Subfont> = None;
        if let Some((prefix, _)) = subfont_parts_found {
            let data = fs::read(&f.sfd).map_err(|e| format!("{}: {}", f.sfd, e).into_bytes())?;
            let sfd = read_sfd(&data).map_err(|e| fatal(e, w))?;
            for sf in sfd.subfonts.iter() {
                let mut nm = prefix.to_vec();
                nm.extend_from_slice(&sf.infix);
                if nm == f.tfm.as_bytes() {
                    subfont = Some(sf.clone());
                }
            }
        }
        // create_fontdictionary
        let mut enc_key = None;
        if let Some(e) = fm.encname {
            if !encs.contains_key(e) {
                let data = fs::read(&f.enc).map_err(|x| format!("{}: {}", f.enc, x).into_bytes())?;
                let names = load_enc_file(&data).map_err(|e| fatal(e, w))?;
                encs.insert(e.to_vec(), (names, CodeSet::new()));
            }
            if fm.is_type1() || fm.is_opentype() {
                let ent = encs.get_mut(e).unwrap();
                mark_chars(&mut ent.1, &marked, first, last);
            }
            enc_key = Some(e.to_vec());
        }
        let ps = fm.ps_name.map(|p| p.to_vec());
        let preset = match fm.ps_name {
            Some(p) => p.to_vec(),
            None => f.tfm.as_bytes().to_vec(),
        };
        if fm.is_builtin() {
            // builtin fonts still get a /FontDescriptor, written at once, from the TFM only
            let (flags, wf) =
                font_descriptor_flags(fm.fd_flags, false, fm.ps_name, f.tfm.as_bytes(), fm.is_std_t1font());
            w.extend(wf);
            if !fm.is_std_t1font() {
                let mut m = b"font `".to_vec();
                m.extend_from_slice(fm.ps_name.unwrap_or(b""));
                m.extend_from_slice(b"' is not a standard font; I suppose it is available to your PDF viewer then");
                w.push(m);
            }
            let d = job.descs.get(desc_index).cloned().unwrap_or_default();
            desc_index += 1;
            if d.flags != flags.to_string() {
                problems.push(format!("builtin flags {} against {}", flags, d.flags));
            }
            if d.name != preset {
                problems.push(format!("builtin name {} against {}", show(&preset), show(&d.name)));
            }
            fos.push(Fo { tfm: f.tfm.clone(), fd: None, enc: enc_key });
            continue;
        }
        if fm.is_type1() {
            let ff = fm.ff_name.unwrap();
            let mut idx = fds.iter().position(|d| d.ff_name == ff && d.slant == fm.slant && d.extend == fm.extend);
            if idx.is_none() {
                fds.push(Fd {
                    ff_name: ff.to_vec(),
                    slant: fm.slant,
                    extend: fm.extend,
                    subsetted: fm.is_subsetted(),
                    fontname: preset.clone(),
                    gl: GlyphSet::new(),
                    tx: None,
                    dims: new_font_dims(),
                    path: f.path.clone(),
                    fm_flags: fm.fd_flags,
                    tfm: f.tfm.clone(),
                    is_std: fm.is_std_t1font(),
                    ps: ps.clone(),
                    builtin: None,
                });
                idx = Some(fds.len() - 1);
            }
            let idx = idx.unwrap();
            if let Some(e) = fm.encname {
                if fm.is_subsetted() {
                    let names = &encs.get(e).unwrap().0;
                    mark_reenc_glyphs(names, &marked, first, last, &mut fds[idx].gl);
                }
            } else {
                let tx = fds[idx].tx.get_or_insert_with(CodeSet::new);
                mark_chars(tx, &marked, first, last);
            }
            fos.push(Fo { tfm: f.tfm.clone(), fd: Some(idx), enc: enc_key });
            continue;
        }
        // TrueType and OpenType: one descriptor each, written at once.
        let mut gl = GlyphSet::new();
        let mut tx = CodeSet::new();
        let names = fm.encname.map(|e| encs.get(e).unwrap().0.clone());
        if let Some(n) = &names {
            if fm.is_subsetted() {
                mark_reenc_glyphs(n, &marked, first, last, &mut gl);
            }
            if fm.is_truetype() || fm.is_opentype() {
                mark_chars(&mut tx, &marked, first, last);
            }
        } else {
            mark_chars(&mut tx, &marked, first, last);
        }
        let data = fs::read(&f.path).map_err(|e| format!("{}: {}", f.path, e).into_bytes())?;
        let mut dims = new_font_dims();
        let mut out = Vec::new();
        let (mut pid, mut eid) = (fm.pid as i32, fm.eid as i32);
        if subfont.is_some() && pid == -1 {
            pid = 3;
            eid = 1;
        }
        let name: Vec<u8>;
        let tag: Option<[u8; 6]>;
        let length: i32;
        if fm.is_truetype() {
            let inp = TtfInput {
                font: &data,
                file_name: f.path.as_bytes(),
                ff_name: fm.ff_name.unwrap(),
                tfm_name: f.tfm.as_bytes(),
                subsetted: fm.is_subsetted(),
                fontname: &preset,
                fe_names: names.as_ref(),
                subfont: subfont.as_ref().map(|s| &*s.charcodes),
                pid,
                eid,
                write_ttf_glyph_names: names.is_some(),
                used_tags: &used_tags,
            };
            let o = write_ttf(&inp, &mut dims, &mut gl, &mut tx, &mut cmaps, &mut out).map_err(|e| fatal(e, w))?;
            w.extend(o.warnings);
            name = o.fontname;
            tag = o.subset_tag;
            length = o.length;
        } else {
            let ow = write_otf(&data, fm.is_subsetted(), &mut dims, &mut out).map_err(|e| fatal(e, w))?;
            w.extend(ow);
            name = preset.clone();
            tag = None;
            length = -1;
        }
        if let Some(t) = tag {
            used_tags.push(t);
        }
        let d = job.descs.get(desc_index).cloned().unwrap_or_default();
        desc_index += 1;
        let len1 = if fm.is_truetype() { length.to_string() } else { "-".to_string() };
        compare_desc(&d, &mut problems, &name, tag, &dims, (len1, "-".into(), "-".into()), &out, None);
        let (flags, _) = font_descriptor_flags(fm.fd_flags, true, fm.ps_name, f.tfm.as_bytes(), fm.is_std_t1font());
        if d.flags != flags.to_string() {
            problems.push(format!("flags {} against {}", flags, d.flags));
        }
        fos.push(Fo { tfm: f.tfm.clone(), fd: None, enc: enc_key });
    }
    // write_fontdescriptors: the tree of Type 1 descriptors, by (ff_name, slant, extend)
    let mut order: Vec<usize> = (0..fds.len()).collect();
    order.sort_by(|&a, &b| {
        (&fds[a].ff_name, fds[a].slant, fds[a].extend).cmp(&(&fds[b].ff_name, fds[b].slant, fds[b].extend))
    });
    for &i in &order {
        let data = fs::read(&fds[i].path).map_err(|e| format!("{}: {}", fds[i].path, e).into_bytes())?;
        let fd = &mut fds[i];
        let before = fd.gl.len();
        let mut out = Vec::new();
        let preset = fd.fontname.clone();
        let inp = T1Input {
            font: &data,
            subsetted: fd.subsetted,
            all_glyphs: false,
            slant: fd.slant,
            extend: fd.extend,
            fontname: &preset,
            tx_codes: fd.tx.as_ref(),
            used_tags: &used_tags,
            decrypt: decrypt_run,
        };
        let start = std::time::Instant::now();
        let o = write_t1(&inp, &mut fd.dims, &mut fd.gl, &mut carry, &mut out).map_err(|e| fatal(e, w))?;
        tally.t1_time += start.elapsed();
        tally.t1_fonts += 1;
        tally.t1_in += data.len();
        tally.t1_out += out.len();
        w.extend(o.warnings);
        if let Some(t) = o.subset_tag {
            used_tags.push(t);
        }
        if fd.gl.len() > before + fd.tx.map_or(0, |t| (0..256).filter(|&c| t.contains(c)).count()) {
            tally.seac += 1;
        }
        fd.fontname = o.fontname.clone();
        fd.builtin = o.builtin_glyph_names;
        let d = job.descs.get(desc_index).cloned().unwrap_or_default();
        desc_index += 1;
        let charset: Option<Vec<u8>> = if fd.subsetted {
            let mut c = Vec::new();
            for g in fd.gl.iter() {
                c.push(b'/');
                c.extend_from_slice(g);
            }
            Some(c)
        } else {
            None
        };
        compare_desc(
            &d,
            &mut problems,
            &o.fontname,
            o.subset_tag,
            &fd.dims,
            (o.length1.to_string(), o.length2.to_string(), o.length3.to_string()),
            &out,
            charset.as_deref(),
        );
        let (flags, wf) =
            font_descriptor_flags(fd.fm_flags, true, fd.ps.as_deref(), fd.tfm.as_bytes(), fd.is_std);
        w.extend(wf);
        if d.flags != flags.to_string() {
            problems.push(format!("flags {} against {}", flags, d.flags));
        }
    }
    if desc_index != job.descs.len() {
        problems.push(format!("{} descriptors against {}", desc_index, job.descs.len()));
    }
    // write_fontencodings
    let mut got = Vec::new();
    for (names, tx) in encs.values() {
        if !tx.is_empty() {
            let mut b = Vec::new();
            write_enc_differences(names, tx, &mut b);
            got.push(b);
        }
    }
    let mut want = job.encs.clone();
    want.sort();
    got.sort();
    if want != got {
        problems.push(format!(
            "encodings differ: {:?} against {:?}",
            got.iter().map(|x| show(x)).collect::<Vec<_>>(),
            want.iter().map(|x| show(x)).collect::<Vec<_>>()
        ));
    }
    // write_fontdictionaries, for /ToUnicode.  The test in write_fontdictionary has a misplaced
    // parenthesis, `(... && strcmp(tfm_name, "dummy-space")) == 0`, so that font always gets there, and
    // with no \pdfglyphtounicode write_tounicode gives this warning.
    if !job.tounicode && fos.iter().any(|f| f.tfm == "dummy-space") {
        w.push(b"no GlyphToUnicode entry has been inserted yet!".to_vec());
    }
    if job.tounicode {
        for fo in fos.iter() {
            let tfm = &fo.tfm;
            let mut out = Vec::new();
            let r = match &fo.enc {
                Some(e) => {
                    let n = &encs.get(e).unwrap().0;
                    write_tounicode(n, tfm.as_bytes(), Some(e), gtu, rstex_fontfile::SealedGlyphMap::default(), &mut out)
                }
                None => match fo.fd.and_then(|i| fds[i].builtin.as_ref()) {
                    Some(n) => write_tounicode(n, tfm.as_bytes(), None, gtu, rstex_fontfile::SealedGlyphMap::default(), &mut out),
                    None => continue,
                },
            };
            let r = r.map_err(|e| fatal(e, w))?;
            w.extend(r);
            match job.unicode.iter().find(|u| &u.0 == tfm) {
                Some((_, _, stream)) => {
                    let expected = fs::read(stream).map_err(|e| format!("{}: {}", stream, e).into_bytes())?;
                    if out != expected {
                        problems.push(format!("ToUnicode of {} differs at {}", tfm, first_diff(&out, &expected)));
                    }
                }
                None => problems.push(format!("the oracle wrote no ToUnicode for {}", tfm)),
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!("MISMATCH {}", problems.join("; ")).into_bytes())
    }
}

fn first_diff(a: &[u8], b: &[u8]) -> String {
    for i in 0..a.len().min(b.len()) {
        if a[i] != b[i] {
            return format!("byte {} ({:#x} against {:#x}), lengths {} and {}", i, a[i], b[i], a.len(), b.len());
        }
    }
    format!("the end, lengths {} and {}", a.len(), b.len())
}

#[allow(clippy::too_many_arguments)]
fn compare_desc(
    d: &Desc,
    p: &mut Vec<String>,
    name: &[u8],
    tag: Option<[u8; 6]>,
    dims: &FontDims,
    lens: (String, String, String),
    out: &[u8],
    charset: Option<&[u8]>,
) {
    let mut full = Vec::new();
    if let Some(t) = tag {
        full.extend_from_slice(&t);
        full.push(b'+');
    }
    full.extend_from_slice(name);
    if full != d.name {
        p.push(format!("FontName {} against {}", show(&full), show(&d.name)));
    }
    if d.stream != "-" {
        match fs::read(&d.stream) {
            Ok(e) => {
                if e != out {
                    p.push(format!("font bytes differ at {}", first_diff(out, &e)));
                    if std::env::var_os("FONTFILE_GATE_DUMP").is_some() {
                        let _ = fs::write(format!("{}.port", d.stream), out);
                    }
                }
            }
            Err(x) => p.push(format!("{}: {}", d.stream, x)),
        }
    } else {
        p.push("the oracle wrote no font stream".to_string());
    }
    if d.len1 != lens.0 {
        p.push(format!("Length1 {} against {}", lens.0, d.len1));
    }
    if d.len2 != lens.1 {
        p.push(format!("Length2 {} against {}", lens.1, d.len2));
    }
    if d.len3 != lens.2 {
        p.push(format!("Length3 {} against {}", lens.2, d.len3));
    }
    if d.length != out.len().to_string() {
        p.push(format!("Length {} against {}", out.len(), d.length));
    }
    // What the font itself sets; the rest of /FontDescriptor comes from the TFM, in the engine.
    if dims[FONTBBOX1_CODE].set {
        let b = format!(
            "{} {} {} {}",
            dims[FONTBBOX1_CODE].val, dims[FONTBBOX2_CODE].val, dims[FONTBBOX3_CODE].val, dims[FONTBBOX4_CODE].val
        );
        if b != d.bbox {
            p.push(format!("FontBBox {} against {}", b, d.bbox));
        }
    }
    let one = |p: &mut Vec<String>, k: usize, want: &str, what: &str| {
        if dims[k].set && dims[k].val.to_string() != want {
            p.push(format!("{} {} against {}", what, dims[k].val, want));
        }
    };
    one(p, ITALIC_ANGLE_CODE, &d.italic, "ItalicAngle");
    one(p, ASCENT_CODE, &d.ascent, "Ascent");
    one(p, CAPHEIGHT_CODE, &d.capheight, "CapHeight");
    one(p, DESCENT_CODE, &d.descent, "Descent");
    one(p, STEMV_CODE, &d.stemv, "StemV");
    one(p, XHEIGHT_CODE, &d.xheight, "XHeight");
    if let Some(c) = charset {
        if c != d.charset {
            p.push(format!("CharSet {} against {}", show(c), show(&d.charset)));
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let manifest = fs::read(&args[1]).expect("the manifest");
    let mut gtu = GlyphUnicodeMap::new();
    let mut gtu_warnings = Vec::new();
    if let Some(p) = args.get(2) {
        let text = fs::read(p).expect("the pairs");
        for line in text.split(|&c| c == b'\n') {
            if let Some(tab) = line.iter().position(|&c| c == b'\t') {
                if let Some(w) = gtu.deftounicode(&line[..tab], &line[tab + 1..]).expect("short names") {
                    gtu_warnings.push(w);
                }
            }
        }
    }
    let mut job = Job::default();
    let (mut pass, mut fail) = (0, 0);
    let mut tally = Tally::default();
    for line in manifest.split(|&c| c == b'\n') {
        let text = String::from_utf8_lossy(line).into_owned();
        let f: Vec<&str> = text.split('\t').collect();
        match f[0] {
            "job" => {
                job = Job::default();
                job.id = f[1].to_string();
            }
            "font" => {
                job.fonts.push(FontUse {
                    tfm: f[1].to_string(),
                    codes: if f[2] == "-" {
                        Vec::new()
                    } else {
                        f[2].split(',').map(|x| x.parse().unwrap()).collect()
                    },
                    enc: f[3].to_string(),
                    path: f[4].to_string(),
                    sfd: f[5].to_string(),
                    file_line: f[6] == "file",
                    line: unhex(f[7]),
                });
            }
            "tounicode" => job.tounicode = f[1] == "1",
            "desc" => job.descs.push(Desc {
                name: unhex(f[1]),
                flags: f[2].to_string(),
                bbox: f[3].to_string(),
                italic: f[4].to_string(),
                ascent: f[5].to_string(),
                capheight: f[6].to_string(),
                descent: f[7].to_string(),
                stemv: f[8].to_string(),
                xheight: f[9].to_string(),
                len1: f[10].to_string(),
                len2: f[11].to_string(),
                len3: f[12].to_string(),
                length: f[13].to_string(),
                stream: f[14].to_string(),
                charset: unhex(f[15]),
            }),
            "enc" => job.encs.push(unhex(f[1])),
            "unicode" => job.unicode.push((f[1].to_string(), unhex(f[2]), f[3].to_string())),
            "warn" => job.warns.push(unhex(f[1])),
            "error" => job.errors.push(unhex(f[1])),
            "crash" => job.crash = Some(show(&unhex(f[1]))),
            "end" => match run(&job, &mut gtu, &gtu_warnings, &mut tally) {
                Ok(note) => {
                    pass += 1;
                    println!("PASS {} {}", job.id, note);
                }
                Err(e) => {
                    fail += 1;
                    println!("FAIL {} {}", job.id, e);
                }
            },
            _ => {}
        }
    }
    println!(
        "TOTAL pass {} fail {} seac-closures {} write_t1: {} fonts, {} MB in, {} MB out, {:.3} s",
        pass,
        fail,
        tally.seac,
        tally.t1_fonts,
        tally.t1_in / 1_000_000,
        tally.t1_out / 1_000_000,
        tally.t1_time.as_secs_f64()
    );
}
