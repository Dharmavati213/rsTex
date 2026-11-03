//! Bitmap fonts: tex-glyph.c, tex-make.c, magstep.c, and what tex-file.c and proginit.c set up for them.
//!
//! Only `mktexpk` runs a program here, for `kpse_pk_format` and `kpse_any_glyph_format`, which are what tex-file.c gives
//! `MKTEXPK_ARGS`.  The other `mktex*` are not ported: `find_file_generic` does not call [`Kpse::make_tex`], and `-mktex=tfm` only
//! sets a flag no one reads.  What `maketex` prints goes to stderr at once, unlike other messages of this crate, since
//! the script's own messages follow it there.
use crate::db::readable_file;
use crate::format::*;
use crate::names::*;
use crate::{Kpse, sys};
use std::io::Write;

/// `kpse_src_type`.
pub const KPSE_SRC_COMPILE: i32 = 1;
pub const KPSE_SRC_TEXMF_CNF: i32 = 2;
pub const KPSE_SRC_CLIENT_CNF: i32 = 3;
pub const KPSE_SRC_ENV: i32 = 4;
pub const KPSE_SRC_X: i32 = 5;
pub const KPSE_SRC_CMDLINE: i32 = 6;

/// `kpse_glyph_source_type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlyphSource {
    Normal,
    Alias,
    Maketex,
    FallbackRes,
    Fallback,
}

/// `kpse_glyph_file_type`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphFile {
    pub name: Vec<u8>,
    pub dpi: u32,
    pub format: i32,
    pub source: GlyphSource,
}

/// The part of `kpse_format_info_type` about `mktex*`.
#[derive(Clone, Debug, Default)]
pub(crate) struct MakeInfo {
    program: Option<Vec<u8>>,
    enabled: bool,
    level: i32,
}

/// `MKTEXPK_ARGS`, with the program first.
const MKTEXPK_ARGV: [&[u8]; 9] =
    [b"mktexpk", b"--mfmode", b"$MAKETEX_MODE", b"--bdpi", b"$MAKETEX_BASE_DPI", b"--mag", b"$MAKETEX_MAG", b"--dpi", b"$KPATHSEA_DPI"];

const UNIX_BITMAP_SPEC: &[u8] = b"$KPATHSEA_NAME.$KPATHSEA_DPI$KPATHSEA_FORMAT";
const DPI_BITMAP_SPEC: &[u8] = b"dpi$KPATHSEA_DPI/$KPATHSEA_NAME.$KPATHSEA_FORMAT";
const DOS_BITMAP_SPEC: &[u8] = b"$KPATHSEA_DPI/$KPATHSEA_NAME.$KPATHSEA_FORMAT";

/// `KPSE_BITMAP_TOLERANCE`, as a C conversion to `unsigned` leaves it.
fn bitmap_tolerance_of(r: f64) -> u32 {
    (r / 500.0 + 1.0) as u32
}

/// `kpathsea_bitmap_tolerance`: is `dpi1` within the tolerance of `dpi2`?
pub fn bitmap_tolerance(dpi1: f64, dpi2: f64) -> bool {
    let tolerance = bitmap_tolerance_of(dpi2) as f64;
    let lower_bound = if ((dpi2 - tolerance) as i32) < 0 { 0 } else { (dpi2 - tolerance) as u32 };
    let upper_bound = (dpi2 + tolerance) as u32;
    lower_bound as f64 <= dpi1 && dpi1 <= upper_bound as f64
}

/// `magstep`.
fn magstep(n: i32, bdpi: u32) -> i32 {
    let (mut n, neg) = if n < 0 { (-n, true) } else { (n, false) };
    let mut t: f64;
    if n & 1 != 0 {
        n &= !1;
        t = 1.095445115;
    } else {
        t = 1.0;
    }
    while n > 8 {
        n -= 8;
        t *= 2.0736;
    }
    while n > 0 {
        n -= 2;
        t *= 1.2;
    }
    (0.5 + if neg { bdpi as f64 / t } else { bdpi as f64 * t }) as i32
}

