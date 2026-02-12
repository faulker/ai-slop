use std::time::Duration;

use color_eyre::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use git2::{DiffFormat, DiffOptions, ErrorClass, ErrorCode, StashApplyOptions, StashSaveOptions, Status, StatusOptions};
use ratatui::backend::Backend;
use ratatui::layout::{Constraint, Direction, Flex, Layout, Position};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs};
use ratatui::{Frame, Terminal};
use strum::{Display, EnumIter, FromRepr, IntoEnumIterator};

// ── Color palette ────────────────────────────────────────────────────
const ACCENT: Color = Color::Indexed(75);        // soft blue — tab highlight, titles
const HIGHLIGHT_BG: Color = Color::Indexed(236);  // dark gray — selected row background
const HIGHLIGHT_FG: Color = Color::Indexed(75);   // soft blue — selected row text
const SUCCESS: Color = Color::Indexed(114);       // soft green — status messages, diff +
const ERROR: Color = Color::Indexed(203);         // soft red — errors, diff -, drop popup
const DIFF_HUNK: Color = Color::Indexed(139);     // muted purple — @@ hunk headers
const DIM: Color = Color::Indexed(242);           // gray — help text, borders

/// Maximum number of diff lines to display before truncation.
/// Prevents UI freezes on very large diffs. Well below ratatui's u16::MAX buffer limit.
const MAX_DIFF_LINES: usize = 10_000;

/// Maximum number of files to display in the Create Stash file list.
/// Prevents UI freezes in repositories with extremely large working trees.
const MAX_FILES_TO_DISPLAY: usize = 1_000;

/// Convert git2 errors into user-friendly messages
pub fn friendly_error_message(err: &git2::Error) -> String {
    match (err.code(), err.class()) {
        (ErrorCode::NotFound, ErrorClass::Repository) => {
            "Not a git repository (or any parent up to mount point)".to_string()
        }
        (ErrorCode::NotFound, _) => {
            format!("Not found: {}", err.message())
        }
        (ErrorCode::Locked, _) => {
            "Git index is locked. Another git process may be running. Try: rm -f .git/index.lock".to_string()
        }
        (ErrorCode::BareRepo, _) => {
            "This is a bare repository. Working directory operations are not supported.".to_string()
        }
        (ErrorCode::UnbornBranch, _) => {
            "Repository has no commits yet. Create an initial commit first.".to_string()
        }
        (ErrorCode::Conflict, _) | (ErrorCode::MergeConflict, _) => {
            "Cannot perform operation: merge conflicts present. Resolve conflicts first.".to_string()
        }
        _ => {
            format!("Git error: {}", err.message())
        }
    }
}

/// A single stash entry with metadata
#[derive(Clone, Debug)]
pub struct StashEntry {
    pub index: usize,
    pub message: String,
    pub branch: String,
    pub oid: git2::Oid,
}

/// A file entry in the working directory for stash creation
#[derive(Clone, Debug)]
pub struct FileEntry {
    pub path: String,
    pub status: git2::Status,
    pub selected: bool,
}

/// State management for the message input popup
pub struct MessageInputState {
    input: String,
    cursor_position: usize, // character-based position, not byte position
}

impl MessageInputState {
    /// Initialize with empty string and cursor at 0
    pub fn new() -> Self {
        Self {
            input: String::new(),
            cursor_position: 0,
        }
    }

    /// Insert character at cursor position
    pub fn enter_char(&mut self, c: char) {
        let byte_index = self.byte_index();
        self.input.insert(byte_index, c);
        self.cursor_position += 1;
    }

    /// Delete the character before the cursor
    pub fn delete_char(&mut self) {
        if self.cursor_position > 0 {
            let chars: Vec<char> = self.input.chars().collect();
            self.input = chars
                .iter()
                .take(self.cursor_position - 1)
                .chain(chars.iter().skip(self.cursor_position))
                .collect();
            self.cursor_position -= 1;
        }
    }

    /// Move cursor left
    pub fn move_cursor_left(&mut self) {
        self.cursor_position = self.cursor_position.saturating_sub(1);
    }

    /// Move cursor right
    pub fn move_cursor_right(&mut self) {
        let max = self.input.chars().count();
        if self.cursor_position < max {
            self.cursor_position += 1;
        }
    }

    /// Convert character position to byte index
    fn byte_index(&self) -> usize {
        self.input
            .char_indices()
            .map(|(i, _)| i)
            .nth(self.cursor_position)
            .unwrap_or(self.input.len())
    }

    /// Get the current input value
    pub fn value(&self) -> &str {
        &self.input
    }
}

/// State management for the file list in Create Stash tab
pub struct FileListState {
    pub list_state: ListState,
    pub files: Vec<FileEntry>,
}

