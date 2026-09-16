use crate::prelude::*;

use reqwest::{Client, Url};

pub static VDM_SOURCE_HTTP: VdmSourceHttp = VdmSourceHttp;

pub struct VdmSourceHttp;

#[derive(Clone, Debug, PartialEq)]
pub struct VdmSourceHttpTarget {
    url: Url,
    key: VdmManifestTargetUrl,
    version: VdmManifestSourceVersion,
    vendor_path: PathBuf,
}

#[async_trait]
impl VdmSource for VdmSourceHttp {
    fn parse(&self, input: &str) -> Result<Option<Box<dyn VdmTarget>>> {
        if !input.starts_with("http://") && !input.starts_with("https://") {
            return Ok(None);
        }

        let url = Url::parse(input).with_context(|| format!("invalid HTTP URL {input:?}"))?;
        let host = url.host_str().context("HTTP URL must include a host")?;
        let path = url.path().trim_start_matches('/');
        ensure!(!path.is_empty(), "HTTP URL must identify a file");
        ensure!(
            !path.ends_with('/'),
            "HTTP URL must not end with a directory"
        );
        ensure!(
            Path::new(path)
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
            "HTTP URL path must not contain traversal components"
        );
        let authority = match url.port() {
            Some(port) => format!("{host}_{port}"),
            None => host.to_owned(),
        };

        let key = url.to_string();
        Ok(Some(Box::new(VdmSourceHttpTarget {
            key: VdmManifestTargetUrl::new(&key),
            version: VdmManifestSourceVersion::new(&key),
            vendor_path: PathBuf::from("@http").join(authority).join(path),
            url,
        })))
    }

    async fn download(&self, target: &dyn VdmTarget) -> Result<VdmSourceFile> {
        let target = target
            .as_any()
            .downcast_ref::<VdmSourceHttpTarget>()
            .context("HTTP source received a target from another source")?;
        let client = Client::builder()
            .user_agent("vdm/0.1")
            .build()
            .context("failed to create HTTP client")?;
        let response = client
            .get(target.url.clone())
            .send()
            .await
            .with_context(|| format!("failed to fetch {}", target.url))?
            .error_for_status()
            .with_context(|| format!("HTTP server could not fetch {}", target.url))?;
        let revision = response.url().to_string();
        let bytes = response
            .bytes()
            .await
            .context("failed to read downloaded file")?
            .to_vec();

        Ok(VdmSourceFile { revision, bytes })
    }
}

impl VdmTarget for VdmSourceHttpTarget {
    fn key(&self) -> &VdmManifestTargetUrl {
        &self.key
    }

    fn version(&self) -> &VdmManifestSourceVersion {
        &self.version
    }

    fn source_url(&self) -> &str {
        self.key.as_str()
    }

    fn vendor_path(&self) -> PathBuf {
        self.vendor_path.clone()
    }

    fn source(&self) -> &'static dyn VdmSource {
        &VDM_SOURCE_HTTP
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn parses_direct_http_urls_only() {
        let target = VDM_SOURCE_HTTP
            .parse("https://example.com/assets/file.js?raw=1")
            .unwrap()
            .unwrap();
        assert_eq!(
            target.key(),
            &VdmManifestTargetUrl::new("https://example.com/assets/file.js?raw=1")
        );
        assert_eq!(
            target.vendor_path(),
            Path::new("@http/example.com/assets/file.js")
        );
        assert!(
            VDM_SOURCE_HTTP
                .parse("gh:owner/repo/file@main")
                .unwrap()
                .is_none()
        );
        assert!(VDM_SOURCE_HTTP.parse("https://example.com/").is_err());
    }

    #[tokio::test]
    async fn downloads_a_direct_http_url() {
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
        let target = VDM_SOURCE_HTTP
            .parse(&format!("http://{address}/file.txt"))
            .unwrap()
            .unwrap();

        let download = target.source().download(target.as_ref()).await.unwrap();

        assert_eq!(download.bytes, b"hello");
        server.join().unwrap();
    }
}
