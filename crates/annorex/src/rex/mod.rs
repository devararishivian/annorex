//! The Rex adapter. Every call goes through the `rex` CLI. The arguments of a block method
//! or an API method go on standard input, never in argv.

mod cli;
mod types;

pub use cli::{Rex, RexError, Runner, SystemRunner, find_binary};
pub use types::{
    BlockInfo, Layer, LayerKind, LayerState, Located, Mode, Modes, NewLayer, Owner, Proc, ProcessInfo,
    SessionEntry, SessionList, SessionView, Size, StatusList, StatusRecord, Window,
};
