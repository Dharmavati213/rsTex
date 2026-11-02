//! The `rstex` command: pdflatex-compatible flags, `--until-stable`, format auto-build, snapshot
//! driver (DESIGN.md section 12).  The engine runs on one dedicated thread with a 1 GiB stack,
//! since TeX recurses deeply (DESIGN.md section 8).
#![forbid(unsafe_code)]

const ENGINE_STACK: usize = 1 << 30;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-version") {
        println!("rstex {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let engine = std::thread::Builder::new()
        .stack_size(ENGINE_STACK)
        .spawn(move || {
            let tex = rstex_engine::state::Tex::alloc();
            tex.toks_init();
            eprintln!("rstex: the engine is not ported yet (M0)");
            std::process::exit(2);
        })
        .expect("cannot start the engine thread");
    let _ = engine.join();
}
