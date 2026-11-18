//! Looking a name up: pathsearch.c, elt-dirs.c, and the file-finding part of tex-file.c.
use crate::db::readable_file;
use crate::expand::cnf_p;
use crate::format::{self, DefaultPath, FormatDef};
use crate::names::*;
use crate::{FormatState, Kpse, sys};
use std::collections::HashMap;
use std::sync::Arc;

/// One element of a search path: what `kpathsea_path_element` returns, with `!!` taken off and
/// normalized, as `path_search` does before using it.
#[derive(Clone, Debug)]
pub(crate) struct Elt {
    pub(crate) text: Vec<u8>,
    /// False for a `!!` element: the database only, never the disk.
    pub(crate) disk: bool,
}

/// One directory of an element's list: `str_llist_elt_type`.
#[derive(Clone, Debug)]
pub(crate) struct DirElt {
    pub(crate) dir: Vec<u8>,
    moved: bool,
    /// Its number in [`Known`].
    known: u32,
}

/// Splits a search path.  kpathsea does this on every lookup; here it is done once per path.
pub(crate) fn split_elts(path: &[u8]) -> Vec<Elt> {
    path_elements(path)
        .into_iter()
        .map(|e| {
            let (e, disk) = match e.strip_prefix(b"!!") {
                Some(rest) => (rest, false),
                None => (e, true),
            };
            let mut text = e.to_vec();
            normalize_path(&mut text);
            Elt { text, disk }
        })
        .collect()
}

/// `str_llist_float`: moves the directory where a file was found towards the top, after the others
/// moved so far.  The next directory to look at stays the same, as the C remembers it beforehand.
fn float(dirs: &mut [DirElt], mover: usize) {
    if dirs[mover].moved {
        return;
    }
    let unmoved = dirs.iter().position(|d| !d.moved).unwrap_or(mover);
    dirs[unmoved..=mover].rotate_right(1);
    dirs[unmoved].moved = true;
}

/// `dir_list_add`: every directory in a list ends in a separator.
fn dir_list_add(out: &mut Vec<DirElt>, dir: &[u8], known: &mut Known) {
    let mut d = dir.to_vec();
    let last = at(dir, dir.len().wrapping_sub(1));
    if !(is_dir_sep_ch(last) || is_device_sep_g::<DOSISH>(last)) {
        d.push(b'/');
    }
    let known = known.id(&d);
    out.push(DirElt { dir: d, moved: false, known });
}

/// `expand_elt`.  `leaf` is the value of `texmf_nlink_for_leaf`.  `scans` counts directories opened.
fn expand_elt(out: &mut Vec<DirElt>, elt: &[u8], start: usize, leaf: bool, scans: &mut u64, known: &mut Known) {
    let mut dir = start;
    while at(elt, dir) != 0 {
        if is_dir_sep_ch(at(elt, dir)) && is_dir_sep_ch(at(elt, dir + 1)) {
            // Two or more consecutive /'s: find subdirectories.
            let mut post = dir + 1;
            while is_dir_sep_ch(at(elt, post)) {
                post += 1;
            }
            do_subdir(out, elt, dir + 1, &elt[post..], leaf, scans, known, None);
            return;
        }
        dir += 1;
    }
    // At the end of elt it is a normal file name.
    if sys::is_dir(elt) {
        dir_list_add(out, elt, known);
    }
}

/// `do_subdir`.  Appends the subdirectories of `elt[..elt_length]`, which ends in a /.  `post` is
/// what follows the doubled /.  C stats every entry; the directory entry's type makes that needless.
///
/// What the directory holds goes to `known` on the way, for the lookups to come, and comes from there if it is known by the stamp the directory has now: `tex///`
/// has all of `tex/latex//` in it again.  `stamp` is the directory's from before it was opened, if the caller has it: the one `stat` that C makes for the count of
/// links gives it.
#[allow(clippy::too_many_arguments)]
fn do_subdir(out: &mut Vec<DirElt>, elt: &[u8], elt_length: usize, post: &[u8], leaf: bool, scans: &mut u64, known: &mut Known, stamp: Option<sys::Stamp>) {
    let mut name = elt[..elt_length].to_vec();
    let learning = known.learns();
    let id = if learning { known.id(&name) } else { 0 };
    let stamp = match stamp {
        Some(s) => s,
        None if learning => sys::file_stamp(&stat_path(&name)),
        None => [0; 5],
    };
    let have = learning && known.listed_by(id, stamp);
    let mut whole = true;
    let list = if have {
        known.take_list(id)
    } else {
        *scans += 1;
        let Ok(rd) = sys::read_dir(&name) else { return };
        let mut list = Listing::default();
        for e in rd {
            match e {
                Ok(e) => list.push(&sys::os_bytes(&e.file_name()), sys::entry_kind(&e)),
                Err(_) => whole = false,
            }
        }
        list
    };
    // Include the top level before subdirectories, if nothing to match.
    if post.is_empty() {
        dir_list_add(out, &name, known);
    } else {
        // If there is something to match, see if it exists here.
        name.extend_from_slice(post);
        expand_elt(out, &name, elt_length, leaf, scans, known);
        name.truncate(elt_length);
    }
    for (file, kind) in list.entries() {
        // A name beginning with `.` allows hidden directories.
        if file.first() == Some(&b'.') || kind == sys::ENTRY_OTHER {
            continue;
        }
        name.extend_from_slice(file);
        if kind == sys::ENTRY_ASK && !sys::is_dir(&name) {
            name.truncate(elt_length);
            continue;
        }
        name.push(b'/');
        let potential_len = name.len();
        let (child, links) = if !leaf {
            (None, 0)
        } else if learning {
            // Elements overlap, and in one moment a directory is as it was the last time it was looked at.
            let child = known.id(&name);
            let seen = known.seen_now(child).unwrap_or_else(|| {
                let seen = sys::dir_stamp_nlink(&name[..potential_len - 1]);
                known.saw(child, seen);
                seen
            });
            (Some(seen.0), seen.1)
        } else {
            let (s, n) = sys::dir_stamp_nlink(&name[..potential_len - 1]);
            (Some(s), n)
        };
        if !post.is_empty() {
            name.extend_from_slice(post);
            expand_elt(out, &name, potential_len, leaf, scans, known);
            name.truncate(potential_len);
        }
        // A directory with two links is a leaf: no . and .. in any subdirectory.
        if !leaf || links != 2 {
            do_subdir(out, &name, potential_len, post, leaf, scans, known, child);
        } else if post.is_empty() {
            dir_list_add(out, &name, known);
        }
        name.truncate(elt_length);
    }
    if have {
        known.put_list(id, list);
    } else if learning && whole {
        known.learn(id, stamp, list);
    }
}

