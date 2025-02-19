#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

mod env;
mod error;
mod eval;
mod parser;
mod policy;
mod traits;
mod types;

pub mod expr;
pub mod mem;

pub use env::Env;
pub use error::{EvalError, ParseError};
pub use eval::eval;
pub use expr::Expr;
pub use parser::parse;
pub use policy::PolicyAccessControl;
pub use traits::PolicyStorage;
pub use types::{Action, Resource, Subject};
