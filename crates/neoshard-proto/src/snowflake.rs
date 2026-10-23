//! Snowflake IDs (ADR-0005): 42 bits of milliseconds since [`EPOCH_MS`],
//! 10 bits of worker id, 12 bits of per-millisecond sequence.

use std::sync::Mutex;

/// 2026-01-01T00:00:00Z in unix milliseconds.
pub const EPOCH_MS: u64 = 1_767_225_600_000;

const WORKER_BITS: u64 = 10;
const SEQ_BITS: u64 = 12;
const MAX_WORKER: u64 = (1 << WORKER_BITS) - 1;
const MAX_SEQ: u64 = (1 << SEQ_BITS) - 1;

/// Unix millis at which this snowflake was created.
pub fn timestamp_ms(id: u64) -> u64 {
    (id >> (WORKER_BITS + SEQ_BITS)) + EPOCH_MS
}

/// Worker id encoded in the snowflake.
pub fn worker(id: u64) -> u16 {
    ((id >> SEQ_BITS) & MAX_WORKER) as u16
}

/// Smallest snowflake that could have been generated at `unix_ms`.
/// Useful for "messages after time T" range queries.
pub fn from_timestamp(unix_ms: u64) -> u64 {
    unix_ms.saturating_sub(EPOCH_MS) << (WORKER_BITS + SEQ_BITS)
}

/// Thread-safe snowflake generator. Monotonic even if the wall clock steps back.
pub struct Generator {
    worker: u64,
    clock: fn() -> u64,
    state: Mutex<(u64, u64)>, // (last ms since epoch, seq)
}

fn system_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(EPOCH_MS)
}

impl Generator {
    /// # Panics
    /// If `worker` does not fit in 10 bits.
    pub fn new(worker: u16) -> Self {
        Self::with_clock(worker, system_unix_ms)
    }

    /// Generator with a custom unix-millis clock (wasm targets, tests).
    pub fn with_clock(worker: u16, clock: fn() -> u64) -> Self {
        assert!(u64::from(worker) <= MAX_WORKER, "worker id must be < 1024");
        Self {
            worker: u64::from(worker),
            clock,
            state: Mutex::new((0, 0)),
        }
    }

    pub fn next(&self) -> u64 {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut now = (self.clock)().saturating_sub(EPOCH_MS).max(st.0);
        if now == st.0 {
            st.1 += 1;
            if st.1 > MAX_SEQ {
                // Sequence exhausted for this millisecond: borrow the next one.
                now += 1;
                st.1 = 0;
            }
        } else {
            st.1 = 0;
        }
        st.0 = now;
        (now << (WORKER_BITS + SEQ_BITS)) | (self.worker << SEQ_BITS) | st.1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_are_unique_and_increasing() {
        let g = Generator::new(7);
        let mut prev = 0;
        let mut seen = HashSet::new();
        for _ in 0..100_000 {
            let id = g.next();
            assert!(id > prev);
            assert!(seen.insert(id));
            prev = id;
        }
    }

    #[test]
    fn roundtrip_fields() {
        let g = Generator::new(513);
        let id = g.next();
        assert_eq!(worker(id), 513);
        let ts = timestamp_ms(id);
        assert!(system_unix_ms().abs_diff(ts) < 5_000);
        assert!(from_timestamp(ts) <= id);
    }

    #[test]
    fn monotonic_when_clock_goes_back() {
        fn frozen() -> u64 {
            EPOCH_MS + 1000
        }
        let g = Generator::with_clock(1, frozen);
        let a = g.next();
        // 5000 ids in one "millisecond" overflow the 12-bit sequence.
        let mut last = a;
        for _ in 0..5000 {
            let n = g.next();
            assert!(n > last);
            last = n;
        }
    }

    #[test]
    #[should_panic]
    fn rejects_large_worker() {
        let _ = Generator::new(1024);
    }
}
