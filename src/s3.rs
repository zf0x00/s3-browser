//! The S3 layer: one client per connection, every call run on a private tokio
//! runtime (the AWS SDK needs a tokio reactor; GPUI has its own executor).
//! UI code awaits [`spawn`], which hands back the result on GPUI's side.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use aws_sdk_s3::Client;
use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region, RequestChecksumCalculation, ResponseChecksumValidation};
use aws_sdk_s3::error::{DisplayErrorContext, ProvideErrorMetadata};
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{
    BucketLocationConstraint, CompletedMultipartUpload, CompletedPart, CreateBucketConfiguration, Delete, ObjectIdentifier,
};
use futures::{StreamExt as _, TryStreamExt as _};
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _, AsyncWriteExt as _};

use crate::store::{Connection, Secret};
use crate::transfers::Transfer;

// ---- runtime bridge ---------------------------------------------------------

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("s3-io")
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

/// Run `fut` on the S3 runtime and await its result from any executor.
pub async fn spawn<T: Send + 'static>(fut: impl Future<Output = Result<T>> + Send + 'static) -> Result<T> {
    runtime().spawn(fut).await.map_err(|e| anyhow!("background task failed: {e}"))?
}

/// Turn an SDK error into a short, human message ("NoSuchBucket: …").
fn friendly<E>(e: E) -> anyhow::Error
where
    E: ProvideErrorMetadata + std::error::Error + Send + Sync + 'static,
{
    match (e.code(), e.message()) {
        (Some(code), Some(msg)) => anyhow!("{code}: {msg}"),
        (Some(code), None) => anyhow!("{code}"),
        _ => anyhow!("{}", DisplayErrorContext(&e)),
    }
}

