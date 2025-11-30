#![feature(thread_local)]

#[macro_use]
extern crate ctor;

pub mod pkg;

pub mod extensions;
pub mod features;
pub mod pycode;
pub mod python;
pub mod repl;

mod setup;
