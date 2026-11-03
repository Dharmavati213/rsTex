//! Finding the TeX tree (DESIGN.md section 12).  kpathsea has this easy: it lives among the binaries,
//! and derives `SELFAUTOLOC` and friends from its own location.  rstex can be anywhere.  So, in order:
//!
//! 1. `RSTEX_TEXMF`: a list of texmf trees, separated like any search path.  The first is the main
//!    one, named as `$TEXMFDIST`; the others are searched before it, as auxiliary trees.
//! 2. An answer cached in the rstex cache directory, if it still holds.
//! 3. `kpsewhich` (TeX Live, MacTeX): it knows its `SELFAUTO*` values and its compile-time cnf path.
//! 4. `initexmf --report` (MiKTeX): the installation roots.  These have no `texmf.cnf` and no `ls-R`.
//! 5. Well-known locations.
//!
//! The programs are only asked where the data is, once; they never compile anything.
use crate::names::*;
use crate::sys;

/// Where a [`Tree`] came from, for messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeSource {
    /// Given by the caller.
    Explicit,
    /// `RSTEX_TEXMF`.
    Env,
    /// `kpsewhich`, asked now or earlier and cached.
    Kpsewhich,
    /// `initexmf --report`, asked now or earlier and cached.
    Miktex,
    /// A well-known location.
    WellKnown,
    /// Nothing: no tree on this machine.  Only the current directory is searched.
    None,
}

/// What kpathsea would know from where its binary lives, plus what stands in for a missing `texmf.cnf`.
#[derive(Clone, Debug)]
pub struct Tree {
    pub source: TreeSource,
    /// `SELFAUTOLOC`, `SELFAUTODIR`, `SELFAUTOPARENT`, `SELFAUTOGRANDPARENT`.
    pub selfauto: [Vec<u8>; 4],
    /// Replaces the compile-time `DEFAULT_TEXMFCNF`, when the tree's owner knows better.
    pub cnf_default: Option<Vec<u8>>,
    /// Settings that act like environment variables.
    pub vars: Vec<(Vec<u8>, Vec<u8>)>,
    /// A cnf text to use if no `texmf.cnf` is found and `TEXMFCNF` is not set.
    pub builtin_cnf: Option<Vec<u8>>,
    /// Trees with no `ls-R`, indexed by walking them.
    pub walk_roots: Vec<Vec<u8>>,
}

impl Tree {
    /// A tree given as the `SELFAUTOLOC` directory, with the rest derived as progname.c does.
    pub fn from_selfautoloc(loc: &[u8]) -> Tree {
        let dir = xdirname(loc);
        let parent = xdirname(&dir);
        let grandparent = xdirname(&parent);
        Tree {
            source: TreeSource::Explicit,
            selfauto: [loc.to_vec(), fix_selfdir(dir), fix_selfdir(parent), fix_selfdir(grandparent)],
            cnf_default: None,
            vars: Vec::new(),
            builtin_cnf: Some(builtin_cnf(&[])),
            walk_roots: Vec::new(),
        }
    }
}

/// `fix_selfdir`: on Windows `SELFAUTO*` must not be just `/`, or `$SELFAUTODIR/x` looks like a UNC name.
fn fix_selfdir(dir: Vec<u8>) -> Vec<u8> {
    if DOSISH && dir == b"/" { Vec::new() } else { dir }
}

/// A short stand-in for `texmf.cnf`: the entries of the TeX Live file (Part 1 and Part 3) that the
/// engine reads, used if there is no file at all.  `trees` is what `$TEXMF` is; with no tree, only `.`
/// is searched.
pub fn builtin_cnf(trees: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    if trees.is_empty() {
        out.extend_from_slice(b"TEXMF = /nonesuch\nTEXMFDBS = /nonesuch\n");
    } else {
        let mut list = Vec::new();
        for t in trees {
            if !list.is_empty() {
                list.push(b',');
            }
            list.extend_from_slice(t);
        }
        for v in [&b"TEXMF"[..], b"TEXMFDBS"] {
            out.extend_from_slice(v);
            out.extend_from_slice(b" = {");
            out.extend_from_slice(&list);
            out.extend_from_slice(b"}\n");
        }
    }
    out.extend_from_slice(BUILTIN_CNF_REST);
    out
}

