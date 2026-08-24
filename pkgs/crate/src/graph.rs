use crate::prelude::*;

use git2::{ObjectType, Oid, Repository};
use oxc_resolver::{
    FileMetadata, FileSystem, ResolveError, ResolveOptions, ResolverGeneric, TsconfigDiscovery,
};
use std::io;
use std::process::Command;
use std::sync::{Arc, Mutex};

pub struct VitGraphFile {
    pub target: Box<dyn VitTarget>,
    pub download: VitSourceFile,
    pub dependencies: Vec<VitManifestTargetUrl>,
}

pub async fn resolve_graph(
    target: Box<dyn VitTarget>,
) -> Result<BTreeMap<VitManifestTargetUrl, VitGraphFile>> {
    if let Some(target) = target.as_any().downcast_ref::<VitSourceGitHubTarget>() {
        return resolve_github_graph(target.clone()).await;
    }
    if let Some(target) = target.as_any().downcast_ref::<VitSourceHttpTarget>() {
        return resolve_http_graph(target.clone()).await;
    }

    let download = target.source().download(target.as_ref()).await?;
    let key = target.key().clone();
    Ok(BTreeMap::from([(
        key,
        VitGraphFile {
            target,
            download,
            dependencies: Vec::new(),
        },
    )]))
}

async fn resolve_http_graph(
    root: VitSourceHttpTarget,
) -> Result<BTreeMap<VitManifestTargetUrl, VitGraphFile>> {
    let mut pending = vec![(root, None)];
    let mut files = BTreeMap::new();
    while let Some((target, downloaded)) = pending.pop() {
        if files.contains_key(target.key()) {
            continue;
        }
        let download = match downloaded {
            Some(download) => download,
            None => target.source().download(&target).await?,
        };
        let final_url = reqwest::Url::parse(&download.revision)
            .with_context(|| format!("Invalid final HTTP URL {:?}", download.revision))?;
        let source_path = Path::new(final_url.path());
        let mut dependencies = Vec::new();
        if is_javascript_path(source_path) {
            let specifiers = javascript_dependencies(source_path, &download.bytes)?;
            let resolver_url = final_url.clone();
            let resolved = tokio::task::spawn_blocking(move || {
                let resolver = HttpResolver::new(&resolver_url)?;
                specifiers
                    .into_iter()
                    .map(|specifier| resolver.resolve(&resolver_url, &specifier))
                    .collect::<Result<Vec<_>>>()
            })
            .await
            .context("HTTP dependency resolution task failed")??;
            for url in resolved {
                let Some(url) = url else {
                    continue;
                };
                let dependency = VitSourceInput::parse_target(url.as_str())?;
                let dependency = dependency
                    .as_any()
                    .downcast_ref::<VitSourceHttpTarget>()
                    .context("Resolved HTTP dependency has a different source")?
                    .clone();
                dependencies.push(dependency.key().clone());
                if !files.contains_key(dependency.key()) {
                    pending.push((dependency, None));
                }
            }
            dependencies.sort();
            dependencies.dedup();
        }
        files.insert(
            target.key().clone(),
            VitGraphFile {
                target: Box::new(target),
                download,
                dependencies,
            },
        );
    }
    Ok(files)
}

