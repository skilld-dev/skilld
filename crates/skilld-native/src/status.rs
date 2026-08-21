use skilld_command::OutputContext;
use skilld_ui::spinner;
use skilld_ui::theme::{RESET, Role, paint};

use std::io::Write;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

const ERASE_LINE: &[u8] = b"\r\x1b[2K";
/// The delay before the spinner first paints, so fast operations never flash.
const START_DELAY: Duration = Duration::from_millis(500);

struct StatusState {
    stopped: bool,
    out: Box<dyn Write + Send>,
    label: String,
    started: Instant,
    frame: usize,
    color: bool,
}

/// The spinner line for one frame. Pure so tests can pin the exact bytes.
fn frame_line(label: &str, frame: usize, seconds: u64, color: bool) -> String {
    let glyph = paint(spinner::frame(frame), Role::Brand, color);
    let text = paint(label, Role::Emphasis, color);
    let clock = paint(&format!("{seconds}s"), Role::Dim, color);
    let reset = if color { RESET } else { "" };
    format!("\r\x1b[2K{glyph} {text}\u{2026} {clock}{reset}")
}

pub struct StatusLine {
    shared: Option<Arc<Mutex<StatusState>>>,
    thread: Option<JoinHandle<()>>,
}

impl StatusLine {
    pub fn disabled() -> Self {
        Self {
            shared: None,
            thread: None,
        }
    }

    #[cfg(test)]
    pub fn is_disabled(&self) -> bool {
        self.shared.is_none()
    }

    pub fn begin(
        label: &str,
        tick: Duration,
        delay: Duration,
        color: bool,
        out: Box<dyn Write + Send>,
    ) -> Self {
        let shared = Arc::new(Mutex::new(StatusState {
            stopped: false,
            out,
            label: label.to_owned(),
            started: Instant::now(),
            frame: 0,
            color,
        }));
        let worker = Arc::clone(&shared);
        let thread = thread::spawn(move || {
            loop {
                // Sleep in short slices so a stop is noticed even with a long tick.
                thread::sleep(tick.min(Duration::from_millis(250)));
                let mut state = match worker.lock() {
                    Ok(state) => state,
                    Err(_) => return,
                };
                if state.stopped {
                    return;
                }
                if state.started.elapsed() >= delay {
                    let line = frame_line(
                        &state.label,
                        state.frame,
                        state.started.elapsed().as_secs(),
                        state.color,
                    );
                    let _ = state.out.write_all(line.as_bytes());
                    let _ = state.out.flush();
                    state.frame += 1;
                }
                drop(state);
            }
        });
        Self {
            shared: Some(shared),
            thread: Some(thread),
        }
    }

    pub fn for_terminal(label: &str, context: OutputContext) -> Self {
        let OutputContext::HumanTerminal { color, .. } = context else {
            return Self::disabled();
        };
        Self::begin(
            label,
            Duration::from_millis(100),
            START_DELAY,
            color,
            Box::new(std::io::stderr()),
        )
    }

    pub fn stop(&mut self) {
        let Some(shared) = &self.shared else {
            return;
        };
        let mut state = match shared.lock() {
            Ok(state) => state,
            Err(_) => return,
        };
        if !state.stopped {
            state.stopped = true;
            let _ = state.out.write_all(ERASE_LINE);
            let _ = state.out.flush();
        }
    }

    pub fn finish(mut self) {
        self.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub struct GatedStderr<'a, W: Write> {
    inner: &'a mut W,
    status: Option<StatusLine>,
}

impl<'a, W: Write> GatedStderr<'a, W> {
    pub fn new(inner: &'a mut W, status: StatusLine) -> Self {
        Self {
            inner,
            status: Some(status),
        }
    }

    pub fn finish_status(&mut self) {
        if let Some(status) = self.status.take() {
            status.finish();
        }
    }
}

impl<W: Write> Write for GatedStderr<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.finish_status();
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.finish_status();
        self.inner.flush()
    }
}

pub fn status_label<I, S>(args: I) -> Option<&'static str>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut args = args.into_iter();
    let _binary = args.next()?;
    let mut subcommand = None;
    for arg in args {
        let arg = arg.as_ref();
        if arg == "--json" {
            return None;
        }
        if arg.starts_with('-') {
            continue;
        }
        subcommand = Some(arg.to_owned());
        break;
    }
    match subcommand.as_deref() {
        Some("search") => Some("Searching"),
        Some("install") => Some("Installing"),
        Some("view") => Some("Loading"),
        Some("verify") => Some("Verifying"),
        Some("update") => Some("Updating"),
        _ => None,
    }
}

/// Streams `skilld outdated` progress to a terminal: found Skills print as
/// lines, remote verification rewrites one spinner line, and `finish` erases
/// it so only results remain.
pub struct OutdatedProgressLine {
    enabled: bool,
    color: bool,
    frame: std::sync::atomic::AtomicUsize,
}