const BUILTIN_CNF_REST: &[u8] = b"\
TEXMFDOTDIR = .
WEB2C = $TEXMF/web2c
TEXINPUTS.tex = $TEXMFDOTDIR;$TEXMF/tex/{plain,generic,latex,}//
TEXINPUTS.latex = $TEXMFDOTDIR;$TEXMF/tex/{latex,generic,}//
TEXINPUTS.pdflatex = $TEXMFDOTDIR;$TEXMF/tex/{latex,generic,}//
TEXINPUTS.etex = $TEXMFDOTDIR;$TEXMF/tex/{plain,generic,latex,}//
TEXINPUTS.pdfetex = $TEXMFDOTDIR;$TEXMF/tex/{plain,generic,latex,}//
TEXINPUTS.pdftex = $TEXMFDOTDIR;$TEXMF/tex/{plain,generic,latex,}//
TEXINPUTS = $TEXMFDOTDIR;$TEXMF/tex/{$progname,generic,latex,}//
TEXFORMATS = $TEXMFDOTDIR;$TEXMF/web2c{/$engine,}
TEXPOOL = $TEXMFDOTDIR;$TEXMF/web2c
VFFONTS = $TEXMFDOTDIR;$TEXMF/fonts/vf//
TFMFONTS = $TEXMFDOTDIR;$TEXMF/fonts/tfm//
TEXFONTMAPS = $TEXMFDOTDIR;$TEXMF/fonts/map/{$progname,pdftex,dvips,}//
BIBINPUTS = $TEXMFDOTDIR;$TEXMF/bibtex/bib//
BSTINPUTS = $TEXMFDOTDIR;$TEXMF/bibtex/{bst,csf}//
TEXPSHEADERS = $TEXMFDOTDIR;$TEXMF/{dvips,fonts/{enc,type1,type42,type3}}//
T1FONTS = $TEXMFDOTDIR;$TEXMF/fonts/type1//
AFMFONTS = $TEXMFDOTDIR;$TEXMF/fonts/afm//
TTFONTS = $TEXMFDOTDIR;$TEXMF/fonts/{truetype,opentype}//
OPENTYPEFONTS = $TEXMFDOTDIR;$TEXMF/fonts/{opentype,truetype}//
ENCFONTS = $TEXMFDOTDIR;$TEXMF/fonts/enc//
CMAPFONTS = $TEXMFDOTDIR;$TEXMF/fonts/cmap//
SFDFONTS = $TEXMFDOTDIR;$TEXMF/fonts/sfd//
PDFTEXCONFIG = $TEXMFDOTDIR;$TEXMF/pdftex/{$progname,}//
MISCFONTS = $TEXMFDOTDIR;$TEXMF/fonts/misc//
TEXDOCS = $TEXMFDOTDIR;$TEXMF/doc//
TEXSOURCES = $TEXMFDOTDIR;$TEXMF/source//
try_std_extension_first = t
texmf_casefold_search = 1
openout_any = p
openin_any = a
parse_first_line = t
log_openout = 0
main_memory = 5000000
extra_mem_top = 0
extra_mem_bot = 0
font_mem_size = 8000000
font_max = 9000
hash_extra = 600000
pool_size = 6250000
string_vacancies = 90000
max_strings = 500000
pool_free = 47500
strings_free = 100
buf_size = 200000
trie_size = 1100000
hyph_size = 8191
nest_size = 1000
max_in_open = 15
param_size = 20000
save_size = 200000
stack_size = 10000
dvi_buf_size = 16384
error_line = 79
half_error_line = 50
max_print_line = 79
";

