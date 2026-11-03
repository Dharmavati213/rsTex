//! A subset of kpathsea: `texmf.cnf`, `ls-R`, search paths, `kpse_find_file` (DESIGN.md sections 6, 12).
//!
//! The specification is `refs/pinned-c/texk/kpathsea/`, and the oracle is `kpsewhich` itself:
//! `tests/oracle/kpse_compare.py` compares the two over tens of thousands of names.  The modules map onto
//! the C files: `cnf` (cnf.c), `expand` (variable.c, expand.c, tilde.c), `db` (db.c), `search`
//! (pathsearch.c, elt-dirs.c, tex-file.c), `fontmap` (fontmap.c), `format` (the table in tex-file.c),
//! `names` (absolute.c and the small string helpers), `glyph` (tex-glyph.c, tex-make.c, magstep.c).  `tree` is new: it finds the
//! TeX tree.  `sys` holds what depends on the operating system.
//!
//! # Using it
//!
//! Most callers use the free functions, which share one lazily built [`Kpse`] behind a lock, as C shares
//! `kpse_def`.  Tests and tools can build their own [`Kpse`].
//!
//! ```no_run
//! rstex_kpse::set_program_name(b"pdflatex"); // kpse_reset_program_name
//! let article = rstex_kpse::find_file(b"article.cls", rstex_kpse::KPSE_TEX_FORMAT, true);
//! ```
//!
//! Before the first lookup, the CLI should set what web2c's `maininit` sets: the program name, which is
//! the dump name unless `-progname` says otherwise, because `TEXINPUTS.pdflatex` and the like depend on
//! it; and `TEXMF_OUTPUT_DIRECTORY` with [`set_var`] when `-output-directory` is given.
//!
//! # What costs what
//!
//! `texmf.cnf` is read at the first lookup of a variable, and `ls-R` when a search first reaches an
//! element a database could answer.  So a file found in `.` reads no `ls-R`.  Reading it is one read and
//! one scan.  A search answered by `ls-R` costs no directory scan: `!!` elements never touch the disk, and
//! other elements that a database covers do so only if `must_exist` and the name is not in it.  Elements
//! outside any database are expanded once and cached, as in C, and [`Kpse::dir_scans`] counts all of it.
//! One cost stays, as in C: under `texmf_casefold_search`, which TeX Live sets, a miss in an element
//! on the disk such as `.` lists that directory, since a file differing in case there comes first.
//!
//! # Differences from C
//!
//! Listed where they occur, and in short: of the `mktex*` only `mktexpk` runs (`glyph`);
//! `TEXMFLOG` is not written; messages that C prints at once go to [`Kpse::take_warnings`] and files it
//! records to [`Kpse::take_recorded`]; `~user` reads `/etc/passwd` instead of calling `getpwnam`; the reading
//! of the databases happens later; on macOS a lookup in them falls back to ignoring case, see
//! [`Config::db_casefold`]; `FOO = $FOO/x` used in a path stops expanding, where kpsewhich does not return.
#![forbid(unsafe_code)]

mod cnf;
mod db;
mod expand;
mod fontmap;
mod format;
mod glyph;
pub mod names;
mod search;
mod sys;
mod tree;

pub use expand::cnf_p;
pub use format::*;
pub use glyph::{
    GlyphFile, GlyphSource, KPSE_SRC_CLIENT_CNF, KPSE_SRC_CMDLINE, KPSE_SRC_COMPILE, KPSE_SRC_ENV, KPSE_SRC_TEXMF_CNF, KPSE_SRC_X, bitmap_tolerance,
    find_glyph, magstep_fix, maketex_format, maketex_option, set_make_tex_discard_errors, set_program_enabled,
};
pub use tree::{Tree, TreeSource, builtin_cnf, discover, from_tree_dirs, parse_kpsewhich_answer, parse_miktex_report};

