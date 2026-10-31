//! Language support shared by the code graph and the document chunker: which language a file is,
//! its symbols (with line spans), and its imports, plus resolving imports to project files.

pub mod lang;
pub mod python;
pub mod resolve;
pub mod rust;
pub mod ts;

pub use lang::{FileIndex, Import, Lang, Symbol, parse_file};
pub use resolve::Resolver;
