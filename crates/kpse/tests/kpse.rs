//! `Kpse` on synthetic trees, with the process's environment switched off.  These run on any machine, TeX
//! Live or not; the comparison with the real kpsewhich is `tests/oracle/kpse_compare.py`.
//!
//! Each `Kpse` here gets its `SELFAUTO*` values and its variables from the test, so the tests can run in
//! parallel and never spawn anything.
use rstex_kpse::names::{DOSISH, ENV_SEP};
use rstex_kpse::*;
use std::fs;
use std::path::{Path, PathBuf};

fn scratch(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("kpse-{name}"));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn put(root: &Path, rel: &str, data: &[u8]) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, data).unwrap();
}

/// A name as kpathsea gives it back: `/` throughout and, on Windows, the drive letter in lower case, which `kpathsea_normalize_path` does to every element and name.
fn b(p: &Path) -> Vec<u8> {
    let mut v = p.to_str().unwrap().replace('\\', "/").into_bytes();
    names::normalize_path(&mut v);
    v
}

/// tilde.c's `HOMEVAR`.
const HOME: &str = if DOSISH { "USERPROFILE" } else { "HOME" };

/// True if this directory's file system answers for a file by a name in another case, as APFS as it is usually set up does.  Then `stat` finds it, and kpathsea, which asks `stat` first, gives the name
/// back as it was asked for: its own fallback, which gives the name in the directory, comes only after a miss.
#[cfg(unix)]
fn disk_folds_case(dir: &Path) -> bool {
    let probe = dir.join("Case-Probe");
    fs::write(&probe, b"").unwrap();
    let folds = fs::metadata(dir.join("case-probe")).is_ok();
    fs::remove_file(&probe).unwrap();
    folds
}

/// `:` in test data is written for the host's path separator.
fn seps(s: &str) -> Vec<u8> {
    s.replace(':', std::str::from_utf8(&[ENV_SEP]).unwrap()).into_bytes()
}

fn kpse(root: &Path, vars: &[(&str, &[u8])]) -> Kpse {
    let mut tree = Tree::from_selfautoloc(b"/nonexistent-bin/x");
    tree.builtin_cnf = None;
    tree.cnf_default = Some(b(root));
    Kpse::new(Config {
        program_name: b"testprog".to_vec(),
        invocation_name: b"test".to_vec(),
        engine: Some(b"pdftex".to_vec()),
        process_env: false,
        vars: vars.iter().map(|(k, v)| (k.as_bytes().to_vec(), v.to_vec())).collect(),
        tree: Some(tree),
        db_casefold: false,
    })
}

fn s(v: &Option<Vec<u8>>) -> Option<String> {
    v.as_ref().map(|x| String::from_utf8_lossy(x).replace('\\', "/"))
}

#[test]
fn cnf_values_and_their_quirks() {
    let d = scratch("cnf");
    put(
        &d,
        "texmf.cnf",
        b"% comment\n   # comment\nFOO = a#b  %something\nBAR=x\nBAR=y\nBAZ   no-equals\nQUX.testprog = mine\nQUX = generic\nQUX.other = theirs\n\
CONT = one\\\ntwo \\\nthree\nEMPTY =\n=novar\nSEMI = a;b;c\nUSES = $FOO/${BAR}/$UNSET/${UNSET}\nSELF = $SELF/x\nLOOPA = $LOOPB\nLOOPB = $LOOPA\n\
MIXED = foo%bar # comment\nMIXED2 = foo %bar#baz\nCRLF = yes\r\nDOTTED.a.b = 1\n",
    );
    let mut k = kpse(&d, &[]);
    assert_eq!(k.var_value(b"FOO").as_deref(), Some(&b"a#b"[..]), "a comment needs whitespace before it");
    assert_eq!(k.var_value(b"BAR").as_deref(), Some(&b"x"[..]), "the first definition wins");
    assert_eq!(k.var_value(b"BAZ").as_deref(), Some(&b"no-equals"[..]), "the = is optional");
    assert_eq!(k.var_value(b"QUX").as_deref(), Some(&b"mine"[..]), ".prog wins");
    assert_eq!(k.var_value(b"CONT").as_deref(), Some(&b"onetwo three"[..]), "backslash continues, keeping spaces");
    assert_eq!(k.var_value(b"EMPTY"), None, "an empty value is an error, and no definition");
    assert_eq!(k.var_value(b"MIXED").as_deref(), Some(&b"foo%bar"[..]));
    assert_eq!(k.var_value(b"MIXED2").as_deref(), Some(&b"foo"[..]));
    assert_eq!(k.var_value(b"CRLF").as_deref(), Some(&b"yes"[..]));
    if ENV_SEP == b':' {
        assert_eq!(k.var_value(b"SEMI").as_deref(), Some(&b"a:b:c"[..]), "; is : in a value");
    }
    // An unset $X stays literal, so that names with dollars can be read; an unset ${X} goes away.
    assert_eq!(k.var_value(b"USES").as_deref(), Some(&b"a#b/x/$UNSET/"[..]));
    // A variable that references itself stays literal at the inner reference.  The outermost lookup is
    // not yet marked as being expanded, which is why this goes round once more than it seems to.
    assert_eq!(k.var_value(b"SELF").as_deref(), Some(&b"$SELF/x/x"[..]));
    assert_eq!(k.var_value(b"LOOPA").as_deref(), Some(&b"$LOOPB"[..]));
    assert_eq!(k.var_value(b"NOT_THERE"), None);
    // `FOO = $FOO/x` in a path: kpsewhich does not return from this one.  We stop.
    let braces = k.brace_expand(b"$SELF");
    assert!(braces.starts_with(b"$SELF/x/x"), "{}", String::from_utf8_lossy(&braces));
    assert!(braces.len() < 1000, "the recursion is bounded, at {} bytes", braces.len());
    assert!(k.take_warnings().iter().any(|w| w.ends_with(b"references itself (eventually)")));
}

#[test]
fn environment_beats_cnf_and_program_name_selects() {
    let d = scratch("env");
    put(&d, "texmf.cnf", b"A = cnf\nB = cnf\nC.testprog = cnf-prog\nC = cnf\nD.other = cnf-other\nD = cnf\n");
    let mut k = kpse(&d, &[("A", b"env"), ("B", b""), ("C_testprog", b"env-under"), ("D.testprog", b"env-dot")]);
    assert_eq!(k.var_value(b"A").as_deref(), Some(&b"env"[..]));
    assert_eq!(k.var_value(b"B").as_deref(), Some(&b"cnf"[..]), "an empty environment variable counts as unset");
    assert_eq!(k.var_value(b"C").as_deref(), Some(&b"env-under"[..]));
    assert_eq!(k.var_value(b"D").as_deref(), Some(&b"env-dot"[..]));
    k.set_program_name(b"other");
    assert_eq!(k.var_value(b"D").as_deref(), Some(&b"cnf-other"[..]));
    assert_eq!(k.program_name(), b"other");
    assert_eq!(k.var_expand(b"$progname"), b"other");
    assert_eq!(k.var_expand(b"$engine"), b"pdftex");
    // The -cnf-line option acts like the environment.
    k.cnf_line_env_progname(b"E = fromline");
    assert_eq!(k.var_value(b"E").as_deref(), Some(&b"fromline"[..]));
}

