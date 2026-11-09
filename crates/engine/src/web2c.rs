//! §1867-1933 Parts 54, 55: web2c, string recycling, MLTeX, SyncTeX, index.
//!
//! MLTeX is dropped: effective_char returns c; §1881-1894 need no marker anywhere.
//! DROPPED §1930: src-specials are dropped; get_nullstr stays, at §1931.
//!
//! SyncTeX (§1895-1929) is wired through the `synctex_*` externals, whose call sites stay in the modules
//! that make them.  What they call is `synctex.rs`.
#![forbid(unsafe_code)]
#![deny(unreachable_patterns, unused_variables)]
#![allow(non_snake_case, unused_mut, unused_imports, dead_code)]

use crate::access::*;
use crate::consts::*;
use crate::host::*;
use crate::platform;
use crate::raw::*;
use crate::state::*;

crate::pod_struct! {
    /// POD state private to this module.  Add fields here, not to state.rs.
    pub struct Vars {
        /// For `search_string`: open addressing by `hash_name` over the strings of the format, the newest of each text, 0 when empty.  Made by
        /// `str_index_make` when a format is stored.  All of those strings stay as they are in a run, below `init_str_ptr`.
        pub str_index: PodVec<i32>,
    }
}

/// Non-POD state private to this module, in `self.host`.  Rarely needed.
#[derive(Default)]
pub struct HVars {}

impl Tex {
    /// §1870 To standard error, for `-debug-format`.
    pub fn print_csnames(&mut self, hstart: i32, hfinish: i32) {
        let msg = format!("fmtdebug:csnames from {} to {}:\n", hstart, hfinish);
        self.host.world.stderr_write(msg.as_bytes());
        for h in hstart..=hfinish {
            if self.text(h) > 0 {
                // if have anything at this position
                let (a, b) = (self.str_start[self.text(h)], self.str_start[self.text(h) + 1]);
                let line = [self.str_pool.range(a, b), b"|\n"].concat();
                self.host.world.stderr_write(&line);
            }
        }
    }

    /// §1872 For file:line:error style messages: looks for a filename in `full_source_filename_stack`,
    /// and if there is none falls back on the other style.
    pub fn print_file_line(&mut self) {
        let mut level = self.in_open;
        while level > 0 && self.full_source_filename_stack[level] == 0 {
            level -= 1;
        }
        if level == 0 {
            self.print_nl_s(b"! ");
        } else {
            self.print_nl_s(b"");
            self.print(self.full_source_filename_stack[level]);
            self.print(b':' as i32);
            if level == self.in_open {
                self.print_int(self.line);
            } else {
                self.print_int(self.line_stack[level + 1]);
            }
            self.print_s(b": ");
        }
    }

    /// §1873
    pub fn scan_four_bit_int_or_18(&mut self) {
        self.scan_int();
        if self.cur_val < 0 || (self.cur_val > 15 && self.cur_val != 18) {
            self.print_err_s(b"Bad number");
            self.help(&[
                b"Since I expected to read a number between 0 and 15,",
                b"I changed this one to zero.",
            ]);
            self.int_error(self.cur_val);
            self.cur_val = 0;
        }
    }

    /// §1876 Either 0 or the number of a string equal to `search`, the newest of those below it.  All of a format's strings, some 30000 with LaTeX for
    /// each file name a document opens, are looked for in `str_index`, and those made since in an index in Host.
    pub fn search_string(&mut self, search: i32) -> i32 {
        let len = self.length(search);
        if len == 0 {
            // trivial case
            return S_EMPTY;
        }
        // The first 256 strings depend on implementation.  With a format, those it holds are in `str_index`, so nothing is built for them, and they all come after what this run has made.
        let indexed = self.v_web2c.str_index.cap > 0 && search >= self.init_str_ptr;
        let lowest = if indexed { self.init_str_ptr } else { 256 };
        // Newest first, as WEB's `s:=search-1` down, but only among those that may be equal.
        let mut s = self.strings_index_first(lowest, search);
        while s != 0 {
            if s < search && self.length(s) == len && self.str_eq_str(s, search) {
                return s;
            }
            s = self.strings_index_next(s);
        }
        if indexed {
            return self.str_index_find(search);
        }
        0
    }