/// How old a directory's stamp has to be for a list to be kept by it: a younger one might not show a change made just after it, in the same tick of the file system's clock.  A stamp in whole
/// seconds may come from a clock as coarse as `index.rs` allows for.  One with a fraction comes from a clock that moves at least every 10 ms, and this is twice that.  The next pass of a document
/// comes moments after the last one put its PDF in the directory, and 2 s would have it list that directory for every miss, 397 times on one paper of the corpus.
fn listed_racy_ns(changed: u64) -> u64 {
    if changed % 1_000_000_000 == 0 { 2_000_000_000 } else { 20_000_000 }
}

/// What a directory held when it was last listed, for the next miss: a run looks in `.` for each file it reads, and the thesis came to 512 listings, a third of its system calls.  It is good
/// while the directory's stamp stays, and it is kept only if that stamp is old enough to show a later change, which also covers what the run itself creates there, or a process it starts.
pub(crate) struct ListedDir {
    stamp: sys::Stamp,
    names: Vec<Vec<u8>>,
}

/// The names a directory held, in the order it gave them, with what `sys::entry_kind` said of each.
#[derive(Default)]
pub(crate) struct Listing {
    pub(crate) text: Vec<u8>,
    pub(crate) ends: Vec<u32>,
    pub(crate) kinds: Vec<u8>,
    /// The hash of each name in lower case, in order, once `finish` has run; nothing if a name is not plain.
    pub(crate) folds: Vec<u64>,
    /// A name that is not ASCII, or too long: then the list cannot say that a name is not there.  See `plain_component`.
    pub(crate) odd: bool,
}

/// The hash a name has in a [`Listing`], whatever the case of its letters.
fn fold_hash(name: &[u8]) -> u64 {
    let mut low = [0u8; NAME_LIMIT];
    let low = &mut low[..name.len()];
    low.copy_from_slice(name);
    low.make_ascii_lowercase();
    crate::cachefile::hash64(low)
}

impl Listing {
    fn push(&mut self, name: &[u8], kind: u8) {
        self.text.extend_from_slice(name);
        self.ends.push(self.text.len() as u32);
        self.kinds.push(kind);
        if name.len() > NAME_LIMIT || !name.is_ascii() {
            self.odd = true;
        } else if !self.odd {
            self.folds.push(fold_hash(name));
        }
    }

    fn finish(&mut self) {
        if self.odd {
            self.folds = Vec::new();
        }
        self.folds.sort_unstable();
    }

    fn entries(&self) -> impl Iterator<Item = (&[u8], u8)> {
        let mut from = 0;
        self.ends.iter().zip(&self.kinds).map(move |(&to, &kind)| {
            let n = &self.text[from..to as usize];
            from = to as usize;
            (n, kind)
        })
    }
}

/// readable.c's `NAME_MAX`: a longer component is another matter there.
const NAME_LIMIT: usize = 255;

/// The hash of `name`'s first component in lower case, if a [`Listing`] of the directory it would be in can say that it is not there.  That is so for a
/// name of ASCII letters and digits and `_ - + .` that does not end in a dot, where every name of the directory is ASCII too: then no file system resolves it to an
/// entry that differs by more than the case of its letters.  One that folds case does that much, and one that knows Unicode takes other characters for letters;
/// trailing dots and blanks, `~` and `:` mean something to those of Windows.  Any other name is asked of the disk, as before.
fn plain_component(name: &[u8]) -> Option<u64> {
    let first = &name[..name.iter().position(|&c| c == b'/').unwrap_or(name.len())];
    if first.is_empty() || first.len() > NAME_LIMIT || first.ends_with(b".") || !first.iter().all(|&c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'+' | b'.')) {
        return None;
    }
    Some(fold_hash(first))
}

/// What goes to `stat` for a directory of a list, which ends in a separator there: `xdirname` of a file in it, which is how `casefold_readable_file` asks.
fn stat_path(dir: &[u8]) -> Vec<u8> {
    xdirname(&[dir, b"x"].concat())
}

/// One directory of [`Known`].
struct KnownDir {
    /// As `stat_path` gives it.
    path: Vec<u8>,
    /// What the list goes by, or what refused one.
    stamp: sys::Stamp,
    /// The moment `stamp` was last compared with the disk's.
    instant: u64,
    /// `list` is what the directory holds while its stamp is `stamp`.
    kept: bool,
    /// No list by `stamp`: the directory is not there or cannot be read.
    refused: bool,
    /// Until then the stamp is too young to keep a list by, and nothing is asked: `listed_racy_ns`.
    young_until: u64,
    /// Changes with `list`.
    version: u64,
    list: Listing,
    /// What `dirstore` has of this directory, from another run.
    stored: Option<crate::dirstore::Stored>,
    /// `list` is what `dirstore` has, or the directory is one it has been told not to keep a list of.
    from_store: bool,
    /// The stamp and the count of links that `do_subdir` got in the moment `seen_instant`.
    seen: (sys::Stamp, u64),
    seen_instant: u64,
    /// How often `usable` has asked for the stamp by the path.
    path_looks: u32,
    /// The directory held open, for its stamp: [`Known::stamp_now`].
    held: Option<Held>,
}

/// A directory held open, with what says that its path still leads to it: it is the entry of its name in the directory `parent`, as that one's list of
/// `parent_version` has it, and that entry is a directory and no link.
struct Held {
    dir: sys::HeldDir,
    parent: u32,
    parent_version: u64,
}

