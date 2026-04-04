use crossterm::event::KeyEvent;
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Tabs},
    Frame,
};

use super::elm_actor::ElmHandle;
use super::event::{self, AppAction};
use super::screens::{backup, dashboard, dtc, raw, scans, settings};
use super::widgets::status_bar::StatusBar;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dashboard,
    Settings,
    Dtc,
    Scans,
    Backup,
    Raw,
}

impl Tab {
    pub fn index(self) -> usize {
        match self {
            Tab::Dashboard => 0,
            Tab::Settings => 1,
            Tab::Dtc => 2,
            Tab::Scans => 3,
            Tab::Backup => 4,
            Tab::Raw => 5,
        }
    }

    fn titles() -> Vec<&'static str> {
        vec![
            "1:Dashboard",
            "2:Settings",
            "3:DTC",
            "4:Scans",
            "5:Backup",
            "6:Raw",
        ]
    }
}

pub struct App {
    pub active_tab: Tab,
    pub should_quit: bool,
    pub elm_handle: ElmHandle,
    pub connection_info: String,
    // Per-tab state
    pub dashboard: dashboard::DashboardState,
    pub settings: settings::SettingsState,
    pub dtc: dtc::DtcState,
    pub scans: scans::ScansState,
    pub backup: backup::BackupState,
    pub raw: raw::RawState,
}

impl App {
    pub fn new(elm_handle: ElmHandle, connection_info: String) -> Self {
        Self {
            active_tab: Tab::Dashboard,
            should_quit: false,
            elm_handle,
            connection_info,
            dashboard: dashboard::DashboardState::default(),
            settings: settings::SettingsState::default(),
            dtc: dtc::DtcState::default(),
            scans: scans::ScansState::default(),
            backup: backup::BackupState::default(),
            raw: raw::RawState::default(),
        }
    }

    pub fn render(&mut self, f: &mut Frame) {
        let chunks = Layout::vertical([
            Constraint::Length(1), // Status bar
            Constraint::Length(2), // Tab bar
            Constraint::Min(1),   // Content
        ])
        .split(f.area());

        // Status bar
        let status = StatusBar::new(&self.connection_info);
        f.render_widget(status, chunks[0]);

        // Tab bar
        let titles: Vec<Line> = Tab::titles()
            .into_iter()
            .map(|t| Line::from(Span::raw(t)))
            .collect();
        let tabs = Tabs::new(titles)
            .select(self.active_tab.index())
            .style(Style::default().fg(Color::DarkGray))
            .highlight_style(Style::default().fg(Color::Cyan).bold())
            .divider("|");
        f.render_widget(tabs, chunks[1]);

        // Content area
        let content = chunks[2];
        match self.active_tab {
            Tab::Dashboard => dashboard::render(&mut self.dashboard, f, content, &self.elm_handle),
            Tab::Settings => settings::render(&mut self.settings, f, content, &self.elm_handle),
            Tab::Dtc => dtc::render(&mut self.dtc, f, content, &self.elm_handle),
            Tab::Scans => scans::render(&mut self.scans, f, content, &self.elm_handle),
            Tab::Backup => backup::render(&mut self.backup, f, content, &self.elm_handle),
            Tab::Raw => raw::render(&mut self.raw, f, content, &self.elm_handle),
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        let input_focused = self.is_input_focused();
        let action = event::map_key(key, self.active_tab, input_focused);

        match action {
            Some(AppAction::Quit) => {
                self.should_quit = true;
            }
            Some(AppAction::SwitchTab(tab)) => {
                self.active_tab = tab;
            }
            Some(AppAction::TabKey(k)) => {
                self.dispatch_tab_key(k);
            }
            None => {}
        }
    }

    pub fn tick(&mut self) {
        match self.active_tab {
            Tab::Dashboard => self.dashboard.tick(&self.elm_handle),
            Tab::Settings => self.settings.tick_with_elm(&self.elm_handle),
            Tab::Dtc => self.dtc.tick(),
            Tab::Scans => self.scans.tick(),
            Tab::Backup => self.backup.tick(),
            Tab::Raw => self.raw.tick(),
        }
    }

    fn is_input_focused(&self) -> bool {
        match self.active_tab {
            Tab::Dashboard => self.dashboard.picker.visible,
            Tab::Settings => self.settings.is_input_focused(),
            Tab::Dtc => self.dtc.is_input_focused(),
            Tab::Scans => self.scans.is_input_focused(),
            Tab::Backup => self.backup.is_input_focused(),
            Tab::Raw => self.raw.is_input_focused(),
        }
    }

    fn dispatch_tab_key(&mut self, key: KeyEvent) {
        match self.active_tab {
            Tab::Dashboard => {
                dashboard::handle_key(&mut self.dashboard, key, &self.elm_handle);
            }
            Tab::Settings => {
                settings::handle_key(&mut self.settings, key, &self.elm_handle);
            }
            Tab::Dtc => {
                dtc::handle_key(&mut self.dtc, key, &self.elm_handle);
            }
            Tab::Scans => {
                scans::handle_key(&mut self.scans, key, &self.elm_handle);
            }
            Tab::Backup => {
                backup::handle_key(&mut self.backup, key, &self.elm_handle);
            }
            Tab::Raw => {
                raw::handle_key(&mut self.raw, key, &self.elm_handle);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tab_index() {
        assert_eq!(Tab::Dashboard.index(), 0);
        assert_eq!(Tab::Raw.index(), 5);
    }

    #[test]
    fn test_tab_titles_count() {
        assert_eq!(Tab::titles().len(), 6);
    }
}
