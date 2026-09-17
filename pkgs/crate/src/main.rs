mod prelude;

mod cli;
pub use cli::*;

mod state;
pub use state::*;

mod manifest;
pub use manifest::*;

mod lock;
pub use lock::*;

mod source;
pub use source::*;

mod target;
pub use target::*;

mod http;
pub use http::*;

mod github;
pub use github::*;

mod git;
pub use git::*;

mod javascript;
pub use javascript::*;

mod rust;
pub(crate) use rust::*;

mod graph;
pub use graph::*;

pub mod file;
pub use file::*;

mod paths;
pub use paths::*;

mod dirs;
pub use dirs::*;

mod vendor;
use usage::RunAsync;
pub use vendor::*;

#[tokio::main]
async fn main() {
    VdmCli::parse().run_async().await.unwrap_or_else(|err| {
        println!("Error: {:?}", err);
        std::process::exit(1);
    });
}
