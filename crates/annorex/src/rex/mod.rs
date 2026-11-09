//! The Rex adapter. Every call goes through the `rex` CLI. The arguments of a block method
//! go on standard input, never in argv.

mod cli;
mod types;

pub use cli::{Rex, RexError, Runner, SystemRunner};
pub use types::{Mode, Modes, Owner, Proc, ProcessInfo, StatusList, StatusRecord};
