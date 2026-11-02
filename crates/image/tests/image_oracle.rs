//! The crate against the oracle: `testdata/image_oracle.tsv` holds what pdfTeX 1.40.29 does with the inputs of
//! `common/gen_inputs.rs`, as written by `tests/oracle/gen_image_tables.py`.  The inputs are made here, at run time, and
//! go through `common/sim.rs`, which replays the operations as the engine would.  No oracle is needed to run this.

mod common;

use common::gen_inputs;
use common::sim;
use std::collections::{BTreeMap, HashMap};

fn table() -> Vec<(String, usize, String)> {
    let text = include_str!("../testdata/image_oracle.tsv");
    let mut rows = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') {
            continue;
        }
        let mut f = line.splitn(3, '\t');
        let name = f.next().unwrap().to_string();
        let count: usize = f.next().unwrap().parse().unwrap();
        rows.push((name, count, f.next().unwrap().to_string()));
    }
    rows
}

#[test]
fn every_case_agrees_with_the_oracle() {
    let set = gen_inputs::all();
    let fontmap: HashMap<Vec<u8>, Vec<u8>> =
        set.fontmap.iter().map(|(a, b)| (a.as_bytes().to_vec(), b.as_bytes().to_vec())).collect();
    let mut families: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut names: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for c in &set.cases {
        let p = sim::Params::parse(&c.params);
        let out = sim::run_case(&p, &c.images, fontmap.clone(), |name| {
            set.files.get(name).unwrap_or_else(|| panic!("no file {name}")).clone()
        });
        let fam = common::family(&c.name);
        families.entry(fam.clone()).or_default().push(common::result_line(&out));
        names.entry(fam).or_default().push(c.name.clone());
    }
    let rows = table();
    assert_eq!(rows.len(), families.len(), "families in the table and in the generator");
    let mut bad = Vec::new();
    let mut cases = 0;
    for (name, count, expected) in &rows {
        let lines = families.get(name).unwrap_or_else(|| panic!("family {name} is not generated"));
        assert_eq!(lines.len(), *count, "cases in family {name}");
        cases += count;
        let got = if *count == 1 {
            lines[0].clone()
        } else {
            let mut body = lines.join("\n");
            body.push('\n');
            common::short_hash(body.as_bytes())
        };
        if &got != expected {
            bad.push(format!("{name}: {got} != {expected}; cases {:?}", names[name]));
        }
    }
    assert!(
        bad.is_empty(),
        "{} of {} families differ from the oracle; python3 -I tests/oracle/gen_image_tables.py --only NAME shows which case:\n{}",
        bad.len(),
        rows.len(),
        bad.join("\n")
    );
    assert_eq!(cases, set.cases.len());
    assert!(cases > 1500, "the table covers {cases} cases");
}

#[test]
fn inputs_are_the_same_every_time() {
    let a = gen_inputs::all();
    let b = gen_inputs::all();
    assert_eq!(a.files.len(), b.files.len());
    for (k, v) in &a.files {
        assert_eq!(&b.files[k], v, "{k}");
    }
}

#[test]
fn sha256_is_sha256() {
    let h = common::sha256(b"abc");
    assert_eq!(h[0], 0xba);
    assert_eq!(h[31], 0xad);
    assert_eq!(common::short_hash(b""), "e3b0c44298fc1c14");
}
