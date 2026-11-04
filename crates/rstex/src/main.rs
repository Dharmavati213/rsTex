//! The `rstex` command: what web2c's `maininit` and `parse_options` do (texmfmp.c), then `main_body`.
//! The engine runs on a stack of 1 GiB, since TeX recurses deeply (DESIGN.md section 8): `platform::run_on_stack`.
//!
//! The rules are texmfmp.c's, because they are what a command that replaces `pdftex` and `pdflatex`
//! has to keep: one or two dashes, unique prefixes of option names, stopping at the first argument
//! that is not an option; a first argument with `&` or `\`; the implicit `-ini` from a program name;
//! `%&` on the first line of the input file.  `TEXMFCNF` is kpathsea's, in `crates/kpse`.
//!
//! Format auto-build is in `autofmt.rs`.  The snapshot driver of DESIGN.md section 7 is `run`.  A run looks for a checkpoint unless it is `-ini`, `--no-snapshot` or
//! `RSTEX_NO_SNAPSHOT` says not to, or it has no command line to key one on.  `--until-stable` comes later.
//!
//! `main` is the C runtime's, without Rust's start-up around it, which costs some 25 system calls before anything happens: it reads `/proc/self/maps`, and sets up
//! a handler for a stack overflow on a stack that is not the engine's.  What of it matters is in `platform::process_init` and `platform::run_on_stack`.
#![cfg_attr(not(test), no_main)]
#![deny(unsafe_code)]

mod autofmt;

use rstex_engine::checkpoint::Restored;
use rstex_engine::consts::*;
use rstex_engine::platform;
use rstex_engine::state::Tex;

const ENGINE_STACK: usize = 1 << 30;

/// `TEXMFENGINENAME`, `DUMP_EXT`, `DUMP_OPTION`, `INI_PROGRAM` and `VIR_PROGRAM` of pdftexextra.h.
const DUMP_EXT: &[u8] = b".fmt";
const INI_PROGRAM: &[u8] = b"pdfinitex";
const VIR_PROGRAM: &[u8] = b"pdfvirtex";
/// `SYNCTEX_NO_OPTION`.
const SYNCTEX_NO_OPTION: i32 = i32::MAX;

// ---------------------------------------------------------------------------
// The options: long_options[] and parse_options of texmfmp.c, and getopt_long_only.
// ---------------------------------------------------------------------------

/// What `struct option` says about the option's argument.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HasArg {
    No,
    Required,
    Optional,
}
use HasArg::*;

const LONG_OPTIONS: &[(&str, HasArg)] = &[
    ("fmt", Required),
    ("efmt", Required),
    ("cnf-line", Required),
    ("help", No),
    ("ini", No),
    ("interaction", Required),
    ("halt-on-error", No),
    ("kpathsea-debug", Required),
    ("progname", Required),
    ("recorder", No),
    ("version", No),
    ("ipc", No),
    ("ipc-start", No),
    ("mltex", No),
    ("enc", No),
    ("etex", No),
    ("output-comment", Required),
    ("draftmode", No),
    ("output-format", Required),
    ("shell-escape", No),
    ("no-shell-escape", No),
    ("enable-write18", No),
    ("disable-write18", No),
    ("shell-restricted", No),
    ("debug-format", No),
    ("src-specials", Optional),
    ("synctex", Required),
    ("file-line-error-style", No),
    ("no-file-line-error-style", No),
    ("file-line-error", No),
    ("no-file-line-error", No),
    ("jobname", Required),
    ("output-directory", Required),
    ("parse-first-line", No),
    ("no-parse-first-line", No),
    ("translate-file", Required),
    ("default-translate-file", Required),
    ("8bit", No),
    ("mktex", Required),
    ("no-mktex", Required),
];

/// Everything the command line says, as texmfmp.c's statics and the globals it sets.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Options {
    /// `argv[0]`.
    invocation: Vec<u8>,
    /// `argv[1]`, which `maininit` looks at for `&`: an option before the name hides it.
    argv1: Option<Vec<u8>>,
    ini_version: bool,
    etex_p: bool,
    halt_on_error: bool,
    recorder_enabled: bool,
    debug_format_file: bool,
    /// As in C: 1 on, -1 off, 0 not said.
    shellenabledp: i32,
    restrictedshell: bool,
    /// As in C: 1 on, -1 off, 0 not said, which leaves it to texmf.cnf.
    file_line_error_style: i32,
    /// As in C: 1 on, -1 off, 0 not said, which leaves it to texmf.cnf.
    parse_first_line: i32,
    eight_bit: bool,
    user_progname: Option<Vec<u8>>,
    user_cnf_lines: Vec<Vec<u8>>,
    /// `-mktex` and `-no-mktex`, for `kpse_maketex_option`.
    mktex_options: Vec<(Vec<u8>, bool)>,
    c_job_name: Option<Vec<u8>>,
    dump_name: Option<Vec<u8>>,
    dump_option: bool,
    dump_line: bool,
    output_directory: Option<Vec<u8>>,
    output_comment: Option<Vec<u8>>,
    pdf_output_option: i32,
    pdf_output_value: i32,
    pdf_draftmode_option: i32,
    pdf_draftmode_value: i32,
    translate_filename: Option<Vec<u8>>,
    default_translate_filename: Option<Vec<u8>>,
    /// `interactionoption`; 4 is unspecified.
    interaction_option: i32,
    synctexoption: i32,
    /// `--no-snapshot`.
    no_snapshot: bool,
    /// `--no-commute` or `RSTEX_NO_COMMUTE`: real PGF for every diagram.
    no_commute: bool,
    /// `--commute-report`: the log says what became of each diagram.
    commute_report: bool,
    /// `--commute-report=names`: and the name of a macro that stood in the way.
    commute_report_names: bool,
    /// `argv[optind..]`, what `topenin` joins into the first line.
    rest: Vec<Vec<u8>>,
    /// Messages for stderr, in order: getopt's, and `WARNING`.
    messages: Vec<Vec<u8>>,
}

/// How parsing ended, short of running the engine.
#[derive(Debug, PartialEq, Eq)]
enum Stop {
    /// `--help`, `--version`: to stdout, and exit 0.
    Print(Vec<u8>),
    /// To stderr, and `uexit (1)`.
    Fail(Vec<u8>),
}

fn msg(parts: &[&[u8]]) -> Vec<u8> {
    let mut m = parts.concat();
    m.push(b'\n');
    m
}

/// kpathsea's `WARNING`: no program name, and a period.
fn warning(parts: &[&[u8]]) -> Vec<u8> {
    [b"warning: ", &parts.concat()[..], b".\n"].concat()
}

/// texmfmp.c `normalize_quotes`: quote only if there is a space, and always as `"name"`.
fn normalize_quotes(name: &[u8], mesg: &[u8]) -> Result<Vec<u8>, Vec<u8>> {
    let must_quote = name.contains(&b' ');
    let mut quoted = false;
    let mut ret = Vec::with_capacity(name.len() + 2);
    if must_quote {
        ret.push(b'"');
    }
    for &q in name {
        if q == b'"' {
            quoted = !quoted;
        } else {
            ret.push(q);
        }
    }
    if must_quote {
        ret.push(b'"');
    }
    if quoted {
        return Err(msg(&[b"! Unbalanced quotes in ", mesg, b" ", name]));
    }
    Ok(ret)
}

/// One step of `getopt_long_only` on `args[*optind]`, known to be an option.  Returns the option and
/// its argument, or `None` for getopt's `'?'`, after its message.
fn getopt_step<'a>(args: &'a [Vec<u8>], optind: &mut usize, o: &mut Options) -> Option<(&'static str, Option<&'a [u8]>)> {
    let arg = &args[*optind];
    let dashes: &[u8] = if arg.starts_with(b"--") { b"--" } else { b"-" };
    let body = &arg[dashes.len()..];
    let eq = body.iter().position(|&c| c == b'=');
    let name = &body[..eq.unwrap_or(body.len())];
    let prog = o.invocation.clone();
    *optind += 1;
    // An exact match first; then a prefix, which getopt_long_only takes only if it is the sole one.
    let mut found = LONG_OPTIONS.iter().find(|(n, _)| n.as_bytes() == name);
    if found.is_none() {
        let matches: Vec<_> = LONG_OPTIONS.iter().filter(|(n, _)| n.as_bytes().starts_with(name)).collect();
        found = matches.first().copied();
        if matches.len() > 1 {
            let mut m = [&prog[..], b": option '", dashes, body, b"' is ambiguous; possibilities:"].concat();
            for (n, _) in matches {
                m.extend_from_slice(&[b" '", dashes, n.as_bytes(), b"'"].concat());
            }
            m.push(b'\n');
            o.messages.push(m);
            return None;
        }
    }
    let Some(&(opt, has_arg)) = found else {
        o.messages.push(msg(&[&prog, b": unrecognized option '", dashes, body, b"'"]));
        return None;
    };
    let mut value: Option<&[u8]> = None;
    if let Some(e) = eq {
        if has_arg == No {
            o.messages.push(msg(&[&prog, b": option '", dashes, opt.as_bytes(), b"' doesn't allow an argument"]));
            return None;
        }
        value = Some(&body[e + 1..]);
    } else if has_arg == Required {
        if *optind < args.len() {
            value = Some(&args[*optind]);
            *optind += 1;
        } else {
            o.messages.push(msg(&[&prog, b": option '", dashes, opt.as_bytes(), b"' requires an argument"]));
            return None;
        }
    }
    Some((opt, value))
}

