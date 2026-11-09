//! A font whose `CarryUse` says it did not read what earlier fonts left gives the same bytes whatever they left, and `T1Carry::apply` leaves what a run leaves.
//! That is what lets a cached result stand for a run.  On the fonts of the installed tree, with several glyph sets, each after each other font of a small set.
use rstex_fontfile::{CodeSet, GlyphSet, T1Carry, T1Input, decrypt_run, new_font_dims, write_t1};

fn fonts() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(o) = std::process::Command::new("kpsewhich").arg("cmr10.pfb").output() else { return out };
    let p = String::from_utf8_lossy(&o.stdout).trim().to_string();
    if p.is_empty() {
        return out;
    }
    let cm = std::path::Path::new(&p).parent().expect("in a directory").to_path_buf();
    let type1 = cm.parent().and_then(|p| p.parent()).and_then(|p| p.parent()).expect("type1/public/amsfonts/cm").to_path_buf();
    fn walk(d: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        let mut es: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        es.sort();
        for e in es {
            if e.is_dir() {
                walk(&e, out);
            } else if e.extension().is_some_and(|x| x == "pfb") {
                out.push(e);
            }
        }
    }
    walk(&type1, &mut out);
    out
}

fn run(font: &[u8], codes: &CodeSet, carry: &mut T1Carry, name: &[u8]) -> Result<(Vec<u8>, rstex_fontfile::T1Output), ()> {
    let mut dims = new_font_dims();
    let mut gl = GlyphSet::new();
    let inp = T1Input { font, subsetted: true, all_glyphs: false, slant: 0, extend: 0, fontname: name, tx_codes: Some(codes), used_tags: &[], decrypt: decrypt_run };
    let mut out = Vec::new();
    let o = write_t1(&inp, &mut dims, &mut gl, carry, &mut out).map_err(|_| ())?;
    Ok((out, o))
}

#[test]
fn a_reusable_font_does_not_depend_on_the_carry() {
    let all = fonts();
    if all.is_empty() {
        eprintln!("no tree: nothing to test");
        return;
    }
    let mut x: u64 = 0x9E3779B97F4A7C15;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    // Carries left by some fonts, each from a glyph set of its own.
    let mut carries = vec![T1Carry::default()];
    let step = (all.len() / 40).max(1);
    for f in all.iter().step_by(step) {
        let Ok(b) = std::fs::read(f) else { continue };
        let mut c = T1Carry::default();
        let mut codes = CodeSet::new();
        for _ in 0..40 {
            codes.insert((next() % 256) as usize);
        }
        if run(&b, &codes, &mut c, b"X").is_ok() {
            carries.push(c);
        }
    }
    let (mut tested, mut reusable, mut with_input, mut failed) = (0u64, 0u64, 0u64, 0u64);
    for (i, f) in all.iter().enumerate() {
        if i % 3 != 0 {
            continue;
        }
        let Ok(b) = std::fs::read(f) else { continue };
        let mut codes = CodeSet::new();
        for _ in 0..(5 + next() % 120) {
            codes.insert((next() % 256) as usize);
        }
        let mut c0 = T1Carry::default();
        let Ok((out0, o0)) = run(&b, &codes, &mut c0, b"X") else {
            failed += 1;
            continue;
        };
        tested += 1;
        if !o0.carry_use.reusable || !o0.warnings.is_empty() {
            continue;
        }
        reusable += 1;
        if o0.carry_use.last_arg_in.is_some() {
            with_input += 1;
        }
        for start in carries.iter().step_by(3) {
            if !start.fits(&o0.carry_use) {
                continue;
            }
            let mut c1 = start.clone();
            let (out1, o1) = run(&b, &codes, &mut c1, b"X").expect("it passed from another start");
            assert_eq!(out0, out1, "{}", f.display());
            assert_eq!(o0.length1, o1.length1);
            assert_eq!(o0.subset_tag, o1.subset_tag);
            let mut applied = start.clone();
            applied.apply(&o0.carry_use);
            assert_eq!(applied, c1, "{}", f.display());
        }
    }
    eprintln!("{} fonts in the tree, {tested} written, {reusable} reusable, {with_input} of those read last_arg, {failed} failed alone", all.len());
    assert!(reusable * 10 >= tested * 8, "{reusable} of {tested}");
}
