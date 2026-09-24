use brushkit_abr::TipBitmap;
use image::{ColorType, ImageBuffer, ImageDecoder, Rgba, RgbaImage};
use std::io::Cursor;
use zune_core::bytestream::{ZByteIoError, ZByteReaderTrait, ZCursor, ZSeekFrom};
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_jpeg::ImageInfo;

#[derive(Debug, Clone)]
pub struct GrayscaleBitmap {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

pub fn to_grayscale(tip: &TipBitmap) -> GrayscaleBitmap {
    match tip.depth {
        8 => GrayscaleBitmap {
            width: tip.width,
            height: tip.height,
            data: tip.data.clone(),
        },
        16 => {
            let data: Vec<u8> = tip
                .data
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| pair[0])
                .collect();
            GrayscaleBitmap {
                width: tip.width,
                height: tip.height,
                data,
            }
        }
        _ => GrayscaleBitmap {
            width: tip.width,
            height: tip.height,
            data: tip.data.clone(),
        },
    }
}

/// Both formats use the same polarity — a `Shape.png` is white=stamp coverage
/// and a `TipBitmap` is 255=full ink — so the pixels are copied unchanged.
pub fn tip_bitmap_of(bitmap: &GrayscaleBitmap) -> TipBitmap {
    TipBitmap {
        width: bitmap.width,
        height: bitmap.height,
        depth: 8,
        data: bitmap.data.clone(),
    }
}

pub const MAX_IMPORT_DIMENSION: u32 = 16384;
/// The decode budget of [`decode_tip_image`]. [`TipImageError::TooLarge`] lists
/// what it reserves.
pub const MAX_IMPORT_DECODED_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug)]
pub enum TipImageError {
    /// The bytes did not decode, or a PNG carries an eXIf chunk over 64 KiB
    /// before the image data.
    Decode(String),
    /// A side over [`MAX_IMPORT_DIMENSION`], or a decode over
    /// [`MAX_IMPORT_DECODED_BYTES`]: the image in its own pixel format, plus
    /// the DCT coefficients of a progressive JPEG or of a baseline JPEG whose
    /// first scan leaves out a component. The decoder's other buffers are not
    /// counted: up to a few hundred KiB at any width, more for wide images,
    /// whose row buffers grow with the width. Neither is a PNG's eXIf chunk of
    /// at most 64 KiB, which the decoder holds twice, up to 128 KiB. A JPEG's
    /// metadata segments cost nothing: zune-jpeg skips them without a copy.
    TooLarge { width: u32, height: u32 },
}

impl std::fmt::Display for TipImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TipImageError::Decode(msg) => f.write_str(msg),
            TipImageError::TooLarge { width, height } => write!(
                f,
                "image is {width}x{height}px; a brush tip must be at most {MAX_IMPORT_DIMENSION}px per side and {} MiB to decode",
                MAX_IMPORT_DECODED_BYTES / (1024 * 1024)
            ),
        }
    }
}

impl std::error::Error for TipImageError {}

/// Decode an imported brush-tip image (PNG/JPEG bytes) into a grayscale tip
/// buffer. Alpha becomes intensity when the image has an alpha channel;
/// otherwise luminance is inverted (dark = opaque), matching Photoshop's
/// convention.
pub fn decode_tip_image(bytes: &[u8]) -> Result<GrayscaleBitmap, TipImageError> {
    let img = decode_guarded(bytes, MAX_IMPORT_DIMENSION, MAX_IMPORT_DECODED_BYTES).map_err(
        |e| match e {
            GuardedDecodeError::TooLarge { width, height } => {
                TipImageError::TooLarge { width, height }
            }
            GuardedDecodeError::Decode(e) => TipImageError::Decode(e.to_string()),
        },
    )?;
    Ok(GrayscaleBitmap {
        width: img.width,
        height: img.height,
        data: tip_plane(img, TipSample::Coverage),
    })
}

/// A decoded image in its own pixel format. 16-bit and float samples are in
/// native byte order, as `ImageDecoder::read_image` writes them.
pub(crate) struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub bytes: Vec<u8>,
}

/// Every pixel format `image` 0.25 decodes to. The float formats come from
/// decoders a dependent crate may enable on the shared `image` dependency.
#[derive(Clone, Copy)]
pub(crate) enum PixelFormat {
    L8,
    La8,
    Rgb8,
    Rgba8,
    L16,
    La16,
    Rgb16,
    Rgba16,
    Rgb32F,
    Rgba32F,
}

impl PixelFormat {
    fn of(color: ColorType) -> Option<Self> {
        Some(match color {
            ColorType::L8 => Self::L8,
            ColorType::La8 => Self::La8,
            ColorType::Rgb8 => Self::Rgb8,
            ColorType::Rgba8 => Self::Rgba8,
            ColorType::L16 => Self::L16,
            ColorType::La16 => Self::La16,
            ColorType::Rgb16 => Self::Rgb16,
            ColorType::Rgba16 => Self::Rgba16,
            ColorType::Rgb32F => Self::Rgb32F,
            ColorType::Rgba32F => Self::Rgba32F,
            _ => return None,
        })
    }

    fn color_type(self) -> ColorType {
        match self {
            Self::L8 => ColorType::L8,
            Self::La8 => ColorType::La8,
            Self::Rgb8 => ColorType::Rgb8,
            Self::Rgba8 => ColorType::Rgba8,
            Self::L16 => ColorType::L16,
            Self::La16 => ColorType::La16,
            Self::Rgb16 => ColorType::Rgb16,
            Self::Rgba16 => ColorType::Rgba16,
            Self::Rgb32F => ColorType::Rgb32F,
            Self::Rgba32F => ColorType::Rgba32F,
        }
    }

    fn bytes_per_pixel(self) -> usize {
        usize::from(self.color_type().bytes_per_pixel())
    }
}

/// What a tip keeps of each pixel.
#[derive(Clone, Copy)]
pub(crate) enum TipSample {
    /// Alpha when the image has it, otherwise inverted luma, so dark is opaque.
    Coverage,
    Luma,
}