/// Accepted, as it is in C, for a feature the port drops: docs/PORTING.md section 19.
fn dropped(o: &mut Options, what: &[u8]) {
    let m = warning(&[what, b" is not supported; ignoring it"]);
    o.messages.push(m);
}

/// texmfmp.c `parse_options`.  `args[0]` is the program.
fn parse_options(args: &[Vec<u8>]) -> Result<Options, Stop> {
    let mut o = Options {
        invocation: args.first().cloned().unwrap_or_else(|| b"rstex".to_vec()),
        argv1: args.get(1).cloned(),
        ini_version: false,
        etex_p: false,
        halt_on_error: false,
        recorder_enabled: false,
        debug_format_file: false,
        shellenabledp: 0,
        restrictedshell: false,
        file_line_error_style: 0,
        parse_first_line: 0,
        eight_bit: false,
        user_progname: None,
        user_cnf_lines: Vec::new(),
        mktex_options: Vec::new(),
        c_job_name: None,
        dump_name: None,
        dump_option: false,
        dump_line: false,
        output_directory: None,
        output_comment: None,
        pdf_output_option: 0,
        pdf_output_value: 0,
        pdf_draftmode_option: 0,
        pdf_draftmode_value: 0,
        translate_filename: None,
        default_translate_filename: None,
        interaction_option: UNSPECIFIED_MODE, // must be initialized before options are parsed
        synctexoption: SYNCTEX_NO_OPTION,
        no_snapshot: false,
        no_commute: false,
        commute_report: false,
        commute_report_names: false,
        rest: Vec::new(),
        messages: Vec::new(),
    };
    let mut optind = 1;
    while optind < args.len() {
        let arg = &args[optind];
        // The optstring is "+": stop at the first argument that is not an option.
        if arg.first() != Some(&b'-') || arg.len() == 1 {
            break;
        }
        if arg == b"--" {
            optind += 1;
            break;
        }
        // Ours, and not in the table: it would make a prefix of pdfTeX's -no-shell-escape ambiguous.
        if arg == b"--no-snapshot" || arg == b"-no-snapshot" {
            o.no_snapshot = true;
            optind += 1;
            continue;
        }
        if arg == b"--no-commute" || arg == b"-no-commute" {
            o.no_commute = true;
            optind += 1;
            continue;
        }
        if arg == b"--commute-report" || arg == b"-commute-report" {
            o.commute_report = true;
            optind += 1;
            continue;
        }
        if arg == b"--commute-report=names" {
            o.commute_report = true;
            o.commute_report_names = true;
            optind += 1;
            continue;
        }
        let Some((opt, value)) = getopt_step(args, &mut optind, &mut o) else {
            continue; // unknown option: getopt has said so
        };
        let v = value.unwrap_or(b"");
        match opt {
            "ini" => o.ini_version = true,
            "etex" => o.etex_p = true,
            "halt-on-error" => o.halt_on_error = true,
            "recorder" => o.recorder_enabled = true,
            "debug-format" => o.debug_format_file = true,
            "shell-escape" | "enable-write18" => o.shellenabledp = 1,
            "no-shell-escape" | "disable-write18" => o.shellenabledp = -1,
            "shell-restricted" => {
                o.shellenabledp = 1;
                o.restrictedshell = true;
            }
            "file-line-error-style" | "file-line-error" => o.file_line_error_style = 1,
            "no-file-line-error-style" | "no-file-line-error" => o.file_line_error_style = -1,
            "parse-first-line" => o.parse_first_line = 1,
            "no-parse-first-line" => o.parse_first_line = -1,
            "8bit" => o.eight_bit = true,
            "kpathsea-debug" => {} // kpathsea_debug: kpse has no such output, and it goes to stderr only
            "progname" => o.user_progname = Some(v.to_vec()),
            "cnf-line" => o.user_cnf_lines.push(v.to_vec()),
            "jobname" => match normalize_quotes(v, b"jobname") {
                Ok(n) => o.c_job_name = Some(n),
                Err(m) => return Err(Stop::Fail(m)),
            },
            "fmt" | "efmt" => {
                o.dump_name = Some(v.to_vec());
                o.dump_option = true;
            }
            "output-directory" => o.output_directory = Some(v.to_vec()),
            "output-comment" => {
                if v.len() < 256 {
                    o.output_comment = Some(v.to_vec());
                } else {
                    let m = warning(&[b"Comment truncated to 255 characters from ", v.len().to_string().as_bytes(), b". (", v, b")"]);
                    o.messages.push(m);
                    o.output_comment = Some(v[..255].to_vec());
                }
            }
            "output-format" => {
                o.pdf_output_option = 1;
                if v == b"dvi" {
                    o.pdf_output_value = 0;
                } else if v == b"pdf" {
                    o.pdf_output_value = 2;
                } else {
                    let m = warning(&[b"Ignoring unknown value `", v, b"' for --output-format"]);
                    o.messages.push(m);
                    o.pdf_output_option = 0;
                }
            }
            "draftmode" => {
                o.pdf_draftmode_option = 1;
                o.pdf_draftmode_value = 1;
            }
            "translate-file" => o.translate_filename = Some(v.to_vec()),
            "default-translate-file" => o.default_translate_filename = Some(v.to_vec()),
            "interaction" => match v {
                // These numbers match @d's in *.ch
                b"batchmode" => o.interaction_option = BATCH_MODE,
                b"nonstopmode" => o.interaction_option = NONSTOP_MODE,
                b"scrollmode" => o.interaction_option = SCROLL_MODE,
                b"errorstopmode" => o.interaction_option = ERROR_STOP_MODE,
                _ => {
                    let m = warning(&[b"Ignoring unknown argument `", v, b"' to --interaction"]);
                    o.messages.push(m);
                }
            },
            "synctex" => o.synctexoption = strtol0(v),
            "help" => return Err(Stop::Print(usage())),
            "version" => return Err(Stop::Print(version())),
            "mltex" => dropped(&mut o, b"-mltex"),
            "enc" => dropped(&mut o, b"-enc"),
            "ipc" | "ipc-start" => dropped(&mut o, b"-ipc"),
            "src-specials" => dropped(&mut o, b"-src-specials"),
            "mktex" => o.mktex_options.push((v.to_vec(), true)),
            "no-mktex" => o.mktex_options.push((v.to_vec(), false)),
            _ => unreachable!("every name in LONG_OPTIONS has an arm"),
        }
    }
    o.rest = args[optind.min(args.len())..].to_vec();
    Ok(o)
}

/// `strtol (optarg, NULL, 0)`, as far as `-synctex` needs it: decimal, a sign, `0x`.
fn strtol0(s: &[u8]) -> i32 {
    let s = std::str::from_utf8(s).unwrap_or("").trim_start();
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let (radix, digits) = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(r) => (16, r),
        None if s.starts_with('0') => (8, s),
        None => (10, s),
    };
    let end = digits.find(|c: char| !c.is_digit(radix)).unwrap_or(digits.len());
    let n = i64::from_str_radix(&digits[..end], radix).unwrap_or(0);
    let n = if neg { -n } else { n };
    n.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

