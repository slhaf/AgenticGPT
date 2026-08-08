use std::{
    io::{self, Stdout},
    sync::{
        atomic::{AtomicBool, Ordering},
        Once,
    },
    time::Duration,
};

use anyhow::{anyhow, Result};
use crossterm::{
    cursor::{Hide, Show},
    event::{self, Event, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

pub(crate) enum TerminalEvent {
    Key(KeyEvent),
    Resize,
    Tick,
}

pub(crate) struct TerminalSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    active: bool,
}

static PANIC_HOOK: Once = Once::new();
static TUI_ACTIVE: AtomicBool = AtomicBool::new(false);

impl TerminalSession {
    pub(crate) fn enter() -> Result<Self> {
        install_panic_hook();
        let mut stdout = io::stdout();
        enable_raw_mode().map_err(|error| anyhow!("tui_raw_mode: {error}"))?;
        if let Err(error) = execute!(stdout, EnterAlternateScreen, Hide) {
            let _ = disable_raw_mode();
            return Err(anyhow!("tui_enter_screen: {error}"));
        }

        let backend = CrosstermBackend::new(stdout);
        let mut terminal = match Terminal::new(backend) {
            Ok(terminal) => terminal,
            Err(error) => {
                restore_primitives();
                return Err(anyhow!("tui_terminal_init: {error}"));
            }
        };
        if let Err(error) = terminal.clear() {
            restore_terminal(&mut terminal);
            return Err(anyhow!("tui_clear: {error}"));
        }

        TUI_ACTIVE.store(true, Ordering::SeqCst);
        Ok(Self {
            terminal,
            active: true,
        })
    }

    pub(crate) fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<Stdout>> {
        &mut self.terminal
    }

    pub(crate) fn next_event(&self, timeout: Duration) -> Result<TerminalEvent> {
        if !event::poll(timeout)? {
            return Ok(TerminalEvent::Tick);
        }
        Ok(match event::read()? {
            Event::Key(key) => TerminalEvent::Key(key),
            Event::Resize(_, _) => TerminalEvent::Resize,
            _ => TerminalEvent::Tick,
        })
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        restore_terminal(&mut self.terminal);
        self.active = false;
        TUI_ACTIVE.store(false, Ordering::SeqCst);
    }
}

fn install_panic_hook() {
    PANIC_HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic| {
            if TUI_ACTIVE.swap(false, Ordering::SeqCst) {
                restore_primitives();
            }
            previous(panic);
        }));
    });
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) {
    restore_with(|step| match step {
        "show_cursor" => {
            let _ = terminal.show_cursor();
        }
        "leave_alt_screen" => {
            let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
        }
        "disable_raw_mode" => {
            let _ = disable_raw_mode();
        }
        _ => {}
    });
}

fn restore_primitives() {
    let mut stdout = io::stdout();
    restore_with(|step| match step {
        "show_cursor" => {
            let _ = execute!(stdout, Show);
        }
        "leave_alt_screen" => {
            let _ = execute!(stdout, LeaveAlternateScreen);
        }
        "disable_raw_mode" => {
            let _ = disable_raw_mode();
        }
        _ => {}
    });
}

fn restore_with(mut step: impl FnMut(&'static str)) {
    for step_name in ["show_cursor", "leave_alt_screen", "disable_raw_mode"] {
        step(step_name);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn restoration_seam_records_cleanup_in_reverse_setup_order() {
        let mut steps = Vec::new();
        super::restore_with(|step| steps.push(step));
        assert_eq!(
            steps,
            ["show_cursor", "leave_alt_screen", "disable_raw_mode"]
        );
    }
}
