//! Reading a TFM file: widths, heights, depths, italic corrections and font parameters.  Enough for the placeholder
//! typesetter; ligatures and kerns are not read.  crates/fontfile has the real reader once the engine is here.

use std::path::{Path, PathBuf};

/// A loaded font metric file, in units of its design size.
#[derive(Clone, Debug, Default)]
pub struct Tfm {
    bc: usize,
    chars: Vec<[u8; 4]>,
    widths: Vec<f64>,
    heights: Vec<f64>,
    depths: Vec<f64>,
    italics: Vec<f64>,
    params: Vec<f64>,
}

/// One character's metrics, in units of the font's size.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct CharMetrics {
    pub width: f64,
    pub height: f64,
    pub depth: f64,
    pub italic: f64,
}

fn fix_word(b: &[u8]) -> f64 {
    f64::from(i32::from_be_bytes([b[0], b[1], b[2], b[3]])) / f64::from(1u32 << 20)
}

impl Tfm {
    /// Parse a TFM file; `None` if it is malformed.
    pub fn parse(data: &[u8]) -> Option<Tfm> {
        if data.len() < 24 {
            return None;
        }
        let half = |i: usize| usize::from(u16::from_be_bytes([data[2 * i], data[2 * i + 1]]));
        let (lf, lh, bc, ec, nw, nh, nd, ni, nl, nk, ne, np) =
            (half(0), half(1), half(2), half(3), half(4), half(5), half(6), half(7), half(8), half(9), half(10), half(11));
        if lf * 4 > data.len() || bc > ec + 1 || ec > 255 {
            return None;
        }
        let nc = ec + 1 - bc;
        if lf != 6 + lh + nc + nw + nh + nd + ni + nl + nk + ne + np {
            return None;
        }
        let words = |start: usize, n: usize| -> Vec<f64> {
            (0..n).map(|i| fix_word(&data[4 * (start + i)..4 * (start + i) + 4])).collect()
        };
        let char_at = 6 + lh;
        let width_at = char_at + nc;
        let height_at = width_at + nw;
        let depth_at = height_at + nh;
        let italic_at = depth_at + nd;
        let param_at = italic_at + ni + nl + nk + ne;
        let chars = (0..nc)
            .map(|i| {
                let p = 4 * (char_at + i);
                [data[p], data[p + 1], data[p + 2], data[p + 3]]
            })
            .collect();
        Some(Tfm {
            bc,
            chars,
            widths: words(width_at, nw),
            heights: words(height_at, nh),
            depths: words(depth_at, nd),
            italics: words(italic_at, ni),
            params: words(param_at, np),
        })
    }

    /// The metrics of character `code`; `None` if the font does not have it.
    pub fn char(&self, code: u8) -> Option<CharMetrics> {
        let info = self.chars.get(usize::from(code).checked_sub(self.bc)?)?;
        if info[0] == 0 {
            return None;
        }
        Some(CharMetrics {
            width: *self.widths.get(usize::from(info[0]))?,
            height: *self.heights.get(usize::from(info[1] >> 4))?,
            depth: *self.depths.get(usize::from(info[1] & 15))?,
            italic: *self.italics.get(usize::from(info[2] >> 2))?,
        })
    }

    /// `\fontdimen n`, in units of the font's size.
    pub fn param(&self, n: usize) -> Option<f64> {
        self.params.get(n.checked_sub(1)?).copied()
    }
}

/// Where to look for a TFM until crates/kpse is ready: `$COMMUTE_TFM_DIR`, then `$TEXMFDIST`, then the usual TeX Live places.
pub fn search_dirs() -> Vec<PathBuf> {
    const CM: &str = "fonts/tfm/public/cm";
    let mut out = Vec::new();
    if let Some(v) = std::env::var_os("COMMUTE_TFM_DIR") {
        out.extend(std::env::split_paths(&v));
    }
    if let Some(v) = std::env::var_os("TEXMFDIST") {
        out.extend(std::env::split_paths(&v).map(|p| p.join(CM)));
    }
    for root in ["/usr/share/texmf-dist", "/usr/share/texlive/texmf-dist", "/usr/local/share/texmf-dist", "C:/texlive/texmf-dist"] {
        out.push(Path::new(root).join(CM));
    }
    out
}

