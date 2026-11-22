//! Deterministic latency budget model for perception -> policy -> actuation pipelines.
//!
//! This is a planning estimate, not a profiler. Sequential latency is the plain sum of
//! stages; summed p95 is a conservative upper bound because real p95s rarely align.

pub const MAX_WORKERS: u32 = 8;
pub const MAX_STAGES: usize = 24;

#[derive(Clone, Debug, PartialEq)]
pub struct Stage {
    pub name: String,
    pub typical_ms: f64,
    pub p95_ms: f64,
    pub workers: u32,
}

impl Stage {
    pub fn new(name: &str, typical_ms: f64, p95_ms: f64, workers: u32) -> Self {
        Stage { name: name.to_string(), typical_ms, p95_ms, workers }.normalized()
    }

    /// Clamp to finite non-negative values, p95 >= typical, workers in 1..=8.
    pub fn normalized(mut self) -> Self {
        self.typical_ms = clean(self.typical_ms);
        self.p95_ms = clean(self.p95_ms).max(self.typical_ms);
        self.workers = self.workers.clamp(1, MAX_WORKERS);
        self
    }

    /// Steady-state service time per frame when `workers` copies run in parallel.
    pub fn effective_ms(&self) -> f64 {
        self.typical_ms / self.workers.max(1) as f64
    }
}

