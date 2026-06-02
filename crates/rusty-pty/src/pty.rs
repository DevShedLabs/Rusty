use anyhow::Result;
use crossbeam_channel::{bounded, Receiver, Sender};
use portable_pty::{native_pty_system, CommandBuilder, PtySize as RawSize};
use std::thread;

#[derive(Debug, Clone, Copy)]
pub struct PtySize {
    pub rows: u16,
    pub cols: u16,
    pub px_w: u16,
    pub px_h: u16,
}

pub struct Pty {
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn std::io::Write + Send>,
    /// Bytes received from the shell.
    pub rx: Receiver<Vec<u8>>,
    /// Fires (with no payload) whenever bytes arrive — safe to clone for a watcher thread.
    pub notify: Receiver<()>,
}

impl Pty {
    pub fn spawn(shell: &str, size: PtySize) -> Result<Self> {
        let sys = native_pty_system();
        let raw = RawSize {
            rows: size.rows,
            cols: size.cols,
            pixel_width: size.px_w,
            pixel_height: size.px_h,
        };
        let pair = sys.openpty(raw)?;
        let mut cmd = CommandBuilder::new(shell);
        // Identify ourselves so shells and tools can detect the terminal.
        cmd.env("TERM_PROGRAM", "rusty");
        cmd.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        // Disable macOS Terminal.app session save/restore — it errors in any other terminal.
        cmd.env("SHELL_SESSION_DID_INIT", "1");
        pair.slave.spawn_command(cmd)?;

        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        let (tx, rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = bounded(256);
        let (notify_tx, notify): (Sender<()>, Receiver<()>) = bounded(1);

        thread::Builder::new()
            .name("pty-reader".into())
            .spawn(move || {
                let mut buf = vec![0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if tx.send(buf[..n].to_vec()).is_err() {
                                break;
                            }
                            // Best-effort notify — if the channel is already full (capacity 1)
                            // the watcher is already awake, so dropping the send is fine.
                            let _ = notify_tx.try_send(());
                        }
                        Err(_) => break,
                    }
                }
            })?;

        Ok(Self { master: pair.master, writer, rx, notify })
    }

    pub fn write_bytes(&mut self, data: &[u8]) -> Result<()> {
        use std::io::Write;
        self.writer.write_all(data)?;
        Ok(())
    }

    pub fn resize(&self, size: PtySize) -> Result<()> {
        self.master.resize(RawSize {
            rows: size.rows,
            cols: size.cols,
            pixel_width: size.px_w,
            pixel_height: size.px_h,
        })?;
        Ok(())
    }
}
