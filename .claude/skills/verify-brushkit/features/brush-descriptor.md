# Brush descriptor

A consumer reads the Photoshop settings of each `.abr` preset as a typed
`BrushDescriptor`: spacing, diameter, computed geometry, the Shape Dynamics,
Scatter, Texture, Dual Brush, Transfer and Color Dynamics panels, the tool
settings saved with the preset and the tip's Intr flag. A readable sampled
brush and a computed preset each carry one. A field is `None` when the file
carries no value for it.

## Sub-features

- `desc-sampled` gives each readable sampled brush the descriptor of its preset in `AbrBrush::descriptor`.
- `desc-computed` gives each computed preset its descriptor in `ComputedPreset::descriptor`, with `computed` always `Some`.
- `desc-default` gives `BrushDescriptor::default()` to a brush with no preset: a v1/v2 pack, a pack without a `desc` block, or an unpaired record on the fallback path.
- `desc-gating` leaves a panel's fields `None` when the panel's `use*` flag (`useTipDynamics`, `useScatter`, `usePaintDynamics`, `useTexture`, `useColorDynamics`, `useDualBrush`) is false.
- `desc-tool-options` reads `toolOptions` into `ToolOptions`, with opacity and blend mode from `MdSl` when the top level lacks them.
- `desc-modes` gives the same descriptors in all four parse modes.

## How to get to it (user POV)

- `brushkit::abr::parse_abr(&bytes)?.brushes[i].descriptor` and `.computed_presets[i].descriptor`, or the same fields of `DeferredPack::pack`.
- `brushkit::abr::BrushDescriptor`, `ToolOptions`, `DualBrush` and `ComputedGeometry`. They derive `Debug`, `Clone`, `Default` and `PartialEq`, not serde.
- An unavailable brush (`UnavailableBrush`) and an unsupported preset (`UnsupportedTipPresetDetail`) carry no descriptor.

## Driving it with verify-brushkit

Preconditions:

- Baseline preconditions from the index hold.
- The probe's `parse` command writes each descriptor as its `Debug` text in `.result.pack.brushes[].descriptor` and `.result.pack.computed_presets[].descriptor`. Assert on it with jq `test`, for example `test("tilt_scale_pct: Some")`.

- **Seed sampled brush.** Run `$H run seed-desc parse fuzz/corpus/preview_abr/unavailable_tips`. Brush `A` at `preset_index` `1` has the all-default descriptor, which starts `BrushDescriptor { spacing_pct: None, computed: None, diameter_px: None,` and ends `tool_options: None, interpolation: None }`, because its preset writes only `Nm  ` and `sampledData`. `.result.pack.sampled_brushes[].unavailable` has no `descriptor` key.
- **Seed computed preset.** The same run's `computed_presets[0].descriptor` contains `computed: Some(ComputedGeometry { diameter_px: Some(30.0), hardness_pct: None, angle_deg: None, roundness_pct: None })` and `diameter_px: Some(30.0)`.
- **Default.** Run `$H run seed-v2 parse fuzz/corpus/preview_abr/zero_area_tip` and `$H run seed-nodesc parse fuzz/corpus/preview_abr/sampled_tip`. Every brush's `descriptor` equals brush `A`'s from `seed-desc`, and its `preset_index` is `null`.
- **Mode parity.** Run a real pack with `--mode eager`, `deferred`, `deferred-no-patterns` and `all-deferred`. `[.result.pack.brushes[].descriptor, .result.pack.computed_presets[].descriptor]` is identical across the four runs.
- **Real tool options.** On a v10 corpus pack, `.result.pack.brushes[].descriptor` contains `tool_options: Some(ToolOptions { tool: "<class>"` exactly where `dump` preset `preset_index` has a `toolOptions` item, and `<class>` is that item's `.value.objc.class_id`.
- **Real gating.** On the same pack, every preset whose `dump` item `useTipDynamics` is `{"bool":false}` has `size_jitter_pct: None`, `tilt_scale_pct: None` and `minimum_diameter_pct: None`.
- **Corpus-wide values.** Run `BRUSHKIT_CORPUS_DIR=... cargo test -p brushkit-abr --test corpus_smoke -- corpus_sets_a_nonzero_control_on_every_brvr_panel corpus_sets_a_non_default_value_for_every_tool_setting_and_intr corpus_sets_a_non_default_tilt_scale_on_size_on_pen_tilt every_corpus_mdsl_preset_reads_its_blend_mode_and_opacity`. Four tests pass. They prove the `bVTy` controls, the tool settings, Intr, Tilt Scale and the `MdSl` fallback on real files.

## Gotchas

- The `Debug` text is long. Query it with `test` or `capture` instead of reading it.
- With `BRUSHKIT_CORPUS_DIR` unset, the corpus tests print `skip:` and pass, which proves nothing.
- The seeds carry no panel, tool or Intr values. Dual brush, texture, color and the `MdSl` fallback order have only synthetic proof in the unit tests of `crates/abr/src/descriptor.rs`, beyond the corpus tests above.
- `interpolation` is the main tip's `Brsh/Intr`. The dual brush's own Intr is not read.
- `Opct` counts only as wire type `long`. A `doub` gives `opacity_pct: None`.