/// Load `<name>.tfm` from the first directory that has it.
pub fn load(name: &str, dirs: &[PathBuf]) -> Option<Tfm> {
    dirs.iter().find_map(|d| std::fs::read(d.join(format!("{name}.tfm"))).ok()).and_then(|b| Tfm::parse(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A TFM made here: two characters and one parameter.
    pub(crate) fn sample() -> Vec<u8> {
        let (lh, bc, ec, nw, nh, nd, ni, nl, nk, ne, np) = (2u16, 65u16, 66u16, 3u16, 2u16, 2u16, 2u16, 0u16, 0u16, 0u16, 3u16);
        let lf = 6 + lh + (ec + 1 - bc) + nw + nh + nd + ni + nl + nk + ne + np;
        let mut v = Vec::new();
        for h in [lf, lh, bc, ec, nw, nh, nd, ni, nl, nk, ne, np] {
            v.extend_from_slice(&h.to_be_bytes());
        }
        let fw = |x: f64| ((x * f64::from(1u32 << 20)) as i32).to_be_bytes();
        v.extend_from_slice(&[0; 8]); // header
        v.extend_from_slice(&[1, 0x10, 1 << 2, 0]); // A: width 1, height 1, depth 0, italic 1
        v.extend_from_slice(&[2, 0x01, 0, 0]); // B: width 2, height 0, depth 1, no italic
        for x in [0.0, 0.75, 0.5] {
            v.extend_from_slice(&fw(x));
        }
        for x in [0.0, 0.7] {
            v.extend_from_slice(&fw(x));
        }
        for x in [0.0, 0.2] {
            v.extend_from_slice(&fw(x));
        }
        for x in [0.0, 0.1] {
            v.extend_from_slice(&fw(x));
        }
        for x in [0.25, 0.5, 1.0] {
            v.extend_from_slice(&fw(x));
        }
        v
    }

    #[test]
    fn reads_a_synthetic_file() {
        let t = Tfm::parse(&sample()).unwrap();
        let a = t.char(65).unwrap();
        assert!((a.width - 0.75).abs() < 1e-5 && (a.height - 0.7).abs() < 1e-5 && a.depth == 0.0 && (a.italic - 0.1).abs() < 1e-5);
        let b = t.char(66).unwrap();
        assert!((b.width - 0.5).abs() < 1e-5 && b.height == 0.0 && (b.depth - 0.2).abs() < 1e-5);
        assert_eq!(t.char(64), None);
        assert_eq!(t.char(67), None);
        assert_eq!(t.param(3), Some(1.0));
        assert_eq!(t.param(0), None);
        assert_eq!(t.param(4), None);
    }

    #[test]
    fn malformed_files_are_refused() {
        assert!(Tfm::parse(&[]).is_none());
        assert!(Tfm::parse(&[0; 24]).is_none());
        let mut s = sample();
        s.truncate(s.len() - 4);
        assert!(Tfm::parse(&s).is_none());
        let mut s = sample();
        s[2] = 0xff;
        assert!(Tfm::parse(&s).is_none());
        for n in 0..sample().len() {
            let _ = Tfm::parse(&sample()[..n]);
        }
    }

    #[test]
    fn a_character_with_an_index_out_of_range_is_absent() {
        let mut s = sample();
        // First char_info byte of A: width index past the table.
        let at = 4 * (6 + 2);
        s[at] = 200;
        let t = Tfm::parse(&s).unwrap();
        assert_eq!(t.char(65), None);
    }
}