impl FileListState {
    /// Initialize with files. If files is non-empty, select index 0.
    pub fn new(files: Vec<FileEntry>) -> Self {
        let mut list_state = ListState::default();
        if !files.is_empty() {
            list_state.select(Some(0));
        }
        Self { list_state, files }
    }

    /// Toggle the selected state of the currently highlighted file
    pub fn toggle_selected(&mut self) {
        if let Some(selected_idx) = self.list_state.selected()
            && let Some(file) = self.files.get_mut(selected_idx)
        {
            file.selected = !file.selected;
        }
    }

    /// Move selection to next item (wraps around)
    pub fn select_next(&mut self) {
        if self.files.is_empty() {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0);
        self.list_state.select(Some((current + 1) % self.files.len()));
    }

    /// Move selection to previous item (wraps around)
    pub fn select_previous(&mut self) {
        if self.files.is_empty() {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0);
        self.list_state
            .select(Some((current + self.files.len() - 1) % self.files.len()));
    }

    /// Get paths of all selected files
    pub fn selected_files(&self) -> Vec<String> {
        self.files
            .iter()
            .filter(|f| f.selected)
            .map(|f| f.path.clone())
            .collect()
    }

    /// Check if any files are selected
    pub fn has_selection(&self) -> bool {
        self.files.iter().any(|f| f.selected)
    }
}

/// The currently selected tab in the application
#[derive(Default, Clone, Copy, PartialEq, Display, FromRepr, EnumIter)]
pub enum SelectedTab {
    #[default]
    #[strum(to_string = "Create Stash")]
    Create,
    #[strum(to_string = "Manage Stashes")]
    Manage,
}

impl SelectedTab {
    /// Cycle to the next tab (wraps around)
    pub fn next(self) -> Self {
        let current = self as usize;
        let count = Self::iter().count();
        Self::from_repr((current + 1) % count).unwrap()
    }

    /// Cycle to the previous tab (wraps around)
    pub fn previous(self) -> Self {
        let current = self as usize;
        let count = Self::iter().count();
        Self::from_repr((current + count - 1) % count).unwrap()
    }
}

/// Main application state
pub struct App {
    selected_tab: SelectedTab,
    should_quit: bool,
    repo: git2::Repository,
    stashes: Vec<StashEntry>,
    stash_list_state: ListState,
    diff_content: String,
    diff_scroll: u16,
    status_message: Option<String>,
    show_confirm_popup: bool,
    confirm_stash_index: Option<usize>,
    file_list_state: Option<FileListState>,
    create_diff_content: String,
    create_diff_scroll: u16,
    show_message_input: bool,
    message_input: MessageInputState,
}

impl App {
    /// Validate that the repository is in a clean state for stash operations
    fn validate_repository_state(&self) -> Result<(), String> {
        // Check for bare repository
        if self.repo.is_bare() {
            return Err("This is a bare repository. Working directory operations are not supported.".to_string());
        }

        // Check repository state (merge, rebase, etc.)
        if self.repo.state() != git2::RepositoryState::Clean {
            return Err(format!(
                "Repository is in {:?} state. Complete or abort that operation first.",
                self.repo.state()
            ));
        }

        Ok(())
    }

    pub fn new(mut repo: git2::Repository) -> Self {
        let stashes = Self::load_stashes(&mut repo);
        let mut stash_list_state = ListState::default();

        // Select first stash if any exist and load its diff
        let diff_content = if !stashes.is_empty() {
            stash_list_state.select(Some(0));
            Self::get_stash_diff(&repo, stashes[0].oid)
        } else {
            String::new()
        };

        let mut app = Self {
            selected_tab: SelectedTab::default(),
            should_quit: false,
            repo,
            stashes,
            stash_list_state,
            diff_content,
            diff_scroll: 0,
            status_message: None,
            show_confirm_popup: false,
            confirm_stash_index: None,
            file_list_state: None,
            create_diff_content: String::new(),
            create_diff_scroll: 0,
            show_message_input: false,
            message_input: MessageInputState::new(),
        };

        // Load file list on startup since Create is the default tab
        app.refresh_file_list();
        app
    }

    /// Load working directory files for stash creation (tracked files only)
    fn load_working_files(repo: &git2::Repository) -> Vec<FileEntry> {
        let mut opts = StatusOptions::new();
        opts.include_untracked(false);
        opts.include_ignored(false);

        let statuses = match repo.statuses(Some(&mut opts)) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let mut files = Vec::new();
        let total_count = statuses.len();
        for entry in statuses.iter() {
            // Check if we've hit the file cap
            if files.len() >= MAX_FILES_TO_DISPLAY {
                // Add sentinel entry showing count of hidden files
                let hidden_count = total_count - files.len();
                files.push(FileEntry {
                    path: format!("... ({} more files not shown)", hidden_count),
                    status: Status::empty(),
                    selected: false,
                });
                break;
            }

            let status = entry.status();

            // Filter for modified/deleted files (working tree or index)
            if status.intersects(
                Status::WT_MODIFIED
                    | Status::WT_DELETED
                    | Status::INDEX_MODIFIED
                    | Status::INDEX_NEW
                    | Status::INDEX_DELETED,
            )
                && let Some(path) = entry.path() {
                    files.push(FileEntry {
                        path: path.to_string(),
                        status,
                        selected: false,
                    });
                }
        }

        files
    }

