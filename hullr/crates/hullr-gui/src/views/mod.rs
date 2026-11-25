//! One module per page, plus the floating dialogs and toasts.

pub mod compose;
pub mod containers;
pub mod dialogs;
pub mod images;
pub mod system;
pub mod volumes;

use crate::state::Level;
use crate::sys;
use crate::state::AppState;

/// Open a URL or folder, toasting on failure.
pub fn open(st: &mut AppState, target: &str) {
    if let Err(e) = sys::open_target(target) {
        st.toasts.push(Level::Error, e, st.now);
    }
}

#[cfg(test)]
mod tests {
    //! Headless smoke tests: run every page and dialog through a real egui
    //! context for a few frames, so layout panics (duplicate ids, bad
    //! allocations) are caught without opening a window.

    use super::*;
    use crate::backend::{Backend, Notifier};
    use crate::mock::MockApi;
    use crate::state::*;
    use crate::testutil::*;
    use eframe::egui;
    use hullr_core::ContainerState;
    use std::sync::Arc;

    fn populated() -> AppState {
        let mut st = AppState::new();
        st.apply(Event::Containers(Ok(vec![container("web", ContainerState::Running), container("db", ContainerState::Exited)])), 0.0);
        st.apply(Event::Images(Ok(vec![image("docker.io/library/nginx:latest")])), 0.0);
        st.apply(Event::Volumes(Ok(vec![volume("pgdata")])), 0.0);
        st.apply(
            Event::Compose(Ok(vec![hullr_core::ComposeStatus {
                project: "shop".into(),
                services: vec![
                    hullr_core::ComposeService { service: "web".into(), container: Some(container("shop-web", ContainerState::Running)) },
                    hullr_core::ComposeService { service: "db".into(), container: None },
                ],
            }])),
            0.0,
        );
        st.apply(
            Event::Info(Ok(hullr_core::SystemInfo {
                version: "0.1.0".into(),
                os: "linux".into(),
                host_platform: platform(),
                mode: "vm".into(),
                data_root: "/x".into(),
                rootless: true,
                emulated_platforms: vec!["linux/amd64".into()],
                containers: 2,
                running: 1,
                images: 1,
                warnings: vec!["no pasta, networking is limited".into()],
            })),
            0.0,
        );
        st
    }

    fn frames(st: &mut AppState, be: &Backend, n: usize, f: impl Fn(&egui::Context, &mut AppState, &Backend)) {
        let ctx = egui::Context::default();
        for i in 0..n {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1100.0, 700.0))),
                time: Some(i as f64 * 0.1),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| f(ctx, st, be));
        }
    }

    fn backend() -> (Backend, tokio::runtime::Runtime) {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        let (tx, _rx) = std::sync::mpsc::channel();
        let be = Backend::new(Arc::new(MockApi::sample()), rt.handle().clone(), Notifier::new(tx, Arc::new(|| {})));
        (be, rt)
    }

    fn page(st: &mut AppState, be: &Backend, p: Page) {
        st.page = p;
        frames(st, be, 3, |ctx, st, be| {
            egui::CentralPanel::default().show(ctx, |ui| match st.page {
                Page::Containers => containers::show(ui, st, be),
                Page::Images => images::show(ui, st, be),
                Page::Volumes => volumes::show(ui, st, be),
                Page::Compose => compose::show(ui, st, be),
                Page::System => system::show(ui, st, be),
            });
            dialogs::show_all(ctx, st, be);
        });
    }

    #[test]
    fn every_page_draws_populated() {
        let (be, _rt) = backend();
        let mut st = populated();
        for p in Page::ALL {
            page(&mut st, &be, p);
        }
    }

    #[test]
    fn every_page_draws_empty_and_errored() {
        let (be, _rt) = backend();
        let mut st = AppState::new();
        for p in Page::ALL {
            page(&mut st, &be, p);
        }
        st.apply(Event::Containers(Err("refused".into())), 0.0);
        st.apply(Event::Images(Err("refused".into())), 0.0);
        for p in Page::ALL {
            page(&mut st, &be, p);
        }
        st.apply(Event::Containers(Ok(vec![])), 0.0);
        st.apply(Event::Images(Ok(vec![])), 0.0);
        st.apply(Event::Volumes(Ok(vec![])), 0.0);
        st.apply(Event::Compose(Ok(vec![])), 0.0);
        for p in Page::ALL {
            page(&mut st, &be, p);
        }
    }

    #[test]
    fn container_detail_tabs_draw() {
        let (be, _rt) = backend();
        let mut st = populated();
        let id = st.data.containers.iter().find(|c| c.name == "web").unwrap().id.clone();
        st.selected = Some(id.clone());
        st.logs.id = Some(id.clone());
        st.stats.id = Some(id.clone());
        for i in 0..50 {
            st.apply(Event::Log { id: id.clone(), stderr: i % 7 == 0, line: format!("line {i}") }, 0.0);
            st.apply(Event::Stats { id: id.clone(), stats: hullr_core::Stats { cpu_percent: i as f64, memory_bytes: 1_000_000 * i, memory_limit: Some(100_000_000), pids: 3, ..Default::default() } }, 0.0);
        }
        for t in [DetailTab::Overview, DetailTab::Logs, DetailTab::Stats] {
            st.detail_tab = t;
            page(&mut st, &be, Page::Containers);
        }
        st.logs.search = "line 4".into();
        st.detail_tab = DetailTab::Logs;
        page(&mut st, &be, Page::Containers);
        st.selected = Some(st.data.containers.iter().find(|c| c.name == "db").unwrap().id.clone());
        for t in [DetailTab::Overview, DetailTab::Logs, DetailTab::Stats] {
            st.detail_tab = t;
            page(&mut st, &be, Page::Containers);
        }
    }

    #[test]
    fn dialogs_draw() {
        let (be, _rt) = backend();
        let mut st = populated();
        st.run = Some(RunDialog::for_image("nginx"));
        st.run.as_mut().unwrap().ports.push(PortRow { host: "80".into(), container: "80".into(), udp: false });
        st.run.as_mut().unwrap().error = Some("oops".into());
        page(&mut st, &be, Page::Containers);
        st.run = None;
        st.pull = Some(PullDialog::default());
        page(&mut st, &be, Page::Images);
        st.pull = None;
        st.compose_path = Some(String::new());
        page(&mut st, &be, Page::Compose);
        st.compose_path = None;
        st.confirm = Some(Confirm::Prune { deep: true });
        page(&mut st, &be, Page::System);
        st.confirm = None;
        let op = st.start_op("Pulling alpine");
        st.apply(Event::Progress { op, progress: hullr_core::api::Progress::Layer { id: "sha256:aaaa".into(), current: 1, total: 4, done: false } }, 0.0);
        let op2 = st.start_op("Compose up");
        st.apply(Event::Done { op: Some(op2), result: Err("nope".into()), then: Then::Nothing }, 0.0);
        page(&mut st, &be, Page::Images);
        assert!(!st.toasts.items.is_empty());
    }
}
