//! §537-564 Part 29: file names.
//!
//! The file operations themselves (`a_open_in` and the rest, openclose.c) are in host.rs.
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

/// `file_name_size`, §11.  WEB's is `maxint`, and web2c allocates `name_of_file` for each name; here
/// it is an array in the image, so a name is cut to fit, as §545 and §549 say a longer one is.  No file
/// system takes a name this long.
const FILE_NAME_SIZE: i32 = FILE_NAME_CAP as i32 - 2;

/// §560: the months, 1-based as WEB's string with its blank in front.
const MONTHS: &[u8] = b" JANFEBMARAPRMAYJUNJULAUGSEPOCTNOVDEC";

impl Tex {
    /// §541
    pub fn begin_name(&mut self) {
        self.area_delimiter = 0;
        self.ext_delimiter = 0;
        self.quoted_filename = false;
    }

    /// §542
    pub fn more_name(&mut self, c: i32) -> bool {
        if c == b' ' as i32 && self.stop_at_space && !self.quoted_filename {
            false
        } else if c == b'"' as i32 {
            self.quoted_filename = !self.quoted_filename;
            true
        } else {
            self.str_room(1);
            self.append_char(c); // contribute c to the current string
            if is_dir_sep(c) {
                self.area_delimiter = self.cur_length();
                self.ext_delimiter = 0;
            } else if c == b'.' as i32 {
                self.ext_delimiter = self.cur_length();
            }
            true
        }
    }

    /// §543.  Note: a string `slow_make_string` or `search_string` found cannot be flushed.
    pub fn end_name(&mut self) {
        if self.str_ptr + 3 > self.max_strings {
            self.overflow(b"number of strings", self.max_strings - self.init_str_ptr);
        }
        self.str_room(6); // room for quotes, if needed
        // add quotes if needed
        if self.area_delimiter != 0 {
            // maybe quote cur_area
            let mut must_quote = false;
            let s = self.str_start[self.str_ptr];
            let t = self.str_start[self.str_ptr] + self.area_delimiter;
            let mut j = s;
            while !must_quote && j < t {
                must_quote = self.str_pool.at(j) == b' ' as i32;
                j += 1;
            }
            if must_quote {
                for j in (t..=self.pool_ptr - 1).rev() {
                    let c = self.str_pool.at(j);
                    self.str_pool.put(j + 2, c);
                }
                self.str_pool.put(t + 1, b'"' as i32);
                for j in (s..=t - 1).rev() {
                    let c = self.str_pool.at(j);
                    self.str_pool.put(j + 1, c);
                }
                self.str_pool.put(s, b'"' as i32);
                if self.ext_delimiter != 0 {
                    self.ext_delimiter = self.ext_delimiter + 2;
                }
                self.area_delimiter = self.area_delimiter + 2;
                self.pool_ptr = self.pool_ptr + 2;
            }
        }
        // maybe quote cur_name
        let s = self.str_start[self.str_ptr] + self.area_delimiter;
        let t = if self.ext_delimiter == 0 { self.pool_ptr } else { self.str_start[self.str_ptr] + self.ext_delimiter - 1 };
        let mut must_quote = false;
        let mut j = s;
        while !must_quote && j < t {
            must_quote = self.str_pool.at(j) == b' ' as i32;
            j += 1;
        }
        if must_quote {
            for j in (t..=self.pool_ptr - 1).rev() {
                let c = self.str_pool.at(j);
                self.str_pool.put(j + 2, c);
            }
            self.str_pool.put(t + 1, b'"' as i32);
            for j in (s..=t - 1).rev() {
                let c = self.str_pool.at(j);
                self.str_pool.put(j + 1, c);
            }
            self.str_pool.put(s, b'"' as i32);
            if self.ext_delimiter != 0 {
                self.ext_delimiter = self.ext_delimiter + 2;
            }
            self.pool_ptr = self.pool_ptr + 2;
        }
        if self.ext_delimiter != 0 {
            // maybe quote cur_ext
            let s = self.str_start[self.str_ptr] + self.ext_delimiter - 1;
            let t = self.pool_ptr;
            let mut must_quote = false;
            let mut j = s;
            while !must_quote && j < t {
                must_quote = self.str_pool.at(j) == b' ' as i32;
                j += 1;
            }
            if must_quote {
                self.str_pool.put(t + 1, b'"' as i32);
                for j in (s..=t - 1).rev() {
                    let c = self.str_pool.at(j);
                    self.str_pool.put(j + 1, c);
                }
                self.str_pool.put(s, b'"' as i32);
                self.pool_ptr = self.pool_ptr + 2;
            }
        }
        if self.area_delimiter == 0 {
            self.cur_area = S_EMPTY;
        } else {
            self.cur_area = self.str_ptr;
            self.str_start[self.str_ptr + 1] = self.str_start[self.str_ptr] + self.area_delimiter;
            self.str_ptr += 1;
            let temp_str = self.search_string(self.cur_area);
            if temp_str > 0 {
                self.cur_area = temp_str;
                self.str_ptr -= 1; // no flush_string, pool_ptr will be wrong!
                for j in self.str_start[self.str_ptr + 1]..=self.pool_ptr - 1 {
                    let c = self.str_pool.at(j);
                    self.str_pool.put(j - self.area_delimiter, c);
                }
                self.pool_ptr = self.pool_ptr - self.area_delimiter; // update pool_ptr
            }
        }
        if self.ext_delimiter == 0 {
            self.cur_ext = S_EMPTY;
            self.cur_name = self.slow_make_string();
        } else {
            self.cur_name = self.str_ptr;
            self.str_start[self.str_ptr + 1] = self.str_start[self.str_ptr] + self.ext_delimiter - self.area_delimiter - 1;
            self.str_ptr += 1;
            self.cur_ext = self.make_string();
            self.str_ptr -= 1; // undo extension string to look at name part
            let temp_str = self.search_string(self.cur_name);
            if temp_str > 0 {
                self.cur_name = temp_str;
                self.str_ptr -= 1; // no flush_string, pool_ptr will be wrong!
                for j in self.str_start[self.str_ptr + 1]..=self.pool_ptr - 1 {
                    let c = self.str_pool.at(j);
                    self.str_pool.put(j - self.ext_delimiter + self.area_delimiter + 1, c);
                }
                self.pool_ptr = self.pool_ptr - self.ext_delimiter + self.area_delimiter + 1; // update pool_ptr
            }
            self.cur_ext = self.slow_make_string(); // remake extension string
        }
    }