    /// Format git status flags into a display string
    fn format_file_status(status: Status) -> &'static str {
        // Staged changes take precedence in display
        if status.contains(Status::INDEX_NEW) {
            "staged new"
        } else if status.contains(Status::INDEX_MODIFIED) {
            "staged"
        } else if status.contains(Status::INDEX_DELETED) {
            "staged deleted"
        } else if status.contains(Status::WT_MODIFIED) {
            "modified"
        } else if status.contains(Status::WT_DELETED) {
            "deleted"
        } else {
            "changed"
        }
    }

    /// Refresh the file list for the Create Stash tab
    fn refresh_file_list(&mut self) {
        let files = Self::load_working_files(&self.repo);
        self.file_list_state = Some(FileListState::new(files));
        self.update_create_diff_preview();
    }

    /// Load all stashes from the repository
    fn load_stashes(repo: &mut git2::Repository) -> Vec<StashEntry> {
        let mut stashes = Vec::new();

        let _ = repo.stash_foreach(|index, name, oid| {
            // Parse the stash message to extract branch and user message
            // Format is typically: "WIP on branch: hash message" or "On branch: message"

            // Extract branch name between "on " and ":"
            let branch = if let Some(start) = name.find("on ") {
                let after_on = &name[start + 3..];
                if let Some(colon_pos) = after_on.find(':') {
                    after_on[..colon_pos].trim().to_string()
                } else {
                    "unknown".to_string()
                }
            } else {
                "unknown".to_string()
            };

            // Use the full name as the message for now
            let message = name.to_string();

            stashes.push(StashEntry {
                index,
                message,
                branch,
                oid: *oid,
            });

            true // Continue iteration
        });

        stashes
    }

    /// Get the diff for a stash
    fn get_stash_diff(repo: &git2::Repository, stash_oid: git2::Oid) -> String {
        // Try to generate the diff, return error string on failure
        match Self::try_get_stash_diff(repo, stash_oid, MAX_DIFF_LINES) {
            Ok(diff) => diff,
            Err(e) => format!("Failed to generate diff: {}", friendly_error_message(&e)),
        }
    }

    /// Try to get the diff for a stash (internal helper)
    fn try_get_stash_diff(repo: &git2::Repository, stash_oid: git2::Oid, max_lines: usize) -> Result<String, git2::Error> {
        let stash_commit = repo.find_commit(stash_oid)?;
        let stash_tree = stash_commit.tree()?;
        let parent_tree = stash_commit.parent(0)?.tree()?;

        let diff = repo.diff_tree_to_tree(Some(&parent_tree), Some(&stash_tree), None)?;

        let mut diff_text = String::new();
        let mut line_count = 0;
        diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
            // Check if we've hit the line limit
            if line_count >= max_lines {
                return false;
            }

            // Add origin character for context, addition, deletion lines
            let origin = line.origin();
            if matches!(origin, ' ' | '+' | '-' | 'B') {
                diff_text.push(origin);
            }
            if let Ok(content) = std::str::from_utf8(line.content()) {
                diff_text.push_str(content);
                // Count lines in the content
                line_count += content.lines().count().max(1);
            }
            true
        })?;

        // Add truncation message if we hit the limit
        if line_count >= max_lines {
            diff_text.push_str(&format!("\n... (diff truncated — showing first {} lines) ...", max_lines));
        }

        Ok(diff_text)
    }

    /// Main event loop - draw and handle events
    pub fn run(&mut self, terminal: &mut Terminal<impl Backend>) -> Result<()> {
        while !self.should_quit {
            terminal.draw(|f| self.draw(f))?;
            self.handle_events()?;
        }
        Ok(())
    }

    /// Handle keyboard events
    fn handle_events(&mut self) -> Result<()> {
        // Poll for events with 100ms timeout for responsive but low-CPU polling
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
        {
            self.handle_key_event(key);
        }
        Ok(())
    }

    /// Handle individual key press events
    fn handle_key_event(&mut self, key: KeyEvent) {
        // Only handle key press events (Windows compatibility)
        if key.kind != KeyEventKind::Press {
            return;
        }

        // Clear status message on any keypress
        self.status_message = None;

        // Handle message input popup keys first (intercepts all other keys)
        if self.show_message_input {
            match key.code {
                KeyCode::Char(c) => {
                    self.message_input.enter_char(c);
                }
                KeyCode::Backspace => {
                    self.message_input.delete_char();
                }
                KeyCode::Left => {
                    self.message_input.move_cursor_left();
                }
                KeyCode::Right => {
                    self.message_input.move_cursor_right();
                }
                KeyCode::Enter => {
                    self.create_stash();
                }
                KeyCode::Esc => {
                    // Cancel message input
                    self.show_message_input = false;
                    self.message_input = MessageInputState::new();
                }
                _ => {
                    // Ignore other keys when popup is visible
                }
            }
            return; // Don't process any other keys while popup is visible
        }

        // Handle confirmation popup keys (intercepts all other keys)
        if self.show_confirm_popup {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.confirm_drop_stash();
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.cancel_confirm_popup();
                }
                _ => {
                    // Ignore other keys when popup is visible
                }
            }
            return; // Don't process any other keys while popup is visible
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                self.should_quit = true;
            }
            KeyCode::Tab => {
                self.selected_tab = self.selected_tab.next();
                // Refresh file list when switching to Create tab
                if self.selected_tab == SelectedTab::Create {
                    self.refresh_file_list();
                }
            }
            KeyCode::BackTab => {
                self.selected_tab = self.selected_tab.previous();
                // Refresh file list when switching to Create tab
                if self.selected_tab == SelectedTab::Create {
                    self.refresh_file_list();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected_tab == SelectedTab::Manage && !self.stashes.is_empty() {
                    let old_selection = self.stash_list_state.selected();
                    let current = old_selection.unwrap_or(0);
                    self.stash_list_state.select(Some((current + 1) % self.stashes.len()));
                    if old_selection != self.stash_list_state.selected() {
                        self.update_diff_preview();
                    }
                } else if self.selected_tab == SelectedTab::Create
                    && let Some(ref mut file_list_state) = self.file_list_state
                {
                    let old_selection = file_list_state.list_state.selected();
                    file_list_state.select_next();
                    if old_selection != file_list_state.list_state.selected() {
                        self.update_create_diff_preview();
                    }
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected_tab == SelectedTab::Manage && !self.stashes.is_empty() {
                    let old_selection = self.stash_list_state.selected();
                    let current = old_selection.unwrap_or(0);
                    self.stash_list_state.select(Some((current + self.stashes.len() - 1) % self.stashes.len()));
                    if old_selection != self.stash_list_state.selected() {
                        self.update_diff_preview();
                    }
                } else if self.selected_tab == SelectedTab::Create
                    && let Some(ref mut file_list_state) = self.file_list_state
                {
                    let old_selection = file_list_state.list_state.selected();
                    file_list_state.select_previous();
                    if old_selection != file_list_state.list_state.selected() {
                        self.update_create_diff_preview();
                    }
                }
            }
            KeyCode::Char(' ') => {
                if self.selected_tab == SelectedTab::Create
                    && let Some(ref mut file_list_state) = self.file_list_state {
                        file_list_state.toggle_selected();
                    }
            }
            KeyCode::Char('s') => {
                if self.selected_tab == SelectedTab::Create {
                    // Check if any files are selected
                    if let Some(ref file_list_state) = self.file_list_state
                        && file_list_state.has_selection()
                    {
                        // Show message input popup
                        self.show_message_input = true;
                        self.message_input = MessageInputState::new();
                    } else {
                        self.status_message = Some("No files selected. Use Space to select files first.".to_string());
                    }
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if self.selected_tab == SelectedTab::Manage {
                    self.diff_scroll = self.diff_scroll.saturating_add(1);
                } else if self.selected_tab == SelectedTab::Create {
                    self.create_diff_scroll = self.create_diff_scroll.saturating_add(1);
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if self.selected_tab == SelectedTab::Manage {
                    self.diff_scroll = self.diff_scroll.saturating_sub(1);
                } else if self.selected_tab == SelectedTab::Create {
                    self.create_diff_scroll = self.create_diff_scroll.saturating_sub(1);
                }
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.selected_tab == SelectedTab::Manage {
                    self.diff_scroll = self.diff_scroll.saturating_add(10);
                } else if self.selected_tab == SelectedTab::Create {
                    self.create_diff_scroll = self.create_diff_scroll.saturating_add(10);
                }
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.selected_tab == SelectedTab::Manage {
                    self.diff_scroll = self.diff_scroll.saturating_sub(10);
                } else if self.selected_tab == SelectedTab::Create {
                    self.create_diff_scroll = self.create_diff_scroll.saturating_sub(10);
                }
            }
            KeyCode::Char('a') => {
                if self.selected_tab == SelectedTab::Manage {
                    self.apply_stash();
                }
            }
            KeyCode::Char('p') => {
                if self.selected_tab == SelectedTab::Manage {
                    self.pop_stash();
                }
            }
            KeyCode::Char('d') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.selected_tab == SelectedTab::Manage {
                    self.initiate_drop_stash();
                }
            }
            _ => {}
        }
    }

    /// Update the diff preview for the currently selected stash
    fn update_diff_preview(&mut self) {
        self.diff_scroll = 0;
        if let Some(selected) = self.stash_list_state.selected()
            && let Some(stash) = self.stashes.get(selected)
        {
            self.diff_content = Self::get_stash_diff(&self.repo, stash.oid);
        }
    }

    /// Get the working directory diff for a single file
    fn get_file_diff(repo: &git2::Repository, path: &str) -> String {
        match Self::try_get_file_diff(repo, path, MAX_DIFF_LINES) {
            Ok(diff) => diff,
            Err(e) => format!("Failed to generate diff: {}", friendly_error_message(&e)),
        }
    }

    /// Try to get the working directory diff for a single file (internal helper)
    fn try_get_file_diff(repo: &git2::Repository, path: &str, max_lines: usize) -> Result<String, git2::Error> {
        let mut opts = DiffOptions::new();
        opts.pathspec(path);

        // Try workdir diff first (unstaged changes)
        let diff = repo.diff_index_to_workdir(None, Some(&mut opts))?;

        let mut diff_text = String::new();
        let mut line_count = 0;

        // If workdir diff is empty, try staged changes (index vs HEAD)
        let diff = if diff.stats()?.files_changed() == 0 {
            let head = repo.head()?.peel_to_tree()?;
            repo.diff_tree_to_index(Some(&head), None, Some(&mut opts))?
        } else {
            diff
        };

        diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
            if line_count >= max_lines {
                return false;
            }

            let origin = line.origin();
            if matches!(origin, ' ' | '+' | '-' | 'B') {
                diff_text.push(origin);
            }
            if let Ok(content) = std::str::from_utf8(line.content()) {
                diff_text.push_str(content);
                line_count += content.lines().count().max(1);
            }
            true
        })?;

        if line_count >= max_lines {
            diff_text.push_str(&format!("\n... (diff truncated — showing first {} lines) ...", max_lines));
        }

        Ok(diff_text)
    }

    /// Update the diff preview for the currently highlighted file in Create tab
    fn update_create_diff_preview(&mut self) {
        self.create_diff_scroll = 0;
        if let Some(ref file_list_state) = self.file_list_state
            && let Some(selected) = file_list_state.list_state.selected()
            && let Some(file) = file_list_state.files.get(selected)
            && !file.status.is_empty()
        {
            self.create_diff_content = Self::get_file_diff(&self.repo, &file.path);
        } else {
            self.create_diff_content = String::new();
        }
    }

    /// Apply the currently selected stash (keeps stash in list)
    fn apply_stash(&mut self) {
        // Validate repository state first
        if let Err(msg) = self.validate_repository_state() {
            self.status_message = Some(msg);
            return;
        }

        // Extract the index first to avoid borrow issues
        let selected_index = match self.stash_list_state.selected() {
            Some(idx) => idx,
            None => return, // No stash selected, do nothing
        };

        // Apply the stash
        match self.repo.stash_apply(selected_index, Some(&mut StashApplyOptions::new())) {
            Ok(()) => {
                self.status_message = Some(format!("Applied stash@{{{}}} successfully", selected_index));
            }
            Err(e) => {
                self.status_message = Some(format!("Apply failed: {}", friendly_error_message(&e)));
            }
        }
    }

    /// Pop the currently selected stash (removes stash from list)
    fn pop_stash(&mut self) {
        // Validate repository state first
        if let Err(msg) = self.validate_repository_state() {
            self.status_message = Some(msg);
            return;
        }

        // Extract the index first to avoid borrow issues
        let selected_index = match self.stash_list_state.selected() {
            Some(idx) => idx,
            None => return, // No stash selected, do nothing
        };

        // Pop the stash
        match self.repo.stash_pop(selected_index, Some(&mut StashApplyOptions::new())) {
            Ok(()) => {
                self.status_message = Some(format!("Popped stash@{{{}}} successfully", selected_index));

                // Reload stash list
                self.stashes = Self::load_stashes(&mut self.repo);

                // Adjust selection
                if self.stashes.is_empty() {
                    // No stashes left
                    self.stash_list_state.select(None);
                    self.diff_content = String::new();
                } else if selected_index >= self.stashes.len() {
                    // The popped stash was the last one, select new last item
                    let new_selection = self.stashes.len() - 1;
                    self.stash_list_state.select(Some(new_selection));
                    self.diff_content = Self::get_stash_diff(&self.repo, self.stashes[new_selection].oid);
                } else {
                    // Keep same index (next stash slides into this position)
                    self.stash_list_state.select(Some(selected_index));
                    self.diff_content = Self::get_stash_diff(&self.repo, self.stashes[selected_index].oid);
                }
            }
            Err(e) => {
                self.status_message = Some(format!("Pop failed: {}", friendly_error_message(&e)));
            }
        }
    }

    /// Initiate drop stash confirmation (shows popup)
    fn initiate_drop_stash(&mut self) {
        // Validate repository state first
        if let Err(msg) = self.validate_repository_state() {
            self.status_message = Some(msg);
            return;
        }

        // Only show popup if a stash is selected
        if let Some(selected) = self.stash_list_state.selected() {
            self.show_confirm_popup = true;
            self.confirm_stash_index = Some(selected);
        }
    }

    /// Confirm and execute the stash drop
    fn confirm_drop_stash(&mut self) {
        if let Some(index) = self.confirm_stash_index {
            // Drop the stash
            match self.repo.stash_drop(index) {
                Ok(()) => {
                    self.status_message = Some(format!("Dropped stash@{{{}}} successfully", index));

                    // Reload stash list
                    self.stashes = Self::load_stashes(&mut self.repo);

                    // Adjust selection (same logic as pop)
                    if self.stashes.is_empty() {
                        self.stash_list_state.select(None);
                        self.diff_content = String::new();
                    } else if index >= self.stashes.len() {
                        let new_selection = self.stashes.len() - 1;
                        self.stash_list_state.select(Some(new_selection));
                        self.diff_content = Self::get_stash_diff(&self.repo, self.stashes[new_selection].oid);
                    } else {
                        self.stash_list_state.select(Some(index));
                        self.diff_content = Self::get_stash_diff(&self.repo, self.stashes[index].oid);
                    }
                }
                Err(e) => {
                    self.status_message = Some(format!("Drop failed: {}", friendly_error_message(&e)));
                }
            }
        }

        // Reset popup state
        self.show_confirm_popup = false;
        self.confirm_stash_index = None;
    }

    /// Cancel the confirmation popup
    fn cancel_confirm_popup(&mut self) {
        self.show_confirm_popup = false;
        self.confirm_stash_index = None;
    }

    /// Render the application UI
    fn draw(&mut self, frame: &mut Frame) {
        // Create vertical layout: tab bar (3 lines) + content area
        let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(0)])
            .split(frame.area());

        // Render tab bar
        self.render_tabs(frame, chunks[0]);

        // Render tab content
        self.render_tab_content(frame, chunks[1]);
    }

    /// Render the tab bar at the top
    fn render_tabs(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        let tab_titles: Vec<String> = SelectedTab::iter().map(|t| t.to_string()).collect();
        let tabs = Tabs::new(tab_titles)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(DIM))
                    .title("stash-mgr")
                    .title_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
            )
            .select(self.selected_tab as usize)
            .style(Style::default().fg(DIM))
            .highlight_style(
                Style::default()
                    .fg(ACCENT)
                    .add_modifier(Modifier::BOLD),
            )
            .divider(Span::styled(" | ", Style::default().fg(DIM)));
        frame.render_widget(tabs, area);
    }

    /// Render the content area for the selected tab
    fn render_tab_content(&mut self, frame: &mut Frame, area: ratatui::layout::Rect) {
        match self.selected_tab {
            SelectedTab::Create => {
                // Render file list if available
                if let Some(ref mut file_list_state) = self.file_list_state
                    && !file_list_state.files.is_empty()
                {
                    // Split the area horizontally: 40% file list, 60% diff
                    let chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
                        .split(area);

                    // Build list items with checkbox notation and colored status
                    let items: Vec<ListItem> = file_list_state
                        .files
                        .iter()
                        .map(|file| {
                            let checkbox = if file.selected { "[x] " } else { "[ ] " };
                            let status_str = Self::format_file_status(file.status);
                            let status_color = if file.status.intersects(
                                Status::INDEX_NEW | Status::INDEX_MODIFIED | Status::INDEX_DELETED,
                            ) {
                                SUCCESS
                            } else {
                                ERROR
                            };
                            ListItem::new(Line::from(vec![
                                Span::raw(checkbox),
                                Span::raw(&file.path),
                                Span::raw(" ("),
                                Span::styled(status_str, Style::default().fg(status_color)),
                                Span::raw(")"),
                            ]))
                        })
                        .collect();

                    let list = List::new(items)
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .border_type(BorderType::Rounded)
                                .border_style(Style::default().fg(DIM))
                                .title("Select Files (Space: toggle, s: stash)")
                                .title_style(Style::default().fg(ACCENT)),
                        )
                        .highlight_style(
                            Style::default()
                                .bg(HIGHLIGHT_BG)
                                .fg(HIGHLIGHT_FG)
                                .add_modifier(Modifier::BOLD),
                        )
                        .highlight_symbol(" > ");

                    frame.render_stateful_widget(list, chunks[0], &mut file_list_state.list_state);

                    // Render diff preview on the right
                    Self::render_diff_panel(frame, chunks[1], &self.create_diff_content, self.create_diff_scroll);
                } else {
                    // Empty state - no modified files
                    let content = Paragraph::new("No modified files -- working directory is clean")
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .border_type(BorderType::Rounded)
                                .border_style(Style::default().fg(DIM))
                                .title("Create Stash")
                                .title_style(Style::default().fg(ACCENT)),
                        )
                        .centered();
                    frame.render_widget(content, area);
                }
            }
            SelectedTab::Manage => {
                if self.stashes.is_empty() {
                    // Show empty state
                    let content = Paragraph::new(
                        "No stashes found. Use 'git stash' or the Create Stash tab to create one.",
                    )
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_type(BorderType::Rounded)
                            .border_style(Style::default().fg(DIM))
                            .title("Manage Stashes")
                            .title_style(Style::default().fg(ACCENT)),
                    )
                    .centered();
                    frame.render_widget(content, area);
                } else {
                    // Split the area horizontally: 40% list, 60% diff
                    let chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
                        .split(area);

                    // Render stash list on the left
                    let items: Vec<ListItem> = self
                        .stashes
                        .iter()
                        .map(|s| {
                            ListItem::new(format!(
                                "stash@{{{}}}: {} ({})",
                                s.index, s.message, s.branch
                            ))
                        })
                        .collect();

                    let list = List::new(items)
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .border_type(BorderType::Rounded)
                                .border_style(Style::default().fg(DIM))
                                .title("Stash List")
                                .title_style(Style::default().fg(ACCENT)),
                        )
                        .highlight_style(
                            Style::default()
                                .bg(HIGHLIGHT_BG)
                                .fg(HIGHLIGHT_FG)
                                .add_modifier(Modifier::BOLD),
                        )
                        .highlight_symbol(" > ");

                    frame.render_stateful_widget(list, chunks[0], &mut self.stash_list_state);

                    // Render diff preview on the right
                    Self::render_diff_panel(frame, chunks[1], &self.diff_content, self.diff_scroll);
                }
            }
        }

        // Render status message and help line at the bottom
        let status_area = ratatui::layout::Rect {
            x: area.x + 1,
            y: area.y + area.height - 2,
            width: area.width.saturating_sub(2),
            height: 1,
        };
        let help_area = ratatui::layout::Rect {
            x: area.x + 1,
            y: area.y + area.height - 1,
            width: area.width.saturating_sub(2),
            height: 1,
        };

        // Render status message if present
        if let Some(ref msg) = self.status_message {
            let style = if msg.contains("failed") || msg.contains("No files") || msg.contains("Please enter") {
                Style::default().fg(ERROR)
            } else {
                Style::default().fg(SUCCESS)
            };
            let status_line = Line::from(Span::styled(msg.as_str(), style));
            frame.render_widget(status_line, status_area);
        }

        // Render help text (changes based on popup visibility and active tab)
        let help_style = Style::default().fg(DIM);
        let help_text = if self.show_message_input {
            Line::from(Span::styled("Enter: Create Stash | Esc: Cancel | Type your stash message", help_style))
        } else if self.show_confirm_popup {
            Line::from(Span::styled("y: Confirm | n/Esc: Cancel", help_style))
        } else if self.selected_tab == SelectedTab::Create {
            Line::from(Span::styled("q: Quit | Tab: Switch Tab | Up/Down: Navigate | Space: Toggle | s: Stash Selected", help_style))
        } else {
            Line::from(Span::styled("q: Quit | Tab: Switch Tab | Up/Down: Navigate | a: Apply | p: Pop | d: Drop", help_style))
        };
        frame.render_widget(help_text, help_area);

        // Render confirmation popup overlay if visible
        if self.show_confirm_popup {
            self.render_confirm_popup(frame, area);
        }

        // Render message input popup overlay if visible
        if self.show_message_input {
            self.render_message_input_popup(frame, area);
        }
    }

    /// Render a diff panel with syntax highlighting (shared by both tabs)
    fn render_diff_panel(frame: &mut Frame, area: ratatui::layout::Rect, content: &str, scroll: u16) {
        let lines: Vec<Line> = content
            .lines()
            .map(|line| {
                if line.starts_with('+') {
                    Line::from(Span::styled(line, Style::default().fg(SUCCESS)))
                } else if line.starts_with('-') {
                    Line::from(Span::styled(line, Style::default().fg(ERROR)))
                } else if line.starts_with("@@") {
                    Line::from(Span::styled(line, Style::default().fg(DIFF_HUNK)))
                } else if line.starts_with("diff ") || line.starts_with("index ") {
                    Line::from(Span::styled(line, Style::default().fg(DIM).add_modifier(Modifier::BOLD)))
                } else {
                    Line::from(line)
                }
            })
            .collect();

        let diff_paragraph = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(DIM))
                    .title("Diff Preview")
                    .title_style(Style::default().fg(ACCENT)),
            )
            .scroll((scroll, 0));

        frame.render_widget(diff_paragraph, area);
    }

    /// Render the confirmation popup overlay
    fn render_confirm_popup(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        // Calculate centered popup area: 60% width, 20% height
        let popup_area = {
            let vertical = Layout::vertical([
                Constraint::Percentage(40),
                Constraint::Percentage(20),
                Constraint::Percentage(40),
            ])
            .flex(Flex::Center)
            .split(area);

            Layout::horizontal([
                Constraint::Percentage(20),
                Constraint::Percentage(60),
                Constraint::Percentage(20),
            ])
            .flex(Flex::Center)
            .split(vertical[1])[1]
        };

        // Get the stash message for display
        let message = if let Some(index) = self.confirm_stash_index
            && let Some(stash) = self.stashes.get(index)
        {
            format!(
                "Drop stash@{{{}}}: {}?\n\nThis cannot be undone.\n\nPress 'y' to confirm, 'n' or Esc to cancel",
                index, stash.message
            )
        } else {
            "Drop stash?\n\nThis cannot be undone.\n\nPress 'y' to confirm, 'n' or Esc to cancel".to_string()
        };

        // Clear the background
        frame.render_widget(Clear, popup_area);

        // Render the popup
        let popup = Paragraph::new(message)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(ERROR))
                    .title("Confirm Drop")
                    .title_style(Style::default().fg(ERROR).add_modifier(Modifier::BOLD)),
            )
            .centered();

        frame.render_widget(popup, popup_area);
    }

    /// Render the message input popup overlay
    fn render_message_input_popup(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        // Calculate centered popup area: 60% width, 20% height
        let popup_area = {
            let vertical = Layout::vertical([
                Constraint::Percentage(40),
                Constraint::Percentage(20),
                Constraint::Percentage(40),
            ])
            .flex(Flex::Center)
            .split(area);

            Layout::horizontal([
                Constraint::Percentage(20),
                Constraint::Percentage(60),
                Constraint::Percentage(20),
            ])
            .flex(Flex::Center)
            .split(vertical[1])[1]
        };

        // Clear the background
        frame.render_widget(Clear, popup_area);

        // Render the popup with input text
        let popup = Paragraph::new(self.message_input.value())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(ACCENT))
                    .title("Enter Stash Message (Enter: confirm, Esc: cancel)")
                    .title_style(Style::default().fg(ACCENT)),
            );

        frame.render_widget(popup, popup_area);

        // Set cursor position (account for left border)
        frame.set_cursor_position(Position::new(
            popup_area.x + self.message_input.cursor_position as u16 + 1,
            popup_area.y + 1,
        ));
    }

    /// Create a stash from the selected files with the entered message
    fn create_stash(&mut self) {
        // Validate repository state first
        if let Err(msg) = self.validate_repository_state() {
            self.status_message = Some(msg);
            self.show_message_input = false;
            self.message_input = MessageInputState::new();
            return;
        }

        // Get selected file paths
        let selected_paths = if let Some(ref file_list_state) = self.file_list_state {
            file_list_state.selected_files()
        } else {
            self.status_message = Some("No files available for stashing".to_string());
            return;
        };

        // Safety check: ensure files are selected (should be caught earlier, but double-check)
        if selected_paths.is_empty() {
            self.status_message = Some("No files selected for stashing".to_string());
            return;
        }

        // Get the message
        let message = self.message_input.value();

        // Require a message (don't allow empty)
        if message.trim().is_empty() {
            self.status_message = Some("Please enter a stash message".to_string());
            // Don't close popup so user can type
            return;
        }

        // Get signature (handle error gracefully)
        let signature = match self.repo.signature() {
            Ok(sig) => sig,
            Err(_) => {
                self.status_message = Some("Stash failed: git user.name/email not configured".to_string());
                self.show_message_input = false;
                self.message_input = MessageInputState::new();
                return;
            }
        };

        // Create stash with pathspecs
        let mut opts = StashSaveOptions::new(signature);
        for path in &selected_paths {
            opts.pathspec(path);
        }

        // Execute stash creation
        match self.repo.stash_save_ext(Some(&mut opts)) {
            Ok(_oid) => {
                // Success!
                let count = selected_paths.len();
                self.status_message = Some(format!("Stashed {} file(s): {}", count, message));

                // Refresh file list to show updated working directory
                self.refresh_file_list();

                // Refresh stash list for Manage tab
                self.stashes = Self::load_stashes(&mut self.repo);

                // Update stash selection (new stash is at index 0)
                if !self.stashes.is_empty() {
                    self.stash_list_state.select(Some(0));
                    self.diff_content = Self::get_stash_diff(&self.repo, self.stashes[0].oid);
                    self.diff_scroll = 0;
                }
            }
            Err(e) => {
                self.status_message = Some(format!("Stash creation failed: {}", friendly_error_message(&e)));
            }
        }

        // Close popup and reset input
        self.show_message_input = false;
        self.message_input = MessageInputState::new();
    }
}
