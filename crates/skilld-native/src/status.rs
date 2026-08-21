use std::io::Write;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

use skilld_command::OutputContext;

const ERASE_LINE: &[u8] = b"\r\x1b[2K";

struct StatusState {
    stopped: bool,
    out: Box<dyn Write + Send>,
    label: String,
    started: Instant,
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

    pub fn begin(label: &str, tick: Duration, out: Box<dyn Write + Send>) -> Self {
        let shared = Arc::new(Mutex::new(StatusState {
            stopped: false,
            out,
            label: label.to_owned(),
            started: Instant::now(),
        }));
        let worker = Arc::clone(&shared);
        let thread = thread::spawn(move || {
            loop {
                let mut state = match worker.lock() {
                    Ok(state) => state,
                    Err(_) => return,
                };
                if state.stopped {
                    return;
                }
                let seconds = state.started.elapsed().as_secs();
                let line = format!("\r\x1b[2K{}… {seconds}s", state.label);
                let _ = state.out.write_all(line.as_bytes());
                let _ = state.out.flush();
                drop(state);
                thread::sleep(tick);
            }
        });
        Self {
            shared: Some(shared),
            thread: Some(thread),
        }
    }

    pub fn for_terminal(label: &str, context: OutputContext) -> Self {
        if !matches!(context, OutputContext::HumanTerminal { .. }) {
            return Self::disabled();
        }
        Self::begin(
            label,
            Duration::from_millis(500),
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

#[cfg(test)]
mod tests {
    use super::{GatedStderr, OutputContext, StatusLine, status_label};
    use std::io::Write;
    use std::sync::Mutex;
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
    fn finish_erases_the_line() {
        let buffer = std::sync::Arc::new(Mutex::new(Vec::new()));
        let status = StatusLine::begin("Searching", never(), Box::new(Writer(buffer.clone())));
        status.finish();
        assert_eq!(output(&buffer), "\r\x1b[2K");
    }

    #[test]
    fn stop_erases_once_and_is_idempotent() {
        let buffer = std::sync::Arc::new(Mutex::new(Vec::new()));
        let mut status = StatusLine::begin("Searching", never(), Box::new(Writer(buffer.clone())));
        status.stop();
        status.stop();
        status.finish();
        assert_eq!(output(&buffer), "\r\x1b[2K");
    }

    #[test]
    fn a_gated_writer_stops_the_line_before_forwarding() {
        let buffer = std::sync::Arc::new(Mutex::new(Vec::new()));
        let status = StatusLine::begin("Searching", never(), Box::new(Writer(buffer.clone())));
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
