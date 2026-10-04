use vdm_fyi::VdmCheckout;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let input = std::env::args().nth(1).expect("expected a remote source");
    let checkout = VdmCheckout::fetch(&input).await?;
    println!("{}", checkout.path().display());
    // Read or copy the file/directory while `checkout` is alive.
    // Dropping it removes the snapshot, retaining vdm's shared Git cache.
    Ok(())
}
