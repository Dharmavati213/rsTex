//! Lexical helpers over the source bytes: spans, brace matching, key lists.  No allocation; nothing here indexes out
//! of bounds on any input.

/// A byte range of the diagram's source buffer, see [`crate::Diagram::text`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub const EMPTY: Span = Span { start: 0, end: 0 };

    #[inline]
    pub(crate) fn new(start: usize, end: usize) -> Span {
        Span { start: start as u32, end: end.max(start) as u32 }
    }

    #[inline]
    pub fn is_empty(self) -> bool {
        self.end <= self.start
    }

    #[inline]
    pub fn get(self, buf: &[u8]) -> &[u8] {
        buf.get(self.start as usize..self.end as usize).unwrap_or(&[])
    }
}

#[inline]
pub(crate) fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\n' | b'\t' | b'\r')
}

#[inline]
pub(crate) fn is_letter(b: u8) -> bool {
    b.is_ascii_alphabetic()
}

#[inline]
pub(crate) fn skip_ws(buf: &[u8], mut i: usize, end: usize) -> usize {
    while i < end && is_ws(buf[i]) {
        i += 1;
    }
    i
}

pub(crate) fn trim(buf: &[u8], s: Span) -> Span {
    let mut a = s.start as usize;
    let mut b = (s.end as usize).min(buf.len());
    while a < b && is_ws(buf[a]) {
        a += 1;
    }
    while b > a && is_ws(buf[b - 1]) {
        b -= 1;
    }
    Span::new(a, b)
}

/// `buf[i]` is `{`.  The index of the matching `}` before `end`.
pub(crate) fn brace_end(buf: &[u8], mut i: usize, end: usize) -> Option<usize> {
    let mut depth = 0i64;
    while i < end {
        match buf[i] {
            b'\\' => {
                i += 2;
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// `buf[i]` is `[`.  The index of the first `]` outside braces before `end` (spec 3.3: the argument is delimited,
/// so `[` does not nest).
pub(crate) fn bracket_end(buf: &[u8], mut i: usize, end: usize) -> Option<usize> {
    let mut depth = 0i64;
    i += 1;
    while i < end {
        match buf[i] {
            b'\\' => {
                i += 2;
                continue;
            }
            b'{' => depth += 1,
            b'}' => depth -= 1,
            b']' if depth == 0 => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// If the whole span is one brace group, its inside, trimmed.  Otherwise the span, trimmed.
pub(crate) fn unbrace(buf: &[u8], s: Span) -> Span {
    let t = trim(buf, s);
    let (a, b) = (t.start as usize, t.end as usize);
    if a < b && buf[a] == b'{' && brace_end(buf, a, b) == Some(b - 1) {
        return trim(buf, Span::new(a + 1, b - 1));
    }
    t
}

/// The items of a comma-separated list, split at commas outside braces and trimmed; empty items are skipped.
pub(crate) struct Items<'a> {
    buf: &'a [u8],
    i: usize,
    end: usize,
}

pub(crate) fn items(buf: &[u8], s: Span) -> Items<'_> {
    Items { buf, i: s.start as usize, end: (s.end as usize).min(buf.len()) }
}

impl Iterator for Items<'_> {
    type Item = Span;

    fn next(&mut self) -> Option<Span> {
        while self.i < self.end {
            let start = self.i;
            let mut depth = 0i64;
            let mut i = start;
            while i < self.end {
                match self.buf[i] {
                    b'\\' => {
                        i += 2;
                        continue;
                    }
                    b'{' => depth += 1,
                    b'}' => depth -= 1,
                    b',' if depth == 0 => break,
                    _ => {}
                }
                i += 1;
            }
            let stop = i.min(self.end);
            self.i = stop + 1;
            let item = trim(self.buf, Span::new(start, stop));
            if !item.is_empty() {
                return Some(item);
            }
        }
        None
    }
}

/// Split an item at the first `=` outside braces.  The value is unbraced.
pub(crate) fn keyval(buf: &[u8], item: Span) -> (Span, Option<Span>) {
    let (a, b) = (item.start as usize, (item.end as usize).min(buf.len()));
    let mut depth = 0i64;
    let mut i = a;
    while i < b {
        match buf[i] {
            b'\\' => {
                i += 2;
                continue;
            }
            b'{' => depth += 1,
            b'}' => depth -= 1,
            b'=' if depth == 0 => {
                return (trim(buf, Span::new(a, i)), Some(unbrace(buf, Span::new(i + 1, b))));
            }
            _ => {}
        }
        i += 1;
    }
    (trim(buf, item), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(buf: &[u8]) -> Span {
        Span::new(0, buf.len())
    }

    fn list(buf: &[u8]) -> Vec<&[u8]> {
        items(buf, s(buf)).map(|x| x.get(buf)).collect()
    }

    #[test]
    fn items_respect_braces_and_skip_empties() {
        assert_eq!(list(b"r, ,\"{f,g}\" , a=b,"), vec![&b"r"[..], b"\"{f,g}\"", b"a=b"]);
        assert_eq!(list(b"a\\,b,c"), vec![&b"a\\,b"[..], b"c"]);
        assert!(list(b"  ").is_empty());
    }

    #[test]
    fn keyval_splits_at_first_equals_outside_braces() {
        let b = b"shorten >= {3pt}";
        let (k, v) = keyval(b, s(b));
        assert_eq!(k.get(b), b"shorten >");
        assert_eq!(v.unwrap().get(b), b"3pt");
        let b = b"{a=b}";
        assert_eq!(keyval(b, s(b)).1, None);
        let b = b"to=";
        assert_eq!(keyval(b, s(b)).1.unwrap().get(b), b"");
    }

    #[test]
    fn bracket_end_does_not_nest_and_skips_braces() {
        assert_eq!(bracket_end(b"[a]b]", 0, 5), Some(2));
        assert_eq!(bracket_end(b"[{]}]", 0, 5), Some(4));
        assert_eq!(bracket_end(b"[a", 0, 2), None);
    }

    #[test]
    fn unbalanced_input_is_harmless() {
        assert_eq!(brace_end(b"{a", 0, 2), None);
        assert_eq!(unbrace(b"{a", s(b"{a")).get(b"{a"), b"{a");
        assert_eq!(list(b"\\"), vec![&b"\\"[..]]);
        let b = b"a\\";
        let _ = keyval(b, s(b));
    }
}
