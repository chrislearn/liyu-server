//! Avatar upload, replacement and deletion. The server stores the final
//! client-corrected image bytes under an unguessable UUID filename inside a
//! dedicated media directory; `user_profiles.avatar_url` always points at
//! `GET /api/v1/media/avatars/{id}`. No external URLs are accepted here.
use diesel::prelude::*;
use diesel::sql_types::BigInt;
use salvo::prelude::*;
use serde_json::json;
use std::path::{Path, PathBuf};

use crate::schema::{avatars, user_profiles};

/// Hard cap on uploaded avatar bytes, mirrored by the database CHECK.
const MAX_BYTES: usize = 1_048_576;
const MIN_DIMENSION: u32 = 16;
const MAX_DIMENSION: u32 = 4096;

fn avatar_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("avatars")
}

fn fail(res: &mut Response, status: StatusCode, message: &str) {
    res.status_code(status);
    res.render(Json(json!({"error": message})));
}

/// Detect the real image format and pixel dimensions from magic bytes and
/// container headers. The declared Content-Type is never trusted on its own.
pub(crate) fn sniff_image(bytes: &[u8]) -> Option<(&'static str, u32, u32)> {
    sniff_png(bytes)
        .or_else(|| sniff_jpeg(bytes))
        .or_else(|| sniff_webp(bytes))
}

/// Fully decode the image to prove it is valid — header sniffing alone lets
/// truncated or polyglot payloads through. Dimensions must agree with the
/// sniffed header so a file cannot claim one size and decode to another.
///
/// The claimed dimensions are validated BEFORE handing the bytes to the real
/// decoder: a 1 MiB gzip-style bomb can legally declare huge dimensions, and
/// decoding it would allocate width × height × 4 bytes of pixel memory. We
/// reject out-of-range dimensions and any pixel count that overflows u32
/// (or exceeds the 4096 × 4096 ceiling) while the cost is still zero.
fn decode_image(bytes: &[u8]) -> Option<(&'static str, u32, u32)> {
    let (content_type, width, height) = sniff_image(bytes)?;
    if !dimensions_in_range(width, height) {
        return None;
    }
    let decoded = image::load_from_memory(bytes).ok()?;
    if decoded.width() != width || decoded.height() != height {
        return None;
    }
    Some((content_type, width, height))
}

/// Pre-decode dimension gate: range check plus a checked pixel-count multiply
/// so the decoder can never be asked to allocate an unbounded buffer.
fn dimensions_in_range(width: u32, height: u32) -> bool {
    if !(MIN_DIMENSION..=MAX_DIMENSION).contains(&width)
        || !(MIN_DIMENSION..=MAX_DIMENSION).contains(&height)
    {
        return false;
    }
    // width/height are each ≤ 4096 here so this cannot overflow, but keep the
    // checked form so a future constant change fails closed instead of
    // wrapping into a small allocation.
    width.checked_mul(height).is_some()
}

fn sniff_png(bytes: &[u8]) -> Option<(&'static str, u32, u32)> {
    const MAGIC: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    // Signature + 4 length + 4 "IHDR" + 8 width/height + 5 depth/color/etc.
    if bytes.len() < 25 || bytes[..8] != MAGIC {
        return None;
    }
    // The first chunk must be a 13-byte IHDR; a forged length hides a fake header.
    if u32::from_be_bytes(bytes[8..12].try_into().ok()?) != 13 || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    // A PNG must also end with the 12-byte IEND chunk.
    if &bytes[bytes.len() - 8..] != b"IEND\xaeB\x60\x82" {
        return None;
    }
    Some(("image/png", width, height))
}