fn usage() -> Vec<u8> {
    b"Usage: rstex [OPTION]... [TEXNAME[.tex]] [COMMANDS]
   or: rstex [OPTION]... \\FIRST-LINE
   or: rstex [OPTION]... &FMT ARGS
  Run a pdfTeX-compatible engine on TEXNAME, usually creating TEXNAME.pdf.
  Options may start with one dash or two, and be shortened to any unique prefix.

-cnf-line=STRING        parse STRING as a configuration file line
-draftmode              switch on draft mode (generates no output PDF)
-enc, -ipc, -mltex, -src-specials   accepted and ignored: not supported
-etex                   enable e-TeX extensions
[-no]-file-line-error   disable/enable file:line:error style messages
-fmt=FMTNAME            use FMTNAME instead of program name or a %& line
-halt-on-error          stop processing at the first error
-help                   display this help and exit
-ini                    be pdfinitex, for dumping formats
-interaction=STRING     set interaction mode (STRING=batchmode/nonstopmode/
                          scrollmode/errorstopmode)
-jobname=STRING         set the job name to STRING
-kpathsea-debug=NUMBER  accepted and ignored
-output-comment=STRING  use STRING for DVI file comment instead of date
-output-directory=DIR   use existing DIR as the directory to write files in
-output-format=FORMAT   use FORMAT for job output; FORMAT is `dvi' or `pdf'
[-no]-parse-first-line  disable/enable parsing of first line of input file
-no-snapshot            do not restore or write a preamble checkpoint or page records
-no-commute             run real PGF for every tikzcd
-commute-report         say in the log what became of each tikzcd
-progname=STRING        set program (and fmt) name to STRING
-recorder               enable filename recorder
[-no]-shell-escape      disable/enable \\write18{SHELL COMMAND}
-shell-restricted       enable restricted \\write18
-synctex=NUMBER         generate SyncTeX data for previewers
-translate-file=TCXNAME use the TCX file TCXNAME; only cp227 is supported
-8bit                   make all characters printable by default
-version                output version information and exit
"
    .to_vec()
}

fn version() -> Vec<u8> {
    format!(
        "rstex {}\nCompatible with {}\n",
        env!("CARGO_PKG_VERSION"),
        String::from_utf8_lossy(&BANNER[b"This is ".len()..]),
    )
    .into_bytes()
}

// ---------------------------------------------------------------------------
// maininit, from where the options end: what is left depends on kpathsea.
// ---------------------------------------------------------------------------

/// What `maininit` asks of kpathsea and of the system.  Tests give their own.
trait System {
    fn getenv(&mut self, name: &[u8]) -> Option<Vec<u8>>;
    /// `xputenv`.
    fn putenv(&mut self, name: &[u8], value: &[u8]);
    fn set_program_name(&mut self, invocation: &[u8], progname: &[u8]);
    fn cnf_line(&mut self, line: &[u8]);
    fn maketex_option(&mut self, fmt: &[u8], value: bool);
    /// `kpse_find_file (name, kpse_tex_format, false)`.
    fn find_tex(&mut self, name: &[u8]) -> Option<Vec<u8>>;
    /// `kpse_find_file (name, kpse_fmt_format, false)`, if `kpse_readable_file`.
    fn find_fmt(&mut self, name: &[u8]) -> Option<Vec<u8>>;
    /// `kpse_find_file (name, kpse_web2c_format, true)`.
    fn find_web2c(&mut self, name: &[u8]) -> Option<Vec<u8>>;
    /// The file's first line, for `%&`.
    fn first_line(&mut self, path: &[u8]) -> Option<Vec<u8>>;
    fn var_value(&mut self, var: &[u8]) -> Option<Vec<u8>>;
    /// True on a system where `kpse_absolute_p` takes `C:\`: only there is a backslash a separator.
    fn is_absolute(&self, name: &[u8]) -> bool;
    /// The file to load for the format `name`, if rstex has to supply it itself: see `autofmt.rs`.
    fn own_format(&mut self, name: &[u8]) -> Option<Vec<u8>>;
}

/// The real one: `crates/kpse`, and `platform` for the rest.
struct RealSystem;

impl System for RealSystem {
    fn getenv(&mut self, name: &[u8]) -> Option<Vec<u8>> {
        platform::env_var(name)
    }
    fn putenv(&mut self, name: &[u8], value: &[u8]) {
        rstex_kpse::set_var(name, value);
    }
    fn set_program_name(&mut self, invocation: &[u8], progname: &[u8]) {
        rstex_kpse::with_global(|k| {
            k.set_invocation_name(invocation);
            k.set_program_name(progname);
        });
    }
    fn maketex_option(&mut self, fmt: &[u8], value: bool) {
        rstex_kpse::maketex_option(fmt, value);
    }
    fn cnf_line(&mut self, line: &[u8]) {
        rstex_kpse::cnf_line_env_progname(line);
    }
    fn find_tex(&mut self, name: &[u8]) -> Option<Vec<u8>> {
        rstex_kpse::find_file(name, rstex_kpse::KPSE_TEX_FORMAT, false)
    }
    fn find_fmt(&mut self, name: &[u8]) -> Option<Vec<u8>> {
        if let Some(f) = rstex_kpse::find_file(name, rstex_kpse::KPSE_FMT_FORMAT, false) {
            std::fs::File::open(platform::path_from_bytes(&f)).ok()?;
            return Some(f);
        }
        let base = name.strip_suffix(b".fmt")?;
        if autofmt::can_build(base) { Some(name.to_vec()) } else { None }
    }
    fn find_web2c(&mut self, name: &[u8]) -> Option<Vec<u8>> {
        rstex_kpse::find_file(name, rstex_kpse::KPSE_WEB2C_FORMAT, true)
    }
    fn first_line(&mut self, path: &[u8]) -> Option<Vec<u8>> {
        use std::io::BufRead;
        let f = std::fs::File::open(platform::path_from_bytes(path)).ok()?;
        let mut line = Vec::new();
        std::io::BufReader::new(f).read_until(b'\n', &mut line).ok()?;
        let end = line.iter().position(|&c| c == b'\n' || c == b'\r').unwrap_or(line.len());
        line.truncate(end);
        Some(line)
    }
    fn var_value(&mut self, var: &[u8]) -> Option<Vec<u8>> {
        rstex_kpse::var_value(var)
    }
    fn is_absolute(&self, name: &[u8]) -> bool {
        platform::is_absolute(name)
    }
    fn own_format(&mut self, name: &[u8]) -> Option<Vec<u8>> {
        autofmt::resolve(name)
    }
}

/// `texmf_yesno`.
fn yesno(sys: &mut dyn System, var: &[u8]) -> bool {
    matches!(sys.var_value(var).as_deref().and_then(|v| v.first()), Some(b't' | b'y' | b'1'))
}

/// `kpse_set_program_name`'s `kpse_program_basename` and its removal of the suffix.
fn program_basename(invocation: &[u8]) -> Vec<u8> {
    let base = rstex_kpse::names::xbasename(invocation);
    rstex_kpse::names::remove_suffix(base).to_vec()
}

/// What `maininit` decides beyond the options.
#[derive(Debug, PartialEq, Eq)]
struct Init {
    /// `kpse_program_name`.
    program_name: Vec<u8>,
    dump_name: Vec<u8>,
    /// `DUMP_VAR`: " name.fmt", "adjust array for Pascal".
    format_default: Vec<u8>,
    /// `DUMP_LENGTH_VAR`.
    format_default_length: i32,
    file_line_error_style_p: bool,
    parse_first_line_p: bool,
    output_comment: Option<Vec<u8>>,
    shellenabledp: bool,
    restrictedshell: bool,
    /// What `topenin` puts in the buffer.
    command_line: Vec<u8>,
}

