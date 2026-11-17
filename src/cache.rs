//! Local cache: every byte fetched from a bucket is kept on disk, so opening,
//! previewing or downloading the same object again costs no egress.
//!
//! ```text
//! ~/Library/Caches/S3 Browser/
//!   objects/<id>/<file name>   full objects (read-only copies)
//!   objects/<id>/meta.json     etag, size, last access
//!   thumbs/<id>-<v>.png|jpg    grid thumbnails (~384 px)
//!   stats.json                 bytes fetched vs. bytes served from cache
//! ```
//!
//! `<id>` hashes endpoint + bucket + key; `<v>` hashes the validator (ETag +
//! size), so a changed object never shows a stale thumbnail or file.

use std::collections::HashSet;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use anyhow::{Context as _, Result, anyhow, bail};
use image::{DynamicImage, ImageDecoder as _, ImageReader};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const THUMB_PX: u32 = 384;

/// Identifies one version of one object.
#[derive(Clone, Debug)]
pub struct ObjectRef {
    /// Endpoint (or `aws:<region>`) — the same object seen through two
    /// connection profiles shares one cache entry.
    pub origin: String,
    pub bucket: String,
    pub key: String,
    pub etag: Option<String>,
    pub size: u64,
}

impl ObjectRef {
    fn id(&self) -> String {
        hex(&Sha256::digest(format!("{}\n{}\n{}", self.origin, self.bucket, self.key).as_bytes()))[..32].to_string()
    }

    fn version(&self) -> String {
        let v = format!("{}|{}", self.etag.as_deref().unwrap_or(""), self.size);
        hex(&Sha256::digest(v.as_bytes()))[..10].to_string()
    }

    pub fn file_name(&self) -> String {
        let name = self.key.rsplit('/').next().unwrap_or("object");
        let clean: String = name.chars().map(|c| if c == '/' || c == '\0' { '_' } else { c }).collect();
        if clean.is_empty() { "object".into() } else { clean }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Serialize, Deserialize)]
struct Meta {
    etag: Option<String>,
    size: u64,
    file: String,
    bucket: String,
    key: String,
    accessed: i64,
}

#[derive(Serialize, Deserialize, Default, Clone, Copy)]
pub struct Stats {
    /// Bytes we had to fetch from storage.
    pub fetched: u64,
    /// Bytes served from the local cache instead of the network.
    pub saved: u64,
}

#[derive(Clone, Copy, Default)]
pub struct Usage {
    pub objects_bytes: u64,
    pub objects: usize,
    pub thumbs_bytes: u64,
    pub thumbs: usize,
}

pub struct Cache {
    root: PathBuf,
    limit: AtomicU64,
    stats: Mutex<Stats>,
    /// Thumbnails already counted as "saved" this session.
    counted: Mutex<HashSet<String>>,
    /// Serialises commit/evict/clear so a scan never sees half-written entries.
    write_lock: Mutex<()>,
}

pub fn global() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| {
        // Tests point this elsewhere so they never touch the real cache.
        let root = std::env::var_os("S3_BROWSER_CACHE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("S3 Browser"));
        let _ = std::fs::create_dir_all(root.join("objects"));
        let _ = std::fs::create_dir_all(root.join("thumbs"));
        let stats = std::fs::read(root.join("stats.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Cache {
            root,
            limit: AtomicU64::new(5 * 1024 * 1024 * 1024),
            stats: Mutex::new(stats),
            counted: Mutex::new(HashSet::new()),
            write_lock: Mutex::new(()),
        }
    })
}

/// Write via a temp file + rename so readers never see a partial file.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("tmp{}", uuid::Uuid::new_v4().simple()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn dir_size(dir: &Path) -> (u64, usize) {
    let mut total = (0, 0);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&d) else { continue };
        for e in read.flatten() {
            let Ok(m) = e.metadata() else { continue };
            if m.is_dir() {
                stack.push(e.path());
            } else if e.file_name() != "meta.json" {
                total.0 += m.len();
                total.1 += 1;
            }
        }
    }
    total
}

impl Cache {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn set_limit(&self, bytes: u64) {
        self.limit.store(bytes, Ordering::Relaxed);
    }

    /// Largest single object worth caching.
    pub fn max_object(&self) -> u64 {
        (self.limit.load(Ordering::Relaxed) / 4).min(1024 * 1024 * 1024)
    }

    pub fn stats(&self) -> Stats {
        *self.stats.lock().unwrap()
    }