/// One byte per pixel, written over the front of the decoded buffer so the tip
/// allocates no second buffer. Values are `image`'s own `into_luma8` and
/// `into_luma_alpha8` conversions. The final shrink is a `realloc`, measured in
/// place from 16 to 512 MiB on glibc and both wasm32 targets, and on macOS
/// except a 16 MiB block shrunk to 2 MiB. An allocator that moves the block
/// adds the tip's size to the peak.
pub(crate) fn tip_plane(image: DecodedImage, sample: TipSample) -> Vec<u8> {
    const CHUNK: usize = 4096;
    let format = image.format;
    let bytes_per_pixel = format.bytes_per_pixel();
    let coverage = matches!(sample, TipSample::Coverage);
    let alpha = coverage && format.color_type().has_alpha();
    let mut raw = image.bytes;
    let pixels = raw.len() / bytes_per_pixel;
    if alpha && matches!(format, PixelFormat::La8 | PixelFormat::Rgba8) {
        for i in 0..pixels {
            raw[i] = raw[i * bytes_per_pixel + bytes_per_pixel - 1];
        }
    } else if bytes_per_pixel > 1 {
        // Chunk `start..end` is written to bytes `start..end`, which precede
        // its own source bytes, so no unread pixel is overwritten.
        for start in (0..pixels).step_by(CHUNK) {
            let end = (start + CHUNK).min(pixels);
            let chunk = &raw[start * bytes_per_pixel..end * bytes_per_pixel];
            let plane = convert_chunk(format, alpha, chunk);
            raw[start..end].copy_from_slice(&plane);
        }
    }
    raw.truncate(pixels);
    raw.shrink_to_fit();
    if coverage && !alpha {
        raw.iter_mut().for_each(|v| *v = 255 - *v);
    }
    raw
}

/// One byte per pixel of `chunk`, alpha when `alpha` and luma otherwise,
/// through a one-row image so `image` does the conversion.
fn convert_chunk(format: PixelFormat, alpha: bool, chunk: &[u8]) -> Vec<u8> {
    use image::DynamicImage::*;
    let width = (chunk.len() / format.bytes_per_pixel()) as u32;
    let u8s = || chunk.to_vec();
    let u16s = || {
        let samples = chunk.as_chunks::<2>().0.iter();
        samples.map(|s| u16::from_ne_bytes(*s)).collect()
    };
    let f32s = || {
        let samples = chunk.as_chunks::<4>().0.iter();
        samples.map(|s| f32::from_ne_bytes(*s)).collect()
    };
    let pixels = match format {
        PixelFormat::L8 => ImageBuffer::from_raw(width, 1, u8s()).map(ImageLuma8),
        PixelFormat::La8 => ImageBuffer::from_raw(width, 1, u8s()).map(ImageLumaA8),
        PixelFormat::Rgb8 => ImageBuffer::from_raw(width, 1, u8s()).map(ImageRgb8),
        PixelFormat::Rgba8 => ImageBuffer::from_raw(width, 1, u8s()).map(ImageRgba8),
        PixelFormat::L16 => ImageBuffer::from_raw(width, 1, u16s()).map(ImageLuma16),
        PixelFormat::La16 => ImageBuffer::from_raw(width, 1, u16s()).map(ImageLumaA16),
        PixelFormat::Rgb16 => ImageBuffer::from_raw(width, 1, u16s()).map(ImageRgb16),
        PixelFormat::Rgba16 => ImageBuffer::from_raw(width, 1, u16s()).map(ImageRgba16),
        PixelFormat::Rgb32F => ImageBuffer::from_raw(width, 1, f32s()).map(ImageRgb32F),
        PixelFormat::Rgba32F => ImageBuffer::from_raw(width, 1, f32s()).map(ImageRgba32F),
    }
    .expect("a chunk holds whole pixels");
    if alpha {
        let luma_alpha = pixels.into_luma_alpha8().into_raw();
        luma_alpha.into_iter().skip(1).step_by(2).collect()
    } else {
        pixels.into_luma8().into_raw()
    }
}

/// Why `decode_guarded` returned no image.
pub(crate) enum GuardedDecodeError {
    /// A side over `max_side`, or a decode over `max_bytes`.
    TooLarge {
        width: u32,
        height: u32,
    },
    Decode(image::ImageError),
}

/// Decode an image of at most `max_side` px per side and `max_bytes` decoded:
/// the image in its own pixel format, plus the DCT coefficients zune-jpeg holds
/// for the whole image, counted by `coefficient_bytes`. The decoder's other
/// buffers are not counted, as [`TipImageError::TooLarge`] describes. Oversize
/// is decided from the header before any pixels are decoded. A JPEG is decoded
/// from `bytes`, and its metadata segments are skipped. A PNG's iCCP
/// profile and text chunks are skipped, and one with an eXIf chunk over
/// [`MAX_PNG_EXIF_BYTES`] before the image data does not decode.
pub(crate) fn decode_guarded(
    bytes: &[u8],
    max_side: u32,
    max_bytes: u64,
) -> Result<DecodedImage, GuardedDecodeError> {
    match image::guess_format(bytes).ok() {
        Some(image::ImageFormat::Jpeg) => return decode_jpeg(bytes, max_side, max_bytes),
        Some(image::ImageFormat::Png) => return decode_png(bytes, max_side, max_bytes),
        _ => {}
    }
    if let Some((width, height)) = header_dimensions(bytes) {
        if width > max_side || height > max_side {
            return Err(GuardedDecodeError::TooLarge { width, height });
        }
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| GuardedDecodeError::Decode(image::ImageError::IoError(e)))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(max_side);
    limits.max_image_height = Some(max_side);
    limits.max_alloc = Some(max_bytes);
    reader.limits(limits.clone());
    let mut decoder = reader.into_decoder().map_err(GuardedDecodeError::Decode)?;
    // What `ImageReader::decode` does, with the budget failure reported as
    // `TooLarge`: the output buffer is reserved and the decoder keeps the rest.
    if limits.reserve(decoder.total_bytes()).is_err() {
        let (width, height) = decoder.dimensions();
        return Err(GuardedDecodeError::TooLarge { width, height });
    }
    decoder
        .set_limits(limits)
        .map_err(GuardedDecodeError::Decode)?;
    let (width, height) = decoder.dimensions();
    let color = decoder.color_type();
    let format = PixelFormat::of(color)
        .ok_or_else(|| unsupported_color(image::error::ImageFormatHint::Unknown, color.into()))?;
    // One buffer in the decoder's own format, so a converting tip reuses it.
    let total_bytes = usize::try_from(decoder.total_bytes())
        .map_err(|_| GuardedDecodeError::TooLarge { width, height })?;
    let mut bytes = vec![0; total_bytes];
    decoder
        .read_image(&mut bytes)
        .map_err(GuardedDecodeError::Decode)?;
    Ok(DecodedImage {
        width,
        height,
        format,
        bytes,
    })
}

fn unsupported_color(
    format: image::error::ImageFormatHint,
    color: image::ExtendedColorType,
) -> GuardedDecodeError {
    GuardedDecodeError::Decode(image::ImageError::Unsupported(
        image::error::UnsupportedError::from_format_and_kind(
            format,
            image::error::UnsupportedErrorKind::Color(color),
        ),
    ))
}

