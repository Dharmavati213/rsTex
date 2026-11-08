//! The environment's options and body (spec sections 3 and 5.3).  Keys of arrows, labels and cells are in keys.rs.
//!
//! The text parsed here is what follows `\begin{tikzcd}`, up to `\end{tikzcd}`.  Parsing evaluates everything it can,
//! so that layout does no text work; font parameters are therefore an input to parsing.

use crate::FontParams;
use crate::color::{self, Color};
use crate::expr;
use crate::font_sp;
use crate::keys::{CELL_LW, LOOP_MIN};
use crate::pgfmath::{PT, Sp, Xy, dec, mul};
use crate::policy::Policy;
use crate::tips::GtTip;
use crate::scan::{Span, bracket_end, brace_end, is_letter, is_ws, items, keyval, skip_ws, trim, unbrace};
use crate::spec::{
    Anchor, ArrowSpec, ArrowStyle, BaseNode, Baseline, CELL_FONT, CELL_IXS, CELL_IYS, CellOpts, CellSpec, ColSpec, Curve, LabelSpec,
    LabelXform, Opt, RowSpec, Settings, Shift, Unsupported, UserKey, UserKeys,
};

pub(crate) type R<T = ()> = Result<T, Unsupported>;

/// Style references and nested key lists deeper than this are refused.
pub(crate) const MAX_DEPTH: u32 = 16;

/// Key lists evaluated through user styles in one diagram.  A style that uses another twice, nested a few levels deep,
/// grows exponentially; real diagrams stay far below this.
pub(crate) const STYLE_BUDGET: u32 = 4096;

/// Passes over one diagram, each bringing one more piece of text into the buffer.
pub(crate) const MAX_PASSES: u32 = 64;

/// The reason on the way out of a pass that needs another, with [`ParseOut::pending`] set.
pub(crate) const AGAIN: &str = "another pass";

/// A style the document made: `.style` on `name`, in `/tikz` or in tikz-cd's own directory.  An `.append style` is one more
/// of these under the same name.
#[derive(Copy, Clone, Debug)]
pub(crate) struct UserStyle {
    pub name: Span,
    pub body: Span,
    pub tikz: bool,
}

/// A key the document defined otherwise, with its code, if that is known.
pub(crate) type UserCode = UserStyle;

/// A style's body with a value in place of `#1`, which only a pass can bring into the buffer.
#[derive(Copy, Clone, Debug)]
pub(crate) struct Instance {
    pub body: Span,
    pub value: Span,
    pub text: Span,
}

/// What the next pass needs.
#[derive(Copy, Clone, Debug)]
pub(crate) enum Pending {
    Instance { body: Span, value: Span },
    /// [`UserKeys`] knows the key: the replacement text is in [`ParseOut::asked`].
    Asked { name: Span, tikz: bool },
    /// And that it has a `.default`, there too.
    AskedDefault { name: Span },
}

/// All vectors are kept between diagrams, so a diagram after the first allocates nothing unless it is the largest yet.
pub(crate) struct ParseOut {
    pub s: Settings,
    pub rows: Vec<RowSpec>,
    pub cols: Vec<ColSpec>,
    pub cells: Vec<CellSpec>,
    pub content: Vec<Span>,
    pub arrows: Vec<ArrowSpec>,
    pub labels: Vec<LabelSpec>,
    pub shifts: Vec<Shift>,
    pub xforms: Vec<LabelXform>,
    pub opts: Vec<Opt>,
    /// What `nodes=` said in a `cells=` being read, which goes after the rest of it.
    pub node_opts: Vec<Opt>,
    pub every_arrow: Vec<Span>,
    pub every_label: Vec<Span>,
    pub styles: Vec<UserStyle>,
    /// Keys defined by `.code` and its like, which do what this crate does not know until it is told.
    pub codes: Vec<UserCode>,
    /// `key/.default=value`.
    pub defaults: Vec<(Span, Span)>,
    // These four outlive a pass, and their spans are in the part of the buffer that passes add to.
    pub instances: Vec<Instance>,
    pub asked_styles: Vec<UserStyle>,
    pub asked_codes: Vec<UserCode>,
    pub asked_defaults: Vec<(Span, Span)>,
    pub pending: Option<Pending>,
    pub asked: Vec<u8>,
    /// `row sep/<name>=<list>`: (is row, name, list).
    pub seps: Vec<(bool, Span, Span)>,
    pub diagrams: Vec<Span>,
}

impl ParseOut {
    pub(crate) fn new() -> ParseOut {
        ParseOut {
            s: default_settings(FontParams::ARTICLE_10PT),
            rows: Vec::new(),
            cols: Vec::new(),
            cells: Vec::new(),
            content: Vec::new(),
            arrows: Vec::new(),
            labels: Vec::new(),
            shifts: Vec::new(),
            xforms: Vec::new(),
            opts: Vec::new(),
            node_opts: Vec::new(),
            every_arrow: Vec::new(),
            every_label: Vec::new(),
            styles: Vec::new(),
            codes: Vec::new(),
            defaults: Vec::new(),
            instances: Vec::new(),
            asked_styles: Vec::new(),
            asked_codes: Vec::new(),
            asked_defaults: Vec::new(),
            pending: None,
            asked: Vec::new(),
            seps: Vec::new(),
            diagrams: Vec::new(),
        }
    }

    pub(crate) fn clear(&mut self, font: FontParams) {
        self.s = default_settings(font);
        self.rows.clear();
        self.cols.clear();
        self.cells.clear();
        self.content.clear();
        self.arrows.clear();
        self.labels.clear();
        self.shifts.clear();
        self.xforms.clear();
        self.opts.clear();
        self.node_opts.clear();
        self.every_arrow.clear();
        self.every_label.clear();
        self.styles.clear();
        self.codes.clear();
        self.defaults.clear();
        self.styles.extend_from_slice(&self.asked_styles);
        self.codes.extend_from_slice(&self.asked_codes);
        self.defaults.extend_from_slice(&self.asked_defaults);
        self.pending = None;
        self.seps.clear();
        self.diagrams.clear();
    }

    /// Before a new diagram: what passes over the last one brought in.
    pub(crate) fn forget(&mut self) {
        self.instances.clear();
        self.asked_styles.clear();
        self.asked_codes.clear();
        self.asked_defaults.clear();
    }

    /// Bring the text a pass asked for into the buffer.
    pub(crate) fn take_pending(&mut self, buf: &mut Vec<u8>) {
        match self.pending.take() {
            Some(Pending::Instance { body, value }) => {
                let start = buf.len();
                let (mut i, end) = (body.start as usize, body.end as usize);
                while i < end {
                    if buf[i] == b'#' {
                        buf.extend_from_within(value.start as usize..value.end as usize);
                        i += 2;
                    } else {
                        let c = buf[i];
                        buf.push(c);
                        i += 1;
                    }
                }
                self.instances.push(Instance { body, value, text: Span::new(start, buf.len()) });
            }
            Some(Pending::Asked { name, tikz }) => {
                let start = buf.len();
                buf.extend_from_slice(&self.asked);
                let end = buf.len();
                if !styles_of_cmd(buf, Span::new(start, end), name, tikz, &mut self.asked_styles) {
                    self.asked_codes.push(UserCode { name, body: Span::new(start, end), tikz });
                }
            }
            Some(Pending::AskedDefault { name }) => {
                let start = buf.len();
                buf.extend_from_slice(&self.asked);
                self.asked_defaults.push((name, Span::new(start, buf.len())));
            }
            None => {}
        }
    }
}

