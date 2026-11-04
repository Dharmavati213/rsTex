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
    /// If it is a whole document, which a `\begin{document}` says.
    pub preamble: Option<Preamble>,
    pub diagrams: Vec<Found>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Preamble {
    /// Where `\documentclass` and its arguments are, if they are in this file and not in something it inputs.
    pub class: Option<(usize, usize)>,
    /// Where `\begin{document}` is.
    pub document: usize,
}

/// Where the `\documentclass` at `at` ends, its options and its name, or none if it is not as usual.
fn class_end(src: &[u8], at: usize) -> Option<usize> {
    let mut i = at + b"\\documentclass".len();
    let skip = |i: &mut usize| {
        while src.get(*i).is_some_and(u8::is_ascii_whitespace) {
            *i += 1;
        }
    };
    skip(&mut i);
    if src.get(i) == Some(&b'[') {
        i += src[i..].iter().position(|&c| c == b']')? + 1;
        skip(&mut i);
    }
    if src.get(i) != Some(&b'{') {
        return None;
    }
    i += src[i..].iter().position(|&c| c == b'}')? + 1;
    Some(i)
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
    if let Some(document) = find_live(src, b"\\begin{document}", 0) {
        let class = find_live(src[..document].as_ref(), b"\\documentclass", 0).and_then(|c| Some((c, class_end(src, c)?)));
        preamble = Some(Preamble { class, document });
        from = document;
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
        let p = i.preamble.unwrap();
        let (a, b) = p.class.unwrap();
        assert_eq!(&src[a..b], b"\\documentclass{article}");
        assert_eq!(&src[a..p.document], b"\\documentclass{article}\n\\usepackage{x}\n");
        assert_eq!(bodies(src, &i), vec![&b"A"[..]]);
        // Options over more than one line.
        let src = b"\\documentclass[a4paper,\n  12pt]\n{amsart}\n\\begin{document}\n\\end{document}";
        let p = read(src).preamble.unwrap();
        assert_eq!(&src[p.class.unwrap().1..p.document], b"\n");
        // A class in a file that this one inputs.
        let src = b"\\input{header}\n\\begin{document}\n\\begin{tikzcd}A\\end{tikzcd}\n\\end{document}\n";
        let i = read(src);
        assert_eq!(i.preamble, Some(Preamble { class: None, document: 15 }));
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
