//! Bounded derivative of viewer-metrics.jsonl. Gaps are never filled with zero.
use super::*;
use crate::streaming::viewer_metrics::ViewerSample;
const MAX_ROWS: u64 = 200_000;
// New API sample holds are at most 15 wall seconds, scaled only when a native
// source mapping exists (playback rate is bounded to 0.25..4 by the clock).
const MAX_HOLD_SECONDS: f64 = 60.0;

pub(super) fn load(session: &Session, connection: &mut Connection) -> Result<(), StreamError> {
    let Some(file) = &session.viewers else {
        return Ok(());
    };
    let mut file = file.try_clone().map_err(|_| storage())?;
    file.seek(SeekFrom::Start(0)).map_err(|_| storage())?;
    let mut reader = BufReader::with_capacity(4096, file);
    let transaction = connection.transaction().map_err(|_| storage())?;
    let mut ordinal = 0u64;
    let mut malformed = 0u64;
    let mut previous = None;
    while let Some(line) = bounded_line(&mut reader, &session.cancel)? {
        ordinal += 1;
        if ordinal > MAX_ROWS {
            malformed += 1;
            break;
        }
        let sample = if !line.oversized && line.terminated && line.bytes.len() <= 2048 {
            serde_json::from_slice::<ViewerSample>(&line.bytes).ok()
        } else {
            None
        };
        let Some(sample) = sample.filter(|sample| sample.valid()) else {
            malformed += 1;
            continue;
        };
        if previous.is_some_and(|last| sample.received_at < last) {
            malformed += 1;
            continue;
        }
        previous = Some(sample.received_at);
        let raw = serde_json::to_value(&sample).map_err(|_| storage())?;
        let mapped = observed_media_time(&transaction, &raw);
        let time = mapped.unwrap_or(sample.offset_seconds);
        let hold = sample.hold_seconds(mapped.is_some());
        if !time.is_finite() || time < 0.0 || time > session.duration + MAX_HOLD_SECONDS {
            malformed += 1;
            continue;
        }
        transaction
            .execute(
                "INSERT INTO viewer_samples VALUES (?1,?2,?3,?4)",
                params![ordinal, time, sample.viewer_count, hold],
            )
            .map_err(|_| storage())?;
    }
    transaction
        .execute("UPDATE metadata SET viewer_malformed=?1", [malformed])
        .map_err(|_| storage())?;
    transaction.commit().map_err(|_| storage())?;
    Ok(())
}

fn accumulate(
    result: &mut ReplayTimeline,
    sums: &mut [f64],
    start: f64,
    end: f64,
    count: Option<u64>,
    hold: f64,
    duration: f64,
) {
    let Some(count) = count else {
        return;
    };
    let mut at = start.max(0.0);
    let end = end.min(duration).min(start + hold);
    while at < end {
        let index = (at / result.bucket_seconds).floor() as usize;
        let Some(bucket) = result.buckets.get_mut(index) else {
            break;
        };
        let boundary = ((index + 1) as f64 * result.bucket_seconds).min(end);
        if boundary <= at {
            break;
        }
        let covered = boundary - at;
        bucket.viewer_coverage_seconds += covered;
        sums[index] += count as f64 * covered;
        at = boundary;
    }
}

