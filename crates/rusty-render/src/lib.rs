pub mod doc;
pub mod markdown;
pub mod json;
pub mod trigger;

pub use doc::{RenderDoc, Span, Style, Color};
pub use trigger::{RenderTrigger, detect_trigger, BuiltinCommand, detect_builtin};
