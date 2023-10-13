use crate::debugger::Debugger;
use crate::tui::output::{OutputLine, OutputStreamProcessor, StreamType};
use crate::tui::tick::Ticker;
use crate::util::DebugeeOutReader;
use crossterm::cursor::{SavePosition, Show};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use once_cell::sync;
use once_cell::sync::Lazy;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use timeout_readwrite::TimeoutReader;
use tui::backend::CrosstermBackend;
use tui::Terminal;

mod context;
pub mod hook;
pub mod output;
mod tick;
pub mod window;

#[derive(Default, Clone)]
pub struct Handle {
    flag: Arc<AtomicBool>,
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.flag.store(true, Ordering::SeqCst)
    }
}

pub enum Event<I> {
    Input(I),
    Tick,
}

pub struct AppBuilder {
    debugee_out: DebugeeOutReader,
    debugee_err: DebugeeOutReader,
}

impl AppBuilder {
    pub fn new(debugee_out: DebugeeOutReader, debugee_err: DebugeeOutReader) -> Self {
        Self {
            debugee_out,
            debugee_err,
        }
    }

    pub fn build(self, debugger: Debugger) -> TuiApplication {
        TuiApplication::new(debugger, self.debugee_out, self.debugee_err)
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum AppState {
    Initial,
    DebugeeRun,
    DebugeeBreak,
    UserInput,
    Finish,
}

#[derive(Default, Clone)]
pub struct DebugeeStreamBuffer {
    data: Arc<Mutex<Vec<OutputLine>>>,
}

static CTRL_C_CHAN: Lazy<Mutex<Option<Sender<Event<KeyEvent>>>>> = sync::Lazy::new(Mutex::default);

fn ctrl_c_handler() {
    let mb_chan = CTRL_C_CHAN.lock().unwrap();
    let mb_chan = mb_chan.as_ref();

    if let Some(chan) = mb_chan {
        _ = chan.send(Event::Input(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::empty(),
        )));
    }
}

pub struct TuiApplication {
    debugger: Debugger,
    debugee_out: DebugeeOutReader,
    debugee_err: DebugeeOutReader,
}

impl TuiApplication {
    pub fn new(
        debugger: Debugger,
        debugee_out: DebugeeOutReader,
        debugee_err: DebugeeOutReader,
    ) -> Self {
        Self {
            debugger,
            debugee_out,
            debugee_err,
        }
    }

    pub fn run(self) -> anyhow::Result<()> {
        let stream_buff = DebugeeStreamBuffer::default();

        // init debugee stdout handler
        let out = TimeoutReader::new(self.debugee_out.clone(), Duration::from_millis(1));
        let std_out_handle =
            OutputStreamProcessor::new(StreamType::StdOut).run(out, stream_buff.data.clone());

        // init debugee stderr handler
        let out = TimeoutReader::new(self.debugee_err.clone(), Duration::from_millis(1));
        let std_err_handle =
            OutputStreamProcessor::new(StreamType::StdErr).run(out, stream_buff.data.clone());

        enable_raw_mode()?;

        let (tx, rx) = mpsc::channel();
        {
            *CTRL_C_CHAN.lock().unwrap() = Some(tx.clone());
        }
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            _ = ctrlc::set_handler(ctrl_c_handler);
        });

        let ticker = Ticker::new(Duration::from_millis(200));
        let ticker_handle = ticker.run(tx);

        let stdout = io::stdout();
        let mut stdout = stdout.lock();

        crossterm::execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        crossterm::execute!(stdout, SavePosition)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;

        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic| {
            disable_raw_mode().unwrap();
            crossterm::execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture).unwrap();
            crossterm::execute!(io::stdout(), Show).unwrap();
            original_hook(panic);
        }));

        window::run(
            terminal,
            self.debugger,
            rx,
            stream_buff,
            self.debugee_out,
            self.debugee_err,
            vec![std_err_handle, std_out_handle, ticker_handle],
        )
    }
}
