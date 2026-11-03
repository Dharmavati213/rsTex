//! Invariants of the foundation that later agents depend on.

use rstex_engine::consts::*;
use rstex_engine::raw::*;
use rstex_engine::state::*;

#[test]
fn print_size_of_tex() {
    let size = core::mem::size_of::<Tex>();
    let pod = core::mem::offset_of!(Tex, host);
    println!("size_of::<Tex>() = {} bytes = {} MiB; POD part = {} bytes = {} MiB", size, size >> 20, pod, pod >> 20);
    assert!(pod < 1 << 31, "total POD size must stay under 2 GiB");
}

/// The arena gives nothing back, so what moves leaves a hole.  The last vector grows where it is.
#[test]
fn the_arena_vector_on_top_grows_in_place() {
    let t = Tex::alloc();
    let start = t.arena_top;
    let mut v: PodVec<[i32; 5]> = PodVec::EMPTY;
    t.arena_push(&mut v, [1; 5]);
    let first = v.start;
    for i in 1..100_000 {
        t.arena_push(&mut v, [i; 5]);
    }
    assert_eq!(v.start, first, "nothing was above it, so it never moved");
    // 16 elements of 20 bytes from the empty vector, then doubling in place: nothing was left behind.
    assert_eq!(first, start);
    assert_eq!(t.arena_top as usize, start as usize + (v.cap as usize * 20).div_ceil(8));
    assert!(v.cap >= 100_000 && v.cap < 200_000);
    // Another vector on top of it: the first moves past that one, and keeps its elements.
    let mut w: PodVec<u8> = PodVec::EMPTY;
    t.arena_push(&mut w, 7);
    while v.len < v.cap {
        let x = [v.len; 5];
        t.arena_push(&mut v, x);
    }
    let cap = v.cap;
    t.arena_push(&mut v, [-1; 5]);
    assert_ne!(v.start, first, "it moved");
    assert_eq!(v.cap, 2 * cap);
    assert_eq!(v.get(&mut t.arena, 0), [1; 5]);
    assert_eq!(v.get(&mut t.arena, 99_999), [99_999; 5]);
    assert_eq!(v.get(&mut t.arena, cap), [-1; 5]);
    assert_eq!(w.get(&mut t.arena, 0), 7);
    Tex::dealloc(t);
}

#[test]
fn hot_scalars_are_in_the_first_cache_line() {
    assert_eq!(core::mem::offset_of!(Tex, cur_cmd), 0);
    assert_eq!(core::mem::offset_of!(Tex, cur_chr), 4);
    assert_eq!(core::mem::offset_of!(Tex, cur_cs), 8);
    assert_eq!(core::mem::offset_of!(Tex, cur_tok), 12);
    assert_eq!(core::mem::offset_of!(Tex, cur_input), 16);
    assert!(core::mem::offset_of!(Tex, input_ptr) < 64);
    assert!(core::mem::offset_of!(Tex, tb_ptr) < 64);
    assert!(core::mem::offset_of!(Tex, eqtb) < 1 << 24, "eqtb comes right after the small state");
}

