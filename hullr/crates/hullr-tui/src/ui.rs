//! The pure render function: state in, frame out.

use crate::state::{App, Modal, Tab};
use crate::theme;
use crate::views::{compose, containers, images, modals, system, volumes};
use ratatui::{
    layout::{Constraint, Layout},
    text::{Line, Span},
    widgets::{Paragraph, Tabs},
    Frame,
};

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(1)]).split(area);

    if matches!(app.modal, Modal::Logs) {
        let id = app.selected_container().map(|c| c.name.clone()).unwrap_or_default();
        let outer = crate::widgets::panel(&format!("Logs: {id}"));
        let inner = outer.inner(chunks[1]);
        f.render_widget(outer, chunks[1]);
        containers::draw_logs(f, inner, &app.detail, true);
    } else {
        draw_tabs(f, chunks[0], app);
        match app.tab {
            Tab::Containers => containers::draw(f, chunks[1], app),
            Tab::Images => images::draw(f, chunks[1], app),
            Tab::Volumes => volumes::draw(f, chunks[1], app),
            Tab::Compose => compose::draw(f, chunks[1], app),
            Tab::System => system::draw(f, chunks[1], app),
        }
    }
    let status = if matches!(app.modal, Modal::Logs) {
        crate::widgets::hints(&[("j/k", "scroll"), ("g", "oldest"), ("G", "follow"), ("Esc", "back")])
    } else {
        modals::status_line(app)
    };
    f.render_widget(Paragraph::new(status), chunks[2]);
    modals::draw(f, area, app);
}

