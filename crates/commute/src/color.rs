//! xcolor's base colours and mix expressions (`red!50!blue`), as they reach the PDF.  A colour name xcolor would have
//! to look up in the document is not known here; the caller gets Unsupported and real PGF runs.
//!
//! Checked against pdflatex: `black!30!red` gives `0.7 0 0`, `cyan!50!red` gives CMYK `0.5 0.5 0.5 0`, `red!50!cyan`
//! gives RGB `0.5 0.5 0.5`, `red!20!blue!40!green` gives `0.08 0.6 0.32`, `black!50` gives gray.

/// A colour in the model xcolor would write it.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Color {
    Gray(f64),
    Rgb(f64, f64, f64),
    Cmyk(f64, f64, f64, f64),
}

impl Color {
    pub const BLACK: Color = Color::Gray(0.0);
    pub const WHITE: Color = Color::Gray(1.0);
}

#[derive(Copy, Clone, PartialEq)]
enum Model {
    Gray,
    Rgb,
    Cmyk,
}

/// A base colour with the alternate values `\definecolorset` gives it in xcolor.sty.
struct Base {
    name: &'static [u8],
    model: Model,
    gray: f64,
    rgb: [f64; 3],
    cmyk: [f64; 4],
}

const fn b(name: &'static [u8], model: Model, gray: f64, rgb: [f64; 3], cmyk: [f64; 4]) -> Base {
    Base { name, model, gray, rgb, cmyk }
}

static BASES: [Base; 19] = [
    b(b"red", Model::Rgb, 0.3, [1.0, 0.0, 0.0], [0.0, 1.0, 1.0, 0.0]),
    b(b"green", Model::Rgb, 0.59, [0.0, 1.0, 0.0], [1.0, 0.0, 1.0, 0.0]),
    b(b"blue", Model::Rgb, 0.11, [0.0, 0.0, 1.0], [1.0, 1.0, 0.0, 0.0]),
    b(b"brown", Model::Rgb, 0.5475, [0.75, 0.5, 0.25], [0.0, 0.25, 0.5, 0.25]),
    b(b"lime", Model::Rgb, 0.815, [0.75, 1.0, 0.0], [0.25, 0.0, 1.0, 0.0]),
    b(b"orange", Model::Rgb, 0.595, [1.0, 0.5, 0.0], [0.0, 0.5, 1.0, 0.0]),
    b(b"pink", Model::Rgb, 0.825, [1.0, 0.75, 0.75], [0.0, 0.25, 0.25, 0.0]),
    b(b"purple", Model::Rgb, 0.2525, [0.75, 0.0, 0.25], [0.0, 0.75, 0.5, 0.25]),
    b(b"teal", Model::Rgb, 0.35, [0.0, 0.5, 0.5], [0.5, 0.0, 0.0, 0.5]),
    b(b"violet", Model::Rgb, 0.205, [0.5, 0.0, 0.5], [0.0, 0.5, 0.0, 0.5]),
    b(b"cyan", Model::Cmyk, 0.7, [0.0, 1.0, 1.0], [1.0, 0.0, 0.0, 0.0]),
    b(b"magenta", Model::Cmyk, 0.41, [1.0, 0.0, 1.0], [0.0, 1.0, 0.0, 0.0]),
    b(b"yellow", Model::Cmyk, 0.89, [1.0, 1.0, 0.0], [0.0, 0.0, 1.0, 0.0]),
    b(b"olive", Model::Cmyk, 0.4945, [0.5, 0.5, 0.0], [0.0, 0.0, 1.0, 0.5]),
    b(b"black", Model::Gray, 0.0, [0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]),
    b(b"darkgray", Model::Gray, 0.25, [0.25, 0.25, 0.25], [0.0, 0.0, 0.0, 0.75]),
    b(b"gray", Model::Gray, 0.5, [0.5, 0.5, 0.5], [0.0, 0.0, 0.0, 0.5]),
    b(b"lightgray", Model::Gray, 0.75, [0.75, 0.75, 0.75], [0.0, 0.0, 0.0, 0.25]),
    b(b"white", Model::Gray, 1.0, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0, 0.0]),
];

/// The base colours, and what they are when xcolor's model is the natural one: for a caller that has to compare them with the document's.
pub fn base_colors() -> impl Iterator<Item = (&'static [u8], Color)> {
    BASES.iter().map(|b| {
        let c = match b.model {
            Model::Gray => Color::Gray(b.gray),
            Model::Rgb => Color::Rgb(b.rgb[0], b.rgb[1], b.rgb[2]),
            Model::Cmyk => Color::Cmyk(b.cmyk[0], b.cmyk[1], b.cmyk[2], b.cmyk[3]),
        };
        (b.name, c)
    })
}

fn base(name: &[u8]) -> Option<&'static Base> {
    BASES.iter().find(|b| b.name == name)
}

