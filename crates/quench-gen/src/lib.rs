//! Writing Quench programs, and running them every way there is.

pub mod oracle;
pub mod write;

pub use oracle::{check, cores, Disagreement, Report, Told};
pub use write::{batch, name_of, program, Seeded};