    /// The string of `str_index` with the text of string `search`, or 0.
    fn str_index_find(&mut self, search: i32) -> i32 {
        let index = self.v_web2c.str_index;
        let mask = index.cap - 1;
        let text = self.str_pool.range(self.str_start[search], self.str_start[search + 1]);
        let mut i = crate::hash::hash_name(text) as i32 & mask;
        loop {
            let s = index.get(&mut self.arena, i);
            if s == 0 || self.str_pool.range(self.str_start[s], self.str_start[s + 1]) == text {
                return s;
            }
            i = (i + 1) & mask;
        }
    }

    /// Makes `str_index`, of the strings above 255: for a format about to be stored.
    pub fn str_index_make(&mut self) {
        let cap = (2 * (self.str_ptr - 255).max(8) as usize).next_power_of_two() as i32;
        let mut index = self.arena_vec::<i32>(cap);
        for _ in 0..cap {
            index.push_within_cap(&mut self.arena, 0);
        }
        self.v_web2c.str_index = index;
        for s in 256..self.str_ptr {
            let text = self.str_pool.range(self.str_start[s], self.str_start[s + 1]);
            let mut i = crate::hash::hash_name(text) as i32 & (cap - 1);
            loop {
                let e = index.get(&mut self.arena, i);
                if e == 0 || self.str_pool.range(self.str_start[e], self.str_start[e + 1]) == text {
                    break;
                }
                i = (i + 1) & (cap - 1);
            }
            // A newer string with the same text takes the place of an older one.
            index.set(&mut self.arena, i, s);
        }
    }

    /// §1877 A variant of `make_string`: a string equal to the one just built is returned instead, and
    /// `flush_string` cannot be applied to that.
    pub fn slow_make_string(&mut self) -> i32 {
        let t = self.make_string();
        let s = self.search_string(t);
        if s > 0 {
            self.flush_string();
            return s;
        }
        t
    }

    /// §1880
    pub fn scan_file_name_braced(&mut self) {
        let save_scanner_status = self.scanner_status; // scan_toks sets scanner_status to absorbing
        let save_def_ref = self.def_ref; // scan_toks uses def_ref to point to the token list just read
        let save_cur_cs = self.cur_cs; // we set cur_cs back a few tokens to use in runaway errors
        // Scanning a token list
        self.cur_cs = self.warning_index; // for possible runaway error
        let _ = self.scan_toks(false, true); // actually do the scanning
        // s := tokens_to_string(def_ref).  old_setting is the global here.
        self.old_setting = self.selector;
        self.selector = NEW_STRING;
        self.show_token_list(self.def_ref + 1, NULL, self.pool_size - self.pool_ptr);
        self.selector = self.old_setting;
        let s = self.make_string();
        // Restoring some variables
        self.delete_token_ref(self.def_ref); // remove the token list from memory
        self.def_ref = save_def_ref;
        self.cur_cs = save_cur_cs;
        self.scanner_status = save_scanner_status;
        // Passing the read string to the input machinery
        let save_stop_at_space = self.stop_at_space;
        self.stop_at_space = false; // to allow spaces in file names
        self.begin_name();
        for i in self.str_start[s]..self.str_start[s + 1] {
            let _ = self.more_name(self.str_pool.at(i)); // add each read character to the current file name
        }
        self.stop_at_space = save_stop_at_space;
    }

    /// §1884
    /// MLTeX is dropped: returns c.
    pub fn effective_char(&mut self, _err_p: bool, _f: i32, c: i32) -> i32 {
        c
    }

    /// §1885
    /// MLTeX is dropped, so `mltex_enabled_p` is false and this is §1885's first branch, `orig_char_info(f)(c)`,
    /// whether or not the character exists.
    pub fn effective_char_info(&mut self, f: i32, c: i32) -> FourQuarters {
        self.orig_char_info(f, c)
    }

    /// §1930
    /// src-specials are dropped, and `src_specials_p` is always false, so this is never called.
    pub fn insert_src_special(&mut self) {}

    /// §1930
    /// src-specials are dropped, and `src_specials_p` is always false, so this is never called.
    pub fn append_src_special(&mut self) {}

    /// §1931
    pub fn get_nullstr(&mut self) -> i32 {
        S_EMPTY
    }

    // ---- External functions: the C side of web2c and pdfTeX, called by WEB. ----