async fn resolve_github_graph(
    root: VitSourceGitHubTarget,
) -> Result<BTreeMap<VitManifestTargetUrl, VitGraphFile>> {
    let cache = VitGitHubCache::try_new()?;
    let root_download = cache.fetch(root.clone()).await?;
    let revision = root_download.revision.clone();
    let repository = cache.repository(&root);
    let resolver = Arc::new(GitResolver::new(repository, revision.clone()));
    let mut pending = vec![(root, Some(root_download))];
    let mut files = BTreeMap::new();

    while let Some((target, downloaded)) = pending.pop() {
        if files.contains_key(target.key()) {
            continue;
        }
        let download = match downloaded {
            Some(download) => download,
            None => cache.fetch_revision(&target, &revision).await?,
        };
        let mut dependencies = Vec::new();
        if is_javascript_path(Path::new(target.repository_path())) {
            let specifiers =
                javascript_dependencies(Path::new(target.repository_path()), &download.bytes)?;
            let importer = PathBuf::from(target.repository_path());
            let graph_resolver = Arc::clone(&resolver);
            let resolved = tokio::task::spawn_blocking(move || {
                specifiers
                    .into_iter()
                    .map(|specifier| graph_resolver.resolve(&importer, &specifier))
                    .collect::<Result<Vec<_>>>()
            })
            .await
            .context("JavaScript dependency resolution task failed")??;
            for path in resolved {
                let Some(path) = path else {
                    continue;
                };
                let dependency = target.with_path(&path)?;
                dependencies.push(dependency.key().clone());
                if !files.contains_key(dependency.key()) {
                    pending.push((dependency, None));
                }
            }
            dependencies.sort();
            dependencies.dedup();
        }

        files.insert(
            target.key().clone(),
            VitGraphFile {
                target: Box::new(target),
                download,
                dependencies,
            },
        );
    }
    Ok(files)
}

fn is_javascript_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts")
    )
}

struct GitResolver {
    resolver: ResolverGeneric<GitFileSystem>,
    fallback: ResolverGeneric<GitFileSystem>,
    root: PathBuf,
}

impl GitResolver {
    fn new(repository: PathBuf, revision: String) -> Self {
        let root = PathBuf::from("/vit");
        let alias = |extensions: &[&str]| extensions.iter().map(ToString::to_string).collect();
        let options = ResolveOptions {
            cwd: Some(root.clone()),
            tsconfig: Some(TsconfigDiscovery::Auto),
            extensions: [
                ".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs", ".json",
            ]
            .map(str::to_owned)
            .to_vec(),
            extension_alias: vec![
                (".js".to_owned(), alias(&[".ts", ".tsx", ".js", ".jsx"])),
                (".jsx".to_owned(), alias(&[".tsx", ".ts", ".jsx", ".js"])),
                (".mjs".to_owned(), alias(&[".mts", ".mjs"])),
                (".cjs".to_owned(), alias(&[".cts", ".cjs"])),
            ],
            condition_names: ["types", "import", "require", "default"]
                .map(str::to_owned)
                .to_vec(),
            main_fields: ["module", "main"].map(str::to_owned).to_vec(),
            modules: Vec::new(),
            symlinks: true,
            ..ResolveOptions::default()
        };
        let file_system = GitFileSystem {
            repository,
            revision,
            root: root.clone(),
        };
        let mut fallback_options = options.clone();
        fallback_options.tsconfig = None;
        Self {
            resolver: ResolverGeneric::new_with_file_system(file_system.clone(), options),
            fallback: ResolverGeneric::new_with_file_system(file_system, fallback_options),
            root,
        }
    }

    fn resolve(&self, importer: &Path, specifier: &str) -> Result<Option<PathBuf>> {
        let importer = self.root.join(importer);
        let resolution = self
            .resolver
            .resolve_file(&importer, specifier)
            .or_else(|_| self.fallback.resolve_file(&importer, specifier));
        match resolution {
            Ok(resolution) => {
                let path = resolution.path();
                let relative = path.strip_prefix(&self.root).with_context(|| {
                    format!(
                        "Resolved dependency {} is outside its source",
                        path.display()
                    )
                })?;
                if relative
                    .components()
                    .any(|part| part.as_os_str() == "node_modules")
                {
                    return Ok(None);
                }
                Ok(Some(relative.to_owned()))
            }
            Err(_error) if !(specifier.starts_with('.') || specifier.starts_with('#')) => Ok(None),
            Err(error) => Err(error).with_context(|| {
                format!(
                    "Failed to resolve {specifier:?} from {}",
                    importer.display()
                )
            }),
        }
    }
}

#[derive(Clone, Default)]
struct GitFileSystem {
    repository: PathBuf,
    revision: String,
    root: PathBuf,
}

