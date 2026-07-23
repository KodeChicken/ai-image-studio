use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, bail};
use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures_util::TryStreamExt;
use object_store::{ObjectStore, ObjectStoreExt, aws::AmazonS3Builder, path::Path as ObjectPath};
use secrecy::ExposeSecret;
use tokio::io::AsyncWriteExt;

use crate::{Settings, config::StorageDriver};

#[derive(Clone, Debug)]
pub struct StoredObject {
    pub driver: &'static str,
    pub container: String,
    pub key: String,
}

#[derive(Clone, Debug)]
pub struct StoredObjectInfo {
    pub driver: &'static str,
    pub container: String,
    pub key: String,
    pub last_modified: DateTime<Utc>,
}

#[async_trait]
trait ImageStorage: Send + Sync {
    async fn put(&self, key: &str, bytes: Bytes) -> anyhow::Result<StoredObject>;
    async fn get(&self, container: &str, key: &str) -> anyhow::Result<Bytes>;
    async fn exists(&self, container: &str, key: &str) -> anyhow::Result<bool>;
    async fn delete(&self, container: &str, key: &str) -> anyhow::Result<()>;
    async fn list(&self) -> anyhow::Result<Vec<StoredObjectInfo>>;
}

pub struct StorageRegistry {
    primary: StorageDriver,
    local: Arc<LocalStorage>,
    s3: Option<Arc<S3Storage>>,
}

impl StorageRegistry {
    pub async fn from_settings(settings: &Settings) -> anyhow::Result<Self> {
        let local = Arc::new(LocalStorage::new(settings.storage_local_path.clone()).await?);
        let s3 = if settings.storage_s3_enabled {
            Some(Arc::new(S3Storage::new(settings)?))
        } else {
            None
        };
        if settings.storage_driver == StorageDriver::S3 && s3.is_none() {
            bail!("S3 is the primary storage driver but is not configured");
        }
        Ok(Self {
            primary: settings.storage_driver,
            local,
            s3,
        })
    }

    pub async fn put(&self, key: &str, bytes: Bytes) -> anyhow::Result<StoredObject> {
        match self.primary {
            StorageDriver::Local => self.local.put(key, bytes).await,
            StorageDriver::S3 => {
                self.s3
                    .as_ref()
                    .context("S3 storage is not configured")?
                    .put(key, bytes)
                    .await
            }
        }
    }

    pub async fn get(&self, driver: &str, container: &str, key: &str) -> anyhow::Result<Bytes> {
        match driver {
            "local" => self.local.get(container, key).await,
            "s3" => {
                self.s3
                    .as_ref()
                    .context("S3 storage is not configured")?
                    .get(container, key)
                    .await
            }
            _ => bail!("unsupported storage driver"),
        }
    }

    pub async fn exists(&self, driver: &str, container: &str, key: &str) -> anyhow::Result<bool> {
        match driver {
            "local" => self.local.exists(container, key).await,
            "s3" => {
                self.s3
                    .as_ref()
                    .context("S3 storage is not configured")?
                    .exists(container, key)
                    .await
            }
            _ => bail!("unsupported storage driver"),
        }
    }

    pub async fn delete(&self, driver: &str, container: &str, key: &str) -> anyhow::Result<()> {
        match driver {
            "local" => self.local.delete(container, key).await,
            "s3" => {
                self.s3
                    .as_ref()
                    .context("S3 storage is not configured")?
                    .delete(container, key)
                    .await
            }
            _ => bail!("unsupported storage driver"),
        }
    }

    pub async fn list_all(&self) -> anyhow::Result<Vec<StoredObjectInfo>> {
        let mut objects = self.local.list().await?;
        if let Some(s3) = &self.s3 {
            objects.extend(s3.list().await?);
        }
        Ok(objects)
    }

    pub fn can_scan(&self, driver: &str, container: &str) -> bool {
        match driver {
            "local" => container == "default",
            "s3" => self
                .s3
                .as_ref()
                .is_some_and(|storage| storage.bucket == container),
            _ => false,
        }
    }
}

struct LocalStorage {
    root: PathBuf,
}

impl LocalStorage {
    async fn new(root: PathBuf) -> anyhow::Result<Self> {
        tokio::fs::create_dir_all(&root)
            .await
            .with_context(|| format!("failed to create local storage {}", root.display()))?;
        Ok(Self { root })
    }

    fn resolve(&self, key: &str) -> anyhow::Result<PathBuf> {
        let relative = Path::new(key);
        if relative.is_absolute()
            || relative
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            bail!("invalid storage key");
        }
        Ok(self.root.join(relative))
    }
}

