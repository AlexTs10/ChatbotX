//! Image fixtures for the ported sharp-based tests: the original builds its
//! inputs with sharp (gaussian noise, JPEG/PNG/GIF encodes); these helpers
//! build equivalent inputs with `image` and `jpeg-encoder`.

#![allow(dead_code)]

use std::io::Cursor;

use image::codecs::gif::GifEncoder;
use image::codecs::png::PngEncoder;
use image::{Delay, Frame, ImageEncoder, RgbaImage};

/// A deterministic xorshift PRNG.
struct Rng(u64);

impl Rng {
    fn next_f64(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Box-Muller.
    fn gaussian(&mut self, mean: f64, sigma: f64) -> f64 {
        let u1 = self.next_f64().max(f64::MIN_POSITIVE);
        let u2 = self.next_f64();
        mean + sigma * (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }
}

/// `sharp({ create: { noise: { type: "gaussian", mean: 128, sigma: 40 } } })`
/// raw pixels, `channels` 3 or 4.
pub fn noise(width: u32, height: u32, channels: usize) -> Vec<u8> {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ (u64::from(width) << 32) ^ u64::from(height));
    (0..width as usize * height as usize * channels)
        .map(|_| rng.gaussian(128.0, 40.0).round().clamp(0.0, 255.0) as u8)
        .collect()
}

/// RGB raw → JPEG (`.jpeg({ quality })`: 4:2:0, optimized Huffman tables).
pub fn jpeg(rgb: &[u8], width: u32, height: u32, quality: u8) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = jpeg_encoder::Encoder::new(&mut out, quality);
    encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_2);
    encoder.set_optimized_huffman_tables(true);
    encoder
        .encode(
            rgb,
            width as u16,
            height as u16,
            jpeg_encoder::ColorType::Rgb,
        )
        .expect("jpeg encode");
    out
}

/// A noisy RGB JPEG.
pub fn noisy_jpeg(width: u32, height: u32, quality: u8) -> Vec<u8> {
    jpeg(&noise(width, height, 3), width, height, quality)
}

/// A flat RGB JPEG (`sharp({ create: { background } }).jpeg()`, quality 80).
pub fn flat_jpeg(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    let pixels: Vec<u8> = (0..width as usize * height as usize)
        .flat_map(|_| rgb)
        .collect();
    jpeg(&pixels, width, height, 80)
}

/// Raw pixels → PNG.
pub fn png(pixels: &[u8], width: u32, height: u32, channels: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let color = if channels == 4 {
        image::ExtendedColorType::Rgba8
    } else {
        image::ExtendedColorType::Rgb8
    };
    PngEncoder::new(&mut out)
        .write_image(pixels, width, height, color)
        .expect("png encode");
    out
}

/// A 2-frame animated GIF of `frame` (RGBA).
pub fn animated_gif(frame: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = GifEncoder::new(&mut out);
        let image = RgbaImage::from_raw(width, height, frame.to_vec()).expect("frame");
        let frames = vec![
            Frame::from_parts(image.clone(), 0, 0, Delay::from_numer_denom_ms(100, 1)),
            Frame::from_parts(image, 0, 0, Delay::from_numer_denom_ms(100, 1)),
        ];
        encoder.encode_frames(frames).expect("gif encode");
    }
    out
}

/// `.withMetadata({ orientation })`: an APP1 Exif segment with the
/// Orientation tag, inserted after SOI.
pub fn with_exif_orientation(jpeg: &[u8], orientation: u16) -> Vec<u8> {
    let mut tiff = Vec::new();
    tiff.extend_from_slice(b"MM\x00\x2a\x00\x00\x00\x08");
    tiff.extend_from_slice(&1u16.to_be_bytes());
    tiff.extend_from_slice(&0x0112u16.to_be_bytes());
    tiff.extend_from_slice(&3u16.to_be_bytes());
    tiff.extend_from_slice(&1u32.to_be_bytes());
    tiff.extend_from_slice(&orientation.to_be_bytes());
    tiff.extend_from_slice(&[0, 0]);
    tiff.extend_from_slice(&0u32.to_be_bytes());
    let mut payload = b"Exif\x00\x00".to_vec();
    payload.extend_from_slice(&tiff);
    let mut out = Vec::with_capacity(jpeg.len() + payload.len() + 4);
    out.extend_from_slice(&jpeg[..2]);
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(&payload);
    out.extend_from_slice(&jpeg[2..]);
    out
}

/// zlib `crc32`.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for b in bytes {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// The width and height of an encoded image.
pub fn dimensions(bytes: &[u8]) -> (u32, u32) {
    image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .unwrap()
        .into_dimensions()
        .unwrap()
}