#[test]
fn expansion_as_variable_c_and_expand_c() {
    let d = scratch("expand");
    put(&d, "texmf.cnf", b"T = {x,y}/z\nEMPTYBRACE = {}\n");
    let mut k = kpse(&d, &[("FOO", b"foo value"), ("Dollar", b"$"), (HOME, b"/home/u/")]);
    // variable.c's own test table.
    assert_eq!(k.var_expand(b"a"), b"a");
    // Windows' `getenv` does not tell the case of a name.
    assert_eq!(k.var_expand(b"$foo"), if DOSISH { &b"foo value"[..] } else { b"$foo" });
    assert_eq!(k.var_expand(b"a$FOO"), b"afoo value");
    assert_eq!(k.var_expand(b"$Dollar a"), b"$ a");
    assert_eq!(k.var_expand(b"a${FOO}b"), b"afoo valueb");
    assert_eq!(k.var_expand(b"a${}b"), b"ab");
    assert_eq!(k.var_expand(b"$$"), b"$$");
    assert_eq!(k.var_expand(b"a${oops"), b"a");
    assert_eq!(k.var_expand(b"a$"), b"a$");
    // Tilde: a home directory's trailing slash is not doubled, and !! stays in front.
    assert_eq!(k.expand(b"~/x"), b"/home/u/x");
    assert_eq!(k.expand(b"~"), b"/home/u/");
    assert_eq!(k.expand(b"!!~/x"), b"!!/home/u/x");
    assert_eq!(k.expand(b"x~/y"), b"x~/y");
    // A user nobody has heard of is the current directory.
    assert_eq!(k.expand(b"~surely-no-such-user/x"), b"./x");
    // Braces.
    let sep = ENV_SEP as char;
    let j = |parts: &[&str]| parts.join(&sep.to_string()).into_bytes();
    assert_eq!(k.brace_expand(b"{a,b}"), j(&["a", "b"]));
    assert_eq!(k.brace_expand(b"x{a,b}y"), j(&["xay", "xby"]));
    assert_eq!(k.brace_expand(b"{a,b}{c,d}"), j(&["ac", "bc", "ad", "bd"]), "the first index varies fastest");
    assert_eq!(k.brace_expand(b"{a,{b,c}}d"), j(&["ad", "bd", "cd"]));
    assert_eq!(k.brace_expand(b"$T"), j(&["x/z", "y/z"]), "braces in a value expand too");
    assert_eq!(k.brace_expand(b"{$T,q}"), j(&["x/z", "y/z", "q"]));
    assert_eq!(k.brace_expand(b"a,b"), j(&["a", "b"]), "a comma at the top level splits as well");
    assert_eq!(k.brace_expand(b"{}x"), b"x");
    assert_eq!(k.brace_expand(b""), b"");
    assert_eq!(k.brace_expand(b"{~/a,~/b}"), j(&["/home/u/a", "/home/u/b"]));
    let _ = k.take_warnings();
}

#[test]
fn format_paths_come_from_cnf_and_environment() {
    let d = scratch("paths");
    put(&d, "texmf.cnf", b"TEXMF = /tree\nTFMFONTS = .;$TEXMF/fonts/tfm//\nTEXFONTS = /wrong\nTEXINPUTS.testprog = a\nTEXINPUTS = b\nTEXFORMATS = .;$TEXMF/web2c{/$engine,}\nTEXMFDBS = {!!/one,/two}\n");
    let mut k = kpse(&d, &[]);
    assert_eq!(k.format_path(KPSE_TFM_FORMAT).unwrap(), seps(".:/tree/fonts/tfm//"), "the first variable that is set");
    assert_eq!(k.format_path(KPSE_TEX_FORMAT).unwrap(), b"a");
    assert_eq!(k.format_path(KPSE_FMT_FORMAT).unwrap(), seps(".:/tree/web2c/pdftex:/tree/web2c"));
    assert_eq!(k.format_path(KPSE_DB_FORMAT).unwrap(), seps("/one:/two"), "!! comes off for the databases");
    // VFFONTS is not set, so the next variable of the format counts.
    assert_eq!(k.format_path(KPSE_VF_FORMAT).unwrap(), b"/wrong");
    // A format the cnf says nothing of has the compile-time path, which is nothing.
    assert_eq!(k.format_path(KPSE_MF_FORMAT).unwrap(), b"/nonesuch");
    assert_eq!(k.format_path(99), None);

    // Extra colons stand for the next level down, and a changed program name forgets the paths.
    let mut k = kpse(&d, &[("TFMFONTS", &seps(":extra")), ("TEXINPUTS", &seps("x::y")), ("TEXINPUTS_other", b"z")]);
    assert_eq!(k.format_path(KPSE_TFM_FORMAT).unwrap(), seps(".:/tree/fonts/tfm//:extra"));
    assert_eq!(k.format_path(KPSE_TEX_FORMAT).unwrap(), seps("x:a:y"));
    k.set_program_name(b"other");
    assert_eq!(k.format_path(KPSE_TEX_FORMAT).unwrap(), b"z");
    // <PREFIX>FONTS overrides, and the mode goes where texmf.cnf says.
    let mut k = kpse(&d, &[("OURFONTS", b"override")]);
    k.init_prog(b"OUR", 600, None);
    assert_eq!(k.format_path(KPSE_TFM_FORMAT).unwrap(), b"override");
    assert_eq!(k.var_value(b"MAKETEX_MODE").as_deref(), Some(&b"/"[..]));
    assert_eq!(k.var_value(b"MAKETEX_BASE_DPI").as_deref(), Some(&b"600"[..]));
}

/// A tree with a database, a tree without one, and the rules that decide between them.
struct Fixture {
    root: PathBuf,
    cnf: PathBuf,
}

fn fixture(name: &str) -> Fixture {
    let root = scratch(name);
    let db = root.join("db");
    for f in ["top.tex", "a/one.tex", "a/dup.tex", "b/dup.tex", "a/disk_only.tex", ".hidden/secret.tex", "c/real.mf", "fonts/tfm/cmr.tfm"] {
        put(&db, f, b"");
    }
    put(
        &db,
        "ls-R",
        b"% ls-R\n./:\nls-R\naliases\ntop.tex\nghost.tex\n\n./a:\none.tex\ndup.tex\nnot_on_disk.tex\n\n./b:\ndup.tex\n\n./.hidden:\nsecret.tex\n\n./c:\nreal.mf\n\n./fonts/tfm:\ncmr.tfm\n",
    );
    put(&db, "aliases", b"% aliases\none.tex uno.tex\n");
    let plain = root.join("plain");
    for f in ["p/only.tex", "q/qonly.tex", "q/r/deep.tex", ".hid/hidden.tex", "CASE.TEX", "p/same.tex", "q/same.tex", "x.tfm", "y.tex", "y"] {
        put(&plain, f, b"");
    }
    let cnf = root.join("cnf");
    let (d, p) = (root.join("db"), root.join("plain"));
    let text = format!(
        "TEXMFDBS = {d}\nTEXINPUTS = {d}//;{p}//\nDBONLY = !!{d}//\nMFINPUTS = !!{d}//\nTFMFONTS = {d}/fonts//;{p}\nENCFONTS = {p}\nTEXFORMATS = !!{d}//;{p}//\ntry_std_extension_first = t\ntexmf_casefold_search = 1\n",
        d = String::from_utf8(b(&d)).unwrap(),
        p = String::from_utf8(b(&p)).unwrap(),
    );
    put(&cnf, "texmf.cnf", text.as_bytes());
    Fixture { root, cnf }
}