/// The largest eXIf chunk a PNG may carry for its tip to decode: what a JPEG's
/// APP1 segment holds.
const MAX_PNG_EXIF_BYTES: usize = 64 * 1024;

/// The length of the first eXIf chunk before the image data that is over
/// [`MAX_PNG_EXIF_BYTES`]. `png` buffers a chunk's declared length.
fn oversize_exif(bytes: &[u8]) -> Option<u32> {
    // Chunks follow the 8-byte signature.
    let mut at = 8usize;
    while let Some(&[l0, l1, l2, l3, ref kind @ ..]) =
        bytes.get(at..).and_then(|rest| rest.get(..8))
    {
        let len = u32::from_be_bytes([l0, l1, l2, l3]);
        match kind {
            b"IDAT" => return None,
            b"eXIf" if len as usize > MAX_PNG_EXIF_BYTES => return Some(len),
            _ => {}
        }
        // Length, type, data and CRC.
        at = at.saturating_add(12).saturating_add(len as usize);
    }
    None
}

/// `decode_guarded` for a PNG. Through `image`'s PNG decoder, `png` inflates
/// an iCCP profile up to the whole budget before the output buffer is
/// reserved, and keeps it while the image decodes. Text chunks are kept the
/// same way. The tip reads neither, so `png` decodes here with both skipped.
/// `png` 0.18.1 keeps an eXIf chunk whatever its options, in its chunk buffer
/// and in a copy, so a PNG with one over [`MAX_PNG_EXIF_BYTES`] fails before
/// `png` reads it. The transformation, output formats and errors are those of
/// `image` 0.25.
fn decode_png(
    bytes: &[u8],
    max_side: u32,
    max_bytes: u64,
) -> Result<DecodedImage, GuardedDecodeError> {
    use png::{BitDepth, ColorType as Png};
    let limits = png::Limits {
        bytes: usize::try_from(max_bytes).unwrap_or(usize::MAX),
    };
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(bytes), limits);
    decoder.set_ignore_iccp_chunk(true);
    decoder.set_ignore_text_chunk(true);
    decoder.set_transformations(png::Transformations::EXPAND);
    let info = decoder.read_header_info().map_err(png_error)?;
    let (width, height) = (info.width, info.height);
    if width > max_side || height > max_side {
        return Err(GuardedDecodeError::TooLarge { width, height });
    }
    // On 32-bit targets `png` rejects an output buffer over `isize::MAX` in
    // `read_info`, so an over-budget PNG is sized from IHDR first. IHDR
    // undercounts indexed color, which EXPAND turns into RGB or RGBA.
    let min_decoded = u64::from(width) * u64::from(height) * info.bytes_per_pixel() as u64;
    if min_decoded > max_bytes {
        return Err(GuardedDecodeError::TooLarge { width, height });
    }
    if let Some(len) = oversize_exif(bytes) {
        return Err(GuardedDecodeError::Decode(image::ImageError::Decoding(
            image::error::DecodingError::new(
                image::ImageFormat::Png.into(),
                format!("eXIf chunk is {len} bytes, over the {MAX_PNG_EXIF_BYTES} a tip reads"),
            ),
        )));
    }
    let mut reader = decoder.read_info().map_err(png_error)?;
    let (color, depth) = reader.output_color_type();
    let format = match (color, depth) {
        (Png::Grayscale, BitDepth::Eight) => PixelFormat::L8,
        (Png::GrayscaleAlpha, BitDepth::Eight) => PixelFormat::La8,
        (Png::Rgb, BitDepth::Eight) => PixelFormat::Rgb8,
        (Png::Rgba, BitDepth::Eight) => PixelFormat::Rgba8,
        (Png::Grayscale, BitDepth::Sixteen) => PixelFormat::L16,
        (Png::GrayscaleAlpha, BitDepth::Sixteen) => PixelFormat::La16,
        (Png::Rgb, BitDepth::Sixteen) => PixelFormat::Rgb16,
        (Png::Rgba, BitDepth::Sixteen) => PixelFormat::Rgba16,
        // EXPAND widens every other pair to one of the above.
        (_, bits) => {
            return Err(unsupported_color(
                image::ImageFormat::Png.into(),
                image::ExtendedColorType::Unknown(bits as u8),
            ))
        }
    };
    let total_bytes = u64::from(width) * u64::from(height) * format.bytes_per_pixel() as u64;
    if total_bytes > max_bytes {
        return Err(GuardedDecodeError::TooLarge { width, height });
    }
    let total_bytes =
        usize::try_from(total_bytes).map_err(|_| GuardedDecodeError::TooLarge { width, height })?;
    let mut bytes = vec![0; total_bytes];
    reader.next_frame(&mut bytes).map_err(png_error)?;
    if depth == BitDepth::Sixteen {
        for sample in bytes.as_chunks_mut::<2>().0 {
            *sample = u16::from_be_bytes(*sample).to_ne_bytes();
        }
    }
    Ok(DecodedImage {
        width,
        height,
        format,
        bytes,
    })
}

/// `image`'s own mapping of a `png` error.
fn png_error(err: png::DecodingError) -> GuardedDecodeError {
    use image::error::{
        DecodingError, ImageFormatHint, LimitError, LimitErrorKind, ParameterError,
        ParameterErrorKind,
    };
    GuardedDecodeError::Decode(match err {
        png::DecodingError::IoError(err) => image::ImageError::IoError(err),
        err @ png::DecodingError::Format(_) => image::ImageError::Decoding(DecodingError::new(
            ImageFormatHint::Exact(image::ImageFormat::Png),
            err,
        )),
        err @ png::DecodingError::Parameter(_) => image::ImageError::Parameter(
            ParameterError::from_kind(ParameterErrorKind::Generic(err.to_string())),
        ),
        png::DecodingError::LimitsExceeded => {
            image::ImageError::Limits(LimitError::from_kind(LimitErrorKind::InsufficientMemory))
        }
    })
}

