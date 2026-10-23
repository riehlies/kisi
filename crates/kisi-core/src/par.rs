//! Parallel work for the native CLI: the channel check computes its lanes in worker threads.
//!
//! Every lane is a pure function of the project, so the rows do not depend on the thread that
//! computes them or on the order (the via model's caches are memos of quantised inputs, see
//! `kisi_si::via_pbm`); the results are put back in their order. In WebAssembly (viewer, Node
//! CLI) there are no threads here: the Node CLI and the viewer run engines in workers of their
//! own (`service::channel_check_part`).

/// Worker threads by default (fewer on machines with fewer CPUs).
pub const WORKERS: usize = 4;

/// Number of worker threads: `WORKERS`, at most the CPUs; the environment variable
/// `KISI_THREADS` overrides it (for timing and for the test that a parallel run gives the
/// same output as one thread). Always 1 in WebAssembly.
pub fn workers() -> usize {
    if cfg!(target_arch = "wasm32") {
        return 1;
    }
    if let Some(n) = std::env::var("KISI_THREADS").ok().and_then(|v| v.trim().parse::<usize>().ok()).filter(|&n| n >= 1) {
        return n;
    }
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(WORKERS)
}

/// `items.iter().map(f).collect()`, on up to `threads` threads (items handed out one by one, the
/// results in the order of `items`). A panic in `f` is passed on.
pub fn par_map_with<T: Sync, R: Send>(items: &[T], threads: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let n = threads.min(items.len());
    if n <= 1 || cfg!(target_arch = "wasm32") {
        return items.iter().map(f).collect();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut out: Vec<Option<R>> = (0..items.len()).map(|_| None).collect();
    std::thread::scope(|sc| {
        let work = || {
            let mut mine = Vec::new();
            loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if i >= items.len() {
                    break;
                }
                mine.push((i, f(&items[i])));
            }
            mine
        };
        // the via model's Laplace solves recurse little, but leave room as the main thread has
        let handles: Vec<_> = (0..n).map(|_| std::thread::Builder::new().stack_size(16 << 20).spawn_scoped(sc, work).expect("worker thread")).collect();
        for h in handles {
            match h.join() {
                Ok(mine) => {
                    for (i, r) in mine {
                        out[i] = Some(r);
                    }
                }
                Err(e) => std::panic::resume_unwind(e),
            }
        }
    });
    out.into_iter().map(|r| r.expect("every item computed")).collect()
}

/// `par_map_with` on `workers()` threads.
pub fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    par_map_with(items, workers(), f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_kept() {
        let v: Vec<u64> = (0..1000).collect();
        let r = par_map_with(&v, 4, |&x| x * x);
        assert_eq!(r, v.iter().map(|x| x * x).collect::<Vec<_>>());
        assert_eq!(par_map_with(&v[..0], 4, |&x| x), Vec::<u64>::new());
        assert!((1..=WORKERS.max(1)).contains(&workers()) || std::env::var("KISI_THREADS").is_ok());
    }
}