#[test]
fn search_prefers_the_database_and_finds_what_is_on_disk() {
    let f = fixture("search");
    let (db, plain) = (f.root.join("db"), f.root.join("plain"));
    let dbs = String::from_utf8(b(&db)).unwrap();
    let pls = String::from_utf8(b(&plain)).unwrap();
    let mut k = kpse(&f.cnf, &[]);

    // In ls-R, and on disk.  The doubled slash at the top of a tree is what kpathsea gives as well.
    assert_eq!(s(&k.find_file(b"top", KPSE_TEX_FORMAT, false)), Some(format!("{dbs}//top.tex")));
    assert_eq!(s(&k.find_file(b"one.tex", KPSE_TEX_FORMAT, false)), Some(format!("{dbs}/a/one.tex")));
    // In ls-R in two directories: the order of the file.
    assert_eq!(s(&k.find_file(b"dup.tex", KPSE_TEX_FORMAT, false)), Some(format!("{dbs}/a/dup.tex")));
    assert_eq!(
        k.find_file_generic(b"dup.tex", KPSE_TEX_FORMAT, false, true).iter().map(|v| String::from_utf8_lossy(v).into_owned()).collect::<Vec<_>>(),
        vec![format!("{dbs}/a/dup.tex"), format!("{dbs}/b/dup.tex")]
    );
    // With a directory part: it must match, at the end of the directory.
    assert_eq!(s(&k.find_file(b"b/dup.tex", KPSE_TEX_FORMAT, false)), Some(format!("{dbs}/b/dup.tex")));
    assert_eq!(k.find_file(b"z/dup.tex", KPSE_TEX_FORMAT, false), None);
    // In ls-R, not on disk: ls-R is not believed.
    assert_eq!(k.find_file(b"ghost.tex", KPSE_TEX_FORMAT, false), None);
    assert_eq!(k.find_file(b"not_on_disk.tex", KPSE_TEX_FORMAT, true), None);
    // Hidden directories are not searched, either way.
    assert_eq!(k.find_file(b"secret.tex", KPSE_TEX_FORMAT, true), None);
    // On disk, not in ls-R: only if it must exist, since the element allows the disk.
    assert_eq!(k.find_file(b"disk_only.tex", KPSE_TEX_FORMAT, false), None);
    assert_eq!(s(&k.find_file(b"disk_only.tex", KPSE_TEX_FORMAT, true)), Some(format!("{dbs}/a/disk_only.tex")));
    // Under !! the disk is never searched, not even so.
    assert_eq!(k.find_file(b"disk_only.mf", KPSE_MF_FORMAT, true), None);
    assert_eq!(s(&k.find_file(b"real", KPSE_MF_FORMAT, false)), Some(format!("{dbs}/c/real.mf")));
    // Outside the database's tree the disk is searched, with // through all subdirectories.
    assert_eq!(s(&k.find_file(b"only.tex", KPSE_TEX_FORMAT, false)), Some(format!("{pls}/p/only.tex")));
    assert_eq!(s(&k.find_file(b"deep", KPSE_TEX_FORMAT, false)), Some(format!("{pls}/q/r/deep.tex")));
    assert_eq!(s(&k.find_file(b"r/deep.tex", KPSE_TEX_FORMAT, false)), Some(format!("{pls}/q/r/deep.tex")));
    assert_eq!(k.find_file(b"hidden.tex", KPSE_TEX_FORMAT, false), None);
    // An alias of the database.
    assert_eq!(s(&k.find_file(b"uno.tex", KPSE_TEX_FORMAT, false)), Some(format!("{dbs}/a/one.tex")));
    // A format that searches with its suffix only, and one that takes the name as well.
    assert_eq!(s(&k.find_file(b"x", KPSE_TFM_FORMAT, false)), Some(format!("{pls}/x.tfm")));
    assert_eq!(k.find_file(b"y", KPSE_TFM_FORMAT, false), None, "y exists but is not y.tfm");
    assert_eq!(s(&k.find_file(b"y", KPSE_TEX_FORMAT, false)), Some(format!("{pls}/y.tex")), "the suffix first");
    // An unknown format is nothing, not a crash.
    assert_eq!(k.find_file(b"y", -1, false), None);
    assert_eq!(k.find_file(b"y", KPSE_LAST_FORMAT, false), None);
}

/// Windows compares in one case throughout and has no fallback: `the_database_can_ignore_case_as_macos_wants` holds what it does.
#[cfg(unix)]
#[test]
fn case_folding_only_on_disk_and_only_when_on() {
    let f = fixture("case");
    let pls = String::from_utf8(b(&f.root.join("plain"))).unwrap();
    let dbs = String::from_utf8(b(&f.root.join("db"))).unwrap();
    let folds = disk_folds_case(&f.root);
    // Which name comes back: on a disk that folds case the one asked for is the file, there is no miss, and the fallback does not come into it.
    let on_disk = |dir: &str, asked: &str, name: &str| Some(format!("{dir}/{}", if folds { asked } else { name }));
    let mut k = kpse(&f.cnf, &[]);
    assert_eq!(s(&k.find_file(b"case.tex", KPSE_TEX_FORMAT, false)), on_disk(&pls, "case.tex", "CASE.TEX"));
    assert_eq!(s(&k.find_file(b"ONLY.TEX", KPSE_TEX_FORMAT, false)), on_disk(&format!("{pls}/p"), "ONLY.TEX", "only.tex"));
    // ls-R is exact, and this search does not go to the disk.
    assert_eq!(k.find_file(b"TOP.TEX", KPSE_TEX_FORMAT, false), None);
    // Absolute names too.
    assert_eq!(s(&k.find_file(format!("{dbs}/TOP.TEX").as_bytes(), KPSE_TEX_FORMAT, false)), on_disk(&dbs, "TOP.TEX", "top.tex"));

    // Turned off, what is left is what the disk says of the name.
    let mut k = kpse(&f.cnf, &[("texmf_casefold_search", b"0")]);
    let by_stat = |dir: &str, asked: &str| folds.then(|| format!("{dir}/{asked}"));
    assert_eq!(s(&k.find_file(b"case.tex", KPSE_TEX_FORMAT, false)), by_stat(&pls, "case.tex"));
    assert_eq!(s(&k.find_file(format!("{dbs}/TOP.TEX").as_bytes(), KPSE_TEX_FORMAT, false)), by_stat(&dbs, "TOP.TEX"));
}

/// A miss lists a directory once, and what comes to it afterwards is found all the same, whether or not its stamp is old enough to keep a list by.
#[cfg(unix)]
#[test]
fn case_folding_lists_a_directory_once_and_sees_what_changes() {
    let d = scratch("listed");
    let dir = d.join("plain");
    put(&dir, "other.tex", b"");
    let p = String::from_utf8(b(&dir)).unwrap();
    let folds = disk_folds_case(&d);
    // A file `stat` finds, on a disk that folds case, comes back as it was asked for, and no list is made for it.
    let found = |asked: &str, name: &str| Some(format!("{p}/{}", if folds { asked } else { name }));
    put(&d, "cnf/texmf.cnf", format!("TEXINPUTS = {p}\ntexmf_casefold_search = 1\n").as_bytes());
    let old = |path: &Path| {
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        fs::File::open(path).unwrap().set_modified(t).unwrap();
    };
    // The change time of a directory cannot be set, so this waits until it is old enough.
    std::thread::sleep(std::time::Duration::from_millis(2100));
    old(&dir);
    std::thread::sleep(std::time::Duration::from_millis(2100));
    let mut k = kpse(&d.join("cnf"), &[]);
    assert_eq!(k.find_file(b"a.tex", KPSE_TEX_FORMAT, false), None);
    let once = k.dir_scans();
    assert_eq!(k.find_file(b"b.tex", KPSE_TEX_FORMAT, false), None);
    assert_eq!(k.find_file(b"OTHER.tex", KPSE_TEX_FORMAT, false).map(|_| ()), Some(()));
    assert_eq!(k.dir_scans(), once, "a directory that has not changed is not listed again");
    // A file that comes later changes the directory's stamp.
    put(&dir, "A.TEX", b"");
    assert_eq!(s(&k.find_file(b"a.tex", KPSE_TEX_FORMAT, false)), found("a.tex", "A.TEX"));
    assert_eq!(k.dir_scans(), once + u64::from(!folds));
    // That stamp is young, so no list is kept by it: the next one comes at once, and is seen too, whatever the clock's tick.
    put(&dir, "B.TEX", b"");
    assert_eq!(s(&k.find_file(b"b.tex", KPSE_TEX_FORMAT, false)), found("b.tex", "B.TEX"));
    // And one that goes.
    fs::remove_file(dir.join("B.TEX")).unwrap();
    assert_eq!(k.find_file(b"b.tex", KPSE_TEX_FORMAT, false), None);
}