/// `kpathsea_magstep_fix`: the true dpi, and what the C leaves in `*m_ret`, which by its `==` is 0 or 1 and not the magstep.
pub fn magstep_fix(dpi: u32, bdpi: u32) -> (u32, i32) {
    const MAGSTEP_MAX: i32 = 40;
    let mut mdpi = -1;
    let mut real_dpi: u32 = 0;
    let sign = if dpi < bdpi { -1 } else { 1 }; // negative or positive magsteps?
    let mut m = 0;
    while real_dpi == 0 && m < MAGSTEP_MAX {
        mdpi = magstep(m * sign, bdpi);
        if (mdpi - dpi as i32).abs() <= 1 {
            real_dpi = mdpi as u32; // if this magstep matches, quit
        } else if (mdpi - dpi as i32) * sign > 0 {
            real_dpi = dpi; // if gone too far, quit
        }
        m += 1;
    }
    let m_ret = (real_dpi == (if mdpi != 0 { (m - 1) * sign } else { 0 }) as u32) as i32;
    (if real_dpi != 0 { real_dpi } else { dpi }, m_ret)
}

/// The format of an argument of `-mktex` and `-no-mktex`, as `kpathsea_maketex_option` tells them.
pub fn maketex_format(fmtname: &[u8]) -> Option<i32> {
    for (name, fmt) in [
        (&b"pk"[..], KPSE_PK_FORMAT),
        (b"mf", KPSE_MF_FORMAT),
        (b"tex", KPSE_TEX_FORMAT),
        (b"tfm", KPSE_TFM_FORMAT),
        (b"fmt", KPSE_FMT_FORMAT),
        (b"ofm", KPSE_OFM_FORMAT),
        (b"ocp", KPSE_OCP_FORMAT),
    ] {
        if file_str_eq(fmtname, name) {
            return Some(fmt);
        }
    }
    None
}

/// C's `atoi`, unsigned as `set_maketex_mag` takes it.
fn atou(v: Option<Vec<u8>>) -> u32 {
    crate::atoi(v.as_deref().unwrap_or(b"")) as u32
}

impl Kpse {
    /// `kpathsea_set_program_enabled`.
    pub fn set_program_enabled(&mut self, format: i32, value: bool, level: i32) {
        let f = &mut self.make[format as usize];
        if level >= f.level {
            f.enabled = value;
            f.level = level;
        }
    }

    /// `kpse->make_tex_discard_errors`, which is §1443's for `\batchmode`.
    pub fn set_make_tex_discard_errors(&mut self, discard: bool) {
        self.make_tex_discard_errors = discard;
    }

    /// `init_maketex`, for the formats that run `mktexpk`.
    pub(crate) fn init_maketex(&mut self, format: i32) {
        let f = format as usize;
        let prog = self.make[f].program.clone().unwrap_or_else(|| MKTEXPK_ARGV[0].to_vec());
        let upper = prog.to_ascii_uppercase();
        let progval = self.var_value(&upper);
        // Doesn't hurt to always set this info.
        self.make[f].program = Some(prog);
        if let Some(v) = progval {
            if !v.is_empty() {
                // This might actually be from an environment variable value, but in that case, we'll have
                // previously set it from kpse_init_prog.
                self.set_program_enabled(format, v[0] == b'1', KPSE_SRC_CLIENT_CNF);
            }
        }
    }

    /// `kpathsea_init_prog`'s part about `<PREFIX>MAKEPK`.
    pub(crate) fn init_prog_makepk(&mut self, prefix: &[u8]) {
        if let Some(prog) = sys::getenv(&[prefix, b"MAKEPK"].concat()) {
            // If we did, we want to enable the program, I think.
            for format in [KPSE_PK_FORMAT, KPSE_ANY_GLYPH_FORMAT] {
                self.set_program_enabled(format, true, KPSE_SRC_ENV);
                self.make[format as usize].program = Some(prog.clone());
            }
        }
    }

    /// `kpathsea_init_fallback_resolutions`.
    pub(crate) fn init_fallback_resolutions(&mut self, envvar: &[u8]) {
        let size_str = self.getenv(envvar).or_else(|| self.getenv(b"TEXSIZES"));
        let size_list = expand_default(size_str.as_deref(), b"");
        let mut last_resort_sizes: Vec<u32> = Vec::new();
        for size in path_elements(&size_list) {
            if size.is_empty() {
                continue; // Skip empty elements.
            }
            let s = crate::atoi(size) as u32;
            if last_resort_sizes.last().is_some_and(|&l| s < l) {
                self.warn(&[b"warning: kpathsea: last resort size ", size, b" not in ascending order; ignored.\n"].concat());
            } else {
                last_resort_sizes.push(s);
            }
        }
        // A zero marks the end of the list, wherever it is.
        if let Some(z) = last_resort_sizes.iter().position(|&s| s == 0) {
            last_resort_sizes.truncate(z);
        }
        self.fallback_resolutions = Some(last_resort_sizes);
    }

