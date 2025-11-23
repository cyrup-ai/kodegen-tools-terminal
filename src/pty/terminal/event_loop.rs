use std::io::{self, ErrorKind, Read, Write};
use std::sync::{mpsc, Arc};
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::VecDeque;
use std::num::NonZeroUsize;
use polling::{Event, Events, PollMode, Poller};
use tokio::sync::broadcast;
use alacritty_terminal::tty::{EventedPty, ChildEvent};
use alacritty_terminal::term::Term;
use vte::ansi::Processor;

use super::types::HeadlessEventProxy;
use super::sync::FairMutex;

/// PTY read/write event token (Alacritty's private constant, redefined here)
const PTY_READ_WRITE_TOKEN: usize = 0;

/// PTY child event token (Alacritty's private constant, redefined here)
const PTY_CHILD_EVENT_TOKEN: usize = 1;

/// Read buffer size: 1MB reused buffer (Alacritty's pattern)
const READ_BUFFER_SIZE: usize = 0x10_0000;

/// Event capacity for polling (1024 events per poll)
const EVENT_CAPACITY: NonZeroUsize = NonZeroUsize::new(1024).unwrap();

/// Maximum bytes to process while holding terminal lock (prevent starvation)
const MAX_LOCKED_READ: usize = u16::MAX as usize;

/// Tracks a write in progress with cursor-based partial write handling
/// Follows Alacritty's pattern from event_loop.rs:326-362
struct Writing {
    source: Vec<u8>,
    written: usize,
}

impl Writing {
    fn new(source: Vec<u8>) -> Self {
        Self { source, written: 0 }
    }

    fn advance(&mut self, n: usize) {
        self.written += n;
    }

    fn remaining_bytes(&self) -> &[u8] {
        &self.source[self.written..]
    }

    fn finished(&self) -> bool {
        self.written >= self.source.len()
    }
}

/// Event loop state that persists across poll iterations
struct EventLoopState {
    /// Queue of bytes waiting to be written to PTY
    write_queue: VecDeque<Vec<u8>>,
    /// Current write in progress with cursor tracking
    writing: Option<Writing>,
    /// VTE processor (processes ANSI escape sequences)
    /// Lives in event loop thread, no mutex needed
    processor: Processor,
}

impl Default for EventLoopState {
    fn default() -> Self {
        Self {
            write_queue: VecDeque::new(),
            writing: None,
            processor: Processor::new(),
        }
    }
}

impl EventLoopState {
    fn take_current(&mut self) -> Option<Writing> {
        self.writing.take()
            .or_else(|| self.write_queue.pop_front().map(Writing::new))
    }

    fn set_current(&mut self, writing: Option<Writing>) {
        self.writing = writing;
    }

    fn goto_next(&mut self) {
        self.writing = self.write_queue.pop_front().map(Writing::new);
    }
}

/// Sender for terminal input that wakes the event loop immediately
///
/// Wraps an mpsc::Sender with a reference to the poller, enabling
/// instant event loop wakeup via `poller.notify()` after sending data.
/// This eliminates the 0-10ms latency from polling timeouts.
#[derive(Clone)]
pub(super) struct InputSender {
    sender: mpsc::Sender<Vec<u8>>,
    poller: Arc<Poller>,
}

impl InputSender {
    pub(super) fn new(sender: mpsc::Sender<Vec<u8>>, poller: Arc<Poller>) -> Self {
        Self { sender, poller }
    }

    pub(super) fn send(&self, bytes: Vec<u8>) -> io::Result<()> {
        self.sender.send(bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e))?;
        self.poller.notify()?;  // Wake event loop immediately!
        Ok(())
    }
}