    /// §544 `check_quoted`: does string `s` need quoting?
    fn check_quoted(&self, s: i32, must_quote: &mut bool) {
        if s != 0 {
            let mut j = self.str_start[s];
            while !*must_quote && j < self.str_start[s + 1] {
                *must_quote = self.str_pool.at(j) == b' ' as i32;
                j += 1;
            }
        }
    }

    /// §544 `print_quoted`: print string `s`, omitting quotes.
    fn print_quoted(&mut self, s: i32) {
        if s != 0 {
            for j in self.str_start[s]..self.str_start[s + 1] {
                let c = self.str_pool.at(j);
                if c != b'"' as i32 {
                    self.print(c);
                }
            }
        }
    }

    /// §544
    pub fn print_file_name(&mut self, n: i32, a: i32, e: i32) {
        let mut must_quote = false;
        self.check_quoted(a, &mut must_quote);
        self.check_quoted(n, &mut must_quote);
        self.check_quoted(e, &mut must_quote);
        if must_quote {
            self.print_char(b'"' as i32);
        }
        self.print_quoted(a);
        self.print_quoted(n);
        self.print_quoted(e);
        if must_quote {
            self.print_char(b'"' as i32);
        }
    }

    /// §545 `append_to_name`
    fn append_to_name(&mut self, k: &mut i32, c: i32) {
        if c != b'"' as i32 {
            *k += 1;
            if *k <= FILE_NAME_SIZE {
                let x = self.xchr.at(c);
                self.name_of_file.put(*k, x);
            }
        }
    }

    /// §545
    pub fn pack_file_name(&mut self, n: i32, a: i32, e: i32) {
        let mut k = 0;
        for j in self.str_start[a]..self.str_start[a + 1] {
            let c = self.str_pool.at(j);
            self.append_to_name(&mut k, c);
        }
        for j in self.str_start[n]..self.str_start[n + 1] {
            let c = self.str_pool.at(j);
            self.append_to_name(&mut k, c);
        }
        for j in self.str_start[e]..self.str_start[e + 1] {
            let c = self.str_pool.at(j);
            self.append_to_name(&mut k, c);
        }
        self.name_length = if k <= FILE_NAME_SIZE { k } else { FILE_NAME_SIZE };
        self.name_of_file.put(self.name_length + 1, 0);
    }