/// `\pgfk@.../.@cmd` of a style, which is `\pgfkeysalso{<list>}` for its `.style` and one more for each `.append style`.
/// False if the text is something else.
fn styles_of_cmd(buf: &[u8], s: Span, name: Span, tikz: bool, out: &mut Vec<UserStyle>) -> bool {
    const ALSO: &[u8] = b"\\pgfkeysalso";
    let (mut i, end) = (s.start as usize, s.end as usize);
    let at = out.len();
    loop {
        i = skip_ws(buf, i, end);
        if i >= end {
            return out.len() > at;
        }
        let j = skip_ws(buf, i + ALSO.len(), end);
        let close = if buf[i..end].starts_with(ALSO) && j < end && buf[j] == b'{' { brace_end(buf, j, end) } else { None };
        let Some(close) = close else {
            out.truncate(at);
            return false;
        };
        out.push(UserStyle { name, body: Span::new(j + 1, close), tikz });
        i = close + 1;
    }
}

/// Where a key lives.  `\tikzcdset` runs in `/tikz/commutative diagrams`, `\tikzset` in `/tikz`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum Dir {
    Cd,
    Tikz,
    /// `/pgf/...`, which PGF reads.
    Pgf,
    /// Any other tree, or a subtree of `/tikz` the document made for itself: nothing there reaches a diagram.
    Own,
}

/// A key's directory, and where its name starts, seen from directory `dir`.
pub(crate) fn place(k: &[u8], dir: Dir) -> (Dir, usize) {
    const CD: &[u8] = b"commutative diagrams/";
    const TIKZ: &[u8] = b"/tikz/";
    if k.first() == Some(&b'/') {
        if !k.starts_with(TIKZ) {
            return (if k.starts_with(b"/pgf/") { Dir::Pgf } else { Dir::Own }, if k.starts_with(b"/pgf/") { 5 } else { 0 });
        }
        return if k[TIKZ.len()..].starts_with(CD) { (Dir::Cd, TIKZ.len() + CD.len()) } else { (Dir::Tikz, TIKZ.len()) };
    }
    if dir == Dir::Tikz && k.starts_with(CD) {
        return (Dir::Cd, CD.len());
    }
    (dir, 0)
}

/// Same text but for white space, as a comparison with what a package wrote.
pub(crate) fn same_text(a: &[u8], b: &[u8]) -> bool {
    a.iter().filter(|c| !is_ws(**c)).eq(b.iter().filter(|c| !is_ws(**c)))
}

fn default_settings(f: FontParams) -> Settings {
    let ex = font_sp(f.ex);
    Settings {
        rowsep: 0,
        colsep: 0,
        row_fixed: false,
        col_fixed: false,
        cell: CellOpts {
            ixs: ex,
            iys: mul(dec(b"0.85"), ex),
            font: Span::EMPTY,
            color: None,
            nodes_color: None,
            nodes_set: 0,
            draw: false,
            draw_color: None,
            rounded: 0,
            alias: Span::EMPTY,
            text_height: None,
            text_depth: None,
            opts_base: (0, 0),
            opts_nodes: (0, 0),
            opts_own: (0, 0),
        },
        mat_inner: 0,
        baseline: Baseline::Dim(0),
        bg: Color::WHITE,
        clearance: mul(dec(b"1.5"), ex),
        math_mode: true,
        style: ArrowStyle::Default,
        cell_lw: CELL_LW,
        nodes_in_empty: false,
        amp: Span::EMPTY,
        gt: GtTip::Start,
        tips_changed: 0,
        cell_font: false,
    }
}

/// Copy `preset` and `env` into `buf` as TeX reads them: no comments, and white space one space, so that `bend⏎   left` is the
/// key it is.  Returns the length of the preset part.  An empty line is `\par`, which breaks a tikzcd in the oracle (spec
/// 3.1), so it stops here.
pub(crate) fn sanitize(preset: &[u8], env: &[u8], buf: &mut Vec<u8>) -> R<usize> {
    if preset.len().saturating_add(env.len()) >= (u32::MAX as usize) / 2 {
        return Err(Unsupported::new("source too large", Span::EMPTY));
    }
    buf.clear();
    buf.reserve(preset.len() + env.len() + 1);
    strip_comments(preset, buf)?;
    // What the preset ended in does not join what the diagram starts with.
    buf.push(b',');
    let n = buf.len();
    strip_comments(env, buf)?;
    Ok(n)
}

#[inline]
fn one_space(buf: &mut Vec<u8>) {
    if buf.last() != Some(&b' ') {
        buf.push(b' ');
    }
}

fn strip_comments(src: &[u8], buf: &mut Vec<u8>) -> R {
    let mut i = 0;
    // Whether the current source line has anything on it.  The input starts in the middle of a line.
    let mut content = true;
    // Whether the last thing copied is a control word, which a following comment must not merge with its next letter.
    let mut in_word = false;
    while i < src.len() {
        let ch = src[i];
        match ch {
            b'\\' => {
                buf.push(ch);
                content = true;
                i += 1;
                if i < src.len() {
                    buf.push(src[i]);
                    in_word = is_letter(src[i]);
                    i += 1;
                }
                continue;
            }
            b'%' => {
                while i < src.len() && src[i] != b'\n' {
                    i += 1;
                }
                i += 1;
                // The next line starts fresh: a comment swallows its own line end, so a comment-only line is no \par.
                content = false;
                if in_word {
                    buf.push(b' ');
                    in_word = false;
                }
                continue;
            }
            b'\n' => {
                if !content {
                    return Err(Unsupported::new("empty line (\\par) inside the diagram", Span::EMPTY));
                }
                content = false;
                in_word = false;
                one_space(buf);
                i += 1;
                continue;
            }
            _ if is_ws(ch) => {
                in_word = false;
                one_space(buf);
                i += 1;
                continue;
            }
            _ => {
                content = true;
                in_word = in_word && is_letter(ch);
            }
        }
        buf.push(ch);
        i += 1;
    }
    Ok(())
}

pub(crate) struct Parser<'a> {
    pub buf: &'a [u8],
    pub o: &'a mut ParseOut,
    pub font: FontParams,
    pub policy: &'a dyn Policy,
    pub keys: &'a dyn UserKeys,
    pub style_uses: u32,
    /// Where the diagram's own text ends: passes add after it.
    pub env_end: usize,
    /// `.cd` of the key list being read.
    pub dir: Dir,
    pub in_quiver: bool,
    /// Inside a style made in `/tikz`: its keys are `/tikz`'s, as tikz-cd's are out of its reach (spec 5.1).
    pub in_tikz: bool,
    /// Reading `cells=` itself, not its `nodes=`: what is said here is not about a node.
    pub in_every_cell: bool,
    /// The fields `CELL_*` that the cell keys read so far have set, for what `nodes=` sets.
    pub cell_written: u8,
}

#[derive(Copy, Clone, PartialEq)]
enum Stage {
    Preset,
    Env,
}

/// Where the body scan is inside the current cell.
struct CellScan {
    r: u32,
    c: u32,
    depth: i64,
    envs: [Span; 8],
    nenv: usize,
    tokens: bool,
    run: Option<(usize, usize)>,
}

