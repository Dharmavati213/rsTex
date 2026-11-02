//! §72-98 Part 6: reporting errors.
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
    /// §81
    pub fn jump_out(&mut self) -> ! {
        self.close_files_and_terminate();
        self.do_final_end()
    }

    /// §82
    pub fn error(&mut self) {
        // The cases of §84, on what the user types.
        const CH_0: i32 = b'0' as i32;
        const CH_9: i32 = b'9' as i32;
        const CH_E: i32 = b'E' as i32;
        const CH_H: i32 = b'H' as i32;
        const CH_I: i32 = b'I' as i32;
        const CH_Q: i32 = b'Q' as i32;
        const CH_R: i32 = b'R' as i32;
        const CH_S: i32 = b'S' as i32;
        const CH_X: i32 = b'X' as i32;
        // `debug "D"` in §84 is not compiled in the oracle.
        if self.history < ERROR_MESSAGE_ISSUED {
            self.history = ERROR_MESSAGE_ISSUED;
        }
        self.print_char(b'.' as i32);
        self.show_context();
        if self.halt_on_error_p != 0 {
            // If close_files_and_terminate generates an error, we'll end up back here; just give up
            // in that case.  If files are truncated, too bad.
            if self.halting_on_error_p {
                self.do_final_end();
            }
            self.halting_on_error_p = true;
            self.errors_put_help_on_transcript();
            self.history = FATAL_ERROR_STOP;
            self.jump_out();
        }
        if self.interaction == ERROR_STOP_MODE {
            // §83 Get user's advice and return
            loop {
                if self.interaction != ERROR_STOP_MODE {
                    return;
                }
                self.clear_for_error_prompt();
                self.prompt_input_s(b"? ");
                if self.last == self.first {
                    return;
                }
                let mut c = self.buffer.at(self.first);
                if c >= b'a' as i32 {
                    c = c + b'A' as i32 - b'a' as i32;
                }
                // §84 Interpret code c and return if done
                match c {
                    CH_0..=CH_9 => {
                        if self.deletions_allowed {
                            // §88 Delete c-"0" tokens and goto continue
                            let s1 = self.cur_tok;
                            let s2 = self.cur_cmd;
                            let s3 = self.cur_chr;
                            let s4 = self.align_state;
                            self.align_state = 1000000;
                            self.OK_to_interrupt = false;
                            if self.last > self.first + 1
                                && self.buffer.at(self.first + 1) >= b'0' as i32
                                && self.buffer.at(self.first + 1) <= b'9' as i32
                            {
                                c = c * 10 + self.buffer.at(self.first + 1) - b'0' as i32 * 11;
                            } else {
                                c = c - b'0' as i32;
                            }
                            while c > 0 {
                                self.get_token(); // one-level recursive call of error is possible
                                c -= 1;
                            }
                            self.cur_tok = s1;
                            self.cur_cmd = s2;
                            self.cur_chr = s3;
                            self.align_state = s4;
                            self.OK_to_interrupt = true;
                            self.help(&[
                                b"I have just deleted some text, as you asked.",
                                b"You can now delete more, or insert, or whatever.",
                            ]);
                            self.show_context();
                            continue;
                        }
                    }
                    CH_E => {
                        if self.base_ptr > 0 && self.input_stack[self.base_ptr].name_field >= 256 {
                            let n = self.input_stack[self.base_ptr].name_field;
                            self.edit_name_start = self.str_start[n];
                            self.edit_name_length = self.str_start[n + 1] - self.str_start[n];
                            self.edit_line = self.line;
                            self.jump_out();
                        }
                    }
                    CH_H => {
                        // §89 Print the help information and goto continue
                        if self.use_err_help {
                            self.give_err_help();
                            self.use_err_help = false;
                        } else {
                            if self.help_ptr == 0 {
                                self.help(&[
                                    b"Sorry, I don't know how to help in this situation.",
                                    b"Maybe you should try asking a human?",
                                ]);
                            }
                            loop {
                                self.help_ptr -= 1;
                                let l = self.help_line(self.help_ptr);
                                self.print_s(l);
                                self.print_ln();
                                if self.help_ptr == 0 {
                                    break;
                                }
                            }
                        }
                        self.help(&[
                            b"Sorry, I already gave what help I could...",
                            b"Maybe you should try asking a human?",
                            b"An error might have occurred before I noticed any problems.",
                            b"``If all else fails, read the instructions.''",
                        ]);
                        continue;
                    }
                    CH_I => {
                        // §87 Introduce new material from the terminal and return
                        self.begin_file_reading(); // now state=mid_line, so an initial blank counts
                        if self.last > self.first + 1 {
                            self.set_loc(self.first + 1);
                            self.buffer.put(self.first, b' ' as i32);
                        } else {
                            self.prompt_input_s(b"insert>");
                            self.set_loc(self.first);
                        }
                        self.first = self.last;
                        self.cur_input.limit_field = self.last - 1; // no end_line_char ends this line
                        return;
                    }
                    CH_Q | CH_R | CH_S => {
                        // §86 Change the interaction level and return
                        self.error_count = 0;
                        self.interaction = BATCH_MODE + c - CH_Q;
                        self.print_s(b"OK, entering ");
                        match c {
                            CH_Q => {
                                self.print_esc_s(b"batchmode");
                                self.selector -= 1;
                            }
                            CH_R => self.print_esc_s(b"nonstopmode"),
                            CH_S => self.print_esc_s(b"scrollmode"),
                            _ => {}
                        }
                        self.print_s(b"...");
                        self.print_ln();
                        self.update_terminal();
                        return;
                    }
                    CH_X => {
                        self.interaction = SCROLL_MODE;
                        self.jump_out();
                    }
                    _ => {}
                }
                // §85 Print the menu of available options
                self.print_s(b"Type <return> to proceed, S to scroll future error messages,");
                self.print_nl_s(b"R to run without stopping, Q to run quietly,");
                self.print_nl_s(b"I to insert something, ");
                if self.base_ptr > 0 && self.input_stack[self.base_ptr].name_field >= 256 {
                    self.print_s(b"E to edit your file,");
                }
                if self.deletions_allowed {
                    self.print_nl_s(b"1 or ... or 9 to ignore the next 1 to 9 tokens of input,");
                }
                self.print_nl_s(b"H for help, X to quit.");
            }
        }
        self.error_count += 1;
        if self.error_count == 100 {
            self.print_nl_s(b"(That makes 100 errors; please try again.)");
            self.history = FATAL_ERROR_STOP;
            self.jump_out();
        }
        self.errors_put_help_on_transcript();
    }

    /// §90 Put help message on the transcript file
    fn errors_put_help_on_transcript(&mut self) {
        if self.interaction > BATCH_MODE {
            self.selector -= 1; // avoid terminal output
        }
        if self.use_err_help {
            self.print_ln();
            self.give_err_help();
        } else {
            while self.help_ptr > 0 {
                self.help_ptr -= 1;
                let l = self.help_line(self.help_ptr);
                self.print_nl_s(l);
            }
        }
        self.print_ln();
        if self.interaction > BATCH_MODE {
            self.selector += 1; // re-enable terminal output
        }
        self.print_ln();
    }

    /// §91
    pub fn int_error(&mut self, n: i32) {
        self.print_s(b" (");
        self.print_int(n);
        self.print_char(b')' as i32);
        self.error();
    }

    /// §92
    pub fn normalize_selector(&mut self) {
        if self.log_opened {
            self.selector = TERM_AND_LOG;
        } else {
            self.selector = TERM_ONLY;
        }
        if self.job_name == 0 {
            self.open_log_file();
        }
        if self.interaction == BATCH_MODE {
            self.selector -= 1;
        }
    }

    /// §93
    pub fn fatal_error(&mut self, s: &[u8]) -> ! {
        self.normalize_selector();
        self.print_err_s(b"Emergency stop");
        // help1(s).  Help lines are static and `s` need not be; this path ends the process.
        self.help_ptr = 1;
        self.set_help_line(0, Box::leak(s.to_vec().into_boxed_slice()));
        self.succumb()
    }

    /// §94
    pub fn overflow(&mut self, s: &[u8], n: i32) -> ! {
        self.normalize_selector();
        self.print_err_s(b"TeX capacity exceeded, sorry [");
        self.print_s(s);
        self.print_char(b'=' as i32);
        self.print_int(n);
        self.print_char(b']' as i32);
        self.help(&[
            b"If you really absolutely need more capacity,",
            b"you can ask a wizard to enlarge me.",
        ]);
        self.succumb()
    }

    /// §95
    pub fn confusion(&mut self, s: &[u8]) -> ! {
        self.normalize_selector();
        if self.history < ERROR_MESSAGE_ISSUED {
            self.print_err_s(b"This can't happen (");
            self.print_s(s);
            self.print_char(b')' as i32);
            self.help(&[b"I'm broken. Please show this to someone who can fix can fix"]);
        } else {
            self.print_err_s(b"I can't go on meeting you like this");
            self.help(&[
                b"One of your faux pas seems to have wounded me deeply...",
                b"in fact, I'm barely conscious. Please fix it and try again.",
            ]);
        }
        self.succumb()
    }

    /// §98
    pub fn pause_for_instructions(&mut self) {
        if self.OK_to_interrupt {
            self.interaction = ERROR_STOP_MODE;
            if self.selector == LOG_ONLY || self.selector == NO_PRINT {
                self.selector += 1;
            }
            self.print_err_s(b"Interruption");
            self.help(&[
                b"You rang?",
                b"Try to insert an instruction for me (e.g., `I\\showlists'),",
                b"unless you just want to quit by typing `X'.",
            ]);
            self.deletions_allowed = false;
            self.error();
            self.deletions_allowed = true;
            self.interrupt = 0;
        }
    }
}
