// SPDX-License-Identifier: AGPL-3.0-or-later
//! Fallible icon allocation shared by the navigation and system tray.

pub(crate) fn render(svg: &[u8], size: u32) -> Option<Vec<u8>> {
    render_with(svg, size, allocate)
}

fn allocate(size: u32) -> Option<resvg::tiny_skia::Pixmap> {
    let dimensions = resvg::tiny_skia::IntSize::from_wh(size, size)?;
    let bytes = usize::try_from(size).ok()?.checked_mul(size as usize)?.checked_mul(4)?;
    let mut data = Vec::new();
    data.try_reserve_exact(bytes).ok()?;
    data.resize(bytes, 0);
    resvg::tiny_skia::Pixmap::from_vec(data, dimensions)
}

fn render_with(svg: &[u8], size: u32, alloc: impl FnOnce(u32) -> Option<resvg::tiny_skia::Pixmap>) -> Option<Vec<u8>> {
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = alloc(size)?;
    let scale = size as f32 / 64.0;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let mut data = pixmap.take();
    // tiny-skia is premultiplied; iced and tray-icon require straight RGBA.
    for px in data.as_chunks_mut::<4>().0 {
        let alpha = px[3] as u32;
        for channel in &mut px[..3] {
            if let Some(value) = (*channel as u32 * 255).checked_div(alpha) { *channel = value as u8; }
        }
    }
    Some(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    const SVG: &[u8] = b"<svg xmlns='http://www.w3.org/2000/svg' width='64' height='64'><rect width='64' height='64' fill='red' opacity='0.5'/></svg>";

    #[test]
    fn icon_failure_returns_none() {
        assert!(render(SVG, 0).is_none());
        assert!(render(SVG, u32::MAX).is_none());
        assert!(render(b"invalid SVG", 44).is_none());
        assert!(render_with(SVG, 44, |_| None).is_none());
    }

    #[test]
    fn icon_keeps_straight_rgba_and_size() {
        let data = render(SVG, 44).unwrap();
        assert_eq!(data.len(), 44 * 44 * 4);
        assert_eq!(&data[..4], &[255, 0, 0, 128]);
    }
}