/// What the directories of the lists on the disk hold.  A personal tree is a path element with no `!!`, so C looks into every directory of it, with a `stat` each, for each
/// name that the databases do not have and that must exist, and LaTeX asks for some dozens of configuration files that nobody has: with 150 directories in `~/texmf`
/// that was 21000 of a run's 25000 system calls, and half of its time.  Here a name is first looked for in the list of each directory, which is good
/// while the directory's stamp stands, under the rule of [`ListedDir`], and only one that a list has, in any case, is asked of the disk.  So a miss costs one `stat` for
/// each directory, not one or two for each directory and name, and none for a directory that was looked at in the same moment: a moment is one search, both passes of
/// it, or as long as a caller holds a [`crate::OneMoment`].
#[derive(Default)]
pub(crate) struct Known {
    dirs: Vec<KnownDir>,
    by_path: HashMap<Vec<u8>, u32>,
    /// The moment that is now: a stamp compared in it needs no second look.
    pub(crate) instant: u64,
    /// The last `version` given out.
    versions: u64,
    /// The clock, read once in `clock_instant`.
    clock: u64,
    clock_instant: u64,
    /// No lists: every name is asked of the disk, as C asks.
    pub(crate) off: bool,
    /// The file of `dirstore`, if lists are kept from one run to another, and what it held when this run first wanted to know.
    store_path: Option<std::path::PathBuf>,
    store: Option<crate::dirstore::Store>,
    store_opened: bool,
    /// How old a stamp has to be for its list to be kept for another run.
    pub(crate) settled_ns: u64,
    /// How many directories are held open, and how many may be: `None` until one is wanted.
    pub(crate) held: usize,
    held_limit: Option<usize>,
    /// No directory is held: every stamp is asked by the path.
    pub(crate) no_hold: bool,
    /// The table of mounts, watched from before the first directory was held, and the moment it was last asked.
    mounts: Option<sys::MountWatch>,
    mounts_instant: u64,
}

/// The union of the lists of an element's directories, for a miss to cost one look whatever their number.
#[derive(Default)]
pub(crate) struct ListFilter {
    /// When `ready` was worked out.
    instant: u64,
    /// Every directory has a list that is good in `instant`.
    ready: bool,
    /// The sum of the directories' versions that `folds` was made from.
    versions: u64,
    folds: Vec<u64>,
}

/// Below this many directories a filter saves nothing over asking each.
const FILTER_FROM: usize = 4;

impl Known {
    /// `shared` for an instance that goes by the process's environment: it keeps its lists in rstex's cache directory, as such an instance does its index.
    pub(crate) fn new(shared: bool) -> Known {
        let off = sys::getenv(b"RSTEX_KPSE_NO_LISTS").is_some();
        let store_path = if shared && !off && cfg!(target_os = "linux") { sys::cache_dir().map(|d| d.join(crate::dirstore::FILE)) } else { None };
        let no_hold = sys::getenv(b"RSTEX_KPSE_NO_HOLD").is_some();
        Known { instant: 1, off, store_path, settled_ns: crate::dirstore::SETTLED_NS, no_hold, ..Default::default() }
    }

    pub(crate) fn set_store(&mut self, file: Option<std::path::PathBuf>) {
        self.store_path = file;
        self.store = None;
        self.store_opened = false;
    }

    fn store(&mut self) -> Option<&crate::dirstore::Store> {
        if !self.store_opened {
            self.store_opened = true;
            self.store = self.store_path.as_deref().and_then(crate::dirstore::Store::open);
        }
        self.store.as_ref()
    }

    /// Whether lists are kept at all: not on Windows, where a name finds a file in more ways than a list of names can tell, and not while an index is being made, when
    /// every path looked at is noted with its stamp.
    fn learns(&self) -> bool {
        cfg!(unix) && !self.off && !sys::observing()
    }

    /// The number of `dir`, which ends in a separator.
    fn id(&mut self, dir: &[u8]) -> u32 {
        if let Some(&id) = self.by_path.get(dir) {
            return id;
        }
        let id = self.dirs.len() as u32;
        let path = stat_path(dir);
        let stored = if path.first() == Some(&b'/') && self.learns() { self.store().and_then(|s| s.find(&path)) } else { None };
        self.dirs.push(KnownDir {
            path,
            stamp: [0; 5],
            instant: 0,
            kept: false,
            refused: false,
            young_until: 0,
            version: 0,
            list: Listing::default(),
            stored,
            from_store: false,
            seen: ([0; 5], 0),
            seen_instant: 0,
            path_looks: 0,
            held: None,
        });
        self.by_path.insert(dir.to_vec(), id);
        id
    }

    /// The stamp and the links of a directory, if `do_subdir` has had them in this moment.
    fn seen_now(&self, id: u32) -> Option<(sys::Stamp, u64)> {
        let d = &self.dirs[id as usize];
        (d.seen_instant == self.instant).then_some(d.seen)
    }

    fn saw(&mut self, id: u32, seen: (sys::Stamp, u64)) {
        let d = &mut self.dirs[id as usize];
        d.seen = seen;
        d.seen_instant = self.instant;
    }

    /// Takes the list `dirstore` has for this directory, if it goes by `stamp`, which is the directory's now.
    fn load_stored(&mut self, id: u32, stamp: sys::Stamp) -> bool {
        let Some(s) = self.dirs[id as usize].stored.filter(|s| !s.elsewhere && s.stamp == stamp && stamp[0] != 0) else { return false };
        let Some(list) = self.store.as_ref().and_then(|store| store.listing(&s)) else {
            self.dirs[id as usize].stored = None;
            return false;
        };
        self.versions += 1;
        let d = &mut self.dirs[id as usize];
        d.list = list;
        d.stamp = stamp;
        d.instant = self.instant;
        d.kept = true;
        d.refused = false;
        d.young_until = 0;
        d.version = self.versions;
        d.from_store = true;
        true
    }

    /// Writes `dirstore`'s file again if this run has a list to add to it.
    pub(crate) fn save(&mut self) {
        let Some(to) = self.store_path.clone() else { return };
        let now = sys::now_ns();
        let settled_ns = self.settled_ns;
        let settled = |d: &KnownDir| d.kept && d.path.first() == Some(&b'/') && d.stamp[2].max(d.stamp[3]).saturating_add(settled_ns) <= now;
        let news = |d: &KnownDir| settled(d) && !d.from_store;
        if !self.dirs.iter().any(news) {
            return;
        }
        let mut fresh = Vec::new();
        for d in self.dirs.iter_mut().filter(|d| news(d)) {
            let list = crate::dirstore::place_is_trusted(&d.path).then_some(&d.list);
            fresh.push((d.path.clone(), crate::dirstore::record(&d.path, &d.stamp, list)));
            // Said once: a second call, as when the instance is given up and the process ends, has nothing to add.
            d.from_store = true;
        }
        // What the file has of a directory this run looked at and found changed is of no use to anyone.
        let gone: std::collections::HashSet<&[u8]> = self.dirs.iter().filter(|d| d.stored.is_some() && d.instant != 0 && !d.from_store).map(|d| &d.path[..]).collect();
        let replaced: std::collections::HashSet<&[u8]> = fresh.iter().map(|f| &f.0[..]).collect();
        if !self.store_opened {
            self.store_opened = true;
            self.store = crate::dirstore::Store::open(&to);
        }
        let drop = |p: &[u8]| gone.contains(p) || replaced.contains(p);
        crate::dirstore::publish(&to, &fresh, self.store.as_ref(), &drop);
    }

