//! The eframe application: owns the state, drains backend events, keeps the
//! container watchers in sync with the selection, and draws.

use crate::backend::{Backend, Notifier, TaskGuard};
use crate::state::*;
use crate::views::{compose, containers, dialogs, images, system, volumes};
use crate::{theme, widgets};
use eframe::egui::{self, RichText};
use hullr_core::{Api, ContainerState};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};

const REFRESH_EVERY: Duration = Duration::from_secs(2);

/// What the watchers currently follow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchKey {
    pub id: String,
    pub logs: bool,
    pub stats: bool,
    pub started_at: Option<i64>,
}

/// Decide what to follow for the current selection and tab.
pub fn watch_key(st: &AppState) -> Option<WatchKey> {
    let c = st.selected_container()?;
    Some(WatchKey {
        id: c.id.clone(),
        logs: st.detail_tab == DetailTab::Logs,
        stats: st.detail_tab == DetailTab::Stats && c.state == ContainerState::Running,
        started_at: c.started_at,
    })
}

pub struct HullrApp {
    pub st: AppState,
    be: Backend,
    rx: Receiver<Event>,
    last_refresh: Instant,
    watching: Option<WatchKey>,
    guards: Vec<TaskGuard>,
    _events: TaskGuard,
    applied_theme: Option<ThemeChoice>,
}

impl HullrApp {
    pub fn new(cc: &eframe::CreationContext<'_>, api: Arc<dyn Api>, rt: tokio::runtime::Handle) -> Self {
        theme::install(&cc.egui_ctx);
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = cc.egui_ctx.clone();
        let be = Backend::new(api, rt, Notifier::new(tx, Arc::new(move || ctx.request_repaint())));
        let mut st = AppState::new();
        st.known_path = KnownProjects::default_path();
        if let Some(p) = &st.known_path {
            st.known = KnownProjects::load(p);
        }
        be.refresh(st.show_all);
        let events = be.watch_engine_events();
        Self { st, be, rx, last_refresh: Instant::now(), watching: None, guards: Vec::new(), _events: events, applied_theme: None }
    }

    fn sync_watchers(&mut self) {
        let want = watch_key(&self.st);
        if want == self.watching {
            return;
        }
        self.guards.clear();
        self.st.logs = LogView { id: None, search: std::mem::take(&mut self.st.logs.search), autoscroll: self.st.logs.autoscroll, ..Default::default() };
        self.st.stats = StatsHistory::default();
        if let Some(k) = &want {
            if k.logs {
                self.st.logs.id = Some(k.id.clone());
                self.guards.push(self.be.follow_logs(k.id.clone()));
            }
            if k.stats {
                self.st.stats.id = Some(k.id.clone());
                self.guards.push(self.be.poll_stats(k.id.clone()));
            }
        }
        self.watching = want;
    }

    fn nav(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("nav").exact_width(200.0).resizable(false).show(ctx, |ui| {
            ui.add_space(10.0);
            ui.label(RichText::new("hullr").size(26.0).strong().color(theme::ACCENT));
            widgets::muted_label(ui, "containers, simply");
            ui.add_space(14.0);
            for p in Page::ALL {
                let count = match p {
                    Page::Containers if self.st.data.loaded[0] => format!("  {}", self.st.data.containers.len()),
                    Page::Images if self.st.data.loaded[1] => format!("  {}", self.st.data.images.len()),
                    Page::Volumes if self.st.data.loaded[2] => format!("  {}", self.st.data.volumes.len()),
                    Page::Compose if self.st.data.loaded[3] => format!("  {}", self.st.data.projects.len()),
                    _ => String::new(),
                };
                let text = RichText::new(format!("{}{count}", p.title())).size(16.0);
                if ui.add_sized([ui.available_width(), 32.0], egui::SelectableLabel::new(self.st.page == p, text)).clicked() {
                    self.st.page = p;
                    self.st.filter.clear();
                }
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.add_space(8.0);
                let dark = ui.visuals().dark_mode;
                match &self.st.data.error {
                    Some(_) => ui.label(RichText::new("Engine unreachable").color(theme::danger(dark))),
                    None if self.st.data.loaded[0] => ui.label(RichText::new("Engine connected").color(theme::success(dark))),
                    None => ui.label("Connecting..."),
                };
                if let Some(i) = &self.st.data.info {
                    widgets::muted_label(ui, format!("{} mode, v{}", i.mode, i.version));
                }
            });
        });
    }
}

impl eframe::App for HullrApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = ctx.input(|i| i.time);
        self.st.now = now;

        while let Ok(ev) = self.rx.try_recv() {
            self.st.apply(ev, now);
        }

        if self.applied_theme != Some(self.st.theme) {
            ctx.set_theme(match self.st.theme {
                ThemeChoice::System => egui::ThemePreference::System,
                ThemeChoice::Light => egui::ThemePreference::Light,
                ThemeChoice::Dark => egui::ThemePreference::Dark,
            });
            self.applied_theme = Some(self.st.theme);
        }

        if self.st.refresh_now || self.last_refresh.elapsed() >= REFRESH_EVERY {
            self.st.refresh_now = false;
            self.last_refresh = Instant::now();
            self.be.refresh(self.st.show_all);
        }

        self.nav(ctx);
        let be = self.be.clone();
        egui::CentralPanel::default().show(ctx, |ui| match self.st.page {
            Page::Containers => containers::show(ui, &mut self.st, &be),
            Page::Images => images::show(ui, &mut self.st, &be),
            Page::Volumes => volumes::show(ui, &mut self.st, &be),
            Page::Compose => compose::show(ui, &mut self.st, &be),
            Page::System => system::show(ui, &mut self.st, &be),
        });
        dialogs::show_all(ctx, &mut self.st, &be);

        self.sync_watchers();
        // Keeps relative times and the refresh timer ticking while idle.
        ctx.request_repaint_after(Duration::from_millis(500));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    fn st_with_selection(tab: DetailTab, state: ContainerState) -> AppState {
        let mut st = AppState::new();
        let c = container("web", state);
        st.selected = Some(c.id.clone());
        st.data.containers = vec![c];
        st.detail_tab = tab;
        st
    }

    #[test]
    fn no_selection_no_watch() {
        assert_eq!(watch_key(&AppState::new()), None);
    }

    #[test]
    fn overview_follows_nothing() {
        let k = watch_key(&st_with_selection(DetailTab::Overview, ContainerState::Running)).unwrap();
        assert!(!k.logs && !k.stats);
    }

    #[test]
    fn logs_tab_follows_logs_even_when_stopped() {
        let k = watch_key(&st_with_selection(DetailTab::Logs, ContainerState::Exited)).unwrap();
        assert!(k.logs && !k.stats);
    }

    #[test]
    fn stats_tab_needs_running() {
        assert!(watch_key(&st_with_selection(DetailTab::Stats, ContainerState::Running)).unwrap().stats);
        assert!(!watch_key(&st_with_selection(DetailTab::Stats, ContainerState::Exited)).unwrap().stats);
    }

    #[test]
    fn restart_changes_key() {
        let mut st = st_with_selection(DetailTab::Logs, ContainerState::Running);
        let a = watch_key(&st);
        st.data.containers[0].started_at = Some(1_700_000_999);
        assert_ne!(a, watch_key(&st));
    }
}
