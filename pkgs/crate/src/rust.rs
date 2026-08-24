use crate::prelude::*;

use std::io;
use syn::visit::{self, Visit};
use syn::{Attribute, Expr, Item, ItemMod, Lit, Meta, UseTree};

pub(crate) trait RustFileSystem: Clone {
    fn read_rust_file(&self, path: &Path) -> io::Result<Vec<u8>>;

    fn is_rust_file(&self, path: &Path) -> bool {
        self.read_rust_file(path).is_ok()
    }
}

pub(crate) struct RustResolver<F> {
    file_system: F,
    modules: BTreeMap<Vec<String>, PathBuf>,
    file_modules: BTreeMap<PathBuf, Vec<String>>,
    declarations: BTreeMap<PathBuf, BTreeSet<PathBuf>>,
}

impl<F: RustFileSystem> RustResolver<F> {
    pub(crate) fn new(file_system: F, requested: &Path) -> Result<Self> {
        let manifest = find_manifest(&file_system, requested)?;
        let roots = crate_roots(&file_system, &manifest)?;
        let mut matches = Vec::new();

        for root in roots {
            let mut resolver = Self {
                file_system: file_system.clone(),
                modules: BTreeMap::new(),
                file_modules: BTreeMap::new(),
                declarations: BTreeMap::new(),
            };
            resolver.load_file(
                Vec::new(),
                root.clone(),
                root.parent().unwrap_or(Path::new("")),
            )?;
            if resolver.file_modules.contains_key(requested) {
                matches.push(resolver);
            }
        }

        ensure!(
            matches.len() == 1,
            "{} belongs to {} discovered Rust crate roots; expected exactly one",
            requested.display(),
            matches.len()
        );
        Ok(matches.pop().unwrap())
    }

    pub(crate) fn dependencies(&self, path: &Path, bytes: &[u8]) -> Result<Vec<PathBuf>> {
        let module = self.file_modules.get(path).with_context(|| {
            format!(
                "{} is not part of the discovered Rust crate",
                path.display()
            )
        })?;
        let source = std::str::from_utf8(bytes)
            .with_context(|| format!("{} is not valid UTF-8", path.display()))?;
        let parsed = syn::parse_file(source)
            .with_context(|| format!("Failed to parse Rust source {}", path.display()))?;
        let mut collector = RustDependencyCollector {
            scope: module.clone(),
            references: Vec::new(),
            includes: Vec::new(),
        };
        collector.visit_file(&parsed);

        let mut dependencies = self.declarations.get(path).cloned().unwrap_or_default();
        for (scope, reference) in collector.references {
            if let Some(dependency) = self.resolve_path(&scope, &reference)
                && dependency != path
            {
                dependencies.insert(dependency);
            }
        }
        let parent = path.parent().unwrap_or(Path::new(""));
        for include in collector.includes {
            let include = normalize_path(&parent.join(include))?;
            ensure!(
                self.file_system.is_rust_file(&include),
                "Rust include {} from {} does not exist",
                include.display(),
                path.display()
            );
            dependencies.insert(include);
        }
        Ok(dependencies.into_iter().collect())
    }

    fn load_file(&mut self, module: Vec<String>, path: PathBuf, module_dir: &Path) -> Result<()> {
        if let Some(existing) = self.file_modules.get(&path) {
            ensure!(
                existing == &module,
                "Rust source {} is mounted as multiple modules",
                path.display()
            );
            return Ok(());
        }
        let bytes = self
            .file_system
            .read_rust_file(&path)
            .with_context(|| format!("Failed to read Rust module {}", path.display()))?;
        let source = std::str::from_utf8(&bytes)
            .with_context(|| format!("{} is not valid UTF-8", path.display()))?;
        let parsed = syn::parse_file(source)
            .with_context(|| format!("Failed to parse Rust source {}", path.display()))?;
        self.modules.insert(module.clone(), path.clone());
        self.file_modules.insert(path.clone(), module.clone());
        let path_base = path.parent().unwrap_or(Path::new(""));
        self.load_items(&parsed.items, &module, &path, module_dir, path_base)
    }

    fn load_items(
        &mut self,
        items: &[Item],
        module: &[String],
        source_path: &Path,
        module_dir: &Path,
        path_base: &Path,
    ) -> Result<()> {
        for item in items {
            let Item::Mod(item_mod) = item else {
                continue;
            };
            let mut child_module = module.to_vec();
            child_module.push(item_mod.ident.to_string());
            if let Some((_, items)) = &item_mod.content {
                self.modules
                    .insert(child_module.clone(), source_path.to_owned());
                self.load_items(
                    items,
                    &child_module,
                    source_path,
                    &module_dir.join(item_mod.ident.to_string()),
                    &module_dir.join(item_mod.ident.to_string()),
                )?;
                continue;
            }

            let child = self.resolve_module_file(item_mod, module_dir, path_base)?;
            let Some(child) = child else {
                continue;
            };
            self.declarations
                .entry(source_path.to_owned())
                .or_default()
                .insert(child.clone());
            let child_dir = if child.file_name().is_some_and(|name| name == "mod.rs") {
                child.parent().unwrap_or(Path::new("")).to_owned()
            } else {
                child.with_extension("")
            };
            self.load_file(child_module, child, &child_dir)?;
        }
        Ok(())
    }