/// A main tree directory, like `/usr/local/texlive/2026/texmf-dist`, named by `RSTEX_TEXMF` or found
/// in a well-known place.  `dir` need not hold a `web2c/texmf.cnf`.
pub fn from_tree_dirs(dirs: &[Vec<u8>], source: TreeSource) -> Tree {
    let dir = &dirs[0];
    let parent = xdirname(dir);
    let bin = [&parent[..], b"/bin"].concat();
    let loc = [&bin[..], b"/rstex"].concat();
    let grandparent = xdirname(&parent);
    let has_cnf = dirs.iter().any(|d| sys::readable(&[&d[..], b"/web2c/texmf.cnf"].concat()) == sys::Readable::Yes);
    let mut vars = Vec::new();
    let mut builtin = None;
    let mut walk_roots = Vec::new();
    let mut cnf_default = None;
    if has_cnf {
        // The file's own idea of the root is about the binaries it came with, so state the facts.
        vars.push((b"TEXMFDIST".to_vec(), dir.clone()));
        vars.push((b"TEXMFROOT".to_vec(), parent.clone()));
        if dirs.len() > 1 {
            let mut aux = Vec::new();
            for d in &dirs[1..] {
                aux.extend_from_slice(d);
                aux.push(b',');
            }
            vars.push((b"TEXMFAUXTREES".to_vec(), aux));
        }
        // texmf.cnf next to the tree holds local overrides, as in TeX Live.
        let mut c = parent.clone();
        for d in dirs {
            c.push(ENV_SEP);
            c.extend_from_slice(d);
            c.extend_from_slice(b"/web2c");
        }
        cnf_default = Some(c);
    } else {
        builtin = Some(builtin_cnf(dirs));
        // Without an ls-R in them, they would be scanned on every first lookup in each subdirectory.
        for d in dirs {
            let has_ls_r = sys::readable(&[&d[..], b"/ls-R"].concat()) == sys::Readable::Yes;
            if !has_ls_r {
                walk_roots.push(d.clone());
            }
        }
    }
    Tree {
        source,
        selfauto: [loc, fix_selfdir(bin), fix_selfdir(parent), fix_selfdir(grandparent)],
        cnf_default,
        vars,
        builtin_cnf: builtin,
        walk_roots,
    }
}

/// Parses `kpsewhich --expand-var=... --show-path=cnf`: four `|`-separated directories, then the path.
/// A `kpsewhich` that is MiKTeX's or otherwise different gives something else, and `None`.
pub fn parse_kpsewhich_answer(out: &[u8]) -> Option<Tree> {
    let mut lines = out.split(|&c| c == b'\n').map(|l| l.strip_suffix(b"\r").unwrap_or(l));
    let vars = lines.next()?;
    let cnf = lines.next()?;
    let fields: Vec<&[u8]> = vars.split(|&c| c == b'|').collect();
    if fields.len() != 4 || !absolute_p(fields[0], false) || fields[0].contains(&b'$') || cnf.is_empty() {
        return None;
    }
    Some(Tree {
        source: TreeSource::Kpsewhich,
        selfauto: [fields[0].to_vec(), fields[1].to_vec(), fields[2].to_vec(), fields[3].to_vec()],
        cnf_default: Some(cnf.to_vec()),
        vars: Vec::new(),
        builtin_cnf: None,
        walk_roots: Vec::new(),
    })
}

/// Parses `initexmf --report`: `Name: value` lines, of which the roots matter.  `Root0`, `Root1`, ...
/// when present, else the installation and data directories.
pub fn parse_miktex_report(out: &[u8]) -> Vec<Vec<u8>> {
    let mut roots = Vec::new();
    let mut others = Vec::new();
    for line in out.split(|&c| c == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Some(colon) = line.iter().position(|&c| c == b':') else { continue };
        let key = &line[..colon];
        let mut value = &line[colon + 1..];
        while value.first().is_some_and(|&c| is_space(c)) {
            value = &value[1..];
        }
        if value.is_empty() {
            continue;
        }
        let slash: Vec<u8> = value.iter().map(|&c| if c == b'\\' { b'/' } else { c }).collect();
        if key.starts_with(b"Root") && key[4..].iter().all(u8::is_ascii_digit) && key.len() > 4 {
            roots.push(slash);
        } else if matches!(key, b"UserInstall" | b"UserConfig" | b"UserData" | b"CommonInstall" | b"CommonConfig" | b"CommonData") {
            others.push(slash);
        }
    }
    let mut list = if roots.is_empty() { others } else { roots };
    let mut seen: Vec<Vec<u8>> = Vec::new();
    list.retain(|d| {
        if seen.contains(d) {
            false
        } else {
            seen.push(d.clone());
            true
        }
    });
    list
}

const CACHE_MAGIC: &[u8] = b"rstex-kpse-tree 1";

/// Serialises the answer of a program, keyed to `PATH` and to the program's identity.
fn cache_text(tree: &Tree, prog: &std::path::Path, stamp: (u64, u64), roots: &[Vec<u8>]) -> Vec<u8> {
    let mut out = CACHE_MAGIC.to_vec();
    out.push(b'\n');
    let mut put = |k: &str, v: &[u8]| {
        out.extend_from_slice(k.as_bytes());
        out.push(b'=');
        out.extend_from_slice(v);
        out.push(b'\n');
    };
    put("path", &sys::path_var());
    put("prog", &sys::os_bytes(prog.as_os_str()));
    put("stamp", format!("{}:{}", stamp.0, stamp.1).as_bytes());
    match tree.source {
        TreeSource::Miktex => {
            for r in roots {
                put("root", r);
            }
        }
        _ => {
            put("loc", &tree.selfauto[0]);
            put("dir", &tree.selfauto[1]);
            put("parent", &tree.selfauto[2]);
            put("grandparent", &tree.selfauto[3]);
            if let Some(c) = &tree.cnf_default {
                put("cnf", c);
            }
        }
    }
    out
}

