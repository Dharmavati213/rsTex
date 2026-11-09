//! A search path whose working-out has something to say, of a variable set wrong, through the life of the index of `src/index.rs`: the index leaves the path out,
//! and a run says it when it asks for the path, as kpathsea works the path out then, whether that run made the index or found it.  One test, since it sets the
//! process's environment; its cache directory is its own.
use rstex_kpse::names::DOSISH;
use rstex_kpse::*;
use std::fs;
use std::path::{Path, PathBuf};

fn put(root: &Path, rel: &str, data: &[u8]) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, data).unwrap();
}

/// A name as kpathsea gives it back: `/` throughout and, on Windows, the drive letter in lower case.
fn text(p: &Path) -> String {
    let mut v = p.to_str().unwrap().replace('\\', "/").into_bytes();
    names::normalize_path(&mut v);
    String::from_utf8(v).unwrap()
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

#[test]
fn a_path_that_warns_is_worked_out_when_asked_for() {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join("kpse-index-warnings");
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    let tree = d.join("tree");
    put(&tree, "ls-R", b"./tex/a:\none.tex\n");
    put(&tree, "tex/a/one.tex", b"");
    let cnf = format!("TEXMFDBS = {t}\nTEXINPUTS = !!{t}/tex//\nTEXMF = {t}\n", t = text(&tree));
    put(&d, "cnf/texmf.cnf", cnf.as_bytes());
    let cache = cache_goes_under(&d.join("cache"));
    // SAFETY: as there.
    unsafe {
        std::env::set_var("TEXMFCNF", d.join("cnf"));
        std::env::set_var("TEXPICTS", "{a,b");
    }
    // expand.c's, as lib.h's `WARNING` prints it.
    let said = b"warning: kpathsea: {a,b: Unmatched {.\n".to_vec();
    let ask = |k: &mut Kpse| {
        assert!(k.find_file(b"one.tex", KPSE_TEX_FORMAT, false).is_some());
        assert_eq!(k.take_warnings(), Vec::<Vec<u8>>::new(), "nothing said of a path not asked for");
        let pict = k.format_path(KPSE_PICT_FORMAT).unwrap();
        assert_eq!(k.take_warnings(), std::slice::from_ref(&said), "said when asked for");
        assert_eq!(k.format_path(KPSE_PICT_FORMAT).unwrap(), pict);
        assert!(k.take_warnings().is_empty(), "and once");
        pict
    };

    // The run that makes the index says nothing of the path until it asks for it, and then the run that finds the index does the same.
    let mut k = kpse();
    let pict = ask(&mut k);
    assert!(!k.index_was_used());
    assert_eq!(indexes(&cache).len(), 1, "one index");
    let mut k = kpse();
    assert_eq!(ask(&mut k), pict);
    assert!(k.index_was_used());

    // So too with more messages waiting than are kept: that the path had something to say is seen all the same.
    std::thread::sleep(std::time::Duration::from_millis(30));
    let many: String = (0..70).map(|_| "=novar\n").collect();
    put(&d, "cnf/texmf.cnf", [cnf.as_bytes(), many.as_bytes()].concat().as_slice());
    for made in [true, false] {
        let mut k = kpse();
        assert!(k.var_value(b"TEXMF").is_some());
        assert_eq!(k.index_was_used(), !made);
        let w = k.take_warnings();
        assert_eq!(w.len(), 64, "as many as are kept");
        assert!(w.iter().all(|m| m.starts_with(b"warning: ") && m.ends_with(b": (kpathsea) No cnf variable name on line: =novar.\n")));
        assert_eq!(k.format_path(KPSE_PICT_FORMAT).unwrap(), pict);
        assert_eq!(k.take_warnings(), std::slice::from_ref(&said), "made: {made}");
    }
}