#[async_trait]
impl ImageStorage for LocalStorage {
    async fn put(&self, key: &str, bytes: Bytes) -> anyhow::Result<StoredObject> {
        let target = self.resolve(key)?;
        let parent = target.parent().context("storage key has no parent")?;
        tokio::fs::create_dir_all(parent).await?;
        let temp = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
        let result = async {
            let mut file = tokio::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp)
                .await?;
            file.write_all(&bytes).await?;
            file.sync_all().await?;
            drop(file);
            tokio::fs::rename(&temp, &target).await?;
            anyhow::Ok(())
        }
        .await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&temp).await;
        }
        result?;
        Ok(StoredObject {
            driver: "local",
            container: "default".to_owned(),
            key: key.to_owned(),
        })
    }

    async fn get(&self, _container: &str, key: &str) -> anyhow::Result<Bytes> {
        Ok(Bytes::from(tokio::fs::read(self.resolve(key)?).await?))
    }

    async fn exists(&self, _container: &str, key: &str) -> anyhow::Result<bool> {
        Ok(tokio::fs::try_exists(self.resolve(key)?).await?)
    }

    async fn delete(&self, _container: &str, key: &str) -> anyhow::Result<()> {
        let path = self.resolve(key)?;
        if tokio::fs::try_exists(&path).await? {
            tokio::fs::remove_file(path).await?;
        }
        Ok(())
    }

    async fn list(&self) -> anyhow::Result<Vec<StoredObjectInfo>> {
        let mut directories = vec![self.root.clone()];
        let mut objects = Vec::new();
        while let Some(directory) = directories.pop() {
            let mut entries = tokio::fs::read_dir(&directory).await?;
            while let Some(entry) = entries.next_entry().await? {
                let file_type = entry.file_type().await?;
                if file_type.is_symlink() {
                    continue;
                }
                let path = entry.path();
                if file_type.is_dir() {
                    directories.push(path);
                    continue;
                }
                if !file_type.is_file() {
                    continue;
                }
                let relative = path
                    .strip_prefix(&self.root)
                    .context("local storage entry escaped its root")?;
                let key = relative
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                let metadata = entry.metadata().await?;
                let last_modified = metadata
                    .modified()
                    .map(DateTime::<Utc>::from)
                    .unwrap_or_else(|_| Utc::now());
                objects.push(StoredObjectInfo {
                    driver: "local",
                    container: "default".to_owned(),
                    key,
                    last_modified,
                });
            }
        }
        Ok(objects)
    }
}

struct S3Storage {
    bucket: String,
    prefix: String,
    store: Arc<dyn ObjectStore>,
}

impl S3Storage {
    fn new(settings: &Settings) -> anyhow::Result<Self> {
        let bucket = settings
            .storage_s3_bucket
            .clone()
            .context("S3 bucket is required")?;
        let mut builder = AmazonS3Builder::new()
            .with_bucket_name(&bucket)
            .with_region(&settings.storage_s3_region)
            .with_access_key_id(
                settings
                    .storage_s3_access_key_id
                    .as_ref()
                    .context("S3 access key is required")?
                    .expose_secret(),
            )
            .with_secret_access_key(
                settings
                    .storage_s3_secret_access_key
                    .as_ref()
                    .context("S3 secret key is required")?
                    .expose_secret(),
            )
            .with_virtual_hosted_style_request(!settings.storage_s3_force_path_style);
        if let Some(endpoint) = &settings.storage_s3_endpoint {
            builder = builder
                .with_endpoint(endpoint)
                .with_allow_http(endpoint.starts_with("http://"));
        }
        let store = builder.build().context("failed to configure S3 storage")?;
        Ok(Self {
            bucket,
            prefix: settings.storage_s3_prefix.trim_matches('/').to_owned(),
            store: Arc::new(store),
        })
    }

    fn object_path(&self, key: &str) -> anyhow::Result<ObjectPath> {
        if key.starts_with('/')
            || key
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            bail!("invalid storage key");
        }
        let full = if self.prefix.is_empty() {
            key.to_owned()
        } else {
            format!("{}/{}", self.prefix, key)
        };
        ObjectPath::parse(full).context("invalid S3 object key")
    }
}

#[async_trait]
impl ImageStorage for S3Storage {
    async fn put(&self, key: &str, bytes: Bytes) -> anyhow::Result<StoredObject> {
        self.store
            .put(&self.object_path(key)?, bytes.into())
            .await?;
        Ok(StoredObject {
            driver: "s3",
            container: self.bucket.clone(),
            key: key.to_owned(),
        })
    }

