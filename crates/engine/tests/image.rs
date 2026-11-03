//! The format file through its real path: `image_write_fmt`, then `w_open_in` and `load_fmt_file`.  What comes before
//! `image_write_fmt` in `store_fmt_file` is left to tests/integration/image and tests/integration/image/run_pairs.py.
//! Here because it reads and writes files, which crates/engine/src may not: tests/foundation.rs.

use rstex_engine::consts::*;
use rstex_engine::host::*;
use rstex_engine::image::FmtRecord;
use rstex_engine::raw::*;
use rstex_engine::state::*;
use std::path::PathBuf;

fn scratch(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("rstex-image-test-{}-{}-{:?}", tag, std::process::id(), std::thread::current().id()));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// The limits a run has when `load_fmt_file` is reached.
fn limits(t: &mut Tex) {
    t.pool_size = 100000;
    t.pool_free = 1000;
    t.max_strings = 3000;
    t.strings_free = 100;
    t.font_mem_size = 20000;
    t.font_max = 75;
    t.hyph_size = 659;
    t.trie_size = 8000;
    t.eqtb_top = EQTB_CAP as i32 - 1;
    t.hash_top = t.eqtb_top;
    t.interaction_option = UNSPECIFIED_MODE;
}

/// State that a format holds: each kind of field has a value that a load must bring back.
fn dumped_state(t: &mut Tex) {
    limits(t);
    for i in 0..256 {
        t.xord.put(i, 255 - i);
        t.xchr.put(i, i);
        t.xprn.put(i, i & 1);
    }
    t.hash_high = 3;
    t.eTeX_mode = 1;
    t.str_ptr = 300;
    t.pool_ptr = 2000;
    for i in 0..=300 {
        t.str_start[i] = i * 6;
    }
    t.str_pool.put(1999, 77);
    t.lo_mem_max = LO_MEM_STAT_MAX + 1500;
    t.rover = LO_MEM_STAT_MAX + 1;
    t.set_node_size(t.rover, 50);
    t.set_rlink(t.rover, t.rover);
    t.set_llink(t.rover, t.rover);
    t.mem[100].set_int(4242);
    t.hi_mem_min = HI_MEM_STAT_MIN - 2;
    t.avail = NULL;
    t.mem_end = MEM_TOP;
    t.mem[t.hi_mem_min].set_int(4343);
    t.mem[MEM_TOP].set_int(4444);
    t.var_used = 11;
    t.dyn_used = 22;
    t.sa_root[INT_VAL] = 100;
    t.sa_root[TOK_VAL] = 101;
    t.par_loc = EQTB_SIZE + 1;
    t.write_loc = EQTB_SIZE + 2;
    t.prim[5] = Word::hh(6, 7);
    t.hash_used = FROZEN_CONTROL_SEQUENCE;
    t.cs_count = 3;
    t.set_pdf_ignored_dimen(-123);
    t.eqtb[EQTB_SIZE + 3].set_int(555);
    t.set_text(EQTB_SIZE + 3, 299);
    t.hash_tab_size = 1 << 14;
    t.hash_tab_used = 3;
    t.hash_tab[7] = 0x1234;
    t.toks_top = NULL_LIST + 1000;
    t.toks[NULL_LIST + 999] = 8;
    t.tok_free[3] = 300;
    t.arena_top = 2;
    t.arena[1] = 99;
    t.fmem_ptr = 7;
    t.font_info[6].set_sc(66);
    t.font_ptr = 1;
    t.font_name[1] = 299;
    t.hyph_count = 1;
    t.hyph_next = 607;
    t.hyph_word[610] = 299;
    t.hyph_list[610] = 123;
    t.hyph_link[610] = 0;
    t.trie_max = 4;
    t.hyph_start = 2;
    t.trie_trl[4] = 9;
    t.trie_tro[4] = 10;
    t.trie_trc[4] = 11;
    t.trie_op_ptr = 5;
    t.hyf_next[5] = 12;
    t.trie_used[200] = 2;
    t.trie_used[100] = 3;
    t.pdf_mem_size = 10000;
    t.pdf_mem_ptr = 3;
    t.pdf_mem[2] = 13;
    t.obj_tab_size = 1000;
    t.obj_ptr = 2;
    t.sys_obj_ptr = 2;
    t.obj_tab[2].int0 = 14;
    t.obj_tab[2].int2 = 15;
    t.pdf_obj_count = 1;
    t.pdf_xform_count = 2;
    t.pdf_ximage_count = 3;
    t.head_tab[OBJ_TYPE_OBJ] = 16;
    t.head_tab[OBJ_TYPE_XFORM] = 17;
    t.head_tab[OBJ_TYPE_XIMAGE] = 18;
    t.pdf_last_obj = 19;
    t.pdf_last_xform = 20;
    t.pdf_last_ximage = 21;
    t.interaction = BATCH_MODE;
    t.format_ident = 299;
}

