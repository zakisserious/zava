use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen,
};
#[cfg(unix)]
use signal_hook::consts::signal::{SIGINT, SIGTERM, SIGUSR1, SIGUSR2, SIGWINCH};
#[cfg(unix)]
use signal_hook::iterator::Signals;
use std::io::{self, stdout, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::thread::JoinHandle;
#[cfg(unix)]
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionEvent {
    Quit,
    Resize,
    ReloadConfig,
    ReloadColors,
    IncreaseSensitivity,
    DecreaseSensitivity,
    DecreaseBars,
    IncreaseBars,
    CycleForegroundColor,
    CycleBackgroundColor,
    CycleOrientation,
    OpenMenu,
}

pub struct TerminalGuard {
    use_alternate_screen: bool,
    active: bool,
    _signal_handle: Option<JoinHandle<()>>,
    event_rx: Receiver<ActionEvent>,
    running: Arc<AtomicBool>,
}

impl TerminalGuard {
    pub fn new(use_alternate_screen: bool) -> io::Result<Self> {
        enable_raw_mode()?;
        let mut out = stdout();
        if use_alternate_screen {
            execute!(out, EnterAlternateScreen, Hide, Clear(ClearType::All))?;
        } else {
            execute!(out, Hide, Clear(ClearType::All))?;
        }
        out.flush()?;

        let (tx, rx) = channel();
        #[cfg(not(unix))]
        let _tx = tx; // channel sender is only used by the unix signal thread
        let running = Arc::new(AtomicBool::new(true));

        // Spawn signal listener thread (unix only; Windows crossterm events
        // already cover quit + resize, and SIGUSR1/SIGUSR2 don't exist there).
        #[cfg(unix)]
        let signal_handle = Some({
            let mut signals = Signals::new(&[SIGINT, SIGTERM, SIGWINCH, SIGUSR1, SIGUSR2])
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

            let tx_sig = tx.clone();
            let running_clone = running.clone();
            thread::Builder::new()
                .name("zava-signals".to_string())
                .spawn(move || {
                    for sig in signals.forever() {
                        if !running_clone.load(Ordering::Relaxed) {
                            break;
                        }
                        match sig {
                            SIGINT | SIGTERM => {
                                let _ = tx_sig.send(ActionEvent::Quit);
                                break;
                            }
                            SIGWINCH => {
                                let _ = tx_sig.send(ActionEvent::Resize);
                            }
                            SIGUSR1 => {
                                let _ = tx_sig.send(ActionEvent::ReloadConfig);
                            }
                            SIGUSR2 => {
                                let _ = tx_sig.send(ActionEvent::ReloadColors);
                            }
                            _ => {}
                        }
                    }
                })?
        });
        #[cfg(not(unix))]
        let signal_handle = None;

        Ok(Self {
            use_alternate_screen,
            active: true,
            _signal_handle: signal_handle,
            event_rx: rx,
            running,
        })
    }

    pub fn poll_event(&self) -> Option<ActionEvent> {
        // First check background signals
        if let Ok(action) = self.event_rx.try_recv() {
            return Some(action);
        }

        // Then check keyboard events (non-blocking) — single read() per poll cycle
        if let Ok(true) = event::poll(Duration::from_millis(0)) {
            match event::read() {
                Ok(Event::Key(KeyEvent { code, modifiers, kind, .. })) => {
                    // Ignore key release events to prevent double-triggering on modern terminals
                    if kind == KeyEventKind::Release {
                        return None;
                    }
                    if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
                        return Some(ActionEvent::Quit);
                    }
                    match code {
                        KeyCode::Char('q') | KeyCode::Esc => return Some(ActionEvent::Quit),
                        KeyCode::Up => return Some(ActionEvent::IncreaseSensitivity),
                        KeyCode::Down => return Some(ActionEvent::DecreaseSensitivity),
                        KeyCode::Left => return Some(ActionEvent::DecreaseBars),
                        KeyCode::Right => return Some(ActionEvent::IncreaseBars),
                        KeyCode::Char('r') => return Some(ActionEvent::ReloadConfig),
                        KeyCode::Char('c') => return Some(ActionEvent::ReloadColors),
                        KeyCode::Char('f') => return Some(ActionEvent::CycleForegroundColor),
                        KeyCode::Char('b') => return Some(ActionEvent::CycleBackgroundColor),
                        KeyCode::Char('o') => return Some(ActionEvent::CycleOrientation),
                        KeyCode::Char('m') => return Some(ActionEvent::OpenMenu),
                        _ => {}
                    }
                }
                Ok(Event::Resize(_, _)) => return Some(ActionEvent::Resize),
                _ => {}
            }
        }

        None
    }

    /// Temporarily releases the terminal without tearing down the guard.
    ///
    /// The signal listener thread and its event channel stay alive, so this is
    /// safe to call in a loop (e.g. every time the settings menu is opened)
    /// whereas dropping and rebuilding the guard leaks a thread and a fresh
    /// `Signals` registration each time.
    pub fn suspend(&mut self) {
        if !self.active {
            return;
        }
        let mut out = stdout();
        let _ = execute!(out, Show);
        if self.use_alternate_screen {
            let _ = execute!(out, LeaveAlternateScreen);
        }
        let _ = write!(out, "\x1b[0m");
        let _ = out.flush();
        let _ = disable_raw_mode();
    }

    /// Re-acquires raw mode, the alternate screen and the hidden cursor after
    /// a call to [`TerminalGuard::suspend`].
    pub fn resume(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        enable_raw_mode()?;
        let mut out = stdout();
        if self.use_alternate_screen {
            execute!(out, EnterAlternateScreen)?;
        }
        execute!(out, Hide, Clear(ClearType::All))?;
        out.flush()?;
        Ok(())
    }

    pub fn restore(&mut self) {
        if self.active {
            self.running.store(false, Ordering::Relaxed);
            let mut out = stdout();
            let _ = execute!(out, Show);
            if self.use_alternate_screen {
                let _ = execute!(out, LeaveAlternateScreen);
            }
            let _ = write!(out, "\x1b[0m\n");
            let _ = out.flush();
            let _ = disable_raw_mode();
            self.active = false;
        }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.restore();
    }
}