    fn now(&mut self) -> u64 {
        if self.clock_instant != self.instant {
            self.clock = sys::now_ns();
            self.clock_instant = self.instant;
        }
        self.clock
    }

    /// True if the list of this directory goes by `stamp`, which is the directory's now.
    fn listed_by(&mut self, id: u32, stamp: sys::Stamp) -> bool {
        let d = &mut self.dirs[id as usize];
        if d.kept && d.stamp == stamp {
            d.instant = self.instant;
            return true;
        }
        self.load_stored(id, stamp)
    }

    /// The list, for `do_subdir` to go through while it adds to `self`, and back.
    fn take_list(&mut self, id: u32) -> Listing {
        std::mem::take(&mut self.dirs[id as usize].list)
    }

    fn put_list(&mut self, id: u32, list: Listing) {
        self.dirs[id as usize].list = list;
    }

    /// What `do_subdir` read of a directory, with the stamp from before.
    fn learn(&mut self, id: u32, stamp: sys::Stamp, mut list: Listing) {
        let now = self.now();
        let changed = stamp[2].max(stamp[3]);
        if stamp[0] == 0 || changed + listed_racy_ns(changed) > now {
            return;
        }
        list.finish();
        self.versions += 1;
        let d = &mut self.dirs[id as usize];
        d.stamp = stamp;
        d.instant = self.instant;
        d.kept = true;
        d.refused = false;
        d.young_until = 0;
        d.version = self.versions;
        d.list = list;
        d.from_store = d.stored.is_some_and(|s| s.elsewhere && s.stamp == stamp);
    }

    /// True if the list of this directory says what it holds now: its stamp is compared, once in a moment, and a directory that changed is listed again if its stamp
    /// is old enough to show the next change.
    fn usable(&mut self, id: u32, scans: &mut u64, looks: &mut u64) -> bool {
        let instant = self.instant;
        if self.dirs[id as usize].instant == instant {
            return self.dirs[id as usize].kept;
        }
        let now = self.now();
        let d = &mut self.dirs[id as usize];
        d.instant = instant;
        if !d.kept && now < d.young_until {
            return false;
        }
        *looks += 1;
        let stamp = self.stamp_now(id);
        let d = &mut self.dirs[id as usize];
        if stamp == d.stamp && (d.kept || d.refused) {
            return d.kept;
        }
        if self.load_stored(id, stamp) {
            return true;
        }
        let d = &mut self.dirs[id as usize];
        self.versions += 1;
        d.version = self.versions;
        d.kept = false;
        d.refused = false;
        d.young_until = 0;
        d.stamp = stamp;
        d.list = Listing::default();
        d.from_store = false;
        if stamp[0] == 0 {
            d.refused = true;
            return false;
        }
        let changed = stamp[2].max(stamp[3]);
        let until = changed + listed_racy_ns(changed);
        if until > now {
            d.young_until = until;
            return false;
        }
        *scans += 1;
        let mut whole = true;
        match sys::read_dir(&d.path) {
            Ok(rd) => {
                for e in rd {
                    match e {
                        Ok(e) => d.list.push(&sys::os_bytes(&e.file_name()), sys::entry_kind(&e)),
                        Err(_) => whole = false,
                    }
                }
            }
            Err(_) => whole = false,
        }
        if !whole {
            d.list = Listing::default();
            d.refused = true;
            return false;
        }
        d.list.finish();
        d.kept = true;
        d.from_store = d.stored.is_some_and(|s| s.elsewhere && s.stamp == stamp);
        true
    }

    /// The stamp the directory has now, for `usable`, which asks once in a moment.  A `stat` by the path walks every component of it, and a personal tree gets one
    /// for each of its directories in each search: 6200 of a cold run's 7800 system calls with 51 directories under `tex/latex`, at 1.1 µs each.  `fstat` on a
    /// descriptor takes 0.4 µs, but it says how the directory is that was opened, and the question is about the one the path leads to now.  They are the same if the
    /// directory this one is an entry of has been looked at in this moment and still has the list it had when this one was opened, in which the entry is a directory
    /// and no link: then nothing was renamed, removed or put in its place, since each of those changes that directory's times.  That directory's own path is good by
    /// the same argument or because it was asked by the path, as the top of an element always is, since the one above it is not looked at, and as a directory behind
    /// a symbolic link is, since where a link leads can change with nothing in the tree changing.  So what is renamed above a tree, or between a link and where it
    /// leads, shows as it did.  One thing changes no directory's times: a file system mounted over one of them, or taken away.  So the table of mounts is
    /// watched, with one `poll` in a moment that asks a descriptor, and when it has changed nothing is held any more.
    ///
    /// A directory is opened the second time it is asked by its path, and that look takes the stamp from the descriptor just opened, which the path led to in that
    /// moment: one that is looked at once pays for no opening, and the others save a `stat` by it.
    fn stamp_now(&mut self, id: u32) -> sys::Stamp {
        let instant = self.instant;
        if self.dirs[id as usize].held.is_some() && self.mounts_instant != instant {
            self.mounts_instant = instant;
            if !self.mounts.as_ref().is_some_and(|w| !sys::mounts_changed(w)) {
                for d in self.dirs.iter_mut().filter(|d| d.held.is_some()) {
                    d.held = None;
                    d.path_looks = 1;
                }
                self.held = 0;
            }
        }
        if let Some(h) = &self.dirs[id as usize].held {
            let p = &self.dirs[h.parent as usize];
            if p.version != h.parent_version {
                // That directory has another list now: this one is asked by its path, and held again if it is still an entry there.
                let d = &mut self.dirs[id as usize];
                d.held = None;
                d.path_looks = 1;
                self.held -= 1;
            } else if p.instant == instant && p.kept {
                return sys::held_stamp(&h.dir);
            }
        }
        let d = &mut self.dirs[id as usize];
        d.path_looks = d.path_looks.saturating_add(1);
        // The third time and later only now and then: a directory that cannot be held, as the top of an element cannot, is not asked about on every look.
        if d.held.is_none() && d.kept && (d.path_looks == 2 || d.path_looks % 64 == 0) {
            if let Some(stamp) = self.hold(id) {
                return stamp;
            }
        }
        sys::file_stamp(&self.dirs[id as usize].path)
    }

