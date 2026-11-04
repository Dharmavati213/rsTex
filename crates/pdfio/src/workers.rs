//! Whole streams deflated on other threads (DESIGN.md section 9).
//!
//! The owner of the output hands over a stream's bytes and gets a slot number, in order; the bytes that come out of a
//! stream depend on nothing but its input and level, so which thread made them does not show.  [`DeflatePool::finish_all`]
//! returns them by slot, and the owner puts them where they belong.  The owner never waits while there is work it can do
//! itself: if the queue is too long it deflates a stream from the queue, and it does the same while waiting for the end.
//!
//! Threads start as the queue asks for them, up to the number given.  They hold nothing that needs an orderly end, so a
//! process may exit with them running.  The owner has nothing to fall back on but itself: no thread at all is what
//! [`DeflatePool::new`] says with `None`.
//!
//! Two kinds of stream go through one pool, each with slots of its own: the PDF's zlib streams, and the gzip members of SyncTeX's file, which is written apart from the PDF and
//! whose owner settles at other moments.

use crate::flate::{Deflater, gzip_member};
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

/// Bytes of input that may wait in the queue before the owner starts to help.
const MAX_QUEUED_BYTES: usize = 64 << 20;
/// Input buffers kept for the owner to use again.
const MAX_SPARE: usize = 16;

/// The kinds of stream, an index into [`State`]'s tables.
const ZLIB: usize = 0;
const GZIP: usize = 1;

struct Job {
    kind: usize,
    slot: usize,
    input: Vec<u8>,
    level: i32,
}

#[derive(Default)]
struct State {
    queue: VecDeque<Job>,
    queued_bytes: usize,
    /// By kind and slot.  A stream is `None` until it is done.
    done: [Vec<Option<Vec<u8>>>; 2],
    /// By kind, slots not yet done, queued or being deflated.
    unfinished: [usize; 2],
    spare: Vec<Vec<u8>>,
    /// Threads waiting for work.
    idle: usize,
    closing: bool,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    /// A job came, or the pool is closing.
    work: Condvar,
    /// A slot is done.
    slot_done: Condvar,
}

/// A thread's deflater, and the space it writes in: that is big, since it grows by 32 KiB at a time, and the stream it
/// holds is mostly small, so what is kept is a copy, and the space is used again with its pages already there.
#[derive(Default)]
struct Hands {
    deflater: Option<Deflater>,
    scratch: Vec<u8>,
}

impl Hands {
    fn deflate(&mut self, job: &Job) -> Vec<u8> {
        self.scratch.clear();
        if job.kind == GZIP {
            gzip_member(&mut self.deflater, &job.input, job.level, &mut self.scratch);
        } else {
            Deflater::whole(&mut self.deflater, &job.input, job.level, &mut self.scratch);
        }
        self.scratch.as_slice().to_vec()
    }
}

impl Shared {
    fn finished(&self, job: Job, out: Vec<u8>) {
        let Job { kind, slot, mut input, .. } = job;
        let mut s = self.state.lock().expect("no thread panics holding this");
        s.done[kind][slot] = Some(out);
        s.unfinished[kind] -= 1;
        if s.spare.len() < MAX_SPARE {
            input.clear();
            s.spare.push(input);
        }
        drop(s);
        self.slot_done.notify_one();
    }
}

fn worker(shared: Arc<Shared>) {
    let mut hands = Hands::default();
    loop {
        let job = {
            let mut s = shared.state.lock().expect("no thread panics holding this");
            loop {
                if s.closing {
                    return;
                }
                if let Some(job) = s.queue.pop_front() {
                    s.queued_bytes -= job.input.len();
                    break job;
                }
                s.idle += 1;
                s = shared.work.wait(s).expect("no thread panics holding this");
                s.idle -= 1;
            }
        };
        let out = hands.deflate(&job);
        shared.finished(job, out);
    }
}

/// See the module documentation.
pub struct DeflatePool {
    shared: Arc<Shared>,
    threads: usize,
    max_threads: usize,
    /// The owner's, for when it helps.
    hands: Hands,
}

