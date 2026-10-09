//! S3-backed storage for post images.
//!
//! Object storage stays entirely optional: [`S3Storage::from_config`] returns
//! `Ok(None)` when no `S3_*` variable is set and a `503` error when the
//! configuration is incomplete or malformed. App startup uses
//! [`S3Storage::setup`], so a missing or broken configuration never prevents
//! the rest of the site from serving; upload routes answer
//! [`S3Storage::unavailable`] instead.
//!
//! Uploads are validated before anything is written to the bucket: only
//! JPEG, PNG and WebP payloads that actually decode are accepted, capped at
//! 10 MiB, 8192 pixels per side and 24 megapixels. SVG and every other
//! format are rejected. The original bytes are stored untouched; decoded
//! pixels are rotated or flipped according to their EXIF orientation before
//! thumbnailing, so the reported dimensions and the lossless WebP thumbnail
//! (bounded to 480 pixels and never upscaled) use the displayed axes.
//! Object keys are derived exclusively from the server-generated image id.
//!
//! Every bucket PUT and DELETE, including compensating deletes, runs under
//! the bounded application deadline `S3_OPERATION_TIMEOUT`.

use std::{future::Future, io::Cursor, time::Duration};

use axum::http::StatusCode;
use bytes::Bytes;
use image::codecs::webp::WebPEncoder;
use image::metadata::Orientation;
use image::{ExtendedColorType, ImageDecoder, ImageFormat, ImageReader, Limits};
use url::Url;

use ::s3::creds::Credentials;
use ::s3::error::S3Error;
use ::s3::{Bucket, Region};

use crate::config::Config;
use crate::error::{ApiError, ApiResult};

/// Hard cap for one uploaded file (10 MiB).
pub const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024;
/// Hard cap for either side of the decoded image.
pub const MAX_IMAGE_DIMENSION: u32 = 8192;
/// Hard cap for the decoded pixel count (24 megapixels).
pub const MAX_IMAGE_PIXELS: u64 = 24_000_000;
/// Longest side of the generated thumbnail.
pub const THUMBNAIL_MAX_SIDE: u32 = 480;
/// Lifetime of the signed GET URLs handed to browsers.
pub const PRESIGN_EXPIRES_SECONDS: u32 = 600;

/// Upper bound for decoder allocations; a 24 MP RGBA frame is 96 MiB.
const DECODER_ALLOC_LIMIT: u64 = 128 * 1024 * 1024;
/// Every stored object lives below this prefix (trailing slash included).
const KEY_PREFIX: &str = "post-images/";
/// Content type of every generated thumbnail.
const THUMBNAIL_CONTENT_TYPE: &str = "image/webp";
/// End-to-end application deadline for every S3 PUT and DELETE, including
/// compensating deletes. `rust-s3`'s own timeout bounds only connection
/// establishment, so this must stay far below the 30-minute upload grace.
const S3_OPERATION_TIMEOUT: Duration = Duration::from_secs(60);

/// Object keys and dimensions of one stored image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredImage {
    pub original_key: String,
    pub thumbnail_key: String,
    pub width: u32,
    pub height: u32,
}

/// Handle for the configured S3 bucket. Cheap to clone and safe to share;
/// the only synchronization lives inside the S3 client itself.
#[derive(Debug, Clone)]
pub struct S3Storage {
    bucket: Bucket,
}

/// Directly configured S3 coordinates resolved from the environment.
struct Settings {
    endpoint: String,
    region: String,
    bucket: String,
    access_key_id: String,
    secret_access_key: String,
}

/// Validated upload ready to be written to the bucket.
#[derive(Debug)]
struct PreparedImage {
    original: Bytes,
    content_type: &'static str,
    width: u32,
    height: u32,
    thumbnail: Vec<u8>,
}

impl S3Storage {
    /// Reads the optional `S3_ENDPOINT`, `S3_REGION`, `S3_BUCKET`,
    /// `S3_ACCESS_KEY_ID` and `S3_SECRET_ACCESS_KEY` variables from `config`.
    ///
    /// Returns `Ok(None)` when none of them is set, `Ok(Some(_))` when the
    /// full set is present and valid, and a `503` error describing the
    /// problem when the set is incomplete or malformed. Startup must not
    /// propagate that error: use [`S3Storage::setup`] for application state.
    pub fn from_config(config: &Config) -> ApiResult<Option<Self>> {
        let Some(settings) = read_settings(config)? else {
            return Ok(None);
        };
        Self::build(settings).map(Some)
    }

    /// Infallible companion of [`S3Storage::from_config`] for app startup:
    /// logs the reason and reports `None` instead of failing, so image
    /// endpoints can answer `503` while the rest of the backend keeps
    /// working.
    pub fn setup(config: &Config) -> Option<Self> {
        match Self::from_config(config) {
            Ok(storage) => storage,
            Err(error) => {
                eprintln!("Image storage is unavailable: {}", error.message);
                None
            }
        }
    }

