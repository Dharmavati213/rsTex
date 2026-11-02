//! files.rs, the host's file operations and print_file_line against the oracle: crates/engine/testdata/files_oracle.tsv,
//! from tests/oracle/gen_files_tables.py.
//!
//! Each case runs in a child, this same test binary, in a directory of its own, with the case's standard input:
//! the terminal is read and the process can end.  The child does what main_body does before main_control, and then
//! calls get_x_token until it sees `\end`.  `expand` does `\input` and `\endinput` itself, and whatever else is in
//! these documents leaves no trace on the oracle, so the logs and terminals must be the same up to there.
//!
//! A case that dies goes on through jump_out into close_files_and_terminate, a stub in main.rs until that module is
//! merged.  Now that is a panic, which the child catches; later the process exits there and then.  Either way the
//! parent asks only that the output begin with what the oracle printed before that point.

use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::{Command, Stdio};

use rstex_engine::access::*;
use rstex_engine::consts::*;
use rstex_engine::host::*;
use rstex_engine::state::*;

const TABLE: &str = include_str!("../testdata/files_oracle.tsv");
const BEGIN: &[u8] = b"\x01BEGIN\n";
const END_OK: &[u8] = b"\x01END ok\n";
const END_PANIC: &[u8] = b"\x01END panic\n";

struct Row<'a> {
    name: &'a str,
    flags: Vec<&'a str>,
    mode: &'a str,
    first: &'a str,
    stdin: Vec<u8>,
    files: Vec<(&'a str, Vec<u8>)>,
    exact: bool,
    log: Vec<u8>,
    term: Vec<u8>,
    log_name: &'a str,
    fls: Vec<u8>,
}

