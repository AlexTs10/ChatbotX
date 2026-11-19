//! `dynamic-image/render.ts`, `fonts.ts` and `qr.ts`: the PNG layers of a
//! dynamic image, drawn with `tiny-skia` (the original: `@napi-rs/canvas`).
//!
//! Rendering is sync and does no I/O: the caller fetches every image URL the
//! drawn elements need ([`image_urls`]) and passes the bytes in.
//!
//! ponytail: text is laid out glyph by glyph on the advance widths (no
//! shaping, no kerning, no synthetic bold / italic for Great Vibes); the
//! `top` baseline is the font's `hhea` ascender. Good enough for Latin and
//! Vietnamese labels; use a shaper (rustybuzz) if a script needs it.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use tiny_skia::{
    FillRule, FilterQuality, Mask, Paint, PathBuilder, Pattern, Pixmap, Rect, SpreadMode, Transform,
};

/// `RENDER_SCALE`: supersampling of the nominal canvas.
pub const RENDER_SCALE: f32 = 2.0;
const LOGO_SIZE_RATIO: f32 = 0.22;
const LOGO_PADDING_RATIO: f32 = 0.1;

/// `DynamicImageDocument` (zod defaults applied by the API input check).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub width: f64,
    pub height: f64,
    pub elements: Vec<Element>,
}

/// `DynamicImageElement`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Element {
    #[serde(rename = "image")]
    Image(ImageElement),
    #[serde(rename = "qrCode")]
    QrCode(QrCodeElement),
    #[serde(rename = "text")]
    Text(TextElement),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageElement {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    #[serde(default)]
    pub priority: bool,
    pub image_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_field_id: Option<String>,
    #[serde(default = "square")]
    pub image_style: String,
}