    /// External: texmfmp.c calledit; `\end` with E, from §1513.  The default is texmf.cnf's `TEXEDIT`, whose
    /// value here decides; C's default, for when it is missing, is `vi +%d '%s'`.
    pub fn call_edit(&mut self, filename_start: i32, fn_length: i32, linenumber: i32) -> ! {
        // Close any open input files: the editor might want to open them for writing, which Windows
        // does not allow while they are open.  Only levels that really are files: state 0 is a token list,
        // and a name below 256 the terminal, a \read or a pseudo-file.
        for is_ptr in 0..self.input_ptr {
            let r = self.input_stack[is_ptr];
            if r.state_field != 0 && r.name_field > 255 {
                let if_ptr = r.index_field;
                if if_ptr < 1 || if_ptr > self.in_open {
                    let msg = format!(
                        ":calledit: unexpected if_ptr={} not in range 1..{},from input_stack[{}].namefield={}\n",
                        if_ptr, self.in_open, is_ptr, r.name_field
                    );
                    let msg = [&rstex_kpse::invocation_name()[..], msg.as_bytes()].concat();
                    self.host.world.stderr_write(&msg);
                    self.uexit(1);
                }
                self.a_close(input_file(if_ptr));
            }
        }
        let edit_value = self.host.world.var_value(b"TEXEDIT").unwrap_or_else(|| b"vi +%d '%s'".to_vec());
        let file: Vec<u8> = (0..fn_length).map(|k| self.xchr.at(self.str_pool.at(filename_start + k)) as u8).collect();
        let edit = match editor_command(&edit_value, &file, linenumber, platform::DOSISH) {
            Ok(edit) => edit,
            // `FATAL` is an `fputs`, so C's `%%` stays as it stands.
            Err(which) => self.web2c_fatal(&[&b"call_edit: `%%"[..], &[which], b"' appears twice in editor command"].concat()),
        };
        // Under `#ifdef WIN32` the first word is a program of the PATH's, and no other.
        let full = match &edit.search_for {
            None => edit.command.clone(),
            Some(name) => {
                if matches!(name.first(), Some(b'.' | b'/' | b'\\')) {
                    self.host.world.stderr_write(&[&name[..], b" is not allowed to execute.\n"].concat());
                    self.uexit(1);
                }
                let Some(found) = platform::search_path(name, ".exe").or_else(|| platform::search_path(name, ".bat")) else {
                    self.host.world.stderr_write(&[b"I cannot find ", &name[..], b" in the PATH.\n"].concat());
                    self.uexit(1)
                };
                [&b"\""[..], &found, b"\"", &edit.command].concat()
            }
        };
        // Execute the command.
        self.host.term_flush();
        if self.host.world.shell(&full) != 0 {
            let msg = [&b"! Trouble executing `"[..], &edit.command, b"'.\n"].concat();
            self.host.world.stderr_write(&msg);
        }
        // Quit, since we found an error.
        self.uexit(1)
    }

    /// kpathsea's `FATAL`.
    fn web2c_fatal(&mut self, s: &[u8]) -> ! {
        let msg = [&rstex_kpse::invocation_name()[..], b": fatal: ", s, b".\n"].concat();
        self.host.world.stderr_write(&msg);
        self.uexit(1)
    }

    /// External: `printcstring`: each byte with print_char
    pub fn print_c_string(&mut self, s: &[u8]) {
        for &c in s {
            self.print_char(c as i32);
        }
    }

    /// External: the .fls recorder, in host.rs
    pub fn recorder_change_filename(&mut self, new_name: &[u8]) {
        self.host.world.recorder_change_filename(new_name);
    }

    /// External: texmfmp.c `getjobname`: `-jobname`, which `maketexstring` turns into a new string each time,
    /// or else the name given.
    pub fn get_job_name(&mut self, name: i32) -> i32 {
        let Some(job) = self.host.c_job_name.take() else {
            return name;
        };
        // maketexstring, with its checkpoolpointer
        if job.is_empty() {
            self.host.c_job_name = Some(job);
            return self.get_nullstr();
        }
        if self.pool_ptr + job.len() as i32 >= self.pool_size {
            let msg = format!("\nstring pool overflow [{} bytes]\n", self.pool_size);
            self.host.world.stderr_write(msg.as_bytes());
            self.uexit(1);
        }
        for &c in &job {
            self.str_pool.put(self.pool_ptr, c as i32);
            self.pool_ptr += 1;
        }
        self.host.c_job_name = Some(job);
        self.make_string()
    }
}

/// What `calledit` has made of the value of `TEXEDIT` by the time it runs it.
#[derive(Debug, PartialEq, Eq)]
struct EditorCommand {
    /// Under `#ifdef WIN32`, the first word, unless the value begins with a drive and a separator: `SearchPath` finds it, and the command is that program in quotes and what follows the word.
    search_for: Option<Vec<u8>>,
    command: Vec<u8>,
}

