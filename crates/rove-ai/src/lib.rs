//! AI agents in RoveTerm. An agent runs as a separate, untrusted process; everything it does on a
//! host goes through [`Policy`], which decides from the operation's assessed [`Risk`] whether it
//! runs or needs the user's approval. See `docs/subsystems/ai.md`.

pub mod acp;
pub mod assistant;
pub mod audit;
pub mod backend;
pub mod detect;
pub mod host;
pub mod llm;
pub mod mcp;
pub mod policy;
pub mod risk;
pub mod runner;
pub mod screen;
pub mod session;
pub mod shell;
pub mod tools;

pub use policy::{Answer, Decision, Mode, Operation, Policy, ToolKind};
pub use risk::{Assessment, Reason, Risk, Rules};
