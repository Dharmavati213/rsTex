//! A dump of everything TeX can see, under the feature `commute-probe`, to establish by experiment what a real diagram leaves behind and
//! what a cell sees: `tests/cd/native/probe.py` compares the dumps (DESIGN section 10).  `\message{rstexprobe NAME}` writes `$RSTEX_PROBE_DIR/NAME.txt`:
//! one line per entry of eqtb and per sparse register, with a macro's or token register's tokens written out and a box's three dimensions.
//!
//! `tests/cd/native/gen_table.py` makes `native_table.rs` from what this reports.  `\message{rstexprobe-begin}` notes every macro's hash and from then on which macros are called and
//! which names of keys are looked up; `\message{rstexprobe-end FILE}` writes what was seen, with the hash then and now; `\message{rstexprobe-safe FILE}` writes every macro and what is in its own body.
#![forbid(unsafe_code)]

use crate::consts::*;
use crate::state::*;
use std::collections::{BTreeSet, HashMap};
use std::fmt::Write;

#[derive(Default)]
pub struct ProbeState {
    on: bool,
    /// Every macro's hash at the mark.
    begin: HashMap<i32, u64>,
    called: BTreeSet<i32>,
    keys: BTreeSet<Vec<u8>>,
}

impl Tex {
    /// `macro_call` is about to run `cur_cs`.
    #[inline]
    pub fn probe_call(&mut self) {
        if self.host.hv.native.probe.on {
            let cs = self.cur_cs;
            self.host.hv.native.probe.called.insert(cs);
        }
    }

    /// `id_lookup` was asked about a name.
    #[inline]
    pub fn probe_lookup(&mut self, name: &[u8]) {
        if self.host.hv.native.probe.on && name.starts_with(b"pgfk@") {
            self.host.hv.native.probe.keys.insert(name.to_vec());
        }
    }

    fn probe_macro_hash(&self, cs: i32) -> Option<u64> {
        let t = self.eq_type(cs) & !crate::native::ARMED_BITS;
        if t >= CALL && t < END_TEMPLATE {
            Some(self.native_macro_hash(cs, self.equiv(cs)))
        } else {
            None
        }
    }

    pub fn probe_message(&mut self, text: &str) {
        if text == "rstexprobe-begin" {
            let mut begin = HashMap::new();
            for cs in HASH_BASE..=self.eqtb_top {
                if (cs >= PRIM_EQTB_BASE && cs < UNDEFINED_CONTROL_SEQUENCE) || (cs > UNDEFINED_CONTROL_SEQUENCE && cs <= EQTB_SIZE) {
                    continue;
                }
                if let Some(h) = self.probe_macro_hash(cs) {
                    begin.insert(cs, h);
                }
            }
            let p = &mut self.host.hv.native.probe;
            p.begin = begin;
            p.on = true;
        } else if let Some(file) = text.strip_prefix("rstexprobe-end ") {
            let mut o = String::new();
            let called: Vec<i32> = self.host.hv.native.probe.called.iter().copied().collect();
            for cs in called {
                let Some(&h0) = self.host.hv.native.probe.begin.get(&cs) else { continue };
                let h1 = self.probe_macro_hash(cs).unwrap_or(0);
                let _ = writeln!(o, "M\t{h0:016x}\t{h1:016x}\t{}", self.probe_name(cs));
            }
            let keys: Vec<Vec<u8>> = self.host.hv.native.probe.keys.iter().cloned().collect();
            for k in keys {
                let cs = self.find_cs(&k);
                let name = String::from_utf8_lossy(&k).into_owned();
                let exists = cs != UNDEFINED_CONTROL_SEQUENCE && !matches!(self.eq_type(cs) & !crate::native::ARMED_BITS, UNDEFINED_CS | RELAX);
                let h0 = if cs != UNDEFINED_CONTROL_SEQUENCE { self.host.hv.native.probe.begin.get(&cs).copied().unwrap_or(0) } else { 0 };
                let h1 = if exists { self.probe_macro_hash(cs).unwrap_or(1) } else { 0 };
                let _ = writeln!(o, "K\t{h0:016x}\t{h1:016x}\t{name}");
            }
            let _ = std::fs::write(file, o);
            self.host.hv.native.probe.on = false;
        } else if let Some(file) = text.strip_prefix("rstexprobe-safe ") {
            let mut o = String::new();
            for cs in HASH_BASE..=self.eqtb_top {
                if (cs >= PRIM_EQTB_BASE && cs < UNDEFINED_CONTROL_SEQUENCE) || (cs > UNDEFINED_CONTROL_SEQUENCE && cs <= EQTB_SIZE) {
                    continue;
                }
                if let Some(h) = self.probe_macro_hash(cs) {
                    // The name, whether the body by itself changes how text is read or how things are placed, and what it mentions.
                    let mut severe = 0;
                    let mut mentions = String::new();
                    let mut p = self.equiv(cs) + 1;
                    while self.toks[p] != 0 {
                        let t = self.toks[p];
                        p += 1;
                        if t >= CS_TOKEN_FLAG {
                            let d = t - CS_TOKEN_FLAG;
                            let w = self.eqtb[d];
                            let ty = w.b0() & !crate::native::ARMED_BITS;
                            if self.native_severe(d, ty, w.rh()) {
                                severe = 1;
                            }
                            if ty >= CALL && ty < END_TEMPLATE {
                                let _ = write!(mentions, "{} ", self.probe_name(d));
                            }
                        }
                    }
                    let _ = writeln!(o, "{h:016x}\t{severe}\t{}\t{mentions}", self.probe_name(cs));
                }
            }
            let _ = std::fs::write(file, o);
        } else if let Some(name) = text.strip_prefix("rstexprobe ") {
            self.probe_dump(name);
        }
    }