    /// `503` for upload routes whenever [`S3Storage::setup`] returned `None`.
    pub fn unavailable() -> ApiError {
        setup_error("Загрузка изображений недоступна: хранилище S3 не настроено.")
    }

    /// Validates `bytes`, generates the thumbnail and stores both objects.
    ///
    /// `id` must be the server-generated id of the pending image row; keys
    /// are derived from it alone, never from client input. The original is
    /// written first; when the thumbnail cannot be written the original is
    /// removed again so no half-stored image survives a failed upload. The
    /// returned keys must be persisted by the caller.
    pub async fn put_image(&self, id: &str, bytes: Bytes) -> ApiResult<StoredImage> {
        let (original_key, thumbnail_key) = image_keys(id)?;
        let prepared = tokio::task::spawn_blocking(move || prepare_image(bytes))
            .await
            .map_err(|error| ApiError::internal(format!("Image worker failed: {error}")))??;

        storage_request(
            "upload original",
            self.bucket.put_object_with_content_type(
                &original_key,
                prepared.original.as_ref(),
                prepared.content_type,
            ),
            S3_OPERATION_TIMEOUT,
        )
        .await?;

        if let Err(error) = storage_request(
            "upload thumbnail",
            self.bucket.put_object_with_content_type(
                &thumbnail_key,
                &prepared.thumbnail,
                THUMBNAIL_CONTENT_TYPE,
            ),
            S3_OPERATION_TIMEOUT,
        )
        .await
        {
            // Do not leave half of an image pair behind when the pair cannot
            // be completed. The compensating request has the same bounded
            // deadline as every other object operation.
            let _ = storage_request(
                "delete original after thumbnail upload failure",
                self.bucket.delete_object(&original_key),
                S3_OPERATION_TIMEOUT,
            )
            .await;
            return Err(error);
        }

        Ok(StoredImage {
            original_key,
            thumbnail_key,
            width: prepared.width,
            height: prepared.height,
        })
    }