    /// `try_format`: font `$KPATHSEA_NAME` at resolution `$KPATHSEA_DPI` in this format.
    fn try_format(&mut self, format: i32) -> Option<Vec<u8>> {
        let specs: &[&[u8]] = if DOSISH { &[UNIX_BITMAP_SPEC, DPI_BITMAP_SPEC, DOS_BITMAP_SPEC] } else { &[UNIX_BITMAP_SPEC, DPI_BITMAP_SPEC] };
        let st = self.init_format(format)?;
        // Set the suffix on the name we'll be searching for.
        if let Some(sfx) = format_def(format).and_then(|d| d.suffix.first()) {
            self.set_var(b"KPATHSEA_FORMAT", sfx.as_bytes());
        }
        // Once with must_exist false, to avoid looking on the disk for cmr10.600pk if dpi600/cmr10.pk is in ls-R.
        for must_exist in [false, true] {
            for spec in specs {
                let name = self.var_expand(spec);
                if let Some(ret) = self.path_search(&st.path, &name, must_exist) {
                    return Some(ret);
                }
            }
        }
        None
    }

    /// `try_size`: the (entire) PK path first, then the GF path, if we're looking for both.
    fn try_size(&mut self, fontname: &[u8], dpi: u32, format: i32) -> Option<(Vec<u8>, GlyphFile)> {
        let try_gf = format == KPSE_GF_FORMAT || format == KPSE_ANY_GLYPH_FORMAT;
        let try_pk = format == KPSE_PK_FORMAT || format == KPSE_ANY_GLYPH_FORMAT;
        self.set_var(b"KPATHSEA_DPI", dpi.to_string().as_bytes());
        // Look for PK first (since it's more likely to be found), then GF.
        let mut ret = if try_pk { self.try_format(KPSE_PK_FORMAT) } else { None };
        let mut format_found = KPSE_PK_FORMAT;
        if ret.is_none() && try_gf {
            ret = self.try_format(KPSE_GF_FORMAT);
            format_found = KPSE_GF_FORMAT;
        }
        ret.map(|r| (r, GlyphFile { name: fontname.to_vec(), dpi, format: format_found, source: GlyphSource::Normal }))
    }