// ---- data -------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct BucketInfo {
    pub name: String,
    pub created: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct Entry {
    /// Display name: the last path segment (folders keep no trailing slash).
    pub name: String,
    /// Full key (folders end with `/`).
    pub key: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<i64>,
    pub storage_class: Option<String>,
    /// Without quotes. Used to validate the local cache.
    pub etag: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PrefixSize {
    pub bytes: u64,
    pub files: u64,
    /// False when counting stopped at the cap.
    pub complete: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Listing {
    pub entries: Vec<Entry>,
    pub next_token: Option<String>,
}

/// HEAD result (some fields are informational only).
#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub struct ObjectMeta {
    pub key: String,
    pub size: u64,
    pub modified: Option<i64>,
    pub etag: Option<String>,
    pub content_type: Option<String>,
    pub storage_class: Option<String>,
    pub cache_control: Option<String>,
    pub server_side_encryption: Option<String>,
    pub version_id: Option<String>,
    pub metadata: Vec<(String, String)>,
}

// ---- client -----------------------------------------------------------------

#[derive(Clone)]
pub struct S3 {
    client: Client,
    pub conn: Connection,
}

const PART_SIZE_MIN: u64 = 8 * 1024 * 1024;
const MULTIPART_THRESHOLD: u64 = 16 * 1024 * 1024;

impl S3 {
    pub fn new(conn: &Connection, secret: &Secret) -> Self {
        let creds = Credentials::new(
            conn.access_key.trim(),
            secret.secret_key.trim(),
            secret.session_token.clone().filter(|t| !t.is_empty()),
            None,
            "s3-browser",
        );
        let region = if conn.region.trim().is_empty() { "us-east-1".to_string() } else { conn.region.trim().to_string() };
        let mut builder = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(region))
            .credentials_provider(creds)
            .force_path_style(conn.path_style);
        let endpoint = conn.endpoint.trim();
        if !endpoint.is_empty() {
            builder = builder.endpoint_url(endpoint.trim_end_matches('/'));
        }
        if !conn.provider.is_aws() || !endpoint.is_empty() {
            builder = builder
                .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
                .response_checksum_validation(ResponseChecksumValidation::WhenRequired);
        }
        Self { client: Client::from_conf(builder.build()), conn: conn.clone() }
    }

    /// Identifies the storage for cache keys: the endpoint, or AWS + region.
    pub fn origin(&self) -> String {
        let endpoint = self.conn.endpoint.trim().trim_end_matches('/');
        if endpoint.is_empty() { format!("aws:{}", self.conn.region.trim()) } else { endpoint.to_string() }
    }

    pub fn object_ref(&self, bucket: &str, entry: &Entry) -> crate::cache::ObjectRef {
        crate::cache::ObjectRef {
            origin: self.origin(),
            bucket: bucket.to_string(),
            key: entry.key.clone(),
            etag: entry.etag.clone(),
            size: entry.size,
        }
    }

    /// The whole object in memory (only used for small files: thumbnails).
    pub async fn get_bytes(&self, bucket: &str, key: &str) -> Result<Vec<u8>> {
        let out = self.client.get_object().bucket(bucket).key(key).send().await.map_err(friendly)?;
        let data = out.body.collect().await.map_err(|e| anyhow!("download interrupted: {e}"))?;
        Ok(data.into_bytes().to_vec())
    }

    /// Buckets visible to these keys. Falls back to the configured bucket when
    /// the key isn't allowed to list buckets (common for scoped tokens).
    pub async fn list_buckets(&self) -> Result<Vec<BucketInfo>> {
        match self.client.list_buckets().send().await {
            Ok(out) => {
                let mut buckets: Vec<BucketInfo> = out
                    .buckets()
                    .iter()
                    .filter_map(|b| Some(BucketInfo { name: b.name()?.to_string(), created: b.creation_date().map(|d| d.secs()) }))
                    .collect();
                if let Some(b) = self.conn.bucket.as_ref().filter(|b| !b.is_empty()) {
                    if !buckets.iter().any(|x| &x.name == b) {
                        buckets.insert(0, BucketInfo { name: b.clone(), created: None });
                    }
                }
                Ok(buckets)
            }
            Err(e) => match self.conn.bucket.as_ref().filter(|b| !b.is_empty()) {
                Some(b) => {
                    // Prove the bucket is reachable before showing it.
                    self.client.list_objects_v2().bucket(b).max_keys(1).send().await.map_err(friendly)?;
                    Ok(vec![BucketInfo { name: b.clone(), created: None }])
                }
                None => Err(friendly(e)),
            },
        }
    }

    /// One page of a "directory": common prefixes become folders.
    pub async fn list(&self, bucket: &str, prefix: &str, token: Option<String>) -> Result<Listing> {
        let out = self
            .client
            .list_objects_v2()
            .bucket(bucket)
            .prefix(prefix)
            .delimiter("/")
            .max_keys(1000)
            .set_continuation_token(token)
            .send()
            .await
            .map_err(friendly)?;
        let mut entries = Vec::new();
        for p in out.common_prefixes() {
            let Some(key) = p.prefix() else { continue };
            let name = key[prefix.len().min(key.len())..].trim_end_matches('/').to_string();
            entries.push(Entry { name, key: key.to_string(), is_dir: true, size: 0, modified: None, storage_class: None, etag: None });
        }
        for o in out.contents() {
            let Some(key) = o.key() else { continue };
            // The zero-byte marker for the folder we're in (s3fs / console style).
            if key == prefix {
                continue;
            }
            entries.push(Entry {
                name: key[prefix.len().min(key.len())..].to_string(),
                key: key.to_string(),
                is_dir: false,
                size: o.size().unwrap_or(0).max(0) as u64,
                modified: o.last_modified().map(|d| d.secs()),
                storage_class: o.storage_class().map(|s| s.as_str().to_string()),
                etag: o.e_tag().map(|e| e.trim_matches('"').to_string()),
            });
        }
        let next_token = if out.is_truncated() == Some(true) { out.next_continuation_token().map(str::to_string) } else { None };
        Ok(Listing { entries, next_token })
    }

    /// Every object under `prefix` (recursive), as (key, size, etag).
    pub async fn list_all(&self, bucket: &str, prefix: &str) -> Result<Vec<(String, u64, Option<String>)>> {
        let mut out = Vec::new();
        let mut token = None;
        loop {
            let page = self
                .client
                .list_objects_v2()
                .bucket(bucket)
                .prefix(prefix)
                .set_continuation_token(token)
                .send()
                .await
                .map_err(friendly)?;
            for o in page.contents() {
                if let Some(k) = o.key() {
                    out.push((k.to_string(), o.size().unwrap_or(0).max(0) as u64, o.e_tag().map(|e| e.trim_matches('"').to_string())));
                }
            }
            if page.is_truncated() == Some(true) {
                token = page.next_continuation_token().map(str::to_string);
                if token.is_none() {
                    break;
                }
            } else {
                break;
            }
        }
        Ok(out)
    }

    /// Total size of everything under `prefix` (S3 has no folder sizes, so
    /// this pages through the listing: list requests, no download egress).
    /// Stops after `cap` objects when given; `complete` says whether it did.
    pub async fn prefix_size(&self, bucket: &str, prefix: &str, cap: Option<u64>) -> Result<PrefixSize> {
        let mut size = PrefixSize { bytes: 0, files: 0, complete: true };
        let mut token = None;
        loop {
            let page = self
                .client
                .list_objects_v2()
                .bucket(bucket)
                .prefix(prefix)
                .set_continuation_token(token)
                .send()
                .await
                .map_err(friendly)?;
            for o in page.contents() {
                // Folder markers (`name/`) aren't files.
                if o.key().is_some_and(|k| !k.ends_with('/')) {
                    size.bytes += o.size().unwrap_or(0).max(0) as u64;
                    size.files += 1;
                }
            }
            token = if page.is_truncated() == Some(true) { page.next_continuation_token().map(str::to_string) } else { None };
            if token.is_none() {
                break;
            }
            if cap.is_some_and(|cap| size.files >= cap) {
                size.complete = false;
                break;
            }
        }
        Ok(size)
    }

    pub async fn head(&self, bucket: &str, key: &str) -> Result<ObjectMeta> {
        let o = self.client.head_object().bucket(bucket).key(key).send().await.map_err(friendly)?;
        let mut metadata: Vec<(String, String)> =
            o.metadata().map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
        metadata.sort();
        Ok(ObjectMeta {
            key: key.to_string(),
            size: o.content_length().unwrap_or(0).max(0) as u64,
            modified: o.last_modified().map(|d| d.secs()),
            etag: o.e_tag().map(|s| s.trim_matches('"').to_string()),
            content_type: o.content_type().map(str::to_string),
            storage_class: o.storage_class().map(|s| s.as_str().to_string()),
            cache_control: o.cache_control().map(str::to_string),
            server_side_encryption: o.server_side_encryption().map(|s| s.as_str().to_string()),
            version_id: o.version_id().map(str::to_string),
            metadata,
        })
    }

    /// Folders are zero-byte objects whose key ends in `/` (as s3fs and the AWS console do).
    pub async fn create_folder(&self, bucket: &str, key: &str) -> Result<()> {
        let key = if key.ends_with('/') { key.to_string() } else { format!("{key}/") };
        self.client
            .put_object()
            .bucket(bucket)
            .key(key)
            .content_type("application/x-directory")
            .body(ByteStream::from_static(b""))
            .send()
            .await
            .map_err(friendly)?;
        Ok(())
    }

    pub async fn create_bucket(&self, name: &str) -> Result<()> {
        let mut req = self.client.create_bucket().bucket(name);
        let region = self.conn.region.trim();
        if self.conn.provider.is_aws() && !region.is_empty() && region != "us-east-1" {
            req = req.create_bucket_configuration(
                CreateBucketConfiguration::builder().location_constraint(BucketLocationConstraint::from(region)).build(),
            );
        }
        req.send().await.map_err(friendly)?;
        Ok(())
    }

    pub async fn delete_bucket(&self, name: &str) -> Result<()> {
        self.client.delete_bucket().bucket(name).send().await.map_err(friendly)?;
        Ok(())
    }

    /// Delete objects in batches of 1000 (the DeleteObjects limit).
    pub async fn delete_keys(&self, bucket: &str, keys: Vec<String>) -> Result<usize> {
        let mut deleted = 0;
        if keys.is_empty() {
            return Ok(0);
        }
        for chunk in keys.chunks(1000) {
            if chunk.len() == 1 {
                self.client.delete_object().bucket(bucket).key(&chunk[0]).send().await.map_err(friendly)?;
                deleted += 1;
                continue;
            }
            let objects: Vec<ObjectIdentifier> =
                chunk.iter().filter_map(|k| ObjectIdentifier::builder().key(k).build().ok()).collect();
            let delete = Delete::builder().set_objects(Some(objects)).quiet(true).build()?;
            let out = match self.client.delete_objects().bucket(bucket).delete(delete).send().await {
                Ok(out) => out,
                // Some providers (GCS's XML API, a few gateways) have no
                // multi-object delete: fall back to one request per key.
                Err(_) => {
                    let this = self.clone();
                    let b = bucket.to_string();
                    futures::stream::iter(chunk.to_vec())
                        .map(|k| {
                            let (this, b) = (this.clone(), b.clone());
                            async move { this.client.delete_object().bucket(b).key(k).send().await.map_err(friendly) }
                        })
                        .buffer_unordered(8)
                        .try_collect::<Vec<_>>()
                        .await?;
                    deleted += chunk.len();
                    continue;
                }
            };
            if let Some(err) = out.errors().first() {
                bail!(
                    "Couldn't delete {}: {}",
                    err.key().unwrap_or("?"),
                    err.message().or(err.code()).unwrap_or("unknown error")
                );
            }
            deleted += chunk.len();
        }
        Ok(deleted)
    }

    /// Delete files and folders (folders recursively). Returns objects removed.
    pub async fn delete_entries(&self, bucket: &str, entries: Vec<(String, bool)>) -> Result<usize> {
        let mut keys = Vec::new();
        for (key, is_dir) in entries {
            if is_dir {
                // The listing includes the `prefix/` marker object when there is one.
                keys.extend(self.list_all(bucket, &key).await?.into_iter().map(|(k, ..)| k));
            } else {
                keys.push(key);
            }
        }
        self.delete_keys(bucket, keys).await
    }

    async fn copy(&self, bucket: &str, from: &str, to_bucket: &str, to: &str) -> Result<()> {
        self.client
            .copy_object()
            .copy_source(format!("{bucket}/{}", url_encode_key(from)))
            .bucket(to_bucket)
            .key(to)
            .send()
            .await
            .map_err(friendly)?;
        Ok(())
    }

    /// Rename = server-side copy + delete (S3 has no rename; s3fs does the same).
    /// For folders every object under the prefix moves.
    pub async fn rename(&self, bucket: &str, from: &str, to: &str, is_dir: bool) -> Result<usize> {
        if from == to {
            return Ok(0);
        }
        if !is_dir {
            self.copy(bucket, from, bucket, to).await?;
            self.delete_keys(bucket, vec![from.to_string()]).await?;
            return Ok(1);
        }
        let objects = self.list_all(bucket, from).await?;
        let pairs: Vec<(String, String)> =
            objects.iter().map(|(k, ..)| (k.clone(), format!("{to}{}", &k[from.len()..]))).collect();
        let this = self.clone();
        let bucket_owned = bucket.to_string();
        futures::stream::iter(pairs.clone())
            .map(|(src, dst)| {
                let this = this.clone();
                let b = bucket_owned.clone();
                async move { this.copy(&b, &src, &b, &dst).await }
            })
            .buffer_unordered(8)
            .try_collect::<Vec<_>>()
            .await?;
        if objects.is_empty() {
            self.create_folder(bucket, to).await?;
        }
        let n = pairs.len();
        self.delete_keys(bucket, pairs.into_iter().map(|(s, _)| s).collect()).await?;
        Ok(n)
    }

    pub async fn presign_get(&self, bucket: &str, key: &str, expires: Duration) -> Result<String> {
        let cfg = PresigningConfig::expires_in(expires)?;
        let req = self.client.get_object().bucket(bucket).key(key).presigned(cfg).await.map_err(friendly)?;
        Ok(req.uri().to_string())
    }

    // ---- transfers ----------------------------------------------------------

    /// Upload a file; returns the new object's ETag.
    pub async fn upload_file(&self, bucket: &str, key: &str, path: &Path, t: Arc<Transfer>) -> Result<Option<String>> {
        let size = tokio::fs::metadata(path).await?.len();
        t.total.store(size, Ordering::Relaxed);
        let content_type = crate::util::guess_mime(key);
        if size <= MULTIPART_THRESHOLD {
            let bytes = tokio::fs::read(path).await?;
            t.check_cancel()?;
            let out = self
                .client
                .put_object()
                .bucket(bucket)
                .key(key)
                .content_type(content_type)
                .body(ByteStream::from(bytes))
                .send()
                .await
                .map_err(friendly)?;
            t.done.store(size, Ordering::Relaxed);
            return Ok(out.e_tag().map(|e| e.trim_matches('"').to_string()));
        }

        // Multipart: parts of ≥8 MiB, at most 10,000 of them, 4 in flight.
        let part_size = PART_SIZE_MIN.max(size.div_ceil(9_000));
        let parts = size.div_ceil(part_size);
        let created = self
            .client
            .create_multipart_upload()
            .bucket(bucket)
            .key(key)
            .content_type(content_type)
            .send()
            .await
            .map_err(friendly)?;
        let upload_id = created.upload_id().ok_or_else(|| anyhow!("no upload id"))?.to_string();

        let result: Result<Vec<CompletedPart>> = futures::stream::iter(0..parts)
            .map(|i| {
                let (client, t, path) = (self.client.clone(), t.clone(), path.to_path_buf());
                let (bucket, key, upload_id) = (bucket.to_string(), key.to_string(), upload_id.clone());
                async move {
                    t.check_cancel()?;
                    let offset = i * part_size;
                    let len = part_size.min(size - offset);
                    let mut file = tokio::fs::File::open(&path).await?;
                    file.seek(std::io::SeekFrom::Start(offset)).await?;
                    let mut buf = vec![0u8; len as usize];
                    file.read_exact(&mut buf).await?;
                    let out = client
                        .upload_part()
                        .bucket(bucket)
                        .key(key)
                        .upload_id(upload_id)
                        .part_number((i + 1) as i32)
                        .body(ByteStream::from(buf))
                        .send()
                        .await
                        .map_err(friendly)?;
                    t.done.fetch_add(len, Ordering::Relaxed);
                    Ok(CompletedPart::builder().set_e_tag(out.e_tag().map(str::to_string)).part_number((i + 1) as i32).build())
                }
            })
            .buffered(4)
            .try_collect()
            .await;

        match result {
            Ok(parts) => {
                let done = self
                    .client
                    .complete_multipart_upload()
                    .bucket(bucket)
                    .key(key)
                    .upload_id(&upload_id)
                    .multipart_upload(CompletedMultipartUpload::builder().set_parts(Some(parts)).build())
                    .send()
                    .await
                    .map_err(friendly)?;
                Ok(done.e_tag().map(|e| e.trim_matches('"').to_string()))
            }
            Err(e) => {
                let _ = self.client.abort_multipart_upload().bucket(bucket).key(key).upload_id(&upload_id).send().await;
                Err(e)
            }
        }
    }

    /// Stream an object to disk through a `.part` file, renamed when complete.
    pub async fn download_file(&self, bucket: &str, key: &str, dest: &Path, t: Arc<Transfer>) -> Result<()> {
        let out = self.client.get_object().bucket(bucket).key(key).send().await.map_err(friendly)?;
        t.total.store(out.content_length().unwrap_or(0).max(0) as u64, Ordering::Relaxed);
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let part = part_path(dest);
        let mut file = tokio::fs::File::create(&part).await?;
        let mut body = out.body;
        let result: Result<()> = async {
            while let Some(chunk) = body.try_next().await.map_err(|e| anyhow!("download interrupted: {e}"))? {
                t.check_cancel()?;
                file.write_all(&chunk).await?;
                t.done.fetch_add(chunk.len() as u64, Ordering::Relaxed);
            }
            file.flush().await?;
            Ok(())
        }
        .await;
        drop(file);
        match result {
            Ok(()) => {
                tokio::fs::rename(&part, dest).await?;
                Ok(())
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&part).await;
                Err(e)
            }
        }
    }
}