/// A directory that changed moments ago, as when the last pass of a document put its PDF there: on a file system whose stamps have a fraction of a second a list is kept all the same, and what comes
/// after it still shows.
#[cfg(unix)]
#[test]
fn case_folding_keeps_a_list_soon_after_a_change_if_the_stamp_is_fine_enough() {
    let d = scratch("listed-soon");
    let dir = d.join("plain");
    put(&dir, "other.tex", b"");
    let p = String::from_utf8(b(&dir)).unwrap();
    let folds = disk_folds_case(&d);
    put(&d, "cnf/texmf.cnf", format!("TEXINPUTS = {p}\ntexmf_casefold_search = 1\n").as_bytes());
    use std::os::unix::fs::MetadataExt;
    if fs::metadata(&dir).unwrap().mtime_nsec() == 0 {
        return;
    }
    std::thread::sleep(std::time::Duration::from_millis(50));
    let mut k = kpse(&d.join("cnf"), &[]);
    assert_eq!(k.find_file(b"a.tex", KPSE_TEX_FORMAT, false), None);
    let once = k.dir_scans();
    assert_eq!(k.find_file(b"b.tex", KPSE_TEX_FORMAT, false), None);
    assert_eq!(k.dir_scans(), once, "a directory that has not changed is not listed again");
    put(&dir, "A.TEX", b"");
    assert_eq!(s(&k.find_file(b"a.tex", KPSE_TEX_FORMAT, false)), Some(format!("{p}/{}", if folds { "a.tex" } else { "A.TEX" })));
    assert_eq!(k.dir_scans(), once + u64::from(!folds));
}

/// The requirement from the measurement in DESIGN.md section 1: a search that ls-R answers scans nothing.
#[test]
fn a_search_ls_r_answers_opens_no_directory() {
    let f = fixture("noscan");
    let dbs = String::from_utf8(b(&f.root.join("db"))).unwrap();

    // Elements under !!, so the disk is not allowed in any case.
    let mut k = kpse(&f.cnf, &[]);
    assert_eq!(s(&k.find_file(b"real", KPSE_MF_FORMAT, true)), Some(format!("{dbs}/c/real.mf")));
    assert_eq!(k.find_file(b"not-there", KPSE_MF_FORMAT, true), None);
    assert_eq!(k.dir_scans(), 0, "!! elements");

    // Elements that allow the disk, but which the database covers: a hit, and a miss that need not exist.
    let tree = String::from_utf8(b(&f.root.join("db"))).unwrap();
    let mut k = kpse(&f.cnf, &[("TEXINPUTS", format!("{tree}//").as_bytes())]);
    assert_eq!(s(&k.find_file(b"one.tex", KPSE_TEX_FORMAT, false)), Some(format!("{dbs}/a/one.tex")));
    assert_eq!(k.find_file(b"not-there", KPSE_TEX_FORMAT, false), None);
    assert_eq!(k.dir_scans(), 0, "covered elements");
    // Only must_exist on a miss goes to the disk, which is what the flag means.
    assert_eq!(k.find_file(b"not-there", KPSE_TEX_FORMAT, true), None);
    assert!(k.dir_scans() > 0);

    // A hit in an uncovered element costs its expansion once, and later lookups there none.
    let mut k = kpse(&f.cnf, &[]);
    assert!(k.find_file(b"only.tex", KPSE_TEX_FORMAT, false).is_some());
    let after_first = k.dir_scans();
    assert!(after_first > 0);
    assert!(k.find_file(b"deep.tex", KPSE_TEX_FORMAT, false).is_some());
    assert!(k.find_file(b"only.tex", KPSE_TEX_FORMAT, false).is_some());
    assert_eq!(k.dir_scans(), after_first, "the expansion is cached");

    // Listing is not even possible in here, and the lookup works all the same.
    if ENV_SEP == b':' {
        let d = scratch("noscan-mode");
        put(&d, "tree/sub/f.tex", b"");
        put(&d, "tree/ls-R", b"% ls-R\n./sub:\nf.tex\n");
        let t = String::from_utf8(b(&d.join("tree"))).unwrap();
        put(&d, "cnf/texmf.cnf", format!("TEXMFDBS = {t}\nTEXINPUTS = {t}//\n").as_bytes());
        set_mode(&d.join("tree/sub"), 0o111);
        let mut k = kpse(&d.join("cnf"), &[]);
        let got = s(&k.find_file(b"f.tex", KPSE_TEX_FORMAT, false));
        set_mode(&d.join("tree/sub"), 0o755);
        assert_eq!(got, Some(format!("{t}/sub/f.tex")));
        assert_eq!(k.dir_scans(), 0);
    }
}

#[cfg(unix)]
fn set_mode(p: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(p, fs::Permissions::from_mode(mode)).unwrap();
}

#[cfg(windows)]
fn set_mode(_p: &Path, _mode: u32) {}

#[test]
fn the_database_can_ignore_case_as_macos_wants() {
    let f = fixture("dbcase");
    let dbs = String::from_utf8(b(&f.root.join("db"))).unwrap();
    let make = |vars: &[(&str, &[u8])]| {
        let mut tree = Tree::from_selfautoloc(b"/nonexistent-bin/x");
        tree.builtin_cnf = None;
        tree.cnf_default = Some(b(&f.cnf));
        Kpse::new(Config {
            program_name: b"testprog".to_vec(),
            process_env: false,
            vars: vars.iter().map(|(k, v)| (k.as_bytes().to_vec(), v.to_vec())).collect(),
            tree: Some(tree),
            db_casefold: true,
            ..Config::default()
        })
    };
    let mut k = make(&[]);
    // An exact match is untouched, and is found first.
    assert_eq!(s(&k.find_file(b"one.tex", KPSE_TEX_FORMAT, false)), Some(format!("{dbs}/a/one.tex")));
    // The name on disk comes back, in the case the database has it.
    assert_eq!(s(&k.find_file(b"ONE.TEX", KPSE_TEX_FORMAT, false)), Some(format!("{dbs}/a/one.tex")));
    assert_eq!(s(&k.find_file(b"One", KPSE_TEX_FORMAT, false)), Some(format!("{dbs}/a/one.tex")));
    // Under !! too, which is the point: the disk is never asked there.
    assert_eq!(s(&k.find_file(b"REAL.MF", KPSE_MF_FORMAT, false)), Some(format!("{dbs}/c/real.mf")));
    // The directory part must still match exactly: only the last component folds.  Windows compares in one case throughout, as kpathsea's `MONOCASE_FILENAMES`.
    assert_eq!(s(&k.find_file(b"b/DUP.TEX", KPSE_TEX_FORMAT, false)), Some(format!("{dbs}/b/dup.tex")));
    assert_eq!(s(&k.find_file(b"B/dup.tex", KPSE_TEX_FORMAT, false)), DOSISH.then(|| format!("{dbs}/b/dup.tex")));
    // In ls-R but not on disk is still nothing, and what is nowhere is nothing.
    assert_eq!(k.find_file(b"GHOST.TEX", KPSE_TEX_FORMAT, false), None);
    assert_eq!(k.find_file(b"nowhere-at-all.tex", KPSE_TEX_FORMAT, false), None);
    // Two files that fold alike: in the order read, and all of them if asked for.
    let all = k.find_file_generic(b"DUP.TEX", KPSE_TEX_FORMAT, false, true);
    assert_eq!(all, vec![format!("{dbs}/a/dup.tex").into_bytes(), format!("{dbs}/b/dup.tex").into_bytes()]);

    // texmf_casefold_search turns it off, as it does for the disk.  On Windows there is nothing to turn off.
    let mut k = make(&[("texmf_casefold_search", b"0")]);
    assert_eq!(s(&k.find_file(b"ONE.TEX", KPSE_TEX_FORMAT, false)), DOSISH.then(|| format!("{dbs}/a/one.tex")));
    assert_eq!(s(&k.find_file(b"one.tex", KPSE_TEX_FORMAT, false)), Some(format!("{dbs}/a/one.tex")));
}