    fn resolve_module_file(
        &self,
        item_mod: &ItemMod,
        module_dir: &Path,
        path_base: &Path,
    ) -> Result<Option<PathBuf>> {
        let candidates = if let Some(path) = module_path(&item_mod.attrs)? {
            vec![normalize_path(&path_base.join(path))?]
        } else {
            let name = item_mod.ident.to_string();
            vec![
                normalize_path(&module_dir.join(format!("{name}.rs")))?,
                normalize_path(&module_dir.join(name).join("mod.rs"))?,
            ]
        };
        let existing = candidates
            .into_iter()
            .filter(|path| self.file_system.is_rust_file(path))
            .collect::<Vec<_>>();
        ensure!(
            existing.len() <= 1,
            "Rust module {} resolves to multiple files",
            item_mod.ident
        );
        if existing.is_empty() && !has_cfg(&item_mod.attrs) {
            bail!("Cannot find source file for Rust module {}", item_mod.ident);
        }
        Ok(existing.into_iter().next())
    }

    fn resolve_path(&self, scope: &[String], path: &[String]) -> Option<PathBuf> {
        let first = path.first()?;
        let mut logical;
        let mut rest = path;
        match first.as_str() {
            "crate" => {
                logical = Vec::new();
                rest = &path[1..];
            }
            "self" => {
                logical = scope.to_vec();
                rest = &path[1..];
            }
            "super" => {
                logical = scope.to_vec();
                while rest.first().is_some_and(|part| part == "super") {
                    logical.pop()?;
                    rest = &rest[1..];
                }
            }
            _ => {
                logical = scope.to_vec();
                logical.push(first.clone());
                if !self
                    .modules
                    .keys()
                    .any(|module| module.starts_with(&logical))
                {
                    logical.clear();
                    logical.push(first.clone());
                }
                rest = &path[1..];
            }
        }

        let mut resolved = if logical.is_empty() {
            None
        } else {
            self.modules.get(&logical).cloned()
        };
        for part in rest {
            logical.push(part.clone());
            if let Some(path) = self.modules.get(&logical) {
                resolved = Some(path.clone());
            }
        }
        if logical.is_empty() { None } else { resolved }
    }
}

#[derive(Deserialize)]
struct CargoManifest {
    lib: Option<CargoTarget>,
    #[serde(default)]
    bin: Vec<CargoTarget>,
}

#[derive(Deserialize)]
struct CargoTarget {
    path: Option<PathBuf>,
}

fn find_manifest<F: RustFileSystem>(file_system: &F, requested: &Path) -> Result<PathBuf> {
    let mut directory = requested.parent();
    while let Some(current) = directory {
        let candidate = current.join("Cargo.toml");
        if file_system.is_rust_file(&candidate) {
            return Ok(candidate);
        }
        directory = current.parent();
    }
    bail!(
        "Cannot find a Cargo.toml containing {}",
        requested.display()
    )
}

fn crate_roots<F: RustFileSystem>(file_system: &F, manifest: &Path) -> Result<Vec<PathBuf>> {
    let bytes = file_system
        .read_rust_file(manifest)
        .with_context(|| format!("Failed to read {}", manifest.display()))?;
    let source = std::str::from_utf8(&bytes)
        .with_context(|| format!("{} is not valid UTF-8", manifest.display()))?;
    let parsed: CargoManifest = toml::from_str(source)
        .with_context(|| format!("Failed to parse {}", manifest.display()))?;
    let directory = manifest.parent().unwrap_or(Path::new(""));
    let mut roots = BTreeSet::new();
    if let Some(path) = parsed.lib.and_then(|target| target.path) {
        roots.insert(normalize_path(&directory.join(path))?);
    } else {
        roots.insert(directory.join("src/lib.rs"));
    }
    for target in parsed.bin {
        if let Some(path) = target.path {
            roots.insert(normalize_path(&directory.join(path))?);
        }
    }
    roots.insert(directory.join("src/main.rs"));
    let roots = roots
        .into_iter()
        .filter(|path| file_system.is_rust_file(path))
        .collect::<Vec<_>>();
    ensure!(
        !roots.is_empty(),
        "{} does not define a readable Rust crate root",
        manifest.display()
    );
    Ok(roots)
}

fn module_path(attributes: &[Attribute]) -> Result<Option<PathBuf>> {
    for attribute in attributes {
        if !attribute.path().is_ident("path") {
            continue;
        }
        if let Meta::NameValue(value) = &attribute.meta
            && let Expr::Lit(expression) = &value.value
            && let Lit::Str(path) = &expression.lit
        {
            return Ok(Some(PathBuf::from(path.value())));
        }
        bail!("Rust module #[path] must be a string literal");
    }
    Ok(None)
}

fn has_cfg(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("cfg"))
}

fn normalize_path(path: &Path) -> Result<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir if normalized.pop() => {}
            _ => bail!("Rust path {} escapes its source", path.display()),
        }
    }
    Ok(normalized)
}