// ---- cache-aware transfers ----------------------------------------------------

impl S3 {
    /// Make sure the object is in the local cache and return its path.
    /// A cache hit costs nothing; a miss streams the object into the cache.
    pub async fn ensure_cached(&self, bucket: &str, entry: &Entry, t: Arc<Transfer>) -> Result<PathBuf> {
        let cache = crate::cache::global();
        let obj = self.object_ref(bucket, entry);
        t.total.store(entry.size, Ordering::Relaxed);
        if let Some(path) = cache.object_path(&obj) {
            t.done.store(entry.size, Ordering::Relaxed);
            cache.record_saved(entry.size);
            return Ok(path);
        }
        if entry.size > cache.max_object() {
            bail!("too large for the cache");
        }
        let staged = cache.staging_path(&obj);
        self.download_file(bucket, &entry.key, &staged, t).await?;
        cache.record_fetched(entry.size);
        let committed = tokio::task::spawn_blocking(move || cache.commit(&obj, &staged)).await??;
        Ok(committed)
    }

    /// Download to `dest`, served from the cache when possible; keeps a cached
    /// copy afterwards so the next open/preview/download is free.
    pub async fn download_cached(&self, bucket: &str, entry: &Entry, dest: &Path, t: Arc<Transfer>) -> Result<bool> {
        let cache = crate::cache::global();
        let obj = self.object_ref(bucket, entry);
        if let Some(path) = cache.object_path(&obj) {
            t.total.store(entry.size, Ordering::Relaxed);
            if let Some(parent) = dest.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let _ = tokio::fs::remove_file(dest).await;
            tokio::fs::copy(&path, dest).await?;
            make_user_writable(dest);
            t.done.store(entry.size, Ordering::Relaxed);
            cache.record_saved(entry.size);
            return Ok(true);
        }
        self.download_file(bucket, &entry.key, dest, t).await?;
        cache.record_fetched(entry.size);
        if entry.size <= cache.max_object() {
            let dest = dest.to_path_buf();
            let _ = tokio::task::spawn_blocking(move || {
                let stored = cache.store_copy(&obj, &dest);
                // A thumbnail too, made locally — no extra egress.
                if stored.is_ok() && crate::cache::is_previewable(&obj.key) && cache.existing_thumb(&obj).is_none() {
                    let _ = cache.thumb_from_file(&obj, &dest);
                }
            })
            .await;
        }
        Ok(false)
    }