    /// Opens the directory by its path, if `stamp_now`'s argument can be made for it, and gives the stamp it has now.  It stays open if that is the stamp its list
    /// goes by: one that has changed is listed again by the caller, and may be held the next time.
    fn hold(&mut self, id: u32) -> Option<sys::Stamp> {
        if self.no_hold {
            return None;
        }
        let limit = *self.held_limit.get_or_insert_with(sys::held_dirs_limit);
        if self.held >= limit {
            return None;
        }
        // The watch begins before the first directory is opened, so no mount can come between the two unseen.
        if self.mounts.is_none() {
            self.mounts = sys::watch_mounts();
            if self.mounts.is_none() {
                self.no_hold = true;
                return None;
            }
        }
        let d = &self.dirs[id as usize];
        let cut = d.path.iter().rposition(|&c| c == b'/')?;
        let name = &d.path[cut + 1..];
        let &parent = self.by_path.get(&d.path[..=cut])?;
        let p = &self.dirs[parent as usize];
        // The list has names as the directory gave them, so a name that is another way to the same entry, or `.` or `..`, is not in it.
        let is_plain_entry = || p.list.entries().any(|(n, kind)| kind == sys::ENTRY_DIR && n == name);
        if name.is_empty() || p.instant != self.instant || !p.kept || p.path != d.path[..cut] || !is_plain_entry() {
            return None;
        }
        // Not through a symbolic link that has taken the entry's place since: then this fails, and the path is asked.
        let dir = sys::hold_dir(&d.path)?;
        let stamp = sys::held_stamp(&dir);
        if stamp[0] == 0 {
            return None;
        }
        if stamp == d.stamp {
            let parent_version = p.version;
            self.dirs[id as usize].held = Some(Held { dir, parent, parent_version });
            self.held += 1;
        }
        Some(stamp)
    }

    /// Whether a usable list can say that a name is not in its directory.
    fn answers(&self, id: u32) -> bool {
        !self.dirs[id as usize].list.odd
    }

    /// Whether a usable list has a name with this hash.
    fn has(&self, id: u32, fold: u64) -> bool {
        self.dirs[id as usize].list.folds.binary_search(&fold).is_ok()
    }

    /// `casefold_readable_file` by a usable list, for a name with no directory part.
    fn casefold_find(&self, id: u32, base: &[u8]) -> Option<Vec<u8>> {
        let d = &self.dirs[id as usize];
        for (entry, _) in d.list.entries() {
            if entry.eq_ignore_ascii_case(base) {
                let mut ret = [&d.path[..], b"/", entry].concat();
                if readable_file(&mut ret) {
                    return Some(ret);
                }
            }
        }
        None
    }

    /// True if no directory of `dirs` can have a name with one of these hashes, by lists that are all good now.
    fn lacks(&mut self, dirs: &[DirElt], filter: &mut ListFilter, folds: &[u64], scans: &mut u64, looks: &mut u64) -> bool {
        if filter.instant != self.instant {
            filter.instant = self.instant;
            filter.ready = dirs.iter().all(|d| self.usable(d.known, scans, looks) && self.answers(d.known));
        }
        if !filter.ready {
            return false;
        }
        if dirs.len() < FILTER_FROM {
            return !dirs.iter().any(|d| folds.iter().any(|&f| self.has(d.known, f)));
        }
        let versions = dirs.iter().fold(0u64, |a, d| a.wrapping_add(self.dirs[d.known as usize].version));
        if filter.versions != versions || filter.folds.is_empty() {
            filter.versions = versions;
            filter.folds.clear();
            for d in dirs {
                filter.folds.extend_from_slice(&self.dirs[d.known as usize].list.folds);
            }
            filter.folds.sort_unstable();
            filter.folds.dedup();
        }
        !folds.iter().any(|f| filter.folds.binary_search(f).is_ok())
    }
}

/// `casefold_readable_file`: the name with its last component matched case-insensitively.
fn casefold_readable_file(name: &[u8], listed: &mut HashMap<Vec<u8>, ListedDir>, scans: &mut u64) -> Option<Vec<u8>> {
    let base = xbasename(name);
    let dir = xdirname(name);
    sys::note(&dir);
    let stamp = sys::file_stamp(&dir);
    // A directory that does not exist is fine: the name might be subdir/somefile.
    if stamp[0] == 0 {
        return None;
    }
    if listed.get(&dir).is_none_or(|l| l.stamp != stamp) {
        *scans += 1;
        let rd = sys::read_dir(&dir).ok()?;
        let names = rd.flatten().map(|e| sys::os_bytes(&e.file_name())).collect();
        listed.insert(dir.to_vec(), ListedDir { stamp, names });
    }
    let l = &listed[&dir];
    let mut found = None;
    for entry in &l.names {
        if entry.eq_ignore_ascii_case(base) {
            let mut ret = [&dir[..], b"/", &entry[..]].concat();
            if readable_file(&mut ret) {
                found = Some(ret);
                break;
            }
        }
    }
    let changed = stamp[2].max(stamp[3]);
    if changed + listed_racy_ns(changed) > sys::now_ns() {
        listed.remove(&dir);
    }
    found
}

/// `dir_list_search_list`.  Absolute and explicitly relative names were handled by the caller.  With `known`, a name that a directory's list does not have, in any
/// case, is not asked of the disk.
#[allow(clippy::too_many_arguments)]
fn dir_list_search_list(
    dirs: &mut [DirElt],
    names: &[Vec<u8>],
    all: bool,
    casefold: bool,
    listed: &mut HashMap<Vec<u8>, ListedDir>,
    scans: &mut u64,
    looks: &mut u64,
    mut known: Option<&mut Known>,
) -> Vec<Vec<u8>> {
    let mut ret = Vec::new();
    let plain: Vec<Option<u64>> = if known.is_some() { names.iter().map(|n| plain_component(n)).collect() } else { Vec::new() };
    let mut i = 0;
    while i < dirs.len() {
        let id = dirs[i].known;
        let listed_here = known.as_mut().is_some_and(|k| k.usable(id, scans, looks));
        let answers = listed_here && known.as_ref().is_some_and(|k| k.answers(id));
        for (n, name) in names.iter().enumerate() {
            if absolute_p(name, true) {
                continue;
            }
            if answers && plain[n].is_some_and(|f| !known.as_ref().is_some_and(|k| k.has(id, f))) {
                continue;
            }
            let found = if casefold {
                match &known {
                    Some(k) if listed_here && !name.contains(&b'/') => k.casefold_find(id, name),
                    _ => {
                        *looks += 1;
                        casefold_readable_file(&[&dirs[i].dir[..], &name[..]].concat(), listed, scans)
                    }
                }
            } else {
                *looks += 1;
                let mut p = [&dirs[i].dir[..], &name[..]].concat();
                if readable_file(&mut p) { Some(p) } else { None }
            };
            if let Some(f) = found {
                ret.push(f);
                float(dirs, i);
                if !all {
                    return ret;
                }
            }
        }
        i += 1;
    }
    ret
}