impl DeflatePool {
    /// A pool of at most `max_threads`, with one started.  `None` if none can be.
    pub fn new(max_threads: usize) -> Option<DeflatePool> {
        let mut pool = DeflatePool { shared: Arc::default(), threads: 0, max_threads, hands: Hands::default() };
        if max_threads > 0 && pool.start_thread() { Some(pool) } else { None }
    }

    fn start_thread(&mut self) -> bool {
        let shared = Arc::clone(&self.shared);
        let started = std::thread::Builder::new().name("deflate".into()).spawn(move || worker(shared)).is_ok();
        self.threads += started as usize;
        started
    }

    /// An empty buffer for the next stream's bytes, one that has been through the pool if there is one.
    pub fn input_buffer(&mut self) -> Vec<u8> {
        self.shared.state.lock().expect("no thread panics holding this").spare.pop().unwrap_or_default()
    }

    /// Hands over a stream's bytes, for the next slot.
    pub fn submit(&mut self, input: Vec<u8>, level: i32) {
        self.hand_over(ZLIB, input, level);
    }

    /// Hands over what is to be one gzip member, for the next slot of those.
    pub fn submit_gzip(&mut self, input: Vec<u8>, level: i32) {
        self.hand_over(GZIP, input, level);
    }

    fn hand_over(&mut self, kind: usize, input: Vec<u8>, level: i32) {
        let mut s = self.shared.state.lock().expect("no thread panics holding this");
        let slot = s.done[kind].len();
        s.done[kind].push(None);
        s.unfinished[kind] += 1;
        s.queued_bytes += input.len();
        s.queue.push_back(Job { kind, slot, input, level });
        let (idle, queued, too_long) = (s.idle, s.queue.len(), s.queued_bytes > MAX_QUEUED_BYTES);
        drop(s);
        if idle > 0 {
            self.shared.work.notify_one();
        } else if queued > 1 && self.threads < self.max_threads {
            self.start_thread();
        }
        if too_long {
            self.help();
        }
    }

    /// Deflates one stream from the queue on this thread.  False if the queue is empty.
    fn help(&mut self) -> bool {
        let job = {
            let mut s = self.shared.state.lock().expect("no thread panics holding this");
            let Some(job) = s.queue.pop_front() else { return false };
            s.queued_bytes -= job.input.len();
            job
        };
        let out = self.hands.deflate(&job);
        self.shared.finished(job, out);
        true
    }

    /// Every stream handed over since the last call, by slot, when all are done.  Slots start again from 0.
    pub fn finish_all(&mut self) -> Vec<Vec<u8>> {
        self.finish(ZLIB)
    }

    /// The same for the gzip members.
    pub fn finish_gzip(&mut self) -> Vec<Vec<u8>> {
        self.finish(GZIP)
    }

    fn finish(&mut self, kind: usize) -> Vec<Vec<u8>> {
        while self.help() {}
        let mut s = self.shared.state.lock().expect("no thread panics holding this");
        while s.unfinished[kind] > 0 {
            s = self.shared.slot_done.wait(s).expect("no thread panics holding this");
        }
        core::mem::take(&mut s.done[kind]).into_iter().map(|d| d.expect("all are done")).collect()
    }
}