    /// Best-effort removal of both stored objects.
    ///
    /// Both keys are attempted even when the first delete fails, and a
    /// missing object is not an error (S3 answers `204`), so callers may
    /// retry with the same keys. Empty keys are skipped.
    pub async fn delete_image(&self, original_key: &str, thumbnail_key: &str) -> ApiResult<()> {
        let mut failure: Option<ApiError> = None;
        for key in [original_key, thumbnail_key] {
            if key.is_empty() {
                continue;
            }
            validate_object_key(key)?;
            if let Err(error) = storage_request(
                "delete",
                self.bucket.delete_object(key),
                S3_OPERATION_TIMEOUT,
            )
            .await
            {
                if failure.is_none() {
                    failure = Some(error);
                }
            }
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Short-lived signed GET URL for a stored object key.
    ///
    /// Signing happens locally, so there is no network request to bound.
    pub async fn presign_get(&self, key: &str) -> ApiResult<String> {
        validate_object_key(key)?;
        self.bucket
            .presign_get(key, PRESIGN_EXPIRES_SECONDS, None)
            .await
            .map_err(|error| storage_error("presign", &error))
    }

    fn build(settings: Settings) -> ApiResult<Self> {
        let region = Region::Custom {
            region: settings.region,
            endpoint: settings.endpoint,
        };
        let credentials = Credentials {
            access_key: Some(settings.access_key_id),
            secret_key: Some(settings.secret_access_key),
            security_token: None,
            session_token: None,
            expiration: None,
        };
        let bucket = Bucket::new(&settings.bucket, region, credentials).map_err(|error| {
            setup_error(format!(
                "Хранилище изображений недоступно: не удалось создать S3-клиент ({error})."
            ))
        })?;
        Ok(Self { bucket: *bucket })
    }
}

/// Resolves and validates the optional `S3_*` configuration.
///
/// `Ok(None)` means object storage is intentionally disabled; a `503` error
/// names the missing or malformed variables.
fn read_settings(config: &Config) -> ApiResult<Option<Settings>> {
    let endpoint = config.s3_endpoint.as_deref().map(str::trim).unwrap_or("");
    let region = config.s3_region.as_deref().map(str::trim).unwrap_or("");
    let bucket = config.s3_bucket.as_deref().map(str::trim).unwrap_or("");
    let access_key_id = config
        .s3_access_key_id
        .as_deref()
        .map(str::trim)
        .unwrap_or("");
    let secret_access_key = config
        .s3_secret_access_key
        .as_deref()
        .map(str::trim)
        .unwrap_or("");

    let missing: Vec<&str> = [
        ("S3_ENDPOINT", endpoint),
        ("S3_REGION", region),
        ("S3_BUCKET", bucket),
        ("S3_ACCESS_KEY_ID", access_key_id),
        ("S3_SECRET_ACCESS_KEY", secret_access_key),
    ]
    .into_iter()
    .filter(|(_, value)| value.is_empty())
    .map(|(name, _)| name)
    .collect();

    if missing.len() == 5 {
        return Ok(None);
    }
    if !missing.is_empty() {
        return Err(setup_error(format!(
            "Хранилище изображений настроено неверно: не заданы {}.",
            missing.join(", ")
        )));
    }

    let endpoint = endpoint.trim_end_matches('/');
    let parsed = Url::parse(endpoint).map_err(|_| {
        setup_error(
            "Хранилище изображений настроено неверно: S3_ENDPOINT должен быть абсолютным URL вида https://host[:port].",
        )
    })?;
    // The endpoint must carry no path, query or fragment: virtual-hosted
    // requests prepend the separately configured bucket to this host.
    let path_is_root = parsed.path().is_empty() || parsed.path() == "/";
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none_or(str::is_empty)
        || !path_is_root
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(setup_error(
            "Хранилище изображений настроено неверно: S3_ENDPOINT должен содержать только схему, хост и порт, например https://s3.ru-6.storage.selcloud.ru.",
        ));
    }
    if !bucket
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
    {
        return Err(setup_error(
            "Хранилище изображений настроено неверно: S3_BUCKET содержит недопустимые символы.",
        ));
    }
    if region.chars().any(char::is_whitespace) {
        return Err(setup_error(
            "Хранилище изображений настроено неверно: S3_REGION содержит пробелы.",
        ));
    }
    if access_key_id.chars().any(char::is_control)
        || secret_access_key.chars().any(char::is_control)
    {
        return Err(setup_error(
            "Хранилище изображений настроено неверно: ключи доступа содержат управляющие символы.",
        ));
    }

    Ok(Some(Settings {
        endpoint: endpoint.to_string(),
        region: region.to_string(),
        bucket: bucket.to_string(),
        access_key_id: access_key_id.to_string(),
        secret_access_key: secret_access_key.to_string(),
    }))
}

/// Generated object keys for one image id.
///
/// Public so the store layer can reuse it instead of duplicating the layout;
/// the format is a contract with already-stored objects and must not change
/// without a migration.
pub fn image_keys(id: &str) -> ApiResult<(String, String)> {
    if !is_valid_image_id(id) {
        return Err(ApiError::bad_request(
            "Некорректный идентификатор изображения.",
        ));
    }
    Ok((
        format!("{KEY_PREFIX}{id}/original"),
        format!("{KEY_PREFIX}{id}/thumbnail"),
    ))
}

/// Only server-generated ids may become keys: a short ASCII token that can
/// never escape the `post-images/` prefix.
fn is_valid_image_id(id: &str) -> bool {
    let mut chars = id.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_alphanumeric())
        && id.len() <= 64
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

/// Guards presign/delete calls against keys the server never issued.
fn validate_object_key(key: &str) -> ApiResult<()> {
    let valid = key.starts_with(KEY_PREFIX)
        && key.len() <= 512
        && !key.contains("..")
        && !key.chars().any(char::is_control);
    if valid {
        Ok(())
    } else {
        Err(ApiError::internal(format!(
            "Refusing to use an unexpected image object key: {key}"
        )))
    }
}

/// Validates the uploaded bytes and renders the thumbnail.
///
/// Runs on the blocking pool from [`S3Storage::put_image`] because decoding a
/// 24 MP image is CPU work; it never performs network calls.
fn prepare_image(bytes: Bytes) -> ApiResult<PreparedImage> {
    if bytes.is_empty() {
        return Err(ApiError::bad_request("Файл изображения пуст."));
    }
    if bytes.len() > MAX_UPLOAD_BYTES {
        return Err(ApiError::bad_request(format!(
            "Изображение больше {} МБ.",
            MAX_UPLOAD_BYTES / (1024 * 1024)
        )));
    }

    let content_type = match image::guess_format(&bytes) {
        Ok(ImageFormat::Jpeg) => "image/jpeg",
        Ok(ImageFormat::Png) => "image/png",
        Ok(ImageFormat::WebP) => "image/webp",
        _ => {
            return Err(ApiError::bad_request(
                "Поддерживаются только изображения JPEG, PNG и WebP.",
            ))
        }
    };

    // Read the header and its orientation metadata before allocating a full
    // frame. The stored-axis dimensions are sufficient for resource limits:
    // orientation can only swap axes and never changes the pixel count.
    // JPEG orientation comes from the first Exif segment (see
    // `jpeg_exif_orientation`); other formats use the decoder's metadata.
    let mut header_decoder = ImageReader::new(Cursor::new(bytes.clone()))
        .with_guessed_format()
        .map_err(|_| unreadable_image())?
        .into_decoder()
        .map_err(|_| unreadable_image())?;
    let (stored_width, stored_height) = header_decoder.dimensions();
    let orientation = match jpeg_exif_orientation(&bytes) {
        Some(orientation) => orientation,
        None => header_decoder
            .orientation()
            .map_err(|_| unreadable_image())?,
    };
    if !dimensions_within_limits(stored_width, stored_height) {
        if stored_width > MAX_IMAGE_DIMENSION || stored_height > MAX_IMAGE_DIMENSION {
            return Err(ApiError::bad_request(format!(
                "Сторона изображения превышает {MAX_IMAGE_DIMENSION} пикселей."
            )));
        }
        return Err(ApiError::bad_request(format!(
            "Изображение содержит больше {} мегапикселей.",
            MAX_IMAGE_PIXELS / 1_000_000
        )));
    }

    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(DECODER_ALLOC_LIMIT);
    let mut reader = ImageReader::new(Cursor::new(bytes.clone()))
        .with_guessed_format()
        .map_err(|_| unreadable_image())?;
    reader.limits(limits);
    let mut decoded = reader.decode().map_err(|_| unreadable_image())?;
    decoded.apply_orientation(orientation);
    let (width, height) = (decoded.width(), decoded.height());

    let (thumbnail_width, thumbnail_height) = thumbnail_dimensions(width, height);
    let thumbnail = decoded
        .thumbnail(thumbnail_width, thumbnail_height)
        .to_rgba8();
    let mut thumbnail_bytes = Vec::new();
    WebPEncoder::new_lossless(&mut thumbnail_bytes)
        .encode(
            thumbnail.as_raw(),
            thumbnail.width(),
            thumbnail.height(),
            ExtendedColorType::Rgba8,
        )
        .map_err(|error| ApiError::internal(format!("WebP thumbnail encoding failed: {error}")))?;

    Ok(PreparedImage {
        original: bytes,
        content_type,
        width,
        height,
        thumbnail: thumbnail_bytes,
    })
}

/// Orientation of the first Exif APP1 segment of a JPEG, matching browsers.
///
/// `image` keeps only the *last* Exif APP1 segment: zune-jpeg overwrites its
/// Exif block for every such marker it parses. A JPEG whose camera Exif
/// block is followed by another Exif block without an orientation tag — macOS
/// `sips` appends exactly such a block when it re-encodes — would therefore
/// lose the camera orientation. Scanning the first Exif segment ourselves
/// reproduces what browsers apply to the untouched original. `None` means the
/// payload is not a JPEG or has no Exif segment, leaving the decoder's own
/// metadata as the source.
fn jpeg_exif_orientation(bytes: &[u8]) -> Option<Orientation> {
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut offset = 2;
    while offset + 4 <= bytes.len() {
        if bytes[offset] != 0xFF {
            return None;
        }
        let marker = bytes[offset + 1];
        if marker == 0xFF {
            // Fill byte between segments.
            offset += 1;
            continue;
        }
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            offset += 2;
            continue;
        }
        if marker == 0xD9 || marker == 0xDA {
            return None;
        }
        let length = usize::from(u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]));
        if length < 2 || offset + 2 + length > bytes.len() {
            return None;
        }
        if marker == 0xE1 {
            let payload = &bytes[offset + 4..offset + 2 + length];
            if let Some(exif) = payload.strip_prefix(b"Exif\0\0") {
                return Some(
                    Orientation::from_exif_chunk(exif).unwrap_or(Orientation::NoTransforms),
                );
            }
        }
        offset += 2 + length;
    }
    None
}

