use clap::Parser;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;
use std::collections::{HashSet, HashMap};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};
use anyhow::Result;

mod parser;
mod scanner;
mod app;
mod ui;

use app::{App, AppState};
use parser::Parser as BookmarkParser;
use scanner::{scan_bookmarks, LinkStatus};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Path to the bookmark HTML file
    #[arg(short, long)]
    pub input_file: PathBuf,

    /// Path to save the cleaned bookmark file
    #[arg(short, long)]
    pub output_file: Option<PathBuf>,

    /// Ignore localhost and private IP addresses
    #[arg(long, default_value_t = false)]
    pub ignore_local: bool,

    /// Exclude bookmarks in folders matching this name (can be used multiple times)
    #[arg(long, visible_alias = "ignore-folder")]
    pub exclude_folder: Vec<String>,

    /// Maximum number of redirects to follow
    #[arg(long, default_value_t = 10)]
    pub redirect_limit: usize,

    /// Number of concurrent requests
    #[arg(long, default_value_t = 1)]
    pub concurrent_requests: usize,

    /// Request timeout in seconds
    #[arg(long, default_value_t = 60)]
    pub timeout: u64,

    /// Number of retries for failed requests
    #[arg(long, default_value_t = 3)]
    pub retries: u32,

    /// Ignore SSL certificate errors
    #[arg(long, default_value_t = false)]
    pub ignore_ssl: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // 1. Parse Bookmarks
    let parser = BookmarkParser::new(args.exclude_folder.clone(), args.ignore_local);
    let bookmarks = parser.parse_file(&args.input_file)?;
    let total_bookmarks = bookmarks.len();

    // 2. Init App State
    let mut app = App::new(bookmarks);

    // 3. Setup TUI
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Set panic hook to restore terminal
    let panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(
            std::io::stdout(),
            LeaveAlternateScreen,
            DisableMouseCapture
        );
        panic_hook(panic_info);
    }));

    // 4. Start Scanner in background
    let (tx, mut rx) = mpsc::channel(100);
    let bookmarks_clone = app.bookmarks.clone();
    let redirect_limit = args.redirect_limit;
    let ignore_ssl = args.ignore_ssl;
    let concurrent_requests = args.concurrent_requests;
    let timeout = args.timeout;
    let retries = args.retries;
    
    let _scanner_handle = tokio::spawn(async move {
        scan_bookmarks(bookmarks_clone, tx, redirect_limit, ignore_ssl, concurrent_requests, timeout, retries).await;
    });

    let mut scanned_count = 0;
    let tick_rate = Duration::from_millis(100);
    let mut last_tick = std::time::Instant::now();
    let mut upgraded_links: HashMap<String, String> = HashMap::new();

    loop {
        terminal.draw(|f| ui::ui(f, &mut app))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                match app.state {
                    AppState::Scanning | AppState::Finished => {
                        match key.code {
                            KeyCode::Char('q') => {
                                app.should_quit = true;
                            },
                            KeyCode::Down => app.next(),
                            KeyCode::Up => app.previous(),
                            KeyCode::Char(' ') => app.toggle_selection(),
                            KeyCode::Char('k') => app.select_all(), // k for Keep All
                            KeyCode::Char('d') => app.deselect_all(), // d for Delete All (default state)
                            KeyCode::Enter => {
                                 if let AppState::Finished = app.state {
                                     // Determine output path (default to cleaned_bookmarks.html)
                                     let output_path = args.output_file.clone()
                                         .unwrap_or_else(|| PathBuf::from("cleaned_bookmarks.html"));

                                     // Perform IO operations
                                     let result = (|| -> Result<()> {
                                         let content = std::fs::read_to_string(&args.input_file)?;
                                         let cleaned_content = process_bookmarks(&content, &app, &upgraded_links);
                                         std::fs::write(&output_path, cleaned_content)?;
                                         Ok(())
                                     })();

                                     match result {
                                         Ok(_) => {
                                             app.output_path = Some(output_path.to_string_lossy().to_string());
                                             app.state = AppState::Saved;
                                         }
                                         Err(e) => {
                                             app.state = AppState::Error(format!("Failed to save: {}", e));
                                         }
                                     }
                                 }
                            }
                            _ => {}
                        }
                    },
                    AppState::Saved | AppState::Error(_) => {
                        // Any key exits
                        app.should_quit = true;
                    }
                }
            }
        }

        // Handle Scanner Updates
        while let Ok((index, status)) = rx.try_recv() {
            scanned_count += 1;
            app.scan_progress = scanned_count as f64 / total_bookmarks as f64;
            
            match status {
                LinkStatus::Dead(reason) => {
                    app.dead_links.push((index, reason));
                },
                LinkStatus::Upgraded(new_url) => {
                    if let Some(bm) = app.bookmarks.get_mut(index) {
                        let old_url = bm.url.clone();
                        bm.url = new_url.clone();
                        upgraded_links.insert(old_url, new_url);
                    }
                },
                LinkStatus::Ok => {}
            }
        }
        
        // Check if scanner finished
        if scanned_count >= total_bookmarks {
            if let AppState::Scanning = app.state {
                app.state = AppState::Finished;
            }
            app.scan_progress = 1.0;
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = std::time::Instant::now();
        }

        if app.should_quit {
            break;
        }
    }

    // Restore Terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    
    // Summary output to stdout after TUI closes
    if let Some(path) = &app.output_path {
         println!("Cleaned bookmarks saved to: {}", path);
         println!("Upgraded {} links to HTTPS", upgraded_links.len());
    }

    Ok(())
}