/// Finite, non-negative value; anything else becomes 0.
pub fn clean(v: f64) -> f64 {
    if v.is_finite() && v > 0.0 {
        v
    } else {
        0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pass,
    Warn,
    Fail,
}

impl Status {
    pub fn key(self) -> &'static str {
        match self {
            Status::Pass => "pass",
            Status::Warn => "warn",
            Status::Fail => "fail",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Budget {
    pub target_hz: f64,
    pub deadline_ms: f64,
    pub stages: Vec<Stage>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub typical_total_ms: f64,
    pub p95_total_ms: f64,
    /// Max steady-state rate from the slowest effective stage; `None` if every stage is 0 ms.
    pub max_hz: Option<f64>,
    pub margin_typical_ms: f64,
    pub margin_p95_ms: f64,
    pub bottleneck: Option<usize>,
    /// Bottleneck typical latency / sequential typical total, 0..=1.
    pub bottleneck_share: f64,
    pub status: Status,
}

impl Budget {
    pub fn normalized(&self) -> Budget {
        Budget {
            target_hz: clean(self.target_hz),
            deadline_ms: clean(self.deadline_ms),
            stages: self.stages.iter().cloned().map(Stage::normalized).collect(),
        }
    }

    /// Status rule (boundaries inclusive):
    /// - Fail: typical total > deadline, or max Hz < target Hz.
    /// - Warn: typical fits but p95 total > deadline.
    /// - Pass: p95 total <= deadline and max Hz >= target Hz.
    pub fn report(&self) -> Report {
        let b = self.normalized();
        let typical_total_ms: f64 = b.stages.iter().map(|s| s.typical_ms).sum();
        let p95_total_ms: f64 = b.stages.iter().map(|s| s.p95_ms).sum();

        let mut bottleneck: Option<usize> = None;
        for (i, s) in b.stages.iter().enumerate() {
            // Strict `>` keeps the first stage on ties.
            if bottleneck.is_none_or(|j| s.effective_ms() > b.stages[j].effective_ms()) {
                bottleneck = Some(i);
            }
        }
        let slowest = bottleneck.map_or(0.0, |i| b.stages[i].effective_ms());
        let max_hz = if slowest > 0.0 { Some(1000.0 / slowest) } else { None };
        let bottleneck_share = match bottleneck {
            Some(i) if typical_total_ms > 0.0 => b.stages[i].typical_ms / typical_total_ms,
            _ => 0.0,
        };

        let rate_ok = max_hz.is_none_or(|hz| hz >= b.target_hz);
        let status = if typical_total_ms > b.deadline_ms || !rate_ok {
            Status::Fail
        } else if p95_total_ms > b.deadline_ms {
            Status::Warn
        } else {
            Status::Pass
        };

        Report {
            typical_total_ms,
            p95_total_ms,
            max_hz,
            margin_typical_ms: b.deadline_ms - typical_total_ms,
            margin_p95_ms: b.deadline_ms - p95_total_ms,
            bottleneck,
            bottleneck_share,
            status,
        }
    }

    pub fn to_csv(&self) -> String {
        let b = self.normalized();
        let r = b.report();
        let mut out = String::from("stage,typical_ms,p95_ms,workers,effective_ms,bottleneck\n");
        for (i, s) in b.stages.iter().enumerate() {
            out.push_str(&format!(
                "{},{},{},{},{},{}\n",
                csv_escape(&s.name),
                fmt(s.typical_ms),
                fmt(s.p95_ms),
                s.workers,
                fmt(s.effective_ms()),
                if r.bottleneck == Some(i) { "yes" } else { "no" }
            ));
        }
        out.push_str(&format!("TOTAL,{},{},,,\n", fmt(r.typical_total_ms), fmt(r.p95_total_ms)));
        out.push_str(&format!("target_hz,{}\n", fmt(b.target_hz)));
        out.push_str(&format!("deadline_ms,{}\n", fmt(b.deadline_ms)));
        out.push_str(&format!(
            "max_hz,{}\n",
            r.max_hz.map_or_else(|| "unbounded".to_string(), fmt)
        ));
        out.push_str(&format!("margin_typical_ms,{}\n", fmt(r.margin_typical_ms)));
        out.push_str(&format!("margin_p95_ms,{}\n", fmt(r.margin_p95_ms)));
        out.push_str(&format!("status,{}\n", r.status.key()));
        out
    }
}

/// Round to 3 decimals and drop trailing zeros.
pub fn fmt(v: f64) -> String {
    let s = format!("{:.3}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".to_string() } else { s.to_string() }
}

/// RFC 4180 quoting plus a leading apostrophe against spreadsheet formula injection.
pub fn csv_escape(field: &str) -> String {
    let mut f = field.to_string();
    if f.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        f.insert(0, '\'');
    }
    if f.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", f.replace('"', "\"\""))
    } else {
        f
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    VisionControl,
    LocalVla,
    RemoteAssisted,
}

impl Preset {
    pub const ALL: [Preset; 3] = [Preset::VisionControl, Preset::LocalVla, Preset::RemoteAssisted];

    pub fn key(self) -> &'static str {
        match self {
            Preset::VisionControl => "vision",
            Preset::LocalVla => "vla",
            Preset::RemoteAssisted => "remote",
        }
    }

    /// Illustrative, synthetic numbers only. Stage names are English; the UI may localize.
    pub fn budget(self) -> Budget {
        let s = Stage::new;
        match self {
            Preset::VisionControl => Budget {
                target_hz: 30.0,
                deadline_ms: 33.0,
                stages: vec![
                    s("Sensor capture", 4.0, 5.0, 1),
                    s("Decode", 2.0, 3.0, 1),
                    s("Perception", 9.0, 12.0, 2),
                    s("Policy", 5.0, 7.0, 1),
                    s("Safety filter", 1.0, 1.5, 1),
                    s("Actuation", 2.0, 2.5, 1),
                ],
            },
            Preset::LocalVla => Budget {
                target_hz: 10.0,
                deadline_ms: 100.0,
                stages: vec![
                    s("Sensor capture", 5.0, 8.0, 1),
                    s("Decode", 3.0, 5.0, 1),
                    s("Perception", 12.0, 18.0, 2),
                    s("Policy", 55.0, 85.0, 1),
                    s("Safety filter", 2.0, 3.0, 1),
                    s("Actuation", 3.0, 5.0, 1),
                ],
            },
            Preset::RemoteAssisted => Budget {
                target_hz: 5.0,
                deadline_ms: 150.0,
                stages: vec![
                    s("Sensor capture", 5.0, 8.0, 1),
                    s("Decode", 4.0, 6.0, 1),
                    s("Perception", 15.0, 22.0, 2),
                    s("Policy", 140.0, 260.0, 4),
                    s("Safety filter", 2.0, 3.0, 1),
                    s("Actuation", 4.0, 6.0, 1),
                ],
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget(hz: f64, deadline: f64, stages: Vec<Stage>) -> Budget {
        Budget { target_hz: hz, deadline_ms: deadline, stages }
    }

    #[test]
    fn sums_typical_and_p95() {
        let b = budget(10.0, 100.0, vec![Stage::new("a", 10.0, 15.0, 1), Stage::new("b", 20.0, 30.0, 1)]);
        let r = b.report();
        assert_eq!(r.typical_total_ms, 30.0);
        assert_eq!(r.p95_total_ms, 45.0);
        assert_eq!(r.margin_typical_ms, 70.0);
        assert_eq!(r.margin_p95_ms, 55.0);
    }

    #[test]
    fn normalizes_invalid_values_and_p95() {
        let s = Stage { name: "x".into(), typical_ms: 12.0, p95_ms: 5.0, workers: 0 }.normalized();
        assert_eq!((s.typical_ms, s.p95_ms, s.workers), (12.0, 12.0, 1));
        let s = Stage { name: "y".into(), typical_ms: f64::NAN, p95_ms: -3.0, workers: 99 }.normalized();
        assert_eq!((s.typical_ms, s.p95_ms, s.workers), (0.0, 0.0, 8));
        assert_eq!(clean(f64::INFINITY), 0.0);
    }

    #[test]
    fn throughput_uses_workers_and_picks_first_bottleneck_on_tie() {
        // Effective: 10/1 = 10, 40/4 = 10, 30/2 = 15 -> stage 2 is the bottleneck, 66.67 Hz.
        let b = budget(60.0, 1000.0, vec![
            Stage::new("a", 10.0, 10.0, 1),
            Stage::new("b", 40.0, 40.0, 4),
            Stage::new("c", 30.0, 30.0, 2),
        ]);
        let r = b.report();
        assert_eq!(r.bottleneck, Some(2));
        assert!((r.max_hz.unwrap() - 1000.0 / 15.0).abs() < 1e-9);
        assert!((r.bottleneck_share - 30.0 / 80.0).abs() < 1e-12);
        // Tie: a(10/1) and b(40/4) -> first wins.
        let tie = budget(1.0, 100.0, vec![Stage::new("a", 10.0, 10.0, 1), Stage::new("b", 40.0, 40.0, 4)]);
        assert_eq!(tie.report().bottleneck, Some(0));
    }

    #[test]
    fn empty_or_zero_pipeline_has_no_rate_limit() {
        let r = budget(100.0, 10.0, vec![Stage::new("z", 0.0, 0.0, 1)]).report();
        assert_eq!(r.max_hz, None);
        assert_eq!(r.bottleneck_share, 0.0);
        assert_eq!(r.status, Status::Pass);
        assert_eq!(budget(1.0, 1.0, vec![]).report().bottleneck, None);
    }

    #[test]
    fn deadline_status_boundaries_are_inclusive() {
        let st = |d: f64, hz: f64| budget(hz, d, vec![Stage::new("a", 20.0, 30.0, 1)]).report().status;
        assert_eq!(st(30.0, 50.0), Status::Pass); // p95 == deadline, max Hz == target
        assert_eq!(st(29.9, 50.0), Status::Warn); // typical fits, p95 does not
        assert_eq!(st(20.0, 50.0), Status::Warn); // typical == deadline
        assert_eq!(st(19.9, 50.0), Status::Fail); // typical over deadline
        assert_eq!(st(30.0, 50.1), Status::Fail); // throughput below target
    }

    #[test]
    fn csv_escapes_and_exports() {
        assert_eq!(csv_escape("plain"), "plain");
        assert_eq!(csv_escape("a,b"), "\"a,b\"");
        assert_eq!(csv_escape("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_escape("=1+1"), "'=1+1");
        let csv = budget(10.0, 50.0, vec![Stage::new("cam, left", 1.5, 2.0, 1)]).to_csv();
        assert!(csv.starts_with("stage,typical_ms,p95_ms,workers,effective_ms,bottleneck\n"));
        assert!(csv.contains("\"cam, left\",1.5,2,1,1.5,yes\n"));
        assert!(csv.contains("status,pass\n"));
    }
}