fn unesc(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' {
            match b[i + 1] {
                b'n' => out.push(b'\n'),
                b't' => out.push(b'\t'),
                b'r' => out.push(b'\r'),
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

fn parse(line: &str) -> Row<'_> {
    let f: Vec<&str> = line.split('\t').collect();
    Row {
        name: f[0],
        flags: f[1].split(' ').filter(|x| !x.is_empty()).collect(),
        mode: f[2],
        first: f[3],
        stdin: unesc(f[4]),
        files: f[5].split('|').map(|e| {
            let (n, c) = e.split_once('=').unwrap();
            (n, unesc(c))
        }).collect(),
        exact: f[6] == "1",
        log: unesc(f[7]),
        term: unesc(f[8]),
        log_name: f[9],
        fls: unesc(f[10]),
    }
}

/// What main_body has done by §1517, with the sizes of texmf.cnf, and INITEX's primitives as far as these documents go.
fn machine(row: &Row) -> &'static mut Tex {
    let t = Tex::alloc();
    t.ini_version = true;
    t.pool_size = 6250000;
    t.max_strings = 500000;
    t.string_vacancies = 90000;
    t.hyph_size = 8191;
    t.dvi_buf_size = 16384;
    t.buf_size = 200000;
    t.stack_size = 10000;
    t.param_size = 20000;
    t.max_in_open = 15;
    t.expand_depth = 10000;
    t.save_size = 200000;
    t.nest_size = 1000;
    t.eqtb_top = EQTB_CAP as i32 - 1;
    t.hash_extra = 600000;
    t.prim_used = PRIM_SIZE;
    t.format_ident = 0;
    t.interaction_option = match row.mode {
        "batchmode" => BATCH_MODE,
        "nonstopmode" => NONSTOP_MODE,
        "scrollmode" => SCROLL_MODE,
        _ => ERROR_STOP_MODE,
    };
    t.print_init_key_variables_to_1571();
    t.print_init_key_variables_from_1629();
    t.print_init_table_entries();
    assert!(t.get_strings_started());
    t.init_str_ptr = t.str_ptr;
    t.init_pool_ptr = t.pool_ptr;
    t.format_ident = S_INITEX_IDENT;
    t.max_print_line = 79;
    t.error_line = 79;
    t.half_error_line = 50;
    t.eTeX_mode = 1;
    t.interaction = t.interaction_option;
    t.stop_at_space = true;
    t.set_end_line_char(13);

    t.host.world.set_program_name(b"pdftex");
    let mut output_directory = None;
    for f in &row.flags {
        match f.split_once('=').unwrap_or((f, "")) {
            ("-file-line-error", _) => t.file_line_error_style_p = 1,
            ("-jobname", v) => t.host.c_job_name = Some(v.as_bytes().to_vec()),
            ("-output-directory", v) => output_directory = Some(v.as_bytes()),
            ("-no-shell-escape", _) => t.shellenabledp = -1,
            ("-shell-escape", _) => t.shellenabledp = 1,
            ("-shell-restricted", _) => {
                t.shellenabledp = 1;
                t.restrictedshell = 1;
            }
            ("-recorder", _) => t.host.world.recorder_enabled = true,
            (other, _) => panic!("flag {other}"),
        }
    }
    t.host.world.init_output_directory(output_directory);
    if t.file_line_error_style_p == 0 {
        t.file_line_error_style_p = t.texmf_yesno(b"file_line_error_style") as i32;
    }
    t.parse_first_line_p = t.texmf_yesno(b"parse_first_line") as i32;
    // What init_shell_escape makes of the options and of texmf.cnf.
    if t.shellenabledp < 0 {
        t.shellenabledp = 0;
    } else if t.shellenabledp == 0 {
        match t.host.world.var_value(b"shell_escape").as_deref().and_then(|v| v.first()) {
            Some(b't' | b'y' | b'1') => t.shellenabledp = 1,
            Some(b'p') => {
                t.shellenabledp = 1;
                t.restrictedshell = 1;
            }
            _ => {}
        }
    }
    t.host.command_line = row.first.as_bytes().to_vec();

    // §1517 Initialize the input routines
    t.input_ptr = 0;
    t.max_in_stack = 0;
    t.in_open = 0;
    t.open_parens = 0;
    t.max_buf_stack = 0;
    t.param_ptr = 0;
    t.max_param_stack = 0;
    t.first = 1;
    t.set_state(NEW_LINE);
    t.set_start(1);
    t.set_index(0);
    t.line = 0;
    t.set_name(0);
    t.force_eof = false;
    t.align_state = 1000000;
    t.no_new_control_sequence = false;
    // These are the only ones needed: the rest of each document is commands that the loop below passes over.
    t.primitive(b"input", INPUT, 0);
    t.primitive(b"endinput", INPUT, 1);
    t.primitive(b"end", STOP, 0);
    t.primitive(b"relax", RELAX, 256);
    let s = t.text(t.cur_val);
    t.set_text(FROZEN_RELAX, s);
    t.eqtb[FROZEN_RELAX] = t.eqtb[t.cur_val];
    t.primitive(b"def", RELAX, 256);
    t.primitive(b"catcode", RELAX, CATCODE);
    t.primitive(b"csname", CS_NAME, 0);
    t.primitive(b"endcsname", END_CS_NAME, 0);
    t.no_new_control_sequence = true;
    define(t, b'x', b"a");
    define(t, b'y', b".tex");
    assert!(t.init_terminal());
    t.set_limit(t.last);
    t.first = t.last + 1;
    if t.end_line_char_inactive() {
        t.set_limit(t.limit() - 1);
    } else {
        t.buffer.put(t.limit(), t.end_line_char());
    }
    t.fix_date_and_time();
    t.selector = if t.interaction == BATCH_MODE { NO_PRINT } else { TERM_ONLY };
    t
}

/// A chr for `\catcode`, on relax, so that the loop knows it.
const CATCODE: i32 = 257;

/// `\def\c{body}`: a call with no parameters.
fn define(t: &mut Tex, c: u8, body: &[u8]) {
    let m = t.tb_mark();
    t.tb_push(END_MATCH_TOKEN);
    for &k in body {
        let cmd = if k.is_ascii_alphabetic() { LETTER } else { OTHER_CHAR };
        t.tb_push(cmd * 256 + k as i32);
    }
    let p = t.tb_commit_ref(m);
    let cs = SINGLE_BASE + c as i32;
    t.set_eq_type(cs, CALL);
    t.set_equiv(cs, p);
    t.set_eq_level(cs, LEVEL_ONE);
}