/// Strict upper bounds: both sides at most 8192 and at most 24 MP total.
fn dimensions_within_limits(width: u32, height: u32) -> bool {
    width > 0
        && height > 0
        && width <= MAX_IMAGE_DIMENSION
        && height <= MAX_IMAGE_DIMENSION
        && u64::from(width) * u64::from(height) <= MAX_IMAGE_PIXELS
}

/// Thumbnail box: scaled so the longer side is at most `THUMBNAIL_MAX_SIDE`,
/// preserving the aspect ratio and never enlarging a smaller image.
fn thumbnail_dimensions(width: u32, height: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= THUMBNAIL_MAX_SIDE {
        return (width.max(1), height.max(1));
    }
    if width >= height {
        (THUMBNAIL_MAX_SIDE, scaled_side(height, width))
    } else {
        (scaled_side(width, height), THUMBNAIL_MAX_SIDE)
    }
}

/// `side * THUMBNAIL_MAX_SIDE / longest`, rounded to the nearest pixel and
/// kept at one pixel minimum.
fn scaled_side(side: u32, longest: u32) -> u32 {
    let scaled = (u64::from(side) * u64::from(THUMBNAIL_MAX_SIDE) + u64::from(longest) / 2)
        / u64::from(longest);
    scaled.max(1) as u32
}

