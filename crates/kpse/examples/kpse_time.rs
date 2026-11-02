//! Where the time of a run's lookups goes.  Each figure is the fastest of many fresh instances, which is
//! about as near as a loaded machine lets one get to what the code costs.  For a figure to mean anything,
//! build with `--release`, and have a TeX tree on the machine.
//!
//! ```text
//! cargo run -p rstex-kpse --release --example kpse_time [ROUNDS]
//! ```
use rstex_kpse::*;
use std::time::Instant;

/// Roughly what plain pdftex looks up for a one-line document, with two misses.
const LOOKUPS: [(&[u8], i32); 10] = [
    (b"pdftex", KPSE_FMT_FORMAT),
    (b"kpse-time-absent-a", KPSE_TEX_FORMAT),
    (b"cmr10", KPSE_TFM_FORMAT),
    (b"cmmi10", KPSE_TFM_FORMAT),
    (b"cmsy10", KPSE_TFM_FORMAT),
    (b"pdftex", KPSE_FONTMAP_FORMAT),
    (b"cmr10", KPSE_TYPE1_FORMAT),
    (b"8r", KPSE_ENC_FORMAT),
    (b"kpse-time-absent-b", KPSE_TEX_FORMAT),
    (b"article.cls", KPSE_TEX_FORMAT),
];

fn main() {
    let rounds: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(200);
    // [new, cnf, format path, first lookup that reads ls-R, the rest]
    let mut best = [f64::MAX; 5];
    let mut scans = 0;
    let mut found = 0;
    for _ in 0..rounds {
        let ms = |t: Instant| t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        let mut k = Kpse::new(Config::default());
        let a = ms(t);
        let t = Instant::now();
        let _ = k.var_value(b"openout_any");
        let b = ms(t);
        let t = Instant::now();
        let _ = k.format_path(KPSE_FMT_FORMAT);
        let c = ms(t);
        let t = Instant::now();
        // In a database and on no disk element before it, so this one loads ls-R.
        let first = k.find_file(LOOKUPS[0].0, LOOKUPS[0].1, false);
        let d = ms(t);
        let t = Instant::now();
        found = usize::from(first.is_some());
        for (name, format) in &LOOKUPS[1..] {
            found += usize::from(k.find_file(name, *format, false).is_some());
        }
        let e = ms(t);
        for (slot, v) in best.iter_mut().zip([a, b, c, d, e]) {
            *slot = slot.min(v);
        }
        scans = k.dir_scans();
    }
    println!("fastest of {rounds} fresh instances, in ms:");
    println!("  find the tree (the cached answer)       {:8.3}", best[0]);
    println!("  read texmf.cnf                          {:8.3}", best[1]);
    println!("  one format's path                       {:8.3}", best[2]);
    println!("  first lookup, which reads ls-R          {:8.3}", best[3]);
    println!("  the other {} lookups                     {:8.3}", LOOKUPS.len() - 1, best[4]);
    println!("  all of it                               {:8.3}", best.iter().sum::<f64>());
    println!("found {found} of {}; directories opened {scans}", LOOKUPS.len());
}
