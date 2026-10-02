//! Small bounded fan-out, with ordered output and no detached work on failure.
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex,
};

pub(super) fn try_map<T: Sync, R: Send, E: Send>(
    items: &[T],
    workers: usize,
    work: impl Fn(&T) -> Result<R, E> + Sync,
) -> Result<Vec<R>, E> {
    let workers = workers.max(1).min(items.len());
    if workers <= 1 {
        return items.iter().map(work).collect();
    }
    let next = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let error = Mutex::new(None);
    let results = Mutex::new((0..items.len()).map(|_| None).collect::<Vec<_>>());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                while !failed.load(Ordering::Acquire) {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(index) else {
                        break;
                    };
                    match work(item) {
                        Ok(result) => results.lock().unwrap()[index] = Some(result),
                        Err(failure) => {
                            // Preserve the first cause, then drain in-flight work.
                            error.lock().unwrap().get_or_insert(failure);
                            failed.store(true, Ordering::Release);
                            break;
                        }
                    }
                }
            });
        }
    });
    if let Some(error) = error.into_inner().unwrap() {
        return Err(error);
    }
    Ok(results
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|value| value.expect("successful bounded work must produce every result"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Barrier, time::Duration};

    #[test]
    fn parallel_output_preserves_source_order_and_worker_bound() {
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let barrier = Barrier::new(4);
        let input: Vec<_> = (0..20).collect();
        let values = try_map(&input, 4, |index| {
            let count = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(count, Ordering::SeqCst);
            if *index < 4 {
                barrier.wait();
            }
            std::thread::sleep(Duration::from_millis((20 - index) as u64));
            active.fetch_sub(1, Ordering::SeqCst);
            Ok::<_, ()>(index * 2)
        })
        .unwrap();
        assert_eq!(
            values,
            input.iter().map(|value| value * 2).collect::<Vec<_>>()
        );
        assert_eq!(peak.load(Ordering::SeqCst), 4);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn failure_stops_scheduling_and_joins_inflight_work() {
        let finished = AtomicUsize::new(0);
        let barrier = Barrier::new(2);
        let result = try_map(&(0..100).collect::<Vec<_>>(), 2, |index| {
            if *index < 2 {
                barrier.wait();
            }
            if *index == 0 {
                return Err("original failure");
            }
            std::thread::sleep(Duration::from_millis(40));
            finished.fetch_add(1, Ordering::SeqCst);
            Ok(*index)
        });
        assert_eq!(result, Err("original failure"));
        assert_eq!(finished.load(Ordering::SeqCst), 1);
        assert_eq!(
            try_map::<u8, u8, ()>(&[], 4, |_| unreachable!()),
            Ok(vec![])
        );
    }
}