impl Kpse {
    /// `texmf_casefold_search`, which C looks up on each miss.
    pub(crate) fn casefold_on(&mut self) -> bool {
        if let Some(v) = self.casefold_memo {
            return v;
        }
        let v = self.var_value(b"texmf_casefold_search");
        let r = cnf_p(v.as_deref());
        if self.cnf_read && !self.doing_cnf_init {
            self.casefold_memo = Some(r);
        }
        r
    }

    /// `kpathsea_element_dirs`.  `None` means the element is empty.
    pub(crate) fn element_dirs(&mut self, elt: &[u8]) -> Option<usize> {
        if elt.is_empty() {
            return None;
        }
        let mut e = elt.to_vec();
        let i = normalize_path(&mut e);
        let key = fold_key(&e);
        if let Some(&id) = self.elt_cache.get(&key) {
            return Some(id);
        }
        let leaf = !DOSISH && {
            let v = self.var_value(b"texmf_nlink_for_leaf");
            cnf_p(v.as_deref())
        };
        let mut list = Vec::new();
        expand_elt(&mut list, &e, i, leaf, &mut self.dir_scans, &mut self.known);
        let id = self.dir_lists.len();
        self.dir_lists.push(list);
        self.list_filters.push(ListFilter::default());
        self.elt_cache.insert(key, id);
        Some(id)
    }

    /// `absolute_search`.
    fn absolute_search(&mut self, name: &[u8]) -> Option<Vec<u8>> {
        let mut found = name.to_vec();
        if readable_file(&mut found) {
            return Some(found);
        }
        // Only if needed and enabled.  Windows is case-insensitive already.
        if !DOSISH && self.casefold_on() {
            return casefold_readable_file(&found, &mut self.listed_dirs, &mut self.dir_scans);
        }
        None
    }

    /// The disk half of one element's search, shared by the two search functions of pathsearch.c.
    fn elt_disk_search(&mut self, elt: &Elt, names: &[Vec<u8>], all: bool, found: &mut Option<Vec<Vec<u8>>>) {
        let Some(id) = self.element_dirs(&elt.text) else { return };
        if self.dir_lists[id].is_empty() {
            return;
        }
        let by_lists = self.known.learns();
        if by_lists {
            // Most of what is asked for is nowhere in the element: then neither pass below finds anything, whatever `texmf_casefold_search` says.
            let folds: Option<Vec<u64>> = names.iter().filter(|n| !absolute_p(n, true)).map(|n| plain_component(n)).collect();
            if folds.is_some_and(|f| self.known.lacks(&self.dir_lists[id], &mut self.list_filters[id], &f, &mut self.dir_scans, &mut self.disk_looks)) {
                *found = Some(Vec::new());
                return;
            }
        }
        let mut r =
            dir_list_search_list(&mut self.dir_lists[id], names, all, false, &mut self.listed_dirs, &mut self.dir_scans, &mut self.disk_looks, by_lists.then_some(&mut self.known));
        if r.is_empty() && !DOSISH && self.casefold_on() {
            // Nothing found: search again, case-insensitively.
            r = dir_list_search_list(&mut self.dir_lists[id], names, all, true, &mut self.listed_dirs, &mut self.dir_scans, &mut self.disk_looks, by_lists.then_some(&mut self.known));
        }
        *found = Some(r);
    }

    /// A search begins: what was seen of a directory before says nothing of it now, unless a caller holds the moment.
    pub(crate) fn new_instant(&mut self) {
        if self.moment_held == 0 {
            self.known.instant += 1;
        }
    }

    /// From here to [`Kpse::end_moment`] every search is of one moment: a directory of a tree on the disk is looked at once for all of them.  For a caller that asks
    /// many questions about the world as it is now, and does nothing in between that could change an answer.  They nest.
    pub fn begin_moment(&mut self) {
        self.new_instant();
        self.moment_held += 1;
    }

    pub fn end_moment(&mut self) {
        self.moment_held = self.moment_held.saturating_sub(1);
    }

    /// `kpathsea_path_search_list_generic`.
    pub(crate) fn path_search_list_generic(&mut self, elts: &[Elt], names: &[Vec<u8>], must_exist: bool, all: bool) -> Vec<Vec<u8>> {
        self.new_instant();
        let mut ret: Vec<Vec<u8>> = Vec::new();
        let mut all_absolute = true;
        let mut finished = false;
        // First catch any absolute or explicit relative names.
        for name in names {
            if absolute_p(name, true) {
                if let Some(f) = self.absolute_search(name) {
                    ret.push(f);
                    if !all {
                        finished = true;
                        break;
                    }
                }
            } else {
                all_absolute = false;
            }
        }
        if !finished && !all_absolute {
            for elt in elts {
                // Try ls-R, unless we are searching for texmf.cnf.
                let mut found = if self.followup_search { self.db_search_list(names, &elt.text, all) } else { None };
                // Search the file system if the path allows it, and either no db is relevant or
                // the name is not in it and must exist.
                if elt.disk && (found.is_none() || (must_exist && found.as_ref().is_some_and(Vec::is_empty))) {
                    self.elt_disk_search(elt, names, all, &mut found);
                }
                if let Some(mut f) = found {
                    if !f.is_empty() {
                        if all {
                            ret.append(&mut f);
                        } else {
                            ret.push(f.swap_remove(0));
                            break;
                        }
                    }
                }
            }
        }
        // Uniqify, since our paths can often end up finding the same file more than once.
        let mut uniq: Vec<Vec<u8>> = Vec::with_capacity(ret.len());
        for r in ret {
            if !uniq.iter().any(|u| file_str_eq(u, &r)) {
                uniq.push(r);
            }
        }
        self.followup_search = true;
        uniq
    }