/// `decode_guarded` for a JPEG. `image`'s JPEG decoder copies its whole input
/// before it decodes, so zune-jpeg decodes the borrowed bytes here, with the
/// options and output color `image` uses.
fn decode_jpeg(
    bytes: &[u8],
    max_side: u32,
    max_bytes: u64,
) -> Result<DecodedImage, GuardedDecodeError> {
    let JpegHeader {
        width,
        height,
        input_color,
        coefficient_bytes,
    } = jpeg_header(bytes).map_err(jpeg_error)?;
    if width > max_side || height > max_side {
        return Err(GuardedDecodeError::TooLarge { width, height });
    }
    // `image` decodes a color space it has no pixel format for to RGB.
    let (output_color, format) = match input_color {
        ColorSpace::Luma => (ColorSpace::Luma, PixelFormat::L8),
        ColorSpace::LumaA => (ColorSpace::LumaA, PixelFormat::La8),
        ColorSpace::RGBA => (ColorSpace::RGBA, PixelFormat::Rgba8),
        _ => (ColorSpace::RGB, PixelFormat::Rgb8),
    };
    let total_bytes = u64::from(width) * u64::from(height) * output_color.num_components() as u64;
    if total_bytes.saturating_add(coefficient_bytes) > max_bytes {
        return Err(GuardedDecodeError::TooLarge { width, height });
    }
    let total_bytes =
        usize::try_from(total_bytes).map_err(|_| GuardedDecodeError::TooLarge { width, height })?;
    let mut pixels = vec![0; total_bytes];
    // A second decoder, because zune-jpeg picks its color conversion while it
    // reads the headers.
    jpeg_decoder(ZCursor::new(bytes), output_color)
        .decode_into(&mut pixels)
        .map_err(jpeg_error)?;
    Ok(DecodedImage {
        width,
        height,
        format,
        bytes: pixels,
    })
}

/// zune-jpeg over `reader`, with the options `image` decodes with.
fn jpeg_decoder<R: ZByteReaderTrait>(
    reader: R,
    output_color: ColorSpace,
) -> zune_jpeg::JpegDecoder<HideMetadata<R>> {
    let options = DecoderOptions::default()
        .jpeg_set_out_colorspace(output_color)
        .set_strict_mode(false)
        .set_max_width(usize::MAX)
        .set_max_height(usize::MAX);
    zune_jpeg::JpegDecoder::new_with_options(HideMetadata(reader), options)
}

/// `reader` with every metadata segment zune-jpeg keeps a copy of hidden from
/// it: EXIF, XMP, extended XMP, ICC, gain map, MPF and IPTC. The tip reads
/// none of them. zune-jpeg keeps a list entry per ICC and gain-map segment,
/// which reaches five times the input for many small ones, and before the
/// first scan it sorts and walks the extended XMP parts it holds after every
/// marker, so parts that never complete make the header parse quadratic.
/// zune-jpeg recognizes each segment by peeking its identifier, so this reader
/// changes that peek and zune-jpeg skips the segment as an unknown one. A
/// segment zune-jpeg would reject, too short for an extended XMP part's
/// 40-byte header or running past the end of the input, stays visible, so
/// zune-jpeg rejects it as `image` does.
struct HideMetadata<R>(R);

/// The identifier of each segment zune-jpeg keeps, and the bytes it requires
/// after the identifier. zune-jpeg peeks each identifier length in one parser
/// only, so matching the identifier alone is exact.
const KEPT_SEGMENTS: &[(&[u8], u64)] = &[
    (b"Exif\0\0", 0),
    (b"http://ns.adobe.com/xap/1.0/\0", 0),
    (b"http://ns.adobe.com/xmp/extension/\0", 40),
    (b"ICC_PROFILE\0", 0),
    (b"urn:iso:std:iso:ts:21496:-1\0", 0),
    (b"MPF\0", 0),
    (b"Photoshop 3.0\0", 0),
];

impl<R: ZByteReaderTrait> HideMetadata<R> {
    /// Whether the segment whose length field ends at the cursor holds `bytes`
    /// after that field and ends within the input.
    fn holds(&mut self, bytes: u64) -> Result<bool, ZByteIoError> {
        let position = self.0.z_seek(ZSeekFrom::Current(-2))?;
        let mut length = [0; 2];
        self.0.read_exact_bytes(&mut length)?;
        let length = u64::from(u16::from_be_bytes(length));
        let end = self.0.z_seek(ZSeekFrom::End(0))?;
        self.0.z_seek(ZSeekFrom::Start(position + 2))?;
        Ok(length >= 2 + bytes && position + length <= end)
    }
}

impl<R: ZByteReaderTrait> ZByteReaderTrait for HideMetadata<R> {
    fn read_byte_no_error(&mut self) -> u8 {
        self.0.read_byte_no_error()
    }

    fn read_exact_bytes(&mut self, buf: &mut [u8]) -> Result<(), ZByteIoError> {
        self.0.read_exact_bytes(buf)
    }

    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, ZByteIoError> {
        self.0.read_bytes(buf)
    }

    fn peek_bytes(&mut self, buf: &mut [u8]) -> Result<usize, ZByteIoError> {
        self.0.peek_bytes(buf)
    }

    fn peek_exact_bytes(&mut self, buf: &mut [u8]) -> Result<(), ZByteIoError> {
        self.0.peek_exact_bytes(buf)?;
        let kept = KEPT_SEGMENTS
            .iter()
            .find(|(identifier, _)| buf == *identifier);
        if let Some(&(identifier, header)) = kept {
            if self.holds(identifier.len() as u64 + header)? {
                buf[0] = 0;
            }
        }
        Ok(())
    }

    fn z_seek(&mut self, from: ZSeekFrom) -> Result<u64, ZByteIoError> {
        self.0.z_seek(from)
    }

    fn is_eof(&mut self) -> Result<bool, ZByteIoError> {
        self.0.is_eof()
    }

    fn z_position(&mut self) -> Result<u64, ZByteIoError> {
        self.0.z_position()
    }

    fn read_remaining(&mut self, sink: &mut Vec<u8>) -> Result<usize, ZByteIoError> {
        self.0.read_remaining(sink)
    }
}

/// What `decode_jpeg` needs from a JPEG's headers.
struct JpegHeader {
    width: u32,
    height: u32,
    input_color: ColorSpace,
    coefficient_bytes: u64,
}

fn jpeg_header(bytes: &[u8]) -> Result<JpegHeader, zune_jpeg::errors::DecodeErrors> {
    // `decode_headers` stops right after the first scan header, so the
    // cursor's position is where that header ends.
    let mut cursor = Cursor::new(bytes);
    let mut decoder = jpeg_decoder(&mut cursor, ColorSpace::RGB);
    decoder.decode_headers()?;
    let frame = decoder.info().expect("headers were decoded");
    let input_color = decoder.input_colorspace().expect("headers were decoded");
    let scan_end = usize::try_from(cursor.position()).expect("within the input");
    Ok(JpegHeader {
        width: frame.width.into(),
        height: frame.height.into(),
        input_color,
        coefficient_bytes: coefficient_bytes(&bytes[..scan_end], &frame),
    })
}