/// The command string of `calledit`: `%d` the line and `%s` the file, once each, and a `%` otherwise as it stands.  `Err` is the letter that came twice.  A plain function, so that the rules of Windows can be tested on any system.
fn editor_command(value: &[u8], file: &[u8], line: i32, dos: bool) -> Result<EditorCommand, u8> {
    let at = |i: usize| value.get(i).copied().unwrap_or(0);
    let by_drive = |i: usize| at(i).is_ascii_alphabetic() && at(i + 1) == b':' && (at(i + 2) == b'/' || at(i + 2) == b'\\');
    let as_it_stands = !dos || by_drive(0) || (at(0) == b'"' && by_drive(1));
    let mut editor = Vec::new();
    let mut in_the_word = true;
    let mut command = Vec::with_capacity(value.len() + file.len() + 11);
    let (mut sdone, mut ddone) = (false, false);
    let mut i = 0;
    while i < value.len() {
        let c = value[i];
        i += 1;
        if c == b'%' {
            let c = at(i);
            i += 1;
            match c {
                b'd' => {
                    if ddone {
                        return Err(b'd');
                    }
                    command.extend_from_slice(line.to_string().as_bytes());
                    ddone = true;
                }
                b's' => {
                    if sdone {
                        return Err(b's');
                    }
                    command.extend_from_slice(file);
                    sdone = true;
                }
                0 => command.push(b'%'), // the end of the string follows
                _ => {
                    command.push(b'%');
                    command.push(c);
                }
            }
        } else if as_it_stands || !in_the_word {
            command.push(c);
        } else if rstex_kpse::names::is_space(c) {
            // What has gone into the command by now, from a `%` in the word, goes, as C sets its pointer back.
            in_the_word = false;
            command.clear();
            command.push(c);
        } else {
            editor.push(c);
        }
    }
    // C has no end for the name of an editor with nothing after it, and reads on past it.  This ends it where it ends.
    if !as_it_stands && in_the_word {
        command.clear();
    }
    Ok(EditorCommand { search_for: (!as_it_stands).then_some(editor), command })
}

#[cfg(test)]
mod editor_tests {
    use super::*;

    fn unix(v: &str) -> Vec<u8> {
        editor_command(v.as_bytes(), b"a.tex", 12, false).unwrap().command
    }

    fn win(v: &str) -> (Option<Vec<u8>>, Vec<u8>) {
        let e = editor_command(v.as_bytes(), b"a.tex", 12, true).unwrap();
        (e.search_for, e.command)
    }

    #[test]
    fn as_it_stands_on_unix() {
        assert_eq!(unix("vi +%d '%s'"), b"vi +12 'a.tex'");
        assert_eq!(unix("%%x %z %"), b"%%x %z %");
        assert_eq!(editor_command(b"%d %d", b"", 1, false), Err(b'd'));
        assert_eq!(editor_command(b"%s %s", b"", 1, false), Err(b's'));
    }

    #[test]
    fn the_first_word_is_a_program_on_windows() {
        assert_eq!(win("texworks --position=+%d \"%s\""), (Some(b"texworks".to_vec()), b" --position=+12 \"a.tex\"".to_vec()));
        assert_eq!(win("vi +%d '%s'"), (Some(b"vi".to_vec()), b" +12 'a.tex'".to_vec()));
        assert_eq!(win("./x +%d"), (Some(b"./x".to_vec()), b" +12".to_vec()));
        assert_eq!(win("\\\\x y"), (Some(b"\\\\x".to_vec()), b" y".to_vec()));
        assert_eq!(win("notepad"), (Some(b"notepad".to_vec()), Vec::new()));
        assert_eq!(win(" x"), (Some(Vec::new()), b" x".to_vec()));
        // What a % in the word has put in the command goes with the first blank, as C sets its pointer back.
        assert_eq!(win("a%db c%s"), (Some(b"ab".to_vec()), b" ca.tex".to_vec()));
    }

    #[test]
    fn a_drive_leaves_it_as_it_stands_on_windows() {
        assert_eq!(win("c:/x/y.exe +%d %s"), (None, b"c:/x/y.exe +12 a.tex".to_vec()));
        assert_eq!(win("C:\\x\\y.exe %s"), (None, b"C:\\x\\y.exe a.tex".to_vec()));
        assert_eq!(win("\"c:/Program Files/y.exe\" %s"), (None, b"\"c:/Program Files/y.exe\" a.tex".to_vec()));
        assert_eq!(win("\"nodrive\" %s"), (Some(b"\"nodrive\"".to_vec()), b" a.tex".to_vec()));
    }
}