    /// §549
    pub fn pack_buffered_name(&mut self, n: i32, a: i32, b: i32) {
        let mut b = b;
        if n + b - a + 1 + FORMAT_EXT_LENGTH > FILE_NAME_SIZE {
            b = a + FILE_NAME_SIZE - n - 1 - FORMAT_EXT_LENGTH;
        }
        let mut k = 0;
        for j in 1..=n {
            let c = self.host.tex_format_default[j as usize] as i32;
            let c = self.xord.at(c);
            self.append_to_name(&mut k, c);
        }
        for j in a..=b {
            let c = self.buffer.at(j);
            self.append_to_name(&mut k, c);
        }
        for j in self.format_default_length - FORMAT_EXT_LENGTH + 1..=self.format_default_length {
            let c = self.host.tex_format_default[j as usize] as i32;
            let c = self.xord.at(c);
            self.append_to_name(&mut k, c);
        }
        self.name_length = if k <= FILE_NAME_SIZE { k } else { FILE_NAME_SIZE };
        self.name_of_file.put(self.name_length + 1, 0);
    }

    /// §550
    pub fn open_fmt_file(&mut self) -> bool {
        let mut j = self.loc(); // the first space after the format file name
        'found: {
            if self.buffer.at(self.loc()) == b'&' as i32 {
                self.set_loc(self.loc() + 1);
                j = self.loc();
                self.buffer.put(self.last, b' ' as i32);
                while self.buffer.at(j) != b' ' as i32 {
                    j += 1;
                }
                self.pack_buffered_name(0, self.loc(), j - 1); // Kpathsea does everything
                if self.w_open_in(FMT_FILE) {
                    break 'found;
                }
                self.wake_up_terminal();
                self.wterm_s(b"Sorry, I can't find the format `");
                let name = self.name_of_file_bytes();
                self.wterm_s(&name);
                self.wterm_s(b"'; will try `");
                let dflt = core::mem::take(&mut self.host.tex_format_default);
                self.wterm_s(&dflt[1..]);
                self.host.tex_format_default = dflt;
                self.wterm_ln_s(b"'.");
                self.update_terminal();
            }
            // now pull out all the stops: try for the system plain file
            self.pack_buffered_name(self.format_default_length - FORMAT_EXT_LENGTH, 1, 0);
            if !self.w_open_in(FMT_FILE) {
                self.wake_up_terminal();
                self.wterm_s(b"I can't find the format file `");
                let dflt = core::mem::take(&mut self.host.tex_format_default);
                self.wterm_s(&dflt[1..]);
                self.host.tex_format_default = dflt;
                self.wterm_ln_s(b"'!");
                return false;
            }
        }
        // found:
        self.set_loc(j);
        true
    }

    /// §551
    pub fn make_name_string(&mut self) -> i32 {
        if self.pool_ptr + self.name_length > self.pool_size || self.str_ptr == self.max_strings || self.cur_length() > 0 {
            b'?' as i32
        } else {
            for k in 1..=self.name_length {
                let c = self.xord.at(self.name_of_file.at(k));
                self.append_char(c);
            }
            let result = self.make_string();
            // At this point we also set cur_name, cur_ext, and cur_area to match the contents of
            // name_of_file.
            let save_area_delimiter = self.area_delimiter;
            let save_ext_delimiter = self.ext_delimiter;
            let save_name_in_progress = self.name_in_progress;
            let save_stop_at_space = self.stop_at_space;
            self.name_in_progress = true;
            self.begin_name();
            self.stop_at_space = false;
            let mut k = 1;
            while k <= self.name_length && self.more_name(self.name_of_file.at(k)) {
                k += 1;
            }
            self.stop_at_space = save_stop_at_space;
            self.end_name();
            self.name_in_progress = save_name_in_progress;
            self.area_delimiter = save_area_delimiter;
            self.ext_delimiter = save_ext_delimiter;
            result
        }
    }

    /// §551
    pub fn a_make_name_string(&mut self, _f: FileId) -> i32 {
        self.make_name_string()
    }

    /// §551
    pub fn b_make_name_string(&mut self, _f: FileId) -> i32 {
        self.make_name_string()
    }

    /// §551
    pub fn w_make_name_string(&mut self, _f: FileId) -> i32 {
        self.make_name_string()
    }

    /// §552
    pub fn scan_file_name(&mut self) {
        self.name_in_progress = true;
        let save_warning_index = self.warning_index;
        self.warning_index = self.cur_cs; // store cur_cs here to remember until later
        // §432 Get the next non-blank non-call token.  The comment here in tex.ch says \relax goes too; the code
        // does not do that.
        loop {
            self.get_x_token();
            if self.cur_cmd != SPACER {
                break;
            }
        }
        self.back_input(); // return the last token to be read by either code path
        if self.cur_cmd == LEFT_BRACE {
            self.scan_file_name_braced();
            self.name_in_progress = false;
        } else {
            self.name_in_progress = true;
            self.begin_name();
            // §432
            loop {
                self.get_x_token();
                if self.cur_cmd != SPACER {
                    break;
                }
            }
            loop {
                if self.cur_cmd > OTHER_CHAR || self.cur_chr > 255 {
                    // not a character
                    self.back_input();
                    break;
                }
                // If cur_chr is a space and we're not scanning a token list, check whether we're at the end of
                // the buffer.  Otherwise we end up adding spurious spaces to file names in some cases.
                if self.cur_chr == b' ' as i32 && self.state() != TOKEN_LIST && self.loc() > self.limit() {
                    break;
                }
                if !self.more_name(self.cur_chr) {
                    break;
                }
                self.get_x_token();
            }
        }
        // done:
        self.end_name();
        self.name_in_progress = false;
        self.warning_index = save_warning_index; // restore warning_index
    }

    /// §555
    pub fn pack_job_name(&mut self, s: i32) {
        self.cur_area = S_EMPTY;
        self.cur_ext = s;
        self.cur_name = self.job_name;
        self.pack_cur_name();
    }

    /// §556
    pub fn prompt_file_name(&mut self, s: &[u8], e: i32) {
        if self.interaction == SCROLL_MODE {
            self.wake_up_terminal();
        }
        if s == b"input file name" {
            self.print_err_s(b"I can't find file `");
        } else {
            self.print_err_s(b"I can't write on file `");
        }
        self.print_file_name(self.cur_name, self.cur_area, self.cur_ext);
        self.print_s(b"'.");
        if e == S_DOT_TEX || e == S_EMPTY {
            self.show_context();
        }
        self.print_ln();
        self.print_c_string(PROMPT_FILE_NAME_HELP_MSG);
        if e != S_EMPTY {
            self.print_s(b"; default file extension is `");
            self.print(e);
            self.print(b'\'' as i32);
        }
        self.print(b')' as i32);
        self.print_ln();
        self.print_nl_s(b"Please type another ");
        self.print_s(s);
        if self.interaction < SCROLL_MODE {
            self.fatal_error(b"*** (job aborted, file error in nonstop mode)");
        }
        let saved_cur_name = self.cur_name; // to catch empty terminal input
        let saved_cur_ext = self.cur_ext;
        let saved_cur_area = self.cur_area;
        self.clear_terminal();
        self.prompt_input_s(b": ");
        // §557 Scan file name in the buffer
        self.begin_name();
        let mut k = self.first;
        while self.buffer.at(k) == b' ' as i32 && k < self.last {
            k += 1;
        }
        loop {
            if k == self.last {
                break;
            }
            if !self.more_name(self.buffer.at(k)) {
                break;
            }
            k += 1;
        }
        // done:
        self.end_name();
        if self.length(self.cur_name) == 0 && self.cur_ext == S_EMPTY && self.cur_area == S_EMPTY {
            self.cur_name = saved_cur_name;
            self.cur_ext = saved_cur_ext;
            self.cur_area = saved_cur_area;
        } else if self.cur_ext == S_EMPTY {
            self.cur_ext = e;
        }
        self.pack_cur_name();
    }

    /// §560
    pub fn open_log_file(&mut self) {
        let old_setting = self.selector;
        if self.job_name == 0 {
            self.job_name = self.get_job_name(S_TEXPUT);
        }
        self.pack_job_name(S_DOT_FLS);
        let fls = self.name_of_file_bytes();
        self.recorder_change_filename(&fls);
        self.pack_job_name(S_DOT_LOG);
        while !self.a_open_out(LOG_FILE) {
            // §561 Try to get a different log file name
            self.selector = TERM_ONLY;
            self.prompt_file_name(b"transcript file name", S_DOT_LOG);
        }
        self.log_name = self.a_make_name_string(LOG_FILE);
        self.selector = LOG_ONLY;
        self.log_opened = true;
        // §562 Print the banner line, including the date and time.  banner_k and banner are the same here.
        self.wlog_s(BANNER);
        self.wlog_s(VERSION_STRING);
        self.slow_print(self.format_ident);
        self.print_s(b"  ");
        self.print_int(self.sys_day);
        self.print_char(b' ' as i32);
        for k in 3 * self.sys_month - 2..=3 * self.sys_month {
            self.wlog_char(MONTHS[k as usize] as i32);
        }
        self.print_char(b' ' as i32);
        self.print_int(self.sys_year);
        self.print_char(b' ' as i32);
        self.print_two(self.sys_time / 60);
        self.print_char(b':' as i32);
        self.print_two(self.sys_time % 60);
        if self.eTeX_ex() {
            self.wlog_cr();
            self.wlog_s(b"entering extended mode");
        }
        if self.shellenabledp != 0 {
            self.wlog_cr();
            self.wlog_s(b" ");
            if self.restrictedshell != 0 {
                self.wlog_s(b"restricted ");
            }
            self.wlog_s(b"\\write18 enabled.");
        }
        if self.src_specials_p {
            self.wlog_cr();
            self.wlog_s(b" Source specials enabled.");
        }
        if self.file_line_error_style_p != 0 {
            self.wlog_cr();
            self.wlog_s(b" file:line:error style messages enabled.");
        }
        if self.parse_first_line_p != 0 {
            self.wlog_cr();
            self.wlog_s(b" %&-line parsing enabled.");
        }
        if let Some(tcx) = self.host.translate_filename.take() {
            self.wlog_cr();
            self.wlog_s(b" (");
            self.wlog_s(&tcx);
            self.wlog_s(b")");
            self.host.translate_filename = Some(tcx);
        }
        // dropped: MLTeX, `MLTeX v2.2 enabled`
        self.input_stack[self.input_ptr] = self.cur_input; // make sure bottom level is in memory
        self.print_nl_s(b"**");
        let mut l = self.input_stack[0].limit_field; // last position of first line
        if self.buffer.at(l) == self.end_line_char() {
            l -= 1;
        }
        for k in 1..=l {
            self.print(self.buffer.at(k));
        }
        self.print_ln(); // now the transcript file contains the first line of input
        self.selector = old_setting + 2; // log_only or term_and_log
    }

    /// §563
    pub fn start_input(&mut self) {
        self.scan_file_name(); // set cur_name to desired file name
        self.pack_cur_name();
        loop {
            self.begin_file_reading(); // set up cur_file and new level of input
            self.tex_input_type = 1; // tell open_input we are \input
            // Kpathsea tries all the various ways to get the file.
            if self.kpse_in_name_ok() && self.a_open_in(self.cur_file(), KPSE_TEX_FORMAT) {
                break;
            }
            self.end_file_reading(); // remove the level that didn't work
            self.prompt_file_name(b"input file name", S_EMPTY);
        }
        // done:
        crate::prof_phase!("first input read");
        let n = self.a_make_name_string(self.cur_file());
        self.set_name(n);
        self.source_filename_stack[self.in_open] = self.name();
        let full = self.make_full_name_string();
        self.full_source_filename_stack[self.in_open] = full;
        if self.name() == self.str_ptr - 1 {
            // we can try to conserve string pool space now
            let temp_str = self.search_string(self.name());
            if temp_str > 0 {
                self.set_name(temp_str);
                self.flush_string();
            }
        }
        if self.job_name == 0 {
            self.job_name = self.get_job_name(self.cur_name);
            self.open_log_file();
        } // open_log_file doesn't show_context, so limit and loc needn't be set to meaningful values yet
        if self.term_offset + self.length(self.full_source_filename_stack[self.in_open]) > self.max_print_line - 2 {
            self.print_ln();
        } else if self.term_offset > 0 || self.file_offset > 0 {
            self.print_char(b' ' as i32);
        }
        self.print_char(b'(' as i32);
        self.open_parens += 1;
        self.slow_print(self.full_source_filename_stack[self.in_open]);
        self.update_terminal();
        if self.tracing_stack_levels() > 0 {
            self.begin_diagnostic();
            self.print_ln();
            self.print_char(b'~' as i32);
            let mut v = self.input_ptr - 1;
            if v < self.tracing_stack_levels() {
                while v > 0 {
                    self.print_char(b'.' as i32);
                    v -= 1;
                }
            } else {
                self.print_char(b'~' as i32);
            }
            self.print_s(b"INPUT ");
            self.slow_print(self.cur_name);
            self.slow_print(self.cur_ext);
            self.print_ln();
            self.end_diagnostic(false);
        }
        self.set_state(NEW_LINE);
        // §1909 Prepare new file SyncTeX information
        self.synctex_start_input(); // give control to the SyncTeX controller
        // §564 Read the first line of the new file
        self.line = 1;
        let _ = self.input_ln(self.cur_file(), false);
        self.firm_up_the_line();
        if self.end_line_char_inactive() {
            self.set_limit(self.limit() - 1);
        } else {
            self.buffer.put(self.limit(), self.end_line_char());
        }
        self.first = self.limit() + 1;
        self.set_loc(self.start());
    }

    // ---- External functions: the C side of web2c and pdfTeX, called by WEB. ----

    /// External: texmfmp.c `makefullnamestring`: the string of `host.full_name_of_file`, not looked for
    /// among the existing strings, and the null string if nothing was opened.  `maketexstring` copies bytes,
    /// without `xord`.
    pub fn make_full_name_string(&mut self) -> i32 {
        let Some(name) = self.host.full_name_of_file.take() else {
            return self.get_nullstr();
        };
        if name.is_empty() {
            self.host.full_name_of_file = Some(name);
            return self.get_nullstr();
        }
        // checkpoolpointer
        if self.pool_ptr + name.len() as i32 >= self.pool_size {
            let msg = format!("\nstring pool overflow [{} bytes]\n", self.pool_size);
            self.host.world.stderr_write(msg.as_bytes());
            self.uexit(1);
        }
        for &c in &name {
            self.str_pool.put(self.pool_ptr, c as i32);
            self.pool_ptr += 1;
        }
        self.host.full_name_of_file = Some(name);
        self.make_string()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::toks::testkit::*;

    pub fn name_machine() -> &'static mut Tex {
        let t = machine();
        print_setup(t);
        t.stop_at_space = true;
        t.max_strings = 100000;
        t.pool_size = 1000000;
        t.init_str_ptr = t.str_ptr;
        t.xchr = {
            let mut x = t.xchr;
            for k in 0..256 {
                x.put(k, k);
            }
            x
        };
        t
    }

    fn string(t: &Tex, s: i32) -> Vec<u8> {
        t.str_pool.range(t.str_start[s], t.str_start[s + 1]).to_vec()
    }

    /// The protocol of begin_name, more_name and end_name over the characters of `name`.
    fn scan(t: &mut Tex, name: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        t.begin_name();
        for &c in name {
            if !t.more_name(c as i32) {
                break;
            }
        }
        t.end_name();
        (string(t, t.cur_area), string(t, t.cur_name), string(t, t.cur_ext))
    }

    fn v(s: &str) -> Vec<u8> {
        s.as_bytes().to_vec()
    }

    #[test]
    fn a_name_splits_into_area_name_and_extension() {
        let t = name_machine();
        let cases = [
            ("foo", "", "foo", ""),
            ("foo.tex", "", "foo", ".tex"),
            ("a/b/foo.tex", "a/b/", "foo", ".tex"),
            ("a.b/foo", "a.b/", "foo", ""),
            ("a.b.c", "", "a.b", ".c"),
            ("/abs", "/", "abs", ""),
            (".hidden", "", "", ".hidden"),
            ("dir/", "dir/", "", ""),
            ("trailing.", "", "trailing", "."),
            ("", "", "", ""),
        ];
        for (input, area, name, ext) in cases {
            assert_eq!(scan(t, input.as_bytes()), (v(area), v(name), v(ext)), "{}", input);
        }
        Tex::dealloc(t);
    }

    #[test]
    fn a_space_ends_a_name_unless_quoted_or_stop_at_space_is_off() {
        let t = name_machine();
        assert_eq!(scan(t, b"foo bar"), (v(""), v("foo"), v("")));
        // Quotes go, and end_name gives them back to the parts that hold a space.
        assert_eq!(scan(t, b"\"foo bar\""), (v(""), v("\"foo bar\""), v("")));
        assert_eq!(scan(t, b"\"foo bar.tex\""), (v(""), v("\"foo bar\""), v(".tex")));
        assert_eq!(scan(t, b"\"a b/foo.tex\""), (v("\"a b/\""), v("foo"), v(".tex")));
        assert_eq!(scan(t, b"\"foo.t x\""), (v(""), v("foo"), v("\".t x\"")));
        assert_eq!(scan(t, b"\"a b/c d.e f\""), (v("\"a b/\""), v("\"c d\""), v("\".e f\"")));
        assert_eq!(scan(t, b"fo\"o b\"ar"), (v(""), v("\"foo bar\""), v("")));
        t.stop_at_space = false;
        assert_eq!(scan(t, b"foo bar"), (v(""), v("\"foo bar\""), v("")));
        Tex::dealloc(t);
    }

    /// `slow_make_string` and `search_string`: the same text gives the same string, and no pool space.
    #[test]
    fn end_name_recycles_strings() {
        let t = name_machine();
        scan(t, b"dir/foo.xyz");
        let (a, n, e) = (t.cur_area, t.cur_name, t.cur_ext);
        let (strs, pool) = (t.str_ptr, t.pool_ptr);
        scan(t, b"dir/foo.xyz");
        assert_eq!((t.cur_area, t.cur_name, t.cur_ext), (a, n, e));
        assert_eq!((t.str_ptr, t.pool_ptr), (strs, pool));
        assert_eq!(string(t, a), b"dir/");
        // A well-known string serves too.
        scan(t, b"new.tex");
        assert_eq!(t.cur_ext, S_DOT_TEX);
        assert_eq!(string(t, t.cur_name), b"new");
        Tex::dealloc(t);
    }

    #[test]
    fn search_string_finds_the_newest_above_255() {
        let t = name_machine();
        for &c in b"zz" {
            t.append_char(c as i32);
        }
        let a = t.make_string();
        for &c in b"zz" {
            t.append_char(c as i32);
        }
        let b = t.make_string();
        for &c in b"zz" {
            t.append_char(c as i32);
        }
        let c = t.make_string();
        assert_eq!(t.search_string(c), b);
        assert_eq!(t.search_string(b), a);
        assert_eq!(t.search_string(a), 0);
        t.append_char(b'y' as i32);
        let one = t.make_string();
        assert_eq!(t.search_string(one), 0, "the single characters are not looked at");
        let empty = t.make_string();
        assert_eq!(t.search_string(empty), S_EMPTY);
        Tex::dealloc(t);
    }

    /// What a format's strings give, through `str_index`, is what looking at each of them gives.
    #[test]
    fn search_string_through_the_index_is_as_good() {
        let t = name_machine();
        let mut add = |t: &mut Tex, s: &[u8]| {
            for &c in s {
                t.append_char(c as i32);
            }
            t.make_string()
        };
        for s in [&b"aa"[..], b"bb", b"aa", b"c", b"bb", b"", b"dddddddd"] {
            add(t, s);
        }
        let format_strings = t.str_ptr;
        // One by one, as with no index.
        let mut plain = Vec::new();
        for s in [&b"aa"[..], b"bb", b"c", b"dddddddd", b"none", b"bb", b"e", b"e", b""] {
            let n = add(t, s);
            plain.push((n, t.search_string(n)));
        }
        // The same strings with the first part indexed, as after a load.
        let top = t.str_ptr;
        t.str_ptr = format_strings;
        t.init_str_ptr = format_strings;
        t.str_index_make();
        t.str_ptr = top;
        assert!(t.v_web2c.str_index.cap > 0);
        for (n, found) in plain {
            assert_eq!(t.search_string(n), found, "string {n}");
        }
        t.v_web2c.str_index = PodVec::EMPTY;
        Tex::dealloc(t);
    }

    #[test]
    fn pack_file_name_drops_quotes_and_joins() {
        let t = name_machine();
        scan(t, b"\"a b/c d.e f\"");
        t.pack_cur_name();
        assert_eq!(t.name_of_file_bytes(), b"a b/c d.e f");
        assert_eq!(t.name_of_file.at(t.name_length + 1), 0);
        Tex::dealloc(t);
    }

    #[test]
    fn print_file_name_quotes_the_whole_name_or_nothing() {
        let t = name_machine();
        scan(t, b"\"a b/c.e\"");
        let (n, a, e) = (t.cur_name, t.cur_area, t.cur_ext);
        printed(t);
        t.print_file_name(n, a, e);
        assert_eq!(printed(t), "\"a b/c.e\"");
        scan(t, b"a/c.e");
        let (n, a, e) = (t.cur_name, t.cur_area, t.cur_ext);
        t.print_file_name(n, a, e);
        assert_eq!(printed(t), "a/c.e");
        // A zero string number prints nothing.
        t.print_file_name(n, 0, 0);
        assert_eq!(printed(t), "c");
        Tex::dealloc(t);
    }

    #[test]
    fn make_name_string_sets_the_current_names() {
        let t = name_machine();
        t.set_name_of_file(b"some dir/x y.tex");
        t.area_delimiter = 5;
        t.ext_delimiter = 6;
        t.name_in_progress = false;
        let s = t.make_name_string();
        assert_eq!(string(t, s), b"some dir/x y.tex");
        assert_eq!(string(t, t.cur_area), b"\"some dir/\"");
        assert_eq!(string(t, t.cur_name), b"\"x y\"");
        assert_eq!(t.cur_ext, S_DOT_TEX);
        assert_eq!((t.area_delimiter, t.ext_delimiter, t.name_in_progress, t.stop_at_space), (5, 6, false, true));
        // While a string is in progress the answer is "?", with no string made.
        t.append_char(b'z' as i32);
        let before = t.str_ptr;
        assert_eq!(t.make_name_string(), b'?' as i32);
        assert_eq!(t.str_ptr, before);
        t.flush_char();
        Tex::dealloc(t);
    }

    #[test]
    fn pack_buffered_name_takes_format_parts() {
        let t = name_machine();
        t.host.tex_format_default = b" plain.fmt".to_vec();
        t.format_default_length = 9;
        t.pack_buffered_name(t.format_default_length - FORMAT_EXT_LENGTH, 1, 0);
        assert_eq!(t.name_of_file_bytes(), b"plain.fmt");
        t.buffer.put(10, b'x' as i32);
        t.buffer.put(11, b'y' as i32);
        t.pack_buffered_name(0, 10, 11);
        assert_eq!(t.name_of_file_bytes(), b"xy.fmt");
        Tex::dealloc(t);
    }

    #[test]
    fn make_full_name_string_makes_a_new_string_or_the_null_string() {
        let t = name_machine();
        t.host.full_name_of_file = None;
        assert_eq!(t.make_full_name_string(), S_EMPTY);
        t.host.full_name_of_file = Some(Vec::new());
        assert_eq!(t.make_full_name_string(), S_EMPTY);
        t.host.full_name_of_file = Some(b"./a.tex".to_vec());
        let a = t.make_full_name_string();
        let b = t.make_full_name_string();
        assert_eq!(string(t, a), b"./a.tex");
        assert_ne!(a, b, "it does not look for the string");
        Tex::dealloc(t);
    }

    #[test]
    fn pack_job_name_uses_the_job_name() {
        let t = name_machine();
        scan(t, b"job");
        t.job_name = t.cur_name;
        t.cur_area = 300;
        t.pack_job_name(S_DOT_LOG);
        assert_eq!(t.name_of_file_bytes(), b"job.log");
        assert_eq!((t.cur_area, t.cur_ext, t.cur_name), (S_EMPTY, S_DOT_LOG, t.job_name));
        Tex::dealloc(t);
    }
}