/// What `store_fmt_file` does to the file: §1504's two and the bytes.
fn write_fmt(t: &mut Tex, path: &[u8]) {
    t.set_name_of_file(path);
    assert!(t.w_open_out(FMT_FILE));
    t.host.hv.image.extra.clear();
    t.dumpimagemeta();
    t.dumptounicode();
    t.image_write_fmt();
    t.w_close(FMT_FILE);
}

/// What is live at section 1517, which a format must leave alone.
fn live_state(t: &mut Tex) {
    limits(t);
    t.first = 40;
    t.last = 39;
    t.buffer.put(30, 65);
    t.cur_input.loc_field = 31;
    t.selector = TERM_ONLY;
    t.history = FATAL_ERROR_STOP;
    t.ini_version = false;
    t.job_name = 0;
    t.log_opened = false;
    t.open_parens = 0;
    t.etex_p = true;
    t.pdf_output_option = 1;
    t.set_prev_depth(5);
}

fn load(t: &mut Tex, path: &[u8]) -> bool {
    t.set_name_of_file(path);
    assert!(t.w_open_in(FMT_FILE));
    let ok = t.load_fmt_file();
    t.w_close(FMT_FILE);
    ok
}

fn terminal(t: &Tex) -> String {
    String::from_utf8_lossy(&t.host.term_buf).to_string()
}

fn file_of(dir: &PathBuf, name: &str) -> Vec<u8> {
    rstex_engine::platform::path_to_bytes(&dir.join(name))
}

