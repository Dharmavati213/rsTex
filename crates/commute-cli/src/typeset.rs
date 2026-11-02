//! Typesetting cells and labels.
//!
//! **Placeholder.**  Real labels need real TeX, through the embedded engine at M6 (DESIGN section 10).  Until then
//! [`Builtin`] does what a tool without TeX can: one line of glyphs, metrics from Computer Modern's TFM files when they
//! can be found, a rough guess when they cannot, base fonts in the PDF and `<text>` in the SVG.  Scripts are raised and
//! made smaller, `\frac{a}{b}` comes out as `a/b`, there are no kerns or ligatures, and most of TeX's math spacing is
//! missing.  The geometry around the boxes is real; only what is in the boxes is approximate.
//!
//! The rest of the tool only knows the [`Typesetter`] trait.

use commute::{BoxDim, FontParams, LabelFont};

use crate::tfm::{self, Tfm};

/// One typeset box: its dimensions, and what to draw relative to its reference point.
#[derive(Clone, Debug, Default)]
pub struct TypesetBox {
    pub dim: BoxDim,
    pub glyphs: Vec<Glyph>,
}

/// A glyph, positioned in points relative to the box's reference point, y up.
#[derive(Copy, Clone, Debug)]
pub struct Glyph {
    pub x: f64,
    pub y: f64,
    pub size: f64,
    pub face: Face,
    /// The byte in the PDF base font's encoding.
    pub pdf: u8,
    pub unicode: char,
}

/// The PDF base fonts in use, which also say how the SVG text looks.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Face {
    Italic,
    Roman,
    Symbol,
}

/// What is asked for.
#[derive(Copy, Clone, Debug)]
pub struct Request<'a> {
    /// The source, in runs; arrow commands are already out.
    pub runs: &'a [&'a [u8]],
    /// Math or text.
    pub math: bool,
    pub style: Style<'a>,
}