fn process_bookmarks(original_html: &str, app: &App, upgraded: &HashMap<String, String>) -> String {
    // We remove dead links that are NOT selected to keep
    let mut urls_to_remove = HashSet::new();
    for (idx, _) in &app.dead_links {
        if !app.bookmarks_to_keep.contains(idx) {
            if let Some(bm) = app.bookmarks.get(*idx) {
                urls_to_remove.insert(bm.url.clone());
            }
        }
    }
    
    let mut new_lines = Vec::new();
    
    for line in original_html.lines() {
        let trimmed = line.trim();
        let lower_trimmed = trimmed.to_lowercase();
        
        // Check if line contains a link
        if lower_trimmed.contains("<a") {
             if let Some(url) = extract_href(trimmed) {
                 // Check for deletion
                 if urls_to_remove.contains(&url) {
                     continue; // Skip this line
                 }
                 
                 // Check for upgrade (exact match on old URL)
                 // Note: 'url' extracted might need to match keys in 'upgraded'
                 // The keys in 'upgraded' come from the parser.
                 // The 'extract_href' here essentially mimics the parser logic, so it should match.
                 if let Some(new_url) = upgraded.get(&url) {
                     // Replace the URL in the line
                     // We use a simple replace here, assuming the URL appears once in the href
                     let new_line = line.replace(&url, new_url);
                     new_lines.push(new_line);
                     continue;
                 }
             }
        }
        new_lines.push(line.to_string());
    }
    
    new_lines.join("\n")
}

// Improved extraction that handles different quoting styles and case insensitivity
fn extract_href(line: &str) -> Option<String> {
    let lower = line.to_lowercase();
    let href_pat = "href=";
    
    if let Some(idx) = lower.find(href_pat) {
        let rest = &line[idx + href_pat.len()..];
        let mut chars = rest.chars();
        
        // Skip possible whitespace after href=
        let mut first_char = chars.next()?;
        while first_char.is_whitespace() {
            if let Some(c) = chars.next() {
                first_char = c;
            } else {
                return None;
            }
        }

        if first_char == '"' || first_char == '\'' {
            let quote = first_char;
            let val: String = chars.take_while(|&c| c != quote).collect();
            return Some(val);
        } else {
            // Unquoted value
            let mut val = String::new();
            val.push(first_char);
            for c in chars {
                if c.is_whitespace() || c == '>' {
                    break;
                }
                val.push(c);
            }
            return Some(val);
        }
    }
    None
}
