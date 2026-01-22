use crate::parser::Bookmark;
use std::collections::HashSet;

pub enum AppState {
    Scanning,
    Finished,
    Saved,
    Error(String),
}

pub struct App {
    pub bookmarks: Vec<Bookmark>,
    pub dead_links: Vec<(usize, String)>, // (Indices into bookmarks, Reason)
    pub bookmarks_to_keep: HashSet<usize>, // Indices into bookmarks
    pub scan_progress: f64,
    pub state: AppState,
    pub list_state: ratatui::widgets::ListState,
    pub should_quit: bool,
    pub output_path: Option<String>,
}

impl App {
    pub fn new(bookmarks: Vec<Bookmark>) -> Self {
        let mut list_state = ratatui::widgets::ListState::default();
        list_state.select(Some(0));
        
        Self {
            bookmarks,
            dead_links: Vec::new(),
            bookmarks_to_keep: HashSet::new(),
            scan_progress: 0.0,
            state: AppState::Scanning,
            list_state,
            should_quit: false,
            output_path: None,
        }
    }

    pub fn next(&mut self) {
        if self.dead_links.is_empty() { return; }
        
        let i = match self.list_state.selected() {
            Some(i) => {
                if i >= self.dead_links.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    pub fn previous(&mut self) {
         if self.dead_links.is_empty() { return; }
         
        let i = match self.list_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.dead_links.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    pub fn toggle_selection(&mut self) {
        if let Some(selected_idx) = self.list_state.selected() {
            if let Some((bookmark_idx, _)) = self.dead_links.get(selected_idx) {
                if self.bookmarks_to_keep.contains(bookmark_idx) {
                    self.bookmarks_to_keep.remove(bookmark_idx);
                } else {
                    self.bookmarks_to_keep.insert(*bookmark_idx);
                }
            }
        }
    }

    pub fn select_all(&mut self) {
        for (idx, _) in &self.dead_links {
            self.bookmarks_to_keep.insert(*idx);
        }
    }

    pub fn deselect_all(&mut self) {
        self.bookmarks_to_keep.clear();
    }
}
