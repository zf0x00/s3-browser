//! S3-compatible provider presets.
//!
//! Like s3fs's `-o url=... -o use_path_request_style`, every provider comes
//! down to three things: an endpoint URL, a region and an addressing style.
//! The presets below fill those in so a user only has to paste their keys.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderKind {
    Aws,
    CloudflareR2,
    Minio,
    BackblazeB2,
    Wasabi,
    DigitalOcean,
    GoogleCloud,
    Linode,
    Scaleway,
    Hetzner,
    Custom,
}

pub struct ProviderInfo {
    pub kind: ProviderKind,
    pub name: &'static str,
    /// Short label for the provider chip.
    pub short: &'static str,
    /// Endpoint template. `{region}` and `{account}` are substituted.
    /// Empty means "let the SDK resolve it" (AWS).
    pub endpoint: &'static str,
    pub default_region: &'static str,
    pub path_style: bool,
    /// Badge background (brand colour).
    pub color: u32,
    /// Monochrome logo (`assets/logos`), drawn in `glyph` on `color`.
    /// Empty: a generic server icon.
    pub logo: &'static str,
    pub glyph: u32,
    pub hint: &'static str,
    pub regions: &'static [&'static str],
}

pub const PROVIDERS: &[ProviderInfo] = &[
    ProviderInfo {
        kind: ProviderKind::Aws,
        name: "Amazon S3",
        short: "AWS",
        endpoint: "",
        default_region: "us-east-1",
        path_style: false,
        color: 0x232f3e,
        logo: "logos/s3-bucket.svg",
        glyph: 0xff9900,
        hint: "Leave the endpoint empty to use the regional AWS endpoint.",
        regions: &[
            "us-east-1", "us-east-2", "us-west-1", "us-west-2", "ca-central-1", "eu-west-1", "eu-west-2",
            "eu-west-3", "eu-central-1", "eu-north-1", "ap-south-1", "ap-southeast-1", "ap-southeast-2",
            "ap-northeast-1", "ap-northeast-2", "sa-east-1", "me-central-1", "us-gov-west-1",
        ],
    },
    ProviderInfo {
        kind: ProviderKind::CloudflareR2,
        name: "Cloudflare R2",
        short: "R2",
        endpoint: "https://{account}.r2.cloudflarestorage.com",
        default_region: "auto",
        path_style: true,
        color: 0xf38020,
        logo: "logos/cloudflare.svg",
        glyph: 0xffffff,
        hint: "Replace {account} with your Cloudflare account ID. Bucket-scoped tokens need the bucket name below.",
        // R2 signs with `auto`; wnam/enam/… are location hints, not regions.
        regions: &["auto"],
    },
    ProviderInfo {
        kind: ProviderKind::Minio,
        name: "MinIO",
        short: "MinIO",
        endpoint: "http://localhost:9000",
        default_region: "us-east-1",
        path_style: true,
        color: 0xc72e49,
        logo: "logos/minio.svg",
        glyph: 0xffffff,
        hint: "Self-hosted MinIO; path-style addressing is required.",
        regions: &["us-east-1"],
    },
    ProviderInfo {
        kind: ProviderKind::BackblazeB2,
        name: "Backblaze B2",
        short: "B2",
        endpoint: "https://s3.{region}.backblazeb2.com",
        default_region: "us-west-004",
        path_style: false,
        color: 0xe21e29,
        logo: "logos/backblaze.svg",
        glyph: 0xffffff,
        hint: "Use an application key; the region is in your bucket's S3 endpoint.",
        regions: &["us-west-000", "us-west-001", "us-west-002", "us-west-004", "us-east-005", "eu-central-003"],
    },
    ProviderInfo {
        kind: ProviderKind::Wasabi,
        name: "Wasabi",
        short: "Wasabi",
        endpoint: "https://s3.{region}.wasabisys.com",
        default_region: "us-east-1",
        path_style: false,
        color: 0x01cd3e,
        logo: "logos/wasabi.svg",
        glyph: 0xffffff,
        hint: "Pick the region your bucket lives in.",
        regions: &["us-east-1", "us-east-2", "us-central-1", "us-west-1", "eu-central-1", "eu-west-1", "ap-northeast-1", "ap-southeast-1"],
    },
    ProviderInfo {
        kind: ProviderKind::DigitalOcean,
        name: "DigitalOcean Spaces",
        short: "Spaces",
        endpoint: "https://{region}.digitaloceanspaces.com",
        default_region: "nyc3",
        path_style: false,
        color: 0x0080ff,
        logo: "logos/digitalocean.svg",
        glyph: 0xffffff,
        hint: "Spaces access keys from the DigitalOcean control panel.",
        regions: &["nyc3", "sfo2", "sfo3", "ams3", "sgp1", "fra1", "syd1", "blr1"],
    },
    ProviderInfo {
        kind: ProviderKind::GoogleCloud,
        name: "Google Cloud Storage",
        short: "GCS",
        endpoint: "https://storage.googleapis.com",
        default_region: "auto",
        path_style: true,
        color: 0x4285f4,
        logo: "logos/googlecloudstorage.svg",
        glyph: 0xffffff,
        hint: "Uses the XML interoperability API with HMAC keys.",
        regions: &["auto"],
    },
    ProviderInfo {
        kind: ProviderKind::Linode,
        name: "Akamai / Linode",
        short: "Linode",
        endpoint: "https://{region}.linodeobjects.com",
        default_region: "us-east-1",
        path_style: false,
        color: 0x0096d6,
        logo: "logos/akamai.svg",
        glyph: 0xffffff,
        hint: "Object Storage cluster region, e.g. us-east-1 or eu-central-1.",
        regions: &["us-east-1", "us-southeast-1", "us-iad-1", "us-ord-1", "eu-central-1", "ap-south-1"],
    },
    ProviderInfo {
        kind: ProviderKind::Scaleway,
        name: "Scaleway",
        short: "Scaleway",
        endpoint: "https://s3.{region}.scw.cloud",
        default_region: "fr-par",
        path_style: false,
        color: 0x4f0599,
        logo: "logos/scaleway.svg",
        glyph: 0xffffff,
        hint: "Scaleway Object Storage API keys.",
        regions: &["fr-par", "nl-ams", "pl-waw"],
    },
    ProviderInfo {
        kind: ProviderKind::Hetzner,
        name: "Hetzner",
        short: "Hetzner",
        endpoint: "https://{region}.your-objectstorage.com",
        default_region: "fsn1",
        path_style: false,
        color: 0xd50c2d,
        logo: "logos/hetzner.svg",
        glyph: 0xffffff,
        hint: "Hetzner Object Storage location, e.g. fsn1, nbg1, hel1.",
        regions: &["fsn1", "nbg1", "hel1"],
    },
    ProviderInfo {
        kind: ProviderKind::Custom,
        name: "Other S3-compatible",
        short: "Custom",
        endpoint: "https://",
        default_region: "us-east-1",
        path_style: true,
        color: 0x475569,
        logo: "",
        glyph: 0xffffff,
        hint: "Ceph RGW, SeaweedFS, Garage, IDrive e2, Oracle… any endpoint that speaks S3.",
        regions: &["us-east-1"],
    },
];

impl ProviderKind {
    pub fn info(self) -> &'static ProviderInfo {
        PROVIDERS.iter().find(|p| p.kind == self).unwrap_or(&PROVIDERS[PROVIDERS.len() - 1])
    }

    /// AWS validates request checksums natively; many S3-compatible stores
    /// reject the newer CRC headers, so we only send them when required.
    pub fn is_aws(self) -> bool {
        self == ProviderKind::Aws
    }
}

impl ProviderInfo {
    /// The endpoint for a region, keeping `{account}` for the user to fill in.
    pub fn endpoint_for(&self, region: &str) -> String {
        self.endpoint.replace("{region}", region)
    }
}
