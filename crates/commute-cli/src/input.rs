//! What a file holds: its diagrams, and a preamble if it is a whole document.

/// One diagram: its body's range in the file, and the line it starts on.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub start: usize,
    pub end: usize,
    pub line: i32,
}

/// The file, read as far as this tool needs.
#[derive(Debug, PartialEq, Eq)]
pub struct Input {
    /// What stands before `\begin{document}`, if there is a `\documentclass`.
    pub preamble: Option<(usize, usize)>,
    pub diagrams: Vec<Found>,
}

const BEGIN: &[u8] = b"\\begin{tikzcd}";
const END: &[u8] = b"\\end{tikzcd}";

/// The next `needle` at or after `from` that no `%` on its line hides.
fn find_live(src: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    let mut at = from;
    loop {
        let k = src.get(at..)?.windows(needle.len()).position(|w| w == needle)? + at;
        let line = src[..k].iter().rposition(|&c| c == b'\n').map_or(0, |n| n + 1);
        let mut i = line;
        let mut commented = false;
        while i < k {
            match src[i] {
                b'\\' => i += 1,
                b'%' => {
                    commented = true;
                    break;
                }
                _ => {}
            }
            i += 1;
        }
        if !commented {
            return Some(k);
        }
        at = k + needle.len();
    }
}

fn line_of(src: &[u8], at: usize, from: usize, line: i32) -> i32 {
    line + src[from..at].iter().filter(|&&c| c == b'\n').count() as i32
}

/// The environments in the file, or the whole file as one body if there are none.  A `\begin{tikzcd}` with no end is left out, as the rest of the file is no diagram.
pub fn read(src: &[u8]) -> Input {
    let mut preamble = None;
    let mut from = 0;
    if let Some(c) = find_live(src, b"\\documentclass", 0) {
        if let Some(b) = find_live(src, b"\\begin{document}", c) {
            preamble = Some((c, b));
            from = b;
        }
    }
    let (mut diagrams, mut line, mut at) = (Vec::new(), 1, 0);
    let mut search = from;
    while let Some(b) = find_live(src, BEGIN, search) {
        let start = b + BEGIN.len();
        let Some(end) = find_live(src, END, start) else { break };
        line = line_of(src, b, at, line);
        at = b;
        diagrams.push(Found { start, end, line });
        search = end + END.len();
    }
    if diagrams.is_empty() && preamble.is_none() {
        diagrams.push(Found { start: 0, end: src.len(), line: 1 });
    }
    Input { preamble, diagrams }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bodies<'a>(src: &'a [u8], i: &Input) -> Vec<&'a [u8]> {
        i.diagrams.iter().map(|d| &src[d.start..d.end]).collect()
    }

    #[test]
    fn environments_with_their_lines() {
        let src = b"text\n\\begin{tikzcd}A\\end{tikzcd}\n\n\\begin{tikzcd} B \\\\ C \\end{tikzcd}\nmore\n";
        let i = read(src);
        assert_eq!(bodies(src, &i), vec![&b"A"[..], &b" B \\\\ C "[..]]);
        assert_eq!(i.diagrams[0].line, 2);
        assert_eq!(i.diagrams[1].line, 4);
        assert_eq!(i.preamble, None);
    }

    #[test]
    fn a_body_alone() {
        let src = b"A \\arrow[r] & B";
        let i = read(src);
        assert_eq!(bodies(src, &i), vec![&src[..]]);
    }

    #[test]
    fn comments_hide_and_escaped_percents_do_not() {
        let src = b"% \\begin{tikzcd} no \\end{tikzcd}\n50\\% \\begin{tikzcd} yes % \\end{tikzcd}\n\\end{tikzcd}\n";
        let i = read(src);
        assert_eq!(bodies(src, &i), vec![&b" yes % \\end{tikzcd}\n"[..]]);
    }

    #[test]
    fn a_document_has_a_preamble() {
        let src = b"% head\n\\documentclass{article}\n\\usepackage{x}\n\\begin{document}\n\\begin{tikzcd}A\\end{tikzcd}\n\\end{document}\n";
        let i = read(src);
        let (a, b) = i.preamble.unwrap();
        assert_eq!(&src[a..b], b"\\documentclass{article}\n\\usepackage{x}\n");
        assert_eq!(bodies(src, &i), vec![&b"A"[..]]);
        // None in it: no body either.
        let src = b"\\documentclass{article}\n\\begin{document}\nx\n\\end{document}\n";
        assert!(read(src).diagrams.is_empty());
    }

    #[test]
    fn a_begin_with_no_end_is_not_a_diagram() {
        let src = b"\\begin{tikzcd}A\\end{tikzcd}\n\\begin{tikzcd}B";
        assert_eq!(read(src).diagrams.len(), 1);
    }
}