    /// `try_resolution`: at resolution `dpi`, then at those within the tolerance of `dpi`.
    fn try_resolution(&mut self, fontname: &[u8], dpi: u32, format: i32) -> Option<(Vec<u8>, GlyphFile)> {
        if let Some(r) = self.try_size(fontname, dpi, format) {
            return Some(r);
        }
        let tolerance = bitmap_tolerance_of(dpi as f64);
        let lower_bound = if (dpi as i32 - tolerance as i32) < 0 { 0 } else { dpi - tolerance };
        let upper_bound = dpi + tolerance;
        // Prefer scaling up to scaling down, since scaling down can omit character features (Tom did this in dvips).
        for r in lower_bound..=upper_bound {
            if r != dpi {
                if let Some(found) = self.try_size(fontname, r, format) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// `try_fontmap`: `fontname` becomes the real name of an alias found, or its first alias if that is not an alias itself.  (This
    /// allows mktexpk to only deal with real names.)
    fn try_fontmap(&mut self, fontname: &mut Vec<u8>, dpi: u32, format: i32) -> Option<(Vec<u8>, GlyphFile)> {
        let mapped_names = self.fontmap_lookup(fontname)?;
        let first_name = mapped_names.first()?.clone();
        for mapped_name in &mapped_names {
            self.set_var(b"KPATHSEA_NAME", mapped_name);
            if let Some(r) = self.try_resolution(mapped_name, dpi, format) {
                // If some alias succeeeded, return that alias.
                *fontname = mapped_name.clone();
                return Some(r);
            }
        }
        // Return first alias name, unless that itself is an alias, in which case do nothing.
        if self.fontmap_lookup(&first_name).is_none() {
            *fontname = first_name;
        }
        None
    }

    /// `try_fallback_resolutions`.
    fn try_fallback_resolutions(&mut self, fontname: &[u8], dpi: u32, format: i32) -> Option<(Vec<u8>, GlyphFile)> {
        let sizes = self.fallback_resolutions.clone()?;
        if sizes.is_empty() {
            return None; // If nothing in list, quit now.
        }
        // First find the fallback size closest to DPI, even including DPI.
        let mut loc = 0i32;
        let mut closest_diff = u32::MAX;
        for (s, &size) in sizes.iter().enumerate() {
            let this_diff = size.abs_diff(dpi);
            if this_diff < closest_diff {
                closest_diff = this_diff;
                loc = s as i32;
            }
        }
        let max_loc = sizes.len() as i32;
        let mut lower_loc = loc - 1;
        let mut upper_loc = loc + 1;
        loop {
            let fallback = sizes[loc as usize];
            // Don't bother to try DPI itself again.
            if fallback != dpi {
                if let Some(r) = self.try_resolution(fontname, fallback, format) {
                    return Some(r);
                }
            }
            // That didn't work.  How far away are the locs above or below?
            const FAR: u32 = i32::MAX as u32;
            let lower_diff = if lower_loc > -1 { dpi.wrapping_sub(sizes[lower_loc as usize]) } else { FAR };
            let upper_diff = if upper_loc < max_loc { sizes[upper_loc as usize].wrapping_sub(dpi) } else { FAR };
            // But if we're at the end in both directions, quit.
            if lower_diff == FAR && upper_diff == FAR {
                return None;
            }
            // Go in whichever direction is closest.
            if lower_diff < upper_diff {
                loc = lower_loc;
                lower_loc -= 1;
            } else {
                loc = upper_loc;
                upper_loc += 1;
            }
        }
    }

    /// `kpathsea_find_glyph`: the file found, with what the C fills in `glyph_file`.  It runs `mktexpk` if the font is not there, when
    /// that is allowed.  A name of `mktexpk`'s is used as in C, with the format of the search.
    pub fn find_glyph(&mut self, passed_fontname: &[u8], dpi: u32, format: i32) -> Option<(Vec<u8>, GlyphFile)> {
        self.ensure_cnf();
        let mut fontname = passed_fontname.to_vec();
        // Start the search: try the name we're given.
        let mut source = GlyphSource::Normal;
        self.set_var(b"KPATHSEA_NAME", &fontname);
        let mut ret = self.try_resolution(&fontname, dpi, format);
        // Try all the various possibilities in order of preference.
        if ret.is_none() {
            // Maybe FONTNAME was an alias.
            source = GlyphSource::Alias;
            ret = self.try_fontmap(&mut fontname, dpi, format);
            // If not an alias, try creating it on the fly with mktexpk, unless FONTNAME is absolute or explicitly relative.
            if ret.is_none() && !absolute_p(&fontname, true) {
                source = GlyphSource::Maketex;
                // `try_resolution' leaves the envvar set randomly.
                self.set_var(b"KPATHSEA_DPI", dpi.to_string().as_bytes());
                if let Some(made) = self.make_tex(format, &fontname) {
                    ret = Some((made, GlyphFile { name: fontname.clone(), dpi, format, source }));
                }
            }
            // If mktex... failed, try any fallback resolutions.  There is no font of last resort: pdfTeX gives `nil`.
            if ret.is_none() && self.fallback_resolutions.is_some() {
                source = GlyphSource::FallbackRes;
                ret = self.try_fallback_resolutions(&fontname, dpi, format);
            }
        }
        ret.map(|(r, mut g)| {
            g.source = source;
            (r, g)
        })
    }

    /// `set_maketex_mag`: `MAKETEX_MAG`, from `KPATHSEA_DPI` and `MAKETEX_BASE_DPI`.
    fn set_maketex_mag(&mut self) {
        let dpi = atou(self.getenv(b"KPATHSEA_DPI"));
        let bdpi = atou(self.getenv(b"MAKETEX_BASE_DPI"));
        // If the environment variables aren't set, it's a bug.
        assert!(dpi != 0 && bdpi != 0);
        // Fix up for roundoff error.  Hopefully the driver has already fixed up DPI, but may as well be safe, and also get the magstep number.
        let (_, mut m) = magstep_fix(dpi, bdpi);
        let q = if m == 0 {
            if bdpi <= 4000 {
                format!("{}+{}/{}", dpi / bdpi, dpi % bdpi, bdpi)
            } else {
                let f = bdpi / 4000;
                let r = bdpi % 4000;
                if f > 1 {
                    if r > 0 {
                        format!("{}+{}/({}*{}+{})", dpi / bdpi, dpi % bdpi, f, (bdpi - r) / f, r)
                    } else {
                        format!("{}+{}/({}*{})", dpi / bdpi, dpi % bdpi, f, bdpi / f)
                    }
                } else {
                    format!("{}+{}/(4000+{})", dpi / bdpi, dpi % bdpi, r)
                }
            }
        } else {
            // m is encoded with LSB being a ``half'' bit (see magstep.h).
            let mut sign = "";
            if m < 0 {
                m = -m;
                sign = "-";
            }
            format!("magstep\\({}{}.{}\\)", sign, m / 2, (m & 1) * 5)
        };
        self.set_var(b"MAKETEX_MAG", q.as_bytes());
    }

    /// `misstex`: this mktex... program was disabled, or the script failed, so the command goes to `missfont.log`.
    fn misstex(&mut self, args: &[Vec<u8>]) {
        // If this is the first time, have to open the log file.  But don't bother logging anything if they were discarding errors.
        if self.missfont.is_none() && !self.make_tex_discard_errors {
            let missfont_name: Option<Vec<u8>> = match self.var_value(b"MISSFONT_LOG") {
                None => Some(b"missfont.log".to_vec()), // take default name
                Some(v) if v.first() == Some(&b'1') => Some(b"missfont.log".to_vec()),
                Some(v) if v.is_empty() || v[0] == b'0' => None, // user requested no missfont.log
                Some(v) => Some(v),                              // else use user's name
            };
            if let Some(name) = missfont_name {
                // If the given name is not already absolute, and TEXMF_OUTPUT_DIRECTORY is set, use that instead of cwd.  This is
                // only an envvar, not a config file value.
                let outdir = if absolute_p(&name, false) { None } else { self.getenv(b"TEXMF_OUTPUT_DIRECTORY") };
                let first_name = match outdir {
                    Some(d) => [&d[..], b"/", &name[..]].concat(),
                    None => name.clone(),
                };
                let mut shown = first_name.clone();
                self.missfont = sys::open_append(&first_name);
                // If that failed, try TEXMFOUTPUT.
                if self.missfont.is_none() {
                    if let Some(d) = self.var_value(b"TEXMFOUTPUT") {
                        shown = [&d[..], b"/", &name[..]].concat();
                        self.missfont = sys::open_append(&shown);
                    }
                }
                if self.missfont.is_some() {
                    let m = [&b"kpathsea: Appending font creation commands to "[..], &shown, b".\n"].concat();
                    let _ = std::io::stderr().write_all(&m);
                }
            }
        }
        // Write the command if we have a log file.
        if let Some(f) = &mut self.missfont {
            let mut line = Vec::new();
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    line.push(b' ');
                }
                line.extend_from_slice(a);
            }
            line.push(b'\n');
            let _ = f.write_all(&line);
        }
    }

    /// `maketex`: runs the script, which outputs the filename it creates (and nothing else) on standard output.
    fn maketex(&mut self, args: &[Vec<u8>]) -> Option<Vec<u8>> {
        if !self.make_tex_discard_errors {
            let mut m = b"\nkpathsea: Running".to_vec();
            for a in args {
                m.push(b' ');
                m.extend_from_slice(a);
            }
            m.push(b'\n');
            let _ = std::io::stderr().write_all(&m);
        }
        let mut name = sys::run_mktex(args, &self.child_env(), self.make_tex_discard_errors, &self.tree.selfauto[0]);
        let mut ret = None;
        if let Some(cut) = name.iter().position(|&c| c == 0) {
            name.truncate(cut);
        }
        // Remove trailing newlines and returns.
        while matches!(name.last(), Some(b'\n' | b'\r')) {
            name.pop();
        }
        if !name.is_empty() {
            let mut f = name.clone();
            if readable_file(&mut f) {
                ret = Some(f);
            } else if name.len() > 1 {
                let m = [&b"warning: kpathsea: "[..], &args[0], b" output `", &name, b"' instead of a filename.\n"].concat();
                let _ = std::io::stderr().write_all(&m);
            }
        }
        match &ret {
            None => self.misstex(args),
            Some(f) => {
                if self.db_state == crate::DbState::Ready && self.db_usable() {
                    self.db_insert(f);
                }
            }
        }
        ret
    }

    /// `kpathsea_make_tex`: creates `base` in this format and returns the filename, for the formats of `MKTEXPK_ARGS`.  We used to
    /// emit warnings for names we declined to pass on to the scripts, but such names are common with system fonts, so now we are silent.
    pub(crate) fn make_tex(&mut self, format: i32, base: &[u8]) -> Option<Vec<u8>> {
        self.init_format(format)?;
        let spec = &self.make[format as usize];
        if spec.program.is_none() || !spec.enabled {
            return None;
        }
        // Err on the side of strictness: may not start with a hyphen; allowed are alphanumeric, underscore, hyphen, period, plus,
        // and also DIRSEP, as we can be fed that when creating pk fonts.
        if base.first() == Some(&b'-') {
            return None;
        }
        if !base.iter().all(|&c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'+' | b'_' | b'.') || is_dir_sep_ch(c)) {
            return None;
        }
        self.set_maketex_mag();
        // If the mode is `/', mktexpk recognizes it as a special case: see kpse_prog_init.
        let mut args: Vec<Vec<u8>> = Vec::with_capacity(MKTEXPK_ARGV.len() + 1);
        for a in MKTEXPK_ARGV {
            args.push(self.var_expand(a));
        }
        args.push(base.to_vec());
        self.maketex(&args)
    }
}

