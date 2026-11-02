#!/usr/bin/env python3
"""Runs the token machine's `#[ignore]`d unit tests before print.rs and friends are ported.

Run with python3 -I.  Those tests print, and until M1 the routines that print are stubs.  So this copies
crates/ to a scratch directory, replaces the stubs there with literal ports of §43, §47-§48, §57-§67, §263
and §284-§285, a linear id_lookup, and a test double for error (§82 in scroll mode with no help on the
terminal), and runs `cargo test --include-ignored` on that copy.  Nothing in the real tree changes.

    print_overlay.py SCRATCH_DIR [cargo test filters]

With OVERLAY_CHECKED=1 in the environment it builds with --features checked.

Remove this script, and the `#[ignore]`s, once M1 has ported the real routines.
"""
import os
import re
import shutil
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

BODIES = {
    "print.rs": {
        "print_ln": r"""
        match self.selector {
            TERM_AND_LOG => { self.wterm_cr(); self.wlog_cr(); self.term_offset = 0; self.file_offset = 0; }
            LOG_ONLY => { self.wlog_cr(); self.file_offset = 0; }
            TERM_ONLY => { self.wterm_cr(); self.term_offset = 0; }
            NO_PRINT | PSEUDO | NEW_STRING => {}
            _ => self.write_file_ln(self.selector),
        }""",
        "print_char": r"""
        if s == self.new_line_char() && self.selector < PSEUDO {
            self.print_ln();
            return;
        }
        match self.selector {
            TERM_AND_LOG => {
                let c = self.xchr.at(s);
                self.wterm_char(c); self.wlog_char(c);
                self.term_offset += 1; self.file_offset += 1;
                if self.term_offset == self.max_print_line { self.wterm_cr(); self.term_offset = 0; }
                if self.file_offset == self.max_print_line { self.wlog_cr(); self.file_offset = 0; }
            }
            LOG_ONLY => {
                let c = self.xchr.at(s);
                self.wlog_char(c); self.file_offset += 1;
                if self.file_offset == self.max_print_line { self.print_ln(); }
            }
            TERM_ONLY => {
                let c = self.xchr.at(s);
                self.wterm_char(c); self.term_offset += 1;
                if self.term_offset == self.max_print_line { self.print_ln(); }
            }
            NO_PRINT => {}
            PSEUDO => { if self.tally < self.trick_count { let k = self.tally % self.error_line; self.trick_buf[k] = s; } }
            NEW_STRING => { if self.pool_ptr < self.pool_size { self.append_char(s); } }
            _ => { let c = self.xchr.at(s); self.write_file_char(self.selector, c); }
        }
        self.tally += 1;""",
        "print": r"""
        let mut s = s;
        if s >= self.str_ptr {
            s = S_QUESTIONS;
        } else if s < 256 {
            if s < 0 {
                s = S_QUESTIONS;
            } else {
                if self.selector > PSEUDO && !self.message_printing { self.print_char(s); return; }
                if s == self.new_line_char() {
                    if self.selector < PSEUDO { self.print_ln(); return; }
                    else if self.message_printing { self.print_char(s); return; }
                }
                if self.xprn.at(s) != 0 { self.print_char(s); return; }
                let nl = self.new_line_char();
                self.set_new_line_char(-1);
                let mut j = self.str_start[s];
                while j < self.str_start[s + 1] { self.print_char(self.str_pool.at(j)); j += 1; }
                self.set_new_line_char(nl);
                return;
            }
        }
        let mut j = self.str_start[s];
        while j < self.str_start[s + 1] { self.print_char(self.str_pool.at(j)); j += 1; }""",
        "slow_print": r"""
        if s >= self.str_ptr || s < 256 {
            self.print(s);
        } else {
            let mut j = self.str_start[s];
            while j < self.str_start[s + 1] { self.print(self.str_pool.at(j)); j += 1; }
        }""",
        "print_nl": r"""
        if self.selector < NO_PRINT || (self.term_offset > 0 && odd(self.selector)) || (self.file_offset > 0 && self.selector >= LOG_ONLY) {
            self.print_ln();
        }
        self.print(s);""",
        "print_esc": r"""
        let c = self.escape_char();
        if c >= 0 && c < 256 { self.print(c); }
        self.slow_print(s);""",
        "print_the_digs": r"""
        let mut k = k;
        while k > 0 {
            k -= 1;
            if self.dig[k] < 10 { self.print_char(b'0' as i32 + self.dig[k]); }
            else { self.print_char(b'A' as i32 - 10 + self.dig[k]); }
        }""",
        "print_int": r"""
        let mut n: i64 = n.into();
        let mut k = 0;
        if n < 0 {
            self.print_char(b'-' as i32);
            if n > -100000000 { n = -n; }
            else {
                let mut m = -1 - n;
                n = m / 10;
                m = (m % 10) + 1;
                k = 1;
                if m < 10 { self.dig[0] = m as i32; } else { self.dig[0] = 0; n += 1; }
            }
        }
        loop {
            self.dig[k] = (n % 10) as i32;
            n /= 10;
            k += 1;
            if n == 0 { break; }
        }
        self.print_the_digs(k);""",
        "print_hex": r"""
        let mut n = n;
        let mut k = 0;
        self.print_char(b'"' as i32);
        loop {
            self.dig[k] = n % 16;
            n /= 16;
            k += 1;
            if n == 0 { break; }
        }
        self.print_the_digs(k);""",
    },
    "strings.rs": {
        "make_string": r"""
        self.str_ptr += 1;
        self.str_start[self.str_ptr] = self.pool_ptr;
        self.str_ptr - 1""",
        "get_strings_started": r"""
        self.pool_ptr = 0;
        self.str_ptr = 0;
        self.str_start[0] = 0;
        for k in 0..256 {
            if k < 32 || k > 126 {
                self.append_char(b'^' as i32);
                self.append_char(b'^' as i32);
                if k < 0o100 { self.append_char(k + 0o100); }
                else if k < 0o200 { self.append_char(k - 0o100); }
                else {
                    let (a, b) = (k / 16, k % 16);
                    self.append_char(if a < 10 { a + 48 } else { a - 10 + 97 });
                    self.append_char(if b < 10 { b + 48 } else { b - 10 + 97 });
                }
            } else {
                self.append_char(k);
            }
            let _ = self.make_string();
        }
        for s in crate::access::WELL_KNOWN_STRINGS {
            for &c in s.iter() { self.append_char(c as i32); }
            let _ = self.make_string();
        }
        true""",
    },
    "hash.rs": {
        "id_lookup": r"""
        let name: Vec<u8> = self.buffer.range(j, j + l).to_vec();
        let top = EQTB_SIZE + 1 + self.hash_high;
        let mut p = EQTB_SIZE + 1;
        while p < top {
            let s = self.text(p);
            if self.str_pool.range(self.str_start[s], self.str_start[s + 1]) == &name[..] { return p; }
            p += 1;
        }
        if self.no_new_control_sequence { return UNDEFINED_CONTROL_SEQUENCE; }
        for &c in &name { self.append_char(c as i32); }
        let s = self.make_string();
        self.hash_high += 1;
        self.set_text(p, s);
        self.set_eq_type(p, UNDEFINED_CS);
        p""",
        "print_cs": r"""
        if p < HASH_BASE {
            if p >= SINGLE_BASE {
                if p == NULL_CS {
                    self.print_esc_s(b"csname"); self.print_esc_s(b"endcsname"); self.print_char(b' ' as i32);
                } else {
                    self.print_esc(p - SINGLE_BASE);
                    if self.cat_code(p - SINGLE_BASE) == LETTER { self.print_char(b' ' as i32); }
                }
            } else if p < ACTIVE_BASE {
                self.print_esc_s(b"IMPOSSIBLE.");
            } else {
                self.print(p - ACTIVE_BASE);
            }
        } else if (p >= UNDEFINED_CONTROL_SEQUENCE && p <= EQTB_SIZE) || p > self.eqtb_top {
            self.print_esc_s(b"IMPOSSIBLE.");
        } else if self.text(p) >= self.str_ptr {
            self.print_esc_s(b"NONEXISTENT.");
        } else {
            self.print_esc(self.text(p));
            self.print_char(b' ' as i32);
        }""",
        "sprint_cs": r"""
        if p < HASH_BASE {
            if p < SINGLE_BASE { self.print(p - ACTIVE_BASE); }
            else if p < NULL_CS { self.print_esc(p - SINGLE_BASE); }
            else { self.print_esc_s(b"csname"); self.print_esc_s(b"endcsname"); }
        } else {
            self.print_esc(self.text(p));
        }""",
    },
    "eqtb.rs": {
        "begin_diagnostic": r"""
        self.old_setting = self.selector;
        if self.tracing_online() <= 0 && self.selector == TERM_AND_LOG {
            self.selector -= 1;
            if self.history == SPOTLESS { self.history = WARNING_ISSUED; }
        }""",
        "end_diagnostic": r"""
        self.print_nl_s(b"");
        if blank_line { self.print_ln(); }
        self.selector = self.old_setting;""",
    },
    "errors.rs": {
        # A test double: §82 for scroll mode, whose help goes to the transcript only.
        "error": r"""
        if self.history < ERROR_MESSAGE_ISSUED { self.history = ERROR_MESSAGE_ISSUED; }
        self.print_char(b'.' as i32);
        self.show_context();
        self.error_count += 1;
        self.help_ptr = 0;
        self.print_ln();""",
        "fatal_error": r'''panic!("fatal_error: {}", String::from_utf8_lossy(s))''',
        "overflow": r'''panic!("overflow: {} {}", String::from_utf8_lossy(s), n)''',
        "confusion": r'''panic!("confusion: {}", String::from_utf8_lossy(s))''',
    },
}