#[test]
fn init_prog_leaves_a_path_already_computed() {
    let d = scratch("initprog");
    put(&d, "texmf.cnf", b"TFMFONTS = fromcnf\nVFFONTS = vf\n");
    let mut k = kpse(&d, &[("OURFONTS", b"override")]);
    assert_eq!(k.format_path(KPSE_TFM_FORMAT).unwrap(), b"fromcnf");
    // pdfTeX calls this at its first shipout, by when TFM files have been looked up.
    k.init_prog(b"OUR", 600, None);
    assert_eq!(k.format_path(KPSE_TFM_FORMAT).unwrap(), b"fromcnf");
}

#[test]
fn files_read_are_reported_for_the_recorder() {
    let d = scratch("recorder");
    let cnf = String::from_utf8(b(&d)).unwrap();
    put(&d, "texmf.cnf", format!("TEXFONTMAPS = {cnf}\nTFMFONTS = {cnf}\n").as_bytes());
    put(&d, "texfonts.map", b"real.tfm alias\n");
    put(&d, "real.tfm", b"");
    let mut k = kpse(&d, &[]);
    assert_eq!(s(&k.find_file(b"alias", KPSE_TFM_FORMAT, false)), Some(format!("{cnf}/real.tfm")), "through the map");
    let seen: Vec<String> = k.take_recorded().iter().map(|f| String::from_utf8_lossy(f).into_owned()).collect();
    assert_eq!(seen, vec![format!("{cnf}/texmf.cnf"), format!("{cnf}/texfonts.map")]);
    assert!(k.take_recorded().is_empty(), "each once");
}

#[test]
fn absolute_and_explicitly_relative_names_ignore_the_path() {
    let f = fixture("abs");
    let dbs = String::from_utf8(b(&f.root.join("db"))).unwrap();
    let mut k = kpse(&f.cnf, &[]);
    assert_eq!(s(&k.find_file(format!("{dbs}/top").as_bytes(), KPSE_TEX_FORMAT, false)), Some(format!("{dbs}/top.tex")));
    assert_eq!(k.find_file(format!("{dbs}/a").as_bytes(), KPSE_TEX_FORMAT, true), None, "a directory is not a file");
    assert_eq!(k.find_file(b"/rstex/surely/not/here", KPSE_TEX_FORMAT, true), None);
    assert_eq!(k.find_file(b"./surely/not/here", KPSE_TEX_FORMAT, true), None);
    // C strings end at the first NUL.
    assert_eq!(s(&k.find_file(format!("{dbs}/top\0junk").as_bytes(), KPSE_TEX_FORMAT, false)), Some(format!("{dbs}/top.tex")));
}

#[test]
fn a_float_changes_the_order_within_an_element_only() {
    // p/same.tex and q/same.tex are in one element.  Whichever directory had the first hit goes to the
    // top of the element's list, so it wins from then on, whatever order readdir gave.
    let f = fixture("float");
    let mut k = kpse(&f.cnf, &[]);
    assert!(k.find_file(b"only.tex", KPSE_TEX_FORMAT, false).is_some(), "in p");
    let after = s(&k.find_file(b"same.tex", KPSE_TEX_FORMAT, false)).unwrap();
    assert!(after.ends_with("p/same.tex"), "{after}");

    let mut k = kpse(&f.cnf, &[]);
    assert!(k.find_file(b"qonly.tex", KPSE_TEX_FORMAT, false).is_some(), "in q");
    let after = s(&k.find_file(b"same.tex", KPSE_TEX_FORMAT, false)).unwrap();
    assert!(after.ends_with("q/same.tex"), "{after}");

    // A second float goes after the first, not before it, so the first stays on top.
    assert!(k.find_file(b"only.tex", KPSE_TEX_FORMAT, false).is_some());
    let after = s(&k.find_file(b"same.tex", KPSE_TEX_FORMAT, false)).unwrap();
    assert!(after.ends_with("q/same.tex"), "{after}");
}

#[test]
fn name_checks() {
    let d = scratch("names");
    put(&d, "texmf.cnf", b"TEXMFVAR = /var/tex\nTEXMFSYSVAR = /sysvar\n");
    let mut k = kpse(&d, &[("TEXMF_OUTPUT_DIRECTORY", b"/out"), (HOME, b"/home/u")]);
    // Anything may be read, as of 2026.
    for n in [&b"/etc/passwd"[..], b"../x", b".rc"] {
        assert!(k.in_name_ok(n));
        assert!(k.in_name_ok_extended(n));
    }
    for (name, ok) in [
        (&b"a.tex"[..], true),
        (b"sub/a.tex", true),
        (b"./a", true),
        (b"sub/./a", true),
        (b"a..", true),
        (b"", true),
        (b"/out/a", true),
        (b"/out", true),
        (b"/outx", false),
        (b"/etc/x", false),
        (b"~/x", false),
        (b"../a", false),
        (b"a/../b", false),
        (b"/out/../a", false),
        (b"/var/tex/a", false),
    ] {
        assert_eq!(k.out_name_ok_silent(name), ok, "{}", String::from_utf8_lossy(name));
    }
    if ENV_SEP == b':' {
        // Dot files are refused on Unix only: Windows has no such convention.
        for name in [&b".rc"[..], b"a/.rc", b"..a", b"/out/.a"] {
            assert!(!k.out_name_ok_silent(name), "{}", String::from_utf8_lossy(name));
        }
    }
    assert!(k.out_name_ok_silent_extended(b"/var/tex/a"));
    assert!(k.out_name_ok_silent_extended(b"/sysvar/a"));
    assert!(!k.out_name_ok_silent_extended(b"/var/texx"));
    assert!(!k.out_name_ok_silent_extended(b"/etc/a"));

    let mut r = kpse(&d, &[("openout_any", b"r")]);
    assert!(r.out_name_ok_silent(b"/etc/x"));
    assert!(r.out_name_ok_silent(b"../x"));
    let mut a = kpse(&d, &[("openout_any", b"a")]);
    assert!(a.out_name_ok_silent(b".rc"));
    assert!(a.out_name_ok_silent(b"/etc/../x"));
}

#[test]
fn yesno_and_bound_variables() {
    let d = scratch("bound");
    put(&d, "texmf.cnf", b"T1 = t\nT2 = yes\nT3 = 1\nF1 = n\nF2 = 0\nF3 = false\nMEM = 12345 % comment\nNEG = -3\nZERO = 0\nJUNK = abc\n");
    let mut k = kpse(&d, &[]);
    for v in [&b"T1"[..], b"T2", b"T3"] {
        assert!(k.yesno(v));
    }
    for v in [&b"F1"[..], b"F2", b"F3", b"UNSET"] {
        assert!(!k.yesno(v));
    }
    assert_eq!(k.setup_bound_var(b"MEM", 7), 12345);
    assert_eq!(k.setup_bound_var(b"UNSET", 7), 7);
    assert_eq!(k.setup_bound_var(b"NEG", 7), 7, "a negative number is always wrong");
    assert_eq!(k.setup_bound_var(b"ZERO", 7), 7, "0 is wrong where the default is positive");
    assert_eq!(k.setup_bound_var(b"ZERO", 0), 0, "and fine where it is not");
    assert_eq!(k.setup_bound_var(b"JUNK", 7), 7);
}

