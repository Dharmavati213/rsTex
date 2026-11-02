//! print.rs against the oracle: crates/engine/testdata/print_oracle.tsv and print_init_oracle.tsv, from
//! tests/oracle/gen_print_tables.py.
//!
//! Each case of the table replays, through the Rust print routines, what the primitive does in the oracle:
//! `write_out`'s selector handling, `token_show` of the tokens, `issue_message`, and so on.  Log and \write
//! streams are real files with a journal, read back here.  Those procedures' own modules are not under test, so what
//! `write_out`, `issue_message`, `print_cs` and `sprint_cs` do is written out below as the WEB has it.

use rstex_engine::access::*;
use rstex_engine::consts::*;
use rstex_engine::host::*;
use rstex_engine::platform::path_to_bytes;
use rstex_engine::state::*;

const TABLE: &str = include_str!("../testdata/print_oracle.tsv");
const INIT: &str = include_str!("../testdata/print_init_oracle.tsv");

fn unesc(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' {
            match b[i + 1] {
                b'n' => out.push(b'\n'),
                b't' => out.push(b'\t'),
                b'\\' => out.push(b'\\'),
                b'x' => {
                    out.push(u8::from_str_radix(&s[i + 2..i + 4], 16).unwrap());
                    i += 2;
                }
                c => panic!("escape {c}"),
            }
            i += 2;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

/// The bytes of an op: ALL, or runs like 61*300,01 in hex.
fn unhex(s: &str) -> Vec<u8> {
    if s == "ALL" {
        return (0..=255).collect();
    }
    let mut out = Vec::new();
    for run in s.split(',').filter(|r| !r.is_empty()) {
        let (b, n) = run.split_once('*').unwrap_or((run, "1"));
        let b = u8::from_str_radix(b, 16).unwrap();
        out.extend(std::iter::repeat(b).take(n.parse().unwrap()));
    }
    out
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("rstex-print-test-{}-{}", std::process::id(), tag));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn journal(t: &Tex, f: FileId) -> &[u8] {
    match &t.host.files[f as usize] {
        Slot::Out(o) => o.journal.as_deref().unwrap(),
        _ => b"",
    }
}

/// §4 without §1584, in INITEX, as the table of initial values has it.
fn just_initialized() -> &'static mut Tex {
    let t = Tex::alloc();
    t.ini_version = true;
    t.pool_size = 1000000;
    t.max_strings = 100000;
    t.hyph_size = 8191;
    t.dvi_buf_size = 16384;
    t.interaction_option = SCROLL_MODE;
    t.print_init_key_variables_to_1571();
    t.print_init_key_variables_from_1629();
    t.print_init_table_entries();
    t
}

/// What these tests print with: §4, §47, and \newlinechar=-1 as the oracle's source has it.
fn initex() -> &'static mut Tex {
    let t = just_initialized();
    assert!(t.get_strings_started());
    t.max_print_line = 79;
    t.error_line = 79;
    t.half_error_line = 50;
    t.set_new_line_char(-1);
    t
}

struct Run {
    t: &'static mut Tex,
    term: Vec<u8>,
}

impl Run {
    fn drain(&mut self) {
        self.term.append(&mut self.t.host.term_buf);
    }

    /// `token_show` of these bytes as tokens of category 12, or 10 for a space if `spaces`: str_toks §490.
    fn show(&mut self, b: &[u8], spaces: bool) {
        let t = &mut *self.t;
        let m = t.tb_mark();
        for &c in b {
            t.tb_push(if spaces && c == b' ' { SPACE_TOKEN } else { OTHER_TOKEN + c as i32 });
        }
        let p = t.tb_commit(m);
        t.token_show(p);
        t.flush_list(p);
    }

