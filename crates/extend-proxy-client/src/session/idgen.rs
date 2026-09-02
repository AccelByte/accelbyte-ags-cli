//! Collision-free stream ID generation. Ported from Go's `pkg/tunnel/idgen.go`.

use std::collections::HashSet;

/// The stream ID value at which a counter wraps back to its starting value.
/// Mirrors `idgenOverflowThreshold` (`math.MaxUint64 - 1000`).
const OVERFLOW_THRESHOLD: u64 = u64::MAX - 1000;

/// Which parity of stream ID this generator produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdParity {
    /// Sidecar-initiated streams (1, 3, 5, …).
    Odd,
    /// Client-initiated streams (2, 4, 6, …).
    Even,
}

/// Produces collision-free stream IDs for one endpoint. Sidecar uses odd IDs;
/// client uses even IDs — mirroring the HTTP/2 §5.1.1 partitioning strategy,
/// eliminating any possibility of collision without coordination. Mirrors
/// Go's `IDGenerator`.
pub struct IdGenerator {
    current: u64,
    start: u64,
}

impl IdGenerator {
    /// Creates a generator for the given parity, starting at 1 (odd) or 2 (even).
    pub fn new(parity: IdParity) -> Self {
        let start = match parity {
            IdParity::Odd => 1,
            IdParity::Even => 2,
        };
        Self {
            current: start,
            start,
        }
    }

    /// Returns the next available stream ID, skipping any IDs still present
    /// in `active`. Resets to the starting value when nearing `u64::MAX`.
    pub fn next(&mut self, active: &HashSet<u64>) -> u64 {
        loop {
            if self.current >= OVERFLOW_THRESHOLD {
                tracing::warn!(
                    reset_to = self.start,
                    "stream ID counter nearing overflow, resetting"
                );
                self.current = self.start;
            }
            let candidate = self.current;
            self.current += 2;
            if !active.contains(&candidate) {
                return candidate;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Translated from `idgen_test.go`'s `TestOddGenerator`.
    #[test]
    fn test_odd_generator() {
        let mut g = IdGenerator::new(IdParity::Odd);
        let active = HashSet::new();
        let ids = [g.next(&active), g.next(&active), g.next(&active)];
        for id in ids {
            assert_eq!(id % 2, 1, "expected odd ID, got {id}");
        }
        assert_eq!(ids, [1, 3, 5]);
    }

    /// Translated from `idgen_test.go`'s `TestEvenGenerator`.
    #[test]
    fn test_even_generator() {
        let mut g = IdGenerator::new(IdParity::Even);
        let active = HashSet::new();
        let ids = [g.next(&active), g.next(&active), g.next(&active)];
        for id in ids {
            assert_eq!(id % 2, 0, "expected even ID, got {id}");
        }
        assert_eq!(ids, [2, 4, 6]);
    }

    /// Translated from `idgen_test.go`'s `TestGeneratorSkipsActiveIDs`.
    #[test]
    fn test_generator_skips_active_ids() {
        let mut g = IdGenerator::new(IdParity::Odd);
        let active: HashSet<u64> = [1, 3].into_iter().collect();
        assert_eq!(g.next(&active), 5);
    }
}
