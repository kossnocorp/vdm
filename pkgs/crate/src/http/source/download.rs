use crate::prelude::*;

use reqwest::Client;

impl VdmHttpSource {
    pub(super) async fn download_file(&self, target: &dyn VdmTarget) -> Result<VdmSourceFile> {
        let target = target
            .as_any()
            .downcast_ref::<VdmHttpTarget>()
            .context("HTTP source received a target from another source")?;
        let client = Client::builder()
            .user_agent("vdm/0.1")
            .build()
            .context("failed to create HTTP client")?;
        let response = client
            .get(target.url().clone())
            .send()
            .await
            .with_context(|| format!("failed to fetch {}", target.url()))?
            .error_for_status()
            .with_context(|| format!("HTTP server could not fetch {}", target.url()))?;
        let revision = response.url().to_string();
        let bytes = response
            .bytes()
            .await
            .context("failed to read downloaded file")?
            .to_vec();

        let hash = format!("sha256:{:x}", Sha256::digest(&bytes));
        ensure!(
            target.version().as_str().is_empty() || target.version().as_str() == hash,
            "HTTP content hash mismatch for {}: expected {}, got {hash}",
            target.key(),
            target.version()
        );
        Ok(VdmSourceFile { revision, bytes })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[tokio::test]
    async fn downloads_a_direct_http_url() {
        check_download(None, true).await;
        check_download(Some(format!("sha256:{:x}", Sha256::digest(b"hello"))), true).await;
        check_download(Some(format!("sha256:{}", "0".repeat(64))), false).await;
    }

    async fn check_download(pin: Option<String>, succeeds: bool) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            let _read = stream.read(&mut request).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
                )
                .unwrap();
        });
        let input = format!(
            "http://{address}/file.txt{}",
            pin.map(|pin| format!("@{pin}")).unwrap_or_default()
        );
        let target = VDM_HTTP_SOURCE.parse(&input).unwrap().unwrap();

        let download = target.source().download(target.as_ref()).await;
        if succeeds {
            assert_eq!(download.unwrap().bytes, b"hello");
        } else {
            assert!(
                download
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("hash mismatch")
            );
        }
        server.join().unwrap();
    }
}