    /// §497-§498: what a converting primitive prints, in `selector=new_string`.
    fn conv(&mut self, f: impl FnOnce(&mut Tex)) -> Vec<u8> {
        let t = &mut *self.t;
        let old = t.selector;
        t.selector = NEW_STRING;
        let b = t.pool_ptr;
        f(t);
        t.selector = old;
        let s = t.str_pool.range(b, t.pool_ptr).to_vec();
        t.pool_ptr = b;
        s
    }

    /// §1623 around what `f` prints, for `\immediate\write stream`; §1599 for the stream number.
    fn write_out(&mut self, stream: i32, f: impl FnOnce(&mut Run)) {
        let j = if stream < 0 {
            17
        } else if stream > 15 {
            16
        } else {
            stream
        };
        let old = self.t.selector;
        if self.t.write_open[j] {
            self.t.selector = j;
        } else {
            if j == 17 && self.t.selector == TERM_AND_LOG {
                self.t.selector = LOG_ONLY;
            }
            self.t.print_nl_s(b"");
        }
        f(self);
        self.t.print_ln();
        self.t.selector = old;
        self.drain();
    }

    /// §1457, §1458 with oracle-compat.ch.
    fn message(&mut self, b: &[u8]) {
        let old = self.t.selector;
        self.t.selector = NEW_STRING;
        self.t.message_printing = true;
        self.show(b, false);
        self.t.message_printing = false;
        self.t.selector = old;
        let t = &mut *self.t;
        t.str_room(1);
        let s = t.make_string();
        if t.term_offset + t.length(s) > t.max_print_line - 2 {
            t.print_ln();
        } else if t.term_offset > 0 || t.file_offset > 0 {
            t.print_char(b' ' as i32);
        }
        t.print(s);
        t.flush_string();
        self.drain();
    }

    /// A string holding this name: what `text(p)` would be.
    fn name_string(&mut self, b: &[u8]) -> i32 {
        let t = &mut *self.t;
        for &c in b {
            t.append_char(c as i32);
        }
        t.make_string()
    }

    /// `print_cs`, §284, as hash.rs has it.
    fn print_cs(&mut self, b: &[u8]) {
        match b.len() {
            0 => {
                self.t.print_esc_s(b"csname");
                self.t.print_esc_s(b"endcsname");
                self.t.print_char(b' ' as i32);
            }
            1 => {
                self.t.print_esc(b[0] as i32);
                if self.t.cat_code(b[0] as i32) == LETTER {
                    self.t.print_char(b' ' as i32);
                }
            }
            _ => {
                let s = self.name_string(b);
                self.t.print_esc(s);
                self.t.print_char(b' ' as i32);
                self.t.flush_string();
            }
        }
    }

    /// `sprint_cs`, §285, as hash.rs has it, for `\string`.
    fn string_of(&mut self, b: &[u8]) -> Vec<u8> {
        match b.len() {
            0 => self.conv(|t| {
                t.print_esc_s(b"csname");
                t.print_esc_s(b"endcsname");
            }),
            1 => {
                let c = b[0] as i32;
                self.conv(|t| t.print_esc(c))
            }
            _ => {
                let s = self.name_string(b);
                let r = self.conv(|t| t.print_esc(s));
                self.t.flush_string();
                r
            }
        }
    }

    fn write_conv(&mut self, stream: i32, f: impl FnOnce(&mut Tex)) {
        let s = self.conv(f);
        self.write_out(stream, |r| r.show(&s, true));
    }