#[test]
fn no_cnf_means_no_path_unless_a_tree_says_otherwise() {
    let d = scratch("nocnf");
    put(&d, "here.tex", b"");
    let mut k = kpse(&d, &[]);
    assert_eq!(k.format_path(KPSE_TEX_FORMAT).unwrap(), b"/nonesuch", "as kpsewhich with TEXMFCNF pointing at nothing");
    assert!(k.take_warnings().iter().any(|w| w.starts_with(b"kpathsea: configuration file texmf.cnf not found")));
    assert_eq!(k.find_file(b"here", KPSE_TEX_FORMAT, true), None);

    // A machine with no tree at all still searches the current directory.
    let mut tree = Tree::from_selfautoloc(b"/nonexistent-bin/x");
    tree.cnf_default = Some(b(&d));
    let mut k = Kpse::new(Config { process_env: false, tree: Some(tree), ..Config::default() });
    let path = k.format_path(KPSE_TEX_FORMAT).unwrap();
    assert!(path.starts_with(&seps(".:")), "{}", String::from_utf8_lossy(&path));
    assert_eq!(k.var_value(b"openout_any").as_deref(), Some(&b"p"[..]));
    assert_eq!(k.setup_bound_var(b"error_line", 1), 79);
    assert_eq!(k.setup_bound_var(b"main_memory", 1), 5_000_000);
}

#[test]
fn a_tree_is_found_from_rstex_texmf_alone() {
    // With a texmf.cnf: its idea of the root is overridden by what the variable says.
    let d = scratch("tree-cnf");
    let dist = d.join("texlive/2026/texmf-dist");
    put(&dist, "web2c/texmf.cnf", b"TEXMFROOT = /wrong\nTEXMFDIST = /wrong/dist\nTEXMFDBS = $TEXMFDIST\nTEXINPUTS = .;$TEXMFDIST/tex//\n");
    put(&dist, "tex/latex/x/x.sty", b"");
    put(&dist, "ls-R", b"% ls-R\n./tex/latex/x:\nx.sty\n");
    let env_val = b(&dist);
    let env = |name: &[u8]| if name == b"RSTEX_TEXMF" { Some(env_val.clone()) } else { None };
    let tree = discover(&env);
    assert_eq!(tree.source, TreeSource::Env);
    assert_eq!(tree.selfauto[2], b(&d.join("texlive/2026")));
    assert!(tree.walk_roots.is_empty());
    let mut k = Kpse::new(Config { process_env: false, tree: Some(tree), ..Config::default() });
    assert_eq!(k.var_value(b"TEXMFDIST").as_deref(), Some(&env_val[..]));
    let want = [&env_val[..], b"/tex/latex/x/x.sty"].concat();
    assert_eq!(k.find_file(b"x.sty", KPSE_TEX_FORMAT, false), Some(want));

    // Without one, and without an ls-R: the built-in settings, and an index from walking the tree.
    let d = scratch("tree-bare");
    put(&d, "tex/latex/y/y.sty", b"");
    put(&d, "fonts/tfm/z/z.tfm", b"");
    put(&d, "tex/latex/.git/hid.sty", b"");
    let tree = from_tree_dirs(&[b(&d)], TreeSource::Env);
    assert_eq!(tree.walk_roots, vec![b(&d)]);
    let mut k = Kpse::new(Config { process_env: false, tree: Some(tree), ..Config::default() });
    let dir = String::from_utf8(b(&d)).unwrap();
    assert_eq!(s(&k.find_file(b"y.sty", KPSE_TEX_FORMAT, false)), Some(format!("{dir}/tex/latex/y/y.sty")));
    assert_eq!(s(&k.find_file(b"z", KPSE_TFM_FORMAT, false)), Some(format!("{dir}/fonts/tfm/z/z.tfm")));
    assert_eq!(k.find_file(b"hid.sty", KPSE_TEX_FORMAT, true), None);
}

#[test]
fn a_bad_cnf_warns_and_carries_on() {
    let d = scratch("badcnf");
    put(&d, "texmf.cnf", b"=novar\n.prog = x\nA.;b = x\nB.$c = y\nC =\nGOOD = 1\nLAST = x\\");
    let mut k = kpse(&d, &[]);
    assert_eq!(k.var_value(b"GOOD").as_deref(), Some(&b"1"[..]));
    assert_eq!(k.var_value(b"LAST").as_deref(), Some(&b"x"[..]));
    let w = k.take_warnings();
    let all: Vec<String> = w.iter().map(|m| String::from_utf8_lossy(m).into_owned()).collect();
    for needle in ["No cnf variable name", "Unlikely character ; in program name", "Unlikely character $ in program name", "No cnf value", "Last line of file ends with"] {
        assert!(all.iter().any(|m| m.contains(needle)), "{needle} in {all:?}");
    }
}

#[test]
fn ls_r_with_nothing_in_it_does_not_count() {
    let d = scratch("emptydb");
    let tree = d.join("t");
    put(&tree, "ls-R", b"% nothing here\n");
    put(&tree, "sub/f.tex", b"");
    let t = String::from_utf8(b(&tree)).unwrap();
    put(&d, "cnf/texmf.cnf", format!("TEXMFDBS = {t}\nTEXINPUTS = !!{t}//;{t}//\n").as_bytes());
    let mut k = kpse(&d.join("cnf"), &[]);
    // No usable database: !! elements give nothing, and the plain element goes to the disk.
    assert_eq!(s(&k.find_file(b"f.tex", KPSE_TEX_FORMAT, false)), Some(format!("{t}/sub/f.tex")));
    assert!(k.take_warnings().iter().any(|w| w.ends_with(b"No usable entries in ls-R")));
}

/// Waits until the stamps of what was just made are old enough to keep a list by: 20 ms where they have a fraction of a second, otherwise 2 s.
#[cfg(unix)]
fn settle(dir: &Path) {
    use std::os::unix::fs::MetadataExt;
    let coarse = fs::metadata(dir).unwrap().mtime_nsec() == 0;
    std::thread::sleep(std::time::Duration::from_millis(if coarse { 2100 } else { 60 }));
}