use names::*;
use search::{DirElt, Elt};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A format's final path: `format_info[f].path`, and split up.
pub(crate) struct FormatState {
    pub(crate) path: Vec<u8>,
    pub(crate) elts: Vec<Elt>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DbState {
    NotLoaded,
    /// Reading the databases, so searches must not look in them yet: `kpse->db.buckets == NULL`.
    Loading,
    Ready,
}

/// How to build a [`Kpse`].
#[derive(Clone, Debug)]
pub struct Config {
    /// `kpse->program_name`: selects `VAR.prog` entries.
    pub program_name: Vec<u8>,
    /// `kpse->invocation_name`, for messages.
    pub invocation_name: Vec<u8>,
    /// `engine`, as `xputenv ("engine", TEXMFENGINENAME)`.  It selects where formats are.
    pub engine: Option<Vec<u8>>,
    /// Whether the process's environment counts.  Tests of a synthetic tree turn it off.
    pub process_env: bool,
    /// Settings as with `xputenv`, which win over the real environment.
    pub vars: Vec<(Vec<u8>, Vec<u8>)>,
    /// The tree; `None` finds it, see [`discover`].
    pub tree: Option<Tree>,
    /// Whether a lookup in the databases falls back to a case-insensitive match of the last component,
    /// if `texmf_casefold_search` allows it (DESIGN.md section 12).  kpathsea does this only on the disk, so
    /// it is for macOS, whose file system is usually case-insensitive while `ls-R` is not.  Windows
    /// compares in one case throughout.
    pub db_casefold: bool,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            program_name: b"pdftex".to_vec(),
            invocation_name: b"rstex".to_vec(),
            engine: Some(b"pdftex".to_vec()),
            process_env: true,
            vars: Vec::new(),
            tree: None,
            db_casefold: cfg!(target_os = "macos"),
        }
    }
}

/// `kpathsea_instance`.
pub struct Kpse {
    pub(crate) program_name: Vec<u8>,
    invocation_name: Vec<u8>,
    process_env: bool,
    /// What C puts in the environment with `xputenv`: `SELFAUTO*`, `progname`, `engine` and the like.
    local: Vec<(Vec<u8>, Vec<u8>)>,
    pub(crate) tree: Tree,
    pub(crate) cnf: HashMap<Vec<u8>, Vec<u8>>,
    pub(crate) cnf_read: bool,
    pub(crate) doing_cnf_init: bool,
    pub(crate) followup_search: bool,
    pub(crate) expansions: Vec<(Vec<u8>, bool)>,
    pub(crate) formats: Vec<Option<Arc<FormatState>>>,
    pub(crate) override_paths: Vec<Option<Vec<u8>>>,
    pub(crate) db: db::Db,
    pub(crate) db_state: DbState,
    pub(crate) alias: Option<HashMap<Vec<u8>, Vec<Vec<u8>>>>,
    pub(crate) elt_cache: HashMap<Vec<u8>, usize>,
    pub(crate) dir_lists: Vec<Vec<DirElt>>,
    pub(crate) map: Option<HashMap<Vec<u8>, Vec<Vec<u8>>>>,
    pub(crate) casefold_memo: Option<bool>,
    pub(crate) db_casefold: bool,
    pub(crate) recorded: Vec<Vec<u8>>,
    pub(crate) dir_scans: u64,
    warnings: Vec<Vec<u8>>,
    /// `program`, `program_enabled_p` and `program_enable_level` of each format.
    pub(crate) make: Vec<glyph::MakeInfo>,
    /// `kpse->make_tex_discard_errors`.
    pub(crate) make_tex_discard_errors: bool,
    /// `kpse->missfont`.
    pub(crate) missfont: Option<std::fs::File>,
    /// `kpse->fallback_resolutions`, until the zero that ends it.
    pub(crate) fallback_resolutions: Option<Vec<u32>>,
}