impl Parser<'_> {
    pub(crate) fn run(&mut self, preset_len: usize) -> R {
        let buf = self.buf;
        let preset = Span::new(0, preset_len);
        for item in items(buf, preset) {
            self.env_item(item, Stage::Preset, 0)?;
        }
        self.dir = Dir::Cd;
        // `every diagram` runs at the start of the picture: after anything set before it, before the options.
        let normal: &[u8] = b"normal";
        let (v, fx) = self.named_sep(true, normal, Span::EMPTY)?;
        self.o.s.rowsep = v;
        self.o.s.row_fixed = fx;
        let (v, fx) = self.named_sep(false, normal, Span::EMPTY)?;
        self.o.s.colsep = v;
        self.o.s.col_fixed = fx;
        let mut i = 0;
        while i < self.o.diagrams.len() {
            let sp = self.o.diagrams[i];
            self.dir = Dir::Cd;
            for item in items(buf, sp) {
                self.env_item(item, Stage::Env, 0)?;
            }
            i += 1;
        }

        let end = self.env_end;
        let mut body = skip_ws(buf, preset_len, end);
        if body < end && buf[body] == b'[' {
            let close = bracket_end(buf, body, end).ok_or(Unsupported::new("unterminated option list", Span::new(body, end)))?;
            self.dir = Dir::Cd;
            for item in items(buf, Span::new(body + 1, close)) {
                self.env_item(item, Stage::Env, 0)?;
            }
            body = close + 1;
        }
        self.body(body, end)
    }

    // ------------------------------------------------------------------------------------------- evaluation helpers

    pub(crate) fn dim(&self, s: Span, lw: Sp) -> R<Sp> {
        Ok(self.value(s, lw)?.v)
    }

    pub(crate) fn value(&self, s: Span, lw: Sp) -> R<expr::Value> {
        let ctx = expr::Ctx { font: self.font, lw };
        expr::eval(self.buf, s, &ctx).ok_or(Unsupported::new("expression outside the supported subset of pgfmath", s))
    }

    pub(crate) fn color(&self, s: Span) -> R<Color> {
        let c = s.get(self.buf);
        // The document's meaning of a name comes first: a base colour can be redefined too.
        if let Some(known) = self.keys.color(c) {
            return Ok(known);
        }
        if !self.keys.base_colors() {
            return Err(Unsupported::new("colour computed from what the document has changed", s));
        }
        color::parse(c).ok_or(Unsupported::new("colour not known without xcolor", s))
    }

    /// `(x,y)`, optionally braced.
    pub(crate) fn coordinate(&self, s: Span, lw: Sp) -> R<Xy> {
        let bad = Unsupported::new("expected a coordinate (x,y)", s);
        let t = unbrace(self.buf, s);
        let b = t.get(self.buf);
        if b.len() < 2 || b[0] != b'(' || b[b.len() - 1] != b')' {
            return Err(bad);
        }
        let inner = Span::new(t.start as usize + 1, t.end as usize - 1);
        let mut it = items(self.buf, inner);
        let (x, y) = (it.next().ok_or(bad)?, it.next().ok_or(bad)?);
        if it.next().is_some() {
            return Err(bad);
        }
        Ok((self.dim(x, lw)?, self.dim(y, lw)?))
    }

    /// A list of `between origins`, `between borders` and lengths added to `val` (spec 3.1).
    fn sep_list(&self, list: Span, mut val: Sp, mut fixed: bool) -> R<(Sp, bool)> {
        for it in items(self.buf, list) {
            match it.get(self.buf) {
                b"between origins" => fixed = true,
                b"between borders" => fixed = false,
                _ => val += self.dim(it, CELL_LW)?,
            }
        }
        Ok((val, fixed))
    }

    /// The value of `row sep=<v>` or `column sep=<v>`: a name if there is one, else a list.
    fn named_sep(&self, row: bool, v: &[u8], vspan: Span) -> R<(Sp, bool)> {
        for &(is_row, name, list) in self.o.seps.iter().rev() {
            if is_row == row && name.get(self.buf) == v {
                return self.sep_list(list, 0, false);
            }
        }
        const TABLE: [(&[u8], Sp, Sp); 6] = [
            (b"huge", dec(b"3.6"), dec(b"4.8")),
            (b"large", dec(b"2.7"), dec(b"3.6")),
            (b"normal", dec(b"1.8"), dec(b"2.4")),
            (b"scriptsize", dec(b"1.35"), dec(b"1.8")),
            (b"small", dec(b"0.9"), dec(b"1.2")),
            (b"tiny", dec(b"0.45"), dec(b"0.6")),
        ];
        for (name, r, c) in TABLE {
            if name == v {
                return Ok((mul(if row { r } else { c }, font_sp(self.font.em)), false));
            }
        }
        self.sep_list(vspan, 0, false)
    }

    // ------------------------------------------------------------------------------------------------ environment keys

    fn env_item(&mut self, item: Span, stage: Stage, depth: u32) -> R {
        if depth > MAX_DEPTH {
            return Err(Unsupported::new("key lists nested too deeply", item));
        }
        let buf = self.buf;
        let (ks, vs) = keyval(buf, item);
        let k = ks.get(buf);
        if k.first() == Some(&b'"') {
            return Err(Unsupported::new("label in the diagram's options", item));
        }

        // `<key>/.style={...}` and friends.
        if let Some(pos) = find_handler(k) {
            return self.handler(ks, pos, vs, stage, depth);
        }
        let (dir, off) = place(k, self.dir);
        let ks = Span::new(ks.start as usize + off, ks.end as usize);
        let k = ks.get(buf);
        match dir {
            // quiver's `curve` reads its directory for what it is not given.
            Dir::Own if self.in_quiver => return Err(Unsupported::new("a default of quiver changed", ks)),
            // A key real PGF looks for in a directory of the document's, which may not have it: an error, which `\tikzset` has given before the diagram.
            Dir::Own if stage == Stage::Env => return Err(Unsupported::new("key in another directory", ks)),
            Dir::Own => return Ok(()),
            Dir::Pgf => return Err(Unsupported::new("key under /pgf", ks)),
            Dir::Tikz | Dir::Cd => {}
        }
        if let Some(v) = self.user_style(k, dir == Dir::Tikz) {
            return self.apply_style_env(v, ks, vs, stage, depth);
        }
        if self.user_code(k, dir == Dir::Tikz).is_some() {
            return Err(Unsupported::new("key defined by code", ks));
        }
        self.ask(ks)?;
        if dir == Dir::Tikz {
            return self.tikz_option(ks, vs);
        }

        // `row sep/<name>=...`
        for (prefix, is_row) in [(&b"row sep/"[..], true), (&b"column sep/"[..], false)] {
            if k.len() > prefix.len() && k.starts_with(prefix) {
                let name = Span::new(ks.start as usize + prefix.len(), ks.end as usize);
                let val = vs.ok_or(Unsupported::new("named separation without a value", ks))?;
                // tikz-cd has six names, and `.initial` adds to them; without that it is an error in the oracle (spec 5.3).
                let known = self.o.seps.iter().any(|&(r, n, _)| r == is_row && n.get(buf) == name.get(buf));
                if !known && !matches!(name.get(buf), b"huge" | b"large" | b"normal" | b"scriptsize" | b"small" | b"tiny") {
                    return Err(Unsupported::new("separation name tikz-cd does not define", ks));
                }
                self.o.seps.push((is_row, name, val));
                return Ok(());
            }
        }

        match k {
            b"row sep" | b"column sep" | b"sep" => {
                if stage == Stage::Preset {
                    // Overwritten by `every diagram` in the oracle; refuse rather than guess what was meant.
                    return Err(Unsupported::new("separation set outside the diagram", ks));
                }
                let v = vs.ok_or(Unsupported::new("separation without a value", ks))?;
                if k != b"column sep" {
                    let (a, b) = self.named_sep(true, v.get(buf), v)?;
                    self.o.s.rowsep = a;
                    self.o.s.row_fixed = b;
                }
                if k != b"row sep" {
                    let (a, b) = self.named_sep(false, v.get(buf), v)?;
                    self.o.s.colsep = a;
                    self.o.s.col_fixed = b;
                }
            }
            b"cramped" => {
                let t = mul(dec(b"0.3"), font_sp(self.font.em));
                self.o.s.mat_inner = -t;
                // `every cell/.append style={inner sep=+0.3em}`, against which what a `nodes=` there said holds.
                if self.o.s.cell.nodes_set & CELL_IXS == 0 {
                    self.o.s.cell.ixs = t;
                }
                if self.o.s.cell.nodes_set & CELL_IYS == 0 {
                    self.o.s.cell.iys = t;
                }
            }
            b"arrows" => self.push_every(true, vs, ks)?,
            b"labels" => self.push_every(false, vs, ks)?,
            b"cells" => self.cell_list_on_base(vs, ks)?,
            b"diagrams" => {
                let v = vs.ok_or(Unsupported::new("diagrams= without a value", ks))?;
                if stage != Stage::Preset {
                    return Err(Unsupported::new("diagrams= inside the diagram's options", ks));
                }
                self.o.diagrams.push(v);
            }
            b"ampersand replacement" => {
                let v = vs.ok_or(Unsupported::new("ampersand replacement without a value", ks))?;
                let b = v.get(buf);
                if b.first() != Some(&b'\\') {
                    return Err(Unsupported::new("ampersand replacement is not a control sequence", v));
                }
                self.o.s.amp = v;
            }
            b"nodes in empty cells" => self.o.s.nodes_in_empty = !is_false(vs, buf),
            // `/tikz/font` for the picture, which a cell has unless `every cell` sets one, as that runs in the cell's scope.
            b"font" => {
                if !self.o.s.cell_font {
                    self.o.s.cell.font = vs.unwrap_or(Span::EMPTY);
                }
            }
            // Every point of an arrow goes through it and back, with what that rounds, and it scales what the arrow's keys say.
            b"scale" => {
                let v = vs.ok_or(Unsupported::new("scale without a value", ks))?;
                if self.dim(v, CELL_LW)? != PT {
                    return Err(Unsupported::new("scale on the diagram", ks));
                }
            }
            b"line width" => {
                let v = vs.ok_or(Unsupported::new("line width without a value", ks))?;
                self.o.s.cell_lw = self.dim(v, CELL_LW)?;
            }
            b"math mode" => self.o.s.math_mode = !is_false(vs, buf),
            b"background color" => {
                let v = vs.ok_or(Unsupported::new("background color without a value", ks))?;
                self.o.s.bg = self.color(v)?;
            }
            b"crossing over clearance" => {
                let v = vs.ok_or(Unsupported::new("clearance without a value", ks))?;
                self.o.s.clearance = self.dim(v, CELL_LW)?;
            }
            b"baseline" => {
                let v = vs.ok_or(Unsupported::new("baseline without a value", ks))?;
                self.o.s.baseline = self.baseline(v)?;
            }
            b"arrow style" => {
                let style = match vs.map(|v| v.get(buf)) {
                    Some(b"Latin Modern") => ArrowStyle::LatinModern,
                    Some(b"tikz") => ArrowStyle::Tikz,
                    _ => return Err(Unsupported::new("arrow style other than Latin Modern and tikz", item)),
                };
                // Each style sets some of the tips and leaves the others as the last one did.
                if self.o.s.style != ArrowStyle::Default && self.o.s.style != style {
                    return Err(Unsupported::new("a second arrow style", item));
                }
                self.o.s.style = style;
            }
            _ => return self.tikz_option(ks, vs),
        }
        Ok(())
    }

    /// A key that reaches `/tikz` from a diagram's options or from `\tikzset`: it stays in force for what follows.
    fn tikz_option(&mut self, ks: Span, vs: Option<Span>) -> R {
        match ks.get(self.buf) {
            // What arrows it reaches is seen when they are made: `->` and all of `arrow style=tikz`.
            b">" => {
                let v = vs.ok_or(Unsupported::new("> without a value", ks))?;
                self.o.s.gt = match v.get(self.buf) {
                    b"stealth" => GtTip::Stealth,
                    b"latex" => GtTip::Latex,
                    _ => GtTip::Other,
                };
            }
            // Of use with active characters, which TeX has resolved in a cell by then or breaks with.
            b"handle active characters in code" | b"handle active characters in nodes" => {}
            _ => return Err(Unsupported::new("unknown diagram option", ks)),
        }
        Ok(())
    }

    fn push_every(&mut self, arrow: bool, vs: Option<Span>, ks: Span) -> R {
        let v = vs.ok_or(Unsupported::new("key list without a value", ks))?;
        if arrow { self.o.every_arrow.push(v) } else { self.o.every_label.push(v) }
        Ok(())
    }

    fn cell_list_on_base(&mut self, vs: Option<Span>, ks: Span) -> R {
        let v = vs.ok_or(Unsupported::new("key list without a value", ks))?;
        let mut base = self.o.s.cell;
        let at = self.o.opts.len() as u32;
        self.in_every_cell = true;
        self.cell_written = 0;
        self.cell_list(&mut base, v, 0)?;
        self.in_every_cell = false;
        if self.cell_written & CELL_FONT != 0 {
            self.o.s.cell_font = true;
        }
        // Nothing else adds to the list while the options run, but what the last `cells=` had of nodes would stand between.
        if base.opts_nodes.0 != base.opts_nodes.1 && (at as usize != self.o.opts.len() || !self.o.node_opts.is_empty()) {
            return Err(Unsupported::new("colours of nodes in one cells= and more in another", ks));
        }
        base.opts_base = (if base.opts_base.0 == base.opts_base.1 { at } else { base.opts_base.0 }, self.o.opts.len() as u32);
        if !self.o.node_opts.is_empty() {
            let start = self.o.opts.len() as u32;
            let ParseOut { opts, node_opts, .. } = &mut *self.o;
            opts.append(node_opts);
            base.opts_nodes = (start, self.o.opts.len() as u32);
        }
        self.o.s.cell = base;
        Ok(())
    }

    /// `<key>/.<handler>`: `pos` is the index of the slash.
    fn handler(&mut self, ks: Span, pos: usize, vs: Option<Span>, stage: Stage, depth: u32) -> R {
        let buf = self.buf;
        let k = ks.get(buf);
        let handler = &k[pos + 1..];
        let (dir, off) = place(k, self.dir);
        if handler == b".cd" {
            if depth > 0 {
                return Err(Unsupported::new(".cd in a style", ks));
            }
            self.dir = if pos < off { dir } else { Dir::Own };
            self.in_quiver = k == b"quiver/.cd" || k == b"/tikz/quiver/.cd";
            return Ok(());
        }
        if handler == b".tip" {
            let name = &k[off.min(pos)..pos];
            self.o.s.tips_changed |= crate::keys::tips_of_name(name);
            return Ok(());
        }
        match dir {
            Dir::Own => return Ok(()),
            Dir::Pgf => return Err(Unsupported::new("key under /pgf", ks)),
            Dir::Tikz | Dir::Cd => {}
        }
        let tikz = dir == Dir::Tikz;
        if pos < off {
            return Err(Unsupported::new("key handler on a directory", ks));
        }
        let name = &k[off..pos];
        let key = Span::new(ks.start as usize + off, ks.start as usize + pos);
        if tikz {
            return self.tikz_handler(ks, key, handler, vs);
        }
        if !matches!(handler, b".style" | b".append style") {
            return self.other_handler(ks, key, handler, vs, false);
        }
        let val = vs.ok_or(Unsupported::new("key handler without a value", ks))?;
        match (name, handler) {
            (b"every arrow", b".append style") => self.push_every(true, vs, ks),
            (b"every label", b".append style") => self.push_every(false, vs, ks),
            (b"every cell", b".append style") => self.cell_list_on_base(vs, ks),
            (b"every matrix", b".append style") => {
                for it in items(buf, val) {
                    let (k2, v2) = keyval(buf, it);
                    match (k2.get(buf), v2) {
                        (b"inner sep", Some(v2)) => self.o.s.mat_inner = self.dim(v2, CELL_LW)?,
                        _ => return Err(Unsupported::new("every matrix key other than inner sep", it)),
                    }
                }
                Ok(())
            }
            (b"every diagram", b".append style") => {
                if stage != Stage::Preset {
                    return Err(Unsupported::new("every diagram inside the diagram's options", ks));
                }
                self.o.diagrams.push(val);
                Ok(())
            }
            (b"every arrow" | b"every label" | b"every cell" | b"every matrix" | b"every diagram", _) => {
                Err(Unsupported::new("tikz-cd's own styles are modified", ks))
            }
            (_, b".style") => {
                if name.ends_with(b"-row diagram") {
                    return Err(Unsupported::new("N-row diagram style", ks));
                }
                if crate::keys::is_builtin_key(name) {
                    return Err(Unsupported::new("tikz-cd's own styles are modified", ks));
                }
                self.define_style(key, val, false);
                Ok(())
            }
            (_, _) => {
                if self.user_style(name, false).is_none() {
                    self.ask(key)?;
                    return Err(Unsupported::new("append to a style that is not a user style", ks));
                }
                self.o.styles.push(UserStyle { name: key, body: val, tikz: false });
                Ok(())
            }
        }
    }

    /// A redefinition replaces what came before.
    fn define_style(&mut self, key: Span, body: Span, tikz: bool) {
        let buf = self.buf;
        self.o.styles.retain(|d| d.tikz != tikz || d.name.get(buf) != key.get(buf));
        self.o.codes.retain(|d| d.tikz != tikz || d.name.get(buf) != key.get(buf));
        self.o.styles.push(UserStyle { name: key, body, tikz });
    }

    /// A handler in `/tikz`.  Harmless until the key is used, except on the styles TikZ runs by itself.
    fn tikz_handler(&mut self, ks: Span, key: Span, handler: &[u8], vs: Option<Span>) -> R {
        let name = key.get(self.buf);
        // What a diagram goes through: its picture, matrix, cells, nodes and paths.
        let runs = matches!(
            name,
            b"every picture"
                | b"every scope"
                | b"every node"
                | b"every text node part"
                | b"every rectangle node"
                | b"every asymmetrical rectangle node"
                | b"every coordinate node"
                | b"every path"
                | b"every to"
                | b"every curve to"
                | b"every matrix"
                | b"every cell"
        );
        if runs || name.starts_with(b"column ") || name.starts_with(b"row ") {
            return Err(Unsupported::new("a style TikZ runs by itself", ks));
        }
        if handler == b".append style" && self.user_style(name, true).is_none() {
            self.ask(key)?;
        }
        match handler {
            b".style" => {
                let v = vs.ok_or(Unsupported::new("key handler without a value", ks))?;
                self.define_style(key, v, true);
                Ok(())
            }
            b".append style" if self.user_style(name, true).is_some() => {
                let v = vs.ok_or(Unsupported::new("key handler without a value", ks))?;
                self.o.styles.push(UserStyle { name: key, body: v, tikz: true });
                Ok(())
            }
            _ => self.other_handler(ks, key, handler, vs, true),
        }
    }

    /// A handler that defines a key or adds to one, which is known only by how it is written.
    fn other_handler(&mut self, ks: Span, key: Span, handler: &[u8], vs: Option<Span>, tikz: bool) -> R {
        let buf = self.buf;
        let name = key.get(buf);
        if !tikz && crate::keys::is_builtin_key(name) {
            return Err(Unsupported::new("tikz-cd's own styles are modified", ks));
        }
        match handler {
            b".default" => {
                let v = vs.ok_or(Unsupported::new("key handler without a value", ks))?;
                self.o.defaults.push((key, v));
            }
            b".initial" if !tikz && (name.starts_with(b"row sep/") || name.starts_with(b"column sep/")) => {
                let v = vs.ok_or(Unsupported::new("key handler without a value", ks))?;
                let is_row = name.starts_with(b"row sep/");
                let start = key.start as usize + if is_row { 8 } else { 11 };
                self.o.seps.push((is_row, Span::new(start, key.end as usize), v));
            }
            b".code" | b".ecode" | b".code 2 args" | b".code n args" | b".code args" | b".ecode 2 args" | b".ecode n args" | b".ecode args"
            | b".style 2 args" | b".style n args" | b".style args" | b".estyle" | b".estyle 2 args" | b".estyle n args" | b".estyle args"
            | b".store in" | b".estore in" | b".is choice" | b".is if" | b".is family" | b".forward to" | b".value required" | b".value forbidden" | b".initial"
            | b".pic" | b".search also" | b".add code" | b".prefix code" | b".append code" | b".add style" | b".add" | b".prefix" | b".append" | b".prefix style" | b".append style" => {
                let body = vs.unwrap_or(Span::EMPTY);
                self.o.codes.push(UserCode { name: key, body, tikz });
            }
            _ => return Err(Unsupported::new("key handler other than those that define a key", ks)),
        }
        Ok(())
    }

    /// Count one use of a user style.
    pub(crate) fn spend_style(&mut self, at: Span) -> R {
        self.style_uses += 1;
        if self.style_uses > STYLE_BUDGET {
            return Err(Unsupported::new("user styles expand too far", at));
        }
        Ok(())
    }

    /// The first definition of a user style, if `k` is one.  tikz-cd's directory is searched first, unless the reader is in
    /// `/tikz` to begin with; a key tikz-cd defines hides what `/tikz` has by that name (spec 5.1).
    pub(crate) fn user_style(&self, k: &[u8], tikz: bool) -> Option<usize> {
        if self.o.styles.is_empty() {
            return None;
        }
        let mut in_tikz = None;
        for (i, d) in self.o.styles.iter().enumerate() {
            if d.name.get(self.buf) == k {
                if d.tikz == tikz {
                    return Some(i);
                }
                if d.tikz && in_tikz.is_none() {
                    in_tikz = Some(i);
                }
            }
        }
        in_tikz.filter(|_| !crate::keys::is_builtin_key(k))
    }

    /// The last definition of a key made by code, if `k` is one.
    pub(crate) fn user_code(&self, k: &[u8], tikz: bool) -> Option<UserCode> {
        if self.o.codes.is_empty() {
            return None;
        }
        let d = self.o.codes.iter().rev().find(|d| d.name.get(self.buf) == k && (d.tikz == tikz || d.tikz))?;
        if d.tikz && !tikz && crate::keys::is_builtin_key(k) { None } else { Some(*d) }
    }

    /// If the document has the key, what it is takes a pass.
    #[inline]
    pub(crate) fn ask(&mut self, ks: Span) -> R {
        // Asked already, and it is not for where this is read: `\tikzcdset{s/.style=...}` and `|[s]|`, which is `/tikz/s`.
        let name = ks.get(self.buf);
        if self.o.asked_styles.iter().chain(&self.o.asked_codes).any(|d| d.name.get(self.buf) == name) {
            return Ok(());
        }
        self.o.asked.clear();
        let tikz = match self.keys.user_key(ks.get(self.buf), &mut self.o.asked) {
            UserKey::Standard => return Ok(()),
            UserKey::Cd => false,
            UserKey::Tikz => true,
        };
        self.o.pending = Some(Pending::Asked { name: ks, tikz });
        Err(Unsupported::new(AGAIN, ks))
    }

    /// The key list of definition `i`, given `vs` as the key's value.
    pub(crate) fn style_text(&mut self, i: usize, vs: Option<Span>, at: Span) -> R<Span> {
        let buf = self.buf;
        let d = self.o.styles[i];
        let b = d.body.get(buf);
        if !b.contains(&b'#') {
            return Ok(d.body);
        }
        // `##` and a second parameter come from another form of definition.
        let mut j = 0;
        while let Some(off) = b[j..].iter().position(|&c| c == b'#') {
            j += off;
            if b.get(j + 1) != Some(&b'1') {
                return Err(Unsupported::new("style with a parameter", at));
            }
            j += 2;
        }
        let name = d.name.get(buf);
        let value = vs.or_else(|| self.o.defaults.iter().rev().find(|(n, _)| n.get(buf) == name).map(|&(_, v)| v));
        let Some(value) = value else {
            self.o.asked.clear();
            if self.keys.default_value(name, &mut self.o.asked) {
                self.o.pending = Some(Pending::AskedDefault { name: d.name });
                return Err(Unsupported::new(AGAIN, at));
            }
            return Err(Unsupported::new("style with a parameter and no value", at));
        };
        let made = self.o.instances.iter().find(|n| n.body == d.body && n.value.get(buf) == value.get(buf));
        if let Some(n) = made {
            return Ok(n.text);
        }
        self.o.pending = Some(Pending::Instance { body: d.body, value });
        Err(Unsupported::new(AGAIN, at))
    }

    fn apply_style_env(&mut self, first: usize, at: Span, vs: Option<Span>, stage: Stage, depth: u32) -> R {
        self.spend_style(at)?;
        let buf = self.buf;
        let UserStyle { name, tikz, .. } = self.o.styles[first];
        let dir = self.dir;
        if tikz {
            self.dir = Dir::Tikz;
        }
        let mut i = first;
        while i < self.o.styles.len() {
            let d = self.o.styles[i];
            if d.tikz == tikz && d.name.get(buf) == name.get(buf) {
                let list = self.style_text(i, vs, at)?;
                for it in items(buf, list) {
                    self.env_item(it, stage, depth + 1)?;
                }
            }
            i += 1;
        }
        self.dir = dir;
        Ok(())
    }

    fn baseline(&self, v: Span) -> R<Baseline> {
        let buf = self.buf;
        let t = trim(buf, v);
        let b = t.get(buf);
        if b.first() != Some(&b'(') {
            return Ok(Baseline::Dim(self.dim(t, CELL_LW)?));
        }
        let bad = Unsupported::new("baseline coordinate outside the supported forms", v);
        if b.last() != Some(&b')') {
            return Err(bad);
        }
        let mut inner = trim(buf, Span::new(t.start as usize + 1, t.end as usize - 1));
        let mut yshift = 0;
        if inner.get(buf).first() == Some(&b'[') {
            let close = bracket_end(buf, inner.start as usize, inner.end as usize).ok_or(bad)?;
            for it in items(buf, Span::new(inner.start as usize + 1, close)) {
                let (k, val) = keyval(buf, it);
                match (k.get(buf), val) {
                    (b"yshift", Some(val)) => yshift += self.dim(val, CELL_LW)?,
                    _ => return Err(bad),
                }
            }
            inner = trim(buf, Span::new(close + 1, inner.end as usize));
        }
        let i = inner.get(buf);
        let dot = i.iter().rposition(|&c| c == b'.').ok_or(bad)?;
        let name = trim(buf, Span::new(inner.start as usize, inner.start as usize + dot));
        let anchor_span = Span::new(inner.start as usize + dot + 1, inner.end as usize);
        let anchor = crate::keys::parse_anchor(self, anchor_span).ok_or(bad)?;
        let n = name.get(buf);
        let node = if n == b"current bounding box" {
            BaseNode::BoundingBox
        } else if n == b"\\tikzcdmatrixname" {
            BaseNode::Matrix
        } else if let Some(rest) = n.strip_prefix(b"\\tikzcdmatrixname-") {
            let (r, c) = crate::keys::parse_rc(rest).ok_or(bad)?;
            BaseNode::Cell(r, c)
        } else {
            BaseNode::Name(name)
        };
        if node == BaseNode::BoundingBox
            && !matches!(
                anchor,
                Anchor::Center
                    | Anchor::North
                    | Anchor::South
                    | Anchor::East
                    | Anchor::West
                    | Anchor::NorthEast
                    | Anchor::NorthWest
                    | Anchor::SouthEast
                    | Anchor::SouthWest
            )
        {
            return Err(bad);
        }
        Ok(Baseline::Node { node, anchor, yshift })
    }

    // ------------------------------------------------------------------------------------------------------ the body

    fn body(&mut self, start: usize, end: usize) -> R {
        let buf = self.buf;
        let amp = self.o.s.amp;
        let amp_text = amp.get(buf);
        let math_mode = self.o.s.math_mode;

        let mut st = CellScan { r: 0, c: 0, depth: 0, envs: [Span::EMPTY; 8], nenv: 0, tokens: false, run: None };
        self.o.rows.push(RowSpec { first_cell: 0, ncells: 0, sep: 0, fixed: false });
        self.o.cols.push(ColSpec { sep: 0, fixed: false, created_row: 0 });
        let mut i = self.begin_cell(&mut st, start, end)?;

        while i < end {
            let ch = buf[i];
            match ch {
                b'\\' => {
                    let Some(&next) = buf.get(i + 1).filter(|_| i + 1 < end) else {
                        return Err(Unsupported::new("a backslash at the end of the diagram", Span::new(i, end)));
                    };
                    if is_letter(next) {
                        let mut j = i + 1;
                        while j < end && is_letter(buf[j]) {
                            j += 1;
                        }
                        let name = &buf[i + 1..j];
                        if !amp_text.is_empty() && &buf[i..j] == amp_text {
                            i = self.next_cell(&mut st, Span::new(i, j), j, end)?;
                            continue;
                        }
                        match name {
                            b"arrow" | b"ar" => i = self.arrow_cmd(&mut st, Span::new(i, j), j, end, None)?,
                            b"rar" => i = self.arrow_cmd(&mut st, Span::new(i, j), j, end, Some((0, 1)))?,
                            b"lar" => i = self.arrow_cmd(&mut st, Span::new(i, j), j, end, Some((0, -1)))?,
                            b"dar" => i = self.arrow_cmd(&mut st, Span::new(i, j), j, end, Some((1, 0)))?,
                            b"uar" => i = self.arrow_cmd(&mut st, Span::new(i, j), j, end, Some((-1, 0)))?,
                            b"urar" => i = self.arrow_cmd(&mut st, Span::new(i, j), j, end, Some((-1, 1)))?,
                            b"ular" => i = self.arrow_cmd(&mut st, Span::new(i, j), j, end, Some((-1, -1)))?,
                            b"drar" => i = self.arrow_cmd(&mut st, Span::new(i, j), j, end, Some((1, 1)))?,
                            b"dlar" => i = self.arrow_cmd(&mut st, Span::new(i, j), j, end, Some((1, -1)))?,
                            b"begin" => i = self.env_begin(&mut st, i, j, end)?,
                            b"end" => i = self.env_end(&mut st, i, j, end)?,
                            b"par" => return Err(Unsupported::new("\\par inside the diagram", Span::new(i, j))),
                            _ => {
                                if !self.policy.control_sequence(name) {
                                    return Err(Unsupported::new("control sequence that may hide structure", Span::new(i, j)));
                                }
                                mark(&mut st, i, j);
                                i = j;
                            }
                        }
                    } else {
                        let j = i + 2;
                        if !amp_text.is_empty() && &buf[i..j] == amp_text {
                            i = self.next_cell(&mut st, Span::new(i, j), j, end)?;
                        } else if next == b'\\' && st.depth == 0 && st.nenv == 0 {
                            i = self.next_row(&mut st, j, end)?;
                        } else if matches!(next, b'[' | b']' | b'(' | b')') {
                            return Err(Unsupported::new("math shift inside the diagram", Span::new(i, j)));
                        } else {
                            mark(&mut st, i, j);
                            i = j;
                        }
                    }
                }
                b'{' => {
                    st.depth += 1;
                    mark(&mut st, i, i + 1);
                    i += 1;
                }
                b'}' => {
                    st.depth -= 1;
                    if st.depth < 0 {
                        return Err(Unsupported::new("unbalanced braces", Span::new(i, i + 1)));
                    }
                    mark(&mut st, i, i + 1);
                    i += 1;
                }
                b'&' => {
                    if st.nenv > 0 {
                        mark(&mut st, i, i + 1);
                        i += 1;
                    } else if !amp_text.is_empty() {
                        return Err(Unsupported::new("& with an ampersand replacement in force", Span::new(i, i + 1)));
                    } else {
                        i = self.next_cell(&mut st, Span::new(i, i + 1), i + 1, end)?;
                    }
                }
                b'$' if math_mode && st.depth == 0 => {
                    return Err(Unsupported::new("$ in a math cell", Span::new(i, i + 1)));
                }
                b'#' => return Err(Unsupported::new("# in a cell", Span::new(i, i + 1))),
                _ if is_ws(ch) => i += 1,
                _ => {
                    mark(&mut st, i, i + 1);
                    i += 1;
                }
            }
        }
        if st.depth != 0 {
            return Err(Unsupported::new("unbalanced braces", Span::new(start, end)));
        }
        if st.nenv != 0 {
            return Err(Unsupported::new("unclosed environment", st.envs[0]));
        }
        self.end_cell(&mut st);
        Ok(())
    }

    /// Open cell `st.c` of row `st.r`.  Returns where its text starts, past any `|[...]|`.
    fn begin_cell(&mut self, st: &mut CellScan, i: usize, end: usize) -> R<usize> {
        let buf = self.buf;
        st.tokens = false;
        st.run = None;
        let mut opts = self.o.s.cell;
        opts.color = opts.nodes_color.or(opts.color);
        let mut next = i;
        let j = skip_ws(buf, i, end);
        if j < end && buf[j] == b'|' {
            let bad = Unsupported::new("cell starting with | but not |[...]|", Span::new(j, j + 1));
            let k = skip_ws(buf, j + 1, end);
            if k >= end || buf[k] != b'[' {
                return Err(bad);
            }
            let close = bracket_end(buf, k, end).ok_or(bad)?;
            let m = skip_ws(buf, close + 1, end);
            if m >= end || buf[m] != b'|' {
                return Err(bad);
            }
            let at = self.o.opts.len() as u32;
            // `\pgfmatrixnextcell`'s node options, which are read in `/tikz`.
            self.in_tikz = true;
            self.cell_list(&mut opts, Span::new(k + 1, close), 0)?;
            self.in_tikz = false;
            opts.opts_own = (at, self.o.opts.len() as u32);
            st.tokens = true;
            next = m + 1;
        }
        let at = self.o.content.len() as u32;
        self.o.cells.push(CellSpec { row: st.r, col: st.c, has_node: false, content: (at, at), opts });
        Ok(next)
    }

    fn flush_run(&mut self, st: &mut CellScan) {
        if let Some((a, b)) = st.run.take() {
            self.o.content.push(Span::new(a, b));
            if let Some(cell) = self.o.cells.last_mut() {
                cell.content.1 = self.o.content.len() as u32;
            }
        }
    }

    fn end_cell(&mut self, st: &mut CellScan) {
        self.flush_run(st);
        let nodes_in_empty = self.o.s.nodes_in_empty;
        if let Some(cell) = self.o.cells.last_mut() {
            cell.has_node = st.tokens || nodes_in_empty;
        }
        if let Some(row) = self.o.rows.last_mut() {
            row.ncells += 1;
        }
    }

    /// At `&` or its replacement; `after` is just past it.
    fn next_cell(&mut self, st: &mut CellScan, at: Span, after: usize, end: usize) -> R<usize> {
        if st.depth != 0 || st.nenv != 0 {
            return Err(Unsupported::new("cell separator inside a group", at));
        }
        self.end_cell(st);
        let buf = self.buf;
        let c = st.c as usize + 1;
        let mut i = after;
        let j = skip_ws(buf, i, end);
        let extras = if j < end && buf[j] == b'[' {
            let close = bracket_end(buf, j, end).ok_or(Unsupported::new("unterminated &[", Span::new(j, end)))?;
            i = close + 1;
            Some(unbrace(buf, Span::new(j + 1, close)))
        } else {
            None
        };
        // Only the first time this column comes into being (spec 3.1).
        if c >= self.o.cols.len() {
            let (colsep, col_fixed) = (self.o.s.colsep, self.o.s.col_fixed);
            let (sep, fixed) = match extras {
                Some(list) => self.sep_list(list, colsep, col_fixed)?,
                None => (colsep, col_fixed),
            };
            self.o.cols.push(ColSpec { sep, fixed, created_row: st.r });
        }
        st.c += 1;
        self.begin_cell(st, i, end)
    }

    /// At `\\`; `after` is just past it.
    fn next_row(&mut self, st: &mut CellScan, after: usize, end: usize) -> R<usize> {
        self.end_cell(st);
        let buf = self.buf;
        let mut i = after;
        let j = skip_ws(buf, i, end);
        let (rowsep, row_fixed) = (self.o.s.rowsep, self.o.s.row_fixed);
        let (sep, fixed) = if j < end && buf[j] == b'[' {
            let close = bracket_end(buf, j, end).ok_or(Unsupported::new("unterminated \\\\[", Span::new(j, end)))?;
            i = close + 1;
            self.sep_list(unbrace(buf, Span::new(j + 1, close)), rowsep, row_fixed)?
        } else if j < end && buf[j] == b'*' {
            return Err(Unsupported::new("\\\\*", Span::new(j, j + 1)));
        } else {
            (rowsep, row_fixed)
        };
        if let Some(row) = self.o.rows.last_mut() {
            row.sep = sep;
            row.fixed = fixed;
        }
        let first_cell = self.o.cells.len() as u32;
        self.o.rows.push(RowSpec { first_cell, ncells: 0, sep: 0, fixed: false });
        st.r += 1;
        st.c = 0;
        self.begin_cell(st, i, end)
    }

    fn env_name(&self, i: usize, j: usize, end: usize) -> R<Span> {
        let buf = self.buf;
        let bad = Unsupported::new("\\begin or \\end without a name", Span::new(i, j));
        if j >= end || buf[j] != b'{' {
            return Err(bad);
        }
        let close = brace_end(buf, j, end).ok_or(bad)?;
        Ok(Span::new(j + 1, close))
    }

    fn env_begin(&mut self, st: &mut CellScan, i: usize, j: usize, end: usize) -> R<usize> {
        let name = self.env_name(i, j, end)?;
        let n = name.get(self.buf);
        if n == b"tikzcd" {
            return Err(Unsupported::new("nested tikzcd", name));
        }
        if !self.policy.environment(n) {
            return Err(Unsupported::new("environment that may hide structure", name));
        }
        if st.nenv == st.envs.len() {
            return Err(Unsupported::new("environments nested too deeply", name));
        }
        st.envs[st.nenv] = name;
        st.nenv += 1;
        let close = name.end as usize + 1;
        mark(st, i, close);
        Ok(close)
    }

    fn env_end(&mut self, st: &mut CellScan, i: usize, j: usize, end: usize) -> R<usize> {
        let name = self.env_name(i, j, end)?;
        let open = st.nenv.checked_sub(1).map(|n| st.envs[n]);
        if open.map(|o| o.get(self.buf)) != Some(name.get(self.buf)) {
            return Err(Unsupported::new("\\end without its \\begin", name));
        }
        st.nenv -= 1;
        let close = name.end as usize + 1;
        mark(st, i, close);
        Ok(close)
    }

    /// `\arrow`, `\ar` or a shortcut; `after` is just past the name (spec 3.3).
    fn arrow_cmd(&mut self, st: &mut CellScan, at: Span, after: usize, end: usize, dir: Option<(i64, i64)>) -> R<usize> {
        if st.depth != 0 || st.nenv != 0 {
            return Err(Unsupported::new("arrow command inside a group", at));
        }
        self.flush_run(st);
        st.tokens = true;
        let buf = self.buf;

        let mut k = skip_ws(buf, after, end);
        let mut opts = Span::EMPTY;
        let had_bracket = k < end && buf[k] == b'[';
        if had_bracket {
            let close = bracket_end(buf, k, end).ok_or(Unsupported::new("unterminated arrow options", at))?;
            opts = Span::new(k + 1, close);
            k = skip_ws(buf, close + 1, end);
        }
        // Old syntax: the argument after the bracket, if there is one in braces, or the shortcut's direction.
        let target = if dir.is_some() {
            None
        } else if k < end && buf[k] == b'{' {
            let close = brace_end(buf, k, end).ok_or(Unsupported::new("unbalanced arrow target", at))?;
            let t = Span::new(k + 1, close);
            k = close + 1;
            Some(t)
        } else if had_bracket {
            return self.finish_arrow(st, opts, None, None, Span::new(after, after), k);
        } else {
            return Err(Unsupported::new("\\arrow without [ or {", at));
        };

        // Positional labels: [opts]{text} or {text}, repeated.
        let labels_start = k;
        let mut j = k;
        loop {
            let m = skip_ws(buf, j, end);
            if m >= end {
                break;
            }
            if buf[m] == b'[' {
                let close = bracket_end(buf, m, end).ok_or(Unsupported::new("unterminated label options", at))?;
                let n = skip_ws(buf, close + 1, end);
                if n >= end || buf[n] != b'{' {
                    return Err(Unsupported::new("positional label options without a label in braces", Span::new(m, close + 1)));
                }
                j = brace_end(buf, n, end).ok_or(Unsupported::new("unbalanced label", at))? + 1;
            } else if buf[m] == b'{' {
                j = brace_end(buf, m, end).ok_or(Unsupported::new("unbalanced label", at))? + 1;
            } else {
                break;
            }
        }
        self.finish_arrow(st, opts, target, dir, Span::new(labels_start, j), j)
    }

    /// Evaluate the arrow's keys: `every arrow`, then `to={target}`, then the options, then the labels.
    fn finish_arrow(
        &mut self,
        st: &CellScan,
        opts: Span,
        target: Option<Span>,
        dir: Option<(i64, i64)>,
        positional: Span,
        next: usize,
    ) -> R<usize> {
        let buf = self.buf;
        let mut a = crate::keys::new_arrow(self, st.r, st.c);
        let shifts_at = self.o.shifts.len() as u32;
        let opts_at = self.o.opts.len() as u32;
        // `every arrow`: `/tikz/line width=rule_thickness`.
        self.o.opts.push(Opt::Width(crate::geom::pt(a.lw)));
        let labels_at = self.o.labels.len() as u32;
        let mut i = 0;
        while i < self.o.every_arrow.len() {
            let sp = self.o.every_arrow[i];
            for it in items(buf, sp) {
                self.arrow_item(&mut a, it, 0)?;
            }
            i += 1;
        }
        if let Some((dr, dc)) = dir {
            a.end = crate::spec::End::Rel(dr, dc);
        }
        if let Some(t) = target {
            a.end = crate::keys::end_of(buf, trim(buf, t));
        }
        for it in items(buf, opts) {
            self.arrow_item(&mut a, it, 0)?;
        }
        // Positional labels.
        let (mut j, stop) = (positional.start as usize, positional.end as usize);
        loop {
            let m = skip_ws(buf, j, stop);
            if m >= stop {
                break;
            }
            let mut lo = Span::EMPTY;
            let mut n = m;
            if buf[m] == b'[' {
                let close = bracket_end(buf, m, stop).ok_or(Unsupported::new("unterminated label options", positional))?;
                lo = Span::new(m + 1, close);
                n = skip_ws(buf, close + 1, stop);
            }
            let close = brace_end(buf, n, stop).ok_or(Unsupported::new("unbalanced label", positional))?;
            self.push_raw_label(unbrace(buf, Span::new(n + 1, close)), false, lo)?;
            j = close + 1;
        }
        a.shifts = (shifts_at, self.o.shifts.len() as u32);
        a.opts = (opts_at, self.o.opts.len() as u32);
        a.labels = (labels_at, self.o.labels.len() as u32);
        if a.double_tikz.is_some() && !a.double_seen && !a.double {
            return Err(Unsupported::new("double distance without double", opts));
        }
        if a.draw {
            let s = &self.o.s;
            if s.gt == GtTip::Other && (s.style == ArrowStyle::Tikz || matches!(a.tips[0], crate::spec::TipName::PlainTo) || matches!(a.tips[1], crate::spec::TipName::PlainTo)) {
                return Err(Unsupported::new("an arrow tip set by >", opts));
            }
            if s.tips_changed & (crate::keys::tip_bit(a.tips[0]) | crate::keys::tip_bit(a.tips[1])) != 0 {
                return Err(Unsupported::new("a tip redefined by the document", opts));
            }
        }
        // `loop`'s `to path` holds these, so they run after every key (spec 8.2).
        if a.is_loop {
            let cv = a.curve.get_or_insert(Curve::DEFAULT);
            cv.l_out = 8 * PT;
            cv.l_in = 8 * PT;
            cv.min_out = LOOP_MIN;
            cv.min_in = LOOP_MIN;
        }
        self.finish_labels(&a)?;
        self.o.arrows.push(a);
        Ok(next)
    }
}