/// Spawn the event loop thread with a generic PTY type
///
/// This creates a single thread that:
/// 1. Polls the PTY for read/write readiness using epoll/kqueue/IOCP
/// 2. Reads from PTY when data is available
/// 3. Processes VTE sequences and updates terminal state
/// 4. Writes queued input to PTY when writable
/// 5. Detects child process exit
/// 6. Broadcasts screen updates
///
/// # Generic Parameters
/// * `T` - PTY implementation that must implement `EventedPty` from Alacritty.
///   This allows the event loop to work with any platform-specific PTY (Unix/Windows).
///
/// The PTY type T must implement EventedPty, which Alacritty's Pty already does.
/// This function takes ownership of the PTY and moves it into the event loop thread.
///
/// Based on Alacritty's event_loop.rs pattern with generic type parameter.
pub fn spawn_event_loop<T>(
    mut pty: T,
    term: Arc<FairMutex<Term<HeadlessEventProxy>>>,
    output_broadcast: Arc<broadcast::Sender<()>>,
    pty_closed: Arc<AtomicBool>,
) -> io::Result<(std::thread::JoinHandle<()>, InputSender)>
where
    T: EventedPty + Send + 'static,
{
    // Create mpsc channel for input
    let (tx, rx) = mpsc::channel();

    // Create poller BEFORE thread so we can clone it for InputSender
    let poller = Arc::new(Poller::new()?);

    // Create InputSender before moving into thread
    let input_sender = InputSender::new(tx, poller.clone());

    let handle = std::thread::spawn(move || {
        log::info!("PTY event loop starting");

        let mut state = EventLoopState::default();
        let mut buf = [0u8; READ_BUFFER_SIZE];

        // Register PTY for read events (writable added on-demand)
        let poll_opts = PollMode::Level;
        let mut interest = Event::readable(PTY_READ_WRITE_TOKEN);

        // SAFETY: The PTY is owned by this thread and will outlive the registration
        if let Err(err) = unsafe { pty.register(&poller, interest, poll_opts) } {
            log::error!("Event loop registration error: {err}");
            pty_closed.store(true, Ordering::SeqCst);
            return;
        }

        let mut events = Events::with_capacity(EVENT_CAPACITY);

        'event_loop: loop {
            // Drain channel (non-blocking) - get all pending writes
            while let Ok(bytes) = rx.try_recv() {
                state.write_queue.push_back(bytes);
            }

            // Poll with 10ms timeout (allows periodic channel checks even if no I/O)
            events.clear();
            log::debug!("Polling for events...");
            if let Err(err) = poller.wait(&mut events, Some(std::time::Duration::from_millis(10))) {
                match err.kind() {
                    ErrorKind::Interrupted => continue,
                    _ => {
                        log::error!("Poll error: {}", err);
                        break 'event_loop;
                    }
                }
            }

            log::debug!("Poll returned {} events", events.iter().count());

            // Process events
            for event in events.iter() {
                log::debug!(
                    "Processing event: key={}, readable={}, writable={}",
                    event.key,
                    event.readable,
                    event.writable
                );

                match event.key {
                    PTY_READ_WRITE_TOKEN => {
                        if event.is_interrupt() {
                            // Don't try to do I/O on a dead PTY
                            continue;
                        }

                        if event.readable {
                            match pty_read(&mut pty, &term, &mut state, &mut buf, &output_broadcast) {
                                Ok(0) => {
                                    log::info!("PTY EOF detected");
                                    pty_closed.store(true, Ordering::SeqCst);
                                    break 'event_loop;
                                }
                                Ok(_bytes_read) => {
                                    // Successfully read and processed
                                }
                                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                                    // No data available
                                }
                                Err(e) => {
                                    log::error!("PTY read error: {}", e);
                                    break 'event_loop;
                                }
                            }
                        }

                        if event.writable
                            && let Err(e) = pty_write(&mut pty, &mut state) {
                                log::error!("PTY write error: {}", e);
                                break 'event_loop;
                            }
                    }

                    PTY_CHILD_EVENT_TOKEN => {
                        if let Some(ChildEvent::Exited(code)) = pty.next_child_event() {
                            log::info!("Child process exited with code: {:?}", code);
                            pty_closed.store(true, Ordering::SeqCst);
                            break 'event_loop;
                        }
                    }

                    _ => {}
                }
            }

            // Update write interest if needed
            let needs_write = state.writing.is_some() || !state.write_queue.is_empty();
            if needs_write != interest.writable {
                interest.writable = needs_write;
                if let Err(e) = pty.reregister(&poller, interest, poll_opts) {
                    log::error!("Failed to reregister PTY: {}", e);
                    break 'event_loop;
                }
            }
        }

        // Cleanup: deregister PTY
        let _ = pty.deregister(&poller);

        pty_closed.store(true, Ordering::SeqCst);
        log::info!("PTY event loop finished");
    });

    Ok((handle, input_sender))
}