#[test]
fn design_constants() {
    assert_eq!(NULL, 0);
    assert_eq!(MEM_BOT, 1);
    assert_eq!(MIN_HALFWORD, 0);
    assert_eq!(MAX_QUARTERWORD, 0xFFFF);
    assert_eq!(WEB_MAX_QUARTERWORD, 255);
    assert_eq!(MIN_QUARTERWORD, 0);
    assert_eq!(HASH_SIZE, 0);
    assert_eq!(CS_TOKEN_FLAG, 0o7777, "as pdfTeX");
    assert_eq!(MEM_TOP as usize, MEM_CAP - 1);
    assert_eq!(MEM_MAX, MEM_TOP);
    assert!(MEM_SPLIT > MEM_BOT && MEM_SPLIT < HI_MEM_STAT_MIN);
    // Tokens fit in i32 and a control sequence never looks like a character.
    assert!(CS_TOKEN_FLAG as i64 + EQTB_CAP as i64 <= i32::MAX as i64);
    assert!(15 * 256 + 255 <= CS_TOKEN_FLAG, "cmd*256+chr is below the flag");
    assert!((EQTB_SIZE as usize) < EQTB_CAP);
    assert!((MAX_FONT_MAX as usize) < FONT_CAP);
    // The classic regions, in order.
    assert!(ACTIVE_BASE < SINGLE_BASE && SINGLE_BASE < NULL_CS && NULL_CS < HASH_BASE);
    assert!(HASH_BASE <= FROZEN_CONTROL_SEQUENCE && FROZEN_CONTROL_SEQUENCE < FROZEN_NULL_FONT);
    assert!(UNDEFINED_CONTROL_SEQUENCE < GLUE_BASE && GLUE_BASE < LOCAL_BASE && LOCAL_BASE < INT_BASE);
    assert!(INT_BASE < DIMEN_BASE && DIMEN_BASE <= EQTB_SIZE);
    // The fixed token lists live at low indices of toks, not in mem.
    // §14 "we will want null_list>255": §532 tells frozen_endv from a character only by cur_chr>255.
    assert!(NULL_LIST > 255);
    assert_eq!(OMIT_TEMPLATE, NULL_LIST + 2);
    // "no zoom" must not be a number \pdfdest can legitimately have: §1565.
    assert!(PDF_ZOOM_NULL < 0);
    // The command codes the token machine relies on.
    assert_eq!(LEFT_BRACE_TOKEN, 1 * 256);
    assert_eq!(RIGHT_BRACE_TOKEN, 2 * 256);
    assert_eq!(MATCH_TOKEN, 13 * 256);
    assert_eq!(END_MATCH_TOKEN, 14 * 256);
    assert_eq!(PROTECTED_TOKEN, END_MATCH_TOKEN + 1);
    assert_eq!(END_TEMPLATE_TOKEN, CS_TOKEN_FLAG + FROZEN_END_TEMPLATE);
    assert_eq!(END_WRITE_TOKEN, CS_TOKEN_FLAG + END_WRITE);
    assert_eq!(TOKEN_LIST, 0);
    assert!(MID_LINE > 0);
    assert_eq!(HMODE, VMODE + MAX_COMMAND + 1);
    assert_eq!(MMODE, HMODE + MAX_COMMAND + 1);
}

#[test]
fn help_lines_go_in_reading_order() {
    let t = Tex::alloc();
    t.help(&[b"first line", b"second", b"third and last"]);
    assert_eq!(t.help_ptr, 3);
    // error prints help_line[help_ptr-1] first.
    assert_eq!(t.help_line(2), b"first line");
    assert_eq!(t.help_line(1), b"second");
    assert_eq!(t.help_line(0), b"third and last");
    t.set_help_line(0, b"changed");
    assert_eq!(t.host.help_line[0], b"changed");
    t.help(&[]);
    assert_eq!(t.help_ptr, 0);
    Tex::dealloc(t);
}