impl Kpse {
    /// A new instance.  It reads nothing yet, unless it must find the tree.
    pub fn new(mut cfg: Config) -> Kpse {
        #[cfg(feature = "prof")]
        let tree_time = prof::begin(&prof::TREE);
        let tree = match cfg.tree.take() {
            Some(t) => t,
            None if cfg.process_env => tree::discover(&|name| {
                cfg.vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()).or_else(|| sys::getenv(name))
            }),
            None => {
                tree::discover(&|name| cfg.vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()))
            }
        };
        #[cfg(feature = "prof")]
        drop(tree_time);
        let mut local = Vec::with_capacity(8 + cfg.vars.len());
        local.push((b"SELFAUTOLOC".to_vec(), tree.selfauto[0].clone()));
        local.push((b"SELFAUTODIR".to_vec(), tree.selfauto[1].clone()));
        local.push((b"SELFAUTOPARENT".to_vec(), tree.selfauto[2].clone()));
        local.push((b"SELFAUTOGRANDPARENT".to_vec(), tree.selfauto[3].clone()));
        local.push((b"progname".to_vec(), cfg.program_name.clone()));
        if let Some(e) = &cfg.engine {
            local.push((b"engine".to_vec(), e.clone()));
        }
        local.extend(tree.vars.iter().cloned());
        local.append(&mut cfg.vars);
        Kpse {
            program_name: cfg.program_name,
            invocation_name: cfg.invocation_name,
            process_env: cfg.process_env,
            local,
            tree,
            cnf: HashMap::new(),
            cnf_read: false,
            doing_cnf_init: false,
            followup_search: false,
            expansions: Vec::new(),
            formats: vec![None; KPSE_LAST_FORMAT as usize],
            override_paths: vec![None; KPSE_LAST_FORMAT as usize],
            db: db::Db::default(),
            db_state: DbState::NotLoaded,
            alias: None,
            elt_cache: HashMap::new(),
            dir_lists: Vec::new(),
            map: None,
            casefold_memo: None,
            db_casefold: cfg.db_casefold,
            recorded: Vec::new(),
            dir_scans: 0,
            warnings: Vec::new(),
            make: vec![glyph::MakeInfo::default(); KPSE_LAST_FORMAT as usize],
            make_tex_discard_errors: false,
            missfont: None,
            fallback_resolutions: None,
        }
    }

    /// How many directories have been opened to list them, whether that succeeded or not.  A search
    /// that `ls-R` answers adds none.  For tests and for measuring.
    pub fn dir_scans(&self) -> u64 {
        self.dir_scans
    }

    /// `getenv`, after what `xputenv` set.
    pub(crate) fn getenv(&self, name: &[u8]) -> Option<Vec<u8>> {
        if let Some((_, v)) = self.local.iter().rev().find(|(k, _)| file_str_eq(k, name)) {
            return Some(v.clone());
        }
        if self.process_env { sys::getenv(name) } else { None }
    }

    /// Records what C's `WARNING` prints.
    pub(crate) fn warn(&mut self, msg: &[u8]) {
        if self.warnings.len() < 64 {
            self.warnings.push(msg.to_vec());
        }
    }

    /// The messages so far, for the caller to print: this crate leaves stderr alone except in
    /// [`Kpse::out_name_ok`], where the message is part of the behaviour.
    pub fn take_warnings(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.warnings)
    }

    /// The cnf and font map files read since the last call: what C reports through `kpse->record_input`,
    /// for the `-recorder` file.  A leaf crate does not call back, so the caller asks.
    pub fn take_recorded(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.recorded)
    }

    /// Makes sure the cnf files were read.
    pub(crate) fn ensure_cnf(&mut self) {
        let _ = self.cnf_get(b"");
    }

    /// `xputenv`, as far as this crate can see: `var` now has this value here, whatever the environment
    /// says.  Answers that depended on it are forgotten.
    pub fn set_var(&mut self, var: &[u8], value: &[u8]) {
        match self.local.iter_mut().find(|(k, _)| k == var) {
            Some(e) => e.1 = value.to_vec(),
            None => self.local.push((var.to_vec(), value.to_vec())),
        }
        self.casefold_memo = None;
    }

    /// What C has in the environment through `xputenv`: `SELFAUTO*`, `progname`, `engine`,
    /// `TEXMF_OUTPUT_DIRECTORY` and whatever else [`Kpse::set_var`] set.  A child of `\write18` inherits
    /// them in C, so whoever starts a child passes these to it, or `kpsewhich` there sees another world.
    pub fn child_env(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.local.clone()
    }

    /// `kpathsea_cnf_line_env_progname`: the `-cnf-line` option.
    pub fn cnf_line_env_progname(&mut self, line: &[u8]) {
        if let Err(msg) = self.do_line(line, true) {
            self.warn(&[b"command line (kpathsea): ", msg.as_bytes(), b" in argument: ", line].concat());
        }
    }

    /// The program name, as `kpathsea_reset_program_name`: paths may differ under the new name.  Those of
    /// the cnf files and the databases stay, so nothing is read twice.
    pub fn set_program_name(&mut self, progname: &[u8]) {
        if self.program_name == progname {
            return;
        }
        self.program_name = progname.to_vec();
        self.set_var(b"progname", progname);
        for (i, f) in self.formats.iter_mut().enumerate() {
            if i as i32 != KPSE_CNF_FORMAT && i as i32 != KPSE_DB_FORMAT {
                *f = None;
            }
        }
    }

    /// The name printed in messages.
    pub fn set_invocation_name(&mut self, name: &[u8]) {
        self.invocation_name = name.to_vec();
    }

    /// `kpse_invocation_name`: what `pdftex_warn` and `pdftex_fail` print.
    pub fn invocation_name(&self) -> &[u8] {
        &self.invocation_name
    }

    /// The program name in force.
    pub fn program_name(&self) -> &[u8] {
        &self.program_name
    }

    /// `kpathsea_init_prog`: `<PREFIX>MAKEPK` names a `mktexpk` of its own, `<PREFIX>FONTS` overrides the font paths,
    /// `<PREFIX>HEADERS` the header path, `<PREFIX>SIZES` gives the fallback resolutions, and `MAKETEX_MODE` and `MAKETEX_BASE_DPI` get
    /// set, since `texmf.cnf` uses the mode in `PKFONTS` and `GFFONTS`.  With no mode it is `/`, as in C.
    pub fn init_prog(&mut self, prefix: &[u8], dpi: u32, mode: Option<&[u8]>) {
        self.init_prog_makepk(prefix);
        let fonts = self.getenv(&[prefix, b"FONTS"].concat());
        let headers = self.getenv(&[prefix, b"HEADERS"].concat());
        self.init_fallback_resolutions(&[prefix, b"SIZES"].concat());
        self.set_var(b"MAKETEX_BASE_DPI", dpi.to_string().as_bytes());
        self.set_var(b"MAKETEX_MODE", mode.unwrap_or(b"/"));
        // As in C a path already computed stays: pdfTeX calls this at its first shipout, after TFM lookups.
        for f in [KPSE_PK_FORMAT, KPSE_GF_FORMAT, KPSE_ANY_GLYPH_FORMAT, KPSE_TFM_FORMAT] {
            self.override_paths[f as usize] = fonts.clone();
        }
        self.override_paths[KPSE_TEX_PS_HEADER_FORMAT as usize] = headers;
    }

    /// `format_info[format].path`: what `kpsewhich --show-path` prints.  `None` for an unknown format.
    pub fn format_path(&mut self, format: i32) -> Option<Vec<u8>> {
        self.init_format(format).map(|s| s.path.clone())
    }

    /// `kpathsea_in_name_ok`.  As of 2026 anything may be read, whatever `openin_any` says, because
    /// the test could only look at the name, and an absolute name found by searching would fail.
    pub fn in_name_ok(&mut self, _name: &[u8]) -> bool {
        true
    }

    /// `kpathsea_in_name_ok_extended`.
    pub fn in_name_ok_extended(&mut self, _name: &[u8]) -> bool {
        true
    }

    /// `kpathsea_out_name_ok_silent`.
    pub fn out_name_ok_silent(&mut self, name: &[u8]) -> bool {
        self.out_name_ok_1(name, true, false)
    }

    /// `kpathsea_out_name_ok`: as the silent form, and says why on stderr if not.
    pub fn out_name_ok(&mut self, name: &[u8]) -> bool {
        self.out_name_ok_1(name, false, false)
    }

    /// `kpathsea_out_name_ok_silent_extended`: also allows `$TEXMFVAR` and `$TEXMFSYSVAR`.
    pub fn out_name_ok_silent_extended(&mut self, name: &[u8]) -> bool {
        self.out_name_ok_1(name, true, true)
    }

    /// `kpathsea_out_name_ok_extended`.
    pub fn out_name_ok_extended(&mut self, name: &[u8]) -> bool {
        self.out_name_ok_1(name, false, true)
    }

    fn out_name_ok_1(&mut self, name: &[u8], silent: bool, extended: bool) -> bool {
        // Output of an executable file is restricted on Windows.
        if DOSISH && self.executable_filep(name, silent) {
            return false;
        }
        let (ok, choice) = self.out_name_check(name, extended);
        if !ok && !silent {
            let msg = [
                &b"\n"[..],
                &self.invocation_name[..],
                b": Not writing to ",
                name,
                b" (openout_any = ",
                &choice[..],
                b"; ",
                if extended { &b""[..] } else { &b"no "[..] },
                b"extended check).\n",
            ]
            .concat();
            use std::io::Write;
            let _ = std::io::stderr().write_all(&msg);
        }
        ok
    }

    /// `kpathsea_name_ok`, writing.  Returns the answer and the setting that gave it.
    fn out_name_check(&mut self, fname: &[u8], extended: bool) -> (bool, Vec<u8>) {
        let open_choice = self.var_value(b"openout_any").unwrap_or_else(|| b"p".to_vec());
        let c = at(&open_choice, 0);
        // If setting is a(nything), we're done.
        if c == b'a' || c == b'y' || c == b'1' {
            return (true, open_choice);
        }
        let expanded = self.expand(fname);

        // Disallow .rhosts, .login, .ssh/, ..somefile, ..somedir/somefile and so on.  Specially
        // allow /foo/.whatever if extended, since it might match against TEXMF[SYS]VAR below.
        if !DOSISH {
            let mut q = 0;
            while let Some(i) = fname[q..].iter().position(|&ch| ch == b'.') {
                let d = q + i;
                if (d == 0 || is_dir_sep(fname[d - 1]))
                    && !is_dir_sep(at(fname, d + 1))
                    && !(at(fname, d + 1) == b'.' && is_dir_sep(at(fname, d + 2)))
                    && !(extended && absolute_p(&expanded, false))
                {
                    return (false, open_choice);
                }
                q = d + 1;
            }
        }

        // If setting is only r(estricted), we're done.
        if c == b'r' || c == b'n' || c == b'0' {
            return (true, open_choice);
        }

        // p(aranoid): an absolute name must be in an allowed directory.
        if absolute_p(&expanded, false) {
            // TEXMF_OUTPUT_DIRECTORY must be an environment variable, not a cnf setting.
            let outdir = self.getenv(b"TEXMF_OUTPUT_DIRECTORY");
            if !abs_fname_ok(&expanded, outdir.as_deref()) {
                let output = self.var_value(b"TEXMFOUTPUT");
                if !abs_fname_ok(&expanded, output.as_deref()) {
                    if !extended {
                        return (false, open_choice);
                    }
                    let var = self.var_value(b"TEXMFVAR");
                    if !abs_fname_ok(&expanded, var.as_deref()) {
                        let sysvar = self.var_value(b"TEXMFSYSVAR");
                        if !abs_fname_ok(&expanded, sysvar.as_deref()) {
                            return (false, open_choice);
                        }
                    }
                }
            }
        }

        // For all names, disallow "../" at the beginning or "/../" anywhere.
        if at(fname, 0) == b'.' && at(fname, 1) == b'.' && is_dir_sep(at(fname, 2)) {
            return (false, open_choice);
        }
        let mut i = 0;
        while let Some(j) = fname[i..].windows(2).position(|w| w == b"..") {
            let d = i + j;
            if is_dir_sep(at(fname, d + 2)) && d > 0 && is_dir_sep(fname[d - 1]) {
                return (false, open_choice);
            }
            i = d + 2;
        }
        (true, open_choice)
    }

    /// `executable_filep`, with progname.c's `mk_suffixlist`: on Windows, under `openout_any = p`, no
    /// `.dll` or executable may be written.
    fn executable_filep(&mut self, fname: &[u8], silent: bool) -> bool {
        const EXE_SUFFIXES: &[u8] = b".com;.exe;.bat;.cmd;.vbs;.vbe;.js;.jse;.wsf;.wsh;.ws;.tcl;.py;.pyw";
        let p = self.var_value(b"openout_any");
        if at(p.as_deref().unwrap_or(b""), 0) != b'p' {
            return false;
        }
        let tail = fname.rsplit(|&c| c == b'/').next().unwrap_or(fname);
        let tail = tail.rsplit(|&c| c == b'\\').next().unwrap_or(tail);
        let mut base = tail.to_ascii_lowercase();
        // Remove a trailing dot, space or tab.
        while base.last().is_some_and(|&c| c == b'.' || c == b' ' || c == b'\t') {
            base.pop();
        }
        let Some(dot) = base.iter().rposition(|&c| c == b'.') else { return false };
        let ext = &base[dot..];
        let list = self.getenv(b"PATHEXT").map(|v| v.to_ascii_lowercase()).unwrap_or_else(|| EXE_SUFFIXES.to_vec());
        let forbidden = ext == b".dll" || list.split(|&c| c == b';').any(|s| s == ext);
        if forbidden && !silent {
            let msg = [&b"\n"[..], fname, b": Forbidden to open for writing\n"].concat();
            use std::io::Write;
            let _ = std::io::stderr().write_all(&msg);
        }
        forbidden
    }

    /// web2c's `texmf_yesno`.
    pub fn yesno(&mut self, var: &[u8]) -> bool {
        let v = self.var_value(var);
        matches!(at(v.as_deref().unwrap_or(b""), 0), b't' | b'y' | b'1')
    }

    /// web2c's `setupboundvariable`: the value in the environment or `texmf.cnf`, or `dflt`.  `atoi`
    /// semantics; negative numbers and a 0 where a positive default exists are wrong and are ignored.
    pub fn setup_bound_var(&mut self, var_name: &[u8], dflt: i32) -> i32 {
        let Some(expansion) = self.var_value(var_name) else { return dflt };
        let conf_val = atoi(&expansion);
        if conf_val < 0 || (conf_val == 0 && dflt > 0) {
            let msg = [
                &self.invocation_name[..],
                b": Bad value (",
                conf_val.to_string().as_bytes(),
                b") in environment or texmf.cnf for ",
                var_name,
                b", keeping ",
                dflt.to_string().as_bytes(),
                b".\n",
            ]
            .concat();
            use std::io::Write;
            let _ = std::io::stderr().write_all(&msg);
            dflt
        } else {
            conf_val
        }
    }
}

