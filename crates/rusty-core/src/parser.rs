/// Actions emitted by the VTE parser that the terminal engine acts on.
#[derive(Debug)]
pub enum Action {
    Print(char),
    Execute(u8),
    CsiDispatch { params: Vec<i64>, intermediates: Vec<u8>, ignore: bool, action: char },
    OscDispatch { params: Vec<Vec<u8>>, bell_terminated: bool },
    Hook { params: Vec<i64>, intermediates: Vec<u8>, ignore: bool, action: char },
    Put(u8),
    Unhook,
}

/// Thin wrapper around the `vte` crate that collects actions into a Vec.
pub struct Parser {
    inner:   vte::Parser,
    pending: Vec<Action>,
}

struct Collector<'a>(&'a mut Vec<Action>);

impl vte::Perform for Collector<'_> {
    fn print(&mut self, c: char) {
        self.0.push(Action::Print(c));
    }

    fn execute(&mut self, byte: u8) {
        self.0.push(Action::Execute(byte));
    }

    fn csi_dispatch(&mut self, params: &vte::Params, intermediates: &[u8], ignore: bool, action: char) {
        let params: Vec<i64> = params.iter().flat_map(|s| s.iter().map(|&v| v as i64)).collect();
        self.0.push(Action::CsiDispatch {
            params,
            intermediates: intermediates.to_vec(),
            ignore,
            action,
        });
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
        self.0.push(Action::OscDispatch {
            params: params.iter().map(|p| p.to_vec()).collect(),
            bell_terminated,
        });
    }

    fn hook(&mut self, params: &vte::Params, intermediates: &[u8], ignore: bool, action: char) {
        let params: Vec<i64> = params.iter().flat_map(|s| s.iter().map(|&v| v as i64)).collect();
        self.0.push(Action::Hook { params, intermediates: intermediates.to_vec(), ignore, action });
    }

    fn put(&mut self, byte: u8) {
        self.0.push(Action::Put(byte));
    }

    fn unhook(&mut self) {
        self.0.push(Action::Unhook);
    }
}

impl Parser {
    pub fn new() -> Self {
        Self { inner: vte::Parser::new(), pending: Vec::new() }
    }

    pub fn advance(&mut self, bytes: &[u8]) -> Vec<Action> {
        let mut collector = Collector(&mut self.pending);
        for &b in bytes {
            self.inner.advance(&mut collector, b);
        }
        std::mem::take(&mut self.pending)
    }
}

impl Default for Parser {
    fn default() -> Self { Self::new() }
}
