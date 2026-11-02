//! Which control sequences a cell or label may contain.
//!
//! The oracle expands macros while it scans a cell, so `\pb` may be `\arrow[...]` and `\nextrow` may be `\\` (spec 3.2,
//! test 27).  A scan of unexpanded text cannot see that, and a diagram with an arrow silently missing would be the wrong
//! diagram.  So the parser asks about every control sequence it passes over, and refuses the diagram on a no.
//!
//! The engine implements this with the real meaning from eqtb.  [`StandardLatex`] is the answer without an engine: the
//! names LaTeX and the AMS packages define, assuming nobody redefined them.

use crate::csnames::STANDARD;

pub trait Policy {
    /// May `\name`, unexpanded, stay in a cell or label?  `name` has no backslash.
    fn control_sequence(&self, name: &[u8]) -> bool;
    /// May `\begin{name}` appear in a cell or label?
    fn environment(&self, name: &[u8]) -> bool;
}

/// LaTeX's and the AMS packages' symbols, spacing, fonts and math environments.
#[derive(Copy, Clone, Debug, Default)]
pub struct StandardLatex;

impl Policy for StandardLatex {
    fn control_sequence(&self, name: &[u8]) -> bool {
        STANDARD.binary_search_by(|s| s.as_bytes().cmp(name)).is_ok()
    }

    fn environment(&self, name: &[u8]) -> bool {
        matches!(
            name,
            b"matrix"
                | b"pmatrix"
                | b"bmatrix"
                | b"Bmatrix"
                | b"vmatrix"
                | b"Vmatrix"
                | b"smallmatrix"
                | b"array"
                | b"cases"
                | b"aligned"
                | b"alignedat"
                | b"gathered"
                | b"split"
                | b"subarray"
        )
    }
}

/// Everything may stay.  For a caller that has already checked, and for measuring what the parser alone would accept.
#[derive(Copy, Clone, Debug, Default)]
pub struct AllowAll;

impl Policy for AllowAll {
    fn control_sequence(&self, _: &[u8]) -> bool {
        true
    }

    fn environment(&self, _: &[u8]) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted_for_binary_search() {
        assert!(STANDARD.windows(2).all(|w| w[0].as_bytes() < w[1].as_bytes()));
    }

    #[test]
    fn hides_nothing_structural() {
        for s in ["arrow", "ar", "rar", "tikzcdset", "tikzset", "def", "let", "par", "path", "draw", "node", "pgfmatrixnextcell"] {
            assert!(!StandardLatex.control_sequence(s.as_bytes()), "{s}");
        }
        for s in ["alpha", "mathrm", "frac", "displaystyle", "lrcorner", "square", "text", "phantom", "Huge"] {
            assert!(StandardLatex.control_sequence(s.as_bytes()), "{s}");
        }
        assert!(StandardLatex.environment(b"pmatrix"));
        assert!(!StandardLatex.environment(b"tikzcd"));
        assert!(!StandardLatex.environment(b"mine"));
    }
}