    fn probe_name(&self, cs: i32) -> String {
        if cs < SINGLE_BASE {
            return format!("active:{}", cs - ACTIVE_BASE);
        }
        if cs < NULL_CS {
            return format!("single:{}", cs - SINGLE_BASE);
        }
        let s = self.text(cs);
        if s <= 0 || s >= self.str_ptr {
            return format!("cs{cs}");
        }
        String::from_utf8_lossy(self.str_pool.range(self.str_start[s], self.str_start[s + 1])).into_owned()
    }

    fn probe_tokens(&self, mut p: i32, out: &mut String) {
        while self.toks[p] != 0 {
            let t = self.toks[p];
            if t >= CS_TOKEN_FLAG {
                let _ = write!(out, "\\{} ", self.probe_name(t - CS_TOKEN_FLAG));
            } else {
                let _ = write!(out, "{}/{} ", t >> 8, t & 255);
            }
            p += 1;
        }
    }

    pub fn probe_dump(&mut self, name: &str) {
        let Some(dir) = std::env::var_os("RSTEX_PROBE_DIR") else { return };
        let mut o = String::new();
        let _ = writeln!(o, "@mode\t{}", self.mode());
        let _ = writeln!(o, "@str_ptr\t{}", self.str_ptr);
        let _ = writeln!(o, "@align_state\t{}", self.align_state);
        let _ = writeln!(o, "@font_ptr\t{}", self.font_ptr);
        let _ = writeln!(o, "@cur_level\t{}", self.cur_level);
        for cs in 1..=self.eqtb_top {
            if (cs >= PRIM_EQTB_BASE && cs < FONT_ID_BASE) || (cs >= FONT_ID_BASE && cs < UNDEFINED_CONTROL_SEQUENCE) || (cs >= GLUE_BASE && cs <= EQTB_SIZE) {
                continue;
            }
            let t = self.eq_type(cs) & !crate::checkpoint::CK_ARMED;
            if t == UNDEFINED_CS {
                continue;
            }
            let mut v = format!("{t} ");
            if t >= CALL && t < END_TEMPLATE {
                self.probe_tokens(self.equiv(cs) + 1, &mut v);
            } else {
                let _ = write!(v, "{}", self.equiv(cs));
            }
            let _ = writeln!(o, "{}\t{}", self.probe_name(cs), v);
        }
        for p in GLUE_BASE..LOCAL_BASE {
            let g = self.equiv(p);
            let _ = writeln!(o, "glue:{}\t{} {} {} {} {}", p - GLUE_BASE, self.width(g), self.stretch(g), self.stretch_order(g), self.shrink(g), self.shrink_order(g));
        }
        for p in LOCAL_BASE..BOX_BASE {
            let e = self.equiv(p);
            let mut v = String::new();
            if p == PAR_SHAPE_LOC || p >= ETEX_PEN_BASE {
                let _ = write!(v, "{e}");
            } else if e != NULL {
                self.probe_tokens(e + 1, &mut v);
            }
            let _ = writeln!(o, "local:{}\t{}", p - LOCAL_BASE, v);
        }
        for p in BOX_BASE..CUR_FONT_LOC {
            let e = self.equiv(p);
            if e != NULL {
                let _ = writeln!(o, "box:{}\t{} {} {} {}", p - BOX_BASE, self.ty(e), self.width(e), self.height(e), self.depth(e));
            }
        }
        for p in CUR_FONT_LOC..INT_BASE {
            let _ = writeln!(o, "code:{}\t{}", p, self.equiv(p));
        }
        for p in INT_BASE..DIMEN_BASE {
            let _ = writeln!(o, "int:{}\t{}", p - INT_BASE, self.eqtb[p].int());
        }
        for p in DIMEN_BASE..=EQTB_SIZE {
            let _ = writeln!(o, "dim:{}\t{}", p - DIMEN_BASE, self.eqtb[p].int());
        }
        for n in 256..32768 {
            self.find_sa_element(INT_VAL, n, false);
            if self.cur_ptr != NULL {
                let _ = writeln!(o, "count:{}\t{}", n, self.sa_int(self.cur_ptr));
            }
            self.find_sa_element(DIMEN_VAL, n, false);
            if self.cur_ptr != NULL {
                let _ = writeln!(o, "dimen:{}\t{}", n, self.sa_dim(self.cur_ptr));
            }
            self.find_sa_element(GLUE_VAL, n, false);
            if self.cur_ptr != NULL {
                let g = self.sa_ptr(self.cur_ptr);
                let _ = writeln!(o, "skip:{}\t{} {} {}", n, self.width(g), self.stretch(g), self.shrink(g));
            }
            self.find_sa_element(BOX_VAL, n, false);
            if self.cur_ptr != NULL {
                let e = self.sa_ptr(self.cur_ptr);
                if e != NULL {
                    let _ = writeln!(o, "sabox:{}\t{} {} {} {}", n, self.ty(e), self.width(e), self.height(e), self.depth(e));
                }
            }
            self.find_sa_element(TOK_VAL, n, false);
            if self.cur_ptr != NULL {
                let e = self.sa_ptr(self.cur_ptr);
                let mut v = String::new();
                if e != NULL {
                    self.probe_tokens(e + 1, &mut v);
                }
                let _ = writeln!(o, "satoks:{}\t{}", n, v);
            }
        }
        let path = std::path::Path::new(&dir).join(format!("{name}.txt"));
        let _ = std::fs::write(path, o);
    }
}