    /// `path_search`, for one name.
    fn path_search_elts(&mut self, elts: &[Elt], name: &[u8], must_exist: bool, all: bool) -> Vec<Vec<u8>> {
        let names = [name.to_vec()];
        let mut ret = Vec::new();
        for elt in elts {
            let mut found = if self.followup_search { self.db_search(name, &elt.text, all) } else { None };
            if elt.disk && (found.is_none() || (must_exist && found.as_ref().is_some_and(Vec::is_empty))) {
                self.elt_disk_search(elt, &names, all, &mut found);
            }
            if let Some(mut f) = found {
                if !f.is_empty() {
                    if all {
                        ret.append(&mut f);
                    } else {
                        ret.push(f.swap_remove(0));
                        break;
                    }
                }
            }
        }
        ret
    }

    /// pathsearch.c's `search`.
    fn search(&mut self, path: &[u8], original_name: &[u8], must_exist: bool, all: bool) -> Vec<Vec<u8>> {
        self.new_instant();
        // Make a leading ~ count as an absolute file name, and expand $FOO's.
        let name = self.expand(original_name);
        let ret = if absolute_p(&name, true) {
            self.absolute_search(&name).into_iter().collect()
        } else {
            let elts = split_elts(path);
            self.path_search_elts(&elts, &name, must_exist, all)
        };
        self.followup_search = true;
        ret
    }

    /// `kpathsea_path_search`: the first `name` along a path given as text, which must already be
    /// brace expanded.
    pub fn path_search(&mut self, path: &[u8], name: &[u8], must_exist: bool) -> Option<Vec<u8>> {
        self.ensure_cnf();
        self.search(path, cut_at_nul(name), must_exist, false).into_iter().next()
    }

    /// `kpathsea_all_path_search`: every `name` along a path.
    pub fn all_path_search(&mut self, path: &[u8], name: &[u8]) -> Vec<Vec<u8>> {
        self.ensure_cnf();
        self.all_path_search_str(path, cut_at_nul(name))
    }

    /// `kpathsea_all_path_search` during initialization: no asking for the cnf files.
    pub(crate) fn all_path_search_str(&mut self, path: &[u8], name: &[u8]) -> Vec<Vec<u8>> {
        self.search(path, name, true, true)
    }

    /// `kpathsea_init_format`: the final search path of a format.  `None` for an unknown format.
    pub(crate) fn init_format(&mut self, format: i32) -> Option<Arc<FormatState>> {
        let def = format::format_def(format)?;
        if let Some(st) = &self.formats[format as usize] {
            return Some(st.clone());
        }
        if format == format::KPSE_PK_FORMAT || format == format::KPSE_ANY_GLYPH_FORMAT {
            self.init_maketex(format);
        }
        let indexed = if self.override_paths[format as usize].is_none() { self.index.as_ref().and_then(|ix| ix.format_path(&self.program_name, format)) } else { None };
        let mut path = match indexed {
            Some(p) => p.to_vec(),
            None => self.init_path(format, def),
        };
        if format == format::KPSE_DB_FORMAT {
            path = remove_dbonly(&path);
        }
        let elts = split_elts(&path);
        let st = Arc::new(FormatState { path, elts });
        self.formats[format as usize] = Some(st.clone());
        Some(st)
    }

    /// `init_path`.
    fn init_path(&mut self, format: i32, def: &FormatDef) -> Vec<u8> {
        let prog = self.program_name.clone();
        let upper = prog.to_ascii_uppercase();
        let program_env = [&upper[..], b"INPUTS"].concat();
        let default_path: Vec<u8> = match def.default {
            DefaultPath::Nonesuch => format::DEFAULT_NONESUCH.to_vec(),
            DefaultPath::Cnf => self.tree.cnf_default.clone().unwrap_or_else(|| format::DEFAULT_TEXMFCNF.to_vec()),
            DefaultPath::TrFonts => format::DEFAULT_TRFONTS.to_vec(),
            DefaultPath::CLua => format::DEFAULT_CLUAINPUTS.to_vec(),
            DefaultPath::Program => [b".", &[ENV_SEP][..], b"$TEXMF/", &prog[..], b"//"].concat(),
        };
        let envs: Vec<&[u8]> =
            if def.program_envs { vec![&program_env[..]] } else { def.envs.iter().map(|s| s.as_bytes()).collect() };

        // The first envvar set to a nonempty value; the first cnf entry that matches.
        let mut var: Option<Vec<u8>> = None;
        let mut cnf_path: Option<Vec<u8>> = None;
        for env_name in envs {
            if var.is_none() {
                var = self
                    .getenv(&[env_name, b".", &prog].concat())
                    .filter(|v| !v.is_empty())
                    .or_else(|| self.getenv(&[env_name, b"_", &prog].concat()).filter(|v| !v.is_empty()))
                    .or_else(|| self.getenv(env_name).filter(|v| !v.is_empty()));
            }
            // If we are initializing the cnf path, do not try to get values from the cnf files.
            if cnf_path.is_none() && format != format::KPSE_CNF_FORMAT {
                cnf_path = self.cnf_get(env_name);
            }
            if var.is_some() && cnf_path.is_some() {
                break;
            }
        }

        // For each level, an extra : is replaced with the path at the next lower level.
        let mut path = default_path;
        if let Some(c) = cnf_path {
            path = expand_default(Some(&c), &path);
        }
        if let Some(mut v) = var {
            if is_env_sep(b':') {
                for c in v.iter_mut() {
                    if *c == b';' {
                        *c = b':';
                    }
                }
            }
            path = expand_default(Some(&v), &path);
        }
        if let Some(over) = self.override_paths[format as usize].clone() {
            path = expand_default(Some(&over), &path);
        }
        self.brace_expand(&path)
    }

    /// `target_fontmaps`.
    fn target_fontmaps(&mut self, target: &mut Vec<Vec<u8>>, name: &[u8]) {
        if let Some(mapped) = self.fontmap_lookup(name) {
            target.extend(mapped);
        }
    }

    /// `target_asis_name`.
    fn target_asis_name(&mut self, target: &mut Vec<Vec<u8>>, def: &FormatDef, name: &[u8], use_fontmaps: bool, has_potential_suffix: bool) {
        if has_potential_suffix || !def.suffix_search_only {
            target.push(name.to_vec());
            if use_fontmaps {
                self.target_fontmaps(target, name);
            }
        }
    }

    /// `target_suffixed_names`.
    fn target_suffixed_names(&mut self, target: &mut Vec<Vec<u8>>, def: &FormatDef, name: &[u8], use_fontmaps: bool, has_potential_suffix: bool) {
        if has_potential_suffix || def.suffix.is_empty() {
            return;
        }
        for ext in def.suffix {
            let with_suffix = [name, ext.as_bytes()].concat();
            target.push(with_suffix.clone());
            if use_fontmaps {
                self.target_fontmaps(target, &with_suffix);
            }
        }
    }