#[test]
fn image_starts_valid_and_accessors_work() {
    let t = Tex::alloc();
    assert_eq!(t.cur_cmd, 0);
    assert!(!t.font_used[0]);
    // §405 can back up cs_token_flag+0; reread, its cur_tok must not be the list terminator.
    assert_eq!(RELAX * 256 + t.equiv(0), 1);
    assert_eq!(t.eq_type(0), RELAX);
    // A node field through its accessors.
    t.set_link(100, 200);
    t.set_info(100, -5);
    t.set_ty(100, GLUE_NODE);
    assert_eq!(t.link(100), 200);
    assert_eq!(t.mem[100].rh(), 200);
    // type and subtype share the word with link, and lh with info; set after info on purpose.
    assert_eq!(t.ty(100), GLUE_NODE);
    t.set_subtype(100, 7);
    assert_eq!(t.subtype(100), 7);
    t.set_width(300, 65536);
    assert_eq!(t.width(300), 65536);
    assert_eq!(t.mem[300 + WIDTH_OFFSET].sc(), 65536);
    t.set_glue_set(300, 0.25);
    assert_eq!(t.glue_set(300), 0.25);
    // eqtb, through a word macro: this must store into eqtb, not into a copy.
    t.set_equiv(CAT_CODE_BASE + 65, 11);
    assert_eq!(t.cat_code(65), 11);
    t.set_eq_type(500, CALL);
    t.set_eq_level(500, LEVEL_ONE);
    t.set_equiv(500, 77);
    assert_eq!((t.eq_type(500), t.eq_level(500), t.equiv(500)), (CALL, LEVEL_ONE, 77));
    t.set_int_par(TOLERANCE_CODE, 200);
    assert_eq!(t.tolerance(), 200);
    t.set_tolerance(300);
    assert_eq!(t.eqtb[INT_BASE + TOLERANCE_CODE].int(), 300);
    t.set_dimen_par(HSIZE_CODE, 1000);
    assert_eq!(t.hsize(), 1000);
    // The semantic nest: aux must store into cur_list, not into a copy of it.
    t.set_prev_depth(-1000);
    assert_eq!(t.cur_list.aux_field.sc(), -1000);
    t.set_space_factor(999);
    assert_eq!(t.cur_list.aux_field.lh(), 999);
    assert_eq!(t.space_factor(), 999);
    t.set_clang(3);
    assert_eq!(t.clang(), 3);
    t.set_tail(42);
    assert_eq!(t.cur_list.tail_field, 42);
    // The input state.
    t.set_loc(9);
    assert_eq!(t.cur_input.loc_field, 9);
    t.set_token_type(BACKED_UP);
    assert_eq!(t.index(), BACKED_UP);
    // Font metrics through two-argument accessors.
    t.char_base[1] = 10;
    t.font_info[10 + 65].set_qqqq(FourQuarters { b0: 1, b1: 0x35, b2: 0x09, b3: 7 });
    let q = t.char_info(1, 65);
    assert!(t.char_exists(q));
    assert_eq!(t.height_depth(q), 0x35);
    assert_eq!(t.char_tag(q), 1);
    t.width_base[1] = 100;
    t.font_info[101].set_sc(5000);
    assert_eq!(t.char_width(1, q), 5000);
    Tex::dealloc(t);
}

/// DESIGN.md sections 6, 12: side effects go through World, OS-specific code lives in platform/, and
/// only raw.rs and platform/ may use unsafe.
#[test]
fn modules_keep_their_discipline() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let mut files = 0;
    for e in std::fs::read_dir(dir).unwrap() {
        let path = e.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        if !name.ends_with(".rs") || name == "lib.rs" {
            continue;
        }
        files += 1;
        let text = std::fs::read_to_string(&path).unwrap();
        let code: String = text.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        if name != "raw.rs" {
            assert!(text.contains("#![forbid(unsafe_code)]"), "{} must forbid unsafe code", name);
        }
        for bad in ["cfg(unix)", "cfg(windows)", "cfg(target_os", "libc::", "windows_sys::", "std::os::"] {
            assert!(!code.contains(bad), "{} uses {}: that belongs in platform/", name, bad);
        }
        // prof.rs is compiled only under the prof feature, and what it writes is the profile, not an effect of the document.
        if name != "host.rs" && name != "raw.rs" && name != "prof.rs" {
            for bad in ["std::fs", "std::env", "std::process", "std::io", "std::time", "std::net", "println!", "eprintln!", "print!("] {
                assert!(!code.contains(bad), "{} uses {}: everything goes through World", name, bad);
            }
        }
    }
    assert!(files > 40, "the module files were not found");
}

/// Not a limit, a baseline: the cost of first touching every section of the image.  With huge pages
/// each touched 2 MiB region costs a zeroed huge page, which matters for the hello.tex target (M5).
#[test]
fn first_touch_cost_is_reported() {
    let start = std::time::Instant::now();
    let t = Tex::alloc();
    let alloc = start.elapsed();
    t.toks_init();
    let touched = POD_SECTIONS.len();
    t.eqtb[1].set_rh(1);
    t.toks[100] = 1;
    t.hash[1] = 1;
    t.hash_tab[1] = 1;
    t.mem[1].set_rh(1);
    t.mem[MEM_TOP].set_rh(1);
    t.tb[1] = 1;
    t.str_pool[1] = 1;
    t.str_start[1] = 1;
    t.buffer[1] = 1;
    t.nest[1].mode_field = 1;
    t.save_stack[1].set_rh(1);
    t.input_stack[1].loc_field = 1;
    t.param_stack[1] = 1;
    t.font_info[1].set_rh(1);
    let all = start.elapsed();
    println!("alloc {:?}, {} sections, touching the 15 hottest arrays {:?}", alloc, touched, all - alloc);
    Tex::dealloc(t);
}