/// The `ImageError` `image` reports for a zune-jpeg error. `image` maps two
/// more variants, which zune-jpeg 0.5.15 never returns.
fn jpeg_error(err: zune_jpeg::errors::DecodeErrors) -> GuardedDecodeError {
    GuardedDecodeError::Decode(image::ImageError::Decoding(
        image::error::DecodingError::new(image::ImageFormat::Jpeg.into(), err),
    ))
}

/// Reads header dimensions without allocating pixels or applying decode limits.
/// PNG goes through the `png` header alone: `image` sizes the output buffer
/// before it reports dimensions, which fails on 32-bit targets for large ones.
/// JPEG goes through zune-jpeg, which reads the borrowed bytes.
pub(crate) fn header_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    match image::guess_format(bytes).ok()? {
        image::ImageFormat::Png => {
            let info = png_header(bytes)?;
            Some((info.width, info.height))
        }
        image::ImageFormat::Jpeg => {
            let header = jpeg_header(bytes).ok()?;
            Some((header.width, header.height))
        }
        _ => image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .ok()?
            .into_dimensions()
            .ok(),
    }
}

/// The DCT coefficients zune-jpeg holds for the whole image during the
/// decode: those of a progressive JPEG, and those of a baseline JPEG whose
/// first scan, the one whose header ends `to_first_scan`, leaves out a
/// component.
///
/// `frame` is what zune-jpeg parsed, which gives the frame type and the
/// component count. Its `SampleRatios` is too coarse for the count, so the
/// sampling factors are read from the bytes. Every frame header of that type
/// in `to_first_scan` that zune-jpeg would accept at the parsed size and
/// component count is read, and the largest count wins. zune-jpeg parses one
/// of them, so the count is never below zune-jpeg's. It can be above it. The
/// largest of several headers wins, and a header is counted even if the decode
/// would fail on it, such as one that names a quantization table no DQT
/// segment before the first scan defines. zune-jpeg checks the tables only
/// when the decode starts, before it allocates the coefficients.
fn coefficient_bytes(to_first_scan: &[u8], frame: &ImageInfo) -> u64 {
    let bytes = to_first_scan;
    let progressive = frame.sof.is_progressive();
    if !progressive && scan_components(bytes) >= frame.components {
        return 0;
    }
    // zune-jpeg parses 0xFFC0 and 0xFFC1 as baseline, and 0xFFC2 as progressive.
    let markers: &[u8] = if progressive { &[0xC2] } else { &[0xC0, 0xC1] };
    (1..bytes.len())
        .filter(|&i| bytes[i - 1] == 0xFF && markers.contains(&bytes[i]))
        .filter_map(|i| frame_coefficients(&bytes[i + 1..], frame))
        .max()
        .unwrap_or(0)
}

/// The component count of the scan header that ends `bytes`: 0xFFDA, its
/// length, the count, two bytes per component and three more. The smallest
/// count that fits wins, and 1 if none does, so a doubt counts coefficients.
fn scan_components(bytes: &[u8]) -> u8 {
    (1..=4)
        .find(|&components| {
            let length = 6 + 2 * usize::from(components);
            bytes.len().checked_sub(length + 2).is_some_and(|marker| {
                bytes[marker..marker + 5] == [0xFF, 0xDA, 0, length as u8, components]
            })
        })
        .unwrap_or(1)
}

/// The coefficients of a frame header that matches `frame` and that zune-jpeg
/// would accept: 64 two-byte coefficients for each block of every MCU, with
/// each side padded to whole MCUs. The blocks per MCU are the sum of each
/// component's sampling factors multiplied. The quantization tables the header
/// names are not checked against the DQT segments before the first scan, so
/// the count includes a header zune-jpeg would parse and then fail to decode.
fn frame_coefficients(header: &[u8], frame: &ImageInfo) -> Option<u64> {
    // Length, precision, height, width, component count, then an id, the
    // sampling factors and a table per component.
    let fixed = header.get(..8)?;
    let field = |at: usize| u16::from_be_bytes([fixed[at], fixed[at + 1]]);
    let components = fixed[7];
    let length = 8 + 3 * usize::from(components);
    if components != frame.components
        || usize::from(field(0)) != length
        || fixed[2] != 8
        || field(3) != frame.height
        || field(5) != frame.width
    {
        return None;
    }
    let (mut h_max, mut v_max, mut blocks) = (1, 1, 0);
    for &[_, factors, table] in header.get(8..length)?.as_chunks::<3>().0 {
        let (h, v) = (u64::from(factors >> 4), u64::from(factors & 0xF));
        // zune-jpeg rejects a horizontal factor other than 1, 2 or 4, a
        // vertical factor outside 1..=4 and a quantization table above 3.
        if !matches!(h, 1 | 2 | 4) || !(1..=4).contains(&v) || table > 3 {
            return None;
        }
        h_max = h_max.max(h);
        v_max = v_max.max(v);
        blocks += h * v;
    }
    let mcus =
        u64::from(frame.width).div_ceil(8 * h_max) * u64::from(frame.height).div_ceil(8 * v_max);
    Some(2 * 64 * mcus * blocks)
}

/// A PNG's IHDR, parsed without allocating pixels. `None` for other formats
/// and for a PNG whose header does not parse.
fn png_header(bytes: &[u8]) -> Option<png::Info<'static>> {
    if image::guess_format(bytes).ok()? != image::ImageFormat::Png {
        return None;
    }
    png::Decoder::new(Cursor::new(bytes))
        .read_header_info()
        .ok()
        .cloned()
}

/// Box-filter `bitmap` so its larger side is at most `max_side` (>= 1). Returns
/// a clone when it already fits. Average of the covered source pixels per
/// destination pixel, the same filter `generate_preview_png` uses.
pub fn downsample(bitmap: &GrayscaleBitmap, max_side: u32) -> GrayscaleBitmap {
    let max_side = max_side.max(1);

    let (tw, th) = if bitmap.width > max_side || bitmap.height > max_side {
        let scale = f64::min(
            max_side as f64 / bitmap.width as f64,
            max_side as f64 / bitmap.height as f64,
        );
        let w = ((bitmap.width as f64 * scale).round() as u32).max(1);
        let h = ((bitmap.height as f64 * scale).round() as u32).max(1);
        (w, h)
    } else {
        return bitmap.clone();
    };

    let src_w = bitmap.width as f64;
    let src_h = bitmap.height as f64;
    let tw_f = tw as f64;
    let th_f = th as f64;

    let mut data = Vec::with_capacity((tw as usize) * (th as usize));
    for y in 0..th {
        let sy0 = (y as f64 * src_h / th_f) as u32;
        let sy1 = (((y + 1) as f64 * src_h / th_f) as u32).min(bitmap.height);
        for x in 0..tw {
            let sx0 = (x as f64 * src_w / tw_f) as u32;
            let sx1 = (((x + 1) as f64 * src_w / tw_f) as u32).min(bitmap.width);

            let mut sum = 0u32;
            let mut count = 0u32;
            for sy in sy0..sy1 {
                let row_offset = (sy * bitmap.width) as usize;
                for sx in sx0..sx1 {
                    sum += bitmap.data[row_offset + sx as usize] as u32;
                    count += 1;
                }
            }

            #[allow(clippy::manual_checked_ops)]
            let value = if count > 0 { (sum / count) as u8 } else { 0 };
            data.push(value);
        }
    }

    GrayscaleBitmap {
        width: tw,
        height: th,
        data,
    }
}

