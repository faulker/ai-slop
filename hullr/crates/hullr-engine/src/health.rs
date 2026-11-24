//! Healthchecks. The daemon probes on each check's interval; without a
//! daemon, compose probes while waiting on `service_healthy` dependencies.

use crate::{records, records::Record, Engine};
use hullr_core::stream::{read_frame, write_frame, Frame};
use hullr_core::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn probe_cmd(h: &HealthCheck) -> Option<Vec<String>> {
    match h.test.first().map(String::as_str) {
        Some("CMD") => Some(h.test[1..].to_vec()),
        Some("CMD-SHELL") => Some(vec!["/bin/sh".into(), "-c".into(), h.test[1..].join(" ")]),
        Some("NONE") | None => None,
        // A bare list is treated like CMD.
        Some(_) => Some(h.test.clone()),
    }
}

/// Run the healthcheck once. Ok(true) = healthy.
pub async fn probe(engine: &Engine, rec: &Record) -> Result<bool> {
    let Some(h) = &rec.healthcheck else { return Ok(true) };
    let Some(cmd) = probe_cmd(h) else { return Ok(true) };
    let spec = ExecSpec { cmd, ..Default::default() };
    let run = async {
        let mut s = engine.runtime.exec(&rec.id, &spec).await?;
        let _ = write_frame(&mut s, &Frame::CloseStdin).await;
        loop {
            match read_frame(&mut s).await? {
                Some(Frame::Exit(code)) => return Ok::<i32, Error>(code),
                Some(_) => {}
                None => return Ok(1),
            }
        }
    };
    match tokio::time::timeout(Duration::from_secs(h.timeout_secs.max(1)), run).await {
        Ok(Ok(code)) => Ok(code == 0),
        Ok(Err(_)) | Err(_) => Ok(false),
    }
}

/// Probe until healthy, honoring start_period and retries; fails after
/// `limit` or when the container exits.
pub async fn wait_healthy(engine: &Engine, id: &str, limit: Duration) -> Result<()> {
    let rec = engine.find_record(id)?;
    let Some(h) = rec.healthcheck.clone() else { return Ok(()) };
    let started = Instant::now();
    let interval = Duration::from_secs(h.interval_secs.clamp(1, 2));
    let mut failures = 0;
    loop {
        if !engine.is_running(&rec.id) {
            return Err(Error::Runtime(format!("{} exited before becoming healthy", rec.name)));
        }
        if probe(engine, &rec).await? {
            records::set_health(&engine.paths, &rec.id, Some(Health::Healthy))?;
            return Ok(());
        }
        if started.elapsed() > Duration::from_secs(h.start_period_secs) {
            failures += 1;
        }
        if started.elapsed() > limit || (h.retries > 0 && failures > h.retries * 10) {
            records::set_health(&engine.paths, &rec.id, Some(Health::Unhealthy))?;
            return Err(Error::Runtime(format!("{} did not become healthy", rec.name)));
        }
        tokio::time::sleep(interval).await;
    }
}

/// Daemon task: keep every running container's health status current.
pub async fn health_loop(engine: Arc<Engine>) {
    struct Track {
        next: Instant,
        failures: u32,
        started: Instant,
    }
    let mut tracks: HashMap<String, Track> = HashMap::new();
    loop {
        let recs = records::list(&engine.paths).unwrap_or_default();
        let now = Instant::now();
        for r in recs.iter().filter(|r| r.healthcheck.is_some()) {
            if !engine.is_running(&r.id) {
                tracks.remove(&r.id);
                continue;
            }
            let h = r.healthcheck.as_ref().expect("filtered");
            let t = tracks.entry(r.id.clone()).or_insert(Track { next: now, failures: 0, started: now });
            if now < t.next {
                continue;
            }
            t.next = now + Duration::from_secs(h.interval_secs.max(1));
            let ok = probe(&engine, r).await.unwrap_or(false);
            let status = if ok {
                t.failures = 0;
                Health::Healthy
            } else if t.started.elapsed() < Duration::from_secs(h.start_period_secs) {
                Health::Starting
            } else {
                t.failures += 1;
                if t.failures >= h.retries.max(1) {
                    Health::Unhealthy
                } else {
                    r.health.unwrap_or(Health::Starting)
                }
            };
            let _ = records::set_health(&engine.paths, &r.id, Some(status));
        }
        engine.collect_garbage();
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_commands() {
        let h = |t: &[&str]| HealthCheck {
            test: t.iter().map(|s| s.to_string()).collect(),
            interval_secs: 1,
            timeout_secs: 1,
            retries: 1,
            start_period_secs: 0,
        };
        assert_eq!(probe_cmd(&h(&["CMD", "true"])).unwrap(), vec!["true"]);
        assert_eq!(probe_cmd(&h(&["CMD-SHELL", "curl -f x"])).unwrap()[2], "curl -f x");
        assert!(probe_cmd(&h(&["NONE"])).is_none());
    }
}
