//! Seeder registry — the static `phf::Map<&'static str, Cadence>`
//! from SPEC-001 §17.7.
//!
//! Every production seeder registers itself here at compile time
//! with its cadence. The scheduler (`crate::scheduler`) iterates
//! the map at boot and spawns one `tokio::time::interval` task
//! per entry.
//!
//! Cadences below match the spec's documented cadence set —
//! they are the same intervals the original WorldMonitor seed
//! scheduler used (`scripts/_seed-utils.mjs` registry).

use std::time::Duration;

/// One cadence — how often a seeder should run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cadence {
    /// Interval between runs.
    pub period: Duration,
    /// Optional initial delay before the first run. Useful for
    /// staggering seeders so a fresh boot doesn't fire 30 of
    /// them in the same tick.
    pub initial_delay: Duration,
}

impl Cadence {
    /// Construct from a period (no initial delay).
    #[must_use]
    pub const fn every(period: Duration) -> Self {
        Self {
            period,
            initial_delay: Duration::from_secs(0),
        }
    }

    /// Override the initial delay.
    #[must_use]
    pub const fn with_initial_delay(mut self, delay: Duration) -> Self {
        self.initial_delay = delay;
        self
    }
}

/// `(seeder_name, cadence)` entry. Public so tests + the
/// scheduler can iterate without going through a typed map.
#[derive(Clone, Copy, Debug)]
pub struct RegistryEntry {
    /// Stable seeder name — used as the registry key + the
    /// `metrics::counter!` label.
    pub name: &'static str,
    /// How often the seeder runs.
    pub cadence: Cadence,
}

/// Static registry of every production seeder. Mirrors the
/// SPEC-001 §17.7 cadence set.
pub const REGISTRY: &[RegistryEntry] = &[
    RegistryEntry {
        name: "market",
        cadence: Cadence::every(Duration::from_secs(5 * 60)),
    },
    RegistryEntry {
        name: "aviation",
        cadence: Cadence::every(Duration::from_secs(30 * 60)),
    },
    RegistryEntry {
        name: "notam",
        cadence: Cadence::every(Duration::from_secs(2 * 60 * 60)),
    },
    RegistryEntry {
        name: "cyber",
        cadence: Cadence::every(Duration::from_secs(2 * 60 * 60)),
    },
    RegistryEntry {
        name: "positive-events",
        cadence: Cadence::every(Duration::from_secs(15 * 60)),
    },
    RegistryEntry {
        name: "theater-posture",
        cadence: Cadence::every(Duration::from_secs(5 * 60)),
    },
    RegistryEntry {
        name: "ucdp",
        cadence: Cadence::every(Duration::from_secs(30 * 60)),
    },
    RegistryEntry {
        name: "corridor-risk",
        cadence: Cadence::every(Duration::from_secs(60 * 60)),
    },
    RegistryEntry {
        name: "shipping-stress",
        cadence: Cadence::every(Duration::from_secs(60 * 60)),
    },
    RegistryEntry {
        name: "satellite-tles",
        cadence: Cadence::every(Duration::from_secs(2 * 60 * 60)),
    },
    RegistryEntry {
        name: "worldbank",
        cadence: Cadence::every(Duration::from_secs(24 * 60 * 60)),
    },
];

/// Look up a registry entry by name. `None` for unknown names.
#[must_use]
pub fn lookup(name: &str) -> Option<&'static RegistryEntry> {
    REGISTRY.iter().find(|e| e.name == name)
}

/// Total number of registered seeders.
#[must_use]
pub const fn registered_count() -> usize {
    REGISTRY.len()
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn registry_has_no_duplicate_names() {
        let names: HashSet<&str> = REGISTRY.iter().map(|e| e.name).collect();
        assert_eq!(names.len(), REGISTRY.len());
    }

    #[test]
    fn registry_includes_documented_cadences() {
        // SPEC-001 §17.7 spot-check.
        assert_eq!(
            lookup("market").unwrap().cadence.period,
            Duration::from_secs(5 * 60)
        );
        assert_eq!(
            lookup("aviation").unwrap().cadence.period,
            Duration::from_secs(30 * 60)
        );
        assert_eq!(
            lookup("theater-posture").unwrap().cadence.period,
            Duration::from_secs(5 * 60)
        );
        assert_eq!(
            lookup("worldbank").unwrap().cadence.period,
            Duration::from_secs(24 * 60 * 60)
        );
    }

    #[test]
    fn lookup_returns_none_for_unknown() {
        assert!(lookup("totally-fake-seeder").is_none());
        assert!(lookup("").is_none());
    }

    #[test]
    fn registered_count_matches_registry_length() {
        assert_eq!(registered_count(), REGISTRY.len());
    }

    #[test]
    fn cadence_every_constructs_with_zero_initial_delay() {
        let c = Cadence::every(Duration::from_secs(60));
        assert_eq!(c.period, Duration::from_secs(60));
        assert_eq!(c.initial_delay, Duration::from_secs(0));
    }

    #[test]
    fn cadence_with_initial_delay_overrides() {
        let c = Cadence::every(Duration::from_secs(60))
            .with_initial_delay(Duration::from_secs(10));
        assert_eq!(c.initial_delay, Duration::from_secs(10));
    }

    #[test]
    fn every_cadence_is_at_least_one_minute() {
        // Sanity guard: nothing in the spec runs faster than 5
        // minutes. A 5-second cadence in production would melt
        // the upstream rate budgets.
        for entry in REGISTRY {
            assert!(
                entry.cadence.period >= Duration::from_secs(60),
                "{}: cadence {:?} is too aggressive",
                entry.name,
                entry.cadence.period,
            );
        }
    }
}