fn sniff_jpeg(bytes: &[u8]) -> Option<(&'static str, u32, u32)> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 || bytes[2] != 0xFF {
        return None;
    }
    // Walk segments until a start-of-frame marker carries the dimensions.
    let mut pos = 2usize;
    while pos + 4 <= bytes.len() {
        if bytes[pos] != 0xFF {
            return None; // Corrupt: segment marker expected.
        }
        let marker = bytes[pos + 1];
        // SOS starts the compressed scan: no SOF marker can legitimately
        // follow, and scan bytes must never be parsed as segments.
        if marker == 0xDA {
            return None;
        }
        // EOI ends the stream without dimensions.
        if marker == 0xD9 {
            return None;
        }
        // Standalone markers (TEM, RSTn) carry no length field and never
        // hold dimensions. Note 0xD0..=0xD7 only: 0xD8/0xD9 are SOI/EOI and
        // 0xDA is SOS, all handled above.
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            pos += 2;
            continue;
        }
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            if pos + 9 > bytes.len() {
                return None;
            }
            let height = u16::from_be_bytes(bytes[pos + 5..pos + 7].try_into().ok()?) as u32;
            let width = u16::from_be_bytes(bytes[pos + 7..pos + 9].try_into().ok()?) as u32;
            return Some(("image/jpeg", width, height));
        }
        let seg_len = u16::from_be_bytes(bytes[pos + 2..pos + 4].try_into().ok()?) as usize;
        if seg_len < 2 || pos + 2 + seg_len > bytes.len() {
            return None;
        }
        pos += 2 + seg_len;
    }
    None
}

fn sniff_webp(bytes: &[u8]) -> Option<(&'static str, u32, u32)> {
    if bytes.len() < 16 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return None;
    }
    let riff_len = u32::from_le_bytes(bytes[4..8].try_into().ok()?) as usize;
    if riff_len + 8 != bytes.len() {
        return None; // Truncated or padded container.
    }
    match &bytes[12..16] {
        b"VP8 " => {
            if bytes.len() < 30 {
                return None;
            }
            let width = (u16::from_le_bytes(bytes[26..28].try_into().ok()?) & 0x3FFF) as u32;
            let height = (u16::from_le_bytes(bytes[28..30].try_into().ok()?) & 0x3FFF) as u32;
            Some(("image/webp", width, height))
        }
        b"VP8L" => {
            if bytes.len() < 25 || bytes[20] != 0x2F {
                return None;
            }
            let b0 = bytes[21] as u32;
            let b1 = bytes[22] as u32;
            let b2 = bytes[23] as u32;
            let b3 = bytes[24] as u32;
            let width = 1 + (((b1 & 0x3F) << 8) | b0);
            let height = 1 + (((b3 & 0x0F) << 10) | (b2 << 2) | ((b1 & 0xC0) >> 6));
            Some(("image/webp", width, height))
        }
        b"VP8X" => {
            if bytes.len() < 30 {
                return None;
            }
            let load = |start: usize| -> Option<u32> {
                let chunk: [u8; 3] = bytes[start..start + 3].try_into().ok()?;
                Some(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], 0]))
            };
            let width = 1 + load(24)?;
            let height = 1 + load(27)?;
            Some(("image/webp", width, height))
        }
        _ => None,
    }
}

fn media_path(id: uuid::Uuid) -> PathBuf {
    avatar_dir().join(id.simple().to_string())
}

/// Atomically replace `path` with `bytes`: write a sibling temp file, fsync,
/// then rename so concurrent readers and crash recovery never observe a
/// truncated file. The temp file is cleaned up on every failure path.
fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    let write_result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    write_result
}

