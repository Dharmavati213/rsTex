//! Reading `texmf.cnf`: cnf.c.
use crate::names::*;
use crate::{Kpse, format, sys};

impl Kpse {
    /// `kpathsea_cnf_get`.  Reads the cnf files on the first call.
    pub(crate) fn cnf_get(&mut self, name: &[u8]) -> Option<Vec<u8>> {
        // When we expand the default of TEXMFCNF, we need assorted variables and get called again.
        if self.doing_cnf_init {
            return None;
        }
        if !self.cnf_read {
            self.cnf_read = true;
            self.doing_cnf_init = true;
            self.read_all_cnf();
            self.doing_cnf_init = false;
            // C calls kpathsea_init_db here.  Reading ls-R waits for the first search that needs it,
            // which changes nothing but when the file is read.
        }
        let mut key = fold_key(name);
        let base_len = key.len();
        key.push(b'.');
        key.extend_from_slice(&fold_key(&self.program_name));
        if let Some(v) = self.cnf.get(&key) {
            return Some(v.clone());
        }
        key.truncate(base_len);
        self.cnf.get(&key).cloned()
    }

    /// `read_all_cnf`.
    fn read_all_cnf(&mut self) {
        #[cfg(feature = "prof")]
        let _p = crate::prof::begin(&crate::prof::CNF);
        let cnf_path = self.init_format(format::KPSE_CNF_FORMAT);
        let files = match &cnf_path {
            Some(st) => self.all_path_search_str(&st.path.clone(), b"texmf.cnf"),
            None => Vec::new(),
        };
        if files.is_empty() {
            // A system with no cnf file at all, when the user has not pointed TEXMFCNF somewhere:
            // kpathsea would have no search path at all, not even `.`; see `Tree::builtin_cnf`.
            let user_chose = self.getenv(b"TEXMFCNF").is_some_and(|v| !v.is_empty());
            match self.tree.builtin_cnf.clone().filter(|_| !user_chose) {
                Some(text) => self.parse_cnf_text(b"(builtin)", &text),
                None => {
                    let warn = self.getenv(b"KPATHSEA_WARNING");
                    if warn.as_deref() != Some(b"0") {
                        let path = cnf_path.map(|s| s.path.clone()).unwrap_or_default();
                        self.warn(&[b"kpathsea: configuration file texmf.cnf not found in these directories: ", &path[..]].concat());
                    }
                }
            }
            return;
        }
        for file in &files {
            if let Ok(text) = std::fs::read(sys::path_of(file)) {
                self.recorded.push(file.clone());
                self.parse_cnf_text(file, &text);
            }
        }
    }

    /// The per-line part of `read_all_cnf`.
    pub(crate) fn parse_cnf_text(&mut self, file: &[u8], text: &[u8]) {
        let lines = read_lines(text);
        let mut lineno = 0;
        let mut it = lines.into_iter();
        while let Some(mut line) = it.next() {
            lineno += 1;
            // Strip trailing spaces.
            while line.last().is_some_and(|&c| is_space(c)) {
                line.pop();
            }
            // Concatenate consecutive lines that end with \.
            while line.last() == Some(&b'\\') {
                line.pop();
                lineno += 1;
                match it.next() {
                    Some(next) => line.extend_from_slice(&next),
                    None => {
                        let msg = [file, b":", lineno.to_string().as_bytes(), b": (kpathsea) Last line of file ends with \\"].concat();
                        self.warn(&msg);
                    }
                }
            }
            if let Err(msg) = self.do_line(&line, false) {
                let w = [file, b":", lineno.to_string().as_bytes(), b": (kpathsea) ", msg.as_bytes(), b" on line: ", &line[..]].concat();
                self.warn(&w);
            }
        }
    }

    /// `do_line`: `<variable>[.<program>] [=] <value>`.  A blank line or a comment is skipped.
    pub(crate) fn do_line(&mut self, line_in: &[u8], env_progname: bool) -> Result<(), String> {
        // Skip leading whitespace.
        let mut l = 0;
        while is_space(at(line_in, l)) {
            l += 1;
        }
        let c0 = at(line_in, l);
        if c0 == 0 || c0 == b'%' || c0 == b'#' {
            return Ok(());
        }
        // Remove a trailing comment: a % or # preceded by whitespace, with that whitespace.  In C the
        // `continue` of this loop is inside a comment, so the extra step back below is real.
        let mut line = line_in[l..].to_vec();
        let mut value = line.len() as isize - 1;
        while value > 0 {
            let c = line[value as usize];
            if c == b'%' || c == b'#' {
                value -= 1;
                while value >= 0 && is_space(line[value as usize]) {
                    line[value as usize] = 0;
                    value -= 1;
                }
            }
            value -= 1;
        }
        if let Some(nul) = line.iter().position(|&c| c == 0) {
            line.truncate(nul);
        }
        let line = line;

        // The variable name is everything up to the next space or = or `.'.
        let mut p = 0;
        while at(&line, p) != 0 && !is_space(at(&line, p)) && at(&line, p) != b'=' && at(&line, p) != b'.' {
            p += 1;
        }
        if p == 0 {
            return Err("No cnf variable name".into());
        }
        let var = &line[..p];

        // If the variable is qualified with a program name, extract it.
        while is_space(at(&line, p)) {
            p += 1;
        }
        let mut prog: Option<&[u8]> = None;
        if at(&line, p) == b'.' {
            p += 1;
            while is_space(at(&line, p)) {
                p += 1;
            }
            let start = p;
            while at(&line, p) != 0 && !is_space(at(&line, p)) && at(&line, p) != b'=' {
                p += 1;
            }
            let q = &line[start..p];
            if q.is_empty() {
                return Err("Empty program name qualifier".into());
            }
            for &c in q {
                if c == b'$' || c == b'{' || c == b'}' || is_kpse_sep(c) {
                    return Err(format!("Unlikely character {} in program name", c as char));
                }
            }
            prog = Some(q);
        }

        // Skip whitespace, an optional =, more whitespace.
        while is_space(at(&line, p)) {
            p += 1;
        }
        if at(&line, p) == b'=' {
            p += 1;
            while is_space(at(&line, p)) {
                p += 1;
            }
        }

        // The value is whatever remains.  Remove trailing whitespace.
        let mut end = line.len();
        while end > p && is_space(line[end - 1]) {
            end -= 1;
        }
        if end <= p {
            return Err("No cnf value".into());
        }
        let mut value = line[p..end].to_vec();

        // A single texmf.cnf can serve Windows and Unix: `;` works as a separator on both.
        if is_env_sep(b':') {
            for c in value.iter_mut() {
                if *c == b';' {
                    *c = b':';
                }
            }
        }

        if env_progname {
            // kpathsea_cnf_line_env_progname: a setting in the environment, and for this program.
            let this_prog = prog.map(<[u8]>::to_vec).unwrap_or_else(|| self.program_name.clone());
            self.set_var(var, &value);
            self.set_var(&[var, b"_", &this_prog].concat(), &value);
        } else {
            // Earlier entries override later ones, in the same or another file.
            let lhs = match prog {
                Some(q) => [var, b".", q].concat(),
                None => var.to_vec(),
            };
            self.cnf.entry(fold_key(&lhs)).or_insert(value);
        }
        Ok(())
    }
}