fn percent(s: &[u8]) -> Option<f64> {
    if s.is_empty() || s.len() > 12 {
        return None;
    }
    let (mut mant, mut den, mut seen_dot, mut digits) = (0.0f64, 1.0f64, false, 0);
    for &c in s {
        match c {
            b'0'..=b'9' => {
                mant = mant * 10.0 + f64::from(c - b'0');
                digits += 1;
                if seen_dot {
                    den *= 10.0;
                }
            }
            b'.' if !seen_dot => seen_dot = true,
            _ => return None,
        }
    }
    let v = mant / den;
    if digits == 0 || v > 100.0 { None } else { Some(v / 100.0) }
}

fn trim(s: &[u8]) -> &[u8] {
    let mut s = s;
    while let [b' ' | b'\n' | b'\t', rest @ ..] = s {
        s = rest;
    }
    while let [rest @ .., b' ' | b'\n' | b'\t'] = s {
        s = rest;
    }
    s
}

/// True if `s` could only be a colour expression: it contains `!`, or it names a base colour.
pub(crate) fn looks_like_color(s: &[u8]) -> bool {
    s.contains(&b'!') || base(s).is_some()
}

/// Parse a colour expression.  `None` if a name is not a base colour, or for forms outside `name(!pct(!name)?)*`.
pub(crate) fn parse(s: &[u8]) -> Option<Color> {
    let mut parts = s.split(|&c| c == b'!');
    let first = base(trim(parts.next()?))?;

    // Collect the steps once to find the model: the first colour's, except that gray takes the next colour's model.
    let mut model = first.model;
    {
        let mut it = s.split(|&c| c == b'!').skip(1);
        while let Some(p) = it.next() {
            percent(trim(p))?;
            if let Some(n) = it.next() {
                let n = base(trim(n))?;
                if model == Model::Gray {
                    model = n.model;
                }
            }
        }
    }

    let mut c = [0.0f64; 4];
    fn load(c: &mut [f64; 4], b: &Base, m: Model) {
        match m {
            Model::Gray => c[0] = b.gray,
            Model::Rgb => c[..3].copy_from_slice(&b.rgb),
            Model::Cmyk => *c = b.cmyk,
        }
    }
    load(&mut c, first, model);
    while let Some(p) = parts.next() {
        let p = percent(trim(p))?;
        let next = match parts.next() {
            Some(n) => base(trim(n))?,
            None => base(b"white")?,
        };
        let mut o = [0.0f64; 4];
        load(&mut o, next, model);
        for i in 0..4 {
            c[i] = p * c[i] + (1.0 - p) * o[i];
        }
    }
    Some(match model {
        Model::Gray => Color::Gray(c[0]),
        Model::Rgb => Color::Rgb(c[0], c[1], c[2]),
        Model::Cmyk => Color::Cmyk(c[0], c[1], c[2], c[3]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(c: Color, want: &[f64]) {
        let got: Vec<f64> = match c {
            Color::Gray(g) => vec![g],
            Color::Rgb(r, g, b) => vec![r, g, b],
            Color::Cmyk(a, b, c, d) => vec![a, b, c, d],
        };
        assert_eq!(got.len(), want.len(), "{c:?}");
        for (a, b) in got.iter().zip(want) {
            assert!((a - b).abs() < 1e-9, "{c:?} {want:?}");
        }
    }

    /// Every line of this table is what pdflatex wrote for that colour.
    #[test]
    fn table_from_pdflatex() {
        close(parse(b"black!30!red").unwrap(), &[0.7, 0.0, 0.0]);
        close(parse(b"red!30!black").unwrap(), &[0.3, 0.0, 0.0]);
        close(parse(b"red!30").unwrap(), &[1.0, 0.7, 0.7]);
        close(parse(b"cyan!50").unwrap(), &[0.5, 0.0, 0.0, 0.0]);
        close(parse(b"cyan!50!red").unwrap(), &[0.5, 0.5, 0.5, 0.0]);
        close(parse(b"red!50!cyan").unwrap(), &[0.5, 0.5, 0.5]);
        close(parse(b"black!50").unwrap(), &[0.5]);
        close(parse(b"gray").unwrap(), &[0.5]);
        close(parse(b"orange").unwrap(), &[1.0, 0.5, 0.0]);
        close(parse(b"red!20!blue!40!green").unwrap(), &[0.08, 0.6, 0.32]);
        close(parse(b"white").unwrap(), &[1.0]);
        close(parse(b"purple").unwrap(), &[0.75, 0.0, 0.25]);
        close(parse(b"yellow!50!black").unwrap(), &[0.0, 0.0, 0.5, 0.5]);
        close(parse(b"lightgray").unwrap(), &[0.75]);
        close(parse(b"magenta").unwrap(), &[0.0, 1.0, 0.0, 0.0]);
        close(parse(b"blue!50").unwrap(), &[0.5, 0.5, 1.0]);
    }

    #[test]
    fn unknown_forms_are_refused() {
        for s in [&b"foo"[..], b"", b"red!", b"red!x", b"red!50!foo", b"-red", b".!50!red", b"red!200", b"!!", b"red!!+"] {
            assert_eq!(parse(s), None, "{}", String::from_utf8_lossy(s));
        }
    }
}
