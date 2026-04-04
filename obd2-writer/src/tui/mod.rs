pub mod app;
pub mod elm_actor;
pub mod event;
pub mod screens;
pub mod widgets;

use std::io;
use std::time::Duration;

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::CrosstermBackend;
use ratatui::Terminal;

use crate::transport::elm327::Elm327;

/// Run the TUI application. Takes ownership of the Elm327 connection.
pub async fn run(elm: Elm327) -> crate::error::Result<()> {
    // Spawn the Elm327 actor
    let (handle, _actor_handle) = elm_actor::spawn(elm);

    // Initialize the adapter
    let version = handle.initialize().await?;
    let connection_info = version;

    // Terminal setup
    enable_raw_mode().map_err(|e| crate::error::Error::Io(e))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .map_err(|e| crate::error::Error::Io(e))?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|e| crate::error::Error::Io(e))?;

    // Set up panic hook to restore terminal on panic
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        original_hook(info);
    }));

    // Create app
    let mut app = app::App::new(handle.clone(), connection_info);

    // Main loop
    let result = main_loop(&mut terminal, &mut app).await;

    // Restore terminal FIRST before any cleanup
    disable_raw_mode().map_err(|e| crate::error::Error::Io(e))?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .map_err(|e| crate::error::Error::Io(e))?;
    terminal.show_cursor().map_err(|e| crate::error::Error::Io(e))?;

    // Hard exit to prevent tokio runtime shutdown from closing the serial
    // port file descriptor. On macOS, closing a Bluetooth RFCOMM serial
    // port causes the OS to terminate the BT connection, requiring a full
    // unpair/repair cycle. By calling process::exit(), we skip all
    // destructors and let the OS reclaim resources without the explicit
    // close that triggers BT disconnect.
    if result.is_ok() {
        std::process::exit(0);
    }

    result
}

async fn main_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut app::App,
) -> crate::error::Result<()> {
    loop {
        // Draw
        terminal
            .draw(|f| app.render(f))
            .map_err(|e| crate::error::Error::Io(e))?;

        // Poll for events (50ms timeout so we can tick)
        if crossterm::event::poll(Duration::from_millis(50)).map_err(crate::error::Error::Io)? {
            if let Event::Key(key) = crossterm::event::read().map_err(crate::error::Error::Io)? {
                app.handle_key(key);
            }
        }

        // Tick (process async results)
        app.tick();

        if app.should_quit {
            break;
        }
    }
    Ok(())
}