def patch(path, bodies):
    with open(path, encoding="utf-8") as f:
        s = f.read()
    for name, body in bodies.items():
        pat = re.compile(r'todo!\("§\d+ ' + re.escape(name) + r'"\)')
        s, n = pat.subn(lambda m: body.strip(), s, count=1)
        if n != 1:
            sys.exit("no stub left for %s in %s: the real routine may exist now; drop this overlay" % (name, path))
    with open(path, "w", encoding="utf-8") as f:
        f.write(s)


def main():
    scratch = sys.argv[1]
    filters = sys.argv[2:] or ["toks::", "input::", "getnext::", "expand::", "macrocall::"]
    copy = os.path.join(scratch, "overlay")
    shutil.rmtree(os.path.join(copy, "crates"), ignore_errors=True)
    os.makedirs(copy, exist_ok=True)
    shutil.copytree(os.path.join(ROOT, "crates"), os.path.join(copy, "crates"))
    for f in ("Cargo.toml", "Cargo.lock"):
        shutil.copy(os.path.join(ROOT, f), copy)
    src = os.path.join(copy, "crates", "engine", "src")
    for file, bodies in BODIES.items():
        patch(os.path.join(src, file), bodies)
    env = dict(os.environ, CARGO_TARGET_DIR=os.path.join(ROOT, "target", "pilot", "overlay"))
    features = ["--features", "checked"] if os.environ.get("OVERLAY_CHECKED") else []
    cmd = ["cargo", "test", "-p", "rstex-engine", "-j6", "--offline", "--lib"] + features + ["--"] + filters + ["--include-ignored"]
    sys.exit(subprocess.run(cmd, cwd=copy, env=env).returncode)


if __name__ == "__main__":
    main()
