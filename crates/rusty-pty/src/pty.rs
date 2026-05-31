use anyhow::Result;
use portable_pty::{CommandBuilder, PtySize as RawSize, native_pty_system};

#[derive(Debug, Clone, Copy)]
pub struct PtySize {
    pub rows:    u16,
    pub cols:    u16,
    pub px_w:    u16,
    pub px_h:    u16,
}

pub struct Pty {
    pair:   portable_pty::PtyPair,
    pub reader: Box<dyn std::io::Read + Send>,
    pub writer: Box<dyn std::io::Write + Send>,
}

impl Pty {
    pub fn spawn(shell: &str, size: PtySize) -> Result<Self> {
        let sys = native_pty_system();
        let raw = RawSize {
            rows: size.rows,
            cols: size.cols,
            pixel_width:  size.px_w,
            pixel_height: size.px_h,
        };
        let pair = sys.openpty(raw)?;
        let cmd  = CommandBuilder::new(shell);
        pair.slave.spawn_command(cmd)?;
        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        Ok(Self { pair, reader, writer })
    }

    pub fn resize(&self, size: PtySize) -> Result<()> {
        self.pair.master.resize(RawSize {
            rows: size.rows,
            cols: size.cols,
            pixel_width:  size.px_w,
            pixel_height: size.px_h,
        })?;
        Ok(())
    }
}