/// texmfmp.c `maininit`, after `parse_options`.
fn maininit(o: &mut Options, sys: &mut dyn System) -> Result<Init, Stop> {
    // parse_options' last part, on the environment.
    if let Some(dir) = &o.output_directory {
        sys.putenv(b"TEXMF_OUTPUT_DIRECTORY", dir);
    } else if let Some(dir) = sys.getenv(b"TEXMF_OUTPUT_DIRECTORY") {
        o.output_directory = Some(dir);
    }
    // If -progname was not specified, default to the dump name.
    if o.user_progname.is_none() {
        o.user_progname = o.dump_name.clone();
    }
    // kpse_set_program_name (argv[0], user_progname).  texmf.cnf knows nothing of `rstex`, which
    // stands in for `pdftex` when called by its own name.
    let program_name = match &o.user_progname {
        Some(p) => p.clone(),
        None => {
            let b = program_basename(&o.invocation);
            if rstex_kpse::names::file_str_eq(&b, b"rstex") { b"pdftex".to_vec() } else { b }
        }
    };
    sys.set_program_name(&program_basename(&o.invocation), &program_name);
    for (fmt, value) in &o.mktex_options {
        sys.maketex_option(fmt, *value);
    }
    for l in &o.user_cnf_lines {
        sys.cnf_line(l);
    }
    // Were we given a simple filename?  get_input_file_name.
    let mut main_input_file = None;
    if let Some(a0) = o.rest.first().cloned() {
        if a0.first() != Some(&b'&') && a0.first() != Some(&b'\\') {
            let mut a = a0;
            // On Windows `C:\name` has its backslashes turned round.  Elsewhere it is no path.
            if a.len() > 2 && a[0].is_ascii_alphabetic() && a[1] == b':' && a[2] == b'\\' && sys.is_absolute(&a) {
                for c in a.iter_mut() {
                    if *c == b'\\' {
                        *c = b'/';
                    }
                }
            }
            let name = normalize_quotes(&a, b"argument").map_err(Stop::Fail)?;
            let unquoted = if name.first() == Some(&b'"') { &name[1..name.len() - 1] } else { &name[..] };
            main_input_file = sys.find_tex(unquoted);
            o.rest[0] = name;
        }
    }
    // Second chance to activate file:line:error style messages, this time from texmf.cnf.
    let file_line_error_style_p = if o.file_line_error_style < 0 {
        false
    } else if o.file_line_error_style == 0 {
        yesno(sys, b"file_line_error_style")
    } else {
        true
    };
    // If no dump default yet, and we're not doing anything special on this run, we may want to look
    // at the first line of the main input file for a %&<dumpname> specifier.
    let parse_first_line_p = if o.parse_first_line < 0 {
        false
    } else if o.parse_first_line == 0 {
        yesno(sys, b"parse_first_line")
    } else {
        true
    };
    let mut program_name = program_name;
    if parse_first_line_p && (o.dump_name.is_none() || o.translate_filename.is_none()) {
        parse_first_line(o, sys, main_input_file.as_deref(), &mut program_name);
    }
    // Check whether there still is no translate_filename known.  If so, use the default.
    if o.translate_filename.is_none() {
        o.translate_filename = o.default_translate_filename.clone();
    }
    if let Some(t) = &o.translate_filename {
        let base = rstex_kpse::names::xbasename(t);
        if rstex_kpse::names::remove_suffix(base) != b"cp227" {
            let m = warning(&[b"the TCX reader is dropped, so `", t, b"' gets the table of cp227.tcx"]);
            o.messages.push(m);
        }
        let with_ext = if rstex_kpse::names::find_suffix(t).is_none() { [&t[..], b".tcx"].concat() } else { t.clone() };
        if sys.find_web2c(&with_ext).is_none() {
            let m = warning(&[b"Could not open char translation file `", &with_ext, b"'"]);
            o.messages.push(m);
        }
    }
    let mut virversion = false;
    if rstex_kpse::names::file_str_eq(&program_name, INI_PROGRAM) {
        o.ini_version = true;
    } else if rstex_kpse::names::file_str_eq(&program_name, VIR_PROGRAM) {
        virversion = true;
    } else if rstex_kpse::names::file_str_eq(&program_name, b"initex") {
        o.ini_version = true;
    } else if rstex_kpse::names::file_str_eq(&program_name, b"virtex") {
        virversion = true;
    }
    // dropped: MLTeX, `mltex` as a program name sets mltexp.
    // If run like `tex \&foo', reasonable to guess "foo" as the fmt name.
    if main_input_file.is_none() {
        if let Some(a1) = &o.argv1 {
            if a1.first() == Some(&b'&') {
                o.dump_name = Some(a1[1..].to_vec());
            }
        }
    }
    if o.dump_name.is_none() {
        // If called as *vir{mf,tex,mpost} use `plain'.  Otherwise, use the name we were invoked under.
        o.dump_name = Some(if virversion { b"plain".to_vec() } else { program_name.clone() });
    }
    // Sanity check: -mltex, -enc, -etex only work in combination with -ini.
    if !o.ini_version && o.etex_p {
        o.messages.push(b"-etex only works with -ini\n".to_vec());
    }
    // If we've set up the fmt/base default in any of the various ways above, also set its length.
    let dump_name = o.dump_name.clone().expect("set just above");
    let name_len = dump_name.len();
    let ext_len = DUMP_EXT.len();
    let with_ext = if name_len > ext_len && rstex_kpse::names::file_str_eq(&dump_name[name_len - ext_len..], DUMP_EXT) {
        dump_name.clone()
    } else {
        [&dump_name[..], DUMP_EXT].concat()
    };
    let mut format_default = [&b" "[..], &with_ext].concat();
    let mut format_default_length = with_ext.len() as i32;
    // Nothing on the command line says which format, so this engine may have to supply it.  `&name` in the first line goes to section 1517.
    if !o.ini_version && !o.dump_option && o.rest.first().and_then(|a| a.first()) != Some(&b'&') {
        if let Some(path) = sys.own_format(&dump_name) {
            format_default_length = path.len() as i32;
            format_default = [&b" "[..], &path].concat();
        }
    }
    // init_shell_escape
    let mut shellenabledp = o.shellenabledp;
    let mut restrictedshell = o.restrictedshell;
    if shellenabledp < 0 {
        shellenabledp = 0; // --no-shell-escape on cmd line
    } else if shellenabledp == 0 {
        // no shell options on cmd line, check cnf
        match sys.var_value(b"shell_escape").as_deref().and_then(|v| v.first()) {
            Some(b't' | b'y' | b'1') => shellenabledp = 1,
            Some(b'p') => {
                shellenabledp = 1;
                restrictedshell = true;
            }
            _ => {}
        }
    }
    if o.output_comment.is_none() {
        o.output_comment = sys.var_value(b"output_comment");
    }
    // topenin: each argument followed by a space, and then the end is trimmed of spaces and line ends.
    let mut command_line = Vec::new();
    for a in &o.rest {
        command_line.extend_from_slice(a);
        command_line.push(b' ');
    }
    while matches!(command_line.last(), Some(b' ' | b'\r' | b'\n')) {
        command_line.pop();
    }
    Ok(Init {
        program_name,
        format_default_length,
        format_default,
        dump_name,
        file_line_error_style_p,
        parse_first_line_p,
        output_comment: o.output_comment.clone(),
        shellenabledp: shellenabledp != 0,
        restrictedshell,
        command_line,
    })
}

/// texmfmp.c `parse_first_line`: `%&FORMAT`, and `-translate-file`, on the input file's first line.
fn parse_first_line(o: &mut Options, sys: &mut dyn System, filename: Option<&[u8]>, program_name: &mut Vec<u8>) {
    let Some(filename) = filename else { return };
    let Some(first_line) = sys.first_line(filename) else { return };
    if !first_line.starts_with(b"%&") {
        return;
    }
    // Parse the first line into at most three space-separated parts.
    let mut s = &first_line[2..];
    while matches!(s.first(), Some(b' ' | b'\t')) {
        s = &s[1..];
    }
    let mut part: Vec<&[u8]> = Vec::new();
    while !s.is_empty() && part.len() != 3 {
        let e = s.iter().position(|&c| c == b' ').unwrap_or(s.len());
        part.push(&s[..e]);
        s = &s[e..];
        while s.first() == Some(&b' ') {
            s = &s[1..];
        }
    }
    let mut p = 0;
    // Look at what we've got.  Very crude!
    if p < part.len() && part[p].first() != Some(&b'-') {
        // A format name
        if o.dump_name.is_none() {
            let f_name = [part[0], DUMP_EXT].concat();
            if sys.find_fmt(&f_name).is_some() {
                o.dump_name = Some(part[0].to_vec());
                *program_name = part[0].to_vec();
                sys.set_program_name(&program_basename(&o.invocation), part[0]); // kpse_reset_program_name
                o.dump_line = true; // Tell TeX/MF/MP we have a %&name line...
            }
        }
        p += 1;
    }
    // The tcx stuff, if any.
    if p < part.len() && o.translate_filename.is_none() {
        let a = part[p];
        let next = part.get(p + 1).copied();
        let val: Option<&[u8]> = if a == b"--translate-file" || a == b"-translate-file" {
            next
        } else if a.starts_with(b"--translate-file=") {
            Some(&a[17..])
        } else if a.starts_with(b"-translate-file=") {
            Some(&a[16..])
        } else {
            None
        };
        // Just set the name, no sanity checks here.
        if let Some(v) = val {
            if !v.is_empty() {
                o.translate_filename = Some(v.to_vec());
            }
        }
    }
}