/// A 200 px RGBA thumbnail: black ink, alpha from the tip's gray value.
pub fn generate_preview_png(bitmap: &GrayscaleBitmap) -> Result<Vec<u8>, String> {
    let small = downsample(bitmap, 200);

    let img: RgbaImage = ImageBuffer::from_fn(small.width, small.height, |x, y| {
        let alpha = small.data[(y * small.width + x) as usize];
        Rgba([0, 0, 0, alpha])
    });

    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| format!("thumbnail encoding failed: {e}"))?;

    Ok(buf.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_grayscale_16bit_takes_high_byte() {
        let tip = TipBitmap {
            width: 2,
            height: 1,
            depth: 16,
            data: vec![0xAB, 0x12, 0xCD, 0x34],
        };
        let result = to_grayscale(&tip);
        assert_eq!(result.width, 2);
        assert_eq!(result.height, 1);
        assert_eq!(result.data.len(), 2);
        assert_eq!(result.data, vec![0xAB, 0xCD], "must keep the high byte");
    }

    #[test]
    fn downsample_averages_covered_pixels() {
        let bitmap = GrayscaleBitmap {
            width: 4,
            height: 2,
            data: vec![0, 255, 0, 255, 0, 255, 0, 255],
        };
        let small = downsample(&bitmap, 2);
        assert_eq!((small.width, small.height), (2, 1));
        assert_eq!(small.data, vec![127, 127]);
    }

    #[test]
    fn downsample_keeps_a_fitting_bitmap() {
        let bitmap = GrayscaleBitmap {
            width: 3,
            height: 2,
            data: vec![1, 2, 3, 4, 5, 6],
        };
        let same = downsample(&bitmap, 8);
        assert_eq!((same.width, same.height), (3, 2));
        assert_eq!(same.data, bitmap.data);
    }

    #[test]
    fn preview_png_is_200_wide_with_alpha_from_gray() {
        let bitmap = GrayscaleBitmap {
            width: 400,
            height: 100,
            data: vec![200u8; 400 * 100],
        };
        let png = generate_preview_png(&bitmap).unwrap();
        let img = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!((img.width(), img.height()), (200, 50));
        assert!(img.pixels().all(|p| p.0 == [0, 0, 0, 200]));
    }
}

#[cfg(test)]
mod tip_image_tests {
    use super::*;

