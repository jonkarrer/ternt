//! The tools an agent can be given that need nothing but the network: no browser, no files.
//! Each is switched on per space by its name (`configure`), and runs only when the process
//! has the key it needs.

mod web_search;

pub use web_search::WebSearch;