/// What main_control would do with these documents: `\catcode`, and nothing with anything else, until `\end`.
fn main_loop(t: &mut Tex) {
    if t.loc() < t.limit() && t.cat_code(t.buffer.at(t.loc())) != ESCAPE {
        t.start_input();
    }
    loop {
        t.get_x_token();
        if t.cur_cmd == STOP {
            return;
        }
        if t.cur_cmd == RELAX && t.cur_chr == CATCODE {
            let mut n = 0;
            loop {
                t.get_x_token();
                if t.cur_chr == b'=' as i32 {
                    break;
                }
                n = n * 10 + t.cur_chr - b'0' as i32;
            }
            let mut v = 0;
            loop {
                t.get_x_token();
                if !(b'0' as i32..=b'9' as i32).contains(&t.cur_chr) || t.cur_cmd != OTHER_CHAR {
                    break;
                }
                v = v * 10 + t.cur_chr - b'0' as i32;
            }
            t.set_cat_code(n, v);
        }
    }
}

/// In the child.
fn child(row: &Row) -> ! {
    // Raw, since the harness captures eprintln!.
    std::panic::set_hook(Box::new(|i| {
        let _ = writeln!(std::io::stderr(), "child panicked: {i}");
    }));
    let t = machine(row);
    std::io::stdout().write_all(BEGIN).unwrap();
    let r = catch_unwind(AssertUnwindSafe(|| main_loop(t)));
    t.host.term_flush();
    t.host.files[LOG_FILE as usize] = Slot::Closed;
    let end = if r.is_ok() { END_OK } else { END_PANIC };
    let mut o = std::io::stdout();
    o.write_all(end).unwrap();
    o.flush().unwrap();
    std::process::exit(0)
}