    /// A grid thumbnail: from disk when we made one before, from the cached
    /// object if we have it, otherwise fetch the object (≤ `max_source` bytes),
    /// cache it and render.
    pub async fn thumbnail(&self, bucket: &str, entry: &Entry, max_source: u64) -> Result<PathBuf> {
        let cache = crate::cache::global();
        let obj = self.object_ref(bucket, entry);
        if let Some(path) = cache.existing_thumb(&obj) {
            return Ok(path);
        }
        if let Some(src) = cache.object_path(&obj) {
            cache.record_saved(entry.size);
            return tokio::task::spawn_blocking(move || cache.thumb_from_file(&obj, &src)).await?;
        }
        if entry.size > max_source {
            bail!("larger than the preview limit");
        }
        let bytes = self.get_bytes(bucket, &entry.key).await?;
        cache.record_fetched(bytes.len() as u64);
        tokio::task::spawn_blocking(move || {
            let stored = cache.store_bytes(&obj, &bytes).ok();
            cache.thumb_from_bytes(&obj, &bytes, stored.as_deref())
        })
        .await?
    }
}

/// Copies out of the read-only cache should be normal, editable files.
#[allow(clippy::permissions_set_readonly_false)]
fn make_user_writable(path: &Path) {
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_readonly(false);
        let _ = std::fs::set_permissions(path, perms);
    }
}

fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".part");
    dest.with_file_name(name)
}

/// Percent-encode a key for `x-amz-copy-source`, keeping `/`.
fn url_encode_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for b in key.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_copy_source() {
        assert_eq!(url_encode_key("a b/c+d.txt"), "a%20b/c%2Bd.txt");
        assert_eq!(url_encode_key("ünï"), "%C3%BCn%C3%AF");
    }
}

/// End-to-end check against a live S3-compatible endpoint. Skipped unless
/// `S3_TEST_ENDPOINT`, `S3_TEST_KEY` and `S3_TEST_SECRET` are set, e.g. with
/// `rclone serve s3 --auth-key KEY,SECRET --addr 127.0.0.1:9123 ./data`.
#[cfg(test)]
mod live {
    use super::*;
    use crate::providers::ProviderKind;

    fn client() -> Option<S3> {
        let endpoint = std::env::var("S3_TEST_ENDPOINT").ok()?;
        let conn = Connection {
            id: "test".into(),
            name: "test".into(),
            provider: ProviderKind::Minio,
            endpoint,
            region: "us-east-1".into(),
            access_key: std::env::var("S3_TEST_KEY").ok()?,
            path_style: true,
            bucket: None,
        };
        let secret = Secret { secret_key: std::env::var("S3_TEST_SECRET").ok()?, session_token: None };
        Some(S3::new(&conn, &secret))
    }