fn square() -> String {
    "square".to_owned()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QrCodeElement {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    #[serde(default)]
    pub priority: bool,
    pub text: String,
    pub size: u32,
    pub color: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextElement {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    #[serde(default)]
    pub priority: bool,
    pub text: String,
    pub font_size: f64,
    pub font_family: String,
    pub align: String,
    pub color: String,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub uppercase: bool,
}

impl Element {
    pub fn id(&self) -> &str {
        match self {
            Element::Image(e) => &e.id,
            Element::QrCode(e) => &e.id,
            Element::Text(e) => &e.id,
        }
    }
}

/// `elementHasVariable` (`/\{\{[^}]+\}\}/`).
pub fn element_has_variable(text: &str) -> bool {
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let inner = after.find('}').unwrap_or(after.len());
        if inner > 0 && after[inner..].starts_with("}}") {
            return true;
        }
        rest = after;
    }
    false
}

/// `isStaticElement`.
pub fn is_static_element(element: &Element) -> bool {
    match element {
        Element::Image(e) => e.image_type == "url" && !e.priority,
        Element::QrCode(e) => !element_has_variable(&e.text),
        Element::Text(e) => !element_has_variable(&e.text),
    }
}

/// `getDynamicElementIds`: every element from the first dynamic one onward.
pub fn dynamic_element_ids(document: &Document) -> HashSet<String> {
    match document.elements.iter().position(|e| !is_static_element(e)) {
        None => HashSet::new(),
        Some(first) => document.elements[first..]
            .iter()
            .map(|e| e.id().to_owned())
            .collect(),
    }
}

/// The image URLs the drawn elements (`ids` = drawn, `None` = the static run)
/// read: `url` images and QR logos.
pub fn image_urls(document: &Document, drawn: &dyn Fn(&Element) -> bool) -> Vec<String> {
    let mut out = Vec::new();
    for element in document.elements.iter().filter(|e| drawn(e)) {
        let url = match element {
            Element::Image(e) => e.url.as_deref(),
            Element::QrCode(e) => e.logo_url.as_deref(),
            Element::Text(_) => None,
        };
        if let Some(url) = url.filter(|u| !u.is_empty())
            && !out.iter().any(|u: &String| u == url)
        {
            out.push(url.to_owned());
        }
    }
    out
}

/// A failed draw (`loadImage` / encode errors are thrown in the original).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderError(pub String);

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn err(message: impl Into<String>) -> RenderError {
    RenderError(message.into())
}

/// The fetched image bytes by URL.
pub type Images = HashMap<String, Vec<u8>>;

/// `renderStaticLayer`: the leading run of static elements.
pub fn render_static_layer(document: &Document, images: &Images) -> Result<Vec<u8>, RenderError> {
    let dynamic = dynamic_element_ids(document);
    let mut canvas = Canvas::new(document.width, document.height)?;
    for element in &document.elements {
        if !dynamic.contains(element.id()) {
            canvas.draw(element, images)?;
        }
    }
    canvas.png()
}

/// `renderDynamicLayer`: the background, then the `dynamic` elements of the
/// variable-resolved `document`.
pub fn render_dynamic_layer(
    document: &Document,
    background: &[u8],
    dynamic: &HashSet<String>,
    images: &Images,
) -> Result<Vec<u8>, RenderError> {
    let mut canvas = Canvas::new(document.width, document.height)?;
    let background = decode(background)?;
    canvas.draw_pixmap(
        &background,
        0.0,
        0.0,
        document.width as f32,
        document.height as f32,
        None,
    );
    for element in &document.elements {
        if dynamic.contains(element.id()) {
            canvas.draw(element, images)?;
        }
    }
    canvas.png()
}

// ---- canvas ---------------------------------------------------------------------

struct Canvas {
    pixmap: Pixmap,
    transform: Transform,
}

impl Canvas {
    fn new(width: f64, height: f64) -> Result<Canvas, RenderError> {
        let w = (width * f64::from(RENDER_SCALE)).round().max(1.0) as u32;
        let h = (height * f64::from(RENDER_SCALE)).round().max(1.0) as u32;
        let pixmap = Pixmap::new(w, h).ok_or_else(|| err("Invalid canvas size"))?;
        Ok(Canvas {
            pixmap,
            transform: Transform::from_scale(RENDER_SCALE, RENDER_SCALE),
        })
    }

    fn draw(&mut self, element: &Element, images: &Images) -> Result<(), RenderError> {
        match element {
            Element::Image(e) => self.draw_image(e, images),
            Element::QrCode(e) => self.draw_qr_code(e, images),
            Element::Text(e) => {
                self.draw_text(e);
                Ok(())
            }
        }
    }

    fn mask(&self, path: &tiny_skia::Path) -> Option<Mask> {
        let mut mask = Mask::new(self.pixmap.width(), self.pixmap.height())?;
        mask.fill_path(path, FillRule::Winding, true, self.transform);
        Some(mask)
    }

    /// `ctx.drawImage(image, x, y, w, h)` (stretched, high quality).
    fn draw_pixmap(&mut self, src: &Pixmap, x: f32, y: f32, w: f32, h: f32, mask: Option<&Mask>) {
        let Some(rect) = Rect::from_xywh(x, y, w, h) else {
            return;
        };
        let shader = Pattern::new(
            src.as_ref(),
            SpreadMode::Pad,
            FilterQuality::Bicubic,
            1.0,
            Transform::from_row(
                w / src.width() as f32,
                0.0,
                0.0,
                h / src.height() as f32,
                x,
                y,
            ),
        );
        let paint = Paint {
            shader,
            anti_alias: true,
            ..Paint::default()
        };
        self.pixmap.fill_rect(rect, &paint, self.transform, mask);
    }

    fn draw_image(&mut self, e: &ImageElement, images: &Images) -> Result<(), RenderError> {
        let Some(url) = e.url.as_deref().filter(|u| !u.is_empty()) else {
            return Ok(());
        };
        let bytes = images
            .get(url)
            .ok_or_else(|| err(format!("Failed to fetch image from {url}")))?;
        let image = decode(bytes)?;
        let mask = if e.image_style == "circle" {
            let radius = e.width.min(e.height) / 2.0;
            PathBuilder::from_circle(
                (e.x + e.width / 2.0) as f32,
                (e.y + e.height / 2.0) as f32,
                radius as f32,
            )
            .and_then(|path| self.mask(&path))
        } else {
            None
        };
        let (dx, dy, dw, dh) = contain_rect(
            f64::from(image.width()),
            f64::from(image.height()),
            e.x,
            e.y,
            e.width,
            e.height,
        );
        if e.image_style == "circle" && mask.is_none() {
            return Ok(());
        }
        self.draw_pixmap(
            &image,
            dx as f32,
            dy as f32,
            dw as f32,
            dh as f32,
            mask.as_ref(),
        );
        Ok(())
    }

    fn draw_qr_code(&mut self, e: &QrCodeElement, images: &Images) -> Result<(), RenderError> {
        let logo = match e.logo_url.as_deref().filter(|u| !u.is_empty()) {
            Some(url) => {
                Some(decode(images.get(url).ok_or_else(|| {
                    err(format!("Failed to fetch image from {url}"))
                })?)?)
            }
            None => None,
        };
        let qr = qr_code_pixmap(&e.text, e.size, &e.color, logo.as_ref())?;
        let side = e.width as f32;
        self.draw_pixmap(&qr, e.x as f32, e.y as f32, side, side, None);
        Ok(())
    }

    fn draw_text(&mut self, e: &TextElement) {
        let Some(clip) = Rect::from_xywh(e.x as f32, e.y as f32, e.width as f32, e.height as f32)
        else {
            return;
        };
        let Some(mask) = self.mask(&PathBuilder::from_rect(clip)) else {
            return;
        };
        let font = Font::new(&e.font_family, e.bold, e.italic, e.font_size as f32);
        let text = if e.uppercase {
            e.text.to_uppercase()
        } else {
            e.text.clone()
        };
        let line_height = e.font_size as f32 * 1.2;
        let mut paint = Paint::default();
        let [r, g, b, a] = parse_color(&e.color).unwrap_or([0, 0, 0, 255]);
        paint.set_color_rgba8(r, g, b, a);
        paint.anti_alias = true;
        let anchor = match e.align.as_str() {
            "center" => e.x + e.width / 2.0,
            "right" => e.x + e.width,
            _ => e.x,
        } as f32;
        for (index, line) in wrap_text(&font, &text, e.width as f32).iter().enumerate() {
            let width = font.measure(line);
            let left = match e.align.as_str() {
                "center" => anchor - width / 2.0,
                "right" => anchor - width,
                _ => anchor,
            };
            let top = e.y as f32 + index as f32 * line_height;
            if let Some(path) = font.path(line, left, top) {
                self.pixmap.fill_path(
                    &path,
                    &paint,
                    FillRule::Winding,
                    self.transform,
                    Some(&mask),
                );
            }
        }
    }

    fn png(&self) -> Result<Vec<u8>, RenderError> {
        encode_png(&self.pixmap)
    }
}

/// `getContainRect` (`object-fit: contain`): `(dx, dy, dWidth, dHeight)`.
pub fn contain_rect(
    src_width: f64,
    src_height: f64,
    box_x: f64,
    box_y: f64,
    box_width: f64,
    box_height: f64,
) -> (f64, f64, f64, f64) {
    let src_ratio = src_width / src_height;
    let box_ratio = box_width / box_height;
    let (w, h) = if src_ratio > box_ratio {
        (box_width, box_width / src_ratio)
    } else {
        (box_height * src_ratio, box_height)
    };
    (
        box_x + (box_width - w) / 2.0,
        box_y + (box_height - h) / 2.0,
        w,
        h,
    )
}

fn decode(bytes: &[u8]) -> Result<Pixmap, RenderError> {
    let image = image::load_from_memory(bytes)
        .map_err(|e| err(format!("Unsupported image: {e}")))?
        .to_rgba8();
    let (w, h) = image.dimensions();
    let mut data = image.into_raw();
    for px in data.chunks_exact_mut(4) {
        let a = u16::from(px[3]);
        for c in &mut px[..3] {
            *c = ((u16::from(*c) * a + 127) / 255) as u8;
        }
    }
    let size = tiny_skia::IntSize::from_wh(w, h).ok_or_else(|| err("Empty image"))?;
    Pixmap::from_vec(data, size).ok_or_else(|| err("Invalid image"))
}

fn encode_png(pixmap: &Pixmap) -> Result<Vec<u8>, RenderError> {
    let mut rgba = Vec::with_capacity(pixmap.data().len());
    for px in pixmap.pixels() {
        let c = px.demultiply();
        rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    let mut out = Vec::new();
    image::ImageEncoder::write_image(
        image::codecs::png::PngEncoder::new(&mut out),
        &rgba,
        pixmap.width(),
        pixmap.height(),
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|e| err(e.to_string()))?;
    Ok(out)
}

/// A CSS color: `#rgb[a]`, `#rrggbb[aa]`, `rgb[a](...)` or a basic name;
/// `None` = not parsed (the canvas keeps its default black).
pub fn parse_color(text: &str) -> Option<[u8; 4]> {
    let text = text.trim().to_ascii_lowercase();
    if let Some(hex) = text.strip_prefix('#') {
        let digit = |i: usize, n: usize| u8::from_str_radix(hex.get(i..i + n)?, 16).ok();
        let short = |i: usize| digit(i, 1).map(|d| d * 17);
        return match hex.len() {
            3 => Some([short(0)?, short(1)?, short(2)?, 255]),
            4 => Some([short(0)?, short(1)?, short(2)?, short(3)?]),
            6 => Some([digit(0, 2)?, digit(2, 2)?, digit(4, 2)?, 255]),
            8 => Some([digit(0, 2)?, digit(2, 2)?, digit(4, 2)?, digit(6, 2)?]),
            _ => None,
        };
    }
    if let Some(args) = text
        .strip_prefix("rgba(")
        .or_else(|| text.strip_prefix("rgb("))
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let parts: Vec<&str> = args
            .split([',', ' ', '/'])
            .filter(|p| !p.is_empty())
            .collect();
        if parts.len() < 3 {
            return None;
        }
        let channel = |p: &str| -> Option<u8> {
            match p.strip_suffix('%') {
                Some(pct) => {
                    Some((pct.parse::<f64>().ok()? * 2.55).round().clamp(0.0, 255.0) as u8)
                }
                None => Some(p.parse::<f64>().ok()?.round().clamp(0.0, 255.0) as u8),
            }
        };
        let alpha = match parts.get(3) {
            Some(p) => match p.strip_suffix('%') {
                Some(pct) => pct.parse::<f64>().ok()? / 100.0,
                None => p.parse::<f64>().ok()?,
            },
            None => 1.0,
        };
        return Some([
            channel(parts[0])?,
            channel(parts[1])?,
            channel(parts[2])?,
            (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
        ]);
    }
    Some(match text.as_str() {
        "black" => [0, 0, 0, 255],
        "white" => [255, 255, 255, 255],
        "red" => [255, 0, 0, 255],
        "green" => [0, 128, 0, 255],
        "lime" => [0, 255, 0, 255],
        "blue" => [0, 0, 255, 255],
        "yellow" => [255, 255, 0, 255],
        "orange" => [255, 165, 0, 255],
        "purple" => [128, 0, 128, 255],
        "gray" | "grey" => [128, 128, 128, 255],
        "transparent" => [0, 0, 0, 0],
        _ => return None,
    })
}

// ---- QR code ----------------------------------------------------------------------

/// `renderQrCodeBuffer`: `QRCode.toBuffer(text, { width: size, margin: 1 })`
/// (error correction M, no ECC boost), plus the centered logo.
fn qr_code_pixmap(
    text: &str,
    size: u32,
    color: &str,
    logo: Option<&Pixmap>,
) -> Result<Pixmap, RenderError> {
    use qrcodegen::{QrCode, QrCodeEcc, QrSegment, Version};
    let segments = QrSegment::make_segments(text);
    let qr = QrCode::encode_segments_advanced(
        &segments,
        QrCodeEcc::Medium,
        Version::MIN,
        Version::MAX,
        None,
        false,
    )
    .map_err(|_| err("The amount of data is too big to be stored in a QR Code"))?;
    let modules = qr.size() as u32;
    const MARGIN: u32 = 1;
    let span = modules + 2 * MARGIN;
    // `getScale`: `width / (size + 2 * margin)` when it fits, else scale 4.
    let scale = if size >= span {
        f64::from(size) / f64::from(span)
    } else {
        4.0
    };
    let symbol = (f64::from(span) * scale).floor() as u32;
    let margin = f64::from(MARGIN) * scale;
    let dark = parse_color(color).unwrap_or([0, 0, 0, 255]);
    let mut qr_pixmap = Pixmap::new(symbol, symbol).ok_or_else(|| err("Invalid QR size"))?;
    let light = tiny_skia::ColorU8::from_rgba(255, 255, 255, 255).premultiply();
    let dark = tiny_skia::ColorU8::from_rgba(dark[0], dark[1], dark[2], dark[3]).premultiply();
    let limit = f64::from(symbol) - margin;
    for (index, px) in qr_pixmap.pixels_mut().iter_mut().enumerate() {
        let (i, j) = (
            (index as u32 / symbol) as f64,
            (index as u32 % symbol) as f64,
        );
        *px = if i >= margin && j >= margin && i < limit && j < limit {
            let row = ((i - margin) / scale).floor() as i32;
            let col = ((j - margin) / scale).floor() as i32;
            if qr.get_module(col, row) { dark } else { light }
        } else {
            light
        };
    }
    let Some(logo) = logo else {
        return Ok(qr_pixmap);
    };
    let mut canvas = Pixmap::new(size, size).ok_or_else(|| err("Invalid QR size"))?;
    let side = size as f32;
    let identity = Transform::identity();
    let stretch = |canvas: &mut Pixmap, src: &Pixmap, x: f32, y: f32, w: f32| {
        if let Some(rect) = Rect::from_xywh(x, y, w, w) {
            let paint = Paint {
                shader: Pattern::new(
                    src.as_ref(),
                    SpreadMode::Pad,
                    FilterQuality::Bicubic,
                    1.0,
                    Transform::from_row(
                        w / src.width() as f32,
                        0.0,
                        0.0,
                        w / src.height() as f32,
                        x,
                        y,
                    ),
                ),
                anti_alias: true,
                ..Paint::default()
            };
            canvas.fill_rect(rect, &paint, identity, None);
        }
    };
    stretch(&mut canvas, &qr_pixmap, 0.0, 0.0, side);
    let logo_size = side * LOGO_SIZE_RATIO;
    let at = (side - logo_size) / 2.0;
    let padding = logo_size * LOGO_PADDING_RATIO;
    if let Some(rect) = Rect::from_xywh(
        at - padding,
        at - padding,
        logo_size + padding * 2.0,
        logo_size + padding * 2.0,
    ) {
        let mut white = Paint::default();
        white.set_color_rgba8(255, 255, 255, 255);
        canvas.fill_rect(rect, &white, identity, None);
    }
    stretch(&mut canvas, logo, at, at, logo_size);
    Ok(canvas)
}

// ---- text -------------------------------------------------------------------------

const ROBOTO_LATIN_400: &[u8] =
    include_bytes!("../../assets/dynamic-image-fonts/roboto_latin_400_normal.ttf");
const ROBOTO_VI_400: &[u8] =
    include_bytes!("../../assets/dynamic-image-fonts/roboto_vietnamese_400_normal.ttf");
const ROBOTO_LATIN_700: &[u8] =
    include_bytes!("../../assets/dynamic-image-fonts/roboto_latin_700_normal.ttf");
const ROBOTO_VI_700: &[u8] =
    include_bytes!("../../assets/dynamic-image-fonts/roboto_vietnamese_700_normal.ttf");
const ROBOTO_LATIN_ITALIC: &[u8] =
    include_bytes!("../../assets/dynamic-image-fonts/roboto_latin_400_italic.ttf");
const ROBOTO_VI_ITALIC: &[u8] =
    include_bytes!("../../assets/dynamic-image-fonts/roboto_vietnamese_400_italic.ttf");
const SERIF_LATIN_400: &[u8] =
    include_bytes!("../../assets/dynamic-image-fonts/noto_serif_latin_400_normal.ttf");
const SERIF_VI_400: &[u8] =
    include_bytes!("../../assets/dynamic-image-fonts/noto_serif_vietnamese_400_normal.ttf");
const SERIF_LATIN_700: &[u8] =
    include_bytes!("../../assets/dynamic-image-fonts/noto_serif_latin_700_normal.ttf");
const SERIF_VI_700: &[u8] =
    include_bytes!("../../assets/dynamic-image-fonts/noto_serif_vietnamese_700_normal.ttf");
const SERIF_LATIN_ITALIC: &[u8] =
    include_bytes!("../../assets/dynamic-image-fonts/noto_serif_latin_400_italic.ttf");
const SERIF_VI_ITALIC: &[u8] =
    include_bytes!("../../assets/dynamic-image-fonts/noto_serif_vietnamese_400_italic.ttf");
const SCRIPT_LATIN: &[u8] =
    include_bytes!("../../assets/dynamic-image-fonts/great_vibes_latin_400_normal.ttf");
const SCRIPT_VI: &[u8] =
    include_bytes!("../../assets/dynamic-image-fonts/great_vibes_vietnamese_400_normal.ttf");

/// `resolveDynamicImageFontFamily` + the face Skia picks for bold / italic:
/// the latin and the vietnamese subset of one face.
fn face_files(family: &str, bold: bool, italic: bool) -> [&'static [u8]; 2] {
    match family {
        "serif" if bold => [SERIF_LATIN_700, SERIF_VI_700],
        "serif" if italic => [SERIF_LATIN_ITALIC, SERIF_VI_ITALIC],
        "serif" => [SERIF_LATIN_400, SERIF_VI_400],
        "greatVibes" => [SCRIPT_LATIN, SCRIPT_VI],
        _ if bold => [ROBOTO_LATIN_700, ROBOTO_VI_700],
        _ if italic => [ROBOTO_LATIN_ITALIC, ROBOTO_VI_ITALIC],
        _ => [ROBOTO_LATIN_400, ROBOTO_VI_400],
    }
}

struct Font {
    faces: Vec<ttf_parser::Face<'static>>,
    size: f32,
}

impl Font {
    fn new(family: &str, bold: bool, italic: bool, size: f32) -> Font {
        Font {
            faces: face_files(family, bold, italic)
                .iter()
                .filter_map(|bytes| ttf_parser::Face::parse(bytes, 0).ok())
                .collect(),
            size,
        }
    }

    /// The face that has `ch` (latin first), else the first face's `.notdef`.
    fn glyph(&self, ch: char) -> Option<(&ttf_parser::Face<'static>, ttf_parser::GlyphId)> {
        self.faces
            .iter()
            .find_map(|face| face.glyph_index(ch).map(|id| (face, id)))
            .or_else(|| {
                self.faces
                    .first()
                    .map(|face| (face, ttf_parser::GlyphId(0)))
            })
    }

    fn scale(face: &ttf_parser::Face<'_>, size: f32) -> f32 {
        size / f32::from(face.units_per_em().max(1))
    }

    /// `ctx.measureText(text).width`.
    fn measure(&self, text: &str) -> f32 {
        text.chars()
            .filter_map(|ch| self.glyph(ch))
            .map(|(face, id)| {
                f32::from(face.glyph_hor_advance(id).unwrap_or(0)) * Self::scale(face, self.size)
            })
            .sum()
    }

    /// The outline of `text` with its em box top-left at `(left, top)`.
    fn path(&self, text: &str, left: f32, top: f32) -> Option<tiny_skia::Path> {
        let first = self.faces.first()?;
        let baseline = top + f32::from(first.ascender()) * Self::scale(first, self.size);
        let mut builder = Outline {
            path: PathBuilder::new(),
            x: left,
            y: baseline,
            scale: 1.0,
        };
        for ch in text.chars() {
            let Some((face, id)) = self.glyph(ch) else {
                continue;
            };
            builder.scale = Self::scale(face, self.size);
            face.outline_glyph(id, &mut builder);
            builder.x += f32::from(face.glyph_hor_advance(id).unwrap_or(0)) * builder.scale;
        }
        builder.path.finish()
    }
}

struct Outline {
    path: PathBuilder,
    x: f32,
    y: f32,
    scale: f32,
}

impl Outline {
    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        (self.x + x * self.scale, self.y - y * self.scale)
    }
}

impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.at(x, y);
        self.path.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.at(x, y);
        self.path.line_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x1, y1) = self.at(x1, y1);
        let (x, y) = self.at(x, y);
        self.path.quad_to(x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x1, y1) = self.at(x1, y1);
        let (x2, y2) = self.at(x2, y2);
        let (x, y) = self.at(x, y);
        self.path.cubic_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.path.close();
    }
}

