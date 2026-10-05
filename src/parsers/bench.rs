//! sysbench and stress-ng result parsers.

use super::{kv_get, leading_num, parse_kv};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SysbenchResult {
    pub threads: Option<u32>,
    pub events_per_sec: Option<f64>,
    pub total_events: Option<u64>,
    pub total_time_s: Option<f64>,
    pub latency_avg_ms: Option<f64>,
    pub latency_p95_ms: Option<f64>,
    pub latency_max_ms: Option<f64>,
}

/// `sysbench cpu ... run`
pub fn parse_sysbench(text: &str) -> Option<SysbenchResult> {
    let kv = parse_kv(text);
    let n = |k: &str| kv_get(&kv, k).and_then(leading_num);
    let r = SysbenchResult {
        threads: n("Number of threads").map(|v| v as u32),
        events_per_sec: n("events per second"),
        total_events: n("total number of events").map(|v| v as u64),
        total_time_s: n("total time"),
        latency_avg_ms: n("avg"),
        latency_p95_ms: n("95th percentile"),
        latency_max_ms: n("max"),
    };
    r.events_per_sec.is_some().then_some(r)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StressMetric {
    pub stressor: String,
    pub bogo_ops: f64,
    pub real_time_s: f64,
    pub usr_time_s: f64,
    pub sys_time_s: f64,
    pub bogo_ops_per_s_real: f64,
    pub bogo_ops_per_s_cpu: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StressResult {
    pub metrics: Vec<StressMetric>,
    pub passed: Option<String>,
    pub failed: Option<String>,
    pub completed: bool,
}

/// `stress-ng ... --metrics-brief` (lines prefixed "stress-ng: <lvl>: [pid]").
pub fn parse_stress_ng(text: &str) -> StressResult {
    let mut r = StressResult::default();
    for l in text.lines() {
        let Some(body) = l.split_once("] ").map(|(_, b)| b.trim()) else { continue };
        if body.starts_with("successful run completed") {
            r.completed = true;
        } else if let Some(p) = body.strip_prefix("passed:") {
            r.passed = Some(p.trim().into());
        } else if let Some(f) = body.strip_prefix("failed:") {
            r.failed = Some(f.trim().into());
        } else {
            let c: Vec<&str> = body.split_whitespace().collect();
            if c.len() == 7 && c[1..].iter().all(|v| v.parse::<f64>().is_ok()) {
                let f = |i: usize| c[i].parse().unwrap_or(0.0);
                r.metrics.push(StressMetric {
                    stressor: c[0].into(),
                    bogo_ops: f(1),
                    real_time_s: f(2),
                    usr_time_s: f(3),
                    sys_time_s: f(4),
                    bogo_ops_per_s_real: f(5),
                    bogo_ops_per_s_cpu: f(6),
                });
            }
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sysbench() {
        let t = "Running the test with following options:\nNumber of threads: 2\n\nCPU speed:\n    events per second:  2154.01\n\nGeneral statistics:\n    total time:                          2.0006s\n    total number of events:              4318\n\nLatency (ms):\n         min:                                    0.90\n         avg:                                    0.93\n         max:                                    1.92\n         95th percentile:                        0.94\n";
        let r = parse_sysbench(t).expect("parses");
        assert_eq!(r.threads, Some(2));
        assert_eq!(r.events_per_sec, Some(2154.01));
        assert_eq!(r.total_events, Some(4318));
        assert_eq!(r.total_time_s, Some(2.0006));
        assert_eq!(r.latency_p95_ms, Some(0.94));
        assert!(parse_sysbench("FATAL: error").is_none());
    }

    #[test]
    fn stress() {
        let t = "stress-ng: info:  [268685] dispatching hogs: 1 cpu\nstress-ng: metrc: [268685] stressor       bogo ops real time  usr time  sys time   bogo ops/s     bogo ops/s\nstress-ng: metrc: [268685]                           (secs)    (secs)    (secs)   (real time) (usr+sys time)\nstress-ng: metrc: [268685] cpu                1858      2.00      1.99      0.00       928.76         932.83\nstress-ng: info:  [268685] passed: 1: cpu (1)\nstress-ng: info:  [268685] failed: 0\nstress-ng: info:  [268685] successful run completed in 2.00 secs\n";
        let r = parse_stress_ng(t);
        assert_eq!(r.metrics.len(), 1);
        assert_eq!(r.metrics[0].bogo_ops, 1858.0);
        assert_eq!(r.metrics[0].bogo_ops_per_s_real, 928.76);
        assert!(r.completed);
        assert_eq!(r.failed.as_deref(), Some("0"));
    }
}