    /// Thumbnails and downloads come from the cache the second time.
    #[test]
    fn cache_saves_egress() {
        let Some(s3) = client() else { return };
        let tmp = std::env::temp_dir().join(format!("s3b-egress-{}", std::process::id()));
        // SAFETY: tests in this module run before anything reads the cache.
        unsafe { std::env::set_var("S3_BROWSER_CACHE_DIR", tmp.join("cache")) };
        std::fs::create_dir_all(&tmp).unwrap();
        runtime().block_on(async {
            let bucket = "s3b-egress-bucket";
            let _ = s3.create_bucket(bucket).await;
            // A real JPEG to thumbnail.
            let img = image::RgbImage::from_fn(800, 600, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 128]));
            let src = tmp.join("photo.jpg");
            image::DynamicImage::ImageRgb8(img).save(&src).unwrap();
            let t = Transfer::new(1, crate::transfers::Direction::Upload, "p".into(), String::new(), String::new());
            s3.upload_file(bucket, "pics/photo.jpg", &src, t).await.unwrap();

            let entry = s3.list(bucket, "pics/", None).await.unwrap().entries.into_iter().find(|e| e.name == "photo.jpg").unwrap();
            assert!(entry.etag.is_some(), "listing carries ETags");
            let cache = crate::cache::global();
            let before = cache.stats();

            // 1st thumbnail: fetches the object once.
            let thumb = s3.thumbnail(bucket, &entry, 50 << 20).await.unwrap();
            assert!(thumb.exists());
            let after_first = cache.stats();
            assert_eq!(after_first.fetched - before.fetched, entry.size, "fetched once");

            // 2nd thumbnail: from disk, nothing fetched.
            s3.thumbnail(bucket, &entry, 50 << 20).await.unwrap();
            assert_eq!(cache.stats().fetched, after_first.fetched, "no second fetch");

            // Download: served from the cache the thumbnail filled.
            let dest = tmp.join("out/photo.jpg");
            let t = Transfer::new(2, crate::transfers::Direction::Download, "p".into(), String::new(), String::new());
            let from_cache = s3.download_cached(bucket, &entry, &dest, t).await.unwrap();
            assert!(from_cache, "download hit the cache");
            assert_eq!(std::fs::read(&dest).unwrap(), std::fs::read(&src).unwrap());
            assert_eq!(cache.stats().fetched, after_first.fetched, "still no egress");
            assert!(cache.stats().saved >= entry.size);

            // A changed object (new ETag) is fetched again.
            let changed = Entry { etag: Some("different".into()), ..entry.clone() };
            assert!(cache.object_path(&s3.object_ref(bucket, &changed)).is_none());

            // Pagination: > 1000 keys page, list fully and batch-delete.
            let keys: Vec<String> = (0..1050).map(|i| format!("many/k{i:04}.txt")).collect();
            futures::stream::iter(keys.clone())
                .map(|k| {
                    let c = s3.client.clone();
                    async move { c.put_object().bucket(bucket).key(k).body(ByteStream::from_static(b"x")).send().await.map_err(friendly) }
                })
                .buffer_unordered(32)
                .try_collect::<Vec<_>>()
                .await
                .unwrap();
            let page1 = s3.list(bucket, "many/", None).await.unwrap();
            assert_eq!(page1.entries.len(), 1000);
            let page2 = s3.list(bucket, "many/", page1.next_token.clone()).await.unwrap();
            assert_eq!(page2.entries.len(), 50);
            assert!(page2.next_token.is_none());
            assert_eq!(s3.list_all(bucket, "many/").await.unwrap().len(), 1050);
            // Folder size: everything, and stopping at a cap.
            let full = s3.prefix_size(bucket, "many/", None).await.unwrap();
            assert_eq!(full, PrefixSize { bytes: 1050, files: 1050, complete: true });
            let capped = s3.prefix_size(bucket, "many/", Some(1000)).await.unwrap();
            assert_eq!((capped.files, capped.complete), (1000, false));
            assert_eq!(s3.delete_keys(bucket, keys).await.unwrap(), 1050);
            assert!(s3.list_all(bucket, "many/").await.unwrap().is_empty());

            let _ = s3.delete_keys(bucket, vec!["pics/photo.jpg".into()]).await;
            let _ = s3.delete_bucket(bucket).await;
        });
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn round_trip() {
        let Some(s3) = client() else { return };
        let dir = std::env::temp_dir().join(format!("s3b-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        runtime().block_on(async {
            let bucket = "s3b-test-bucket";
            let _ = s3.create_bucket(bucket).await;
            let buckets = s3.list_buckets().await.unwrap();
            assert!(buckets.iter().any(|b| b.name == bucket), "bucket listed");

            s3.create_folder(bucket, "empty-folder").await.unwrap();
            // small upload
            let small = dir.join("note.txt");
            std::fs::write(&small, b"hello world").unwrap();
            let t = Transfer::new(1, crate::transfers::Direction::Upload, "note".into(), String::new(), String::new());
            s3.upload_file(bucket, "docs/note.txt", &small, t.clone()).await.unwrap();
            assert_eq!(t.done.load(Ordering::Relaxed), 11);
            // 20 MiB → multipart (3 parts) when S3_TEST_MULTIPART is set; rclone's
            // fake S3 answers CreateMultipartUpload with a malformed root element.
            let big = dir.join("big.bin");
            let mib = if std::env::var("S3_TEST_MULTIPART").is_ok() { 20 } else { 12 };
            let data: Vec<u8> = (0..mib * 1024 * 1024u32).map(|i| (i % 251) as u8).collect();
            std::fs::write(&big, &data).unwrap();
            let t = Transfer::new(2, crate::transfers::Direction::Upload, "big".into(), String::new(), String::new());
            s3.upload_file(bucket, "docs/big.bin", &big, t.clone()).await.unwrap();
            assert_eq!(t.done.load(Ordering::Relaxed), data.len() as u64);

            // listing with delimiter
            let root = s3.list(bucket, "", None).await.unwrap();
            assert!(root.entries.iter().any(|e| e.is_dir && e.name == "docs"), "{:?}", root.entries);
            // Real S3 keeps the `empty-folder/` marker; filesystem-backed servers
            // (rclone) may store it as a plain `empty-folder` object.
            assert!(root.entries.iter().any(|e| e.name == "empty-folder"), "{:?}", root.entries);
            let docs = s3.list(bucket, "docs/", None).await.unwrap();
            let names: Vec<_> = docs.entries.iter().map(|e| e.name.as_str()).collect();
            assert!(names.contains(&"note.txt") && names.contains(&"big.bin"), "{names:?}");
            assert!(!names.contains(&""), "folder marker hidden");

            let meta = s3.head(bucket, "docs/big.bin").await.unwrap();
            assert_eq!(meta.size, data.len() as u64);

            // download and compare
            let out = dir.join("down/big.bin");
            let t = Transfer::new(3, crate::transfers::Direction::Download, "big".into(), String::new(), String::new());
            s3.download_file(bucket, "docs/big.bin", &out, t).await.unwrap();
            assert_eq!(std::fs::read(&out).unwrap(), data);

            // rename file and folder
            s3.rename(bucket, "docs/note.txt", "docs/renamed.txt", false).await.unwrap();
            let moved = s3.rename(bucket, "docs/", "archive/", true).await.unwrap();
            assert!(moved >= 2);
            let all: Vec<String> = s3.list_all(bucket, "").await.unwrap().into_iter().map(|(k, ..)| k).collect();
            assert!(all.iter().any(|k| k == "archive/renamed.txt"), "{all:?}");
            assert!(!all.iter().any(|k| k.starts_with("docs/")), "{all:?}");

            let url = s3.presign_get(bucket, "archive/big.bin", Duration::from_secs(60)).await.unwrap();
            assert!(url.contains("X-Amz-Signature"), "{url}");

            // recursive delete, then the bucket
            let n = s3.delete_entries(bucket, vec![("archive/".into(), true), ("empty-folder/".into(), true)]).await.unwrap();
            let _ = s3.delete_keys(bucket, vec!["empty-folder".into()]).await;
            assert!(n >= 2);
            assert!(s3.list_all(bucket, "").await.unwrap().is_empty());
            // Filesystem-backed servers keep empty directories, so this may be refused there.
            let _ = s3.delete_bucket(bucket).await;
        });
        let _ = std::fs::remove_dir_all(dir);
    }
}