fn unreadable_image() -> ApiError {
    ApiError::bad_request("Не удалось прочитать изображение. Поддерживаются JPEG, PNG и WebP.")
}

/// Runs one S3 object request under an end-to-end application deadline.
///
/// `rust-s3` applies its configured timeout only to connection establishment,
/// so an endpoint that accepts the connection and then stalls could otherwise
/// pin a handler and its provisional upload row forever. Every network PUT
/// and DELETE, including compensating deletes, must go through here.
async fn storage_request<T, F>(context: &str, request: F, deadline: Duration) -> ApiResult<T>
where
    F: Future<Output = Result<T, S3Error>>,
{
    match tokio::time::timeout(deadline, request).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(storage_error(context, &error)),
        Err(_) => {
            eprintln!("S3 {context} timed out after {deadline:?}.");
            Err(ApiError::new(
                StatusCode::BAD_GATEWAY,
                "Хранилище изображений недоступно.",
            ))
        }
    }
}

/// Maps S3 and transport failures to a gateway error without leaking details
/// to the client; the full cause goes to the server log.
fn storage_error(context: &str, error: &S3Error) -> ApiError {
    eprintln!("S3 {context} failed: {error}");
    ApiError::new(StatusCode::BAD_GATEWAY, "Хранилище изображений недоступно.")
}

