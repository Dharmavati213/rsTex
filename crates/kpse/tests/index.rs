//! The index of `src/index.rs` through its whole life, on a synthetic tree.  One test, since it sets the process's environment; its cache
//! directory is its own.  What a lookup answers is compared with real kpsewhich by `tests/oracle/kpse_compare.py`, with and without the index.
use rstex_kpse::names::{DOSISH, ENV_SEP};
use rstex_kpse::*;
use std::fs;
use std::path::{Path, PathBuf};

fn put(root: &Path, rel: &str, data: &[u8]) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, data).unwrap();
}

/// A name as kpathsea gives it back: `/` throughout and, on Windows, the drive letter in lower case.
fn b(p: &Path) -> Vec<u8> {
    let mut v = p.to_str().unwrap().replace('\\', "/").into_bytes();
    names::normalize_path(&mut v);
    v
}

fn text(p: &Path) -> String {
    String::from_utf8(b(p)).unwrap()
}

/// Sets the environment variable that decides where the cache directory is, so that it is under `cache`, and says where: DESIGN.md section 12.
fn cache_goes_under(cache: &Path) -> PathBuf {
    let (var, dir) = if DOSISH {
        ("LOCALAPPDATA", cache.join("rstex").join("cache"))
    } else if cfg!(target_os = "macos") {
        ("HOME", cache.join("Library/Caches/rstex"))
    } else {
        ("XDG_CACHE_HOME", cache.join("rstex"))
    };
    // SAFETY: this binary has this one test, and nothing else runs yet.
    unsafe {
        std::env::set_var(var, cache);
    }
    dir
}

fn kpse() -> Kpse {
    let mut tree = Tree::from_selfautoloc(b"/nonexistent-bin/x");
    tree.builtin_cnf = None;
    let mut k = Kpse::new(Config {
        program_name: b"testprog".to_vec(),
        invocation_name: b"test".to_vec(),
        engine: Some(b"pdftex".to_vec()),
        process_env: true,
        vars: Vec::new(),
        tree: Some(tree),
        db_casefold: false,
    });
    k.set_index_racy_window(0);
    k
}

fn indexes(cache_dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(cache_dir) else { return Vec::new() };
    rd.flatten().map(|e| e.path()).filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("kpse-index-")).collect()
}

fn found(k: &mut Kpse, name: &[u8]) -> Option<String> {
    k.find_file(name, KPSE_TEX_FORMAT, false).map(|f| String::from_utf8_lossy(&f).to_string())
}

/// Time for a rewritten file to get another stamp, on a file system with a coarse clock.
fn tick() {
    std::thread::sleep(std::time::Duration::from_millis(30));
}

