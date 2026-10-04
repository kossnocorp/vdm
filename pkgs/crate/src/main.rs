use usage::RunAsync;
use vdm_fyi::VdmCli;

#[tokio::main]
async fn main() {
    VdmCli::parse().run_async().await.unwrap_or_else(|err| {
        println!("Error: {:?}", err);
        std::process::exit(1);
    });
}