    fn op(&mut self, op: &str) {
        let f: Vec<&str> = op.split(':').collect();
        let int = |k: usize| f[k].parse::<i32>().unwrap();
        match f[0] {
            "nl" => self.t.set_new_line_char(int(1)),
            "esc" => self.t.set_escape_char(int(1)),
            "w" => {
                let b = unhex(f[2]);
                self.write_out(int(1), |r| r.show(&b, false));
            }
            "msg" => self.message(&unhex(f[1])),
            "cs" => {
                let b = unhex(f[2]);
                self.write_out(int(1), |r| r.print_cs(&b));
            }
            "str" => {
                let s = self.string_of(&unhex(f[2]));
                self.write_out(int(1), |r| r.show(&s, true));
            }
            "num" => {
                let n = int(2);
                self.write_conv(int(1), |t| t.print_int(n));
            }
            "roman" => {
                let n = int(2);
                self.write_conv(int(1), |t| t.print_roman_int(n));
            }
            "scaled" => {
                let n = int(2);
                self.write_conv(int(1), |t| {
                    t.print_scaled(n);
                    t.print_s(b"pt");
                });
            }
            "char" | "mchar" => {
                let n = int(2);
                let name: &[u8] = if f[0] == "char" { b"char" } else { b"mathchar" };
                self.write_conv(int(1), |t| {
                    t.print_esc_s(name);
                    t.print_hex(n);
                });
            }
            "mode" => {
                // §1443, §75
                let t = &mut *self.t;
                t.print_ln();
                t.interaction = if f[1] == "batch" { BATCH_MODE } else { SCROLL_MODE };
                t.selector = if t.interaction == BATCH_MODE { NO_PRINT } else { TERM_ONLY };
                if t.log_opened {
                    t.selector += 2;
                }
                self.drain();
            }
            "show" => {
                // §1475, §1932 and common_ending of §1471
                let n = int(2);
                let s = self.conv(|t| t.print_int(n));
                let t = &mut *self.t;
                t.selector = int(1);
                t.print_nl_s(b"> ");
                self.show(&s, false);
                let t = &mut *self.t;
                t.print_ln();
                t.selector = if t.interaction == BATCH_MODE { NO_PRINT } else { TERM_ONLY };
                if t.log_opened {
                    t.selector += 2;
                }
                self.drain();
            }
            _ => panic!("op {op}"),
        }
    }
}