#[derive(Copy, Clone, Debug)]
pub enum Style<'a> {
    /// A cell, with its `font=` if any: `$ { ... } $`.
    Cell(&'a [u8]),
    /// A label: `\scriptstyle` by default (spec 6.1).
    Label(LabelFont, &'a [u8]),
}

/// What the tool needs from TeX.
pub trait Typesetter {
    /// The ambient font's parameters (spec section 4).
    fn font_params(&self) -> FontParams;
    /// Typeset a cell or a label.
    fn typeset(&mut self, req: &Request<'_>) -> TypesetBox;
}

/// A symbol: its name, its place in Computer Modern, and how a base font shows it.
struct Sym {
    name: &'static str,
    /// 0 cmr, 1 cmmi, 2 cmsy.
    fam: u8,
    code: u8,
    face: Face,
    pdf: u8,
    unicode: char,
    /// 0 ordinary, 1 binary, 2 relation: the spaces TeX puts around it, in text style.
    class: u8,
}

const fn sym(name: &'static str, fam: u8, code: u8, face: Face, pdf: u8, unicode: char, class: u8) -> Sym {
    Sym { name, fam, code, face, pdf, unicode, class }
}

/// Sorted by name.  Symbol-font bytes follow Adobe's Symbol encoding.
static SYMS: &[Sym] = &[
    sym("Delta", 0, 1, Face::Symbol, b'D', 'Δ', 0),
    sym("Gamma", 0, 0, Face::Symbol, b'G', 'Γ', 0),
    sym("Lambda", 0, 3, Face::Symbol, b'L', 'Λ', 0),
    sym("Omega", 0, 10, Face::Symbol, b'W', 'Ω', 0),
    sym("Phi", 0, 8, Face::Symbol, b'F', 'Φ', 0),
    sym("Pi", 0, 5, Face::Symbol, b'P', 'Π', 0),
    sym("Psi", 0, 9, Face::Symbol, b'Y', 'Ψ', 0),
    sym("Sigma", 0, 6, Face::Symbol, b'S', 'Σ', 0),
    sym("Theta", 0, 2, Face::Symbol, b'Q', 'Θ', 0),
    sym("Xi", 0, 4, Face::Symbol, b'X', 'Ξ', 0),
    sym("alpha", 1, 11, Face::Symbol, b'a', 'α', 0),
    sym("beta", 1, 12, Face::Symbol, b'b', 'β', 0),
    sym("cap", 2, 92, Face::Symbol, 0xC7, '∩', 1),
    sym("cdot", 2, 1, Face::Symbol, 0xD7, '⋅', 1),
    sym("chi", 1, 31, Face::Symbol, b'c', 'χ', 0),
    sym("circ", 2, 14, Face::Symbol, 0xB0, '∘', 1),
    sym("cong", 2, 25, Face::Symbol, 0x40, '≅', 2),
    sym("cup", 2, 91, Face::Symbol, 0xC8, '∪', 1),
    sym("delta", 1, 14, Face::Symbol, b'd', 'δ', 0),
    sym("ell", 1, 96, Face::Italic, b'l', 'ℓ', 0),
    sym("epsilon", 1, 15, Face::Symbol, b'e', 'ε', 0),
    sym("equiv", 2, 17, Face::Symbol, 0xBA, '≡', 2),
    sym("eta", 1, 17, Face::Symbol, b'h', 'η', 0),
    sym("exists", 2, 57, Face::Symbol, 0x24, '∃', 0),
    sym("forall", 2, 56, Face::Symbol, 0x22, '∀', 0),
    sym("gamma", 1, 13, Face::Symbol, b'g', 'γ', 0),
    sym("ge", 2, 21, Face::Symbol, 0xB3, '≥', 2),
    sym("geq", 2, 21, Face::Symbol, 0xB3, '≥', 2),
    sym("in", 2, 50, Face::Symbol, 0xCE, '∈', 2),
    sym("infty", 2, 49, Face::Symbol, 0xA5, '∞', 0),
    sym("iota", 1, 19, Face::Symbol, b'i', 'ι', 0),
    sym("kappa", 1, 20, Face::Symbol, b'k', 'κ', 0),
    sym("lambda", 1, 21, Face::Symbol, b'l', 'λ', 0),
    sym("langle", 2, 104, Face::Symbol, 0xE1, '⟨', 0),
    sym("le", 2, 20, Face::Symbol, 0xA3, '≤', 2),
    sym("leq", 2, 20, Face::Symbol, 0xA3, '≤', 2),
    sym("llcorner", 2, 98, Face::Symbol, 0xEB, '⌞', 0),
    sym("lrcorner", 2, 99, Face::Symbol, 0xFB, '⌟', 0),
    sym("mu", 1, 22, Face::Symbol, b'm', 'μ', 0),
    sym("nabla", 2, 114, Face::Symbol, 0xD1, '∇', 0),
    sym("ne", 2, 54, Face::Symbol, 0xB9, '≠', 2),
    sym("neq", 2, 54, Face::Symbol, 0xB9, '≠', 2),
    sym("nu", 1, 23, Face::Symbol, b'n', 'ν', 0),
    sym("omega", 1, 33, Face::Symbol, b'w', 'ω', 0),
    sym("oplus", 2, 8, Face::Symbol, 0xC5, '⊕', 1),
    sym("otimes", 2, 10, Face::Symbol, 0xC4, '⊗', 1),
    sym("partial", 1, 64, Face::Symbol, 0xB6, '∂', 0),
    sym("phi", 1, 30, Face::Symbol, b'f', 'φ', 0),
    sym("pi", 1, 25, Face::Symbol, b'p', 'π', 0),
    sym("pm", 2, 6, Face::Symbol, 0xB1, '±', 1),
    sym("psi", 1, 32, Face::Symbol, b'y', 'ψ', 0),
    sym("rangle", 2, 105, Face::Symbol, 0xF1, '⟩', 0),
    sym("rho", 1, 26, Face::Symbol, b'r', 'ρ', 0),
    sym("sigma", 1, 27, Face::Symbol, b's', 'σ', 0),
    sym("sim", 2, 24, Face::Symbol, 0x7E, '∼', 2),
    sym("subset", 2, 26, Face::Symbol, 0xCC, '⊂', 2),
    sym("subseteq", 2, 18, Face::Symbol, 0xCD, '⊆', 2),
    sym("sum", 2, 80, Face::Symbol, 0xE5, '∑', 0),
    sym("supset", 2, 27, Face::Symbol, 0xC9, '⊃', 2),
    sym("tau", 1, 28, Face::Symbol, b't', 'τ', 0),
    sym("theta", 1, 18, Face::Symbol, b'q', 'θ', 0),
    sym("times", 2, 2, Face::Symbol, 0xB4, '×', 1),
    sym("to", 2, 33, Face::Symbol, 0xAE, '→', 2),
    sym("ulcorner", 2, 100, Face::Symbol, 0xE9, '⌜', 0),
    sym("urcorner", 2, 101, Face::Symbol, 0xF9, '⌝', 0),
    sym("xi", 1, 24, Face::Symbol, b'x', 'ξ', 0),
    sym("zeta", 1, 16, Face::Symbol, b'z', 'ζ', 0),
];

fn symbol(name: &[u8]) -> Option<&'static Sym> {
    SYMS.binary_search_by(|s| s.name.as_bytes().cmp(name)).ok().map(|i| &SYMS[i])
}

/// A font size command.
fn size_of(cmd: &[u8]) -> Option<f64> {
    Some(match cmd {
        b"\\tiny" => 5.0,
        b"\\scriptsize" => 7.0,
        b"\\footnotesize" => 8.0,
        b"\\small" => 9.0,
        b"\\normalsize" => 10.0,
        b"\\large" => 12.0,
        b"\\Large" => 14.4,
        b"\\LARGE" => 17.28,
        b"\\huge" => 20.74,
        b"\\Huge" => 24.88,
        _ => return None,
    })
}

/// Computer Modern at its usual design sizes, as far as it could be found.
struct Family {
    tfm: [Option<Tfm>; 3], // 10, 7, 5
}

impl Family {
    fn load(name: &str, dirs: &[std::path::PathBuf]) -> Family {
        let at = |size: u32| tfm::load(&format!("{name}{size}"), dirs);
        Family { tfm: [at(10), at(7), at(5)] }
    }

    /// The nearest design size at or above: what LaTeX's math sizes use.
    fn at(&self, size: f64) -> Option<&Tfm> {
        let i = if size >= 8.0 {
            0
        } else if size >= 6.0 {
            1
        } else {
            2
        };
        self.tfm[i].as_ref().or(self.tfm[0].as_ref())
    }
}

/// The placeholder typesetter; see the module documentation.
pub struct Builtin {
    fams: [Family; 3],
    params: FontParams,
    /// True if the TFM files were found.
    pub has_metrics: bool,
}

impl Builtin {
    pub fn new() -> Builtin {
        Builtin::with_dirs(&tfm::search_dirs())
    }

    pub fn with_dirs(dirs: &[std::path::PathBuf]) -> Builtin {
        let fams = [Family::load("cmr", dirs), Family::load("cmmi", dirs), Family::load("cmsy", dirs)];
        let cmr = fams[0].tfm[0].as_ref();
        let cmsy = fams[2].tfm[0].as_ref();
        let cmex = tfm::load("cmex10", dirs);
        let d = FontParams::ARTICLE_10PT;
        let param = |t: Option<&Tfm>, n: usize, default: f64| t.and_then(|t| t.param(n)).map_or(default, |v| v * 10.0);
        let params = FontParams {
            ex: param(cmr, 5, d.ex),
            em: param(cmr, 6, d.em),
            rt: param(cmex.as_ref(), 8, d.rt),
            ax: param(cmsy, 22, d.ax),
        };
        Builtin { has_metrics: cmr.is_some() && fams[1].tfm[0].is_some(), fams, params }
    }

    fn metrics(&self, fam: u8, code: u8, size: f64) -> (f64, f64, f64, f64) {
        let t = self.fams[usize::from(fam)].at(size).and_then(|t| t.char(code));
        match t {
            Some(c) => (c.width * size, c.height * size, c.depth * size, c.italic * size),
            // No TFM: the proportions of a typical letter.
            None => (0.5 * size, 0.68 * size, 0.0, 0.0),
        }
    }
}

impl Default for Builtin {
    fn default() -> Self {
        Builtin::new()
    }
}

/// Spaces around binary operators and relations in text style, in points at 10pt: 4mu and 5mu of cmsy10's quad.
const MEDMUSKIP: f64 = 2.22222;
const THICKMUSKIP: f64 = 2.77779;

/// The layout state of one box.
struct Run<'t> {
    t: &'t Builtin,
    out: TypesetBox,
    x: f64,
    /// Whether math spacing applies: not in script sizes.
    spaced: bool,
}

