//! §1867-1933 Parts 54, 55: web2c, string recycling, MLTeX, SyncTeX, index.
//!
//! MLTeX is dropped: effective_char returns c; §1881-1894 need no marker anywhere.
//! DROPPED §1930: src-specials are dropped; get_nullstr stays, at §1931.
//!
//! SyncTeX (§1895-1929) is wired through the `synctex_*` externals, whose call sites stay in the modules
//! that make them.  Their bodies wait for M7, so here they do nothing.
#![forbid(unsafe_code)]
#![deny(unreachable_patterns, unused_variables)]
#![allow(non_snake_case, unused_mut, unused_imports, dead_code)]

use crate::access::*;
use crate::consts::*;
use crate::host::*;
use crate::raw::*;
use crate::state::*;

crate::pod_struct! {
    /// POD state private to this module.  Add fields here, not to state.rs.
    pub struct Vars {}
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

    /// §1876 Either 0 or the number of a string equal to `search`, the newest of those below it.
    pub fn search_string(&mut self, search: i32) -> i32 {
        let len = self.length(search);
        if len == 0 {
            // trivial case
            return S_EMPTY;
        }
        let mut s = search - 1; // start search with newest string below s; search>1!
        while s > 255 {
            // first 256 strings depend on implementation!!
            if self.length(s) == len && self.str_eq_str(s, search) {
                return s;
            }
            s -= 1;
        }
        0
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
        // Construct the command string.
        let mut command = Vec::with_capacity(edit_value.len() + fn_length as usize + 11);
        let (mut sdone, mut ddone) = (false, false);
        let mut i = 0;
        while i < edit_value.len() {
            let c = edit_value[i];
            i += 1;
            if c != b'%' {
                command.push(c);
                continue;
            }
            let c = edit_value.get(i).copied().unwrap_or(0);
            i += 1;
            match c {
                b'd' => {
                    if ddone {
                        self.web2c_fatal(b"call_edit: `%%d' appears twice in editor command");
                    }
                    command.extend_from_slice(linenumber.to_string().as_bytes());
                    ddone = true;
                }
                b's' => {
                    if sdone {
                        self.web2c_fatal(b"call_edit: `%%s' appears twice in editor command");
                    }
                    for k in 0..fn_length {
                        command.push(self.xchr.at(self.str_pool.at(filename_start + k)) as u8);
                    }
                    sdone = true;
                }
                0 => command.push(b'%'), // the end of the string follows
                _ => {
                    command.push(b'%');
                    command.push(c);
                }
            }
        }
        // Execute the command.
        self.host.term_flush();
        if self.host.world.shell(&command) != 0 {
            let msg = [&b"! Trouble executing `"[..], &command, b"'.\n"].concat();
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

    /// External: src-specials are dropped
    pub fn make_src_special(&mut self, _srcfilename: i32, _lineno: i32) -> i32 {
        S_EMPTY
    }

    /// External: src-specials are dropped
    pub fn remember_source_info(&mut self, _srcfilename: i32, _lineno: i32) {}

    /// External: src-specials are dropped
    pub fn is_new_source(&mut self, _srcfilename: i32, _lineno: i32) -> bool {
        false
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

    /// External: IPC is dropped: no-op
    pub fn ipc_page(&mut self, _is_eof: i32) {}

    /// External: synctex.c, M7
    pub fn synctex_abort(&mut self, _log_opened: bool) {}

    /// External: synctex.c, M7
    pub fn synctex_sheet(&mut self, _mag: i32) {}

    /// External: synctex.c, M7
    pub fn synctex_teehs(&mut self) {}

    /// External: synctex.c, M7
    pub fn synctex_vlist(&mut self, _this_box: i32) {}

    /// External: synctex.c, M7
    pub fn synctex_tsilv(&mut self, _this_box: i32) {}

    /// External: synctex.c, M7
    pub fn synctex_void_vlist(&mut self, _p: i32, _this_box: i32) {}

    /// External: synctex.c, M7
    pub fn synctex_hlist(&mut self, _this_box: i32) {}

    /// External: synctex.c, M7
    pub fn synctex_tsilh(&mut self, _this_box: i32) {}

    /// External: synctex.c, M7
    pub fn synctex_void_hlist(&mut self, _p: i32, _this_box: i32) {}

    /// External: synctex.c, M7
    pub fn synctex_math(&mut self, _p: i32, _this_box: i32) {}

    /// External: synctex.c, M7
    pub fn synctex_horizontal_rule_or_glue(&mut self, _p: i32, _this_box: i32) {}

    /// External: synctex.c, M7
    pub fn synctex_kern(&mut self, _p: i32, _this_box: i32) {}

    /// External: synctex.c, M7
    pub fn synctex_char(&mut self, _p: i32, _this_box: i32) {}

    /// External: synctex.c, M7
    pub fn synctex_node(&mut self, _p: i32, _this_box: i32) {}

    /// External: synctex.c, M7
    pub fn synctex_current(&mut self) {}

    /// External: synctex.c, M7
    pub fn synctex_pdfxform(&mut self, _p: i32) {}

    /// External: synctex.c, M7
    pub fn synctex_mrofxfdp(&mut self) {}

    /// External: synctex.c, M7
    pub fn synctex_pdfrefxform(&mut self, _objnum: i32) {}

    /// External: synctex.c, M7
    pub fn synctex_terminate(&mut self, _log_opened: bool) {}

    /// External: synctex.c, M7
    pub fn synctex_init_command(&mut self) {}

    /// External: synctex.c, M7
    pub fn synctex_start_input(&mut self) {}
}
