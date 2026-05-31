/// Maps winit keyboard/mouse events to terminal input bytes or mux commands.
pub enum InputEvent {
    Bytes(Vec<u8>),
    NewTab,
    CloseTab,
    SplitHorizontal,
    SplitVertical,
    FocusPane(u32),
    AcceptHint,
}
