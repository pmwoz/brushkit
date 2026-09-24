use brushkit_preview::{PreviewError, PreviewOptions, PreviewSet, TipPreview};

/// Panics if an available tip has no pixels, a side over `max_cell`, or not one byte per pixel.
pub fn assert_tip_shapes(result: Result<PreviewSet, PreviewError>, opts: PreviewOptions) {
    let Ok(set) = result else { return };
    for entry in &set.entries {
        let TipPreview::Available(tip) = &entry.tip else {
            continue;
        };
        let (index, name, width, height) = (entry.index, &entry.name, tip.width, tip.height);
        assert!(
            width > 0 && height > 0,
            "entry {index} {name:?}: {width}x{height} tip has no pixels"
        );
        assert!(
            width.max(height) <= opts.max_cell,
            "entry {index} {name:?}: {width}x{height} tip has a side over max_cell {}",
            opts.max_cell
        );
        assert_eq!(
            tip.data.len(),
            width as usize * height as usize,
            "entry {index} {name:?}: {width}x{height} tip is not one byte per pixel"
        );
    }
}