#[test]
fn an_index_is_made_used_and_thrown_away() {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join("kpse-index");
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    let cache = d.join("cache");
    let tree = d.join("tree");
    let other = d.join("other");
    let sep = char::from(ENV_SEP);
    let cnf = format!("TEXMFDBS = {t}{sep}$EXTRA_DB{sep}{o}\nTEXINPUTS = !!{t}/tex//{sep}!!{o}/tex//{sep}!!$EXTRA_DB/tex//\nTEXMF = {t}\nA_VAR = hello\nA_VAR.testprog = hello program\n", t = text(&tree), o = text(&other));
    put(&d, "cnf/texmf.cnf", cnf.as_bytes());
    put(&tree, "ls-R", b"./tex/a:\none.tex\n");
    put(&tree, "tex/a/one.tex", b"");
    fs::create_dir_all(&other).unwrap();
    let cache = cache_goes_under(&cache);
    // SAFETY: as there.
    unsafe {
        std::env::set_var("TEXMFCNF", d.join("cnf"));
        std::env::remove_var("EXTRA_DB");
    }
    let one = String::from_utf8_lossy(&b(&tree.join("tex/a/one.tex"))).to_string();

    // A first run makes it, and answers as a later one will.
    let mut k = kpse();
    assert_eq!(k.var_value(b"A_VAR").as_deref(), Some(&b"hello program"[..]));
    assert!(!k.index_was_used());
    assert_eq!(found(&mut k, b"one.tex").as_deref(), Some(&one[..]));
    assert_eq!(found(&mut k, b"none.tex"), None);
    assert_eq!(indexes(&cache).len(), 1, "one index");
    let path = indexes(&cache).pop().unwrap();

    // A second finds it and reads nothing else.
    let mut k = kpse();
    assert_eq!(k.var_value(b"A_VAR").as_deref(), Some(&b"hello program"[..]));
    assert!(k.index_was_used());
    assert_eq!(found(&mut k, b"one.tex").as_deref(), Some(&one[..]));
    assert_eq!(found(&mut k, b"none.tex"), None);
    let recorded = k.take_recorded();
    assert_eq!(recorded.len(), 1, "texmf.cnf is reported as read: {recorded:?}");
    assert!(k.format_path(KPSE_DB_FORMAT).unwrap().ends_with(&b(&other)));
    // A search path comes from the file, and is what it would have been.
    let inputs = b(&tree.join("tex")).into_iter().chain(b"//".iter().copied()).collect::<Vec<u8>>();
    assert!(k.format_path(KPSE_TEX_FORMAT).unwrap().windows(inputs.len()).any(|w| w == &inputs[..]));
    // And a variable that goes into one is another setting, with the new path seen at once, and a file of its own: runs in turn under the two make nothing over, as a plain run and one under `--until-stable` with an output directory have it.
    let setting = |value: Option<&Path>| {
        // SAFETY: as above.
        unsafe {
            match value {
                Some(v) => std::env::set_var("TEXINPUTS", text(v)),
                None => std::env::remove_var("TEXINPUTS"),
            }
        }
    };
    let elsewhere = d.join("elsewhere");
    setting(Some(&elsewhere));
    let mut e = kpse();
    assert_eq!(e.format_path(KPSE_TEX_FORMAT).unwrap(), b(&elsewhere));
    assert!(!e.index_was_used());
    assert_eq!(indexes(&cache).len(), 2);
    let stamps = |cache: &Path| {
        let mut v: Vec<_> = indexes(cache).iter().map(|p| (p.clone(), fs::metadata(p).unwrap().modified().unwrap(), fs::metadata(p).unwrap().len())).collect();
        v.sort();
        v
    };
    let before = stamps(&cache);
    for _ in 0..2 {
        setting(None);
        let mut e = kpse();
        assert!(e.format_path(KPSE_TEX_FORMAT).unwrap().windows(inputs.len()).any(|w| w == &inputs[..]));
        assert!(e.index_was_used(), "the first setting");
        setting(Some(&elsewhere));
        let mut e = kpse();
        assert_eq!(e.format_path(KPSE_TEX_FORMAT).unwrap(), b(&elsewhere));
        assert!(e.index_was_used(), "the other setting");
    }
    assert_eq!(stamps(&cache), before, "nothing made over");
    // A third has a third, and goes by its own answers, with a name that is not any of theirs.
    let third = d.join("third");
    setting(Some(&third));
    let mut e = kpse();
    assert_eq!(e.format_path(KPSE_TEX_FORMAT).unwrap(), b(&third));
    assert!(!e.index_was_used());
    let mut e = kpse();
    assert_eq!(e.format_path(KPSE_TEX_FORMAT).unwrap(), b(&third));
    assert!(e.index_was_used());
    assert_eq!(indexes(&cache).len(), 3);
    fs::remove_file(indexes(&cache).into_iter().find(|p| !before.iter().any(|b| b.0 == *p)).unwrap()).unwrap();
    setting(None);
    assert_eq!(indexes(&cache).len(), 2);
    // Under another program name, and the same file for the cnf table, what it holds for this one does not count.
    let mut e = kpse();
    assert!(e.var_value(b"A_VAR").is_some());
    assert!(e.index_was_used());
    e.set_program_name(b"anotherprog");
    let another = e.format_path(KPSE_PROGRAM_TEXT_FORMAT).unwrap();
    assert!(another.windows(11).any(|w| w == b"anotherprog"), "{}", String::from_utf8_lossy(&another));

    // Another program name is another index, since `VAR.prog` can differ.
    let mut p = kpse();
    p.set_program_name(b"anotherprog");
    assert_eq!(p.var_value(b"A_VAR").as_deref(), Some(&b"hello"[..]));
    assert!(!p.index_was_used());
    assert_eq!(indexes(&cache).len(), 3);

    // A changed ls-R makes it stale, and the next one knows the new name.
    tick();
    put(&tree, "ls-R", b"./tex/a:\none.tex\ntwo.tex\n");
    put(&tree, "tex/a/two.tex", b"");
    let mut k = kpse();
    assert!(found(&mut k, b"two.tex").is_some());
    assert!(!k.index_was_used());
    let mut k = kpse();
    assert!(found(&mut k, b"two.tex").is_some());
    assert!(k.index_was_used());

    // So does an ls-R where there was none: a miss is remembered too.
    tick();
    put(&other, "ls-R", b"./tex:\nthree.tex\n");
    put(&other, "tex/three.tex", b"");
    let mut k = kpse();
    assert!(found(&mut k, b"three.tex").is_some());
    assert!(!k.index_was_used());

    // So does a changed texmf.cnf.
    tick();
    let cnf2 = cnf.replace("A_VAR = hello\n", "A_VAR = hello again\nA_NEW = yes\n");
    put(&d, "cnf/texmf.cnf", cnf2.as_bytes());
    let mut k = kpse();
    assert_eq!(k.var_value(b"A_NEW").as_deref(), Some(&b"yes"[..]));
    assert!(!k.index_was_used());

    // A variable the search paths went through is another setting too, and it finds the database that its value names.
    let extra = d.join("extra");
    put(&extra, "ls-R", b"./tex:\nfour.tex\n");
    put(&extra, "tex/four.tex", b"");
    let mut k = kpse();
    assert!(k.var_value(b"A_NEW").is_some());
    assert!(k.index_was_used());
    // SAFETY: as above.
    unsafe {
        std::env::set_var("EXTRA_DB", text(&extra));
    }
    let mut k = kpse();
    assert!(k.var_value(b"A_NEW").is_some());
    assert!(!k.index_was_used());
    assert_eq!(k.format_path(KPSE_DB_FORMAT).unwrap().windows(5).filter(|w| w == b"extra").count(), 1);
    assert!(found(&mut k, b"four.tex").is_some());
    // SAFETY: as above.
    unsafe {
        std::env::remove_var("EXTRA_DB");
    }
    let mut k = kpse();
    assert!(k.var_value(b"A_NEW").is_some());
    assert!(k.index_was_used());
    assert_eq!(found(&mut k, b"four.tex"), None);
    // That its stamps have moved makes a setting's own file stale, and it is made again under the same name.
    // SAFETY: as above.
    unsafe {
        std::env::set_var("EXTRA_DB", &extra);
    }
    let mut k = kpse();
    assert!(found(&mut k, b"four.tex").is_some());
    assert!(k.index_was_used());
    tick();
    put(&extra, "ls-R", b"./tex:\nfive.tex\n");
    put(&extra, "tex/five.tex", b"");
    let count = indexes(&cache).len();
    let mut k = kpse();
    assert!(found(&mut k, b"five.tex").is_some());
    assert!(!k.index_was_used());
    assert_eq!(indexes(&cache).len(), count);
    let mut k = kpse();
    assert!(found(&mut k, b"five.tex").is_some());
    assert!(k.index_was_used());
    // A broken file of a setting goes, and only that one.
    let own = {
        let mut v = indexes(&cache);
        v.sort_by_key(|p| fs::metadata(p).unwrap().modified().unwrap());
        v.pop().unwrap()
    };
    let whole = fs::read(&own).unwrap();
    fs::write(&own, &whole[..whole.len() / 2]).unwrap();
    let mut k = kpse();
    assert!(found(&mut k, b"five.tex").is_some());
    assert!(!k.index_was_used());
    assert_eq!(fs::read(&own).unwrap(), whole, "made again in its place");
    // SAFETY: as above.
    unsafe {
        std::env::remove_var("EXTRA_DB");
    }

    // A broken file is never believed, whichever part breaks and however.  The answers stay right, and the file gets replaced.
    let good = fs::read(&path).unwrap();
    assert!(good.len() > 4096);
    let breaks: Vec<(&str, Vec<u8>)> = vec![
        ("empty", Vec::new()),
        ("cut", good[..good.len() / 2].to_vec()),
        ("magic", {
            let mut g = good.clone();
            g[0] ^= 1;
            g
        }),
        ("front", {
            let mut g = good.clone();
            g[100] ^= 1;
            g
        }),
        ("garbage", vec![0x55; good.len()]),
    ];
    for (what, bad) in breaks {
        fs::write(&path, &bad).unwrap();
        let mut k = kpse();
        assert_eq!(found(&mut k, b"two.tex").is_some(), true, "{what}");
        assert_eq!(found(&mut k, b"none.tex"), None, "{what}");
        assert!(!k.index_was_used(), "{what}");
        assert_eq!(fs::read(&path).unwrap(), good, "{what}: made again");
    }
    // In the databases themselves it shows only at a lookup that touches the page, which then asks again.
    let flip_from = good.len() - 1;
    let mut g = good.clone();
    g[flip_from] ^= 1;
    fs::write(&path, &g).unwrap();
    let mut k = kpse();
    assert!(k.var_value(b"A_VAR").is_some());
    assert!(k.index_was_used());
    // Each of the names in the last page: whichever of them holds the flipped byte is compared, and fails.
    assert!(found(&mut k, b"one.tex").is_some());
    assert!(found(&mut k, b"two.tex").is_some());
    assert!(found(&mut k, b"three.tex").is_some());
    assert_eq!(fs::read(&path).unwrap(), good, "made again after the failed sum");
}