impl GitFileSystem {
    fn relative(&self, path: &Path) -> io::Result<PathBuf> {
        let path = path
            .strip_prefix(&self.root)
            .map_err(|_| io::ErrorKind::NotFound)?;
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                Component::Normal(part) => normalized.push(part),
                Component::ParentDir if normalized.pop() => {}
                Component::CurDir => {}
                _ => return Err(io::ErrorKind::NotFound.into()),
            }
        }
        Ok(normalized)
    }

    fn entry(&self, path: &Path) -> io::Result<(ObjectType, i32)> {
        if path.as_os_str().is_empty() {
            return Ok((ObjectType::Tree, 0o040000));
        }
        let repository = Repository::open_bare(&self.repository).map_err(io::Error::other)?;
        let revision = Oid::from_str(&self.revision).map_err(io::Error::other)?;
        let commit = repository.find_commit(revision).map_err(io::Error::other)?;
        let tree = commit.tree().map_err(io::Error::other)?;
        let entry = tree
            .get_path(path)
            .map_err(|_| io::Error::from(io::ErrorKind::NotFound))?;
        Ok((
            entry.kind().ok_or(io::ErrorKind::InvalidData)?,
            entry.filemode(),
        ))
    }

    fn raw_read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let object = format!("{}:{}", self.revision, path.to_string_lossy());
        let output = Command::new("git")
            .arg("--git-dir")
            .arg(&self.repository)
            .args(["show", &object])
            .output()?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ))
        }
    }

    fn resolve_relative_link(&self, path: &Path) -> io::Result<PathBuf> {
        let target = String::from_utf8(self.raw_read(path)?)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let target = path.parent().unwrap_or_else(|| Path::new("")).join(target);
        let mut normalized = PathBuf::new();
        for component in target.components() {
            match component {
                Component::Normal(part) => normalized.push(part),
                Component::ParentDir if normalized.pop() => {}
                Component::CurDir => {}
                _ => return Err(io::ErrorKind::NotFound.into()),
            }
        }
        Ok(normalized)
    }
}

impl FileSystem for GitFileSystem {
    fn new() -> Self {
        Self::default()
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let path = self.relative(path)?;
        if self.entry(&path)?.1 == 0o120000 {
            self.raw_read(&self.resolve_relative_link(&path)?)
        } else {
            self.raw_read(&path)
        }
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        String::from_utf8(self.read(path)?)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    fn metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        let path = self.relative(path)?;
        let (kind, mode) = self.entry(&path)?;
        if mode == 0o120000 {
            return self.metadata(&self.root.join(self.resolve_relative_link(&path)?));
        }
        match kind {
            ObjectType::Blob => Ok(FileMetadata::new(true, false, false)),
            ObjectType::Tree => Ok(FileMetadata::new(false, true, false)),
            _ => Err(io::ErrorKind::NotFound.into()),
        }
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        let path = self.relative(path)?;
        let (kind, mode) = self.entry(&path)?;
        if mode == 0o120000 {
            Ok(FileMetadata::new(false, false, true))
        } else {
            match kind {
                ObjectType::Blob => Ok(FileMetadata::new(true, false, false)),
                ObjectType::Tree => Ok(FileMetadata::new(false, true, false)),
                _ => Err(io::ErrorKind::NotFound.into()),
            }
        }
    }

    fn read_link(&self, path: &Path) -> std::result::Result<PathBuf, ResolveError> {
        let path = self.relative(path)?;
        Ok(self.root.join(self.resolve_relative_link(&path)?))
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        let relative = self.relative(path)?;
        let relative = if self.entry(&relative)?.1 == 0o120000 {
            self.resolve_relative_link(&relative)?
        } else {
            relative
        };
        self.metadata(&self.root.join(&relative))?;
        Ok(self.root.join(relative))
    }
}