/// `kpse_find_glyph`, with a format of `kpse_pk_format` for `kpse_find_pk`.
pub fn find_glyph(name: &[u8], dpi: u32, format: i32) -> Option<(Vec<u8>, GlyphFile)> {
    crate::with_global(|k| k.find_glyph(name, dpi, format))
}

/// `kpse_set_program_enabled`.
pub fn set_program_enabled(format: i32, value: bool, level: i32) {
    crate::with_global(|k| k.set_program_enabled(format, value, level))
}

/// `kpse_make_tex_discard_errors`.
pub fn set_make_tex_discard_errors(discard: bool) {
    crate::with_global(|k| k.set_make_tex_discard_errors(discard))
}

/// `kpse_maketex_option`.  An unknown format says so on stderr and is ignored.
pub fn maketex_option(fmtname: &[u8], value: bool) {
    match maketex_format(fmtname) {
        Some(fmt) => set_program_enabled(fmt, value, KPSE_SRC_CMDLINE),
        None => {
            let m = [&b"\nkpathsea: Unknown mktex format: "[..], fmtname, b"\n"].concat();
            let _ = std::io::stderr().write_all(&m);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, Tree};

    #[test]
    fn magsteps_as_the_c_gives_them() {
        assert_eq!(magstep(0, 600), 600);
        assert_eq!(magstep(1, 600), 657);
        assert_eq!(magstep(2, 600), 720);
        assert_eq!(magstep(-2, 600), 500);
        assert_eq!(magstep(10, 600), 1493);
        assert_eq!(magstep_fix(721, 600).0, 720);
        assert_eq!(magstep_fix(700, 600).0, 700);
        assert_eq!(magstep_fix(499, 600).0, 500);
        assert_eq!(magstep_fix(72, 72).0, 72);
        assert_eq!(magstep_fix(600, 600), (600, 0));
        assert_eq!(magstep_fix(720, 600), (720, 0));
        assert_eq!(magstep_fix(719, 600), (720, 0));
        assert_eq!(magstep_fix(657, 600), (657, 0)); // magstep 0.5
        assert_eq!(magstep_fix(1037, 600), (1037, 0)); // magstep 3
        assert_eq!(magstep_fix(500, 600), (500, 0));
        assert_eq!(magstep_fix(417, 600), (417, 0));
        // With a dpi of no magstep, the first one too far ends it.
        assert_eq!(magstep_fix(650, 600), (650, 0));
    }

    #[test]
    fn tolerance() {
        assert!(bitmap_tolerance(601.0, 600.0));
        assert!(bitmap_tolerance(602.0, 600.0));
        assert!(!bitmap_tolerance(603.0, 600.0));
        assert!(bitmap_tolerance(598.0, 600.0));
        assert!(!bitmap_tolerance(597.0, 600.0));
    }

    #[test]
    fn mktex_names() {
        assert_eq!(maketex_format(b"pk"), Some(KPSE_PK_FORMAT));
        assert_eq!(maketex_format(b"tfm"), Some(KPSE_TFM_FORMAT));
        assert_eq!(maketex_format(b"nosuch"), None);
    }

    fn kpse(dir: &[u8]) -> Kpse {
        let mut tree = Tree::from_selfautoloc(b"/nonexistent/bin");
        tree.cnf_default = Some(b"/nonexistent".to_vec());
        Kpse::new(Config {
            process_env: false,
            tree: Some(tree),
            vars: vec![(b"PKFONTS".to_vec(), dir.to_vec()), (b"MKTEXPK".to_vec(), b"0".to_vec())],
            ..Config::default()
        })
    }

    /// What a script made is found through the database in the same run, after what the tables hold: the tables are mapped and do not change.
    #[test]
    fn a_made_file_is_found_through_the_database() {
        let d = std::env::temp_dir().join(format!("rstex-glyph-db-{}", std::process::id()));
        std::fs::create_dir_all(d.join("a")).unwrap();
        std::fs::create_dir_all(d.join("b")).unwrap();
        std::fs::write(d.join("ls-R"), b"% ls-R\n./:\nls-R\n\n./a:\nread.pk\n").unwrap();
        std::fs::write(d.join("a").join("read.pk"), b"x").unwrap();
        std::fs::write(d.join("b").join("read.pk"), b"x").unwrap();
        std::fs::write(d.join("b").join("made.pk"), b"y").unwrap();
        let top = d.to_str().unwrap().as_bytes().to_vec();
        let mut tree = Tree::from_selfautoloc(b"/nonexistent/bin");
        tree.cnf_default = Some(b"/nonexistent".to_vec());
        let mut k = Kpse::new(Config { process_env: false, tree: Some(tree), vars: vec![(b"TEXMFDBS".to_vec(), top.clone())], ..Config::default() });
        k.ensure_db();
        assert!(k.db_usable());
        let elt = [&top[..], b"//"].concat();
        assert_eq!(k.db_search(b"made.pk", &elt, false), Some(vec![]));
        k.db_insert(&[&top[..], b"/b/made.pk"].concat());
        assert_eq!(k.db_search(b"made.pk", &elt, false), Some(vec![[&top[..], b"/b/made.pk"].concat()]));
        // Its directory counts as well: `match` looks at the whole name.
        assert_eq!(k.db_search(b"made.pk", &[&top[..], b"/a//"].concat(), false), Some(vec![]));
        k.db_insert(&[&top[..], b"/b/read.pk"].concat());
        assert_eq!(k.db_search(b"read.pk", &elt, false), Some(vec![[&top[..], b"/a/read.pk"].concat()]));
        assert_eq!(k.db_search(b"read.pk", &elt, true), Some(vec![[&top[..], b"/a/read.pk"].concat(), [&top[..], b"/b/read.pk"].concat()]));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn found_at_the_size_within_the_tolerance_or_not_at_all() {
        let d = std::env::temp_dir().join(format!("rstex-glyph-{}", std::process::id()));
        std::fs::create_dir_all(d.join("dpi600")).unwrap();
        std::fs::write(d.join("aaa.601pk"), b"x").unwrap();
        std::fs::write(d.join("dpi600").join("bbb.pk"), b"y").unwrap();
        let mut k = kpse(d.to_str().unwrap().as_bytes());
        let (p, g) = k.find_glyph(b"aaa", 600, KPSE_PK_FORMAT).unwrap();
        assert!(p.ends_with(b"aaa.601pk"), "{}", String::from_utf8_lossy(&p));
        assert_eq!((g.dpi, g.source), (601, GlyphSource::Normal));
        let (p, g) = k.find_glyph(b"bbb", 600, KPSE_PK_FORMAT).unwrap();
        assert!(p.ends_with(b"dpi600/bbb.pk"), "{}", String::from_utf8_lossy(&p));
        assert_eq!(g.dpi, 600);
        assert_eq!(k.find_glyph(b"ccc", 600, KPSE_PK_FORMAT), None);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