    fn bump(&self, fetched: u64, saved: u64) {
        let snapshot = {
            let mut s = self.stats.lock().unwrap();
            s.fetched += fetched;
            s.saved += saved;
            *s
        };
        let _ = std::fs::write(self.root.join("stats.json"), serde_json::to_vec(&snapshot).unwrap_or_default());
    }

    pub fn record_fetched(&self, bytes: u64) {
        self.bump(bytes, 0);
    }

    pub fn record_saved(&self, bytes: u64) {
        self.bump(0, bytes);
    }

    pub fn usage(&self) -> Usage {
        let (objects_bytes, objects) = dir_size(&self.root.join("objects"));
        let (thumbs_bytes, thumbs) = dir_size(&self.root.join("thumbs"));
        Usage { objects_bytes, objects, thumbs_bytes, thumbs }
    }

    pub fn clear(&self) -> Result<()> {
        let _guard = self.write_lock.lock().unwrap();
        for sub in ["objects", "thumbs"] {
            let dir = self.root.join(sub);
            // Cached files are read-only; make them writable so they can go.
            make_writable(&dir);
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir)?;
        }
        Ok(())
    }

    // ---- full objects -------------------------------------------------------

    fn entry_dir(&self, obj: &ObjectRef) -> PathBuf {
        self.root.join("objects").join(obj.id())
    }

    fn lookup(&self, obj: &ObjectRef) -> Option<(PathBuf, Meta)> {
        let dir = self.entry_dir(obj);
        let meta: Meta = serde_json::from_slice(&std::fs::read(dir.join("meta.json")).ok()?).ok()?;
        let fresh = meta.size == obj.size
            && match (&meta.etag, &obj.etag) {
                (Some(a), Some(b)) => a == b,
                // Provider gives no ETags at all: size must do.
                (None, None) => true,
                // ETag on one side only: can't prove it's the same version.
                _ => false,
            };
        let path = dir.join(&meta.file);
        if !fresh || std::fs::metadata(&path).map(|m| m.len()).ok() != Some(obj.size) {
            return None;
        }
        Some((path, meta))
    }

    /// The cached file for this exact version, if we have it (marks it used).
    pub fn object_path(&self, obj: &ObjectRef) -> Option<PathBuf> {
        let (path, meta) = self.lookup(obj)?;
        let touched = Meta { accessed: now(), ..meta };
        let _ = write_atomic(&self.entry_dir(obj).join("meta.json"), &serde_json::to_vec(&touched).unwrap_or_default());
        Some(path)
    }

    /// Read-only check for the UI (doesn't touch LRU order).
    pub fn is_cached(&self, obj: &ObjectRef) -> bool {
        self.lookup(obj).is_some()
    }

    /// Where to stream a download before [`commit`](Self::commit) moves it in.
    pub fn staging_path(&self, obj: &ObjectRef) -> PathBuf {
        self.root.join("objects").join(format!("{}.{}.part", obj.id(), uuid::Uuid::new_v4().simple()))
    }

    /// Move a complete file (from [`staging_path`](Self::staging_path)) into the cache.
    pub fn commit(&self, obj: &ObjectRef, staged: &Path) -> Result<PathBuf> {
        let _guard = self.write_lock.lock().unwrap();
        let dir = self.entry_dir(obj);
        make_writable(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        let file = obj.file_name();
        let path = dir.join(&file);
        std::fs::rename(staged, &path).or_else(|_| std::fs::copy(staged, &path).map(|_| ()))?;
        set_readonly(&path);
        let meta = Meta { etag: obj.etag.clone(), size: obj.size, file, bucket: obj.bucket.clone(), key: obj.key.clone(), accessed: now() };
        write_atomic(&dir.join("meta.json"), &serde_json::to_vec(&meta)?)?;
        self.evict_if_needed(&dir);
        Ok(path)
    }

    /// Copy a local file (an upload or a finished download) into the cache.
    /// On APFS `fs::copy` clones, so this is nearly free.
    pub fn store_copy(&self, obj: &ObjectRef, src: &Path) -> Result<PathBuf> {
        if obj.size > self.max_object() {
            bail!("too large to cache");
        }
        let staged = self.staging_path(obj);
        std::fs::copy(src, &staged)?;
        self.commit(obj, &staged)
    }

    pub fn store_bytes(&self, obj: &ObjectRef, bytes: &[u8]) -> Result<PathBuf> {
        let staged = self.staging_path(obj);
        std::fs::write(&staged, bytes)?;
        self.commit(obj, &staged)
    }

    /// Drop least-recently-used objects until the cache is under its limit.
    /// Called with `write_lock` held; never removes `keep` (just committed).
    fn evict_if_needed(&self, keep: &Path) {
        let limit = self.limit.load(Ordering::Relaxed);
        let objects = self.root.join("objects");
        let Ok(read) = std::fs::read_dir(&objects) else { return };
        let mut entries: Vec<(i64, u64, PathBuf)> = Vec::new();
        let mut total = 0u64;
        for e in read.flatten() {
            let dir = e.path();
            if !dir.is_dir() {
                continue;
            }
            // Unreadable entries are skipped, never deleted on a guess.
            let Some(meta) = std::fs::read(dir.join("meta.json")).ok().and_then(|b| serde_json::from_slice::<Meta>(&b).ok()) else {
                continue;
            };
            total += meta.size;
            entries.push((meta.accessed, meta.size, dir));
        }
        if total <= limit {
            return;
        }
        entries.sort_by_key(|(accessed, ..)| *accessed);
        for (_, size, dir) in entries {
            if total <= limit * 9 / 10 {
                break;
            }
            if dir == keep {
                continue;
            }
            make_writable(&dir);
            if std::fs::remove_dir_all(&dir).is_ok() {
                total = total.saturating_sub(size);
            }
        }
    }

    // ---- thumbnails ---------------------------------------------------------

    fn thumb_path(&self, obj: &ObjectRef) -> PathBuf {
        let ext = if keeps_alpha(&obj.key) { "png" } else { "jpg" };
        self.root.join("thumbs").join(format!("{}-{}.{ext}", obj.id(), obj.version()))
    }

    /// A thumbnail we already made for this exact version.
    pub fn existing_thumb(&self, obj: &ObjectRef) -> Option<PathBuf> {
        let path = self.thumb_path(obj);
        if !path.exists() {
            return None;
        }
        // The first time per session a cached thumbnail replaces a fetch,
        // count the bytes we didn't download.
        if self.counted.lock().unwrap().insert(path.display().to_string()) {
            self.record_saved(obj.size);
        }
        Some(path)
    }

    /// Make a thumbnail from a local file of the object (blocking).
    pub fn thumb_from_file(&self, obj: &ObjectRef, src: &Path) -> Result<PathBuf> {
        let dest = self.thumb_path(obj);
        let bytes = std::fs::read(src)?;
        match render_thumb(&bytes, &dest) {
            Ok(()) => Ok(dest),
            Err(e) => quicklook_thumb(src, &dest).map(|_| dest).map_err(|_| e),
        }
    }

    /// Make a thumbnail from bytes already in memory (blocking).
    pub fn thumb_from_bytes(&self, obj: &ObjectRef, bytes: &[u8], cached_file: Option<&Path>) -> Result<PathBuf> {
        let dest = self.thumb_path(obj);
        match render_thumb(bytes, &dest) {
            Ok(()) => Ok(dest),
            Err(e) => match cached_file {
                Some(src) => quicklook_thumb(src, &dest).map(|_| dest).map_err(|_| e),
                None => Err(e),
            },
        }
    }
}