impl Run<'_> {
    #[allow(clippy::too_many_arguments)]
    fn glyph(&mut self, fam: u8, code: u8, face: Face, pdf: u8, unicode: char, size: f64, y: f64) {
        let (w, h, d, ital) = self.t.metrics(fam, code, size);
        self.out.glyphs.push(Glyph { x: self.x, y, size, face, pdf, unicode });
        self.x += w + if fam == 1 { ital } else { 0.0 };
        self.out.dim.h = self.out.dim.h.max(h + y);
        self.out.dim.d = self.out.dim.d.max(d - y);
    }

    fn text_byte(&mut self, b: u8, size: f64, y: f64, upright: bool) {
        let c = char::from(b);
        if !upright && c.is_ascii_alphabetic() {
            self.glyph(1, b, Face::Italic, b, c, size, y);
        } else {
            match b {
                b'-' if !upright => self.binary(2, 0, Face::Roman, b'-', '−', size, y),
                b'+' if !upright => self.binary(0, b'+', Face::Roman, b'+', '+', size, y),
                b'=' if !upright => self.relation(0, b'=', Face::Roman, b'=', '=', size, y),
                b'<' if !upright => self.relation(1, b'<', Face::Roman, b'<', '<', size, y),
                b'>' if !upright => self.relation(1, b'>', Face::Roman, b'>', '>', size, y),
                b',' if !upright => self.glyph(1, 59, Face::Roman, b',', ',', size, y),
                b'.' if !upright => self.glyph(1, 58, Face::Roman, b'.', '.', size, y),
                b'*' if !upright => self.binary(2, 3, Face::Roman, b'*', '∗', size, y),
                _ if b < 0x80 => self.glyph(0, b, Face::Roman, b, c, size, y),
                _ => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn binary(&mut self, fam: u8, code: u8, face: Face, pdf: u8, unicode: char, size: f64, y: f64) {
        let gap = if self.spaced && !self.out.glyphs.is_empty() { MEDMUSKIP * size / 10.0 } else { 0.0 };
        self.x += gap;
        self.glyph(fam, code, face, pdf, unicode, size, y);
        self.x += gap;
    }

    #[allow(clippy::too_many_arguments)]
    fn relation(&mut self, fam: u8, code: u8, face: Face, pdf: u8, unicode: char, size: f64, y: f64) {
        let gap = if self.spaced && !self.out.glyphs.is_empty() { THICKMUSKIP * size / 10.0 } else { 0.0 };
        self.x += gap;
        self.glyph(fam, code, face, pdf, unicode, size, y);
        self.x += gap;
    }

    /// The brace group or single token at `i`.  Returns its range and what follows.
    fn group(src: &[u8], i: usize) -> (usize, usize, usize) {
        let mut i = i;
        while i < src.len() && src[i] == b' ' {
            i += 1;
        }
        if i >= src.len() {
            return (i, i, i);
        }
        if src[i] == b'{' {
            let mut depth = 0;
            let mut j = i;
            while j < src.len() {
                match src[j] {
                    b'\\' => j += 1,
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            return (i + 1, j, j + 1);
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            return (i + 1, src.len(), src.len());
        }
        if src[i] == b'\\' {
            let mut j = i + 1;
            while j < src.len() && src[j].is_ascii_alphabetic() {
                j += 1;
            }
            if j == i + 1 {
                j = (i + 2).min(src.len());
            }
            return (i, j, j);
        }
        (i, i + 1, i + 1)
    }

    /// Lay out `src` at `size`, raised by `y`.
    fn scan(&mut self, src: &[u8], mut size: f64, y: f64, mut upright: bool, depth: u32) {
        if depth > 16 {
            return;
        }
        let mut i = 0;
        while i < src.len() {
            let b = src[i];
            match b {
                b'\\' => {
                    let mut j = i + 1;
                    while j < src.len() && src[j].is_ascii_alphabetic() {
                        j += 1;
                    }
                    if j == i + 1 {
                        // A control symbol: its character, unless it is spacing.
                        if let Some(&c) = src.get(i + 1) {
                            if matches!(c, b'{' | b'}' | b'%' | b'#' | b'$' | b'&' | b'_') {
                                self.glyph(0, c, Face::Roman, c, char::from(c), size, y);
                            } else if c == b' ' || c == b',' || c == b';' || c == b':' {
                                self.x += 0.25 * size;
                            }
                        }
                        i += 2;
                        continue;
                    }
                    let name = &src[i + 1..j];
                    i = j;
                    if let Some(s) = symbol(name) {
                        match s.class {
                            1 => self.binary(s.fam, s.code, s.face, s.pdf, s.unicode, size, y),
                            2 => self.relation(s.fam, s.code, s.face, s.pdf, s.unicode, size, y),
                            _ => self.glyph(s.fam, s.code, s.face, s.pdf, s.unicode, size, y),
                        }
                        continue;
                    }
                    match name {
                        b"mathrm" | b"text" | b"mbox" | b"operatorname" | b"textrm" | b"mathbf" | b"mathsf" | b"mathtt" | b"mathbb" | b"mathcal"
                        | b"mathscr" | b"mathfrak" | b"textbf" | b"textit" => {
                            let (a, e, next) = Self::group(src, i);
                            self.scan(&src[a..e], size, y, true, depth + 1);
                            i = next;
                        }
                        b"frac" | b"dfrac" | b"tfrac" => {
                            let (a, e, next) = Self::group(src, i);
                            let (c, f, next2) = Self::group(src, next);
                            self.scan(&src[a..e], size, y, upright, depth + 1);
                            self.glyph(0, b'/', Face::Roman, b'/', '/', size, y);
                            self.scan(&src[c..f], size, y, upright, depth + 1);
                            i = next2;
                        }
                        b"phantom" | b"vphantom" | b"hphantom" => {
                            let (_, _, next) = Self::group(src, i);
                            i = next;
                        }
                        b"quad" => self.x += size,
                        b"qquad" => self.x += 2.0 * size,
                        b"displaystyle" | b"textstyle" => size = size.max(10.0),
                        b"scriptstyle" => size = 7.0 * size / 10.0,
                        b"scriptscriptstyle" => size = 5.0 * size / 10.0,
                        b"rm" | b"mathstrut" | b"strut" | b"limits" | b"nolimits" | b"left" | b"right" | b"big" | b"Big" | b"bigg" | b"Bigg" => {
                            upright = upright || name == b"rm";
                        }
                        _ => {
                            // A name without a glyph here, such as \ker: its letters, upright.
                            for &c in name {
                                self.text_byte(c, size, y, true);
                            }
                        }
                    }
                }
                b'{' => {
                    let (a, e, next) = Self::group(src, i);
                    self.scan(&src[a..e], size, y, upright, depth + 1);
                    i = next;
                }
                b'}' | b'$' | b'&' | b'~' => i += 1,
                b'^' | b'_' => {
                    let (a, e, next) = Self::group(src, i + 1);
                    let raise = if b == b'^' { 0.413 * size } else { -0.15 * size };
                    let inner = (0.7 * size).max(5.0);
                    let saved = self.spaced;
                    self.spaced = false;
                    self.scan(&src[a..e], inner, y + raise, upright, depth + 1);
                    self.spaced = saved;
                    i = next;
                }
                b' ' | b'\n' | b'\t' | b'\r' => {
                    if upright {
                        self.x += 0.33333 * size;
                    }
                    i += 1;
                }
                _ => {
                    self.text_byte(b, size, y, upright);
                    i += 1;
                }
            }
        }
    }
}

impl Typesetter for Builtin {
    fn font_params(&self) -> FontParams {
        self.params
    }

    fn typeset(&mut self, req: &Request<'_>) -> TypesetBox {
        let (size, spaced) = match req.style {
            Style::Cell(font) => (size_of(font).unwrap_or(10.0), true),
            Style::Label(LabelFont::Script, _) => {
                if req.math {
                    (7.0, false)
                } else {
                    (10.0, true)
                }
            }
            Style::Label(LabelFont::Text, _) => (10.0, true),
            Style::Label(LabelFont::Command(_), cmd) => (size_of(cmd).unwrap_or(10.0), true),
        };
        let mut run = Run { t: self, out: TypesetBox::default(), x: 0.0, spaced };
        for (n, src) in req.runs.iter().enumerate() {
            if n > 0 && !req.math {
                run.x += 0.33333 * size;
            }
            run.scan(src, size, 0.0, !req.math, 0);
        }
        let mut out = run.out;
        out.dim.w = run.x;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_are_sorted() {
        assert!(SYMS.windows(2).all(|w| w[0].name < w[1].name));
    }

    fn set(t: &mut Builtin, src: &str, style: Style<'_>, math: bool) -> TypesetBox {
        t.typeset(&Request { runs: &[src.as_bytes()], math, style })
    }

    #[test]
    fn without_metrics_there_is_still_a_box() {
        let mut t = Builtin::with_dirs(&[]);
        assert!(!t.has_metrics);
        assert_eq!(t.font_params(), FontParams::ARTICLE_10PT);
        let b = set(&mut t, "AB", Style::Cell(b""), true);
        assert_eq!(b.glyphs.len(), 2);
        assert_eq!(b.dim.w, 10.0);
        let b = set(&mut t, r"\alpha", Style::Label(LabelFont::Script, b""), true);
        assert_eq!(b.glyphs[0].face, Face::Symbol);
        assert_eq!(b.glyphs[0].pdf, b'a');
        assert_eq!(b.glyphs[0].size, 7.0);
    }

    #[test]
    fn no_source_breaks_it() {
        let mut t = Builtin::with_dirs(&[]);
        let many = "{".repeat(100);
        for s in ["", "\\", "^", "_", "{", "}", "\\frac", "\\frac{a", "\\mathrm", "a^", "\\ ", "\u{e9}", "^^", many.as_str()] {
            set(&mut t, s, Style::Cell(b""), true);
            set(&mut t, s, Style::Label(LabelFont::Text, b""), false);
        }
    }

    #[test]
    fn scripts_and_fractions() {
        let mut t = Builtin::with_dirs(&[]);
        let b = set(&mut t, "x^2_y", Style::Cell(b""), true);
        assert_eq!(b.glyphs.len(), 3);
        assert!(b.glyphs[1].y > 0.0 && b.glyphs[2].y < 0.0);
        assert!(b.glyphs[1].size < 10.0);
        let b = set(&mut t, r"\frac{a}{b}", Style::Cell(b""), true);
        let text: String = b.glyphs.iter().map(|g| g.unicode).collect();
        assert_eq!(text, "a/b");
        let b = set(&mut t, r"\mathrm{Hom}", Style::Cell(b""), true);
        assert!(b.glyphs.iter().all(|g| g.face == Face::Roman));
        let b = set(&mut t, r"\ker f", Style::Cell(b""), true);
        let text: String = b.glyphs.iter().map(|g| g.unicode).collect();
        assert_eq!(text, "kerf");
    }

    /// If Computer Modern is installed, single letters measure what pdflatex measured
    /// (crates/commute/testdata/cd/01-square.tsv).
    #[test]
    fn metrics_are_computer_moderns_where_installed() {
        let mut t = Builtin::new();
        if !t.has_metrics {
            println!("no TFM files here; not compared");
            return;
        }
        let p = t.font_params();
        let f = FontParams::ARTICLE_10PT;
        assert!((p.ex - f.ex).abs() < 1e-4 && (p.em - f.em).abs() < 1e-4 && (p.rt - f.rt).abs() < 1e-4 && (p.ax - f.ax).abs() < 1e-4, "{p:?}");
        for (src, w, h, d) in [("A", 7.50002, 6.83331, 0.0), ("B", 8.0868, 6.83331, 0.0), ("C", 7.86249, 6.83331, 0.0), ("D", 8.55695, 6.83331, 0.0)] {
            let b = set(&mut t, src, Style::Cell(b""), true);
            assert!((b.dim.w - w).abs() < 1e-3 && (b.dim.h - h).abs() < 1e-3 && (b.dim.d - d).abs() < 1e-3, "{src}: {:?}", b.dim);
        }
        for (src, w, h, d) in [("f", 4.68408, 4.8611, 1.3611), ("g", 4.15245, 3.01389, 1.3611), ("h", 4.68175, 4.8611, 0.0), ("k", 4.42017, 4.8611, 0.0)] {
            let b = set(&mut t, src, Style::Label(LabelFont::Script, b""), true);
            assert!((b.dim.w - w).abs() < 1e-3 && (b.dim.h - h).abs() < 1e-3 && (b.dim.d - d).abs() < 1e-3, "{src}: {:?}", b.dim);
        }
    }

    #[test]
    fn font_commands_set_the_size() {
        let mut t = Builtin::with_dirs(&[]);
        let b = set(&mut t, "A", Style::Cell(b"\\tiny"), true);
        assert_eq!(b.glyphs[0].size, 5.0);
        let b = set(&mut t, "A", Style::Label(LabelFont::Text, b""), true);
        assert_eq!(b.glyphs[0].size, 10.0);
    }
}