    /// `kpathsea_find_file`.
    pub fn find_file(&mut self, name: &[u8], format: i32, must_exist: bool) -> Option<Vec<u8>> {
        self.find_file_generic(name, format, must_exist, false).into_iter().next()
    }

    /// `kpathsea_find_file_generic`.  With `all` every match is returned, otherwise at most one.
    /// Where C would then run `mktex*`, this returns nothing: only a bitmap font is ever made, by [`Kpse::find_glyph`].
    pub fn find_file_generic(&mut self, const_name: &[u8], format: i32, must_exist: bool, all: bool) -> Vec<Vec<u8>> {
        let Some(def) = format::format_def(format) else {
            self.warn(format!("kpse_init_format: Unknown format {format}").as_bytes());
            return Vec::new();
        };
        let use_fontmaps = matches!(
            format,
            format::KPSE_TFM_FORMAT | format::KPSE_GF_FORMAT | format::KPSE_PK_FORMAT | format::KPSE_OFM_FORMAT
        );
        let Some(st) = self.init_format(format) else { return Vec::new() };

        // Do variable and tilde expansion.
        let name = self.expand(cut_at_nul(const_name));

        let dir_sep = b'/';
        let has_any_suffix = match name.iter().rposition(|&c| c == b'.') {
            Some(d) => !name[d..].contains(&dir_sep),
            None => false,
        };

        // Does the name already end in a possible suffix?
        let ends_with = |ext: &&str| {
            let e = ext.as_bytes();
            name.len() >= e.len() && file_str_eq(e, &name[name.len() - e.len()..])
        };
        let has_potential_suffix = def.suffix.iter().any(ends_with) || def.alt_suffix.iter().any(ends_with);

        // The list of target names, in an order that depends on try_std_extension_first.
        let mut target: Vec<Vec<u8>> = Vec::new();
        let try_std = self.var_value(b"try_std_extension_first");
        if has_any_suffix && !cnf_p(try_std.as_deref()) {
            self.target_asis_name(&mut target, def, &name, use_fontmaps, has_potential_suffix);
            self.target_suffixed_names(&mut target, def, &name, use_fontmaps, has_potential_suffix);
        } else {
            self.target_suffixed_names(&mut target, def, &name, use_fontmaps, has_potential_suffix);
            self.target_asis_name(&mut target, def, &name, use_fontmaps, has_potential_suffix);
        }

        // Search, trying to minimize disk-pounding.  The two passes are one look at the disk.
        self.begin_moment();
        let mut ret = self.path_search_list_generic(&st.elts, &target, false, all);

        // Do we need to pound the disk?
        if ret.is_empty() && must_exist {
            target.clear();
            // A subset of the previous names; no alternate names from the fontmaps.
            if !has_potential_suffix && def.suffix_search_only {
                for ext in def.suffix {
                    target.push([&name[..], ext.as_bytes()].concat());
                }
            }
            if has_potential_suffix || !def.suffix_search_only {
                target.push(name.clone());
            }
            ret = self.path_search_list_generic(&st.elts, &target, true, all);
        }
        self.end_moment();
        ret
    }
}

/// C strings end at the first NUL.
fn cut_at_nul(s: &[u8]) -> &[u8] {
    match s.iter().position(|&c| c == 0) {
        Some(n) => &s[..n],
        None => s,
    }
}

/// `remove_dbonly`: the `!!` of each path element.
fn remove_dbonly(path: &[u8]) -> Vec<u8> {
    let mut ret = Vec::with_capacity(path.len());
    let mut new_elt = true;
    let mut p = 0;
    while p < path.len() {
        if new_elt && path[p] == b'!' && at(path, p + 1) == b'!' {
            p += 2;
        } else {
            new_elt = path[p] == ENV_SEP;
            ret.push(path[p]);
            p += 1;
        }
    }
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(names: &[&[u8]]) -> Vec<DirElt> {
        names.iter().map(|n| DirElt { dir: n.to_vec(), moved: false, known: 0 }).collect()
    }
    fn order(d: &[DirElt]) -> Vec<Vec<u8>> {
        d.iter().map(|e| e.dir.clone()).collect()
    }

    #[test]
    fn float_as_str_llist_c() {
        let mut d = list(&[b"a", b"b", b"c", b"d"]);
        float(&mut d, 2);
        assert_eq!(order(&d), vec![b"c".to_vec(), b"a".to_vec(), b"b".to_vec(), b"d".to_vec()]);
        // The next one goes after the first moved, not before it.
        float(&mut d, 3);
        assert_eq!(order(&d), vec![b"c".to_vec(), b"d".to_vec(), b"a".to_vec(), b"b".to_vec()]);
        // Moving a moved element is a no-op.
        float(&mut d, 0);
        assert_eq!(order(&d), vec![b"c".to_vec(), b"d".to_vec(), b"a".to_vec(), b"b".to_vec()]);
        // The first unmoved element only gets marked.
        float(&mut d, 2);
        assert_eq!(order(&d), vec![b"c".to_vec(), b"d".to_vec(), b"a".to_vec(), b"b".to_vec()]);
        assert!(d[2].moved);
    }

    #[test]
    fn dbonly_marks_are_removed() {
        let s = |t: &str| t.replace(':', std::str::from_utf8(&[ENV_SEP]).unwrap()).into_bytes();
        assert_eq!(remove_dbonly(&s("!!a:b:!!c")), s("a:b:c"));
        assert_eq!(remove_dbonly(&s("a!!b")), s("a!!b"));
        assert_eq!(remove_dbonly(b""), b"");
    }

    #[test]
    fn elements_keep_the_disk_mark_and_normalize() {
        let sep = std::str::from_utf8(&[ENV_SEP]).unwrap();
        let e = split_elts(format!(".{sep}!!/a//{sep}//b/c{sep}").as_bytes());
        assert_eq!(e.len(), 4);
        assert!(e[0].disk && !e[1].disk && e[2].disk && e[3].disk);
        assert_eq!(e[1].text, b"/a//");
        // On Windows two slashes and a name begin a UNC name, `IS_UNC_NAME`: they stay.
        assert_eq!(e[2].text, if DOSISH { &b"//b/c"[..] } else { b"/b/c" });
        assert_eq!(e[3].text, b"");
    }
}