/// Puts what `maininit` decided where `main_body` finds it.
fn apply(o: &Options, i: &Init, t: &mut Tex) {
    t.ini_version = o.ini_version;
    t.etex_p = o.etex_p;
    t.halt_on_error_p = o.halt_on_error as i32;
    t.debug_format_file = o.debug_format_file;
    t.shellenabledp = i.shellenabledp as i32;
    t.restrictedshell = i.restrictedshell as i32;
    t.file_line_error_style_p = i.file_line_error_style_p as i32;
    t.parse_first_line_p = i.parse_first_line_p as i32;
    t.eight_bit_p = o.eight_bit as i32;
    t.dump_option = o.dump_option;
    t.dump_line = o.dump_line;
    t.pdf_output_option = o.pdf_output_option;
    t.pdf_output_value = o.pdf_output_value;
    t.pdf_draftmode_option = o.pdf_draftmode_option;
    t.pdf_draftmode_value = o.pdf_draftmode_value;
    t.interaction_option = o.interaction_option;
    t.synctexoption = o.synctexoption;
    t.format_default_length = i.format_default_length;
    t.host.tex_format_default = i.format_default.clone();
    t.host.command_line = i.command_line.clone();
    t.host.translate_filename = o.translate_filename.clone();
    t.host.output_comment = i.output_comment.clone();
    t.host.world.output_directory = o.output_directory.clone();
    t.host.hv.main.dump_name = i.dump_name.clone();
    t.host.c_job_name = o.c_job_name.clone();
    t.host.world.recorder_enabled = o.recorder_enabled;
    t.v_image.cd.on = !o.no_commute && !o.ini_version;
    t.v_image.cd.report = o.commute_report;
    t.v_image.cd.report_names = o.commute_report_names;
}

fn write_all(to_stderr: bool, bytes: &[u8]) {
    use std::io::Write;
    if to_stderr {
        let _ = std::io::stderr().write_all(bytes);
    } else {
        let _ = std::io::stdout().write_all(bytes);
    }
}

fn run(args: Vec<Vec<u8>>) -> i32 {
    rstex_engine::prof_engine_start!();
    let mut o = match parse_options(&args) {
        Ok(o) => o,
        Err(Stop::Print(s)) => {
            write_all(false, &s);
            return 0;
        }
        Err(Stop::Fail(s)) => {
            write_all(true, &s);
            return 1;
        }
    };
    rstex_engine::prof_phase!("options parsed");
    let init = match maininit(&mut o, &mut RealSystem) {
        Ok(i) => i,
        Err(Stop::Print(s)) => {
            write_all(false, &s);
            return 0;
        }
        Err(Stop::Fail(s)) => {
            write_all(true, &s);
            return 1;
        }
    };
    for m in &o.messages {
        write_all(true, m);
    }
    for mut m in rstex_kpse::take_warnings() {
        m.push(b'\n');
        write_all(true, &m);
    }
    // Before the key is made, since a checkpoint holds what it decides.
    o.no_commute |= platform::env_var(b"RSTEX_NO_COMMUTE").is_some();
    rstex_engine::prof_phase!("kpse init");
    let mut t = Tex::alloc();
    rstex_engine::prof_image!(t);
    rstex_engine::prof_phase!("image reserved");
    apply(&o, &init, t);
    if !o.no_snapshot && !o.ini_version && !init.command_line.is_empty() && platform::env_var(b"RSTEX_NO_SNAPSHOT").is_none() {
        // All that the command line and texmf.cnf decided is part of the key, by being in these two.
        let cli = format!("{:?}\n{:?}", o, init).into_bytes();
        if let Some(key) = t.checkpoint_key(&cli) {
            let found = t.checkpoint_try_restore(&key);
            if found == Restored::Yes {
                t.checkpoint_resume(); // ends in do_final_end, so it does not return
                return 0;
            }
            if found == Restored::ColdImageDirty {
                Tex::dealloc(t);
                t = Tex::alloc();
                rstex_engine::prof_image!(t);
                apply(&o, &init, t);
            }
            // The lookups of a checkpoint that did not hold would show in the order of the recorder's lines.
            if found != Restored::Cold && o.recorder_enabled {
                rstex_kpse::reset_global();
                if let Ok(mut again) = parse_options(&args) {
                    let _ = maininit(&mut again, &mut RealSystem);
                }
                let _ = rstex_kpse::take_warnings();
            }
            t.checkpoint_begin_recording(key);
        }
    }
    t.host.world.format_scratch = autofmt::take_scratch();
    t.main_body(); // ends in do_final_end, so it does not return
    0
}

/// What a panic says.  It ends the process then, with `panic = "abort"`, and so a core dump where there are such: nothing that the log's buffer holds gets out.
#[cfg_attr(test, allow(dead_code))]
fn panic_message(info: &std::panic::PanicHookInfo<'_>) {
    let m = format!("rstex: internal error: {info}\nThis is a bug in rstex, not in the document.\n");
    platform::write_stderr(m.as_bytes());
}

