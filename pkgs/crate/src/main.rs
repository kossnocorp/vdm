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
    VitCli::parse().run_async().await.unwrap_or_else(|err| {
        println!("Error: {:?}", err);
        std::process::exit(1);
    });
}