fn setup_error(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::SERVICE_UNAVAILABLE, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageEncoder};

    fn test_config(
        s3_endpoint: Option<&str>,
        s3_region: Option<&str>,
        s3_bucket: Option<&str>,
        s3_access_key_id: Option<&str>,
        s3_secret_access_key: Option<&str>,
    ) -> Config {
        Config {
            database_path: "data/test.sqlite".into(),
            data_dir: "data".into(),
            import_dir: "data".into(),
            public_data_dir: "public/data".into(),
            bind_addr: "127.0.0.1:8787".to_string(),
            frontend_origin: None,
            admin_username: None,
            admin_password: None,
            admin_session_secret: None,
            s3_endpoint: s3_endpoint.map(str::to_string),
            s3_region: s3_region.map(str::to_string),
            s3_bucket: s3_bucket.map(str::to_string),
            s3_access_key_id: s3_access_key_id.map(str::to_string),
            s3_secret_access_key: s3_secret_access_key.map(str::to_string),
            fresh_install: true,
        }
    }

    fn encoded_sample(format: ImageFormat, width: u32, height: u32) -> Vec<u8> {
        let rgba = DynamicImage::new_rgba8(width, height).to_rgba8();
        match format {
            ImageFormat::Png => write_encoded(DynamicImage::ImageRgba8(rgba), format),
            ImageFormat::Jpeg => {
                let rgb = DynamicImage::ImageRgba8(rgba).to_rgb8();
                write_encoded(DynamicImage::ImageRgb8(rgb), format)
            }
            ImageFormat::WebP => {
                let mut out = Vec::new();
                WebPEncoder::new_lossless(&mut out)
                    .encode(rgba.as_raw(), width, height, ExtendedColorType::Rgba8)
                    .unwrap();
                out
            }
            other => panic!("unsupported sample format {other:?}"),
        }
    }

    fn write_encoded(image: DynamicImage, format: ImageFormat) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        image.write_to(&mut out, format).unwrap();
        out.into_inner()
    }

    fn prepared(format: ImageFormat, width: u32, height: u32) -> PreparedImage {
        prepare_image(Bytes::from(encoded_sample(format, width, height))).unwrap()
    }

    fn rejected(bytes: Vec<u8>) -> ApiError {
        prepare_image(Bytes::from(bytes)).unwrap_err()
    }

    fn decoded_dimensions(bytes: &[u8]) -> (u32, u32) {
        let image = ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap();
        (image.width(), image.height())
    }

    /// Minimal little-endian Exif TIFF block carrying only the orientation tag.
    fn orientation_exif_tiff(orientation: u8) -> Vec<u8> {
        let mut exif = Vec::new();
        exif.extend_from_slice(b"II*\0"); // little-endian TIFF magic
        exif.extend_from_slice(&8u32.to_le_bytes()); // offset of IFD0
        exif.extend_from_slice(&1u16.to_le_bytes()); // one IFD entry
        exif.extend_from_slice(&0x0112u16.to_le_bytes()); // Orientation tag
        exif.extend_from_slice(&3u16.to_le_bytes()); // type SHORT
        exif.extend_from_slice(&1u32.to_le_bytes()); // count
        exif.extend_from_slice(&u16::from(orientation).to_le_bytes()); // value
        exif.extend_from_slice(&0u16.to_le_bytes()); // value padding
        exif.extend_from_slice(&0u32.to_le_bytes()); // no next IFD
        exif
    }

    /// JPEG whose quadrants are solid red (top-left), green (top-right), blue
    /// (bottom-left) and yellow (bottom-right), tagged with `exif_orientation`,
    /// so a missing or reversed rotation shows in both axes and pixel order.
    fn oriented_quadrant_jpeg(width: u32, height: u32, exif_orientation: u8) -> Vec<u8> {
        let mut pixels = image::RgbImage::new(width, height);
        let (mid_x, mid_y) = (width / 2, height / 2);
        for (x, y, pixel) in pixels.enumerate_pixels_mut() {
            *pixel = match (x < mid_x, y < mid_y) {
                (true, true) => image::Rgb([255, 0, 0]),
                (false, true) => image::Rgb([0, 255, 0]),
                (true, false) => image::Rgb([0, 0, 255]),
                (false, false) => image::Rgb([255, 255, 0]),
            };
        }
        let mut out = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new(&mut out);
        encoder
            .set_exif_metadata(orientation_exif_tiff(exif_orientation))
            .unwrap();
        encoder
            .encode(pixels.as_raw(), width, height, ExtendedColorType::Rgb8)
            .unwrap();
        out
    }

    fn decoded_rgb(bytes: &[u8]) -> image::RgbImage {
        ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap()
            .to_rgb8()
    }

    /// Checks the four solid quadrants of a thumbnail (top-left, top-right,
    /// bottom-left, bottom-right), sampling at each quadrant's center so
    /// JPEG artifacts near the seams cannot decide the result.
    fn assert_quadrant_colors(
        thumbnail: &image::RgbImage,
        expected: [[u8; 3]; 4],
        context: &str,
    ) {
        let (width, height) = thumbnail.dimensions();
        let points: [(u32, u32); 4] = [
            (width / 4, height / 4),
            (3 * width / 4, height / 4),
            (width / 4, 3 * height / 4),
            (3 * width / 4, 3 * height / 4),
        ];
        for ((x, y), expected) in points.into_iter().zip(expected) {
            let actual = thumbnail.get_pixel(x, y).0;
            for (channel, (a, e)) in actual.iter().zip(expected).enumerate() {
                assert!(
                    i32::from(*a).abs_diff(i32::from(e)) <= 60,
                    "{context}: channel {channel} at ({x}, {y}) is {a}, expected {e}"
                );
            }
        }
    }

    #[test]
    fn absent_configuration_disables_storage() {
        let config = test_config(None, None, None, None, None);
        assert!(read_settings(&config).unwrap().is_none());
    }

    #[test]
    fn partial_configuration_reports_missing_variables() {
        let config = test_config(
            Some("http://127.0.0.1:9000"),
            Some("ru-1"),
            Some("media"),
            None,
            None,
        );
        let error = read_settings(&config).err().unwrap();
        assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(error.message.contains("S3_ACCESS_KEY_ID"));
        assert!(error.message.contains("S3_SECRET_ACCESS_KEY"));
    }

    #[test]
    fn complete_configuration_is_accepted_and_normalized() {
        let config = test_config(
            Some("https://s3.ru-6.storage.selcloud.ru/"),
            Some("ru-6"),
            Some("sparrow"),
            Some("access"),
            Some("secret"),
        );
        let settings = read_settings(&config).unwrap().unwrap();
        assert_eq!(
            settings.endpoint,
            "https://s3.ru-6.storage.selcloud.ru"
        );
        assert_eq!(settings.region, "ru-6");
        assert_eq!(settings.bucket, "sparrow");
    }

    fn test_settings() -> Settings {
        Settings {
            endpoint: "https://s3.ru-6.storage.selcloud.ru".to_string(),
            region: "ru-6".to_string(),
            bucket: "sparrow".to_string(),
            access_key_id: "access".to_string(),
            secret_access_key: "secret".to_string(),
        }
    }

    /// The signed GET URL handed to browsers must carry the bucket in the
    /// host, keep the object key as the path and stay SigV4-signed.
    #[tokio::test]
    async fn presigned_get_url_is_virtual_hosted_and_signed() {
        let storage = S3Storage::build(test_settings()).unwrap();
        let url = storage
            .presign_get("post-images/abc/original")
            .await
            .unwrap();
        let parsed = Url::parse(&url).unwrap();
        assert_eq!(
            parsed.host_str(),
            Some("sparrow.s3.ru-6.storage.selcloud.ru")
        );
        assert_eq!(parsed.path(), "/post-images/abc/original");
        let query = parsed.query().unwrap();
        assert!(query.contains("X-Amz-Algorithm=AWS4-HMAC-SHA256"), "{url}");
        assert!(query.contains("X-Amz-Signature="), "{url}");
    }

    #[test]
    fn malformed_endpoint_or_bucket_is_unavailable() {
        let endpoint = test_config(
            Some("s3.storage.selcloud.ru"),
            Some("ru-1"),
            Some("media"),
            Some("access"),
            Some("secret"),
        );
        assert_eq!(
            read_settings(&endpoint).err().unwrap().status,
            StatusCode::SERVICE_UNAVAILABLE
        );
        let bucket = test_config(
            Some("http://127.0.0.1:9000"),
            Some("ru-1"),
            Some("bad/bucket"),
            Some("access"),
            Some("secret"),
        );
        assert_eq!(
            read_settings(&bucket).err().unwrap().status,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn object_keys_are_derived_from_the_id_only() {
        let (original, thumbnail) = image_keys("7f3c2a10-1111-4bbb-8ccc-0123456789ab").unwrap();
        assert_eq!(
            original,
            "post-images/7f3c2a10-1111-4bbb-8ccc-0123456789ab/original"
        );
        assert_eq!(
            thumbnail,
            "post-images/7f3c2a10-1111-4bbb-8ccc-0123456789ab/thumbnail"
        );
        for invalid in ["", "../../etc/passwd", "has space", "slash/inside"] {
            assert_eq!(
                image_keys(invalid).unwrap_err().status,
                StatusCode::BAD_REQUEST
            );
        }
    }

    #[test]
    fn object_key_validation_rejects_foreign_keys() {
        assert!(validate_object_key("post-images/abc/original").is_ok());
        assert!(validate_object_key("post-images/abc/thumbnail").is_ok());
        assert!(validate_object_key("other/abc/original").is_err());
        assert!(validate_object_key("post-images/../secret").is_err());
        assert!(validate_object_key("post-images/abc\n/thumbnail").is_err());
    }

    #[test]
    fn accepts_jpeg_png_and_webp() {
        for (format, content_type) in [
            (ImageFormat::Jpeg, "image/jpeg"),
            (ImageFormat::Png, "image/png"),
            (ImageFormat::WebP, "image/webp"),
        ] {
            let prepared = prepared(format, 64, 48);
            assert_eq!(prepared.content_type, content_type);
            assert_eq!((prepared.width, prepared.height), (64, 48));
            assert_eq!(
                image::guess_format(&prepared.thumbnail).unwrap(),
                ImageFormat::WebP
            );
            assert_eq!(decoded_dimensions(&prepared.thumbnail), (64, 48));
        }
    }

    #[test]
    fn original_bytes_are_stored_untouched() {
        let source = encoded_sample(ImageFormat::Png, 32, 24);
        let prepared = prepare_image(Bytes::from(source.clone())).unwrap();
        assert_eq!(prepared.original.as_ref(), source.as_slice());
    }

    #[test]
    fn thumbnail_is_bounded_and_never_upscaled() {
        assert_eq!(
            decoded_dimensions(&prepared(ImageFormat::Png, 960, 480).thumbnail),
            (480, 240)
        );
        assert_eq!(
            decoded_dimensions(&prepared(ImageFormat::Png, 480, 960).thumbnail),
            (240, 480)
        );
        assert_eq!(
            decoded_dimensions(&prepared(ImageFormat::Png, 300, 200).thumbnail),
            (300, 200)
        );
        assert_eq!(
            decoded_dimensions(&prepared(ImageFormat::Png, 1000, 1000).thumbnail),
            (480, 480)
        );
    }

    #[test]
    fn rejects_unsupported_and_unreadable_payloads() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10"/></svg>"#.to_vec();
        assert_eq!(rejected(svg).status, StatusCode::BAD_REQUEST);
        assert_eq!(
            rejected(b"not an image at all".to_vec()).status,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(rejected(Vec::new()).status, StatusCode::BAD_REQUEST);

        let png = encoded_sample(ImageFormat::Png, 16, 16);
        let truncated = png[..png.len() / 2].to_vec();
        assert_eq!(rejected(truncated).status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn rejects_oversized_uploads() {
        assert_eq!(
            rejected(vec![0u8; MAX_UPLOAD_BYTES + 1]).status,
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn rejects_oversized_dimensions_before_decoding() {
        let wide = encoded_sample(ImageFormat::Png, MAX_IMAGE_DIMENSION + 1, 1);
        let error = rejected(wide);
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert!(error.message.contains(&MAX_IMAGE_DIMENSION.to_string()));
    }

    #[test]
    fn pixel_budget_boundaries() {
        assert!(dimensions_within_limits(8192, 2929));
        assert!(dimensions_within_limits(5000, 4800));
        assert!(!dimensions_within_limits(5001, 4800));
        assert!(!dimensions_within_limits(MAX_IMAGE_DIMENSION + 1, 1));
        assert!(!dimensions_within_limits(
            MAX_IMAGE_DIMENSION,
            MAX_IMAGE_DIMENSION
        ));
        assert!(!dimensions_within_limits(0, 10));
        assert!(!dimensions_within_limits(10, 0));
    }

    #[test]
    fn thumbnail_dimensions_preserve_aspect_without_upscaling() {
        assert_eq!(thumbnail_dimensions(960, 480), (480, 240));
        assert_eq!(thumbnail_dimensions(480, 960), (240, 480));
        assert_eq!(thumbnail_dimensions(100, 50), (100, 50));
        assert_eq!(thumbnail_dimensions(480, 480), (480, 480));
        assert_eq!(thumbnail_dimensions(5000, 1), (480, 1));
        assert_eq!(thumbnail_dimensions(1, 5000), (1, 480));
        assert_eq!(thumbnail_dimensions(481, 100), (480, 100));
    }

    #[test]
    fn exif_orientation_rotates_thumbnail_and_reported_dimensions() {
        const RED: [u8; 3] = [255, 0, 0];
        const GREEN: [u8; 3] = [0, 255, 0];
        const BLUE: [u8; 3] = [0, 0, 255];
        const YELLOW: [u8; 3] = [255, 255, 0];

        // Orientation 6 rotates the stored pixels 90° clockwise and
        // orientation 8 rotates them 270° clockwise; both swap the axes.
        // Displayed quadrants are listed top-left, top-right, bottom-left,
        // bottom-right.
        for (exif, displayed) in [
            (6u8, [BLUE, RED, YELLOW, GREEN]),
            (8u8, [GREEN, YELLOW, RED, BLUE]),
        ] {
            let source = oriented_quadrant_jpeg(64, 48, exif);
            let prepared = prepare_image(Bytes::from(source.clone())).unwrap();

            // The stored original keeps its bytes, Exif tag and stored axes.
            assert_eq!(prepared.original.as_ref(), source.as_slice());
            assert_eq!(
                (prepared.width, prepared.height),
                (48, 64),
                "orientation {exif}"
            );

            let thumbnail = decoded_rgb(&prepared.thumbnail);
            assert_eq!(thumbnail.dimensions(), (48, 64), "orientation {exif}");
            assert_quadrant_colors(&thumbnail, displayed, &format!("orientation {exif}"));
        }
    }

    /// Consumer-visible regression: macOS `sips` appended its own Exif APP1
    /// block (without an orientation tag) after the camera block, and the
    /// decoder's last-wins Exif lookup silently dropped orientation 6, so
    /// the thumbnail and reported dimensions stayed at the stored axes.
    #[test]
    fn jpeg_orientation_comes_from_the_first_exif_segment() {
        const BLUE: [u8; 3] = [0, 0, 255];
        const RED: [u8; 3] = [255, 0, 0];
        const YELLOW: [u8; 3] = [255, 255, 0];
        const GREEN: [u8; 3] = [0, 255, 0];

        let source = include_bytes!("../tests/fixtures/post_image_orientation6.jpg").to_vec();
        // The fixture must keep both Exif segments or it stops covering the
        // last-wins decoder bug it was captured for.
        let exif_blocks = source
            .windows(6)
            .filter(|window| window.starts_with(b"Exif\0\0"))
            .count();
        assert_eq!(exif_blocks, 2);

        let prepared = prepare_image(Bytes::from(source.clone())).unwrap();
        assert_eq!(prepared.original.as_ref(), source.as_slice());
        assert_eq!((prepared.width, prepared.height), (600, 800));

        let thumbnail = decoded_rgb(&prepared.thumbnail);
        assert_eq!(thumbnail.dimensions(), (360, 480));
        assert_quadrant_colors(&thumbnail, [BLUE, RED, YELLOW, GREEN], "sips fixture");
    }

    #[tokio::test]
    async fn stalled_storage_requests_fail_with_the_established_gateway_error() {
        let value = storage_request(
            "test",
            std::future::ready(Ok::<u8, S3Error>(7)),
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(value, 7);

        let error = storage_request(
            "test",
            std::future::pending::<Result<u8, S3Error>>(),
            Duration::from_millis(10),
        )
        .await
        .err()
        .unwrap();
        assert_eq!(error.status, StatusCode::BAD_GATEWAY);
        assert_eq!(error.message, "Хранилище изображений недоступно.");
    }
}