/// Reads the cache, if it holds for the present `PATH` and the program is unchanged.
fn cache_read(text: &[u8]) -> Option<Tree> {
    let mut lines = text.split(|&c| c == b'\n');
    if lines.next()? != CACHE_MAGIC {
        return None;
    }
    let mut prog = None;
    let mut stamp = None;
    let mut path = None;
    let (mut loc, mut dir, mut parent, mut grandparent, mut cnf) = (None, None, None, None, None);
    let mut roots = Vec::new();
    for l in lines {
        let Some(eq) = l.iter().position(|&c| c == b'=') else { continue };
        let (k, v) = (&l[..eq], &l[eq + 1..]);
        match k {
            b"path" => path = Some(v),
            b"prog" => prog = Some(v),
            b"stamp" => stamp = Some(v),
            b"loc" => loc = Some(v),
            b"dir" => dir = Some(v),
            b"parent" => parent = Some(v),
            b"grandparent" => grandparent = Some(v),
            b"cnf" => cnf = Some(v),
            b"root" => roots.push(v.to_vec()),
            _ => {}
        }
    }
    if path? != &sys::path_var()[..] {
        return None;
    }
    let prog = sys::path_of(prog?);
    let (size, mtime) = sys::stamp(&prog)?;
    if stamp? != format!("{size}:{mtime}").as_bytes() {
        return None;
    }
    if !roots.is_empty() {
        return Some(miktex_tree(&roots));
    }
    Some(Tree {
        source: TreeSource::Kpsewhich,
        selfauto: [loc?.to_vec(), dir?.to_vec(), parent?.to_vec(), grandparent?.to_vec()],
        cnf_default: cnf.map(<[u8]>::to_vec),
        vars: Vec::new(),
        builtin_cnf: None,
        walk_roots: Vec::new(),
    })
}

fn miktex_tree(roots: &[Vec<u8>]) -> Tree {
    let mut t = from_tree_dirs(roots, TreeSource::Miktex);
    t.source = TreeSource::Miktex;
    t
}

fn cache_file() -> Option<std::path::PathBuf> {
    Some(sys::cache_dir()?.join("kpse-tree"))
}

/// The arguments that make `kpsewhich` say all it knows at once.
const KPSEWHICH_ARGS: [&str; 2] = ["--expand-var=$SELFAUTOLOC|$SELFAUTODIR|$SELFAUTOPARENT|$SELFAUTOGRANDPARENT", "--show-path=cnf"];

