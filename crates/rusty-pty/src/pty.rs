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
        let cmd = CommandBuilder::new(shell);
        pair.slave.spawn_command(cmd)?;

        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        let (tx, rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = bounded(256);

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
                        }
                        Err(_) => break,
                    }
                }
            })?;

        Ok(Self { master: pair.master, writer, rx })
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