/// Decode, apply EXIF orientation, downscale and write a thumbnail.
fn render_thumb(bytes: &[u8], dest: &Path) -> Result<()> {
    let mut decoder = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?.into_decoder().context("unsupported image")?;
    let orientation = decoder.orientation().ok();
    let mut img = DynamicImage::from_decoder(decoder)?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    let thumb = if img.width() > THUMB_PX || img.height() > THUMB_PX { img.thumbnail(THUMB_PX, THUMB_PX) } else { img };
    let tmp = dest.with_extension("tmp");
    if dest.extension().is_some_and(|e| e == "png") {
        thumb.save_with_format(&tmp, image::ImageFormat::Png)?;
    } else {
        let rgb = thumb.to_rgb8();
        let mut out = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 82).encode_image(&rgb)?;
    }
    std::fs::rename(tmp, dest)?;
    Ok(())
}

/// macOS Quick Look renders HEIC, PDF, PSD, RAW, videos… that `image` can't.
#[cfg(target_os = "macos")]
fn quicklook_thumb(src: &Path, dest: &Path) -> Result<()> {
    let out_dir = dest.with_extension("ql");
    let _ = std::fs::create_dir_all(&out_dir);
    let status = std::process::Command::new("/usr/bin/qlmanage")
        .args(["-t", "-s", &THUMB_PX.to_string(), "-o"])
        .arg(&out_dir)
        .arg(src)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    let produced = std::fs::read_dir(&out_dir)?.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e == "png"));
    let result = match (status.success(), produced) {
        (true, Some(png)) => {
            // Re-encode to the expected extension.
            let bytes = std::fs::read(&png)?;
            render_thumb(&bytes, dest)
        }
        _ => Err(anyhow!("no preview")),
    };
    let _ = std::fs::remove_dir_all(&out_dir);
    result
}