/// A tree on the disk gives the same answers by what its directories are known to hold as by asking the disk for every name, which is what C does: for names that
/// are there, that are not, that differ in case, that lie in a subdirectory or are odd, with one answer and with all, while the tree changes under both.
#[cfg(unix)]
#[test]
fn lists_of_directories_answer_as_the_disk_does() {
    let d = scratch("lists");
    let tree = d.join("tree");
    let files = [
        "a/one.tex", "a/Two.TEX", "a/deep/three.tex", "a/deep/one.tex", "b/one.tex", "b/four.sty", "c/.hidden/five.tex", "c/six.tex", "c/sub/x/seven.tex", "top.tex",
        "c/odd name.tex", "c/tilde~1.tex", "c/dot.", "b/na\u{ef}ve.tex", "e/plain.tex", "e/CaSe.tex", "e/case.TEX", "post/deep/eight.tex", "f/post/nine.tex",
    ];
    for f in files {
        put(&tree, f, b"x");
    }
    std::os::unix::fs::symlink(tree.join("a/deep"), tree.join("b/link")).unwrap();
    std::os::unix::fs::symlink(tree.join("top.tex"), tree.join("e/alias.tex")).unwrap();
    std::os::unix::fs::symlink(tree.join("nowhere"), tree.join("e/dangling.tex")).unwrap();
    let t = String::from_utf8(b(&tree)).unwrap();
    put(&d, "cnf/texmf.cnf", format!("TEXINPUTS = {t}/e;{t}//;{t}/c;{t}//post;{t}/missing//\ntexmf_casefold_search = 1\n").as_bytes());
    let names: Vec<&[u8]> = vec![
        b"one.tex", b"one", b"two.tex", b"Two.TEX", b"TWO.TEX", b"three.tex", b"deep/three.tex", b"DEEP/three.tex", b"deep/THREE.tex", b"four.sty", b"five.tex",
        b"six.tex", b"seven.tex", b"x/seven.tex", b"sub/x/seven.tex", b"top.tex", b"TOP.TEX", b"odd name.tex", b"ODD NAME.tex", b"tilde~1.tex", b"TILDE~1.TEX", b"dot.",
        b"DOT.", b"na\xc3\xafve.tex", b"NA\xc3\xafVE.TEX", b"plain.tex", b"case.tex", b"CASE.TEX", b"CaSe.tex", b"eight.tex", b"nine.tex", b"alias.tex", b"ALIAS.tex",
        b"dangling.tex", b"nothing.tex", b"nothing", b"a", b"deep", b"link/three.tex", b"", b"deep/", b"..", b"../tree/top.tex", b"sub//x/seven.tex", b"new.tex",
        b"NEW.tex", b"late/new.tex", b"gone.tex", b"moved.tex",
    ];
    let ask = |k: &mut Kpse| {
        let mut out = Vec::new();
        for name in &names {
            for must in [false, true] {
                out.push((String::from_utf8_lossy(name).into_owned(), must, k.find_file_generic(name, KPSE_TEX_FORMAT, must, false), k.find_file_generic(name, KPSE_TEX_FORMAT, must, true)));
            }
        }
        out
    };
    let mut with = kpse(&d.join("cnf"), &[]);
    let mut without = kpse(&d.join("cnf"), &[]);
    without.set_dir_lists(false);
    let both = |with: &mut Kpse, without: &mut Kpse, when: &str| {
        let (a, z) = (ask(with), ask(without));
        for (x, y) in a.iter().zip(&z) {
            assert_eq!(x, y, "{when}");
        }
    };
    // The tree is a moment old: nothing is kept yet, and then it is.
    both(&mut with, &mut without, "new");
    settle(&tree);
    both(&mut with, &mut without, "settled");
    let (before, before_without) = (with.disk_looks(), without.disk_looks());
    both(&mut with, &mut without, "again");
    let (looks, looks_without) = (with.disk_looks() - before, without.disk_looks() - before_without);
    // Half of these names are in the tree, in some case, and those are asked of the disk as before.
    assert!(looks * 2 < looks_without, "{looks} looks at the disk by the lists, {looks_without} without");
    // A file comes, one goes, one moves, one changes its case, and a directory can no longer be read: at once, when no list is good, and later.
    put(&tree, "a/new.tex", b"");
    both(&mut with, &mut without, "a file came");
    put(&tree, "late/new.tex", b"");
    fs::remove_file(tree.join("b/four.sty")).unwrap();
    fs::rename(tree.join("c/six.tex"), tree.join("a/moved.tex")).unwrap();
    fs::rename(tree.join("e/plain.tex"), tree.join("e/PLAIN.tex")).unwrap();
    both(&mut with, &mut without, "files went and moved");
    settle(&tree);
    both(&mut with, &mut without, "files went and moved, settled");
    set_mode(&tree.join("a/deep"), 0o000);
    both(&mut with, &mut without, "a directory closed");
    settle(&tree);
    both(&mut with, &mut without, "a directory closed, settled");
    set_mode(&tree.join("a/deep"), 0o755);
    settle(&tree);
    both(&mut with, &mut without, "and open again");
    fs::remove_dir_all(tree.join("c")).unwrap();
    both(&mut with, &mut without, "a directory went");
    settle(&tree);
    both(&mut with, &mut without, "a directory went, settled");
    put(&tree, "c/six.tex", b"");
    settle(&tree);
    both(&mut with, &mut without, "and came back");
    // An instance that starts on the tree as it now is agrees with both, where the order of the directories allows: each of the two has moved its own.
    let mut fresh = kpse(&d.join("cnf"), &[]);
    let mut fresh_without = kpse(&d.join("cnf"), &[]);
    fresh_without.set_dir_lists(false);
    both(&mut fresh, &mut fresh_without, "fresh");
}

/// What a miss costs in a tree on the disk: a look at each directory, and within one moment nothing more.
#[cfg(unix)]
#[test]
fn a_miss_looks_at_each_directory_once_in_a_moment() {
    let d = scratch("moment");
    let tree = d.join("tree");
    for i in 0..20 {
        put(&tree, &format!("d{i}/f{i}.tex"), b"");
    }
    let t = String::from_utf8(b(&tree)).unwrap();
    put(&d, "cnf/texmf.cnf", format!("TEXINPUTS = {t}//\ntexmf_casefold_search = 1\n").as_bytes());
    settle(&tree);
    let mut k = kpse(&d.join("cnf"), &[]);
    assert_eq!(s(&k.find_file(b"f7.tex", KPSE_TEX_FORMAT, true)), Some(format!("{t}/d7/f7.tex")));
    let scans = k.dir_scans();
    let before = k.disk_looks();
    assert_eq!(k.find_file(b"nothing.cfg", KPSE_TEX_FORMAT, true), None);
    // The tree's own directory and its 20: C asks each for `nothing.cfg.tex` and `nothing.cfg`, and then for its list, twice.
    let one = k.disk_looks() - before;
    assert_eq!(one, 21);
    k.begin_moment();
    let before = k.disk_looks();
    for name in [&b"nothing.cfg"[..], b"other.cfg", b"third.sty"] {
        assert_eq!(k.find_file(name, KPSE_TEX_FORMAT, true), None);
    }
    assert_eq!(k.disk_looks() - before, 21);
    // A file that comes in the moment is one that came just after it.
    put(&tree, "d3/other.cfg", b"");
    assert_eq!(k.find_file(b"other.cfg", KPSE_TEX_FORMAT, true), None);
    k.end_moment();
    assert_eq!(s(&k.find_file(b"other.cfg", KPSE_TEX_FORMAT, true)), Some(format!("{t}/d3/other.cfg")));
    assert_eq!(k.dir_scans(), scans, "no directory was listed again: the one that changed is too young to keep a list by");
}

/// The lists of a tree's directories are kept for the next run, which then reads no directory, and sees what has changed since all the same.
#[cfg(target_os = "linux")]
#[test]
fn lists_are_kept_for_the_next_run_and_checked_by_it() {
    let d = scratch("store");
    let tree = d.join("tree");
    for i in 0..12 {
        put(&tree, &format!("d{i}/f{i}.tex"), b"");
        put(&tree, &format!("d{i}/sub/g{i}.tex"), b"");
    }
    put(&tree, "d3/na\u{ef}ve.tex", b"");
    let t = String::from_utf8(b(&tree)).unwrap();
    if !fs_keeps_dir_stamps(t.as_bytes()) {
        return;
    }
    put(&d, "cnf/texmf.cnf", format!("TEXINPUTS = {t}//\ntexmf_casefold_search = 1\n").as_bytes());
    let file = d.join("cache/dirs");
    fs::create_dir_all(d.join("cache")).unwrap();
    settle(&tree);
    let run = |store: bool| {
        let mut k = kpse(&d.join("cnf"), &[]);
        if store {
            k.set_dir_store(Some(file.clone()));
            k.set_dir_store_age(30_000_000);
        } else {
            k.set_dir_lists(false);
        }
        let mut out = Vec::new();
        // A name with a directory in it that is there goes to the disk as before, and that reads directories of its own.
        for name in [&b"f7.tex"[..], b"g2.tex", b"F4.TEX", b"nothing.cfg", b"na\xc3\xafve.tex", b"new.tex", b"more/f7.tex", b"f0.tex"] {
            out.push((k.find_file(name, KPSE_TEX_FORMAT, true), k.find_file_generic(name, KPSE_TEX_FORMAT, false, true)));
        }
        let scans = k.dir_scans();
        k.keep_dirs();
        (out, scans)
    };
    let literal = run(false);
    let first = run(true);
    assert_eq!(first.0, literal.0);
    assert!(first.1 > 0 && file.exists(), "the first run reads the tree and leaves its lists");
    let written = fs::metadata(&file).unwrap().modified().unwrap();
    let second = run(true);
    assert_eq!(second.0, literal.0);
    assert_eq!(second.1, 0, "the second reads no directory");
    assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), written, "and has nothing to add");
    // What changes shows: a file comes, one goes, a directory comes.
    put(&tree, "d5/new.tex", b"");
    put(&tree, "d6/sub/new.tex", b"");
    fs::remove_file(tree.join("d0/f0.tex")).unwrap();
    put(&tree, "d1/more/f7.tex", b"");
    let (literal, third) = (run(false), run(true));
    assert_eq!(third.0, literal.0, "at once, when the stamps are young");
    settle(&tree);
    let (literal, fourth) = (run(false), run(true));
    assert_eq!(fourth.0, literal.0, "and later");
    assert!(fourth.1 > 0 && fourth.1 < first.1, "only what changed is read again: {} of {}", fourth.1, first.1);
    let fifth = run(true);
    assert_eq!(fifth.0, literal.0);
    assert_eq!(fifth.1, 0);
    // A damaged file is no file.
    let mut bytes = fs::read(&file).unwrap();
    let at = bytes.len() - 40;
    bytes[at] ^= 0x40;
    fs::write(&file, &bytes).unwrap();
    let sixth = run(true);
    assert_eq!(sixth.0, literal.0);
    fs::write(&file, &bytes[..bytes.len() / 2]).unwrap();
    assert_eq!(run(true).0, literal.0);
    assert_eq!(run(true).1, 0, "and it is written again");
}