#[test]
fn a_format_comes_back_and_the_run_stays() {
    let dir = scratch("round");
    let f = file_of(&dir, "a.fmt");
    let a = Tex::alloc();
    dumped_state(a);
    write_fmt(a, &f);
    Tex::dealloc(a);

    let b = Tex::alloc();
    live_state(b);
    assert!(load(b, &f), "{}", terminal(b));
    // The record.
    assert_eq!((b.xord.at(0), b.xprn.at(1), b.xprn.at(2)), (255, 1, 0));
    assert_eq!((b.hash_high, b.eTeX_mode, b.max_reg_num, b.max_reg_help_line), (3, 1, 32767, 1));
    assert_eq!((b.lo_mem_max, b.rover, b.hi_mem_min, b.avail), (LO_MEM_STAT_MAX + 1500, LO_MEM_STAT_MAX + 1, HI_MEM_STAT_MIN - 2, NULL));
    assert_eq!((b.var_used, b.dyn_used), (11, 22));
    assert_eq!((b.sa_root[INT_VAL], b.sa_root[TOK_VAL]), (100, 101));
    assert_eq!((b.par_loc, b.par_token, b.write_loc), (EQTB_SIZE + 1, CS_TOKEN_FLAG + EQTB_SIZE + 1, EQTB_SIZE + 2));
    assert_eq!((b.prim[5].rh(), b.prim[5].lh()), (6, 7));
    assert_eq!((b.toks_top, b.tok_free[3], b.hash_tab_size, b.hash_tab_used, b.arena_top), (NULL_LIST + 1000, 300, 1 << 14, 3, 2));
    assert_eq!((b.pdf_obj_count, b.pdf_xform_count, b.pdf_ximage_count), (1, 2, 3));
    assert_eq!((b.head_tab[OBJ_TYPE_OBJ], b.head_tab[OBJ_TYPE_XFORM], b.head_tab[OBJ_TYPE_XIMAGE]), (16, 17, 18));
    assert_eq!((b.pdf_last_obj, b.pdf_last_xform, b.pdf_last_ximage), (19, 20, 21));
    assert_eq!((b.interaction, b.format_ident), (BATCH_MODE, 299));
    // The sections.
    assert_eq!((b.str_start[300], b.str_pool.at(1999)), (1800, 77));
    assert_eq!((b.mem[100].int(), b.mem[HI_MEM_STAT_MIN - 2].int(), b.mem[MEM_TOP].int()), (4242, 4343, 4444));
    assert_eq!((b.eqtb[EQTB_SIZE + 3].int(), b.text(EQTB_SIZE + 3), b.hash_tab[7]), (555, 299, 0x1234));
    assert_eq!((b.toks[NULL_LIST + 999], b.arena[1], b.font_info[6].sc(), b.font_name[1]), (8, 99, 66, 299));
    assert_eq!((b.trie_trl[4], b.trie_tro[4], b.trie_trc[4], b.hyf_next[5]), (9, 10, 11, 12));
    assert_eq!((b.pdf_mem[2], b.obj_tab[2].int0), (13, 14));
    // Not dumped, and set: section 1486, section 1488, section 1490, section 1505, section 1481.
    assert_eq!((b.head(), b.tail(), b.page_tail), (CONTRIB_HEAD, CONTRIB_HEAD, PAGE_HEAD));
    assert_eq!((b.init_str_ptr, b.init_pool_ptr), (300, 2000));
    assert_eq!(b.str_ptr, 301, "section 1501's banner comes after init_str_ptr");
    assert_eq!(b.mem_end, MEM_TOP);
    assert_eq!(b.obj_tab[2].int2, -1);
    assert_eq!(b.prev_depth(), -123, "\\pdfignoreddimen, from the format's eqtb");
    assert!(!b.trie_not_ready);
    // section 1503: the largest occupied location is 610, so 611, above hyph_prime, is incremented.
    assert_eq!((b.hyph_count, b.hyph_next, b.hyph_word[610], b.hyph_list[610]), (1, 612, 299, 123));
    // section 1503: ops, going down from language 255.
    assert_eq!((b.trie_used[200], b.op_start[200], b.trie_used[100], b.op_start[100], b.trie_used[0]), (2, 3, 3, 0, 0));
    // The run stays what it was.
    assert_eq!((b.first, b.last, b.buffer.at(30), b.cur_input.loc_field), (40, 39, 65, 31));
    assert_eq!((b.selector, b.history, b.job_name, b.log_opened), (TERM_ONLY, FATAL_ERROR_STOP, 0, false));
    assert!(b.etex_p && b.pdf_output_option == 1 && !b.ini_version);
    assert_ne!(b.pdftex_banner, 0, "section 1501 makes the banner on load");
    Tex::dealloc(b);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_same_run_gives_the_same_bytes() {
    let dir = scratch("same");
    let (f, g) = (file_of(&dir, "a.fmt"), file_of(&dir, "b.fmt"));
    for p in [&f, &g] {
        let a = Tex::alloc();
        dumped_state(a);
        write_fmt(a, p);
        Tex::dealloc(a);
    }
    let read = |b: &[u8]| std::fs::read(rstex_engine::platform::path_from_bytes(b)).unwrap();
    assert_eq!(read(&f), read(&g));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Runs a load of a format changed by `change`, returning what it printed.  The load must fail.
fn bad_load(tag: &str, change: impl Fn(&mut Tex), edit: impl Fn(&mut Vec<u8>), set: impl Fn(&mut Tex)) -> String {
    let dir = scratch(tag);
    let f = file_of(&dir, "a.fmt");
    let a = Tex::alloc();
    dumped_state(a);
    change(a);
    write_fmt(a, &f);
    Tex::dealloc(a);
    let path = rstex_engine::platform::path_from_bytes(&f);
    let mut bytes = std::fs::read(&path).unwrap();
    edit(&mut bytes);
    std::fs::write(&path, &bytes).unwrap();
    let b = Tex::alloc();
    live_state(b);
    set(b);
    assert!(!load(b, &f), "{}: the load should fail", tag);
    let s = terminal(b);
    assert!(s.ends_with("(Fatal format file error; I'm stymied)\n"), "{}: {}", tag, s);
    Tex::dealloc(b);
    std::fs::remove_dir_all(&dir).unwrap();
    s
}

const HEADER: usize = 4 + 4 + 8 + 8;

#[test]
fn a_file_that_is_no_format_is_refused() {
    // gzip's magic: every format of TeX Live's own.
    let s = bad_load("magic", |_| {}, |b| b[..4].copy_from_slice(&[0x1f, 0x8b, 8, 0]), |_| {});
    assert_eq!(s, "(Fatal format file error; I'm stymied)\n");
    // Cut short: C says "Could not undump" and stops; here it is the same failure as a bad file.
    let s = bad_load("short", |_| {}, |b| b.truncate(HEADER + 100), |_| {});
    assert_eq!(s, "(Fatal format file error; I'm stymied)\n");
    let s = bad_load("closing", |_| {}, |b| *b.last_mut().unwrap() ^= 1, |_| {});
    assert_eq!(s, "(Fatal format file error; I'm stymied)\n");
}

#[test]
fn another_engine_and_another_build_are_named() {
    let s = bad_load("engine", |_| {}, |b| b[8..14].copy_from_slice(b"xetex\0"), |_| {});
    assert!(s.contains(" was written by xetex\n"), "{}", s);
    assert!(s.starts_with("---! "), "{}", s);
    let s = bad_load("build", |_| {}, |b| b[16] ^= 1, |_| {});
    assert!(s.contains(" made by different executable version, strings are different\n"), "{}", s);
}

/// Each `undump` of a scalar, with its bounds: section 1486-section 1507.
#[test]
fn each_range_check_refuses() {
    let plain = "(Fatal format file error; I'm stymied)\n";
    let cases: Vec<(&str, fn(&mut Tex))> = vec![
        ("hash_high", |t| t.hash_high = -1),
        ("etex_mode", |t| t.eTeX_mode = 2),
        ("lo_mem_max", |t| t.lo_mem_max = LO_MEM_STAT_MAX + 999),
        ("rover", |t| t.rover = LO_MEM_STAT_MAX),
        ("sa_root", |t| t.sa_root[BOX_VAL] = t.lo_mem_max + 1),
        ("hi_mem_min", |t| t.hi_mem_min = HI_MEM_STAT_MIN + 1),
        ("write_loc", |t| t.write_loc = HASH_BASE - 1),
        ("fmem_ptr", |t| t.fmem_ptr = 6),
        ("hyph_next", |t| t.hyph_next = HYPH_PRIME - 1),
        ("hyph_start", |t| t.hyph_start = t.trie_max + 1),
        ("interaction", |t| t.interaction = ERROR_STOP_MODE + 1),
        ("format_ident", |t| t.format_ident = t.str_ptr + 2), // str_ptr has the banner's by then
        ("trie_used_sum", |t| t.trie_used[100] = 1),
    ];
    for (tag, change) in cases {
        let s = bad_load(tag, change, |_| {}, |_| {});
        assert_eq!(s, plain, "{}", tag);
    }
}

#[test]
fn a_table_too_small_says_which() {
    let s = bad_load("hyph", |t| t.hyph_next = 700, |_| {}, |_| {});
    assert_eq!(s, "---! Must increase the hyph_size\n(Fatal format file error; I'm stymied)\n");
    let s = bad_load("trie", |t| t.trie_max = 9000, |_| {}, |_| {});
    assert_eq!(s, "---! Must increase the trie size\n(Fatal format file error; I'm stymied)\n");
    let s = bad_load("pool", |t| t.pool_ptr = SUP_POOL_SIZE, |_| {}, |t| t.pool_free = 5);
    assert_eq!(s, "---! Must increase the string pool size\n(Fatal format file error; I'm stymied)\n");
}

#[test]
fn a_broken_free_list_is_refused() {
    let s = bad_load("rover1", |t| t.set_node_size(t.rover, t.lo_mem_max), |_| {}, |_| {});
    assert_eq!(s, "(Fatal format file error; I'm stymied)\n");
    let s = bad_load("rover2", |t| t.set_rlink(t.rover, t.lo_mem_max + 1), |_| {}, |_| {});
    assert_eq!(s, "(Fatal format file error; I'm stymied)\n");
}

#[test]
fn a_table_out_of_range_is_refused() {
    // Cannot come from store, so only a damaged file: the end of the first section past its capacity.
    let at = HEADER + core::mem::size_of::<FmtRecord>() + 8;
    let s = bad_load("section", |_| {}, |b| b[at..at + 8].copy_from_slice(&u64::MAX.to_le_bytes()), |_| {});
    assert_eq!(s, "(Fatal format file error; I'm stymied)\n");
}

#[test]
fn interaction_and_tables_follow_the_command_line() {
    let dir = scratch("cli");
    let f = file_of(&dir, "a.fmt");
    let a = Tex::alloc();
    dumped_state(a);
    write_fmt(a, &f);
    Tex::dealloc(a);
    // section 1507: -interaction wins over the format's.
    let b = Tex::alloc();
    live_state(b);
    b.interaction_option = ERROR_STOP_MODE;
    assert!(load(b, &f));
    assert_eq!(b.interaction, ERROR_STOP_MODE);
    Tex::dealloc(b);
    // section 1875: -8bit makes everything printable.
    let b = Tex::alloc();
    live_state(b);
    b.eight_bit_p = 1;
    assert!(load(b, &f));
    assert_eq!((b.xord.at(0), b.xprn.at(2)), (255, 1));
    Tex::dealloc(b);
    // section 1875: with -translate-file the format's tables are not read.
    let b = Tex::alloc();
    live_state(b);
    b.host.translate_filename = Some(b"x".to_vec());
    b.xord.put(0, 7);
    assert!(load(b, &f));
    assert_eq!(b.xord.at(0), 7);
    Tex::dealloc(b);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn hyph_next_as_the_load_recomputes_it() {
    let run = |word_at: Option<i32>, hyph_size: i32| {
        let dir = scratch(&format!("hyph{}", hyph_size));
        let f = file_of(&dir, "a.fmt");
        let a = Tex::alloc();
        dumped_state(a);
        a.hyph_word[610] = 0;
        a.hyph_count = 0;
        if let Some(k) = word_at {
            a.hyph_word[k] = 299;
            a.hyph_count = 1;
        }
        write_fmt(a, &f);
        Tex::dealloc(a);
        let b = Tex::alloc();
        live_state(b);
        b.hyph_size = hyph_size;
        assert!(load(b, &f));
        let n = b.hyph_next;
        Tex::dealloc(b);
        std::fs::remove_dir_all(&dir).unwrap();
        n
    };
    assert_eq!(run(None, 8191), HYPH_PRIME + 1, "j:=0, then hyph_prime, which is incremented");
    assert_eq!(run(Some(3), 8191), HYPH_PRIME + 1);
    assert_eq!(run(Some(607), 8191), 609);
    assert_eq!(run(Some(610), 611), HYPH_PRIME, "611>=hyph_size wraps");
}

/// The three `PodVec`s of §1504, and nothing else of the modules' state, through a file.
#[test]
fn a_format_holds_the_image_table_and_the_glyph_log_only() {
    let dir = scratch("extra");
    let f = file_of(&dir, "a.fmt");
    let a = Tex::alloc();
    dumped_state(a);
    a.arena_top = 0;
    let mut names = a.v_ext.image_names;
    a.arena_push(&mut names, b'n');
    a.v_ext.image_names = names;
    let mut log = a.v_pdffont.gtu_log;
    a.arena_push(&mut log, b'g');
    a.v_pdffont.gtu_log = log;
    // State the C keeps in statics, each of a kind a load must not bring back.
    let mut pool = a.v_pdffont.pool;
    a.arena_push(&mut pool, b'p');
    a.v_pdffont.pool = pool;
    a.v_pdffont.trees = true;
    a.v_pdfobj.avl_root[1] = 5;
    a.v_ext.transparent_page_group = 7;
    a.v_pdfship.page_mode = true;
    a.v_buildtoks.match_string_set = true;
    write_fmt(a, &f);
    Tex::dealloc(a);
    let b = Tex::alloc();
    live_state(b);
    assert!(load(b, &f), "{}", terminal(b));
    let (names, log) = (b.v_ext.image_names, b.v_pdffont.gtu_log);
    assert_eq!((names.len, log.len, b.v_ext.images.len), (1, 1, 0));
    assert_eq!((names.get(&mut b.arena, 0), log.get(&mut b.arena, 0)), (b'n', b'g'));
    assert_eq!((b.v_pdffont.pool.len, b.v_pdffont.trees, b.v_pdfobj.avl_root[1]), (0, false, 0));
    assert_eq!((b.v_ext.transparent_page_group, b.v_pdfship.page_mode, b.v_buildtoks.match_string_set), (0, false, false));
    Tex::dealloc(b);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_format_without_its_extra_part_is_refused() {
    let dir = scratch("noextra");
    let f = file_of(&dir, "a.fmt");
    let a = Tex::alloc();
    dumped_state(a);
    a.set_name_of_file(&f);
    assert!(a.w_open_out(FMT_FILE));
    a.image_write_fmt();
    a.w_close(FMT_FILE);
    Tex::dealloc(a);
    let b = Tex::alloc();
    live_state(b);
    assert!(!load(b, &f));
    assert!(terminal(b).contains("(Fatal format file error; I'm stymied)"), "{}", terminal(b));
    Tex::dealloc(b);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A load overwrites the arena from 0, so whatever comes before it must not have allocated there.
#[test]
fn nothing_before_a_load_allocates_in_the_arena() {
    let t = Tex::alloc();
    limits(t);
    t.hyph_size = 8191;
    t.trie_size = 100000;
    t.pool_size = 1000000;
    t.buf_size = 200000;
    t.max_strings = 100000;
    t.ini_version = true;
    t.initialize();
    assert_eq!(t.arena_top, 0, "initialize");
    assert!(t.get_strings_started());
    t.init_prim();
    assert_eq!(t.arena_top, 0, "init_prim");
    t.fix_date_and_time();
    assert_eq!(t.arena_top, 0, "the rest of section 1512");
    t.initialize();
    assert_eq!(t.arena_top, 0, "section 1517's second initialize");
    Tex::dealloc(t);
}