#[cfg(not(target_os = "macos"))]
fn quicklook_thumb(_src: &Path, _dest: &Path) -> Result<()> {
    Err(anyhow!("no preview"))
}

fn keeps_alpha(key: &str) -> bool {
    let ext = key.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    matches!(ext.as_str(), "png" | "gif" | "webp" | "svg" | "ico" | "bmp" | "tif" | "tiff" | "pdf" | "psd")
}

fn set_readonly(path: &Path) {
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_readonly(true);
        let _ = std::fs::set_permissions(path, perms);
    }
}

#[allow(clippy::permissions_set_readonly_false)]
fn make_writable(dir: &Path) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    for e in read.flatten() {
        let path = e.path();
        if path.is_dir() {
            make_writable(&path);
        } else if let Ok(meta) = std::fs::metadata(&path) {
            let mut perms = meta.permissions();
            perms.set_readonly(false);
            let _ = std::fs::set_permissions(&path, perms);
        }
    }
}

// ---- what can be previewed ---------------------------------------------------

/// Files we try to thumbnail.
pub fn is_previewable(name: &str) -> bool {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "ico" | "avif" | "heic" | "heif" | "pdf" | "psd"
    )
}

/// Files the in-app Quick Preview can show full size (GPUI decodes these).
pub fn is_viewable_image(name: &str) -> bool {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "ico" | "tif" | "tiff")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cache(root: &Path) -> Cache {
        Cache {
            root: root.to_path_buf(),
            limit: AtomicU64::new(u64::MAX),
            stats: Mutex::new(Stats::default()),
            counted: Mutex::new(HashSet::new()),
            write_lock: Mutex::new(()),
        }
    }

    fn obj(etag: &str, size: u64) -> ObjectRef {
        ObjectRef { origin: "test".into(), bucket: "b".into(), key: "dir/pic.png".into(), etag: Some(etag.into()), size }
    }

    /// HEIC (iPhone photos) goes through Quick Look on macOS.
    #[cfg(target_os = "macos")]
    #[test]
    fn heic_thumbnail_via_quicklook() {
        let Some(src) = std::fs::read_dir("/System/Library/Desktop Pictures/.thumbnails")
            .ok()
            .and_then(|d| d.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e == "heic")))
        else {
            return;
        };
        let root = std::env::temp_dir().join(format!("s3b-heic-{}", std::process::id()));
        let cache = test_cache(&root);
        std::fs::create_dir_all(root.join("thumbs")).unwrap();
        let obj = ObjectRef { origin: "t".into(), bucket: "b".into(), key: "IMG_0001.heic".into(), etag: None, size: 1 };
        let thumb = cache.thumb_from_file(&obj, &src).unwrap();
        let t = image::open(&thumb).unwrap();
        assert!(t.width().max(t.height()) <= THUMB_PX && t.width() > 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn validates_versions_and_thumbnails() {
        let root = std::env::temp_dir().join(format!("s3b-cache-{}", std::process::id()));
        let cache = test_cache(&root);
        std::fs::create_dir_all(root.join("objects")).unwrap();
        std::fs::create_dir_all(root.join("thumbs")).unwrap();

        // A 600x300 PNG.
        let img = image::RgbaImage::from_pixel(600, 300, image::Rgba([10, 120, 250, 255]));
        let mut png = Vec::new();
        DynamicImage::ImageRgba8(img).write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();

        let v1 = obj("abc", png.len() as u64);
        assert!(cache.object_path(&v1).is_none());
        let path = cache.store_bytes(&v1, &png).unwrap();
        assert!(path.ends_with("pic.png"));
        assert_eq!(cache.object_path(&v1).unwrap(), path);
        // A new ETag (object changed) is a miss, and so is a missing one.
        assert!(cache.object_path(&obj("def", png.len() as u64)).is_none());
        assert!(cache.object_path(&ObjectRef { etag: None, ..v1.clone() }).is_none());

        let thumb = cache.thumb_from_bytes(&v1, &png, None).unwrap();
        let t = image::open(&thumb).unwrap();
        assert_eq!((t.width(), t.height()), (384, 192));
        assert!(cache.existing_thumb(&v1).is_some());
        assert!(cache.existing_thumb(&obj("def", 1)).is_none());

        cache.clear().unwrap();
        assert!(cache.object_path(&v1).is_none());
        let _ = std::fs::remove_dir_all(root);
    }
}
