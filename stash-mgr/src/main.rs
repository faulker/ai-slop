mod app;
mod tui;

use color_eyre::Result;

fn main() -> Result<()> {
    // Install panic hook BEFORE any terminal initialization
    tui::install_panic_hook();

    // Verify we're in a git repository before entering TUI mode
    let repo = match git2::Repository::discover(".") {
        Ok(repo) => repo,
        Err(e) => {
            eprintln!("Error: {}", app::friendly_error_message(&e));
            std::process::exit(1);
        }
    };

    // Check for detached HEAD state and warn user
    if repo.head_detached().unwrap_or(false) {
        eprintln!("Warning: Repository is in detached HEAD state. Stash operations will work but without a branch reference.");
    }

    // Initialize terminal
    let mut terminal = tui::init()?;

    // Create and run the app
    let mut app = app::App::new(repo);
    app.run(&mut terminal)?;

    // Restore terminal to normal state
    tui::restore()?;

    Ok(())
}
