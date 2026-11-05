//! Which of the verified sets of macros a run is held against: `native_table.rs`, and DESIGN.md section 10, "Versions".
//!
//! A profile is what one tree gave, as a whole, and has passed `tests/cd/native/new_profile.py` as a whole.  So a run takes one, the nearest by `PICK`, and from there on nothing else counts: every hash a diagram asks about is that profile's, and a
//! macro that another profile has otherwise is as much not as installed as a document's own.  The choice only says which proof to try.  It stands in the image, so a restored run does not make it again and a checkpoint is judged by the files it read,
//! as any other: another tree is another run.
//!
//! `native_tree_report` is for `rstex --check-tree`, which reads what a run of its own document left here: nothing of it reaches a log or a terminal, and a run without `RSTEX_TREE_REPORT` pays for looking that up once, at its end.
#![forbid(unsafe_code)]

use super::{ARMED_BITS, native_table};
use crate::consts::*;
use crate::state::*;

impl Tex {
    /// Takes the profile that most of `PICK` agrees with, the newest of equals.  A name that is not there says nothing: with no PGF loaded only LaTeX's own count.
    pub(crate) fn native_profile_pick(&mut self) {
        if self.v_image.cd.profile_picked {
            return;
        }
        self.v_image.cd.profile_picked = true;
        let mut score = [0u32; native_table::PROFILES.len()];
        for (name, hashes) in native_table::PICK.iter() {
            let cs = self.find_cs(name);
            if cs == UNDEFINED_CONTROL_SEQUENCE {
                continue;
            }
            let Some(h) = self.native_meaning_hash(cs) else { continue };
            for (s, &want) in score.iter_mut().zip(hashes.iter()) {
                *s += (want == h) as u32;
            }
        }
        let mut best = 0;
        for (k, &s) in score.iter().enumerate() {
            if s >= score[best] {
                best = k;
            }
        }
        self.v_image.cd.profile = best as i32;
    }

    /// The profile this run is held against.
    #[inline]
    pub(crate) fn nt(&self) -> &'static native_table::Profile {
        &native_table::PROFILES[self.v_image.cd.profile as usize]
    }

    #[inline]
    pub(crate) fn nt_known(&self, h: u64) -> bool {
        native_table::is_known(self.v_image.cd.profile, h)
    }

    #[inline]
    pub(crate) fn nt_safe(&self, h: u64) -> bool {
        native_table::is_safe(self.v_image.cd.profile, h)
    }

    #[inline]
    pub(crate) fn nt_stock_key(&self, h: u64) -> bool {
        native_table::is_stock_key(self.v_image.cd.profile, h)
    }

    /// What real PGF is about to draw a diagram for, and which macro it was, if it was one of the table's.
    pub(crate) fn native_decline_text(&self, reason: &str) -> String {
        let hv = &self.host.hv.native;
        let name = hv.detail.map(|k| self.nt().guard[k].0).or(hv.detail_any.map(|k| self.nt().guard_any[k]));
        match name {
            Some(n) => format!("{reason}: {}", String::from_utf8_lossy(n)),
            None => reason.to_string(),
        }
    }

    /// What `commute::pdf` has to write for this PGF.
    #[inline]
    pub(crate) fn nt_bp_rule(&self) -> commute::BpRule {
        self.nt().bp
    }

    /// `RSTEX_TREE_REPORT=FILE`: what stands between this tree and each table, for `rstex --check-tree`.  Lines of fields with tabs in between.
    #[cold]
    pub fn native_tree_report(&mut self) {
        let Some(file) = crate::platform::env_var(b"RSTEX_TREE_REPORT") else { return };
        let mut o = String::new();
        if self.v_image.cd.tikzcd_cs != 0 {
            self.native_profile_pick();
            let p = self.nt();
            o += &format!("profile\t{}\t{}\t{}\n", p.id, p.pgf, p.latex);
            let mut differ = Vec::new();
            for &(name, h) in p.guard.iter() {
                let cs = self.find_cs(name);
                if cs == UNDEFINED_CONTROL_SEQUENCE || self.native_meaning_hash(cs) != Some(h) {
                    differ.push(String::from_utf8_lossy(name).into_owned());
                }
            }
            o += &format!("guard\t{}\t{}\t{}\n", differ.len(), p.guard.len(), differ.iter().take(8).cloned().collect::<Vec<_>>().join("\t"));
            let mut any = Vec::new();
            for &name in p.guard_any.iter() {
                let cs = self.find_cs(name);
                if cs != UNDEFINED_CONTROL_SEQUENCE && (CALL..END_TEMPLATE).contains(&(self.eq_type(cs) & !ARMED_BITS)) && !self.nt_known(self.native_macro_hash(cs, self.equiv(cs))) {
                    any.push(String::from_utf8_lossy(name).into_owned());
                }
            }
            o += &format!("any\t{}\t{}\t{}\n", any.len(), p.guard_any.len(), any.iter().take(8).cloned().collect::<Vec<_>>().join("\t"));
            o += &format!("ran\t{}\n", self.v_image.cd.ran as i32);
        }
        o += &self.pf_tree_report();
        let _ = std::fs::write(crate::platform::path_from_bytes(&file), o);
    }
}

#[cfg(test)]
mod tests {
    use super::native_table::{PICK, PROFILES};

    /// A profile that nothing tells from another could never be taken.
    #[test]
    fn the_profiles_can_be_told_apart() {
        for a in 0..PROFILES.len() {
            for b in 0..a {
                assert!(PICK.iter().any(|(_, h)| h[a] != h[b]), "{} and {}", PROFILES[a].id, PROFILES[b].id);
            }
        }
        for p in PROFILES.iter() {
            assert!(p.guard.iter().any(|e| e.0 == b"pgfversion"), "{}", p.id);
            assert_eq!(p.latex_id.len(), 2, "{}", p.id);
            assert!(p.guard.windows(2).all(|w| w[0].0 < w[1].0), "{}: the generator keeps a list sorted, and a name once", p.id);
        }
    }
}