struct RustDependencyCollector {
    scope: Vec<String>,
    references: Vec<(Vec<String>, Vec<String>)>,
    includes: Vec<PathBuf>,
}

impl RustDependencyCollector {
    fn add_use(&mut self, prefix: Vec<String>, tree: &UseTree) {
        match tree {
            UseTree::Path(path) => {
                let mut prefix = prefix;
                prefix.push(path.ident.to_string());
                self.add_use(prefix, &path.tree);
            }
            UseTree::Name(name) => {
                let mut path = prefix;
                path.push(name.ident.to_string());
                self.references.push((self.scope.clone(), path));
            }
            UseTree::Rename(rename) => {
                let mut path = prefix;
                path.push(rename.ident.to_string());
                self.references.push((self.scope.clone(), path));
            }
            UseTree::Glob(_) => self.references.push((self.scope.clone(), prefix)),
            UseTree::Group(group) => {
                for tree in &group.items {
                    self.add_use(prefix.clone(), tree);
                }
            }
        }
    }
}

impl<'ast> Visit<'ast> for RustDependencyCollector {
    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        self.add_use(Vec::new(), &item.tree);
    }

    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        if item.content.is_some() {
            self.scope.push(item.ident.to_string());
            visit::visit_item_mod(self, item);
            self.scope.pop();
        }
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        let owned = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        if owned.len() > 1
            || matches!(
                owned.first().map(String::as_str),
                Some("crate" | "self" | "super")
            )
        {
            self.references.push((self.scope.clone(), owned));
        }
        visit::visit_path(self, path);
    }

    fn visit_macro(&mut self, item: &'ast syn::Macro) {
        let name = item
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string());
        if matches!(
            name.as_deref(),
            Some("include" | "include_str" | "include_bytes")
        ) && let Ok(path) = syn::parse2::<syn::LitStr>(item.tokens.clone())
        {
            self.includes.push(PathBuf::from(path.value()));
        }
        visit::visit_macro(self, item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Clone, Default)]
    struct MemoryFileSystem(Arc<BTreeMap<PathBuf, Vec<u8>>>);

    impl RustFileSystem for MemoryFileSystem {
        fn read_rust_file(&self, path: &Path) -> io::Result<Vec<u8>> {
            self.0
                .get(path)
                .cloned()
                .ok_or_else(|| io::ErrorKind::NotFound.into())
        }
    }

    #[test]
    fn resolves_crate_and_super_module_dependencies() {
        let files = BTreeMap::from([
            (
                PathBuf::from("pkg/Cargo.toml"),
                b"[package]\nname='fixture'\nversion='0.1.0'\n".to_vec(),
            ),
            (
                PathBuf::from("pkg/src/lib.rs"),
                b"mod drill; mod error;\n".to_vec(),
            ),
            (
                PathBuf::from("pkg/src/drill.rs"),
                b"use crate::error::{Error, Result}; mod nested { use super::*; }\n".to_vec(),
            ),
            (
                PathBuf::from("pkg/src/error.rs"),
                b"pub struct Error; pub type Result = ();\n".to_vec(),
            ),
        ]);
        let file_system = MemoryFileSystem(Arc::new(files));
        let resolver = RustResolver::new(file_system, Path::new("pkg/src/drill.rs")).unwrap();
        let source = b"use crate::error::{Error, Result}; mod nested { use super::*; }\n";
        assert_eq!(
            resolver
                .dependencies(Path::new("pkg/src/drill.rs"), source)
                .unwrap(),
            vec![PathBuf::from("pkg/src/error.rs")]
        );
    }

    #[test]
    fn resolves_module_layouts_path_attributes_and_includes() {
        let files = BTreeMap::from([
            (
                PathBuf::from("pkg/Cargo.toml"),
                b"[package]\nname='fixture'\nversion='0.1.0'\n".to_vec(),
            ),
            (PathBuf::from("pkg/src/lib.rs"), b"mod parent;\n".to_vec()),
            (
                PathBuf::from("pkg/src/parent.rs"),
                b"#[path = \"alternate.rs\"] mod child;\n".to_vec(),
            ),
            (
                PathBuf::from("pkg/src/alternate.rs"),
                b"include_str!(\"data.txt\");\n".to_vec(),
            ),
            (PathBuf::from("pkg/src/data.txt"), b"data".to_vec()),
        ]);
        let file_system = MemoryFileSystem(Arc::new(files));
        let resolver = RustResolver::new(file_system, Path::new("pkg/src/parent.rs")).unwrap();
        assert_eq!(
            resolver
                .dependencies(
                    Path::new("pkg/src/parent.rs"),
                    b"#[path = \"alternate.rs\"] mod child;\n"
                )
                .unwrap(),
            vec![PathBuf::from("pkg/src/alternate.rs")]
        );
        assert_eq!(
            resolver
                .dependencies(
                    Path::new("pkg/src/alternate.rs"),
                    b"include_str!(\"data.txt\");\n"
                )
                .unwrap(),
            vec![PathBuf::from("pkg/src/data.txt")]
        );
    }
}
