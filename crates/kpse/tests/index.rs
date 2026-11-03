//! The index of `src/index.rs` through its whole life, on a synthetic tree.  One test, since it sets the process's environment; its cache
//! directory is its own.  What a lookup answers is compared with real kpsewhich by `tests/oracle/kpse_compare.py`, with and without the index.
use rstex_kpse::*;
use std::fs;
use std::path::{Path, PathBuf};

fn put(root: &Path, rel: &str, data: &[u8]) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, data).unwrap();
}

fn b(p: &Path) -> Vec<u8> {
    p.to_str().unwrap().replace('\\', "/").into_bytes()
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

fn indexes(cache: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(cache.join("rstex")) else { return Vec::new() };
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
    let cnf = format!("TEXMFDBS = {t}:$EXTRA_DB:{o}\nTEXINPUTS = !!{t}/tex//:!!{o}/tex//\nTEXMF = {t}\nA_VAR = hello\nA_VAR.testprog = hello program\n", t = tree.display(), o = other.display());
    put(&d, "cnf/texmf.cnf", cnf.as_bytes());
    put(&tree, "ls-R", b"./tex/a:\none.tex\n");
    put(&tree, "tex/a/one.tex", b"");
    fs::create_dir_all(&other).unwrap();
    // SAFETY: this binary has this one test, and nothing else runs yet.
    unsafe {
        std::env::set_var("XDG_CACHE_HOME", &cache);
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

    // Another program name is another index, since `VAR.prog` can differ.
    let mut p = kpse();
    p.set_program_name(b"anotherprog");
    assert_eq!(p.var_value(b"A_VAR").as_deref(), Some(&b"hello"[..]));
    assert!(!p.index_was_used());
    assert_eq!(indexes(&cache).len(), 2);

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

    // So does a variable the search paths went through, set or unset.
    let extra = d.join("extra");
    put(&extra, "ls-R", b"./tex:\nfour.tex\n");
    put(&extra, "tex/four.tex", b"");
    let mut k = kpse();
    assert!(k.var_value(b"A_NEW").is_some());
    assert!(k.index_was_used());
    // SAFETY: as above.
    unsafe {
        std::env::set_var("EXTRA_DB", &extra);
    }
    let mut k = kpse();
    assert!(k.var_value(b"A_NEW").is_some());
    assert!(!k.index_was_used());
    assert_eq!(k.format_path(KPSE_DB_FORMAT).unwrap().windows(5).filter(|w| w == b"extra").count(), 1);
    // SAFETY: as above.
    unsafe {
        std::env::remove_var("EXTRA_DB");
    }
    let mut k = kpse();
    assert!(k.var_value(b"A_NEW").is_some());
    assert!(!k.index_was_used());

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