/// A token at `a..b` belongs to the cell's text.
#[inline]
fn mark(st: &mut CellScan, a: usize, b: usize) {
    st.tokens = true;
    st.run = match st.run {
        Some((s, _)) => Some((s, b)),
        None => Some((a, b)),
    };
}

fn is_false(v: Option<Span>, buf: &[u8]) -> bool {
    v.map(|v| v.get(buf)) == Some(b"false")
}

/// The slash of `<key>/.<handler>`, if the key has that form.
fn find_handler(k: &[u8]) -> Option<usize> {
    k.windows(2).rposition(|w| w == b"/.")
}

/// A `\tikzcd`-free check of the text of a label: it goes to TeX in a box, so only what would act on the diagram matters.
pub(crate) fn check_label_text(buf: &[u8], s: Span, policy: &dyn Policy) -> R {
    let (mut i, end) = (s.start as usize, (s.end as usize).min(buf.len()));
    while i < end {
        match buf[i] {
            b'\\' => {
                let mut j = i + 1;
                while j < end && is_letter(buf[j]) {
                    j += 1;
                }
                if j == i + 1 {
                    i += 2;
                    continue;
                }
                let name = &buf[i + 1..j];
                match name {
                    b"begin" => {
                        let bad = Unsupported::new("\\begin without a name in a label", Span::new(i, j));
                        if j >= end || buf[j] != b'{' {
                            return Err(bad);
                        }
                        let close = brace_end(buf, j, end).ok_or(bad)?;
                        let env = &buf[j + 1..close];
                        if env == b"tikzcd" || !policy.environment(env) {
                            return Err(Unsupported::new("environment that may hide structure", Span::new(j + 1, close)));
                        }
                        i = close + 1;
                        continue;
                    }
                    b"end" => {}
                    _ => {
                        if !policy.control_sequence(name) {
                            return Err(Unsupported::new("control sequence that may hide structure", Span::new(i, j)));
                        }
                    }
                }
                i = j;
            }
            b'#' => return Err(Unsupported::new("# in a label", Span::new(i, i + 1))),
            _ => i += 1,
        }
    }
    Ok(())
}
