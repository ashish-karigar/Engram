//! Language support shared by the code graph and the document chunker: which language a file is,
//! its symbols (with line spans), and its imports, plus resolving imports to project files; the
//! HTTP routes and calls that connect parts of a system; and the parts (areas) themselves.

pub mod areas;
pub mod lang;
pub mod net;
pub mod python;
pub mod resolve;
pub mod rust;
pub mod ts;

pub use lang::{CallSite, FileIndex, Import, Lang, Symbol, parse_file};
pub use resolve::Resolver;
pub use areas::{Area, area_of, detect};
pub use net::{Call, Net, NetLang, Route};