/// Read from PTY and process VTE sequences
///
/// This follows Alacritty's pattern:
/// 1. Read as much as possible from PTY
/// 2. Try to acquire terminal lock (non-blocking)
/// 3. If buffer is full, force blocking lock
/// 4. Process VTE sequences while holding lock
/// 5. Release lock after MAX_LOCKED_READ bytes to prevent starvation
/// 6. Broadcast screen contents
fn pty_read<T: EventedPty>(
    pty: &mut T,
    term: &Arc<FairMutex<Term<HeadlessEventProxy>>>,
    state: &mut EventLoopState,
    buf: &mut [u8],
    output_broadcast: &broadcast::Sender<()>,
) -> io::Result<usize> {
    log::debug!("pty_read: Starting");
    let mut unprocessed = 0;
    let mut processed = 0;
    let mut terminal_lock = None;

    // Reserve the next terminal lock for PTY reading (prevents starvation from external API calls)
    let _terminal_lease = Some(term.lease());

    loop {
        // Read from PTY (non-blocking when data available)
        log::debug!("pty_read: Attempting to read from PTY, unprocessed={}", unprocessed);
        match pty.reader().read(&mut buf[unprocessed..]) {
            Ok(0) if unprocessed == 0 => {
                log::debug!("pty_read: EOF detected");
                return Ok(0);
            }
            Ok(n) => {
                log::debug!("pty_read: Read {} bytes from PTY", n);
                unprocessed += n;
            }
            Err(err) => match err.kind() {
                ErrorKind::Interrupted | ErrorKind::WouldBlock => {
                    log::debug!("pty_read: WouldBlock/Interrupted, unprocessed={}", unprocessed);
                    if unprocessed == 0 {
                        break;  // Caught up, return to poller
                    }
                }
                _ => {
                    log::error!("pty_read: Read error: {}", err);
                    return Err(err);
                }
            },
        }

        // Attempt to lock terminal (Alacritty pattern from event_loop.rs:137-145)
        log::debug!("pty_read: Attempting to lock terminal, unprocessed={}", unprocessed);
        let term_guard = match &mut terminal_lock {
            Some(guard) => guard,
            None => {
                terminal_lock.insert(match term.try_lock_unfair() {
                    // Force blocking lock if buffer is full (must process or we'll deadlock)
                    None if unprocessed >= READ_BUFFER_SIZE => term.lock_unfair(),
                    None => continue,  // Can't lock, keep reading
                    Some(guard) => guard,
                })
            }
        };

        // Process VTE bytes (blocking, synchronous - NO async, NO tokio runtime!)
        // ONLY ONE LOCK NOW - processor lives in state, no mutex needed
        state.processor.advance(&mut **term_guard, &buf[..unprocessed]);

        processed += unprocessed;
        unprocessed = 0;

        // Don't hold terminal lock too long (Alacritty pattern from event_loop.rs:162-165)
        if processed >= MAX_LOCKED_READ {
            break;
        }
    }

    // Send notification if screen was updated (Alacritty pattern from event_loop.rs:167)
    if processed > 0 {
        // Broadcast lightweight notification (non-blocking, ignore if no receivers)
        // Receivers call get_output() or screen() to get actual data
        let _ = output_broadcast.send(());
    }

    Ok(processed)
}

/// Write queued bytes to PTY with zero-allocation cursor pattern
///
/// Follows Alacritty's pattern from event_loop.rs:298-322
fn pty_write<T: EventedPty>(
    pty: &mut T,
    state: &mut EventLoopState,
) -> io::Result<()> {
    'write_many: while let Some(mut current) = state.take_current() {
        'write_one: loop {
            match pty.writer().write(current.remaining_bytes()) {
                Ok(0) => {
                    state.set_current(Some(current));
                    break 'write_many;
                }
                Ok(n) => {
                    current.advance(n);  // NO ALLOCATION - just cursor increment!
                    if current.finished() {
                        state.goto_next();
                        break 'write_one;
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    state.set_current(Some(current));
                    break 'write_many;
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => {
                    state.set_current(Some(current));
                    return Err(e);
                }
            }
        }
    }
    Ok(())
}