/// Finds the tree, as the module documentation describes.  `env` is the environment to consult
/// for `RSTEX_TEXMF`; the programs and the cache use the process's.
pub fn discover(env: &dyn Fn(&[u8]) -> Option<Vec<u8>>) -> Tree {
    if let Some(v) = env(b"RSTEX_TEXMF").filter(|v| !v.is_empty()) {
        let dirs: Vec<Vec<u8>> = path_elements(&v).into_iter().filter(|e| !e.is_empty()).map(<[u8]>::to_vec).collect();
        if !dirs.is_empty() {
            return from_tree_dirs(&dirs, TreeSource::Env);
        }
    }
    if let Some(file) = cache_file() {
        if let Ok(text) = std::fs::read(&file) {
            if let Some(t) = cache_read(&text) {
                return t;
            }
        }
    }
    if let Some(prog) = sys::find_on_path("kpsewhich").or_else(sys::known_kpsewhich) {
        if let Some(t) = sys::capture(&prog, &KPSEWHICH_ARGS).and_then(|o| parse_kpsewhich_answer(&o)) {
            if let (Some(file), Some(stamp)) = (cache_file(), sys::stamp(&prog)) {
                sys::publish_path(&file, &cache_text(&t, &prog, stamp, &[]));
            }
            return t;
        }
    }
    if let Some(prog) = sys::find_on_path("initexmf") {
        if let Some(out) = sys::capture(&prog, &["--report"]) {
            let roots: Vec<Vec<u8>> = parse_miktex_report(&out).into_iter().filter(|d| sys::is_dir(d)).collect();
            if !roots.is_empty() {
                let t = miktex_tree(&roots);
                if let (Some(file), Some(stamp)) = (cache_file(), sys::stamp(&prog)) {
                    sys::publish_path(&file, &cache_text(&t, &prog, stamp, &roots));
                }
                return t;
            }
        }
    }
    for d in sys::well_known_trees() {
        if sys::readable(&[&d[..], b"/web2c/texmf.cnf"].concat()) == sys::Readable::Yes {
            return from_tree_dirs(&[d], TreeSource::WellKnown);
        }
    }
    let miktex: Vec<Vec<u8>> = sys::well_known_miktex().into_iter().filter(|d| sys::is_dir(d)).collect();
    if !miktex.is_empty() {
        return miktex_tree(&miktex);
    }
    Tree {
        source: TreeSource::None,
        selfauto: [b".".to_vec(), b".".to_vec(), b".".to_vec(), b".".to_vec()],
        cnf_default: None,
        vars: Vec::new(),
        builtin_cnf: Some(builtin_cnf(&[])),
        walk_roots: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kpsewhich_answer() {
        let t = parse_kpsewhich_answer(b"/usr/bin|/usr|/|/\n/usr/bin:/usr/share/texmf-dist/web2c\n").unwrap();
        assert_eq!(t.selfauto[0], b"/usr/bin");
        assert_eq!(t.selfauto[3], b"/");
        assert_eq!(t.cnf_default.as_deref(), Some(&b"/usr/bin:/usr/share/texmf-dist/web2c"[..]));
        assert_eq!(t.source, TreeSource::Kpsewhich);
        // A kpsewhich that does not know the variables, or says nothing, is no answer.
        assert!(parse_kpsewhich_answer(b"$SELFAUTOLOC|$SELFAUTODIR|$SELFAUTOPARENT|$SELFAUTOGRANDPARENT\n/x\n").is_none());
        assert!(parse_kpsewhich_answer(b"").is_none());
        assert!(parse_kpsewhich_answer(b"/usr/bin|/usr|/|/\n").is_none());
        assert!(parse_kpsewhich_answer(b"usage: kpsewhich\n").is_none());
    }

    #[test]
    fn miktex_report() {
        let report = b"MiKTeX: 24.1\r\nOS: Windows 11\r\nSharedSetup: no\r\nRoot0: C:\\Users\\u\\AppData\\Roaming\\MiKTeX\r\nRoot1: C:\\Users\\u\\AppData\\Local\\MiKTeX\r\nRoot2: C:\\Users\\u\\AppData\\Local\\Programs\\MiKTeX\r\nUserInstall: C:\\Users\\u\\AppData\\Local\\Programs\\MiKTeX\r\n";
        let r = parse_miktex_report(report);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0], b"C:/Users/u/AppData/Roaming/MiKTeX");
        // Without RootN, the install and data directories, once each.
        let r = parse_miktex_report(b"UserInstall: /a\nUserConfig: /b\nUserData: /b\nCommonInstall: /c\nRootPrivileges: no\n");
        assert_eq!(r, vec![b"/a".to_vec(), b"/b".to_vec(), b"/c".to_vec()]);
    }

    #[test]
    fn builtin_cnf_names_the_trees() {
        let text = builtin_cnf(&[b"/a".to_vec(), b"/b".to_vec()]);
        assert!(text.starts_with(b"TEXMF = {/a,/b}\nTEXMFDBS = {/a,/b}\n"));
        let none = builtin_cnf(&[]);
        assert!(none.starts_with(b"TEXMF = /nonesuch\n"));
    }

    #[test]
    fn cached_answer_round_trips_only_for_the_same_program() {
        let exe = std::env::current_exe().unwrap();
        let st = sys::stamp(&exe).unwrap();
        let t = parse_kpsewhich_answer(b"/x/bin/arch|/x/bin|/x|/\n/x/texmf-dist/web2c\n").unwrap();
        let text = cache_text(&t, &exe, st, &[]);
        let back = cache_read(&text).unwrap();
        assert_eq!(back.selfauto, t.selfauto);
        assert_eq!(back.cnf_default, t.cnf_default);
        // A replaced binary or another PATH makes it stale.
        let stale = cache_text(&t, &exe, (st.0 + 1, st.1), &[]);
        assert!(cache_read(&stale).is_none());
        let other: Vec<u8> = text.iter().copied().map(|c| if c == b'/' { b'_' } else { c }).collect();
        assert!(cache_read(&other).is_none());
        assert!(cache_read(b"something else\n").is_none());
    }
}