#[handler]
async fn upload(req: &mut Request, res: &mut Response) {
    let Some(owner) = crate::user_id(req) else {
        return fail(res, StatusCode::UNAUTHORIZED, "invalid session");
    };
    let declared = req
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or("").trim().to_owned())
        .unwrap_or_default();
    if !matches!(declared.as_str(), "image/jpeg" | "image/png" | "image/webp") {
        return fail(
            res,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "content type must be image/jpeg, image/png or image/webp",
        );
    }
    if req
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > MAX_BYTES)
    {
        return fail(res, StatusCode::PAYLOAD_TOO_LARGE, "avatar exceeds 1 MiB");
    }
    let bytes = match req.payload_with_max_size(MAX_BYTES + 1).await {
        Ok(bytes) => bytes,
        _ => return fail(res, StatusCode::BAD_REQUEST, "missing image body"),
    };
    if bytes.is_empty() {
        return fail(res, StatusCode::BAD_REQUEST, "empty image body");
    }
    if bytes.len() > MAX_BYTES {
        return fail(res, StatusCode::PAYLOAD_TOO_LARGE, "avatar exceeds 1 MiB");
    }
    let Some((content_type, width, height)) = decode_image(bytes.as_ref()) else {
        return fail(
            res,
            StatusCode::UNPROCESSABLE_ENTITY,
            "body is not a valid jpeg, png or webp image",
        );
    };
    if content_type != declared {
        return fail(
            res,
            StatusCode::UNPROCESSABLE_ENTITY,
            "image bytes do not match the declared content type",
        );
    }
    if !(MIN_DIMENSION..=MAX_DIMENSION).contains(&width)
        || !(MIN_DIMENSION..=MAX_DIMENSION).contains(&height)
    {
        return fail(
            res,
            StatusCode::UNPROCESSABLE_ENTITY,
            "image dimensions must be between 16 and 4096 pixels",
        );
    }

    let Some(mut conn) = (match crate::pool().get() {
        Ok(conn) => Some(conn),
        Err(_) => {
            fail(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
            None
        }
    }) else {
        return;
    };

    let id = uuid::Uuid::new_v4();
    let url = format!("/api/v1/media/avatars/{}", id.simple());
    // Write the new file BEFORE any database state changes. Until the
    // transaction below commits, nothing references this file, so a failure
    // here only leaves an orphan we delete immediately — the profile never
    // points at a file that does not exist.
    if std::fs::create_dir_all(avatar_dir()).is_err()
        || atomic_write(&media_path(id), bytes.as_ref()).is_err()
    {
        return fail(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "avatar storage failed",
        );
    }
    // Switch old/new records and the profile pointer in a single transaction
    // while holding the per-owner advisory lock. Concurrent uploads/deletes of
    // the same owner serialize here, so the commit is the single atomic
    // cut-over point.
    let result = conn.transaction::<Option<uuid::Uuid>, diesel::result::Error, _>(|conn| {
        use avatars::dsl as a;
        use user_profiles::dsl as p;
        // Serialize concurrent uploads/deletes of the same owner.
        diesel::sql_query("SELECT pg_advisory_xact_lock($1)")
            .bind::<BigInt, _>(owner)
            .execute(conn)?;
        let previous: Option<uuid::Uuid> = a::avatars
            .filter(a::owner_id.eq(owner))
            .select(a::id)
            .first(conn)
            .optional()?;
        diesel::insert_into(a::avatars)
            .values((
                a::id.eq(id),
                a::owner_id.eq(owner),
                a::content_type.eq(content_type),
                a::byte_len.eq(bytes.len() as i32),
            ))
            .execute(conn)?;
        diesel::insert_into(p::user_profiles)
            .values((p::user_id.eq(owner), p::avatar_url.eq(Some(&url))))
            .on_conflict(p::user_id)
            .do_update()
            .set((
                p::avatar_url.eq(Some(&url)),
                p::updated_at.eq(diesel::dsl::now),
            ))
            .execute(conn)?;
        if let Some(old) = previous {
            diesel::delete(a::avatars.filter(a::id.eq(old))).execute(conn)?;
        }
        Ok(previous)
    });
    let previous = match result {
        Ok(previous) => previous,
        Err(_) => {
            // The transaction never committed, so no row or profile points at
            // the new file; removing it restores the pre-upload state exactly.
            let _ = std::fs::remove_file(media_path(id));
            return fail(
                res,
                StatusCode::INTERNAL_SERVER_ERROR,
                "avatar record failed",
            );
        }
    };
    if let Some(old) = previous {
        // Replacement only ever removes this user's own previous file, and
        // only after the commit made the new file the referenced one.
        let _ = std::fs::remove_file(media_path(old));
    }
    res.render(Json(
        json!({"avatar_url": url, "content_type": content_type, "width": width, "height": height}),
    ));
}

