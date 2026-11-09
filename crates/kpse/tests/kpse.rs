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
    assert!(k.take_warnings().iter().any(|w| w.starts_with(b"warning: kpathsea: variable `") && w.ends_with(b"' references itself (eventually).\n")));
}

/// A user's cnf directory before the installation's, as `TEXMFCNF=DIR:` puts it: the run reads the user's, and what the installation's alone says can still be asked, for a
/// format build.
#[test]
fn what_the_installation_alone_says() {
    let d = scratch("installation");
    put(&d, "inst/texmf.cnf", b"TEXINPUTS.testprog = $TEXMFDOTDIR/inst\nTEXINPUTS = generic\nTEXMFDOTDIR = .\n");
    put(&d, "user/texmf.cnf", b"TEXINPUTS.testprog = user\nTEXMFDOTDIR = mine\nONLY = user\n");
    let mut tree = Tree::from_selfautoloc(b"/nonexistent-bin/x");
    tree.builtin_cnf = None;
    tree.cnf_default = Some(b(&d.join("inst")));
    let user_first = [b(&d.join("user")), vec![ENV_SEP]].concat();
    let mut k = Kpse::new(Config { program_name: b"testprog".to_vec(), process_env: false, vars: vec![(b"TEXMFCNF".to_vec(), user_first)], tree: Some(tree), ..Config::default() });
    assert_eq!(s(&k.var_value(b"TEXINPUTS")).as_deref(), Some("user"));
    assert_eq!(s(&k.var_value(b"ONLY")).as_deref(), Some("user"));
    let inst = k.installation_cnf_values(&[b"TEXINPUTS", b"TEXMFDOTDIR", b"ONLY"], b"testprog");
    assert_eq!(inst, [Some(b"$TEXMFDOTDIR/inst".to_vec()), Some(b".".to_vec()), None], "as written, for the program's name");
    assert_eq!(k.installation_cnf_values(&[b"TEXINPUTS"], b"other"), [Some(b"generic".to_vec())]);
    // The run's own tables are as they were.
    assert_eq!(s(&k.var_value(b"TEXMFDOTDIR")).as_deref(), Some("mine"));
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
    let _ = k.take_warnings();
    assert_eq!(k.var_expand(b"a$"), b"a$");
    assert_eq!(k.var_expand(b"$:"), b"$:");
    // As C's `WARNING` prints them, with `%c` of the zero that ends the string.
    assert_eq!(
        k.take_warnings(),
        [&b"warning: kpathsea: a$: Unrecognized variable construct `$\0'.\n"[..], b"warning: kpathsea: $:: Unrecognized variable construct `$:'.\n"]
    );
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
    // What C prints with `fprintf`, each message its own line, not a `WARNING`.
    let w = k.take_warnings();
    assert_eq!(w.len(), 3, "{w:?}");
    assert_eq!(w[0], b"test: Bad value (-3) in environment or texmf.cnf for NEG, keeping 7.\n");
}

#[test]
fn no_cnf_means_no_path_unless_a_tree_says_otherwise() {
    let d = scratch("nocnf");
    put(&d, "here.tex", b"");
    let mut k = kpse(&d, &[]);
    assert_eq!(k.format_path(KPSE_TEX_FORMAT).unwrap(), b"/nonesuch", "as kpsewhich with TEXMFCNF pointing at nothing");
    assert!(k.take_warnings().iter().any(|w| w.starts_with(b"warning: kpathsea: configuration file texmf.cnf not found") && w.ends_with(b".\n")));
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
    // Each as C's `WARNING` prints it, a line of its own.
    assert!(all.iter().all(|m| m.starts_with("warning: ") && m.ends_with(".\n") && m.matches('\n').count() == 1), "{all:?}");
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
    assert!(k.take_warnings().iter().any(|w| w.starts_with(b"warning: kpathsea: ") && w.ends_with(b": No usable entries in ls-R.\n")));
}