    fn rgba_png(width: u32, height: u32, pixels: &[[u8; 4]]) -> Vec<u8> {
        let img: RgbaImage =
            ImageBuffer::from_fn(width, height, |x, y| Rgba(pixels[(y * width + x) as usize]));
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    fn luma_png(width: u32, height: u32, luma: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut buf, width, height);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(luma).unwrap();
        }
        buf
    }

    #[test]
    fn alpha_channel_becomes_intensity() {
        let png = rgba_png(3, 1, &[[255, 0, 0, 255], [255, 0, 0, 128], [255, 0, 0, 0]]);
        let bitmap = decode_tip_image(&png).unwrap();
        assert_eq!((bitmap.width, bitmap.height), (3, 1));
        assert_eq!(bitmap.data, vec![255, 128, 0]);
    }

    #[test]
    fn opaque_image_inverts_luminance() {
        let png = luma_png(3, 1, &[0, 40, 255]);
        let bitmap = decode_tip_image(&png).unwrap();
        assert_eq!((bitmap.width, bitmap.height), (3, 1));
        assert_eq!(bitmap.data, vec![255, 215, 0]);
    }

    #[test]
    fn oversized_image_is_rejected_with_the_ceiling_message() {
        let over = MAX_IMPORT_DIMENSION + 1;
        let png = luma_png(over, 1, &vec![0u8; over as usize]);
        let err = decode_tip_image(&png).expect_err("over-ceiling image must be rejected");
        assert!(
            matches!(err, TipImageError::TooLarge { width, height } if width == over && height == 1),
            "expected TooLarge, got {err:?}"
        );
        assert!(
            err.to_string().starts_with("image is "),
            "unexpected message: {err}"
        );
    }

    /// Every decodable pixel format, filled with varied samples, over several
    /// conversion chunks.
    fn every_format() -> [(PixelFormat, image::DynamicImage); 10] {
        let base = image::DynamicImage::ImageRgba16(ImageBuffer::from_fn(100, 100, |x, y| {
            let i = (y * 100 + x) * 4;
            image::Rgba([0, 1, 2, 3].map(|c| ((i + c).wrapping_mul(0x9E37_79B9) >> 16) as u16))
        }));
        [
            (PixelFormat::L8, base.to_luma8().into()),
            (PixelFormat::La8, base.to_luma_alpha8().into()),
            (PixelFormat::Rgb8, base.to_rgb8().into()),
            (PixelFormat::Rgba8, base.to_rgba8().into()),
            (PixelFormat::L16, base.to_luma16().into()),
            (PixelFormat::La16, base.to_luma_alpha16().into()),
            (PixelFormat::Rgb16, base.to_rgb16().into()),
            (PixelFormat::Rgb32F, base.to_rgb32f().into()),
            (PixelFormat::Rgba32F, base.to_rgba32f().into()),
            (PixelFormat::Rgba16, base),
        ]
    }

    #[test]
    fn tips_match_image_conversions_for_every_format() {
        for (format, image) in every_format() {
            let name = format!("{:?}", image.color());
            let coverage = if image.color().has_alpha() {
                image.to_luma_alpha8().pixels().map(|p| p.0[1]).collect()
            } else {
                let luma = image.to_luma8().into_raw();
                luma.into_iter().map(|v| 255 - v).collect::<Vec<_>>()
            };
            let luma = image.to_luma8().into_raw();
            let decoded = |image: &image::DynamicImage| DecodedImage {
                width: image.width(),
                height: image.height(),
                format,
                bytes: image.as_bytes().to_vec(),
            };
            assert_eq!(
                tip_plane(decoded(&image), TipSample::Coverage),
                coverage,
                "coverage {name}"
            );
            assert_eq!(
                tip_plane(decoded(&image), TipSample::Luma),
                luma,
                "luma {name}"
            );
            if !matches!(format, PixelFormat::Rgb32F | PixelFormat::Rgba32F) {
                let mut png = std::io::Cursor::new(Vec::new());
                image.write_to(&mut png, image::ImageFormat::Png).unwrap();
                let png = png.into_inner();
                assert_eq!(
                    decode_tip_image(&png).unwrap().data,
                    coverage,
                    "decode_tip_image {name}"
                );
                assert_eq!(
                    crate::procreate::decode_tip_png(&png).unwrap().data,
                    luma,
                    "decode_tip_png {name}"
                );
            }
        }
    }

    /// `decode_guarded` decodes JPEG with zune-jpeg directly, so it must
    /// return the pixels and errors `image` returns for the same bytes.
    #[test]
    fn jpeg_decodes_as_image_does() {
        let base = image::DynamicImage::ImageRgb8(ImageBuffer::from_fn(37, 23, |x, y| {
            let i = y * 37 + x;
            image::Rgb([0, 1, 2].map(|c| ((i * 3 + c).wrapping_mul(0x9E37_79B9) >> 24) as u8))
        }));
        let mut gray = std::io::Cursor::new(Vec::new());
        base.to_luma8()
            .write_to(&mut gray, image::ImageFormat::Jpeg)
            .unwrap();
        for image in [base.to_luma8().into(), base] {
            let mut jpeg = std::io::Cursor::new(Vec::new());
            image.write_to(&mut jpeg, image::ImageFormat::Jpeg).unwrap();
            let jpeg = jpeg.into_inner();
            let expected = image::load_from_memory(&jpeg).unwrap();
            let Ok(decoded) = decode_guarded(&jpeg, 64, u64::MAX) else {
                panic!("{:?} JPEG must decode", image.color());
            };
            assert_eq!((decoded.width, decoded.height), (37, 23));
            assert_eq!(decoded.bytes, expected.as_bytes(), "{:?}", image.color());
        }

        let truncated = [0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x43, 0x00];
        let expected = image::load_from_memory(&truncated).unwrap_err();
        let Err(GuardedDecodeError::Decode(err)) = decode_guarded(&truncated, 64, u64::MAX) else {
            panic!("a truncated JPEG must not decode");
        };
        assert_eq!(err.to_string(), expected.to_string());

        // zune-jpeg rejects a kept segment that runs past the end of the input
        // and an extended XMP part with a 39-byte header, as `image` does.
        let mut cases = Vec::new();
        for (name, marker, body) in kept_segments() {
            cases.push((name.to_string(), segment(marker, &body, None), true));
            let past_end = segment(marker, &body, Some(u16::MAX));
            cases.push((format!("{name} past the end"), past_end, false));
        }
        let mut short_part = b"http://ns.adobe.com/xmp/extension/\0".to_vec();
        short_part.resize(short_part.len() + 39, 0);
        let short_part = segment(0xE1, &short_part, None);
        cases.push(("extended XMP with a short header".into(), short_part, false));
        for (name, segment, decodes) in cases {
            let mut jpeg = gray.get_ref().clone();
            jpeg.splice(2..2, segment);
            match (
                image::load_from_memory(&jpeg),
                decode_guarded(&jpeg, 64, u64::MAX),
            ) {
                (Ok(expected), Ok(decoded)) if decodes => {
                    assert_eq!(decoded.bytes, expected.as_bytes(), "{name}");
                }
                (Err(expected), Err(GuardedDecodeError::Decode(err))) if !decodes => {
                    assert_eq!(err.to_string(), expected.to_string(), "{name}");
                }
                (expected, decoded) => panic!(
                    "{name}: image {:?}, decode_guarded {:?}",
                    expected.is_ok(),
                    decoded.is_ok()
                ),
            }
        }
    }

    /// A JPEG segment: `marker`, a length, `declared` or the real one, and
    /// `body`.
    fn segment(marker: u8, body: &[u8], declared: Option<u16>) -> Vec<u8> {
        let length = declared.unwrap_or_else(|| u16::try_from(2 + body.len()).unwrap());
        let mut segment = vec![0xFF, marker];
        segment.extend_from_slice(&length.to_be_bytes());
        segment.extend_from_slice(body);
        segment
    }

    /// One segment of each kind zune-jpeg 0.5.15 keeps, with the identifiers
    /// copied from its `headers.rs` and 5 bytes of data, enough for every
    /// kind to be kept.
    fn kept_segments() -> [(&'static str, u8, Vec<u8>); 7] {
        let with_data = |identifier: &[u8]| [identifier, b"12345"].concat();
        let mut extended_xmp = b"http://ns.adobe.com/xmp/extension/\0".to_vec();
        extended_xmp.extend_from_slice(&[b'G'; 32]);
        extended_xmp.extend_from_slice(&5u32.to_be_bytes());
        extended_xmp.extend_from_slice(&0u32.to_be_bytes());
        [
            ("EXIF", 0xE1, with_data(b"Exif\0\0")),
            ("XMP", 0xE1, with_data(b"http://ns.adobe.com/xap/1.0/\0")),
            ("extended XMP", 0xE1, with_data(&extended_xmp)),
            ("ICC", 0xE2, with_data(b"ICC_PROFILE\0\x01\x01")),
            (
                "gain map",
                0xE2,
                with_data(b"urn:iso:std:iso:ts:21496:-1\0"),
            ),
            ("MPF", 0xE2, with_data(b"MPF\0")),
            ("IPTC", 0xED, with_data(b"Photoshop 3.0\0")),
        ]
    }

    /// zune-jpeg keeps none of the metadata segments behind `jpeg_decoder`,
    /// and all of them without it.
    #[test]
    fn jpeg_decoder_keeps_no_metadata() {
        let mut jpeg = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(ImageBuffer::from_pixel(8, 8, image::Luma([0x40])))
            .write_to(&mut jpeg, image::ImageFormat::Jpeg)
            .unwrap();
        let mut jpeg = jpeg.into_inner();
        let metadata: Vec<u8> = kept_segments()
            .iter()
            .flat_map(|(_, marker, body)| segment(*marker, body, None))
            .collect();
        jpeg.splice(2..2, metadata);

        let kept = |info: &zune_jpeg::ImageInfo, icc: Option<Vec<u8>>| {
            [
                ("EXIF", info.exif_data.is_some()),
                ("XMP", info.xmp_data.is_some()),
                ("extended XMP", info.extended_xmp.is_some()),
                ("ICC", icc.is_some()),
                ("gain map", !info.gain_map_info.is_empty()),
                ("MPF", info.multi_picture_information.is_some()),
                ("IPTC", info.iptc_data.is_some()),
            ]
        };
        let options = DecoderOptions::default().set_strict_mode(false);
        let mut plain = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(&jpeg), options);
        plain.decode_headers().unwrap();
        for (name, kept) in kept(&plain.info().unwrap(), plain.icc_profile()) {
            assert!(kept, "zune-jpeg alone keeps {name}");
        }
        let mut hidden = jpeg_decoder(ZCursor::new(&jpeg), ColorSpace::Luma);
        hidden.decode_headers().unwrap();
        for (name, kept) in kept(&hidden.info().unwrap(), hidden.icc_profile()) {
            assert!(!kept, "jpeg_decoder must hide {name}");
        }
    }

    /// `decode_guarded` decodes PNG with `png` directly, so it must return the
    /// pixels and errors `image` returns for the same bytes, including the
    /// color types and bit depths EXPAND widens.
    #[test]
    fn png_decodes_as_image_does() {
        use png::{BitDepth, ColorType};
        let (width, height) = (37, 23);
        let cases: [(ColorType, BitDepth, u32, &[u8]); 5] = [
            (ColorType::Grayscale, BitDepth::One, 1, &[]),
            (ColorType::Grayscale, BitDepth::Four, 4, &[0, 3]),
            (ColorType::Indexed, BitDepth::Eight, 8, &[0, 64, 128, 192]),
            (ColorType::Rgb, BitDepth::Sixteen, 48, &[0, 7, 0, 7, 0, 7]),
            (ColorType::Rgba, BitDepth::Sixteen, 64, &[]),
        ];
        let mut last = Vec::new();
        for (color, depth, bits_per_pixel, trns) in cases {
            let len = (width * bits_per_pixel).div_ceil(8) * height;
            let data: Vec<u8> = (0..len)
                .map(|i| (i.wrapping_mul(0x9E37_79B9) >> 24) as u8)
                .collect();
            let mut png = Vec::new();
            let mut encoder = png::Encoder::new(&mut png, width, height);
            encoder.set_color(color);
            encoder.set_depth(depth);
            if color == ColorType::Indexed {
                encoder.set_palette((0..=255u8).flat_map(|i| [i, !i, i / 2]).collect::<Vec<_>>());
            }
            if !trns.is_empty() {
                encoder.set_trns(trns);
            }
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&data).unwrap();
            writer.finish().unwrap();

            let name = format!("{color:?} {depth:?}");
            let expected = image::load_from_memory(&png).unwrap();
            let Ok(decoded) = decode_guarded(&png, 64, u64::MAX) else {
                panic!("{name} PNG must decode");
            };
            assert_eq!((decoded.width, decoded.height), (width, height), "{name}");
            assert_eq!(decoded.format.color_type(), expected.color(), "{name}");
            assert_eq!(decoded.bytes, expected.as_bytes(), "{name}");
            last = png;
        }

        let mut bad_crc = last.clone();
        bad_crc[29] ^= 1;
        for (name, broken) in [("truncated", &last[..20]), ("bad IHDR CRC", &bad_crc[..])] {
            let expected = image::load_from_memory(broken).unwrap_err();
            let Err(GuardedDecodeError::Decode(err)) = decode_guarded(broken, 64, u64::MAX) else {
                panic!("a PNG with {name} must not decode");
            };
            assert_eq!(err.to_string(), expected.to_string(), "{name}");
        }
    }

    /// An eXIf chunk before the image data decodes up to
    /// `MAX_PNG_EXIF_BYTES`, whatever the pixel format. One after the image
    /// data is never read.
    #[test]
    fn png_exif_decodes_up_to_the_limit() {
        let png = |exif_len: usize, after_image: bool| {
            let mut png = Vec::new();
            let mut encoder = png::Encoder::new(&mut png, 3, 2);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Sixteen);
            let mut writer = encoder.write_header().unwrap();
            let exif = png::chunk::ChunkType(*b"eXIf");
            if !after_image {
                writer.write_chunk(exif, &vec![0; exif_len]).unwrap();
            }
            writer.write_image_data(&[0x40; 3 * 2 * 8]).unwrap();
            if after_image {
                writer.write_chunk(exif, &vec![0; exif_len]).unwrap();
            }
            writer.finish().unwrap();
            png
        };
        for (exif_len, after_image) in [(MAX_PNG_EXIF_BYTES, false), (MAX_PNG_EXIF_BYTES + 1, true)]
        {
            assert!(
                decode_guarded(&png(exif_len, after_image), 64, u64::MAX).is_ok(),
                "a {exif_len}-byte eXIf, after the image data: {after_image}, must decode"
            );
        }
        let Err(GuardedDecodeError::Decode(err)) =
            decode_guarded(&png(MAX_PNG_EXIF_BYTES + 1, false), 64, u64::MAX)
        else {
            panic!("an eXIf over the limit must not decode");
        };
        assert_eq!(
            err.to_string(),
            "Format error decoding Png: eXIf chunk is 65537 bytes, over the 65536 a tip reads"
        );
    }

    #[test]
    fn garbage_bytes_report_a_decode_error() {
        let err = decode_tip_image(b"not an image").expect_err("garbage must not decode");
        assert!(matches!(err, TipImageError::Decode(_)), "got {err:?}");
    }
}

#[cfg(test)]
mod tip_bitmap_of_tests {
    use super::*;

    #[test]
    fn tip_bitmap_of_copies_pixels_and_declares_depth_8() {
        let bitmap = GrayscaleBitmap {
            width: 3,
            height: 2,
            data: vec![0, 17, 255, 128, 1, 0],
        };
        let tip = tip_bitmap_of(&bitmap);
        assert_eq!((tip.width, tip.height), (3, 2));
        assert_eq!(tip.depth, 8, "GrayscaleBitmap is 8-bit by construction");
        assert_eq!(tip.data, bitmap.data, "polarity matches, so no conversion");
    }

    #[test]
    fn tip_bitmap_of_round_trips_an_8_bit_tip() {
        let tip = TipBitmap {
            width: 2,
            height: 2,
            depth: 8,
            data: vec![9, 200, 0, 255],
        };
        let back = tip_bitmap_of(&to_grayscale(&tip));
        assert_eq!((back.width, back.height, back.depth), (2, 2, 8));
        assert_eq!(back.data, tip.data);
    }
}
