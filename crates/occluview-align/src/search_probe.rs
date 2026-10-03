//! Optional serial phase measurements, excluded from registration decisions.
//!
//! Enable `search-probe` for synthetic measurements. Nested scopes report
//! exclusive wall time and charged counter deltas, including interrupted work.
//! Output contains aggregate work only, never geometry or source identifiers.

use occluview_geometry::surface::GeometryControl;

#[derive(Clone, Copy)]
pub(crate) enum Phase {
    Preparation,
    Indices,
    Sampling,
    Descriptors,
    Proposals,
    Scoring,
    Coarse,
    Middle,
    Dense,
    Verification,
}

pub(crate) struct Session;
pub(crate) struct Span {
    #[cfg(feature = "search-probe")]
    measurement: enabled::Measurement,
}

impl Session {
    pub(crate) fn new() -> Self {
        #[cfg(feature = "search-probe")]
        enabled::reset();
        Self
    }
}

impl Span {
    pub(crate) fn new(phase: Phase, control: &GeometryControl) -> Self {
        #[cfg(not(feature = "search-probe"))]
        let _ = (phase, control);
        Self {
            #[cfg(feature = "search-probe")]
            measurement: enabled::Measurement::new(phase, control),
        }
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        #[cfg(feature = "search-probe")]
        self.measurement.finish();
    }
}

#[cfg(feature = "search-probe")]
impl Drop for Session {
    fn drop(&mut self) {
        enabled::publish();
    }
}

#[cfg(feature = "search-probe")]
mod enabled {
    use super::{GeometryControl, Phase};
    use std::{cell::RefCell, io::Write, time::Instant};

    #[derive(Clone, Copy, Default)]
    struct Cost {
        seconds: f64,
        queries: u64,
        triangles: u64,
        pairs: u64,
        operations: u64,
        calls: u64,
    }
    impl Cost {
        fn add(self, other: Self) -> Self {
            Self {
                seconds: self.seconds + other.seconds,
                queries: self.queries.saturating_add(other.queries),
                triangles: self.triangles.saturating_add(other.triangles),
                pairs: self.pairs.saturating_add(other.pairs),
                operations: self.operations.saturating_add(other.operations),
                calls: self.calls.saturating_add(other.calls),
            }
        }
        fn subtract(self, other: Self) -> Self {
            Self {
                seconds: (self.seconds - other.seconds).max(0.),
                queries: self.queries.saturating_sub(other.queries),
                triangles: self.triangles.saturating_sub(other.triangles),
                pairs: self.pairs.saturating_sub(other.pairs),
                operations: self.operations.saturating_sub(other.operations),
                calls: self.calls,
            }
        }
    }
    thread_local! {
        static COSTS: RefCell<[Cost; 10]> = const { RefCell::new([Cost {
            seconds: 0., queries: 0, triangles: 0, pairs: 0, operations: 0, calls: 0
        }; 10]) };
    }
    fn sum() -> Cost {
        COSTS.with(|costs| {
            costs
                .borrow()
                .iter()
                .copied()
                .fold(Cost::default(), Cost::add)
        })
    }
    fn counters(control: &GeometryControl) -> Cost {
        let c = control.counters();
        Cost {
            queries: c.query_calls,
            triangles: c.triangle_tests,
            pairs: c.point_pair_tests,
            operations: c.operations,
            ..Cost::default()
        }
    }
    pub(super) struct Measurement {
        phase: Phase,
        control: GeometryControl,
        started: Instant,
        before: Cost,
        children: Cost,
    }
    impl Measurement {
        pub(super) fn new(phase: Phase, control: &GeometryControl) -> Self {
            Self {
                phase,
                control: control.clone(),
                started: Instant::now(),
                before: counters(control),
                children: sum(),
            }
        }
        pub(super) fn finish(&self) {
            let mut delta = counters(&self.control).subtract(self.before);
            delta.seconds = self.started.elapsed().as_secs_f64();
            delta = delta.subtract(sum().subtract(self.children));
            delta.calls = 1;
            COSTS.with(|costs| {
                let mut costs = costs.borrow_mut();
                let slot = &mut costs[self.phase as usize];
                *slot = slot.add(delta);
            });
        }
    }
    pub(super) fn reset() {
        COSTS.with(|costs| *costs.borrow_mut() = [Cost::default(); 10]);
    }
    pub(super) fn publish() {
        const NAMES: [&str; 10] = [
            "preparation",
            "indices",
            "sampling",
            "descriptors",
            "proposals",
            "scoring",
            "coarse",
            "middle",
            "dense",
            "verification",
        ];
        COSTS.with(|costs| {
            let stderr = std::io::stderr();
            let mut output = stderr.lock();
            for (name, c) in NAMES.into_iter().zip(costs.borrow().iter()) {
                let _ = writeln!(output,
                    "ALIGN_PERF {name} calls={} seconds={:.6} queries={} triangles={} pairs={} operations={}",
                    c.calls, c.seconds, c.queries, c.triangles, c.pairs, c.operations);
            }
        });
    }
}