fn draw_tabs(f: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let titles: Vec<Line> = Tab::ALL
        .iter()
        .map(|t| {
            let count = match t {
                Tab::Containers if app.loaded[0] => format!(" ({})", app.containers.len()),
                Tab::Images if app.loaded[1] => format!(" ({})", app.images.len()),
                Tab::Volumes if app.loaded[2] => format!(" ({})", app.volumes.len()),
                Tab::Compose if app.loaded[3] => format!(" ({})", app.projects.len()),
                _ => String::new(),
            };
            Line::from(format!(" {} {}{} ", t.index() + 1, t.title(), count))
        })
        .collect();
    let [left, right] = Layout::horizontal([Constraint::Min(10), Constraint::Length(8)]).areas(area);
    let tabs = Tabs::new(titles)
        .select(app.tab.index())
        .highlight_style(theme::title().add_modifier(ratatui::style::Modifier::REVERSED))
        .divider(Span::styled("|", theme::muted()));
    f.render_widget(tabs, left);
    f.render_widget(Paragraph::new(Span::styled("hullr ", theme::title())).right_aligned(), right);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Msg, ProgressView};
    use crate::testutil::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use hullr_core::ContainerState;

    fn key(c: char) -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    fn populated() -> App {
        let mut app = App::new();
        app.now = 1_700_000_300;
        app.update(Msg::Containers(Ok(vec![
            container("web", ContainerState::Running),
            container("db", ContainerState::Exited),
        ])));
        app.update(Msg::Images(Ok(vec![image("docker.io/library/nginx:latest")])));
        app.update(Msg::Volumes(Ok(vec![volume("pgdata")])));
        app.update(Msg::Compose(Ok(vec![])));
        app.update(Msg::Info(Ok(system_info())));
        app
    }

    #[test]
    fn containers_table_renders_columns_and_status() {
        let app = populated();
        let s = render(&app, 130, 30);
        for want in ["Containers", "NAME", "IMAGE", "STATUS", "PORTS", "web", "db", "nginx:latest", "8080->80", "Exited (0)"] {
            assert!(s.contains(want), "missing {want:?} in:\n{s}");
        }
        assert!(s.contains("1 Containers (2)"));
    }

    #[test]
    fn detail_pane_shows_summary_stats_and_logs() {
        let mut app = populated();
        app.update(key('G')); // web
        let id = app.selected_container().unwrap().id.clone();
        app.update(Msg::Stats {
            id: id.clone(),
            stats: hullr_core::Stats { cpu_percent: 12.5, memory_bytes: 20_000_000, memory_limit: Some(512_000_000), pids: 4, ..Default::default() },
        });
        app.update(Msg::Log { id, stderr: false, line: "hello from web".into() });
        let s = render(&app, 130, 34);
        for want in ["Live stats", "CPU", "12.5%", "20.0MB / 512.0MB", "PIDS 4", "hello from web", "Platform"] {
            assert!(s.contains(want), "missing {want:?} in:\n{s}");
        }
    }

    #[test]
    fn empty_states_are_friendly() {
        let mut app = App::new();
        app.update(Msg::Containers(Ok(vec![])));
        let s = render(&app, 100, 24);
        assert!(s.contains("No containers yet"), "{s}");
        assert!(s.contains("Press r to run one"), "{s}");
        app.update(key('2'));
        app.update(Msg::Images(Ok(vec![])));
        assert!(render(&app, 100, 24).contains("Press p to pull one"));
        app.update(key('3'));
        app.update(Msg::Volumes(Ok(vec![])));
        assert!(render(&app, 100, 24).contains("Press n to create one"));
        app.update(key('4'));
        app.update(Msg::Compose(Ok(vec![])));
        assert!(render(&app, 100, 24).contains("No compose projects"));
    }

    #[test]
    fn loading_and_error_states() {
        let mut app = App::new();
        assert!(render(&app, 100, 24).contains("Loading containers"));
        app.update(Msg::Containers(Err("connection refused".into())));
        let s = render(&app, 100, 24);
        assert!(s.contains("Can't reach the engine") && s.contains("connection refused"), "{s}");
    }

    #[test]
    fn running_only_empty_hint() {
        let mut app = App::new();
        app.show_all = false;
        app.update(Msg::Containers(Ok(vec![])));
        assert!(render(&app, 100, 24).contains("No running containers"));
    }

    #[test]
    fn help_lists_every_key() {
        let mut app = populated();
        app.update(key('?'));
        let s = render(&app, 100, 60);
        for (_, keys) in crate::views::modals::HELP {
            for (k, d) in *keys {
                assert!(s.contains(k), "help is missing key {k:?}");
                assert!(s.contains(&d[..d.len().min(20)]), "help is missing text {d:?}");
            }
        }
    }

    #[test]
    fn confirm_and_input_modals() {
        let mut app = populated();
        app.update(key('d'));
        let s = render(&app, 100, 24);
        assert!(s.contains("Are you sure?") && s.contains("Remove container 'db'"), "{s}");
        app.update(key('n'));
        app.update(key('r'));
        app.update(key('n'));
        app.update(key('g'));
        let s = render(&app, 100, 24);
        assert!(s.contains("Run a container") && s.contains("> ng"), "{s}");
    }

    #[test]
    fn progress_popup_shows_layers_and_result() {
        let mut app = populated();
        let mut v = ProgressView::new("Pulling alpine", true);
        v.apply(hullr_core::api::Progress::Layer { id: "sha256:aabbccddeeff1122".into(), current: 5_000_000, total: 10_000_000, done: false });
        v.apply(hullr_core::api::Progress::Status { message: "Resolving manifest".into() });
        app.modal = crate::state::Modal::Progress(v);
        let s = render(&app, 100, 30);
        for want in ["Pulling alpine", "50%", "aabbccddeeff", "Resolving manifest", "hide"] {
            assert!(s.contains(want), "missing {want:?} in:\n{s}");
        }
        app.update(Msg::OpDone(Err("registry error: denied".into())));
        let s = render(&app, 100, 30);
        assert!(s.contains("Failed: registry error: denied"), "{s}");
    }

    #[test]
    fn other_tabs_render() {
        let mut app = populated();
        app.update(key('2'));
        let s = render(&app, 120, 24);
        assert!(s.contains("nginx:latest") && s.contains("142.0MB") && s.contains("linux/arm64/v8"), "{s}");
        app.update(key('3'));
        assert!(render(&app, 120, 24).contains("pgdata"));
        app.update(key('4'));
        app.update(Msg::Compose(Ok(vec![hullr_core::ComposeStatus {
            project: "shop".into(),
            services: vec![hullr_core::ComposeService { service: "web".into(), container: Some(container("shop-web", ContainerState::Running)) }],
        }])));
        let s = render(&app, 120, 24);
        assert!(s.contains("shop") && s.contains("1/1 running") && s.contains("web"), "{s}");
        app.update(key('5'));
        let s = render(&app, 120, 24);
        assert!(s.contains("Warnings") && s.contains("no cgroup delegation") && s.contains("in-process") && s.contains("linux/amd64"), "{s}");
    }

    #[test]
    fn fullscreen_logs_and_status_bar() {
        let mut app = populated();
        let id = app.selected_container().unwrap().id.clone();
        app.update(Msg::Log { id, stderr: true, line: "kaboom".into() });
        app.update(key('l'));
        let s = render(&app, 100, 20);
        assert!(s.contains("kaboom") && s.contains("Esc back"), "{s}");
        app.update(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE).into_msg());
        app.set_status(crate::state::Level::Error, "boom happened");
        assert!(render(&app, 100, 20).contains("boom happened"));
    }

    #[test]
    fn tiny_terminals_do_not_panic() {
        let mut app = populated();
        for (w, h) in [(1, 1), (10, 3), (30, 8), (60, 12), (200, 60)] {
            for t in ['1', '2', '3', '4', '5'] {
                app.update(key(t));
                render(&app, w, h);
            }
            app.update(key('?'));
            render(&app, w, h);
            app.update(key('x'));
        }
    }

    trait IntoMsg {
        fn into_msg(self) -> Msg;
    }
    impl IntoMsg for KeyEvent {
        fn into_msg(self) -> Msg {
            Msg::Key(self)
        }
    }
}