/// `wrapText`: greedy word wrap on spaces.
fn wrap_text(font: &Font, text: &str, max_width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split(' ') {
        let candidate = if current.is_empty() {
            word.to_owned()
        } else {
            format!("{current} {word}")
        };
        if !current.is_empty() && font.measure(&candidate) > max_width {
            lines.push(std::mem::replace(&mut current, word.to_owned()));
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(elements: serde_json::Value) -> Document {
        serde_json::from_value(
            serde_json::json!({ "width": 40, "height": 30, "elements": elements }),
        )
        .unwrap()
    }

    #[test]
    fn variables_and_dynamic_ids() {
        assert!(element_has_variable("Hi {{first_name}}"));
        assert!(!element_has_variable("Hi {{}}"));
        assert!(!element_has_variable("Hi {first}"));
        let d = doc(serde_json::json!([
            { "id": "a", "type": "text", "text": "x", "x": 0, "y": 0, "width": 1, "height": 1,
              "fontSize": 10, "fontFamily": "arial", "align": "left", "color": "#000" },
            { "id": "b", "type": "image", "imageType": "avatarUser", "x": 0, "y": 0, "width": 1, "height": 1 },
            { "id": "c", "type": "qrCode", "text": "q", "x": 0, "y": 0, "width": 1, "height": 1, "size": 64, "color": "#000" },
        ]));
        let ids = dynamic_element_ids(&d);
        assert_eq!(ids, ["b", "c"].iter().map(|s| s.to_string()).collect());
    }

    #[test]
    fn colors_and_contain() {
        assert_eq!(parse_color("#111"), Some([17, 17, 17, 255]));
        assert_eq!(parse_color("#ff000080"), Some([255, 0, 0, 128]));
        assert_eq!(parse_color("rgba(0, 0, 255, 0.5)"), Some([0, 0, 255, 128]));
        assert_eq!(parse_color("nope"), None);
        assert_eq!(
            contain_rect(4.0, 2.0, 0.0, 0.0, 10.0, 10.0),
            (0.0, 2.5, 10.0, 5.0)
        );
    }

    #[test]
    fn renders_text_and_qr_at_twice_the_size() {
        let d = doc(serde_json::json!([
            { "id": "t", "type": "text", "text": "Hello world", "x": 0, "y": 0, "width": 40, "height": 12,
              "fontSize": 10, "fontFamily": "roboto", "align": "center", "color": "#111111" },
            { "id": "q", "type": "qrCode", "text": "parity", "x": 10, "y": 12, "width": 16, "height": 16, "size": 64, "color": "#000000" },
        ]));
        let png = render_static_layer(&d, &Images::new()).unwrap();
        let image = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(image.dimensions(), (80, 60));
        // Some text pixels are drawn; the corner below the text is empty.
        assert!(image.pixels().take(80 * 24).any(|p| p.0[3] > 0));
        assert_eq!(image.get_pixel(2, 58).0, [0, 0, 0, 0]);
        // The QR quiet zone is white.
        assert_eq!(image.get_pixel(21, 25).0, [255, 255, 255, 255]);
    }
}