fn check_config(config: &str) {
    let dir = scratch(config);
    let t = initex();
    t.host.journal_outputs = true;
    t.max_print_line = match config {
        "mpl60" => 60,
        "mpl200" => 200,
        _ => 79,
    };
    match config {
        "eight" => {
            // -8bit, §24
            for k in 0..256 {
                t.xprn[k] = 1;
            }
        }
        "cp227" => {
            // What read_tcx_file installs.
            for k in 0..256 {
                t.xprn[k] = 0;
            }
            let row = TABLE.lines().find(|l| l.starts_with("printable\tcp227\t")).unwrap();
            for c in row.split('\t').nth(2).unwrap().split(',') {
                t.xprn[c.parse::<i32>().unwrap()] = 1;
            }
        }
        _ => {}
    }
    let first = config == "first";
    if !first {
        t.set_name_of_file(&path_to_bytes(&dir.join("t.log")));
        assert!(t.a_open_out(LOG_FILE));
        t.log_opened = true;
        t.set_name_of_file(&path_to_bytes(&dir.join("o3.out")));
        assert!(t.a_open_out(write_file(3)));
        t.write_open[3] = true;
    }
    t.interaction = if config == "batch" { BATCH_MODE } else { SCROLL_MODE };
    t.selector = if first {
        TERM_ONLY
    } else if config == "batch" {
        LOG_ONLY
    } else {
        TERM_AND_LOG
    };
    let mut r = Run { t, term: Vec::new() };
    let mut seen = 0;
    for row in TABLE.lines() {
        let f: Vec<&str> = row.split('\t').collect();
        if f[0] != "case" || f[1] != config {
            continue;
        }
        let name = f[2];
        r.t.term_offset = 0;
        r.t.file_offset = 0;
        r.term.clear();
        let (lg0, fl0) = (journal(r.t, LOG_FILE).len(), journal(r.t, write_file(3)).len());
        if !f[3].is_empty() {
            for op in f[3].split(' ') {
                r.op(op);
            }
        }
        // \endgroup, and the print_nl("") of the \write after the case.
        r.t.set_new_line_char(-1);
        r.t.set_escape_char(b'\\' as i32);
        r.t.print_nl_s(b"");
        r.drain();
        let got_log = journal(r.t, LOG_FILE)[lg0..].to_vec();
        let got_file = journal(r.t, write_file(3))[fl0..].to_vec();
        let show = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
        assert_eq!(show(&got_log), show(&unesc(f[4])), "{config} {name}: the log");
        assert_eq!(got_log, unesc(f[4]), "{config} {name}: the log's bytes");
        assert_eq!(show(&r.term), show(&unesc(f[5])), "{config} {name}: the terminal");
        assert_eq!(r.term, unesc(f[5]), "{config} {name}: the terminal's bytes");
        assert_eq!(show(&got_file), show(&unesc(f[6])), "{config} {name}: the file");
        assert_eq!(got_file, unesc(f[6]), "{config} {name}: the file's bytes");
        seen += 1;
    }
    assert!(seen > 0, "no cases for {config}");
    let t = r.t;
    Tex::dealloc(t);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn oracle_default() {
    check_config("default");
}
#[test]
fn oracle_max_print_line_60() {
    check_config("mpl60");
}
#[test]
fn oracle_max_print_line_200() {
    check_config("mpl200");
}
#[test]
fn oracle_8bit() {
    check_config("eight");
}
#[test]
fn oracle_cp227() {
    check_config("cp227");
}
#[test]
fn oracle_batchmode() {
    check_config("batch");
}
/// `selector=term_only`: on the first line, before the log opens.
#[test]
fn oracle_first_line() {
    check_config("first");
}

/// `selector` is `no_print` on the first line in batch mode, where the oracle shows nothing to compare with.
#[test]
fn no_print_prints_nothing() {
    let t = initex();
    t.selector = NO_PRINT;
    t.tally = 0;
    t.print_s(b"abc");
    t.print(1);
    t.print_ln();
    t.print_nl_s(b"x");
    assert!(t.host.term_buf.is_empty());
    assert_eq!((t.term_offset, t.file_offset), (0, 0));
    // §58: tally counts what would have been printed, ^^A being three.
    assert_eq!(t.tally, 7);
    Tex::dealloc(t);
}

/// `selector=pseudo`, §58 with §342: the characters go round `trick_buf` while `tally<trick_count`.
#[test]
fn pseudo_fills_trick_buf_modulo_error_line() {
    let t = initex();
    t.error_line = 5;
    t.trick_count = 7;
    t.tally = 0;
    t.selector = PSEUDO;
    t.print_s(b"abcdefghij");
    assert_eq!(t.tally, 10);
    let got: Vec<u8> = (0..5).map(|k| t.trick_buf[k] as u8).collect();
    assert_eq!(&got, b"fgcde");
    // new_line_char does not break a line here, and a non-printing character is written out.
    t.set_new_line_char(b'a' as i32);
    t.tally = 0;
    t.trick_count = 100;
    t.print(b'a' as i32);
    t.print(1);
    assert_eq!(t.tally, 4);
    Tex::dealloc(t);
}

/// `new_string`: "we drop characters if the string space is full", and tally goes on.
#[test]
fn new_string_drops_characters_when_the_pool_is_full() {
    let t = initex();
    t.selector = NEW_STRING;
    t.pool_size = t.pool_ptr + 2;
    t.tally = 0;
    let b = t.pool_ptr;
    t.print_s(b"abcd");
    assert_eq!(t.str_pool.range(b, t.pool_ptr), b"ab");
    assert_eq!(t.tally, 4);
    Tex::dealloc(t);
}

/// §59: a string number that cannot be, "this can't happen", prints `???`.
#[test]
fn print_of_a_wild_string_number() {
    let t = initex();
    t.selector = TERM_ONLY;
    for s in [-1, -1000, t.str_ptr, t.str_ptr + 5000] {
        t.print(s);
        t.print_ln();
        t.slow_print(s);
        t.print_ln();
    }
    let got = String::from_utf8(core::mem::take(&mut t.host.term_buf)).unwrap();
    assert_eq!(got, "???\n".repeat(8));
    Tex::dealloc(t);
}

#[test]
fn print_nl_and_print_esc_with_string_numbers() {
    let t = initex();
    t.selector = TERM_ONLY;
    t.print(b'x' as i32);
    t.print_nl(S_TEXPUT);
    t.print_nl(b'y' as i32);
    t.print_nl(S_EMPTY);
    t.print_esc(S_NULLFONT);
    t.print_ln();
    t.print_esc(b'.' as i32);
    t.print_ln();
    t.print_esc(1);
    let got = String::from_utf8(core::mem::take(&mut t.host.term_buf)).unwrap();
    assert_eq!(got, "x\ntexput\ny\n\\nullfont\n\\.\n\\^^A");
    Tex::dealloc(t);
}

#[test]
fn print_current_string_leaves_the_string() {
    let t = initex();
    t.selector = NEW_STRING;
    t.print_s(b"abc");
    let p = t.pool_ptr;
    t.selector = TERM_ONLY;
    t.print_current_string();
    assert_eq!(t.pool_ptr, p);
    assert_eq!(&t.host.term_buf[..], b"abc");
    Tex::dealloc(t);
}

/// print_int on what the oracle's 64-bit `integer` can hold: only here does `impl Into<i64>` count.
#[test]
fn print_int_past_32_bits() {
    let t = initex();
    t.selector = NEW_STRING;
    let mut seen = 0;
    for row in TABLE.lines().filter(|l| l.starts_with("wide\t")) {
        let text = row.split('\t').nth(1).unwrap();
        let n: i64 = text.parse().unwrap();
        assert!(n.abs() > i32::MAX as i64);
        let b = t.pool_ptr;
        t.print_int(n);
        assert_eq!(t.str_pool.range(b, t.pool_ptr), text.as_bytes());
        t.pool_ptr = b;
        seen += 1;
    }
    assert_eq!(seen, 4);
    for (n, want) in [
        (i64::MAX, "9223372036854775807"),
        (i64::MIN, "-9223372036854775808"),
        (-100000000, "-100000000"),
        (-99999999, "-99999999"),
    ] {
        let b = t.pool_ptr;
        t.print_int(n);
        assert_eq!(t.str_pool.range(b, t.pool_ptr), want.as_bytes());
        t.pool_ptr = b;
    }
    Tex::dealloc(t);
}

/// §66, on the time at the head of the oracle's log, and on what no primitive reaches.
#[test]
fn print_two_is_modulo_100() {
    let t = initex();
    t.selector = NEW_STRING;
    let mut seen = 0;
    for row in TABLE.lines().filter(|l| l.starts_with("two\t")) {
        let f: Vec<&str> = row.split('\t').collect();
        let b = t.pool_ptr;
        t.print_two(f[1].parse().unwrap());
        t.print_char(b':' as i32);
        t.print_two(f[2].parse().unwrap());
        assert_eq!(t.str_pool.range(b, t.pool_ptr), f[3].as_bytes());
        t.pool_ptr = b;
        seen += 1;
    }
    assert_eq!(seen, 6);
    for (n, want) in [(0, "00"), (5, "05"), (59, "59"), (99, "99"), (100, "00"), (123, "23"), (-5, "05"), (-123, "23"), (2147483647, "47")] {
        let b = t.pool_ptr;
        t.print_two(n);
        assert_eq!(t.str_pool.range(b, t.pool_ptr), want.as_bytes());
        t.pool_ptr = b;
    }
    Tex::dealloc(t);
}

/// §67 and §64 past what `\meaning` shows.
#[test]
fn print_hex_of_large_values() {
    let t = initex();
    t.selector = NEW_STRING;
    for (n, want) in [(0, "\"0"), (10, "\"A"), (0x7ABCDEF, "\"7ABCDEF"), (2147483647, "\"7FFFFFFF")] {
        let b = t.pool_ptr;
        t.print_hex(n);
        assert_eq!(t.str_pool.range(b, t.pool_ptr), want.as_bytes());
        t.pool_ptr = b;
    }
    Tex::dealloc(t);
}

/// §37 when the command line holds something: no prompt, and `loc` past the blanks.
#[test]
fn init_terminal_takes_the_command_line() {
    let t = initex();
    t.buf_size = 1000;
    t.first = 1;
    t.host.command_line = b"  \\input x  ".to_vec();
    assert!(t.init_terminal());
    assert_eq!(t.loc(), 3);
    assert_eq!(t.buffer.at(t.loc()), b'\\' as i32);
    // The trailing spaces went in t_open_in.
    assert_eq!(t.last, 1 + 10);
    assert!(t.host.term_buf.is_empty());
    Tex::dealloc(t);
}

// ---- §4 ----

fn check_initial_values(t: &Tex) {
    let mut pars = 0;
    for row in INIT.lines() {
        let f: Vec<&str> = row.split('\t').collect();
        match f[0] {
            "table" => {
                for (k, v) in f[2].split(',').enumerate() {
                    let (k, v) = (k as i32, v.parse::<i32>().unwrap());
                    let got = match f[1] {
                        "catcode" => t.cat_code(k),
                        "lccode" => t.lc_code(k),
                        "uccode" => t.uc_code(k),
                        "sfcode" => t.sf_code(k),
                        "mathcode" => t.math_code(k),
                        "delcode" => t.del_code(k),
                        x => panic!("table {x}"),
                    };
                    // The oracle's source had to make braces of these two first.
                    if f[1] == "catcode" && (k == b'{' as i32 || k == b'}' as i32) {
                        assert_eq!(got, OTHER_CHAR);
                        continue;
                    }
                    assert_eq!(got, v, "{} {k}", f[1]);
                }
            }
            "par" => {
                let loc = f[3].parse::<i32>().unwrap();
                pars += 1;
                match f[2] {
                    "int" => assert_eq!(t.eqtb[loc].int().to_string(), f[4], "\\{}", f[1]),
                    "dimen" => assert_eq!(t.eqtb[loc].sc().to_string(), f[4], "\\{}", f[1]),
                    "glue" | "muglue" => {
                        assert_eq!(t.equiv(loc), ZERO_GLUE, "\\{}", f[1]);
                        assert_eq!(f[4], if f[2] == "glue" { "0.0pt" } else { "0.0mu" }, "\\{}", f[1]);
                    }
                    "toks" => {
                        assert_eq!(t.equiv(loc), NULL, "\\{}", f[1]);
                        assert_eq!(f[4], "[]", "\\{}", f[1]);
                    }
                    x => panic!("kind {x}"),
                }
            }
            x => panic!("row {x}"),
        }
    }
    assert!(pars > 150, "only {pars} parameters");
}

fn check_constant_structures(t: &Tex) {
    // §182: the glue specifications and the first free node.
    assert_eq!(t.glue_ref_count(ZERO_GLUE), NULL + 1 + LOCAL_BASE - GLUE_BASE);
    assert_eq!((t.width(ZERO_GLUE), t.stretch(ZERO_GLUE), t.shrink(ZERO_GLUE)), (0, 0, 0));
    assert_eq!((t.stretch(FIL_GLUE), t.stretch_order(FIL_GLUE)), (UNITY, FIL));
    assert_eq!((t.stretch(FILL_GLUE), t.stretch_order(FILL_GLUE)), (UNITY, FILL));
    assert_eq!((t.stretch(SS_GLUE), t.stretch_order(SS_GLUE), t.shrink(SS_GLUE), t.shrink_order(SS_GLUE)), (UNITY, FIL, UNITY, FIL));
    assert_eq!((t.stretch(FIL_NEG_GLUE), t.stretch_order(FIL_NEG_GLUE)), (-UNITY, FIL));
    assert_eq!(t.rover, LO_MEM_STAT_MAX + 1);
    assert_eq!(t.link(t.rover), EMPTY_FLAG);
    assert_eq!(t.node_size(t.rover), 1000);
    assert_eq!((t.llink(t.rover), t.rlink(t.rover)), (t.rover, t.rover));
    assert_eq!(t.lo_mem_max, t.rover + 1000);
    assert_eq!((t.link(t.lo_mem_max), t.info(t.lo_mem_max)), (NULL, NULL));
    // §973, §996, §1158, §1165: the special list heads.
    assert_eq!((t.link(END_SPAN), t.info(END_SPAN)), (WEB_MAX_QUARTERWORD + 1, NULL));
    assert_eq!((t.ty(LAST_ACTIVE), t.line_number(LAST_ACTIVE), t.subtype(LAST_ACTIVE)), (HYPHENATED, MAX_HALFWORD, 0));
    assert_eq!((t.subtype(PAGE_INS_HEAD), t.ty(PAGE_INS_HEAD), t.link(PAGE_INS_HEAD)), (255, SPLIT_UP, PAGE_INS_HEAD));
    assert_eq!((t.ty(PAGE_HEAD), t.subtype(PAGE_HEAD)), (GLUE_NODE, NORMAL));
    assert_eq!((t.avail, t.mem_end, t.hi_mem_min), (NULL, MEM_TOP, HI_MEM_STAT_MIN));
    assert_eq!((t.var_used, t.dyn_used), (LO_MEM_STAT_MAX + 1 - MEM_BOT, HI_MEM_STAT_USAGE));
    // §966 through toks_init: the omit template is `\endtemplate` alone.
    assert_eq!(t.toks[OMIT_TEMPLATE + 1], END_TEMPLATE_TOKEN);
    assert_eq!(t.toks[OMIT_TEMPLATE + 2], 0);
    // §240-§250: where each region's entries start.
    for p in [ACTIVE_BASE, SINGLE_BASE, NULL_CS, HASH_BASE, UNDEFINED_CONTROL_SEQUENCE] {
        assert_eq!((t.eq_type(p), t.equiv(p), t.eq_level(p)), (UNDEFINED_CS, NULL, LEVEL_ZERO), "eqtb[{p}]");
    }
    assert_eq!((t.eq_type(GLUE_BASE), t.equiv(GLUE_BASE), t.eq_level(GLUE_BASE)), (GLUE_REF, ZERO_GLUE, LEVEL_ONE));
    assert_eq!((t.eq_type(LOCAL_BASE - 1), t.equiv(LOCAL_BASE - 1)), (GLUE_REF, ZERO_GLUE));
    assert_eq!((t.eq_type(PAR_SHAPE_LOC), t.equiv(PAR_SHAPE_LOC), t.eq_level(PAR_SHAPE_LOC)), (SHAPE_REF, NULL, LEVEL_ONE));
    assert_eq!((t.eq_type(ETEX_PENS - 1), t.equiv(ETEX_PENS - 1)), (SHAPE_REF, NULL));
    assert_eq!((t.eq_type(OUTPUT_ROUTINE_LOC), t.eq_level(OUTPUT_ROUTINE_LOC)), (UNDEFINED_CS, LEVEL_ZERO));
    assert_eq!((t.eq_type(TOKS_BASE + 255), t.eq_level(TOKS_BASE + 255)), (UNDEFINED_CS, LEVEL_ZERO));
    assert_eq!((t.eq_type(BOX_BASE + 255), t.equiv(BOX_BASE + 255), t.eq_level(BOX_BASE + 255)), (BOX_REF, NULL, LEVEL_ONE));
    assert_eq!(t.cur_font(), NULL_FONT);
    assert_eq!((t.eq_type(MATH_FONT_BASE + 47), t.equiv(MATH_FONT_BASE + 47)), (DATA, NULL_FONT));
    assert_eq!((t.eq_type(INT_BASE - 1), t.eq_level(INT_BASE - 1)), (DATA, LEVEL_ONE));
    // §277, §1394, §1616: the frozen names, which are the well-known strings.
    assert_eq!((t.eq_type(FROZEN_DONT_EXPAND), t.text(FROZEN_DONT_EXPAND)), (DONT_EXPAND, S_NOTEXPANDED));
    assert_eq!((t.eq_type(FROZEN_PRIMITIVE), t.equiv(FROZEN_PRIMITIVE), t.eq_level(FROZEN_PRIMITIVE)), (IGNORE_SPACES, 1, LEVEL_ONE));
    assert_eq!(t.text(FROZEN_PRIMITIVE), S_PDFPRIMITIVE);
    assert_eq!(t.text(FROZEN_PROTECTION), S_INACCESSIBLE);
    assert_eq!((t.text(END_WRITE), t.eq_type(END_WRITE), t.equiv(END_WRITE), t.eq_level(END_WRITE)), (S_ENDWRITE, OUTER_CALL, NULL, LEVEL_ONE));
    assert_eq!(t.format_ident, S_INITEX_IDENT);
    assert_eq!(t.pdf_space_font_name, S_PDFTEXSPACE);
    // §118, §688
    assert_eq!(t.two_to_the[30], 1 << 30);
    assert_eq!((t.spec_log[14], t.spec_log[27], t.spec_log[28]), (1 << 13, 1, 1));
    assert_eq!(t.ten_pow[9], 1000000000);
    // §1105, §1653, §1812
    assert_eq!(t.hyph_next, HYPH_PRIME + 1);
    assert_eq!((t.eTeX_mode, t.max_reg_num, t.max_reg_help_line), (0, 255, 0));
    // §233: the page builder's and the semantic nest's start.
    assert_eq!((t.mode(), t.head(), t.tail()), (VMODE, CONTRIB_HEAD, CONTRIB_HEAD));
    assert_eq!((t.prev_depth(), t.page_contents, t.page_tail), (IGNORE_DEPTH, EMPTY, PAGE_HEAD));
    assert_eq!((t.last_glue, t.last_node_type), (MAX_HALFWORD, -1));
    // §24
    for k in 0..256 {
        assert_eq!(t.xchr.at(k), k);
        assert_eq!(t.xord.at(k), k);
        assert_eq!(t.xprn.at(k) != 0, (32..=126).contains(&k));
    }
}

/// §4 without its §1584, which needs ext.rs's `init_start_time`.
#[test]
fn initial_tables_against_the_oracle() {
    let t = just_initialized();
    check_initial_values(t);
    check_constant_structures(t);
    Tex::dealloc(t);
}

#[test]
fn initialize_as_a_whole() {
    let t = Tex::alloc();
    t.ini_version = true;
    t.pool_size = 1000000;
    t.max_strings = 100000;
    t.hyph_size = 8191;
    t.dvi_buf_size = 16384;
    t.interaction_option = UNSPECIFIED_MODE;
    t.initialize();
    assert_eq!(t.interaction, ERROR_STOP_MODE);
    assert_eq!(t.synctexoffset, INT_BASE + SYNCTEX_CODE);
    assert!(t.host.world.log.iter().any(|o| matches!(o, Observation::Clock { .. })), "§1584 reads the clock");
    check_initial_values(t);
    check_constant_structures(t);
    Tex::dealloc(t);
}

/// Without INITEX only the key variables are set; the tables wait for the format: §8's `Init ... Tini`.
#[test]
fn initialize_without_initex_leaves_the_tables() {
    let t = Tex::alloc();
    t.ini_version = false;
    t.hyph_size = 8191;
    t.dvi_buf_size = 16384;
    t.interaction_option = BATCH_MODE;
    t.initialize();
    assert_eq!(t.interaction, BATCH_MODE);
    assert_eq!(t.tolerance(), 0);
    assert_eq!(t.toks_top, 0);
    assert_eq!(t.format_ident, 0);
    assert_eq!(t.mode(), VMODE);
    Tex::dealloc(t);
}
