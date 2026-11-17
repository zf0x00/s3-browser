//! Assets: Lucide icons plus our own full-colour artwork (folder, file,
//! bucket) rendered with `img("art/…")`.
//!
//! gpui-kit's default bundle has ~100 Lucide icons; every other icon the app
//! uses must be listed here, or it silently renders as nothing.

use std::borrow::Cow;

use gpui_kit::assets::{Assets, icon_assets};
use gpui_kit::{AssetSource, Result, SharedString};

icon_assets!(ExtraIcons, [
    FileImage, FileVideoCamera, FileMusic, FileArchive, FileSpreadsheet, FileCode,
    Trash, Upload, Download, FolderPlus, Database, Server, Cloud, CloudUpload, CloudOff,
    Link, Pencil, House, KeyRound, Plug, Unplug, X, Cylinder, ArrowUpDown, Import,
    LogOut, FolderInput, ShieldCheck, HardDriveDownload, FileUp, FileDown, CircleStop,
    Archive, List, Files, LayoutGrid, Images, Image, ZoomIn, ZoomOut, Tag, PiggyBank,
    Gauge, ArrowDownWideNarrow, ArrowUpNarrowWide, SlidersHorizontal, ScanEye, Maximize2,
    SquareArrowOutUpRight, CalendarClock, Box, Boxes, RotateCcw, FolderSync,
]);

const ART: &[(&str, &[u8])] = &[
    ("art/folder.svg", include_bytes!("../assets/art/folder.svg")),
    ("art/file-dark.svg", include_bytes!("../assets/art/file-dark.svg")),
    ("art/file-light.svg", include_bytes!("../assets/art/file-light.svg")),
    ("art/bucket.svg", include_bytes!("../assets/art/bucket.svg")),
    ("icon/AppIcon.svg", include_bytes!("../assets/icon/AppIcon.svg")),
    ("logos/s3-bucket.svg", include_bytes!("../assets/logos/s3-bucket.svg")),
    ("logos/cloudflare.svg", include_bytes!("../assets/logos/cloudflare.svg")),
    ("logos/minio.svg", include_bytes!("../assets/logos/minio.svg")),
    ("logos/backblaze.svg", include_bytes!("../assets/logos/backblaze.svg")),
    ("logos/wasabi.svg", include_bytes!("../assets/logos/wasabi.svg")),
    ("logos/digitalocean.svg", include_bytes!("../assets/logos/digitalocean.svg")),
    ("logos/googlecloudstorage.svg", include_bytes!("../assets/logos/googlecloudstorage.svg")),
    ("logos/akamai.svg", include_bytes!("../assets/logos/akamai.svg")),
    ("logos/scaleway.svg", include_bytes!("../assets/logos/scaleway.svg")),
    ("logos/hetzner.svg", include_bytes!("../assets/logos/hetzner.svg")),
];

pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = ART.iter().find(|(p, _)| *p == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut all: Vec<SharedString> = ART.iter().filter(|(p, _)| p.starts_with(path)).map(|(p, _)| (*p).into()).collect();
        all.extend(ExtraIcons.list(path)?);
        all.extend(Assets.list(path)?);
        Ok(all)
    }
}
