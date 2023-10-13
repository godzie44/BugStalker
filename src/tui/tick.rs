use crate::tui::{Event, Handle};
use crossterm::event;
use crossterm::event::KeyEvent;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

pub struct Ticker {
    rate: Duration,
}

impl Ticker {
    pub fn new(rate: Duration) -> Self {
        Self { rate }
    }

    pub fn run(self, tx: Sender<Event<KeyEvent>>) -> Handle {
        let handle = Handle::default();
        {
            let handle = handle.clone();
            thread::spawn(move || {
                let mut last_tick = Instant::now();
                loop {
                    let timeout = self
                        .rate
                        .checked_sub(last_tick.elapsed())
                        .unwrap_or_else(|| Duration::from_secs(0));

                    if handle.flag.load(Ordering::SeqCst) {
                        return;
                    }

                    if event::poll(timeout).expect("poll works") {
                        if let event::Event::Key(key) = event::read().expect("can read events") {
                            tx.send(Event::Input(key)).expect("can send events");
                        }
                    }

                    if last_tick.elapsed() >= self.rate && tx.send(Event::Tick).is_ok() {
                        last_tick = Instant::now();
                    }
                }
            });
        }
        handle
    }
}