pub(super) fn apply(
    session: &Session,
    connection: &Connection,
    offset: f64,
    result: &mut ReplayTimeline,
) -> Result<(), StreamError> {
    let mut sums = vec![0.0; result.buckets.len()];
    let mut previous: Option<(f64, Option<u64>, f64)> = None;
    let mut rows_seen = 0;
    let mut statement = connection.prepare("SELECT media_time,viewer_count,hold_seconds FROM viewer_samples ORDER BY media_time,ordinal LIMIT 200001").map_err(|_| storage())?;
    let mut rows = statement.query([]).map_err(|_| storage())?;
    while let Some(row) = rows.next().map_err(|_| storage())? {
        if session.cancel.load(Ordering::Acquire) {
            return Err(stale());
        }
        rows_seen += 1;
        if rows_seen > MAX_ROWS {
            return Err(storage());
        }
        let time = row.get::<_, f64>(0).map_err(|_| storage())? + offset;
        let count = row.get::<_, Option<u64>>(1).map_err(|_| storage())?;
        let hold = row.get::<_, f64>(2).map_err(|_| storage())?;
        if !time.is_finite()
            || count.is_some_and(|value| value > 100_000_000)
            || !hold.is_finite()
            || !(0.0..=MAX_HOLD_SECONDS).contains(&hold)
        {
            return Err(storage());
        }
        if let Some((start, value, hold)) = previous {
            accumulate(
                result,
                &mut sums,
                start,
                time,
                value,
                hold,
                session.duration,
            );
        }
        if time >= 0.0 && time < session.duration && count.is_some() {
            if let Some(bucket) = result
                .buckets
                .get_mut((time / result.bucket_seconds).floor() as usize)
            {
                bucket.viewer_sample_count += 1;
            }
        }
        previous = Some((time, count, hold));
    }
    if let Some((start, value, hold)) = previous {
        accumulate(
            result,
            &mut sums,
            start,
            session.duration,
            value,
            hold,
            session.duration,
        );
    }
    let malformed: u64 = connection
        .query_row("SELECT viewer_malformed FROM metadata", [], |row| {
            row.get(0)
        })
        .map_err(|_| storage())?;
    let mut covered = 0.0;
    for (bucket, sum) in result.buckets.iter_mut().zip(sums) {
        covered += bucket.viewer_coverage_seconds;
        if bucket.viewer_coverage_seconds > 0.0 {
            bucket.viewer_count = Some((sum / bucket.viewer_coverage_seconds).round() as u64);
        }
    }
    result.viewer_metric_status = if rows_seen == 0 && malformed == 0 {
        "not_recorded"
    } else if malformed == 0 && covered >= session.duration * 0.95 {
        "recorded"
    } else {
        "partial"
    }
    .into();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ten_second_api_samples_cover_the_interval_but_unknown_and_stale_leave_gaps() {
        let mut result = ReplayTimeline {
            bucket_seconds: 2.0,
            buckets: (0..20)
                .map(|i| ReplayTimelineBucket {
                    start_seconds: i as f64 * 2.0,
                    chat_count: 0,
                    unique_sender_count: None,
                    viewer_count: None,
                    viewer_sample_count: 0,
                    viewer_coverage_seconds: 0.0,
                })
                .collect(),
            viewer_metric_status: "not_recorded".into(),
            index_state: "ready".into(),
        };
        let mut sums = vec![0.0; 20];
        accumulate(&mut result, &mut sums, 0.0, 10.0, Some(10), 15.0, 40.0);
        accumulate(&mut result, &mut sums, 10.0, 20.0, Some(0), 15.0, 40.0);
        accumulate(&mut result, &mut sums, 20.0, 24.0, None, 15.0, 40.0);
        accumulate(&mut result, &mut sums, 24.0, 40.0, Some(20), 15.0, 40.0);
        assert!(result.buckets[..10]
            .iter()
            .all(|bucket| bucket.viewer_coverage_seconds == 2.0));
        assert_eq!(sums[5], 0.0); // Observed zero, not missing.
        assert!(result.buckets[10..12]
            .iter()
            .all(|bucket| bucket.viewer_coverage_seconds == 0.0));
        assert_eq!(result.buckets[19].viewer_coverage_seconds, 1.0); // expires at 39s
    }
    #[test]
    fn weighted_viewers_keep_zero_unknown_and_bounded_gaps() {
        let mut result = ReplayTimeline {
            bucket_seconds: 2.0,
            buckets: (0..6)
                .map(|i| ReplayTimelineBucket {
                    start_seconds: i as f64 * 2.0,
                    chat_count: 0,
                    unique_sender_count: None,
                    viewer_count: None,
                    viewer_sample_count: 0,
                    viewer_coverage_seconds: 0.0,
                })
                .collect(),
            viewer_metric_status: "not_recorded".into(),
            index_state: "ready".into(),
        };
        let mut sums = vec![0.0; 6];
        accumulate(&mut result, &mut sums, 0.0, 1.0, Some(0), 6.0, 12.0);
        accumulate(&mut result, &mut sums, 1.0, 2.0, Some(100), 6.0, 12.0);
        accumulate(&mut result, &mut sums, 2.0, 4.0, None, 6.0, 12.0);
        accumulate(&mut result, &mut sums, 4.0, 12.0, Some(50), 6.0, 12.0);
        assert_eq!(sums[0] / result.buckets[0].viewer_coverage_seconds, 50.0);
        assert_eq!(result.buckets[1].viewer_coverage_seconds, 0.0);
        assert_eq!(result.buckets[4].viewer_coverage_seconds, 2.0);
        assert_eq!(result.buckets[5].viewer_coverage_seconds, 0.0);
    }
}
