//! Worker threads that stay around: starting threads for every picture
//! costs about half a millisecond each time (on Windows), which adds up
//! when a picture is split over the processor cores several times.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Condvar, Mutex, OnceLock};

/// The job of the moment, borrowed from `run` (which waits until it is
/// done before it returns, so the borrow outlives every use).
#[derive(Clone, Copy)]
struct Job(*const (dyn Fn(usize) + Sync));
// SAFETY: the closure behind it is Sync, and `run` keeps it alive while
// any worker can call it
unsafe impl Send for Job {}

struct State {
    job: Option<Job>,
    /// Parts in all, the next one to hand out, and those not yet finished.
    count: usize,
    next: usize,
    unfinished: usize,
    panicked: bool,
}

struct Pool {
    state: Mutex<State>,
    work: Condvar,
    done: Condvar,
    /// Only one `run` at a time.
    busy: Mutex<()>,
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<&'static Pool> = OnceLock::new();
    POOL.get_or_init(|| {
        let pool: &'static Pool = Box::leak(Box::new(Pool {
            state: Mutex::new(State { job: None, count: 0, next: 0, unfinished: 0, panicked: false }),
            work: Condvar::new(),
            done: Condvar::new(),
            busy: Mutex::new(()),
        }));
        // the calling thread works along, so one thread fewer than cores
        for _ in 1..cores() {
            std::thread::spawn(move || loop {
                let (job, i) = next_part(pool);
                finish(pool, job, i);
            });
        }
        pool
    })
}

/// Waits for a part of a job that nobody works on yet and takes it.
fn next_part(pool: &Pool) -> (Job, usize) {
    let mut s = pool.state.lock().unwrap();
    loop {
        if let (Some(job), true) = (s.job, s.next < s.count) {
            s.next += 1;
            return (job, s.next - 1);
        }
        s = pool.work.wait(s).unwrap();
    }
}

/// Does part `i` of a job and counts it as finished.
fn finish(pool: &Pool, job: Job, i: usize) {
    // SAFETY: see `Job`
    let ok = catch_unwind(AssertUnwindSafe(|| unsafe { (*job.0)(i) })).is_ok();
    let mut s = pool.state.lock().unwrap();
    s.panicked |= !ok;
    s.unfinished -= 1;
    if s.unfinished == 0 {
        pool.done.notify_all();
    }
}

/// How many processor cores to spread work over.
pub fn cores() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16)
}

/// Calls `f(0)` .. `f(count - 1)`, spread over the cores, and returns
/// when all calls are done.
pub fn run(count: usize, f: &(dyn Fn(usize) + Sync)) {
    if count == 0 {
        return;
    }
    let pool = pool();
    let _one = pool.busy.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: only the lifetime is changed; this function does not return
    // before every call of `f` has finished
    let job = Job(unsafe { std::mem::transmute::<*const (dyn Fn(usize) + Sync + '_), *const (dyn Fn(usize) + Sync + 'static)>(f) });
    {
        let mut s = pool.state.lock().unwrap();
        *s = State { job: Some(job), count, next: 0, unfinished: count, panicked: false };
    }
    pool.work.notify_all();
    // work along
    loop {
        let i = {
            let mut s = pool.state.lock().unwrap();
            if s.next >= s.count {
                break;
            }
            s.next += 1;
            s.next - 1
        };
        finish(pool, job, i);
    }
    let mut s = pool.state.lock().unwrap();
    while s.unfinished > 0 {
        s = pool.done.wait(s).unwrap();
    }
    s.job = None;
    let panicked = s.panicked;
    drop(s);
    if panicked {
        panic!("a part of the work failed");
    }
}

/// Hands each of `items` to `f` (with its number), spread over the cores.
pub fn each<T: Send>(items: Vec<T>, f: impl Fn(usize, T) + Sync) {
    let slots: Vec<Mutex<Option<T>>> = items.into_iter().map(|t| Mutex::new(Some(t))).collect();
    run(slots.len(), &|i| {
        if let Some(t) = slots[i].lock().unwrap().take() {
            f(i, t);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_part_once() {
        for _ in 0..50 {
            let mut v = [0u32; 37];
            each(v.chunks_mut(5).collect(), |n, c: &mut [u32]| {
                for x in c.iter_mut() {
                    *x += 1 + n as u32;
                }
            });
            for (i, x) in v.iter().enumerate() {
                assert_eq!(*x, 1 + (i / 5) as u32);
            }
        }
    }
}