struct HttpResolver {
    resolver: ResolverGeneric<HttpFileSystem>,
    fallback: ResolverGeneric<HttpFileSystem>,
    root: PathBuf,
    origin: reqwest::Url,
}

impl HttpResolver {
    fn new(url: &reqwest::Url) -> Result<Self> {
        let mut origin = url.clone();
        origin.set_path("/");
        origin.set_query(None);
        origin.set_fragment(None);
        let root = PathBuf::from("/vit");
        let alias = |extensions: &[&str]| extensions.iter().map(ToString::to_string).collect();
        let options = ResolveOptions {
            cwd: Some(root.clone()),
            tsconfig: Some(TsconfigDiscovery::Auto),
            extensions: [
                ".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs", ".json",
            ]
            .map(str::to_owned)
            .to_vec(),
            extension_alias: vec![
                (".js".to_owned(), alias(&[".ts", ".tsx", ".js", ".jsx"])),
                (".jsx".to_owned(), alias(&[".tsx", ".ts", ".jsx", ".js"])),
                (".mjs".to_owned(), alias(&[".mts", ".mjs"])),
                (".cjs".to_owned(), alias(&[".cts", ".cjs"])),
            ],
            condition_names: ["types", "import", "require", "default"]
                .map(str::to_owned)
                .to_vec(),
            main_fields: ["module", "main"].map(str::to_owned).to_vec(),
            modules: Vec::new(),
            symlinks: false,
            ..ResolveOptions::default()
        };
        let file_system = HttpFileSystem::new_for(origin.clone(), root.clone())?;
        let mut fallback_options = options.clone();
        fallback_options.tsconfig = None;
        Ok(Self {
            resolver: ResolverGeneric::new_with_file_system(file_system.clone(), options),
            fallback: ResolverGeneric::new_with_file_system(file_system, fallback_options),
            root,
            origin,
        })
    }

    fn resolve(&self, importer: &reqwest::Url, specifier: &str) -> Result<Option<reqwest::Url>> {
        if importer.origin() != self.origin.origin() {
            bail!("HTTP dependency redirected to a different origin: {importer}");
        }
        let importer_path = self.root.join(importer.path().trim_start_matches('/'));
        let resolution = self
            .resolver
            .resolve_file(&importer_path, specifier)
            .or_else(|_| self.fallback.resolve_file(&importer_path, specifier));
        match resolution {
            Ok(resolution) => {
                let relative = resolution
                    .path()
                    .strip_prefix(&self.root)
                    .with_context(|| {
                        format!(
                            "Resolved dependency {} is outside its HTTP origin",
                            resolution.path().display()
                        )
                    })?;
                if relative
                    .components()
                    .any(|part| part.as_os_str() == "node_modules")
                {
                    return Ok(None);
                }
                let mut url = self.origin.join(&relative.to_string_lossy())?;
                url.set_query(
                    resolution
                        .query()
                        .map(|query| query.trim_start_matches('?')),
                );
                url.set_fragment(
                    resolution
                        .fragment()
                        .map(|fragment| fragment.trim_start_matches('#')),
                );
                Ok(Some(url))
            }
            Err(_error) if !(specifier.starts_with('.') || specifier.starts_with('#')) => Ok(None),
            Err(error) => Err(error)
                .with_context(|| format!("Failed to resolve {specifier:?} from {importer}")),
        }
    }
}

#[derive(Clone, Default)]
struct HttpFileSystem {
    origin: Option<reqwest::Url>,
    root: PathBuf,
    client: Option<reqwest::blocking::Client>,
    files: Arc<Mutex<BTreeMap<PathBuf, Option<Vec<u8>>>>>,
}