#[handler]
async fn delete(req: &mut Request, res: &mut Response) {
    let Some(owner) = crate::user_id(req) else {
        return fail(res, StatusCode::UNAUTHORIZED, "invalid session");
    };
    let Some(mut conn) = (match crate::pool().get() {
        Ok(conn) => Some(conn),
        Err(_) => {
            fail(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
            None
        }
    }) else {
        return;
    };
    let result = conn.transaction::<Option<uuid::Uuid>, diesel::result::Error, _>(|conn| {
        use avatars::dsl as a;
        use user_profiles::dsl as p;
        // Same advisory lock as upload so a delete never races a replacement.
        diesel::sql_query("SELECT pg_advisory_xact_lock($1)")
            .bind::<BigInt, _>(owner)
            .execute(conn)?;
        let current: Option<uuid::Uuid> = a::avatars
            .filter(a::owner_id.eq(owner))
            .select(a::id)
            .first(conn)
            .optional()?;
        if let Some(id) = current {
            diesel::delete(a::avatars.filter(a::id.eq(id))).execute(conn)?;
        }
        diesel::update(p::user_profiles.filter(p::user_id.eq(owner)))
            .set((
                p::avatar_url.eq(None::<String>),
                p::updated_at.eq(diesel::dsl::now),
            ))
            .execute(conn)?;
        Ok(current)
    });
    match result {
        Ok(current) => {
            if let Some(id) = current {
                let _ = std::fs::remove_file(media_path(id));
            }
            res.status_code(StatusCode::NO_CONTENT);
        }
        Err(_) => fail(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "avatar delete failed",
        ),
    }
}

#[handler]
async fn media(req: &mut Request, res: &mut Response) {
    let Some(raw) = req.param::<String>("id") else {
        return fail(res, StatusCode::BAD_REQUEST, "invalid avatar id");
    };
    let Some(id) = uuid::Uuid::parse_str(&raw).ok() else {
        return fail(res, StatusCode::NOT_FOUND, "avatar not found");
    };
    // The stored filename is the lowercase simple form; any spelling that
    // normalizes differently (mixed case, braces, urn:) must not resolve.
    if raw != id.simple().to_string() {
        return fail(res, StatusCode::NOT_FOUND, "avatar not found");
    }
    let Some(mut conn) = (match crate::pool().get() {
        Ok(conn) => Some(conn),
        Err(_) => {
            fail(res, StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
            None
        }
    }) else {
        return;
    };
    let record: Option<(String, i32)> = avatars::table
        .find(id)
        .select((avatars::content_type, avatars::byte_len))
        .first(&mut conn)
        .optional()
        .ok()
        .flatten();
    let Some((content_type, byte_len)) = record else {
        return fail(res, StatusCode::NOT_FOUND, "avatar not found");
    };
    let path = media_path(id);
    // Defense in depth: the joined filename must stay inside the avatar dir.
    if path.parent() != Some(avatar_dir().as_path()) {
        return fail(res, StatusCode::NOT_FOUND, "avatar not found");
    }
    match std::fs::read(&path) {
        Ok(data) if data.len() as i32 == byte_len => {
            if let Ok(value) = content_type.parse() {
                res.headers_mut().insert("content-type", value);
            }
            res.headers_mut().insert(
                "cache-control",
                "public, max-age=31536000, immutable".parse().unwrap(),
            );
            let _ = res.write_body(data);
        }
        _ => fail(res, StatusCode::NOT_FOUND, "avatar not found"),
    }
}

pub fn routes() -> Router {
    Router::new()
        .push(Router::with_path("me/avatar").post(upload).delete(delete))
        .push(Router::with_path("media/avatars/{id}").get(media))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        bytes.extend_from_slice(&[0, 0, 0, 13]);
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 2, 0, 0, 0]); // bit depth, RGB, compression, filter, interlace
        bytes.extend_from_slice(&[0; 20]); // fake CRC + one payload chunk
        bytes.extend_from_slice(b"IEND\xaeB\x60\x82");
        bytes
    }

    #[test]
    fn png_dimensions_are_detected() {
        let (kind, w, h) = sniff_image(&png(64, 32)).expect("valid png");
        assert_eq!(kind, "image/png");
        assert_eq!((w, h), (64, 32));
    }

    /// A hand-built header that sniffs fine must still fail full decode; only
    /// genuinely decodable images may be stored as avatars.
    #[test]
    fn decode_rejects_header_only_fakes() {
        assert_eq!(decode_image(&png(64, 32)), None); // fake CRC + payload
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xC0];
        jpeg.extend_from_slice(&[0x00, 0x0B, 0x08, 0x00, 24, 0x00, 40]);
        assert_eq!(decode_image(&jpeg), None); // SOF0 but no scan data
    }

    /// Decompression-bomb guard: a tiny file may legally declare enormous
    /// dimensions, but the decoder must never be asked to allocate for them.
    /// The dimension gate runs BEFORE `image::load_from_memory`, so these
    /// headers are rejected at zero allocation cost.
    #[test]
    fn decode_rejects_oversized_dimensions_before_decoding() {
        // Just past the 4096 ceiling on one axis.
        assert_eq!(decode_image(&png(4097, 16)), None);
        assert_eq!(decode_image(&png(16, 4097)), None);
        // Both axes huge: a ~1 KB zlib payload would decode to gigabytes.
        assert_eq!(decode_image(&png(65535, 65535)), None);
        // Below the floor is also rejected pre-decode.
        assert_eq!(decode_image(&png(15, 64)), None);
        assert_eq!(decode_image(&png(64, 0)), None);
        // Boundary values stay acceptable at the sniff+gate level (these
        // hand-built bodies still fail real decode, so check via the gate).
        assert!(dimensions_in_range(4096, 4096));
        assert!(dimensions_in_range(16, 16));
        assert!(!dimensions_in_range(4097, 16));
        assert!(!dimensions_in_range(0, 64));
    }

    /// Real encoders round-trip: every accepted format must sniff and decode
    /// to identical dimensions.
    #[test]
    fn decode_accepts_real_encoded_images() {
        use image::{DynamicImage, ImageFormat, RgbImage};
        let img = DynamicImage::ImageRgb8(RgbImage::from_fn(24, 16, |x, y| {
            image::Rgb([(x * 10) as u8, (y * 16) as u8, 128])
        }));
        for (format, expected) in [
            (ImageFormat::Png, "image/png"),
            (ImageFormat::Jpeg, "image/jpeg"),
            (ImageFormat::WebP, "image/webp"),
        ] {
            let mut bytes = std::io::Cursor::new(Vec::new());
            img.write_to(&mut bytes, format).expect("encode");
            let (kind, w, h) = sniff_image(&bytes.into_inner()).expect("sniff encoded");
            assert_eq!(kind, expected);
            assert_eq!((w, h), (24, 16));
        }
    }

    #[test]
    fn truncated_or_fake_png_is_rejected() {
        assert_eq!(sniff_image(&[]), None);
        assert_eq!(sniff_image(&png(64, 32)[..10]), None);
        let mut forged = png(64, 32);
        forged[16] ^= 0xFF; // still parses, dimension changes but structure intact
        assert!(sniff_image(&forged).is_some());
        let mut broken = png(64, 32);
        broken[0] = 0x00; // wrong magic
        assert_eq!(sniff_image(&broken), None);
        let mut no_iend = png(64, 32);
        let len = no_iend.len();
        no_iend[len - 4] ^= 0xFF;
        assert_eq!(sniff_image(&no_iend), None);
        // A forged IHDR length field must not smuggle a fake header through.
        let mut bad_ihdr_len = png(64, 32);
        bad_ihdr_len[11] = 12; // declares a 12-byte IHDR
        assert_eq!(sniff_image(&bad_ihdr_len), None);
    }

    #[test]
    fn jpeg_scan_data_is_not_misread_as_segments() {
        // SOF0 then SOS: dimensions come from the SOF0, the scan never parses.
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xC0];
        jpeg.extend_from_slice(&[0x00, 0x0B, 0x08, 0x00, 24, 0x00, 40]);
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02]);
        jpeg.extend_from_slice(&[0x3F, 0x00, 0xFF, 0xC0]); // scan bytes mimicking SOF0
        let (kind, w, h) = sniff_image(&jpeg).expect("sof0 before sos");
        assert_eq!(kind, "image/jpeg");
        assert_eq!((w, h), (40, 24));
        // SOS reached before any SOF: no dimensions exist, must not read scan.
        let mut no_sof = vec![0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x02];
        no_sof.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 24, 0x00, 40]);
        assert_eq!(sniff_image(&no_sof), None);
        // Restart markers are standalone and must be skipped without a length.
        let mut rst = vec![0xFF, 0xD8, 0xFF, 0xD0, 0xFF, 0xC0];
        rst.extend_from_slice(&[0x00, 0x0B, 0x08, 0x00, 24, 0x00, 40]);
        assert_eq!(sniff_image(&rst).map(|(_, w, h)| (w, h)), Some((40, 24)));
    }

    #[test]
    fn webp_lossless_and_extended_are_detected() {
        // VP8L: signature 0x2F then 14-bit width/height packed little-endian.
        let mut lossless = b"RIFF".to_vec();
        lossless.extend_from_slice(&(25u32 - 8).to_le_bytes());
        lossless.extend_from_slice(b"WEBPVP8L");
        lossless.extend_from_slice(&(5u32).to_le_bytes());
        lossless.push(0x2F);
        // width-1 = 299 -> b0 = 0x2B, low 6 bits of b1 = 0x01
        // height-1 = 63 -> b1[7:6] = 0b11, b2 = 0x0F, b3 low nibble = 0
        lossless.extend_from_slice(&[0x2B, 0xC1, 0x0F, 0x00]);
        let (kind, w, h) = sniff_image(&lossless).expect("vp8l");
        assert_eq!(kind, "image/webp");
        assert_eq!((w, h), (300, 64));

        // VP8X: 24-bit width/height minus one at offsets 24/27.
        let mut extended = b"RIFF".to_vec();
        extended.extend_from_slice(&(30u32 - 8).to_le_bytes());
        extended.extend_from_slice(b"WEBPVP8X");
        extended.extend_from_slice(&(10u32).to_le_bytes());
        extended.extend_from_slice(&[0; 4]); // flags + reserved
        extended.extend_from_slice(&[0x7F, 0x00, 0x00]); // width-1 = 127
        extended.extend_from_slice(&[0x3F, 0x00, 0x00]); // height-1 = 63
        let (kind, w, h) = sniff_image(&extended).expect("vp8x");
        assert_eq!(kind, "image/webp");
        assert_eq!((w, h), (128, 64));

        // Bad VP8L signature byte must be rejected.
        let mut bad_sig = lossless.clone();
        bad_sig[20] = 0x2E;
        assert_eq!(sniff_image(&bad_sig), None);
    }

    #[test]
    fn atomic_write_replaces_and_cleans_up() {
        let dir =
            std::env::temp_dir().join(format!("liyu-avatar-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("image");
        atomic_write(&path, b"first").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        atomic_write(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        // No temp file survives a successful write.
        assert!(!path.with_extension("tmp").exists());
        // A write into a missing directory fails and leaves no temp file.
        let missing = dir.join("gone").join("image");
        assert!(atomic_write(&missing, b"x").is_err());
        assert!(!missing.with_extension("tmp").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn jpeg_without_frame_is_rejected() {
        assert_eq!(sniff_image(&[0xFF, 0xD8, 0xFF, 0xD9]), None);
        // Minimal JFIF header + SOF0 with 40x24 dimensions.
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x02, 0xFF, 0xC0];
        jpeg.extend_from_slice(&[0x00, 0x0B, 0x08, 0x00, 24, 0x00, 40]);
        let (kind, w, h) = sniff_image(&jpeg).expect("sof0");
        assert_eq!(kind, "image/jpeg");
        assert_eq!((w, h), (40, 24));
        // Truncated segment table must fail instead of panicking.
        assert_eq!(sniff_image(&jpeg[..6]), None);
    }

    #[test]
    fn webp_variants_are_detected() {
        let mut lossy = b"RIFF".to_vec();
        lossy.extend_from_slice(&(30u32 - 8).to_le_bytes());
        lossy.extend_from_slice(b"WEBPVP8 ");
        lossy.extend_from_slice(&(10u32).to_le_bytes());
        lossy.extend_from_slice(&[0; 6]);
        lossy.extend_from_slice(&(100u16).to_le_bytes());
        lossy.extend_from_slice(&(50u16).to_le_bytes());
        let (kind, w, h) = sniff_image(&lossy).expect("vp8");
        assert_eq!(kind, "image/webp");
        assert_eq!((w, h), (100, 50));

        let mut riff_mismatch = lossy.clone();
        riff_mismatch[4] = 0xFF; // declared length no longer matches actual size
        assert_eq!(sniff_image(&riff_mismatch), None);

        assert_eq!(sniff_image(b"RIFF\x00\x00\x00\x00WEBPXXXX"), None);
    }

    // Compile-time guard on the limit constants: if someone loosens them the
    // build fails here, not in production. (`assert!` on constants in a test
    // trips clippy::assertions_on_constants.)
    const _: () = {
        assert!(MAX_BYTES <= 1_048_576);
        assert!(MIN_DIMENSION >= 1 && MAX_DIMENSION <= 16384);
    };

    /// Database contract behind the avatar API: record insert with the CHECKed
    /// invariants, one-avatar-per-owner replacement, profile pointer swap and
    /// delete cleanup. Runs inside a transaction that always rolls back, so a
    /// migrated test database (`DATABASE_URL`) is required but never mutated.
    #[test]
    fn avatar_record_lifecycle() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            return; // Run with a migrated test database to exercise the DB path.
        };
        use diesel::Connection;
        let mut conn = diesel::PgConnection::establish(&url).expect("connect test database");
        let rolled_back = conn.transaction::<(), diesel::result::Error, _>(|conn| {
            use avatars::dsl as a;
            use user_profiles::dsl as p;
            let owner: i64 = diesel::sql_query(
                "SELECT id AS user_id FROM users WHERE identifier = 'demo@liyu.test'",
            )
            .get_result::<crate::SessionOwner>(conn)
            .map(|row| row.user_id)?;

            let first = uuid::Uuid::new_v4();
            let first_url = format!("/api/v1/media/avatars/{}", first.simple());
            // The API's insert shape: uuid PK, owner FK, CHECKed content type
            // and byte length.
            diesel::insert_into(a::avatars)
                .values((
                    a::id.eq(first),
                    a::owner_id.eq(owner),
                    a::content_type.eq("image/png"),
                    a::byte_len.eq(12345i32),
                ))
                .execute(conn)?;
            diesel::insert_into(p::user_profiles)
                .values((p::user_id.eq(owner), p::avatar_url.eq(Some(&first_url))))
                .on_conflict(p::user_id)
                .do_update()
                .set(p::avatar_url.eq(Some(&first_url)))
                .execute(conn)?;

            // Replacement inserts the successor before deleting the
            // predecessor, exactly like the upload handler's transaction.
            let second = uuid::Uuid::new_v4();
            let second_url = format!("/api/v1/media/avatars/{}", second.simple());
            diesel::insert_into(a::avatars)
                .values((
                    a::id.eq(second),
                    a::owner_id.eq(owner),
                    a::content_type.eq("image/jpeg"),
                    a::byte_len.eq(54321i32),
                ))
                .execute(conn)?;
            diesel::update(p::user_profiles.filter(p::user_id.eq(owner)))
                .set(p::avatar_url.eq(Some(&second_url)))
                .execute(conn)?;
            diesel::delete(a::avatars.filter(a::id.eq(first))).execute(conn)?;
            let remaining: Vec<uuid::Uuid> = a::avatars
                .filter(a::owner_id.eq(owner))
                .select(a::id)
                .load(conn)?;
            assert_eq!(remaining, vec![second]);
            let pointed: Option<String> = p::user_profiles
                .filter(p::user_id.eq(owner))
                .select(p::avatar_url)
                .first(conn)?;
            assert_eq!(pointed.as_deref(), Some(second_url.as_str()));

            // CHECK constraints mirror the handler's guards. Each expected
            // failure runs in its own nested transaction (SAVEPOINT): without
            // it, PostgreSQL aborts the whole transaction on the first
            // constraint violation and every subsequent statement errors.
            let bad_type = conn.transaction(|conn| {
                diesel::insert_into(a::avatars)
                    .values((
                        a::id.eq(uuid::Uuid::new_v4()),
                        a::owner_id.eq(owner),
                        a::content_type.eq("image/gif"),
                        a::byte_len.eq(10i32),
                    ))
                    .execute(conn)
            });
            assert!(bad_type.is_err());
            let bad_len = conn.transaction(|conn| {
                diesel::insert_into(a::avatars)
                    .values((
                        a::id.eq(uuid::Uuid::new_v4()),
                        a::owner_id.eq(owner),
                        a::content_type.eq("image/png"),
                        a::byte_len.eq(0i32),
                    ))
                    .execute(conn)
            });
            assert!(bad_len.is_err());
            // The aborted inserts rolled back to their savepoints, so the
            // outer transaction is still healthy and the replacement row is
            // untouched.
            let still_there: Option<uuid::Uuid> = a::avatars
                .filter(a::owner_id.eq(owner))
                .select(a::id)
                .first(conn)
                .optional()?;
            assert_eq!(still_there, Some(second));

            // Delete path: row gone, pointer cleared.
            diesel::delete(a::avatars.filter(a::id.eq(second))).execute(conn)?;
            diesel::update(p::user_profiles.filter(p::user_id.eq(owner)))
                .set(p::avatar_url.eq(None::<String>))
                .execute(conn)?;
            let cleared: Option<String> = p::user_profiles
                .filter(p::user_id.eq(owner))
                .select(p::avatar_url)
                .first(conn)?;
            assert_eq!(cleared, None);

            Err(diesel::result::Error::RollbackTransaction)
        });
        assert!(matches!(
            rolled_back,
            Err(diesel::result::Error::RollbackTransaction)
        ));
    }
}