impl Drop for DeflatePool {
    fn drop(&mut self) {
        self.shared.state.lock().expect("no thread panics holding this").closing = true;
        self.shared.work.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flate::{LEVEL_CONTENT, LEVEL_FONTFILE, LEVEL_SYNCTEX, deflate, inflate};

    fn sample(seed: u64, len: usize) -> Vec<u8> {
        let mut x = seed.wrapping_mul(0x9E3779B97F4A7C15) | 1;
        let mut v = Vec::with_capacity(len);
        while v.len() < len {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            // Compressible, with runs and repeats, like a page.
            let word = b"BT /F12 9.9626 Tf 72 720 Td [(hello)-333(world)]TJ ET\n";
            let at = (x % word.len() as u64) as usize;
            v.extend_from_slice(&word[at..]);
            v.push((x >> 40) as u8);
        }
        v.truncate(len);
        v
    }

    #[test]
    fn slots_come_back_in_order_and_the_bytes_do_not_depend_on_the_thread() {
        let inputs: Vec<(Vec<u8>, i32)> = (0..300u64)
            .map(|i| (sample(i + 1, (i as usize * 977) % 70_000), if i % 7 == 0 { LEVEL_FONTFILE } else { LEVEL_CONTENT }))
            .collect();
        let mut pool = DeflatePool::new(3).expect("threads");
        // Twice, so that the second round finds used deflaters and slots start again.
        for _ in 0..2 {
            for (input, level) in &inputs {
                let mut b = pool.input_buffer();
                assert!(b.is_empty());
                b.extend_from_slice(input);
                pool.submit(b, *level);
            }
            let got = pool.finish_all();
            assert_eq!(got.len(), inputs.len());
            for ((input, level), z) in inputs.iter().zip(&got) {
                let mut whole = Vec::new();
                let mut d = Deflater::new(*level);
                d.finish(input, &mut whole);
                assert_eq!(z, &whole, "a stream of {} bytes", input.len());
                let mut back = Vec::new();
                inflate(z, &mut back).expect("valid");
                assert_eq!(&back, input);
            }
        }
    }

    /// The serial path keeps one deflater for all streams, so this is what makes its bytes the same as a worker's.
    #[test]
    fn a_deflater_used_before_gives_what_a_new_one_gives() {
        let mut used = Deflater::new(LEVEL_CONTENT);
        for i in 0..200u64 {
            let input = sample(i + 1000, (i as usize * 3301) % 200_000);
            let level = if i % 5 == 0 { LEVEL_FONTFILE } else { LEVEL_CONTENT };
            used.reset(level);
            let mut a = Vec::new();
            used.finish(&input, &mut a);
            let mut b = Vec::new();
            Deflater::new(level).finish(&input, &mut b);
            assert_eq!(a, b, "stream {i}");
        }
    }

    /// Each kind comes back whole and in its own order, whichever is asked for first, and a member is what one made on this thread is.
    #[test]
    fn the_two_kinds_keep_apart() {
        let inputs: Vec<Vec<u8>> = (0..64u64).map(|i| sample(i + 7, (i as usize * 4099) % 90_000)).collect();
        let mut pool = DeflatePool::new(2).expect("threads");
        for input in &inputs {
            pool.submit(input.clone(), LEVEL_CONTENT);
            pool.submit_gzip(input.clone(), LEVEL_SYNCTEX);
        }
        let members = pool.finish_gzip();
        // Nothing of the other kind is lost by that, and a slot starts again from 0 for this kind alone.
        pool.submit_gzip(inputs[1].clone(), LEVEL_SYNCTEX);
        let streams = pool.finish_all();
        assert_eq!((members.len(), streams.len()), (inputs.len(), inputs.len()));
        let mut one = None;
        for ((input, member), stream) in inputs.iter().zip(&members).zip(&streams) {
            let mut here = Vec::new();
            gzip_member(&mut one, input, LEVEL_SYNCTEX, &mut here);
            assert_eq!(member, &here);
            let mut whole = Vec::new();
            Deflater::new(LEVEL_CONTENT).finish(input, &mut whole);
            assert_eq!(stream, &whole);
        }
        assert_eq!(pool.finish_gzip().len(), 1);
    }

    #[test]
    fn no_threads_is_no_pool() {
        assert!(DeflatePool::new(0).is_none());
    }

    #[test]
    fn an_empty_stream_and_no_streams() {
        let mut pool = DeflatePool::new(1).expect("a thread");
        assert!(pool.finish_all().is_empty());
        pool.submit(Vec::new(), LEVEL_CONTENT);
        let got = pool.finish_all();
        let mut d = Vec::new();
        Deflater::new(LEVEL_CONTENT).finish(&[], &mut d);
        assert_eq!(got, vec![d]);
        let _ = deflate;
    }

    #[test]
    fn a_long_queue_gets_help() {
        let mut pool = DeflatePool::new(1).expect("a thread");
        // More than MAX_QUEUED_BYTES in all, at level 0's cost: what matters is that it ends and all is there.
        let big = vec![7u8; 8 << 20];
        for _ in 0..12 {
            pool.submit(big.clone(), LEVEL_FONTFILE);
        }
        let got = pool.finish_all();
        assert_eq!(got.len(), 12);
        assert!(got.iter().all(|z| *z == got[0]));
    }
}
