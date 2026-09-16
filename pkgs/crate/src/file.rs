use crate::prelude::*;

use tempfile::NamedTempFile;
use tokio::io::AsyncWriteExt;

#[async_trait]
pub trait VdmFileToml
where
    Self: for<'de> Deserialize<'de> + Serialize + Default + Sync,
{
    async fn read_toml(path: &Path) -> Result<Self> {
        match tokio::fs::read_to_string(path).await {
            Ok(source) => toml::from_str(&source)
                .with_context(|| format!("Failed to parse {}", path.display())),

            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),

            Err(error) => Err(error).with_context(|| format!("Failed to read {}", path.display())),
        }
    }

    async fn write_toml(&self, path: &Path) -> Result<()> {
        let mut source = toml::to_string_pretty(self).context("Failed to serialize TOML")?;
        if !source.ends_with('\n') {
            source.push('\n');
        }
        atomic_write(path, source.as_bytes()).await
    }
}

#[async_trait]
pub trait VdmFileWritable: Sync {
    fn bytes(&self) -> &[u8];

    async fn write(&self, path: &Path) -> Result<()> {
        atomic_write(path, self.bytes()).await
    }
}

async fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    tokio::fs::create_dir_all(parent)
        .await
        .with_context(|| format!("Failed to create {}", parent.display()))?;

    let temp_parent = parent.to_owned();
    let (temp, file) = tokio::task::spawn_blocking(move || {
        let temp = NamedTempFile::new_in(&temp_parent).with_context(|| {
            format!(
                "Failed to create temporary file in {}",
                temp_parent.display()
            )
        })?;
        let file = temp.reopen().context("Failed to reopen temporary file")?;
        Ok::<_, Error>((temp, file))
    })
    .await
    .context("Temporary file task failed")??;

    let mut file = tokio::fs::File::from_std(file);
    file.write_all(bytes)
        .await
        .with_context(|| format!("Failed to write {}", path.display()))?;
    file.flush()
        .await
        .with_context(|| format!("Failed to flush {}", path.display()))?;
    drop(file);

    let destination = path.to_owned();
    tokio::task::spawn_blocking(move || {
        temp.persist(&destination)
            .map_err(|error| error.error)
            .with_context(|| format!("Failed to replace {}", destination.display()))
    })
    .await
    .context("File replacement task failed")??;

    Ok(())
}