/// The child's entry: nothing unless it was started to run a case.
#[test]
fn child_entry() {
    if std::env::var("RSTEX_FILES_EDIT").is_ok() {
        let row = parse(TABLE.lines().next().unwrap());
        let t = machine(&row);
        let start = t.pool_ptr;
        for &c in b"ed.tex" {
            t.append_char(c as i32);
        }
        t.call_edit(start, 6, 3);
    }
    if let Ok(name) = std::env::var("RSTEX_FILES_CASE") {
        let line = TABLE.lines().find(|l| l.split('\t').next() == Some(&name)).unwrap();
        child(&parse(line));
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

struct Out {
    term: Vec<u8>,
    log: Vec<u8>,
    fls: Vec<u8>,
    /// None if the process ended by itself, through uexit.
    end: Option<&'static [u8]>,
    stderr: String,
}

/// The lines of a `.fls` file that do not depend on the installation, as the generator keeps them.
fn recorded(fls: &[u8]) -> Vec<u8> {
    let lines: Vec<&[u8]> = fls
        .split(|&c| c == b'\n')
        .filter(|l| !l.is_empty() && !l.starts_with(b"PWD ") && !l.ends_with(b"texmf.cnf") && !l.ends_with(b".fmt") && !l.ends_with(b".map"))
        .collect();
    lines.join(&b'\n')
}

fn run_child(row: &Row) -> Out {
    let mut dir = std::env::temp_dir();
    dir.push(format!("rstex-files-test-{}-{}", std::process::id(), row.name));
    std::fs::create_dir_all(&dir).unwrap();
    for (path, content) in &row.files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }
    let mut c = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_entry", "--test-threads=1"])
        .current_dir(&dir)
        .env("RSTEX_FILES_CASE", row.name)
        .env("SOURCE_DATE_EPOCH", "1700000000")
        .env("FORCE_SOURCE_DATE", "1")
        .env_remove("TEXMF_OUTPUT_DIRECTORY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(&row.stdin).unwrap();
    let o = c.wait_with_output().unwrap();
    let out = &o.stdout;
    let stderr = String::from_utf8_lossy(&o.stderr).into_owned();
    let a = find(out, BEGIN).unwrap_or_else(|| panic!("{}: the child did not start: {stderr}", row.name)) + BEGIN.len();
    let rest = &out[a..];
    let (term, end) = if let Some(k) = find(rest, END_OK) {
        (&rest[..k], Some(END_OK))
    } else if let Some(k) = find(rest, END_PANIC) {
        (&rest[..k], Some(END_PANIC))
    } else {
        (rest, None)
    };
    let log = if row.log_name.is_empty() { Vec::new() } else { std::fs::read(dir.join(row.log_name)).unwrap_or_default() };
    let fls = if row.log_name.is_empty() {
        Vec::new()
    } else {
        std::fs::read(dir.join(row.log_name.replace(".log", ".fls"))).map(|f| recorded(&f)).unwrap_or_default()
    };
    std::fs::remove_dir_all(&dir).unwrap();
    Out { term: term.to_vec(), log, fls, end, stderr }
}

fn trim_end(b: &[u8]) -> &[u8] {
    let n = b.iter().rposition(|c| !c.is_ascii_whitespace()).map_or(0, |k| k + 1);
    &b[..n]
}

#[test]
fn against_the_oracle() {
    if rstex_engine::platform::DOSISH {
        return; // the table is from a Unix oracle: its names, and its pipes for sh
    }
    let mut seen = 0;
    let mut failed = Vec::new();
    for line in TABLE.lines() {
        let row = parse(line);
        let out = run_child(&row);
        seen += 1;
        let (term, log) = (String::from_utf8_lossy(&out.term), String::from_utf8_lossy(&out.log));
        let (want_term, want_log) = (String::from_utf8_lossy(&row.term), String::from_utf8_lossy(&row.log));
        let ok = if row.exact {
            out.end == Some(END_OK) && trim_end(&out.term) == trim_end(&row.term) && trim_end(&out.log) == trim_end(&row.log)
        } else {
            // It went on to jump_out.  While that is a stub it must be the stub that stopped it.
            let stub = out.end != Some(END_PANIC) || out.stderr.contains("close_files_and_terminate");
            out.end != Some(END_OK)
                && stub
                && trim_end(&out.term).starts_with(trim_end(&row.term))
                && trim_end(&out.log).starts_with(trim_end(&row.log))
        };
        if !ok || out.fls != row.fls {
            failed.push(format!(
                "==== {}: end {:?}\n{}\n---- oracle log\n{}\n---- rust log\n{}\n---- oracle terminal\n{}\n---- rust terminal\n{}\n---- oracle fls\n{}\n---- rust fls\n{}\n",
                row.name,
                out.end.map(|e| String::from_utf8_lossy(e)),
                out.stderr,
                want_log,
                log,
                want_term,
                term,
                String::from_utf8_lossy(&row.fls),
                String::from_utf8_lossy(&out.fls),
            ));
        }
    }
    assert!(seen > 70, "{seen} rows");
    assert!(failed.is_empty(), "{} of {} differ:\n{}", failed.len(), seen, failed.join("\n"));
}

/// What the oracle does with these values of TEXEDIT, as read from runs of pdftex with `e` at the error prompt.
#[test]
fn call_edit_expands_the_editor_command() {
    if rstex_engine::platform::DOSISH {
        return; // these commands are for sh
    }
    let run = |editor: &str| {
        let mut dir = std::env::temp_dir();
        dir.push(format!("rstex-files-edit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let o = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "child_entry", "--test-threads=1"])
            .current_dir(&dir)
            .env("RSTEX_FILES_EDIT", "1")
            .env("TEXEDIT", editor)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let edited = std::fs::read(dir.join("edited")).unwrap_or_default();
        std::fs::remove_dir_all(&dir).unwrap();
        (o.status.code(), String::from_utf8_lossy(&edited).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned())
    };
    let (code, edited, _) = run("echo %s:%d %z %% > edited; true %");
    assert_eq!(code, Some(1), "it ends the run with 1, whatever the editor did");
    assert_eq!(edited, "ed.tex:3 %z %%\n");
    let (code, _, err) = run("false %s");
    assert_eq!(code, Some(1));
    assert!(err.contains("! Trouble executing `false ed.tex'."), "{err}");
    let (code, edited, err) = run("echo %s %s > edited");
    assert_eq!(code, Some(1));
    assert_eq!(edited, "");
    assert!(err.contains("call_edit: `%%s' appears twice in editor command."), "{err}");
    let (_, _, err) = run("echo %d %d > edited");
    assert!(err.contains("call_edit: `%%d' appears twice in editor command."), "{err}");
}