    async fn get(&self, container: &str, key: &str) -> anyhow::Result<Bytes> {
        if container != self.bucket {
            bail!("asset belongs to an unconfigured S3 bucket");
        }
        Ok(self
            .store
            .get(&self.object_path(key)?)
            .await?
            .bytes()
            .await?)
    }

    async fn exists(&self, container: &str, key: &str) -> anyhow::Result<bool> {
        if container != self.bucket {
            return Ok(false);
        }
        match self.store.head(&self.object_path(key)?).await {
            Ok(_) => Ok(true),
            Err(object_store::Error::NotFound { .. }) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    async fn delete(&self, container: &str, key: &str) -> anyhow::Result<()> {
        if container != self.bucket {
            bail!("asset belongs to an unconfigured S3 bucket");
        }
        self.store.delete(&self.object_path(key)?).await?;
        Ok(())
    }

    async fn list(&self) -> anyhow::Result<Vec<StoredObjectInfo>> {
        let prefix = if self.prefix.is_empty() {
            None
        } else {
            Some(ObjectPath::parse(&self.prefix).context("invalid S3 prefix")?)
        };
        let prefix_with_separator = (!self.prefix.is_empty()).then(|| format!("{}/", self.prefix));
        let mut stream = self.store.list(prefix.as_ref());
        let mut objects = Vec::new();
        while let Some(metadata) = stream.try_next().await? {
            let full_key = metadata.location.as_ref();
            let key = match &prefix_with_separator {
                Some(prefix) => match full_key.strip_prefix(prefix) {
                    Some(key) if !key.is_empty() => key.to_owned(),
                    _ => continue,
                },
                None => full_key.to_owned(),
            };
            objects.push(StoredObjectInfo {
                driver: "s3",
                container: self.bucket.clone(),
                key,
                last_modified: metadata.last_modified,
            });
        }
        Ok(objects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_storage_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let storage = LocalStorage::new(directory.path().to_path_buf())
            .await
            .unwrap();
        let stored = storage
            .put("2026/07/test.bin", Bytes::from_static(b"image"))
            .await
            .unwrap();
        assert_eq!(stored.driver, "local");
        assert_eq!(
            storage.get("default", &stored.key).await.unwrap(),
            b"image"[..]
        );
        assert!(storage.exists("default", &stored.key).await.unwrap());
        storage.delete("default", &stored.key).await.unwrap();
        assert!(!storage.exists("default", &stored.key).await.unwrap());
    }

    #[tokio::test]
    async fn local_storage_rejects_traversal() {
        let directory = tempfile::tempdir().unwrap();
        let storage = LocalStorage::new(directory.path().to_path_buf())
            .await
            .unwrap();
        assert!(storage.put("../escape", Bytes::new()).await.is_err());
    }

    #[tokio::test]
    async fn s3_storage_contract_when_configured() {
        let Ok(endpoint) = std::env::var("TEST_S3_ENDPOINT") else {
            return;
        };
        let bucket =
            std::env::var("TEST_S3_BUCKET").unwrap_or_else(|_| "ai-image-studio-test".to_owned());
        let access_key =
            std::env::var("TEST_S3_ACCESS_KEY").expect("TEST_S3_ACCESS_KEY is required");
        let secret_key =
            std::env::var("TEST_S3_SECRET_KEY").expect("TEST_S3_SECRET_KEY is required");
        let store = AmazonS3Builder::new()
            .with_bucket_name(&bucket)
            .with_region("us-east-1")
            .with_endpoint(endpoint)
            .with_allow_http(true)
            .with_access_key_id(access_key)
            .with_secret_access_key(secret_key)
            .with_virtual_hosted_style_request(false)
            .build()
            .unwrap();
        let storage = S3Storage {
            bucket,
            prefix: "contract-tests".to_owned(),
            store: Arc::new(store),
        };
        let key = format!("{}.bin", uuid::Uuid::new_v4());
        let expected = Bytes::from_static(b"s3-contract-test");
        let stored = storage.put(&key, expected.clone()).await.unwrap();
        assert!(
            storage
                .exists(&stored.container, &stored.key)
                .await
                .unwrap()
        );
        assert_eq!(
            storage.get(&stored.container, &stored.key).await.unwrap(),
            expected
        );
        assert!(
            storage
                .list()
                .await
                .unwrap()
                .iter()
                .any(|object| object.container == stored.container && object.key == stored.key)
        );
        storage
            .delete(&stored.container, &stored.key)
            .await
            .unwrap();
        assert!(
            !storage
                .exists(&stored.container, &stored.key)
                .await
                .unwrap()
        );
    }
}
