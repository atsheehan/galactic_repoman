//! Screenshotting from inside the app: copy a presented swapchain image into a
//! host-visible buffer and write it out as a PNG.
//!
//! This exists because the desktop cannot do it for us. Under GNOME's Wayland
//! compositor the Shell's screenshot D-Bus interface is reserved for the Shell's own
//! UI (`AccessDenied` to everyone else, `gnome-screenshot` included), X11 grabbers
//! see only the empty XWayland root, and the wlroots tools do not work under mutter.
//! Capturing from inside the swapchain sidesteps all of it — and is the only approach
//! that will still work over SSH and under gamescope on the Deck.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, anyhow};
use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage, Subbuffer};
use vulkano::format::Format;
use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator};

/// A host-visible landing pad for one frame's pixels. Allocated per capture rather
/// than kept around: captures are a manual debugging action, so paying for the
/// allocation each time is cheaper than carrying a full framebuffer forever.
pub fn staging_buffer(
    allocator: Arc<StandardMemoryAllocator>,
    extent: [u32; 2],
) -> anyhow::Result<Subbuffer<[u8]>> {
    let bytes = u64::from(extent[0]) * u64::from(extent[1]) * 4;

    Buffer::new_slice::<u8>(
        allocator,
        BufferCreateInfo {
            usage: BufferUsage::TRANSFER_DST,
            ..Default::default()
        },
        AllocationCreateInfo {
            memory_type_filter: MemoryTypeFilter::PREFER_HOST
                | MemoryTypeFilter::HOST_RANDOM_ACCESS,
            ..Default::default()
        },
        bytes,
    )
    .context("allocating the capture staging buffer")
}

/// Write the staged pixels out as an 8-bit RGBA PNG.
///
/// The swapchain format decides the channel order in the buffer, so it is passed in
/// rather than assumed — a B8G8R8A8 surface hands us BGRA and writing that straight
/// out would swap red and blue in every screenshot.
pub fn write_png(
    path: &Path,
    extent: [u32; 2],
    format: Format,
    pixels: &[u8],
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }

    let rgba = to_rgba(format, pixels)?;

    let file = File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), extent[0], extent[1]);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    // The swapchain is an sRGB format, so the bytes are already sRGB-encoded and the
    // PNG should say so rather than leave a viewer to guess.
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);

    encoder
        .write_header()
        .context("writing the PNG header")?
        .write_image_data(&rgba)
        .context("writing the PNG image data")?;

    Ok(())
}

/// Reorder the staged bytes into RGBA. Only the 8-bit-per-channel formats we actually
/// select a swapchain from are handled; anything else is an error rather than a
/// silently miscolored screenshot.
fn to_rgba(format: Format, pixels: &[u8]) -> anyhow::Result<Vec<u8>> {
    match format {
        Format::R8G8B8A8_SRGB | Format::R8G8B8A8_UNORM => Ok(pixels.to_vec()),
        Format::B8G8R8A8_SRGB | Format::B8G8R8A8_UNORM => Ok(pixels
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|bgra| [bgra[2], bgra[1], bgra[0], bgra[3]])
            .collect()),
        other => Err(anyhow!(
            "cannot capture a {other:?} swapchain; only 8-bit RGBA/BGRA formats are handled"
        )),
    }
}