/// C's `atoi`, saturating where C's result is undefined.
pub(crate) fn atoi(s: &[u8]) -> i32 {
    let mut p = 0;
    while is_space(at(s, p)) {
        p += 1;
    }
    let neg = at(s, p) == b'-';
    if neg || at(s, p) == b'+' {
        p += 1;
    }
    let mut n: i64 = 0;
    while at(s, p).is_ascii_digit() {
        n = (n * 10 + (s[p] - b'0') as i64).min(1 << 40);
        p += 1;
    }
    let n = if neg { -n } else { n };
    n.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// `abs_fname_ok`: `checkdir` is set, non-empty, begins `fname`, and is followed there by a separator
/// or the end of the string.
fn abs_fname_ok(fname: &[u8], checkdir: Option<&[u8]>) -> bool {
    match checkdir {
        Some(d) if !d.is_empty() => {
            fname.starts_with(d) && (is_dir_sep(at(fname, d.len())) || fname.len() == d.len())
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// The shared instance: kpse_def.
// ---------------------------------------------------------------------------

static GLOBAL: Mutex<Option<Kpse>> = Mutex::new(None);

/// What a first search costs, for the engine's profiler, which prints this with its own scopes: `tools/prof/`.
#[cfg(feature = "prof")]
pub mod prof {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
    use std::time::Instant;

    pub struct Slot {
        name: &'static str,
        ns: AtomicU64,
        count: AtomicU64,
    }

    impl Slot {
        const fn new(name: &'static str) -> Slot {
            Slot { name, ns: AtomicU64::new(0), count: AtomicU64::new(0) }
        }
    }

    pub(crate) static TREE: Slot = Slot::new("kpse: find the tree");
    pub(crate) static CNF: Slot = Slot::new("kpse: texmf.cnf read and parse");
    pub(crate) static DB: Slot = Slot::new("kpse: ls-R load, all");
    pub(crate) static DB_READ: Slot = Slot::new("kpse: ls-R read");
    pub(crate) static DB_BUILD: Slot = Slot::new("kpse: ls-R index");

    pub(crate) struct Guard(&'static Slot, Instant);

    pub(crate) fn begin(s: &'static Slot) -> Guard {
        Guard(s, Instant::now())
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.ns.fetch_add(self.1.elapsed().as_nanos() as u64, Relaxed);
            self.0.count.fetch_add(1, Relaxed);
        }
    }

    /// Name, total nanoseconds, count.
    pub fn take() -> Vec<(&'static str, u64, u64)> {
        [&TREE, &CNF, &DB, &DB_READ, &DB_BUILD].iter().map(|s| (s.name, s.ns.load(Relaxed), s.count.load(Relaxed))).collect()
    }
}

/// Runs `f` on the shared instance, creating it on first use.
pub fn with_global<R>(f: impl FnOnce(&mut Kpse) -> R) -> R {
    let mut guard = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let k = guard.get_or_insert_with(|| Kpse::new(Config::default()));
    f(k)
}

/// `kpse_find_file`.  `must_exist` asks for a search of the disk if the databases do not have the name.
pub fn find_file(name: &[u8], format: i32, must_exist: bool) -> Option<Vec<u8>> {
    with_global(|k| k.find_file(name, format, must_exist))
}

/// `kpse_invocation_name`.
pub fn invocation_name() -> Vec<u8> {
    with_global(|k| k.invocation_name().to_vec())
}

/// `kpse_find_file_generic` with `all = true`: every match.
pub fn find_all(name: &[u8], format: i32, must_exist: bool) -> Vec<Vec<u8>> {
    with_global(|k| k.find_file_generic(name, format, must_exist, true))
}

/// `kpse_in_name_ok`.
pub fn in_name_ok(name: &[u8]) -> bool {
    with_global(|k| k.in_name_ok(name))
}

/// `kpse_out_name_ok`: under `openout_any = p`, the default, no dotfiles, no parent directories and no
/// absolute names outside the output directory.
pub fn out_name_ok(name: &[u8]) -> bool {
    with_global(|k| k.out_name_ok(name))
}

/// `kpse_out_name_ok_silent`.
pub fn out_name_ok_silent(name: &[u8]) -> bool {
    with_global(|k| k.out_name_ok_silent(name))
}

/// web2c's `texmf_yesno`: a boolean from `texmf.cnf` or the environment.
pub fn yesno(var: &[u8]) -> bool {
    with_global(|k| k.yesno(var))
}

/// `kpse_var_value`.
pub fn var_value(var: &[u8]) -> Option<Vec<u8>> {
    with_global(|k| k.var_value(var))
}

/// `kpse_var_expand`.
pub fn var_expand(s: &[u8]) -> Vec<u8> {
    with_global(|k| k.var_expand(s))
}

/// web2c's `setupboundvariable`.
pub fn setup_bound_var(var_name: &[u8], dflt: i32) -> i32 {
    with_global(|k| k.setup_bound_var(var_name, dflt))
}

/// `kpse_reset_program_name`.
pub fn set_program_name(progname: &[u8]) {
    with_global(|k| k.set_program_name(progname))
}

/// `xputenv`, as far as this crate can see.  For `TEXMF_OUTPUT_DIRECTORY` and the like.
pub fn set_var(var: &[u8], value: &[u8]) {
    with_global(|k| k.set_var(var, value))
}

/// What a child process must inherit: see [`Kpse::child_env`].
pub fn child_env() -> Vec<(Vec<u8>, Vec<u8>)> {
    with_global(|k| k.child_env())
}

/// `kpathsea_cnf_line_env_progname`.
pub fn cnf_line_env_progname(line: &[u8]) {
    with_global(|k| k.cnf_line_env_progname(line))
}

/// `kpse_init_prog`, see [`Kpse::init_prog`].
pub fn init_prog(prefix: &[u8], dpi: u32, mode: Option<&[u8]>) {
    with_global(|k| k.init_prog(prefix, dpi, mode))
}

/// The search path of a format: for a checkpoint's key, as DESIGN.md section 7 asks.
pub fn format_path(format: i32) -> Option<Vec<u8>> {
    with_global(|k| k.format_path(format))
}

/// The files read so far for the `-recorder` file: see [`Kpse::take_recorded`].
pub fn take_recorded() -> Vec<Vec<u8>> {
    with_global(|k| k.take_recorded())
}

/// The messages so far: see [`Kpse::take_warnings`].
pub fn take_warnings() -> Vec<Vec<u8>> {
    with_global(|k| k.take_warnings())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_names() {
        assert!(out_name_ok(b"a.tex"));
        assert!(out_name_ok(b"sub/a.tex"));
        assert!(!out_name_ok(b".rc"));
        assert!(!out_name_ok(b"../a.tex"));
    }

    #[test]
    fn a_child_inherits_what_xputenv_set() {
        let mut k = Kpse::new(Config { process_env: false, tree: Some(Tree::from_selfautoloc(b"/a/bin")), ..Config::default() });
        k.set_var(b"TEXMF_OUTPUT_DIRECTORY", b"out");
        let env = k.child_env();
        let get = |name: &[u8]| env.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone());
        assert_eq!(get(b"SELFAUTOLOC").as_deref(), Some(&b"/a/bin"[..]));
        assert_eq!(get(b"progname").as_deref(), Some(&b"pdftex"[..]));
        assert_eq!(get(b"engine").as_deref(), Some(&b"pdftex"[..]));
        assert_eq!(get(b"TEXMF_OUTPUT_DIRECTORY").as_deref(), Some(&b"out"[..]));
    }

    #[test]
    fn missing_file() {
        assert_eq!(find_file(b"/rstex/surely/not/here", KPSE_TEX_FORMAT, true), None);
    }

    #[test]
    fn c_atoi() {
        assert_eq!(atoi(b"42"), 42);
        assert_eq!(atoi(b"  -7x"), -7);
        assert_eq!(atoi(b"+3"), 3);
        assert_eq!(atoi(b""), 0);
        assert_eq!(atoi(b"abc"), 0);
    }

    #[test]
    fn abs_names() {
        assert!(abs_fname_ok(b"/a/b", Some(b"/a")));
        assert!(abs_fname_ok(b"/a", Some(b"/a")));
        assert!(!abs_fname_ok(b"/ab", Some(b"/a")));
        assert!(!abs_fname_ok(b"/a", Some(b"")));
        assert!(!abs_fname_ok(b"/a", None));
    }
}
