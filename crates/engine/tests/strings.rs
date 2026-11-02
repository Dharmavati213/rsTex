//! strings.rs, Part 4.  What the strings hold reaches the oracle's output through print: the first 256 are
//! exercised byte for byte by the `all_*` cases of tests/print.rs, and these are the routines themselves.

use rstex_engine::access::*;
use rstex_engine::state::*;

fn started() -> &'static mut Tex {
    let t = Tex::alloc();
    t.pool_size = 100000;
    t.max_strings = 1000;
    assert!(t.get_strings_started());
    t
}

fn text(t: &Tex, s: i32) -> Vec<u8> {
    t.str_pool.range(t.str_start[s], t.str_start[s + 1]).to_vec()
}

/// §48-§49: `^^` notation for what cannot be printed, two lower-case hex digits from 128 up.
#[test]
fn the_first_256_strings() {
    let t = started();
    for k in 0..256 {
        let want: Vec<u8> = if (32..=126).contains(&k) {
            vec![k as u8]
        } else if k < 64 {
            vec![b'^', b'^', (k + 64) as u8]
        } else if k < 128 {
            vec![b'^', b'^', (k - 64) as u8]
        } else {
            format!("^^{:02x}", k).into_bytes()
        };
        assert_eq!(text(t, k), want, "string {k}");
    }
    Tex::dealloc(t);
}

/// §51: the table of access.rs, in order, right after them, so that each `S_*` is that string.
#[test]
fn the_well_known_strings() {
    let t = started();
    for (s, want) in [
        (S_EMPTY, &b""[..]),
        (S_DOT_TEX, b".tex"),
        (S_DOT_LOG, b".log"),
        (S_DOT_DVI, b".dvi"),
        (S_DOT_PDF, b".pdf"),
        (S_DOT_FLS, b".fls"),
        (S_DOT_FMT, b".fmt"),
        (S_DOT_VF, b".vf"),
        (S_TEXPUT, b"texput"),
        (S_INITEX_IDENT, b" (INITEX)"),
        (S_FONT, b"FONT"),
        (S_NULLFONT, b"nullfont"),
        (S_NOTEXPANDED, b"notexpanded:"),
        (S_PDFPRIMITIVE, b"pdfprimitive"),
        (S_ENDTEMPLATE, b"endtemplate"),
        (S_INACCESSIBLE, b"inaccessible"),
        (S_ENDWRITE, b"endwrite"),
        (S_PDFTEXSPACE, b"pdftexspace"),
        (S_QUESTIONS, b"???"),
        (S_QUESTION, b"?"),
    ] {
        assert_eq!(text(t, s), want, "string {s}");
    }
    assert_eq!(t.str_ptr, S_FIRST_FREE);
    assert_eq!(t.str_start[t.str_ptr], t.pool_ptr);
    Tex::dealloc(t);
}

/// §51's `loadpoolstrings`: it gives up on `i>=spare_size`, and the message goes to the terminal directly.
#[test]
fn a_pool_too_small_is_refused() {
    let total: i32 = WELL_KNOWN_STRINGS.iter().map(|s| s.len() as i32).sum();
    // The 256 single characters do not count against the spare size; only the table does.
    for (spare, ok) in [(total + 1, true), (total, false), (1, false)] {
        let t = Tex::alloc();
        t.pool_size = 100000;
        t.string_vacancies = 100000 - spare;
        t.max_strings = 1000;
        assert_eq!(t.get_strings_started(), ok, "spare {spare}");
        let want: &[u8] = if ok { b"" } else { b"! You have to increase POOLSIZE.\n" };
        assert_eq!(&t.host.term_buf[..], want, "spare {spare}");
        Tex::dealloc(t);
    }
}

/// §43, §44, §41, §40
#[test]
fn make_string_and_its_macros() {
    let t = started();
    let n = t.str_ptr;
    for &c in b"abc" {
        t.append_char(c as i32);
    }
    assert_eq!(t.cur_length(), 3);
    let s = t.make_string();
    assert_eq!(s, n);
    assert_eq!(t.str_ptr, n + 1);
    assert_eq!(t.length(s), 3);
    assert_eq!(text(t, s), b"abc");
    assert_eq!(t.cur_length(), 0);
    let e = t.make_string();
    assert_eq!(t.length(e), 0);
    t.flush_string();
    t.flush_string();
    assert_eq!(t.str_ptr, n);
    assert_eq!(t.pool_ptr, t.str_start[n]);
    Tex::dealloc(t);
}

fn make(t: &mut Tex, b: &[u8]) -> i32 {
    for &c in b {
        t.append_char(c as i32);
    }
    t.make_string()
}

/// §45: `buffer` from position `k` holds at least the string's length; what follows does not count.
#[test]
fn str_eq_buf_compares_only_the_string() {
    let t = started();
    let s = make(t, b"hello");
    let e = make(t, b"");
    for (i, &c) in b"xxhelloyy".iter().enumerate() {
        t.buffer.put(i as i32, c as i32);
    }
    assert!(t.str_eq_buf(s, 2));
    assert!(!t.str_eq_buf(s, 1));
    assert!(!t.str_eq_buf(s, 3));
    assert!(t.str_eq_buf(e, 0));
    t.buffer.put(6, b'O' as i32);
    assert!(!t.str_eq_buf(s, 2));
    Tex::dealloc(t);
}

/// §46: strings of different lengths differ, whatever their prefixes.
#[test]
fn str_eq_str_compares_length_and_bytes() {
    let t = started();
    let a = make(t, b"hello");
    let b = make(t, b"hello");
    let c = make(t, b"hellp");
    let d = make(t, b"hell");
    let e = make(t, b"");
    assert!(t.str_eq_str(a, a));
    assert!(t.str_eq_str(a, b));
    assert!(!t.str_eq_str(a, c));
    assert!(!t.str_eq_str(a, d));
    assert!(!t.str_eq_str(d, a));
    assert!(t.str_eq_str(e, S_EMPTY));
    assert!(!t.str_eq_str(e, a));
    assert!(t.str_eq_str(b'?' as i32, S_QUESTION));
    Tex::dealloc(t);
}

/// §42's macro: the message is `overflow`'s, tested with errors.rs; here, only that the test is at the limit.
#[test]
fn str_room_at_the_limit_does_not_overflow() {
    let t = started();
    t.pool_size = t.pool_ptr + 5;
    t.str_room(5);
    for &c in b"abcde" {
        t.append_char(c as i32);
    }
    assert_eq!(t.pool_ptr, t.pool_size);
    Tex::dealloc(t);
}