/// One instance that asks every directory by its path, and one that asks the disk for every name.
#[cfg(target_os = "linux")]
fn by_path_and_literal(d: &Path) -> (Kpse, Kpse) {
    let mut by_path = kpse(&d.join("cnf"), &[]);
    by_path.set_dir_holding(false);
    let mut literal = kpse(&d.join("cnf"), &[]);
    literal.set_dir_lists(false);
    (by_path, literal)
}

/// A directory that is looked at again and again is held open and asked by its descriptor, and the answers stay those of a search that asks the disk for every
/// name, and of one that asks every directory by its path: while files come and go, a directory is put in another's place, one becomes a symbolic link, the
/// directory a link leads to is renamed away with nothing in the tree changing, and the tree itself, and the directory above it, are replaced.
#[cfg(target_os = "linux")]
#[test]
fn a_held_directory_answers_as_its_path_does() {
    let d = scratch("held");
    let tree = d.join("up/tree");
    for f in ["a/one.tex", "a/deep/three.tex", "a/deep/er/nine.tex", "b/two.tex", "b/sub/four.tex", "c/five.tex", "top.tex"] {
        put(&tree, f, b"x");
    }
    for f in ["out/x/six.tex", "out/x/y/eight.tex", "other-b/other.tex", "other-b/sub/four.tex", "other-c/cc.tex", "tree2/a/one.tex", "tree2/a/deep/ten.tex", "tree2/z/zed.tex", "up2/tree/q/queue.tex", "up2/tree/a/deep/er/eleven.tex"] {
        put(&d, f, b"x");
    }
    std::os::unix::fs::symlink(d.join("out/x"), tree.join("link")).unwrap();
    let t = String::from_utf8(b(&tree)).unwrap();
    put(&d, "cnf/texmf.cnf", format!("TEXINPUTS = {t}/a//;{t}//\ntexmf_casefold_search = 1\n").as_bytes());
    let names: Vec<&[u8]> = vec![
        b"one.tex", b"three.tex", b"nine.tex", b"two.tex", b"four.tex", b"five.tex", b"top.tex", b"six.tex", b"eight.tex", b"other.tex", b"cc.tex", b"zed.tex",
        b"queue.tex", b"new.tex", b"seven.tex", b"nothing.cfg", b"sub/four.tex", b"y/eight.tex", b"ONE.TEX", b"ten.tex", b"eleven.tex", b"twelve.tex",
    ];
    let ask = |k: &mut Kpse| {
        let mut out = Vec::new();
        // Twice: the second look at a directory by its path is the one that opens it.
        for _ in 0..2 {
            for name in &names {
                for must in [false, true] {
                    out.push((String::from_utf8_lossy(name).into_owned(), must, k.find_file_generic(name, KPSE_TEX_FORMAT, must, false), k.find_file_generic(name, KPSE_TEX_FORMAT, must, true)));
                }
            }
        }
        out
    };
    let mut held = kpse(&d.join("cnf"), &[]);
    let mut literal_too = by_path_and_literal(&d);
    let mut all = |held: &mut Kpse, when: &str| {
        let (by_path, literal) = &mut literal_too;
        let (a, p, z) = (ask(held), ask(by_path), ask(literal));
        for ((x, y), w) in a.iter().zip(&p).zip(&z) {
            assert_eq!(x, w, "{when}: held against the disk");
            assert_eq!(y, w, "{when}: by path against the disk");
        }
        (held.held_dirs(), by_path.held_dirs())
    };
    settle(&tree);
    let (n, none) = all(&mut held, "settled");
    // a/deep/, a/deep/er/, b/, b/sub/, c/ and link/y/: every directory that is a plain entry of one that was looked at before it in the same moment.  Not the
    // tree and not a/, each the top of an element, and not link/, which is a symbolic link.
    assert_eq!((n, none), (6, 0), "directories held");
    let before = held.disk_looks();
    assert_eq!(held.find_file(b"nothing.cfg", KPSE_TEX_FORMAT, true), None);
    assert_eq!(held.disk_looks() - before, 9, "and a miss still looks at each directory once");
    put(&tree, "a/deep/new.tex", b"");
    all(&mut held, "a file came");
    settle(&tree);
    all(&mut held, "a file came, settled");
    // a/deep can no longer be searched: a/deep/er, which is held, is no longer reached by its path, though nothing of it has changed.
    set_mode(&tree.join("a/deep"), 0o000);
    all(&mut held, "a directory above a held one closed");
    settle(&tree);
    all(&mut held, "a directory above a held one closed, settled");
    set_mode(&tree.join("a/deep"), 0o755);
    settle(&tree);
    let (n, _) = all(&mut held, "and open again");
    assert_eq!(n, 6, "directories held again");
    // Another directory in b's place, with a subdirectory of the same name.
    fs::rename(tree.join("b"), d.join("b-gone")).unwrap();
    fs::rename(d.join("other-b"), tree.join("b")).unwrap();
    all(&mut held, "a directory was replaced");
    settle(&tree);
    all(&mut held, "a directory was replaced, settled");
    // c becomes a symbolic link to another directory.
    fs::rename(tree.join("c"), d.join("c-gone")).unwrap();
    std::os::unix::fs::symlink(d.join("other-c"), tree.join("c")).unwrap();
    all(&mut held, "a directory became a link");
    settle(&tree);
    all(&mut held, "a directory became a link, settled");
    // What link leads to is renamed away, and another directory takes its path: nothing in the tree has changed, and nothing of link/y, which is held.  A list
    // that is taken for good when it is not hides what has come, so each new directory has a file the old one had not.
    fs::rename(d.join("out"), d.join("out-gone")).unwrap();
    put(&d, "out/x/seven.tex", b"");
    put(&d, "out/x/y/twelve.tex", b"");
    all(&mut held, "a link leads elsewhere");
    settle(&d.join("out/x"));
    all(&mut held, "a link leads elsewhere, settled");
    // Another tree in the tree's place.
    fs::rename(&tree, d.join("tree-gone")).unwrap();
    fs::rename(d.join("tree2"), &tree).unwrap();
    all(&mut held, "the tree was replaced");
    settle(&tree);
    all(&mut held, "the tree was replaced, settled");
    // And another directory in the place of the one above it.
    fs::rename(d.join("up"), d.join("up-gone")).unwrap();
    fs::rename(d.join("up2"), d.join("up")).unwrap();
    all(&mut held, "the directory above was replaced");
    settle(&tree);
    let (n, _) = all(&mut held, "the directory above was replaced, settled");
    // The directories of an element are found once, as in C, so q/ of the newest tree is none of them: a/deep/ and a/deep/er/ are, and they are the new ones.
    assert_eq!(n, 2, "nothing of the old trees is held");
}