impl OutdatedProgressLine {
    /// Progress streams only for humans on a terminal. Agents, CI, and pipes
    /// get quiet stderr.
    pub fn for_terminal(is_terminal: bool, active_agent: bool, color: bool) -> Self {
        Self {
            enabled: is_terminal && !active_agent,
            color,
            frame: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn erase(&self) {
        if self.enabled {
            let mut stderr = std::io::stderr().lock();
            let _ = stderr.write_all(b"\r\x1b[2K");
            let _ = stderr.flush();
        }
    }
}

impl skilld_command::OutdatedProgress for OutdatedProgressLine {
    fn found(&self, line: &str) {
        if !self.enabled {
            return;
        }
        self.erase();
        let mut stderr = std::io::stderr().lock();
        let _ = writeln!(
            stderr,
            "{}",
            paint(&format!("• {line}"), Role::Dim, self.color)
        );
        let _ = stderr.flush();
    }

    fn checking(&self, name: &str) {
        if !self.enabled {
            return;
        }
        let frame = spinner::frame(self.frame.fetch_add(1, Ordering::Relaxed));
        let glyph = paint(frame, Role::Brand, self.color);
        let mut stderr = std::io::stderr().lock();
        let _ = write!(stderr, "\r\x1b[2K{glyph} Checking {name}…");
        let _ = stderr.flush();
    }

    fn finish(&self) {
        self.erase();
    }
}

#[cfg(test)]
mod tests {
    use super::{GatedStderr, OutputContext, StatusLine, frame_line, status_label};
    use std::io::Write;
    use std::sync::Mutex;
    use std::thread;
    use std::time::Duration;

    fn never() -> Duration {
        Duration::MAX
    }

    fn output(buffer: &Mutex<Vec<u8>>) -> String {
        String::from_utf8(buffer.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn a_disabled_line_writes_nothing() {
        StatusLine::disabled().finish();
    }

    #[test]
    fn frame_lines_lead_with_the_spinner_glyph() {
        assert_eq!(
            frame_line("Searching", 0, 2, false),
            "\r\x1b[2K⠋ Searching… 2s"
        );
        let colored = frame_line("Searching", 1, 2, true);
        assert!(colored.starts_with("\r\x1b[2K\u{1b}[1m\u{1b}[36m⠙"));
        assert!(colored.contains("Searching"));
    }

    #[test]
    fn nothing_paints_before_the_start_delay() {
        let buffer = std::sync::Arc::new(Mutex::new(Vec::new()));
        let status = StatusLine::begin(
            "Searching",
            Duration::from_millis(2),
            Duration::from_secs(30),
            false,
            Box::new(Writer(buffer.clone())),
        );
        thread::sleep(Duration::from_millis(20));
        status.finish();
        assert_eq!(output(&buffer), "\r\x1b[2K");
    }

    #[test]
    fn finish_erases_the_line() {
        let buffer = std::sync::Arc::new(Mutex::new(Vec::new()));
        let status = StatusLine::begin(
            "Searching",
            never(),
            never(),
            false,
            Box::new(Writer(buffer.clone())),
        );
        status.finish();
        assert_eq!(output(&buffer), "\r\x1b[2K");
    }

    #[test]
    fn stop_erases_once_and_is_idempotent() {
        let buffer = std::sync::Arc::new(Mutex::new(Vec::new()));
        let mut status = StatusLine::begin(
            "Searching",
            never(),
            never(),
            false,
            Box::new(Writer(buffer.clone())),
        );
        status.stop();
        status.stop();
        status.finish();
        assert_eq!(output(&buffer), "\r\x1b[2K");
    }

    #[test]
    fn a_gated_writer_stops_the_line_before_forwarding() {
        let buffer = std::sync::Arc::new(Mutex::new(Vec::new()));
        let status = StatusLine::begin(
            "Searching",
            never(),
            never(),
            false,
            Box::new(Writer(buffer.clone())),
        );
        let mut sink = Vec::new();
        let mut gated = GatedStderr::new(&mut sink, status);
        gated.write_all(b"done").unwrap();
        gated.finish_status();
        assert_eq!(output(&buffer), "\r\x1b[2K");
        assert_eq!(sink, b"done");
    }

    #[test]
    fn labels_map_to_network_commands() {
        let args = ["skilld", "--json", "search", "hi"];
        assert_eq!(status_label(args), None);
        let args = ["skilld", "search", "hi"];
        assert_eq!(status_label(args), Some("Searching"));
        let args = ["skilld", "install", "skilld:owner/repo/skill"];
        assert_eq!(status_label(args), Some("Installing"));
        let args = ["skilld", "list"];
        assert_eq!(status_label(args), None);
        let args = ["skilld"];
        assert_eq!(status_label(args), None);
    }

    #[test]
    fn plain_contexts_get_no_status_line() {
        assert!(matches!(
            StatusLine::for_terminal("Searching", OutputContext::Plain),
            line if line.is_disabled()
        ));
        assert!(matches!(
            StatusLine::for_terminal(
                "Searching",
                OutputContext::HumanTerminal {
                    width: 80,
                    color: true
                }
            ),
            line if !line.is_disabled()
        ));
    }

    #[test]
    fn outdated_progress_stays_quiet_for_agents() {
        use skilld_command::OutdatedProgress;

        let agent = super::OutdatedProgressLine::for_terminal(true, true, true);
        agent.found("example (project scope)");
        agent.checking("example");
        agent.finish();
        assert!(!agent.enabled);

        let human = super::OutdatedProgressLine::for_terminal(true, false, true);
        assert!(human.enabled);

        let piped = super::OutdatedProgressLine::for_terminal(false, false, true);
        assert!(!piped.enabled);
    }

    struct Writer(std::sync::Arc<Mutex<Vec<u8>>>);

    impl Write for Writer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
