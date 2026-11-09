//! §414-427 Part 25: macro_call.
//!
//! Token lists are in `toks`, not `mem`: docs/PORTING.md, "macro_call".
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
    pub struct Vars {
        /// `r` where `capture_delimited` has been called, for the stop in its cold path: as an argument there it costs this routine's hot lines their registers.
        pub delimiter: i32,
    }
}

/// Non-POD state private to this module, in `self.host`.  Rarely needed.
#[derive(Default)]
pub struct HVars {}

impl Tex {
    /// §415
    ///
    /// r, s, t, u, v are positions in `toks`: a walk `link(x)` is `x + 1`, and `s=null` is `s == NULL`,
    /// which no position equals.  WEB's p, q and rbrace_ptr are gone: the argument in flight is the
    /// top of the builder, from `mark`.
    ///
    /// The loop over the parameters does only what a capture loop that reads the whole argument needs, and leaves every other case to
    /// `macrocall_param`, out of line: with §418's literal loop inlined here, its positions and copies of tokens were set up and saved
    /// at each parameter, about 25 instructions, before a capture that needs none of them.
    pub fn macro_call(&mut self) {
        #[cfg(feature = "commute-probe")]
        self.probe_call();
        crate::prof_macro_call!(self);
        let save_scanner_status = self.scanner_status;
        let save_warning_index = self.warning_index;
        self.warning_index = self.cur_cs;
        let ref_count = self.cur_chr;
        let mut r = ref_count + 1;
        let mut n = 0;
        'exit: {
            if self.tracing_macros() > 0 {
                // §427
                self.begin_diagnostic();
                if self.tracing_stack_levels() > 0 {
                    if self.input_ptr < self.tracing_stack_levels() {
                        let mut v = self.input_ptr;
                        self.print_ln();
                        self.print_char(b'~' as i32);
                        while v > 0 {
                            self.print_char(b'.' as i32);
                            v -= 1;
                        }
                        self.print_cs(self.warning_index);
                        self.token_show(ref_count);
                    } else {
                        self.print_char(b'~' as i32);
                        self.print_char(b'~' as i32);
                        self.print_cs(self.warning_index);
                    }
                } else {
                    self.print_ln();
                    self.print_cs(self.warning_index);
                    self.token_show(ref_count);
                }
                self.end_diagnostic(false);
            }
            if self.toks[r] == PROTECTED_TOKEN {
                r += 1;
            }
            if self.toks[r] == END_MATCH_TOKEN {
                // A native macro takes no parameters, so it costs the others nothing: crate::native_font.
                if self.cur_cs == self.v_image.pf.cs && self.native_pickup() {
                    break 'exit;
                }
            } else {
                // §417
                self.scanner_status = MATCHING;
                self.long_state = self.eq_type(self.cur_cs);
                if self.long_state >= OUTER_CALL {
                    // Where an armed macro comes too: crate::checkpoint.
                    self.long_state &= !crate::native::ARMED_BITS;
                    if self.long_state >= OUTER_CALL {
                        self.long_state -= 2;
                    }
                }
                loop {
                    // link(temp_head):=null
                    let mark = self.tb_mark();
                    self.tb_match_mark = mark;
                    // What the capture below gave, for `macrocall_param`: none tried is as one that read nothing.
                    let mut tried = (0, 0);
                    // §418's first pass, `r=s`, where it is a capture.  Most arguments are read whole here.
                    #[cfg(not(feature = "slow_capture"))]
                    {
                        let t = self.toks[r];
                        if ((t - MATCH_TOKEN) as u32) <= 255 && self.state() == TOKEN_LIST {
                            // §417's s, and r:=s, which a delimited capture moves past the delimiter when it is done.
                            let mut q = r + 1;
                            let mut m = 0;
                            let c = self.macrocall_capture::<true>(&mut q, n, mark, &mut m);
                            if c == crate::capture::CAPTURE_DONE {
                                // found: and §426 for an argument a loop has put in pstack[n]
                                r = q;
                                n += 1;
                                if self.tracing_macros() > 0 {
                                    self.macrocall_show_arg(t - MATCH_TOKEN, n);
                                }
                                if self.toks[r] == END_MATCH_TOKEN {
                                    self.capture_leave();
                                    break;
                                }
                                continue;
                            }
                            tried = (c, m);
                        }
                    }
                    let x = self.macrocall_param(r, n, mark, tried);
                    if x < 0 {
                        break 'exit;
                    }
                    r = (x >> 32) as i32;
                    n = x as i32;
                    // now toks[r] is a token whose command code is either match or end_match
                    if self.toks[r] == END_MATCH_TOKEN {
                        break;
                    }
                }
            }
            // §416
            while self.at_list_end() && self.token_type() != V_TEMPLATE && self.token_type() != OUTPUT_TEXT {
                self.end_token_list();
            }
            self.begin_macro_list(ref_count);
            self.set_name(self.warning_index);
            self.set_loc(r + 1);
            if n > 0 {
                if self.param_ptr + n > self.max_param_stack {
                    self.max_param_stack = self.param_ptr + n;
                    if self.max_param_stack > self.param_size {
                        self.overflow(b"parameter stack size", self.param_size);
                    }
                }
                for m in 0..n {
                    self.param_stack[self.param_ptr + m] = self.pstack[m];
                }
                self.param_ptr += n;
            }
        }
        // exit:
        self.scanner_status = save_scanner_status;
        self.warning_index = save_warning_index;
    }

    /// §418-§426 for the parameter at `r`, where the loop of `macro_call` has not read all of its argument: the literal loop, with the capture
    /// loops where they apply.  `tried` is what the capture of that loop gave, `(c, m)`, from which the first pass goes on, as nothing has been read since;
    /// `(0, 0)` if it tried none, as this loop's own test would not have tried one either.  Out of line, so that its state stays out of `macro_call`'s
    /// registers.  Gives `r` for the next parameter in the high half and `n` in the low one, or -1 where §422 or §424 has ended the call.  As a pair of
    /// `i32`s the same took 0.6% more instructions on the format build, in `macro_call`'s code around the call.
    #[inline(never)]
    #[cfg_attr(feature = "slow_capture", allow(unused_variables))]
    fn macrocall_param(&mut self, mut r: i32, mut n: i32, mark: i32, tried: (i32, i32)) -> i64 {
        let mut unbalance = 0;
        let s;
        let mut m = 0;
        let mut match_chr = 0;
        if self.toks[r] > MATCH_TOKEN + 255 || self.toks[r] < MATCH_TOKEN {
            s = NULL;
        } else {
            match_chr = self.toks[r] - MATCH_TOKEN;
            s = r + 1;
            r = s;
        }
        // Whether capture.rs has put the whole argument in pstack[n].
        let mut captured = false;
        // What the first pass has instead of a capture of its own.
        #[cfg(not(feature = "slow_capture"))]
        let mut first = Some(tried);
        // §418.  `continue:` is the top of this loop, `found:` is right after it.
        'found: loop {
            #[cfg(not(feature = "slow_capture"))]
            {
                let c = match first.take() {
                    Some((c, m0)) => {
                        m = m0;
                        c
                    }
                    None if r == s && self.state() == TOKEN_LIST => self.macrocall_capture::<false>(&mut r, n, mark, &mut m),
                    None => 0,
                };
                if c == crate::capture::CAPTURE_DONE {
                    captured = true;
                    break 'found;
                }
                if c > 0 {
                    unbalance = c;
                    if !self.macrocall_group(mark, n, &mut unbalance) {
                        return -1;
                    }
                    m += 1;
                    if self.toks[r] > END_MATCH_TOKEN {
                        continue 'found;
                    }
                    if self.toks[r] < MATCH_TOKEN {
                        continue 'found;
                    }
                    break 'found;
                }
            }
            self.get_token();
            if self.cur_tok == self.toks[r] {
                // §420
                r += 1;
                if self.toks[r] >= MATCH_TOKEN && self.toks[r] <= END_MATCH_TOKEN {
                    if self.cur_tok < LEFT_BRACE_LIMIT {
                        self.align_state -= 1;
                    }
                    break 'found;
                }
                continue 'found;
            }
            // §423
            if s != r {
                if s == NULL {
                    // §424
                    self.print_err_s(b"Use of ");
                    self.sprint_cs(self.warning_index);
                    self.print_s(b" doesn't match its definition");
                    self.help(&[
                        b"If you say, e.g., `\\def\\a1{...}', then you must always",
                        b"put `1' after `\\a', since control sequence names are",
                        b"made up of letters only. The macro here has not been",
                        b"followed by the required stuff, so I'm ignoring it.",
                    ]);
                    self.error();
                    return -1;
                }
                let mut t = s;
                loop {
                    self.tb_push(self.toks[t]);
                    m += 1;
                    let mut u = t + 1;
                    let mut v = s;
                    loop {
                        if u == r {
                            if self.cur_tok != self.toks[v] {
                                break; // goto done
                            }
                            r = v + 1;
                            continue 'found;
                        }
                        if self.toks[u] != self.toks[v] {
                            break; // goto done
                        }
                        u += 1;
                        v += 1;
                    }
                    // done:
                    t += 1;
                    if t == r {
                        break;
                    }
                }
                r = s;
            }
            if self.cur_tok == self.par_token && self.long_state != LONG_CALL {
                self.macrocall_runaway_abort(mark, n, unbalance);
                return -1;
            }
            if self.cur_tok < RIGHT_BRACE_LIMIT {
                if self.cur_tok < LEFT_BRACE_LIMIT {
                    // §425
                    unbalance = 1;
                    self.tb_push(self.cur_tok);
                    if !self.macrocall_group(mark, n, &mut unbalance) {
                        return -1;
                    }
                } else {
                    // §421
                    self.back_input();
                    self.print_err_s(b"Argument of ");
                    self.sprint_cs(self.warning_index);
                    self.print_s(b" has an extra }");
                    self.help(&[
                        b"I've run across a `}' that doesn't seem to match anything.",
                        b"For example, `\\def\\a#1{...}' and `\\a}' would produce",
                        b"this error. If you simply proceed now, the `\\par' that",
                        b"I've just inserted will cause me to report a runaway",
                        b"argument that might be the root of the problem. But if",
                        b"your `}' was spurious, just type `2' and it will go away.",
                    ]);
                    self.align_state += 1;
                    self.long_state = CALL;
                    self.cur_tok = self.par_token;
                    self.ins_error();
                    continue 'found;
                }
            } else {
                // §419
                if self.cur_tok == SPACE_TOKEN
                    && self.toks[r] <= END_MATCH_TOKEN
                    && self.toks[r] >= MATCH_TOKEN
                {
                    continue 'found;
                }
                self.tb_push(self.cur_tok);
            }
            m += 1;
            if self.toks[r] > END_MATCH_TOKEN {
                continue 'found;
            }
            if self.toks[r] < MATCH_TOKEN {
                continue 'found;
            }
            break 'found;
        }
        // found:
        if s != NULL {
            // §426.  m=1 means the list is one token or one group, so a last token
            // below right_brace_limit is the group's closing brace.
            if !captured {
                self.pstack[n] = if m == 1 && self.tb[self.tb_ptr - 1] < RIGHT_BRACE_LIMIT {
                    self.tb_commit_strip(mark)
                } else {
                    self.tb_commit(mark)
                };
            }
            n += 1;
            if self.tracing_macros() > 0 {
                self.macrocall_show_arg(match_chr, n);
            }
        }
        // now toks[r] is a token whose command code is either match or end_match
        if self.toks[r] == END_MATCH_TOKEN && captured {
            self.capture_leave();
        }
        ((r as i64) << 32) | (n as u32 as i64)
    }

    /// §418's first pass, `r=s`, as a capture: of an undelimited parameter if the parameter text goes on at `s` with a match or the end_match, else
    /// of a delimited one, which moves `s` past its delimiter when it is done.  `macro_call`'s loop has it with `HERE`, the undelimited capture in
    /// line, and `macrocall_param` without: one choice for both, so that the `tried` that the first hands on is the capture the second would make.
    #[cfg(not(feature = "slow_capture"))]
    #[inline(always)]
    fn macrocall_capture<const HERE: bool>(&mut self, s: &mut i32, n: i32, mark: i32, m: &mut i32) -> i32 {
        if self.toks[*s] <= END_MATCH_TOKEN && self.toks[*s] >= MATCH_TOKEN {
            if HERE { self.capture_undelimited_here(n) } else { self.capture_undelimited(n) }
        } else {
            self.v_macrocall.delimiter = *s;
            self.capture_delimited(s, n, mark, m)
        }
    }

    /// §426's line of `\tracingmacros` for the argument `n`, with its test of `\tracingstacklevels`.
    #[cold]
    #[inline(never)]
    fn macrocall_show_arg(&mut self, match_chr: i32, n: i32) {
        if self.tracing_stack_levels() == 0 || self.input_ptr < self.tracing_stack_levels() {
            self.begin_diagnostic();
            self.print_nl(match_chr);
            self.print_int(n);
            self.print_s(b"<-");
            self.show_token_list(self.pstack[n - 1] + 1, NULL, 1000);
            self.end_diagnostic(false);
        }
    }

    /// §425's loop, with its left brace on the builder and its `unbalance` above 0.  False if §422 ended it, for the caller's `return`.
    #[inline]
    fn macrocall_group(&mut self, mark: i32, n: i32, unbalance: &mut i32) -> bool {
        loop {
            #[cfg(not(feature = "slow_capture"))]
            if self.state() == TOKEN_LIST {
                let par = if self.long_state == LONG_CALL { 0 } else { self.par_token };
                self.capture_group(unbalance, par);
                if *unbalance == 1 {
                    if let Some(t) = self.capture_right_brace() {
                        *unbalance = 0;
                        self.tb_push(t);
                        return true;
                    }
                }
            }
            self.get_token();
            if self.cur_tok == self.par_token && self.long_state != LONG_CALL {
                self.macrocall_runaway_abort(mark, n, *unbalance);
                return false;
            }
            if self.cur_tok < RIGHT_BRACE_LIMIT {
                if self.cur_tok < LEFT_BRACE_LIMIT {
                    *unbalance += 1;
                } else {
                    *unbalance -= 1;
                    if *unbalance == 0 {
                        break; // goto done1
                    }
                }
            }
            self.tb_push(self.cur_tok);
        }
        // done1: the right brace
        self.tb_push(self.cur_tok);
        true
    }

    /// §422 Report a runaway argument and abort; the caller does the `return`.  `pstack[n]`, the list
    /// in flight, has no block yet: it is the builder from `mark`, hence `0..n` where WEB has `0..n`
    /// inclusive.
    fn macrocall_runaway_abort(&mut self, mark: i32, n: i32, unbalance: i32) {
        if self.long_state == CALL {
            self.runaway();
            self.print_err_s(b"Paragraph ended before ");
            self.sprint_cs(self.warning_index);
            self.print_s(b" was complete");
            self.help(&[
                b"I suspect you've forgotten a `}', causing me to apply this",
                b"control sequence to too much text. How can we recover?",
                b"My plan is to forget the whole thing and hope for the best.",
            ]);
            self.back_error();
        }
        self.tb_discard(mark);
        self.align_state -= unbalance;
        for m in 0..n {
            self.arg_release(self.pstack[m]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toks::testkit::*;

    fn table(data: &'static [u8]) -> Vec<Vec<Vec<u8>>> {
        data.split(|&c| c == b'\n')
            .filter(|l| !l.is_empty())
            .map(|l| l.split(|&c| c == b'\t').map(|f| f.iter().map(|&c| if c == 0x0B { b'\n' } else { c }).collect()).collect())
            .collect()
    }

    /// Reads a parameter text from a line, as `scan_toks` §500-§502 would: `#1` is a match token.
    fn parameter_text(t: &mut Tex, text: &[u8]) -> Vec<i32> {
        set_line(t, text, SKIP_BLANKS);
        let toks = drain(t, 0);
        t.first = 1;
        let mut out = Vec::new();
        let mut i = 0;
        while i < toks.len() {
            if toks[i] == MAC_PARAM * 256 + 35 {
                out.push(MATCH_TOKEN + 35);
                i += 2;
            } else {
                out.push(toks[i]);
                i += 1;
            }
        }
        out
    }

    /// Calls `\m`, and gives its arguments as `\tracingmacros` prints them and what is left of the
    /// input as `\detokenize` prints it.  Everything allocated is given back.
    fn call(t: &mut Tex, params: &[i32], cmd: i32, body: &[i32]) -> (Vec<Vec<u8>>, Vec<u8>) {
        let m = cs(b'm');
        let h = def_macro(t, m, cmd, params, body);
        t.cur_cs = m;
        t.cur_cmd = cmd;
        t.cur_chr = h;
        let (ss, wi, pp) = (t.scanner_status, t.warning_index, t.param_ptr);
        t.macro_call();
        assert_eq!((t.scanner_status, t.warning_index), (ss, wi), "restored");
        assert_eq!(t.tb_ptr, 0, "nothing left on the builder");
        assert_eq!(t.token_type(), MACRO);
        assert_eq!(t.name(), m);
        assert_eq!(t.param_start(), pp);
        let args = (pp..t.param_ptr)
            .map(|k| {
                let l = list(t, t.param_stack[k]);
                show(t, &l)
            })
            .collect();
        let rest = drain(t, 0);
        assert_eq!(t.param_ptr, pp, "the macro level ended, and its arguments went with it");
        let rest = show(t, &rest);
        t.delete_token_ref(h);
        (args, rest)
    }

    fn eq(got: &[u8], want: &[u8], what: &str, r: &[Vec<u8>]) {
        assert_eq!(
            String::from_utf8_lossy(got),
            String::from_utf8_lossy(want),
            "{} of \\def\\m {:?}, input {:?}",
            what,
            String::from_utf8_lossy(&r[0]),
            String::from_utf8_lossy(&r[1])
        );
    }

    fn check(got: (Vec<Vec<u8>>, Vec<u8>), r: &[Vec<u8>]) {
        let n: usize = String::from_utf8_lossy(&r[2]).parse().unwrap();
        assert_eq!(got.0.len(), n, "number of arguments, for {:?}", String::from_utf8_lossy(&r[1]));
        for k in 0..n {
            eq(&got.0[k], &r[3 + k], "an argument", r);
        }
        eq(&got.1, &r[3 + n], "the rest", r);
    }

    /// The whole table, with the input in a file level and in a token list, three times over in one
    /// machine: a block that leaked would make the heap grow on the later rounds.
    #[test]
    fn arguments_against_the_oracle() {
        let rows = table(include_bytes!("../testdata/macrocall_oracle.tsv"));
        assert!(rows.len() > 70);
        let t = machine();
        t.set_eq_type(cs(b'x'), RELAX);
        let mut top_after_first_round = 0;
        for round in 0..3 {
            for r in &rows {
                let params = parameter_text(t, &r[0]);
                // From the file.
                set_line(t, &r[1], SKIP_BLANKS);
                check(call(t, &params, CALL, &[]), r);
                assert_eq!(t.align_state, 1000000, "align_state drifted, for {:?}", String::from_utf8_lossy(&r[1]));
                t.first = 1;
                // The same tokens from a list.
                set_line(t, &r[1], SKIP_BLANKS);
                let toks = drain(t, 0);
                t.first = 1;
                feed(t, &toks);
                let a = call(t, &params, CALL, &[]);
                check(a, r);
                assert_eq!(t.align_state, 1000000, "align_state drifted, for {:?}", String::from_utf8_lossy(&r[1]));
                assert_eq!(t.input_ptr, 0);
            }
            if round == 0 {
                top_after_first_round = t.toks_top;
            }
        }
        assert_eq!(t.toks_top, top_after_first_round, "a token list leaked");
        Tex::dealloc(t);
    }

    #[test]
    fn no_parameters_feeds_the_body() {
        let t = machine();
        let h = def_macro(t, cs(b'm'), CALL, &[], &[LETTER_TOKEN + 97]);
        feed(t, &[LETTER_TOKEN + 98]);
        t.cur_cs = cs(b'm');
        t.cur_chr = h;
        t.macro_call();
        assert_eq!(t.scanner_status, NORMAL, "no scanning happened");
        assert_eq!(t.token_ref_count(h), 1, "the level holds a reference");
        assert_eq!(t.loc(), h + 2, "the first token after the end_match");
        assert_eq!(drain(t, 0), vec![LETTER_TOKEN + 97, LETTER_TOKEN + 98]);
        assert_eq!(t.token_ref_count(h), 0);
        Tex::dealloc(t);
    }

    #[test]
    fn protected_token_is_skipped() {
        let t = machine();
        let h = def_macro(t, cs(b'm'), CALL, &[PROTECTED_TOKEN, MATCH_TOKEN + 35], &[OUT_PARAM_TOKEN + 1]);
        // A protected macro's list starts with the token before its parameter text... §1396.
        assert_eq!(t.toks[h + 1], PROTECTED_TOKEN);
        feed(t, &[LETTER_TOKEN + 97]);
        t.cur_cs = cs(b'm');
        t.cur_chr = h;
        t.macro_call();
        assert_eq!(drain(t, 0), vec![LETTER_TOKEN + 97]);
        Tex::dealloc(t);
    }

    #[test]
    fn a_tail_call_does_not_grow_the_stack() {
        let t = machine();
        let h = def_macro(t, cs(b'm'), CALL, &[], &[cs_tok(b'm')]);
        t.cur_cs = cs(b'm');
        t.cur_chr = h;
        t.macro_call();
        let level = t.input_ptr;
        for _ in 0..100 {
            t.get_next();
            assert_eq!(t.cur_cmd, CALL);
            t.macro_call();
            assert_eq!(t.input_ptr, level, "§416 ends the exhausted level first");
        }
        Tex::dealloc(t);
    }

    #[test]
    fn par_in_a_long_macro_is_an_argument() {
        let t = machine();
        let par = t.par_token;
        let params = [MATCH_TOKEN + 35, LETTER_TOKEN + 46];
        feed(t, &[LETTER_TOKEN + 97, par, LEFT_BRACE_TOKEN + 123, par, RIGHT_BRACE_TOKEN + 125, LETTER_TOKEN + 46]);
        let (args, rest) = call(t, &params, LONG_CALL, &[]);
        assert_eq!(String::from_utf8_lossy(&args[0]), "a\\par {\\par }");
        assert!(rest.is_empty());
        assert_eq!(t.align_state, 1000000);
        // Long outer works the same: the outer-ness is taken off by long_state-2.
        feed(t, &[par, LETTER_TOKEN + 46]);
        let (args, _) = call(t, &params, LONG_OUTER_CALL, &[]);
        assert_eq!(String::from_utf8_lossy(&args[0]), "\\par ");
        Tex::dealloc(t);
    }

    #[test]
    fn par_as_a_delimiter_is_allowed() {
        // The first \par is a partial match of `\par!`; §423 lets it in, even without \long.
        let t = machine();
        let par = t.par_token;
        let params = [MATCH_TOKEN + 35, par, OTHER_TOKEN + 33];
        feed(t, &[LETTER_TOKEN + 97, par, LETTER_TOKEN + 98, par, OTHER_TOKEN + 33]);
        let (args, _) = call(t, &params, CALL, &[]);
        assert_eq!(String::from_utf8_lossy(&args[0]), "a\\par b");
        Tex::dealloc(t);
    }

    #[test]
    fn the_left_brace_delimiter_of_a_hash_brace_macro() {
        // \def\m#1#{...}: `{` ends the argument, is not consumed for align_state's purposes, and the
        // body starts with `{`.  §420.
        let t = machine();
        let lb = LEFT_BRACE_TOKEN + 123;
        let params = [MATCH_TOKEN + 35, lb];
        feed(t, &[LETTER_TOKEN + 97, lb, LETTER_TOKEN + 98, RIGHT_BRACE_TOKEN + 125]);
        let h = def_macro(t, cs(b'm'), CALL, &params, &[lb]);
        t.cur_cs = cs(b'm');
        t.cur_chr = h;
        t.macro_call();
        assert_eq!(t.align_state, 1000000, "read one {{, then one correction");
        let a = list(t, t.param_stack[0]);
        assert_eq!(a, vec![LETTER_TOKEN + 97]);
        assert_eq!(drain(t, 0), vec![lb, LETTER_TOKEN + 98, RIGHT_BRACE_TOKEN + 125]);
        assert_eq!(t.align_state, 1000000);
        Tex::dealloc(t);
    }

    #[test]
    fn nine_arguments_on_top_of_an_earlier_macro() {
        let t = machine();
        let params: Vec<i32> = (0..9).map(|_| MATCH_TOKEN + 35).collect();
        let body: Vec<i32> = (1..=9).rev().map(|k| OUT_PARAM_TOKEN + k).collect();
        let m = cs(b'm');
        let h = def_macro(t, m, CALL, &params, &body);
        let input: Vec<i32> = (b'a'..=b'i').map(|c| LETTER_TOKEN + c as i32).collect();
        // An outer macro's level below, with parameters on the stack.
        let o = t.tok_one(LETTER_TOKEN + 122);
        t.param_stack[0] = o;
        t.param_ptr = 1;
        let hb = def_macro(t, cs(b'b'), CALL, &[], &[OUT_PARAM_TOKEN + 1, LETTER_TOKEN + 33]);
        t.begin_token_list(hb, MACRO);
        t.set_param_start(0);
        t.set_loc(hb + 2 + 1);
        feed(t, &input);
        t.cur_cs = m;
        t.cur_chr = h;
        t.macro_call();
        assert_eq!(t.param_ptr, 10);
        assert_eq!(t.max_param_stack, 10);
        let got = drain(t, LETTER_TOKEN + 33);
        let want: Vec<i32> = (b'a'..=b'i').rev().map(|c| LETTER_TOKEN + c as i32).collect();
        assert_eq!(got, want);
        assert_eq!(t.param_ptr, 1);
        Tex::dealloc(t);
    }

    // -------------------------------------------------------------------------------------------
    // The error paths print, so they need print.rs, strings.rs, hash.rs and errors.rs.  What each
    // expects is what the oracle's log shows for the same line, at line 7 of its file.
    // -------------------------------------------------------------------------------------------

    /// With `\par` under its real name, a file level at line 7, and `\m` defined.
    fn error_machine(params: &[i32], body: &[i32]) -> (&'static mut Tex, i32) {
        let t = machine();
        print_setup(t);
        let p = define(t, b"par", PAR_END, 256);
        t.par_loc = p;
        t.par_token = CS_TOKEN_FLAG + p;
        t.line = 7;
        t.in_open = 1;
        t.set_index(1);
        t.set_name(20);
        t.set_end_line_char(13);
        let h = def_macro(t, cs(b'm'), CALL, params, body);
        (t, h)
    }

    /// Reads `\m` from the line and calls it.
    fn read_and_call(t: &mut Tex, line: &[u8]) {
        set_line(t, line, NEW_LINE);
        t.get_next();
        assert_eq!(t.cur_cmd, CALL);
        t.macro_call();
    }

    #[test]
    fn runaway_argument_at_par() {
        let (t, _) = error_machine(&[MATCH_TOKEN + 35, OTHER_TOKEN + 46], &[]);
        read_and_call(t, b"\\m ab\\par c\r");
        let s = printed(t);
        let want = "Runaway argument?\nab\n! Paragraph ended before \\m was complete.\n<to be read again> \n                   \\par \nl.7 \\m ab\\par\n              c\n";
        assert!(s.starts_with(want), "got {:?}", s);
        assert_eq!(t.tb_ptr, 0, "the argument in flight was abandoned");
        assert_eq!(t.input_ptr, 1, "no macro level, and \\par is backed up");
        assert_eq!(t.toks[t.loc()], t.par_token);
        assert_eq!((t.scanner_status, t.warning_index), (NORMAL, 0));
        Tex::dealloc(t);
    }

    #[test]
    fn runaway_in_a_group_gives_back_align_state() {
        let (t, _) = error_machine(&[MATCH_TOKEN + 35], &[]);
        read_and_call(t, b"\\m{{a\\par c\r");
        let s = printed(t);
        let want = "Runaway argument?\n{{a\n! Paragraph ended before \\m was complete.\n<to be read again> \n                   \\par \nl.7 \\m{{a\\par\n              c\n";
        assert!(s.starts_with(want), "got {:?}", s);
        assert_eq!(t.align_state, 1000000, "the two left braces read do not count any more");
        Tex::dealloc(t);
    }

    #[test]
    fn extra_right_brace() {
        let (t, _) = error_machine(&[MATCH_TOKEN + 35], &[]);
        read_and_call(t, b"\\m}x\r");
        let s = printed(t);
        // The \par that the first error inserts is seen by the same call, as a runaway.  (The oracle's
        // log has the help message, then an empty line, where the terminal has only the newline.)
        let want = "! Argument of \\m has an extra }.\n<inserted text> \n                \\par \n<to be read again> \n                   }\nl.7 \\m}\n       x\nRunaway argument?\n! Paragraph ended before \\m was complete.\n<to be read again> \n                   \\par \n<to be read again> \n                   }\nl.7 \\m}\n       x\n";
        assert!(s.starts_with(want), "got {:?}", s);
        Tex::dealloc(t);
    }

    #[test]
    fn use_does_not_match() {
        let (t, _) = error_machine(&[LETTER_TOKEN + 120], &[LETTER_TOKEN + 122]);
        read_and_call(t, b"\\m y\r");
        let s = printed(t);
        assert!(s.starts_with("! Use of \\m doesn't match its definition.\nl.7 \\m y\n        \n"), "got {:?}", s);
        assert_eq!(t.input_ptr, 0, "no macro level");
        assert_eq!(t.tb_ptr, 0);
        Tex::dealloc(t);
    }

    #[test]
    fn forbidden_control_sequence_in_an_argument() {
        let (t, _) = error_machine(&[MATCH_TOKEN + 35, OTHER_TOKEN + 46], &[]);
        t.set_eq_type(cs(b'O'), OUTER_CALL);
        read_and_call(t, b"\\m ab\\O\r");
        let s = printed(t);
        let want = "Runaway argument?\nab\n! Forbidden control sequence found while scanning use of \\m.\n<inserted text> \n                \\par \n<to be read again> \n                   \\O \nl.7 \\m ab\\O\n           \n";
        assert!(s.starts_with(want), "got {:?}", s);
        assert_eq!(t.tb_ptr, 0);
        Tex::dealloc(t);
    }

    #[test]
    fn tracing_macros_shows_the_call_and_its_arguments() {
        let (t, h) = error_machine(&[MATCH_TOKEN + 35, OTHER_TOKEN + 46], &[LETTER_TOKEN + 122]);
        t.set_tracing_macros(1);
        read_and_call(t, b"\\m ab.\r");
        let s = printed(t);
        assert_eq!(s, "\n\\m #1.->z\n#1<-ab\n");
        t.delete_token_ref(h);
        Tex::dealloc(t);
    }

    /// `\tracingstacklevels`, tracingstacklevels.ch; the oracle's log for `\m ab.` at the top of a file.
    #[test]
    fn tracing_stack_levels_shows_the_depth() {
        for (levels, want) in [(3, "\n~.\\m #1.->z\n#1<-ab\n"), (1, "~~\\m \n")] {
            let (t, _) = error_machine(&[MATCH_TOKEN + 35, OTHER_TOKEN + 46], &[LETTER_TOKEN + 122]);
            t.set_tracing_macros(1);
            t.set_tracing_stack_levels(levels);
            t.begin_file_reading();
            t.set_name(20);
            assert_eq!(t.input_ptr, 1);
            read_and_call(t, b"\\m ab.\r");
            assert_eq!(printed(t), want);
            Tex::dealloc(t);
        }
    }

    /// `\b`'s body `\m#1!`, its `#1` the one token `y`: capture.rs gives `\m` that token with no level of §381, which the oracle's `input_ptr` still
    /// counts when §426 tests it against `\tracingstacklevels`.  At 2 the oracle shows the call of `\m` and not its argument, as its `input_ptr` is 2 there;
    /// without that level it would be 1.  So under that tracing the literal loop reads the argument.  tests/integration/capture/param_one_stacklevels.tex
    /// has the same against the oracle's log.
    #[test]
    fn a_bare_parameter_handed_on_under_tracing_stack_levels() {
        for (levels, want) in [(0, "\n\\m #1->\n#1<-y\n"), (1, "~~\\m \n"), (2, "\n~.\\m #1->\n"), (3, "\n~.\\m #1->\n#1<-y\n")] {
            let (t, _) = error_machine(&[MATCH_TOKEN + 35], &[]);
            t.set_tracing_macros(1);
            t.set_tracing_stack_levels(levels);
            // The level of `\b`, its #1 the list `y`, with a high-water mark that §381's push would not raise.
            let y = t.tok_one(LETTER_TOKEN + 121);
            t.param_stack[0] = y;
            t.param_ptr = 1;
            let hb = def_macro(t, cs(b'b'), CALL, &[], &[cs_tok(b'm'), OUT_PARAM_TOKEN + 1, OTHER_TOKEN + 33]);
            t.begin_token_list(hb, MACRO);
            t.set_param_start(0);
            t.set_loc(hb + 2);
            t.max_in_stack = 10;
            t.get_next();
            assert_eq!(t.cur_cmd, CALL);
            t.macro_call();
            assert_eq!(printed(t), want, "at \\tracingstacklevels={}", levels);
            assert_eq!((t.input_ptr, t.param_ptr, t.max_in_stack), (2, 2, 10), "§416 has ended any level of #1");
            assert_eq!(list(t, t.param_stack[1]), vec![LETTER_TOKEN + 121]);
            assert_eq!(t.toks[t.param_stack[1]] & crate::toks::ARG_CLEAN, 0, "not marked, as tb_commit leaves a block");
            assert_eq!(drain(t, OTHER_TOKEN + 33), vec![]);
            assert_eq!(t.loc(), hb + 5, "past the ! of \\b's body");
            Tex::dealloc(t);
        }
    }

    /// The same call of `\m` with a bare `#1`, where the literal loop's `tb_push` or `tb_commit` calls `webmem.rs`: with the class-1 free list empty and
    /// `toks_watch` at the top of the heap, `web_bump` has `web_schedule` count the token on the builder, and with the builder at `tb_limit`, `web_room`
    /// does.  A block that capture.rs took for it would have `webmem.rs` ask without the token, and set the limits, and any peak it notes, one cell lower,
    /// so there it leaves the argument to the literal loop.  Each case runs as it comes and again under `\tracingmacros` and `\tracingstacklevels`, where the literal loop reads
    /// the argument in any case, and the heap, the limits and the peak have to come out the same.  With a free block and no limit near, nothing calls.
    #[test]
    fn a_bare_parameter_leaves_main_memory_as_the_literal_loop_does() {
        #[derive(Clone, Copy, Debug, PartialEq)]
        enum At {
            NewBlock,
            BuilderLimit,
            FreeBlock,
        }
        fn run(at: At, literal: bool) -> Vec<i32> {
            let (t, _) = error_machine(&[MATCH_TOKEN + 35], &[]);
            if literal {
                t.set_tracing_macros(1);
                t.set_tracing_stack_levels(100);
            }
            let y = t.tok_one(LETTER_TOKEN + 121);
            t.param_stack[0] = y;
            t.param_ptr = 1;
            let hb = def_macro(t, cs(b'b'), CALL, &[], &[cs_tok(b'm'), OUT_PARAM_TOKEN + 1, OTHER_TOKEN + 33]);
            if at != At::NewBlock {
                let f = t.tok_one(LETTER_TOKEN + 122);
                t.flush_list(f);
            }
            assert_eq!(t.tok_free[1] == 0, at == At::NewBlock);
            t.begin_token_list(hb, MACRO);
            t.set_param_start(0);
            t.set_loc(hb + 2);
            t.max_in_stack = 10;
            t.get_next();
            assert_eq!(t.cur_cmd, CALL);
            // A memory whose coarse room is some thousands of cells, so that `web_schedule` sets limits that the one token moves.
            t.main_memory = (1 << 16) + 4000;
            t.v_webmem.toks_watch = t.toks_top;
            if at == At::BuilderLimit {
                t.tb_limit = t.tb_ptr;
            }
            t.macro_call();
            assert_eq!(list(t, t.param_stack[1]), vec![LETTER_TOKEN + 121]);
            let w = &t.v_webmem;
            let state = vec![t.toks_top, t.tok_free[1], t.param_stack[1], t.tb_ptr, t.tb_limit, t.tx_limit, w.toks_watch, w.lo_watch, w.peak, w.fine, w.committed];
            // The machine's first `web_schedule`, with no main memory, set `tb_limit` to the builder's capacity.
            assert_eq!(t.tb_limit < TB_CAP as i32, at != At::FreeBlock, "{:?}: whether webmem.rs set the limits again", at);
            Tex::dealloc(t);
            state
        }
        for at in [At::NewBlock, At::BuilderLimit, At::FreeBlock] {
            assert_eq!(run(at, false), run(at, true), "{:?}", at);
        }
    }
}