/// The C runtime's call.
#[cfg(not(test))]
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
extern "C" fn main(_argc: i32, _argv: *const *const std::ffi::c_char) -> i32 {
    rstex_engine::prof_main_start!();
    std::panic::set_hook(Box::new(panic_message));
    platform::process_init();
    let args: Vec<Vec<u8>> = std::env::args_os().map(|a| platform::path_to_bytes(std::path::Path::new(&a))).collect();
    let code = platform::run_on_stack(ENGINE_STACK, move || run(args));
    rstex_engine::prof_finish!();
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// kpathsea and the system, as a test says they are.
    #[derive(Default)]
    struct FakeSystem {
        env: HashMap<Vec<u8>, Vec<u8>>,
        cnf: HashMap<Vec<u8>, Vec<u8>>,
        tex: HashMap<Vec<u8>, Vec<u8>>,
        fmt: Vec<Vec<u8>>,
        files: HashMap<Vec<u8>, Vec<u8>>,
        windows: bool,
        put: Vec<(Vec<u8>, Vec<u8>)>,
        names: Vec<(Vec<u8>, Vec<u8>)>,
        cnf_lines: Vec<Vec<u8>>,
        own: Vec<Vec<u8>>,
    }

    impl System for FakeSystem {
        fn getenv(&mut self, name: &[u8]) -> Option<Vec<u8>> {
            self.env.get(name).cloned()
        }
        fn putenv(&mut self, name: &[u8], value: &[u8]) {
            self.put.push((name.to_vec(), value.to_vec()));
        }
        fn set_program_name(&mut self, invocation: &[u8], progname: &[u8]) {
            self.names.push((invocation.to_vec(), progname.to_vec()));
        }
        fn maketex_option(&mut self, _fmt: &[u8], _value: bool) {}
        fn cnf_line(&mut self, line: &[u8]) {
            self.cnf_lines.push(line.to_vec());
        }
        fn find_tex(&mut self, name: &[u8]) -> Option<Vec<u8>> {
            self.tex.get(name).cloned()
        }
        fn find_fmt(&mut self, name: &[u8]) -> Option<Vec<u8>> {
            self.fmt.iter().find(|f| f.as_slice() == name).cloned()
        }
        fn find_web2c(&mut self, name: &[u8]) -> Option<Vec<u8>> {
            if name == b"cp227.tcx" { Some(b"/x/web2c/cp227.tcx".to_vec()) } else { None }
        }
        fn first_line(&mut self, path: &[u8]) -> Option<Vec<u8>> {
            self.files.get(path).cloned()
        }
        fn var_value(&mut self, var: &[u8]) -> Option<Vec<u8>> {
            self.cnf.get(var).cloned()
        }
        fn is_absolute(&self, name: &[u8]) -> bool {
            self.windows && name.len() > 2 && name[1] == b':'
        }
        fn own_format(&mut self, name: &[u8]) -> Option<Vec<u8>> {
            self.own.iter().find(|f| f.as_slice() == name).map(|f| [b"/cache/", &f[..], b".fmt"].concat())
        }
    }

    fn argv(a: &[&str]) -> Vec<Vec<u8>> {
        a.iter().map(|s| s.as_bytes().to_vec()).collect()
    }

    fn parse(a: &[&str]) -> Options {
        parse_options(&argv(a)).unwrap()
    }

    fn init(a: &[&str], sys: &mut FakeSystem) -> (Options, Init) {
        let mut o = parse(a);
        let i = maininit(&mut o, sys).unwrap();
        (o, i)
    }

    fn text(m: &[Vec<u8>]) -> String {
        m.iter().map(|l| String::from_utf8_lossy(l).to_string()).collect()
    }

    #[test]
    fn one_dash_or_two_and_equals_or_a_separate_argument() {
        for a in [&["rstex", "-ini"][..], &["rstex", "--ini"]] {
            assert!(parse(a).ini_version);
        }
        for a in [
            &["rstex", "-jobname=x"][..],
            &["rstex", "--jobname=x"],
            &["rstex", "-jobname", "x"],
            &["rstex", "--jobname", "x"],
        ] {
            let o = parse(a);
            assert_eq!(o.c_job_name.as_deref(), Some(&b"x"[..]));
            assert!(o.rest.is_empty());
        }
    }

    #[test]
    fn every_flag_the_task_names() {
        let o = parse(&[
            "rstex", "-ini", "-etex", "-fmt=f", "-jobname=j", "-output-directory=d", "-interaction=batchmode", "-progname=p", "-recorder", "-shell-escape",
            "-file-line-error", "-halt-on-error", "-8bit", "-translate-file=cp227.tcx", "-output-format=dvi", "-kpathsea-debug=4095", "first",
        ]);
        assert!(o.ini_version && o.etex_p && o.dump_option && o.recorder_enabled && o.halt_on_error && o.eight_bit);
        assert_eq!(o.dump_name.as_deref(), Some(&b"f"[..]));
        assert_eq!(o.c_job_name.as_deref(), Some(&b"j"[..]));
        assert_eq!(o.output_directory.as_deref(), Some(&b"d"[..]));
        assert_eq!(o.interaction_option, BATCH_MODE);
        assert_eq!(o.user_progname.as_deref(), Some(&b"p"[..]));
        assert_eq!(o.shellenabledp, 1);
        assert_eq!(o.file_line_error_style, 1);
        assert_eq!(o.translate_filename.as_deref(), Some(&b"cp227.tcx"[..]));
        assert_eq!((o.pdf_output_option, o.pdf_output_value), (1, 0));
        assert_eq!(o.rest, argv(&["first"]));
        assert!(o.messages.is_empty(), "{}", text(&o.messages));
    }

    #[test]
    fn interaction_values() {
        for (v, want) in [("batchmode", BATCH_MODE), ("nonstopmode", NONSTOP_MODE), ("scrollmode", SCROLL_MODE), ("errorstopmode", ERROR_STOP_MODE)] {
            assert_eq!(parse(&["rstex", &format!("-interaction={}", v)]).interaction_option, want);
        }
        let o = parse(&["rstex", "-interaction=loud"]);
        assert_eq!(o.interaction_option, UNSPECIFIED_MODE);
        assert_eq!(text(&o.messages), "warning: Ignoring unknown argument `loud' to --interaction.\n");
    }

    #[test]
    fn output_format_and_draftmode() {
        let o = parse(&["rstex", "-output-format=pdf"]);
        assert_eq!((o.pdf_output_option, o.pdf_output_value), (1, 2));
        let o = parse(&["rstex", "-output-format=ps"]);
        assert_eq!(o.pdf_output_option, 0);
        assert_eq!(text(&o.messages), "warning: Ignoring unknown value `ps' for --output-format.\n");
        let o = parse(&["rstex", "-draftmode"]);
        assert_eq!((o.pdf_draftmode_option, o.pdf_draftmode_value), (1, 1));
    }

    #[test]
    fn shell_options_in_order() {
        assert_eq!(parse(&["rstex", "-shell-escape", "-no-shell-escape"]).shellenabledp, -1);
        assert_eq!(parse(&["rstex", "-no-shell-escape", "-shell-escape"]).shellenabledp, 1);
        assert_eq!(parse(&["rstex", "--enable-write18"]).shellenabledp, 1);
        assert_eq!(parse(&["rstex", "--disable-write18"]).shellenabledp, -1);
        let o = parse(&["rstex", "-shell-restricted"]);
        assert!(o.shellenabledp == 1 && o.restrictedshell);
    }

    #[test]
    fn options_stop_at_the_first_other_argument() {
        let o = parse(&["rstex", "-ini", "foo.tex", "-etex", "--version"]);
        assert!(o.ini_version && !o.etex_p);
        assert_eq!(o.rest, argv(&["foo.tex", "-etex", "--version"]));
        let o = parse(&["rstex", "--", "-ini"]);
        assert!(!o.ini_version);
        assert_eq!(o.rest, argv(&["-ini"]));
        let o = parse(&["rstex", "-"]);
        assert_eq!(o.rest, argv(&["-"]));
        let o = parse(&["rstex", "&fmt", "foo"]);
        assert_eq!(o.rest, argv(&["&fmt", "foo"]));
        let o = parse(&["rstex", "\\input foo"]);
        assert_eq!(o.rest, argv(&["\\input foo"]));
    }

    #[test]
    fn getopt_abbreviations_and_errors() {
        // A unique prefix does, and an exact name beats the longer ones it begins.
        assert_eq!(parse(&["rstex", "-inter=batchmode"]).interaction_option, BATCH_MODE);
        assert_eq!(parse(&["rstex", "--pro", "x"]).user_progname.as_deref(), Some(&b"x"[..]));
        assert!(parse(&["rstex", "-ini"]).ini_version);
        assert_eq!(parse(&["rstex", "-file-line-error"]).file_line_error_style, 1);
        let o = parse(&["rstex", "--h"]);
        // What the oracle says to --h=1, --nosuch=2, --ini=3, -jobname at the end.
        assert_eq!(text(&o.messages), "rstex: option '--h' is ambiguous; possibilities: '--help' '--halt-on-error'\n");
        let o = parse(&["rstex", "--h=1"]);
        assert_eq!(text(&o.messages), "rstex: option '--h=1' is ambiguous; possibilities: '--help' '--halt-on-error'\n");
        let o = parse(&["rstex", "--nosuch=2", "x"]);
        assert_eq!(text(&o.messages), "rstex: unrecognized option '--nosuch=2'\n");
        assert_eq!(o.rest, argv(&["x"]), "and it goes on");
        let o = parse(&["rstex", "-jobname", "-ini"]);
        assert_eq!(o.c_job_name.as_deref(), Some(&b"-ini"[..]), "the next argument, whatever it looks like");
        assert!(!o.ini_version);
        let o = parse(&["rstex", "-ini=3"]);
        assert!(!o.ini_version);
        assert_eq!(text(&o.messages), "rstex: option '-ini' doesn't allow an argument\n");
        let o = parse(&["rstex", "-jobname"]);
        assert_eq!(text(&o.messages), "rstex: option '-jobname' requires an argument\n");
    }

    #[test]
    fn no_snapshot_is_exact_and_does_not_disturb_the_table() {
        for a in ["--no-snapshot", "-no-snapshot"] {
            let o = parse(&["rstex", a, "-ini", "x"]);
            assert!(o.no_snapshot && o.ini_version && o.messages.is_empty());
            assert_eq!(o.rest, argv(&["x"]));
        }
        assert!(!parse(&["rstex", "x", "--no-snapshot"]).no_snapshot, "it is an option like the others: they stop at the name");
        assert_eq!(parse(&["rstex", "-no-s"]).shellenabledp, -1, "still a unique prefix of -no-shell-escape");
    }

    #[test]
    fn version_and_help_end_the_run_where_they_are_met() {
        for a in ["-version", "--version"] {
            match parse_options(&argv(&["rstex", a])) {
                Err(Stop::Print(s)) => assert!(s.starts_with(b"rstex "), "{}", String::from_utf8_lossy(&s)),
                r => panic!("{:?}", r),
            }
        }
        match parse_options(&argv(&["rstex", "-help"])) {
            Err(Stop::Print(s)) => assert!(s.starts_with(b"Usage: rstex")),
            r => panic!("{:?}", r),
        }
        assert!(parse_options(&argv(&["rstex", "x", "--version"])).is_ok());
    }

    #[test]
    fn jobname_quotes() {
        assert_eq!(parse(&["rstex", "-jobname=\"a b\""]).c_job_name.as_deref(), Some(&b"\"a b\""[..]));
        assert_eq!(parse(&["rstex", "-jobname=a\" \"b"]).c_job_name.as_deref(), Some(&b"\"a b\""[..]));
        assert_eq!(parse(&["rstex", "-jobname=a\"b\""]).c_job_name.as_deref(), Some(&b"ab"[..]));
        match parse_options(&argv(&["rstex", "-jobname=a\"b"])) {
            Err(Stop::Fail(s)) => assert_eq!(s, b"! Unbalanced quotes in jobname a\"b\n"),
            r => panic!("{:?}", r),
        }
    }

    #[test]
    fn long_comment_is_cut() {
        let long = "x".repeat(300);
        let o = parse(&["rstex", &format!("-output-comment={}", long)]);
        assert_eq!(o.output_comment.as_ref().unwrap().len(), 255);
        assert!(text(&o.messages).contains("Comment truncated to 255 characters from 300."));
    }

    #[test]
    fn dropped_features_are_said() {
        for a in ["-mltex", "-enc", "-ipc", "-ipc-start", "-src-specials"] {
            let o = parse(&["rstex", a]);
            assert!(text(&o.messages).contains("is not supported"), "{}", a);
        }
        // kpse_maketex_option and kpathsea_debug have nothing to set.
        assert!(parse(&["rstex", "-mktex=tfm", "-no-mktex=tex", "-kpathsea-debug=1"]).messages.is_empty());
    }

    #[test]
    fn the_program_name_gives_the_format_and_ini() {
        let mut sys = FakeSystem::default();
        let (o, i) = init(&["/usr/bin/pdflatex", "x"], &mut sys);
        assert!(!o.ini_version);
        assert_eq!(sys.names, vec![(b"pdflatex".to_vec(), b"pdflatex".to_vec())]);
        assert_eq!(i.dump_name, b"pdflatex");
        assert_eq!(i.format_default, b" pdflatex.fmt");
        assert_eq!(i.format_default_length, 12);
        for prog in ["pdfinitex", "initex"] {
            let (o, _) = init(&[prog], &mut FakeSystem::default());
            assert!(o.ini_version, "{}", prog);
        }
        for prog in ["pdfvirtex", "virtex"] {
            let (o, i) = init(&[prog], &mut FakeSystem::default());
            assert!(!o.ini_version);
            assert_eq!(i.dump_name, b"plain", "{}", prog);
        }
        // Its own name stands for pdftex.
        let (_, i) = init(&["rstex"], &mut FakeSystem::default());
        assert_eq!((i.program_name.as_slice(), i.dump_name.as_slice()), (&b"pdftex"[..], &b"pdftex"[..]));
    }

    #[test]
    fn run_trip_starts_it_this_way() {
        let mut sys = FakeSystem::default();
        let (o, i) = init(&["/x/rstex", "--progname=initex", "--ini"], &mut sys);
        assert!(o.ini_version);
        assert_eq!(i.program_name, b"initex");
        assert!(i.command_line.is_empty(), "the first line comes from stdin, at the ** prompt");
        let (o, i) = init(&["/x/rstex", "--progname=tex"], &mut FakeSystem::default());
        assert!(!o.ini_version);
        assert_eq!(i.dump_name, b"tex");
        let (o, _) = init(&["/x/rstex", "--progname=einitex", "--ini"], &mut FakeSystem::default());
        assert!(o.ini_version);
        let (o, _) = init(&["/x/rstex", "--progname=etex"], &mut FakeSystem::default());
        assert!(!o.ini_version);
    }

    #[test]
    fn progname_defaults_to_the_dump_name() {
        let mut sys = FakeSystem::default();
        let (_, i) = init(&["rstex", "-fmt=pdflatex", "x"], &mut sys);
        assert_eq!(i.program_name, b"pdflatex");
        assert_eq!(i.dump_name, b"pdflatex");
        let (o, i) = init(&["rstex", "-fmt=mine.fmt", "-progname=pdflatex", "x"], &mut sys);
        assert!(o.dump_option);
        assert_eq!(i.program_name, b"pdflatex");
        assert_eq!(i.format_default, b" mine.fmt", "an extension already there stays single");
    }

    #[test]
    fn ampersand_and_backslash_are_no_file_names() {
        let mut sys = FakeSystem::default();
        sys.tex.insert(b"foo.tex".to_vec(), b"./foo.tex".to_vec());
        // &fmt in argv[1] names the format, and stays in the first line for section 1517.
        let (_, i) = init(&["rstex", "&mine", "foo.tex"], &mut sys);
        assert_eq!(i.dump_name, b"mine");
        assert_eq!(i.command_line, b"&mine foo.tex");
        // An option before it hides it, as C looks at argv[1]; section 1517 still loads it from the line.
        let (_, i) = init(&["rstex", "-ini", "&mine", "foo.tex"], &mut sys);
        assert_eq!(i.dump_name, b"pdftex");
        assert_eq!(i.command_line, b"&mine foo.tex");
        let (_, i) = init(&["rstex", "\\input foo"], &mut sys);
        assert_eq!(i.command_line, b"\\input foo");
        // *name is a name kpathsea does not find, and section 1648 reads the star.
        let (o, i) = init(&["rstex", "-ini", "*foo"], &mut sys);
        assert!(o.ini_version);
        assert_eq!(i.command_line, b"*foo");
    }

    #[test]
    fn the_first_line_is_the_arguments_joined_and_trimmed() {
        let mut sys = FakeSystem::default();
        let (_, i) = init(&["rstex", "a b", "c d", "e \r\n "], &mut sys);
        assert_eq!(i.command_line, b"\"a b\" c d e", "a first argument is normalised; the rest is copied");
        let (_, i) = init(&["rstex", "\\relax", "b c", "d \r\n "], &mut sys);
        assert_eq!(i.command_line, b"\\relax b c d");
        let (_, i) = init(&["rstex", "\"a b\".tex"], &mut sys);
        assert_eq!(i.command_line, b"\"a b.tex\"");
    }

    #[test]
    fn a_windows_path_turns_its_backslashes() {
        let mut sys = FakeSystem { windows: true, ..Default::default() };
        let (_, i) = init(&["rstex", "C:\\a\\b.tex"], &mut sys);
        assert_eq!(i.command_line, b"C:/a/b.tex");
        let (_, i) = init(&["rstex", "C:\\a\\b.tex"], &mut FakeSystem::default());
        assert_eq!(i.command_line, b"C:\\a\\b.tex");
    }

    #[test]
    fn output_directory_and_its_variable() {
        let mut sys = FakeSystem::default();
        let (o, _) = init(&["rstex", "-output-directory=out"], &mut sys);
        assert_eq!(o.output_directory.as_deref(), Some(&b"out"[..]));
        assert_eq!(sys.put, vec![(b"TEXMF_OUTPUT_DIRECTORY".to_vec(), b"out".to_vec())]);
        let mut sys = FakeSystem::default();
        sys.env.insert(b"TEXMF_OUTPUT_DIRECTORY".to_vec(), b"env".to_vec());
        let (o, _) = init(&["rstex"], &mut sys);
        assert_eq!(o.output_directory.as_deref(), Some(&b"env"[..]));
        assert!(sys.put.is_empty());
    }

    #[test]
    fn what_comes_from_texmf_cnf() {
        let mut sys = FakeSystem::default();
        sys.cnf.insert(b"file_line_error_style".to_vec(), b"t".to_vec());
        sys.cnf.insert(b"parse_first_line".to_vec(), b"y".to_vec());
        sys.cnf.insert(b"shell_escape".to_vec(), b"p".to_vec());
        sys.cnf.insert(b"output_comment".to_vec(), b"hello".to_vec());
        let (_, i) = init(&["rstex"], &mut sys);
        assert!(i.file_line_error_style_p && i.parse_first_line_p && i.shellenabledp && i.restrictedshell);
        assert_eq!(i.output_comment.as_deref(), Some(&b"hello"[..]));
        // The command line wins.
        let (_, i) = init(&["rstex", "-no-file-line-error", "-no-parse-first-line", "-no-shell-escape", "-output-comment=c"], &mut sys);
        assert!(!i.file_line_error_style_p && !i.parse_first_line_p && !i.shellenabledp);
        assert_eq!(i.output_comment.as_deref(), Some(&b"c"[..]));
        sys.cnf.insert(b"shell_escape".to_vec(), b"t".to_vec());
        let (_, i) = init(&["rstex"], &mut sys);
        assert!(i.shellenabledp && !i.restrictedshell);
        sys.cnf.insert(b"shell_escape".to_vec(), b"f".to_vec());
        let (_, i) = init(&["rstex"], &mut sys);
        assert!(!i.shellenabledp);
    }

    #[test]
    fn a_percent_ampersand_line() {
        let mut sys = FakeSystem::default();
        sys.cnf.insert(b"parse_first_line".to_vec(), b"t".to_vec());
        sys.tex.insert(b"a.tex".to_vec(), b"./a.tex".to_vec());
        sys.fmt.push(b"latexish.fmt".to_vec());
        sys.files.insert(b"./a.tex".to_vec(), b"%&latexish --translate-file=cp227 ignored".to_vec());
        let (o, i) = init(&["rstex", "a.tex"], &mut sys);
        assert!(o.dump_line);
        assert_eq!(i.dump_name, b"latexish");
        assert_eq!(i.program_name, b"latexish");
        assert_eq!(sys.names.last().unwrap().1, b"latexish", "kpse_reset_program_name");
        assert_eq!(o.translate_filename.as_deref(), Some(&b"cp227"[..]));
        // A format that is not there: nothing changes.
        sys.files.insert(b"./a.tex".to_vec(), b"%&nonesuch".to_vec());
        let (o, i) = init(&["rstex", "a.tex"], &mut sys);
        assert!(!o.dump_line);
        assert_eq!(i.dump_name, b"pdftex");
        // -fmt was given, so only the TCX part counts.
        sys.files.insert(b"./a.tex".to_vec(), b"%&latexish -translate-file cp227".to_vec());
        let (o, i) = init(&["rstex", "-fmt=mine", "a.tex"], &mut sys);
        assert!(!o.dump_line);
        assert_eq!(i.dump_name, b"mine");
        assert_eq!(o.translate_filename.as_deref(), Some(&b"cp227"[..]));
        // Not a %& line.
        sys.files.insert(b"./a.tex".to_vec(), b"% latexish".to_vec());
        let (o, _) = init(&["rstex", "a.tex"], &mut sys);
        assert!(!o.dump_line);
        // Off by the option.
        sys.files.insert(b"./a.tex".to_vec(), b"%&latexish".to_vec());
        let (o, _) = init(&["rstex", "-no-parse-first-line", "a.tex"], &mut sys);
        assert!(!o.dump_line);
    }

    #[test]
    fn translate_files() {
        let mut sys = FakeSystem::default();
        let (o, _) = init(&["rstex", "-translate-file=cp227.tcx"], &mut sys);
        assert!(o.messages.is_empty(), "{}", text(&o.messages));
        let (o, _) = init(&["rstex", "-translate-file=cp227"], &mut sys);
        assert!(o.messages.is_empty(), "{}", text(&o.messages));
        let (o, _) = init(&["rstex", "-default-translate-file=cp227.tcx"], &mut sys);
        assert_eq!(o.translate_filename.as_deref(), Some(&b"cp227.tcx"[..]));
        let (o, _) = init(&["rstex", "-translate-file=il2-t1"], &mut sys);
        let t = text(&o.messages);
        assert!(t.contains("the TCX reader is dropped") && t.contains("Could not open char translation file `il2-t1.tcx'"), "{}", t);
    }

    /// `autofmt.rs` comes in only where the command line names no format.
    #[test]
    fn a_format_of_its_own_only_when_none_is_named() {
        let mut sys = FakeSystem { own: vec![b"pdflatex".to_vec(), b"pdftex".to_vec()], ..Default::default() };
        let (_, i) = init(&["pdflatex", "x"], &mut sys);
        assert_eq!(i.dump_name, b"pdflatex", "the banner still says this");
        assert_eq!((i.format_default.as_slice(), i.format_default_length), (&b" /cache/pdflatex.fmt"[..], 19));
        let (_, i) = init(&["rstex", "x"], &mut sys);
        assert_eq!(i.format_default, b" /cache/pdftex.fmt");
        let (_, i) = init(&["rstex", "-progname=pdflatex", "x"], &mut sys);
        assert_eq!(i.format_default, b" /cache/pdflatex.fmt");
        // A name there is no recipe for stays what it was.
        let (_, i) = init(&["rstex", "-progname=mine", "x"], &mut sys);
        assert_eq!(i.format_default, b" mine.fmt");
        // -fmt, -ini and &name keep their meanings.
        let (_, i) = init(&["rstex", "-fmt=pdflatex", "x"], &mut sys);
        assert_eq!(i.format_default, b" pdflatex.fmt");
        let (_, i) = init(&["rstex", "-ini", "x"], &mut sys);
        assert_eq!(i.format_default, b" pdftex.fmt");
        let (_, i) = init(&["pdfinitex", "x"], &mut sys);
        assert_eq!(i.format_default, b" pdfinitex.fmt");
        let (_, i) = init(&["rstex", "&pdflatex", "x"], &mut sys);
        assert_eq!(i.format_default, b" pdflatex.fmt");
        let (_, i) = init(&["rstex", "-interaction=batchmode", "&pdflatex", "x"], &mut sys);
        assert_eq!(i.format_default, b" pdftex.fmt");
        // A %& line names no format on the command line.
        sys.cnf.insert(b"parse_first_line".to_vec(), b"t".to_vec());
        sys.tex.insert(b"a.tex".to_vec(), b"./a.tex".to_vec());
        sys.fmt.push(b"pdflatex.fmt".to_vec());
        sys.files.insert(b"./a.tex".to_vec(), b"%&pdflatex".to_vec());
        let (o, i) = init(&["rstex", "a.tex"], &mut sys);
        assert!(o.dump_line);
        assert_eq!(i.format_default, b" /cache/pdflatex.fmt");
    }

    #[test]
    fn etex_without_ini_is_said() {
        let (o, _) = init(&["rstex", "-etex"], &mut FakeSystem::default());
        assert_eq!(text(&o.messages), "-etex only works with -ini\n");
    }

    #[test]
    fn cnf_lines_reach_kpathsea() {
        let mut sys = FakeSystem::default();
        init(&["rstex", "-cnf-line=a=1", "--cnf-line", "b=2"], &mut sys);
        assert_eq!(sys.cnf_lines, argv(&["a=1", "b=2"]));
    }

    /// The whole road up to the engine call: every field `main_body` reads from the command line.
    #[test]
    fn apply_fills_what_main_body_reads() {
        let mut sys = FakeSystem::default();
        let (o, i) = init(
            &[
                "rstex", "-ini", "-etex", "-fmt=f", "-jobname=j", "-output-directory=d", "-interaction=scrollmode", "-recorder", "-shell-restricted",
                "-file-line-error", "-parse-first-line", "-halt-on-error", "-8bit", "-translate-file=cp227.tcx", "-output-format=pdf", "-draftmode", "-synctex=1",
                "-output-comment=c", "-debug-format", "*x", "y",
            ],
            &mut sys,
        );
        let t = Tex::alloc();
        apply(&o, &i, t);
        assert!(t.ini_version && t.etex_p && t.dump_option && !t.dump_line && t.debug_format_file);
        assert_eq!((t.halt_on_error_p, t.eight_bit_p, t.file_line_error_style_p, t.parse_first_line_p), (1, 1, 1, 1));
        assert_eq!((t.shellenabledp, t.restrictedshell), (1, 1));
        assert_eq!((t.pdf_output_option, t.pdf_output_value, t.pdf_draftmode_option, t.pdf_draftmode_value), (1, 2, 1, 1));
        assert_eq!((t.interaction_option, t.synctexoption), (SCROLL_MODE, 1));
        assert_eq!((t.format_default_length, t.host.tex_format_default.as_slice()), (5, &b" f.fmt"[..]));
        assert_eq!(t.host.command_line, b"*x y");
        assert_eq!(t.host.translate_filename.as_deref(), Some(&b"cp227.tcx"[..]));
        assert_eq!(t.host.output_comment.as_deref(), Some(&b"c"[..]));
        assert_eq!(t.host.world.output_directory.as_deref(), Some(&b"d"[..]));
        assert_eq!(t.host.hv.main.dump_name, b"f");
        assert_eq!(t.host.c_job_name.as_deref(), Some(&b"j"[..]));
        assert!(t.host.world.recorder_enabled);
        Tex::dealloc(t);
        // Nothing said: the defaults of maininit.
        let (o, i) = init(&["rstex"], &mut FakeSystem::default());
        let t = Tex::alloc();
        apply(&o, &i, t);
        assert_eq!((t.interaction_option, t.synctexoption), (UNSPECIFIED_MODE, SYNCTEX_NO_OPTION));
        assert!(!t.ini_version && t.host.c_job_name.is_none() && !t.host.world.recorder_enabled && t.host.translate_filename.is_none());
        Tex::dealloc(t);
    }

    #[test]
    fn strtol_base_zero() {
        assert_eq!(strtol0(b"1"), 1);
        assert_eq!(strtol0(b"-1"), -1);
        assert_eq!(strtol0(b"0x10"), 16);
        assert_eq!(strtol0(b"010"), 8);
        assert_eq!(strtol0(b"12x"), 12);
        assert_eq!(strtol0(b"x"), 0);
        assert_eq!(strtol0(b""), 0);
    }
}