impl HttpFileSystem {
    fn new_for(origin: reqwest::Url, root: PathBuf) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .user_agent("vendorit/0.1")
            .build()
            .context("Failed to create HTTP resolver client")?;
        Ok(Self {
            origin: Some(origin),
            root,
            client: Some(client),
            files: Arc::new(Mutex::new(BTreeMap::new())),
        })
    }

    fn relative(&self, path: &Path) -> io::Result<PathBuf> {
        let path = path
            .strip_prefix(&self.root)
            .map_err(|_| io::ErrorKind::NotFound)?;
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                Component::Normal(part) => normalized.push(part),
                Component::ParentDir if normalized.pop() => {}
                Component::CurDir => {}
                _ => return Err(io::ErrorKind::NotFound.into()),
            }
        }
        Ok(normalized)
    }

    fn fetch(&self, path: &Path) -> io::Result<Vec<u8>> {
        if let Some(cached) = self
            .files
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .get(path)
        {
            return cached.clone().ok_or_else(|| io::ErrorKind::NotFound.into());
        }
        let origin = self.origin.as_ref().ok_or(io::ErrorKind::NotFound)?;
        let client = self.client.as_ref().ok_or(io::ErrorKind::NotFound)?;
        let url = origin
            .join(&path.to_string_lossy())
            .map_err(io::Error::other)?;
        let response = client.get(url).send().map_err(io::Error::other)?;
        let bytes = if response.status().is_success() {
            Some(response.bytes().map_err(io::Error::other)?.to_vec())
        } else {
            None
        };
        self.files
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .insert(path.to_owned(), bytes.clone());
        bytes.ok_or_else(|| io::ErrorKind::NotFound.into())
    }
}

impl FileSystem for HttpFileSystem {
    fn new() -> Self {
        Self::default()
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let path = self.relative(path)?;
        self.fetch(&path)
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        String::from_utf8(self.read(path)?)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    fn metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        let relative = self.relative(path)?;
        if relative.as_os_str().is_empty() {
            return Ok(FileMetadata::new(false, true, false));
        }
        match self.fetch(&relative) {
            Ok(_) => Ok(FileMetadata::new(true, false, false)),
            Err(error)
                if error.kind() == io::ErrorKind::NotFound && relative.extension().is_none() =>
            {
                Ok(FileMetadata::new(false, true, false))
            }
            Err(error) => Err(error),
        }
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        self.metadata(path)
    }

    fn read_link(&self, _path: &Path) -> std::result::Result<PathBuf, ResolveError> {
        Err(io::Error::from(io::ErrorKind::InvalidInput).into())
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        self.metadata(path)?;
        Ok(path.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn resolves_recursive_http_graphs() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut request = [0; 4096];
                    let read = stream.read(&mut request).await.unwrap();
                    let request = String::from_utf8_lossy(&request[..read]);
                    let path = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap_or("/");
                    let (status, body) = match path {
                        "/root.ts" => (
                            "200 OK",
                            "import './dep'; import '#internal'; import '@alias/alias';\n",
                        ),
                        "/dep.ts" => ("200 OK", "export const dep = true;\n"),
                        "/internal.ts" => ("200 OK", "export const internal = true;\n"),
                        "/alias.ts" => ("200 OK", "export const alias = true;\n"),
                        "/package.json" => {
                            ("200 OK", r##"{"imports":{"#internal":"./internal.ts"}}"##)
                        }
                        "/tsconfig.json" => (
                            "200 OK",
                            r#"{"compilerOptions":{"baseUrl":".","paths":{"@alias/*":["./*"]}}}"#,
                        ),
                        _ => ("404 Not Found", ""),
                    };
                    let response = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    stream.write_all(response.as_bytes()).await.unwrap();
                });
            }
        });

        let target = VitSourceInput::parse_target(&format!("http://{address}/root.ts")).unwrap();
        let graph = resolve_graph(target).await.unwrap();
        server.abort();

        assert_eq!(graph.len(), 4);
        assert!(graph.keys().any(|key| key.as_str().ends_with("/root.ts")));
        assert!(graph.keys().any(|key| key.as_str().ends_with("/dep.ts")));
        assert!(
            graph
                .keys()
                .any(|key| key.as_str().ends_with("/internal.ts"))
        );
        assert!(graph.keys().any(|key| key.as_str().ends_with("/alias.ts")));
    }
}
