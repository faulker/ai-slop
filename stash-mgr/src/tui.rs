use std::io::{self, Stdout};
use std::panic;

use color_eyre::Result;
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

/// Initialize the terminal with raw mode and alternate screen.
pub fn init() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(io::stdout());
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

/// Restore the terminal to its original state.
pub fn restore() -> Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}

/// Install a panic hook that restores the terminal before printing the panic message.
/// This ensures panics don't leave the terminal in a broken state.
/// Must be called BEFORE init().
pub fn install_panic_hook() {
    // Install color-eyre for better error reporting
    color_eyre::install().expect("Failed to install color-eyre");

    // Capture the original panic hook
    let original_hook = panic::take_hook();

    // Install a new panic hook that restores terminal before calling original
    panic::set_hook(Box::new(move |panic_info| {
        // Restore terminal first so panic message is readable
        let _ = restore();
        // Call the original hook to print panic info
        original_hook(panic_info);
    }));
}
