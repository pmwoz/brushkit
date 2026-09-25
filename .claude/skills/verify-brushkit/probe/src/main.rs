//! Drives the public `brushkit` API on one file and writes what it returned.
//!
//!   brushkit-probe parse   <file> --out DIR [--mode eager|deferred|deferred-no-patterns|all-deferred]
//!   brushkit-probe preview <file> --out DIR [--kind abr|brush|brushset] [--max-cell N] [--first N]
//!   brushkit-probe dump    <file> --out DIR
//!
//! Every command writes `summary.json` into DIR and prints it to stdout.
//! `preview` also writes `tips/NNN.png` per available tip and `sheet.png`.
//! Exit code: 0 when the library returned Ok, 1 when it returned Err (the
//! error is in `summary.json`), 2 on a usage or I/O error.

use brushkit::abr::{self, AbrPack};
use brushkit::preview::{
    self, ContactSheetConfig, PreviewOptions, PreviewSet, SheetBrush, TipPreview,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

struct Args {
    command: String,
    file: PathBuf,
    out: PathBuf,
    flags: Vec<(String, String)>,
}

impl Args {
    fn flag(&self, name: &str) -> Option<&str> {
        self.flags
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let command = it.next().ok_or("missing command")?;
    let file = PathBuf::from(it.next().ok_or("missing file")?);
    let mut flags = Vec::new();
    while let Some(key) = it.next() {
        let name = key
            .strip_prefix("--")
            .ok_or_else(|| format!("unexpected argument {key}"))?;
        let value = it.next().ok_or_else(|| format!("{key} needs a value"))?;
        flags.push((name.to_string(), value));
    }
    let out = flags
        .iter()
        .find(|(k, _)| k == "out")
        .map(|(_, v)| PathBuf::from(v))
        .ok_or("missing --out DIR")?;
    Ok(Args {
        command,
        file,
        out,
        flags,
    })
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("brushkit-probe: {e}");
            ExitCode::from(2)
        }
    }
}

/// `Ok(true)` when the library call succeeded.
fn run() -> Result<bool, String> {
    let args = parse_args()?;
    let bytes =
        std::fs::read(&args.file).map_err(|e| format!("read {}: {e}", args.file.display()))?;
    std::fs::create_dir_all(&args.out).map_err(|e| format!("create out dir: {e}"))?;

    let (ok, body) = match args.command.as_str() {
        "parse" => parse(&bytes, args.flag("mode").unwrap_or("eager"))?,
        "preview" => preview(&bytes, &args)?,
        "dump" => dump(&bytes),
        other => return Err(format!("unknown command {other}")),
    };

    let summary = json!({
        "command": args.command,
        "input": args.file.display().to_string(),
        "input_bytes": bytes.len(),
        "ok": ok,
        "result": body,
    });
    let text = serde_json::to_string_pretty(&summary).expect("json values serialize");
    write(&args.out.join("summary.json"), text.as_bytes())?;
    println!("{text}");
    Ok(ok)
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("write {}: {e}", path.display()))
}

fn parse(bytes: &[u8], mode: &str) -> Result<(bool, Value), String> {
    let result = match mode {
        "eager" => {
            abr::parse_abr(bytes).map(|pack| pack_json(&pack, |i| tip_json(&pack.brushes[i].tip)))
        }
        "deferred" => abr::parse_abr_deferred(bytes).map(|d| deferred_json(&d)),
        "deferred-no-patterns" => {
            abr::parse_abr_deferred_without_patterns(bytes).map(|d| deferred_json(&d))
        }
        "all-deferred" => {
            abr::parse_abr_all_deferred_without_patterns(bytes).map(|d| deferred_json(&d))
        }
        other => return Err(format!("unknown --mode {other}")),
    };
    Ok(match result {
        Ok(v) => (true, json!({ "mode": mode, "pack": v })),
        Err(e) => (false, json!({ "mode": mode, "error": e.to_string() })),
    })
}

/// Tips of a deferred pack are decoded through `decode_tip`, the way a
/// consumer reads them.
fn deferred_json(d: &abr::DeferredPack) -> Value {
    pack_json(&d.pack, |i| match d.decode_tip(i) {
        Ok(tip) => tip_json(&tip),
        Err(e) => json!({ "error": e.to_string() }),
    })
}

fn tip_json(tip: &abr::TipBitmap) -> Value {
    json!({ "width": tip.width, "height": tip.height, "depth": tip.depth, "data_len": tip.data.len() })
}

fn pack_json(pack: &AbrPack, tip: impl Fn(usize) -> Value) -> Value {
    json!({
        "version": pack.version.to_string(),
        "preset_count": pack.preset_count,
        "brushes": pack.brushes.iter().enumerate().map(|(i, b)| json!({
            "index": i,
            "id": b.id,
            "name": b.name,
            "preset_index": b.preset_index,
            "tip": tip(i),
        })).collect::<Vec<_>>(),
        "computed_presets": pack.computed_presets.iter().map(|p| {
            let g = p.descriptor.computed.clone().unwrap_or_default();
            json!({
                "name": p.name,
                "preset_index": p.preset_index,
                "diameter_px": g.diameter_px,
                "hardness_pct": g.hardness_pct,
                "angle_deg": g.angle_deg,
                "roundness_pct": g.roundness_pct,
            })
        }).collect::<Vec<_>>(),
        "patterns": pack.patterns.iter().map(|p| json!({
            "id": p.id, "name": p.name, "width": p.width, "height": p.height, "mode": p.mode,
        })).collect::<Vec<_>>(),
        "diagnostics": {
            "dropped_pattern_count": pack.dropped_pattern_count,
            "unreadable_patt_chunk_count": pack.unreadable_patt_chunk_count,
            "dropped_samp_count": pack.dropped_samp_count,
            "skipped_preset_count": pack.skipped_preset_count,
            "unsupported_tip_count": pack.unsupported_tip_count,
            "unsupported_tip_presets": pack.unsupported_tip_presets.iter().map(|p| &p.name).collect::<Vec<_>>(),
            "desc_parse_error": pack.desc_parse_error,
        },
    })
}

fn preview(bytes: &[u8], args: &Args) -> Result<(bool, Value), String> {
    let kind = match args.flag("kind") {
        Some(k) => k.to_string(),
        None => args
            .file
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .ok_or("no file extension, pass --kind")?,
    };
    let max_cell: u32 = parse_num(args.flag("max-cell").unwrap_or("128"), "--max-cell")?;
    let first: Option<usize> = args
        .flag("first")
        .map(|n| parse_num(n, "--first"))
        .transpose()?;
    let opts = PreviewOptions { max_cell };

    let result = match (kind.as_str(), first) {
        ("abr", None) => preview::preview_abr(bytes, opts),
        ("abr", Some(n)) => preview::preview_abr_first_available(bytes, opts, n),
        ("brush", None) => preview::preview_brush(bytes, opts),
        ("brush", Some(n)) => preview::preview_brush_first_available(bytes, opts, n),
        ("brushset", None) => preview::preview_brushset(bytes, opts),
        ("brushset", Some(n)) => preview::preview_brushset_first_available(bytes, opts, n),
        (other, _) => return Err(format!("unknown kind {other}")),
    };
    let head = json!({ "kind": kind, "max_cell": max_cell, "first": first });
    match result {
        Ok(set) => Ok((true, set_json(&set, head, &args.out)?)),
        Err(e) => Ok((false, json!({ "call": head, "error": e.to_string() }))),
    }
}

fn parse_num<T: std::str::FromStr>(s: &str, flag: &str) -> Result<T, String> {
    s.parse()
        .map_err(|_| format!("{flag} needs a number, got {s}"))
}

fn set_json(set: &PreviewSet, head: Value, out: &Path) -> Result<Value, String> {
    let tips = out.join("tips");
    std::fs::create_dir_all(&tips).map_err(|e| format!("create tips dir: {e}"))?;

    let mut entries = Vec::new();
    let mut sheet = Vec::new();
    let (mut available, mut unavailable) = (0, 0);
    for entry in &set.entries {
        let source = entry
            .source_dimensions
            .map(|d| json!({ "width": d.width(), "height": d.height() }));
        let tip = match &entry.tip {
            TipPreview::Available(bitmap) => {
                available += 1;
                let file = format!("tips/{:03}.png", entry.index);
                let png = preview::generate_preview_png(bitmap)?;
                write(&out.join(&file), &png)?;
                sheet.push(SheetBrush {
                    name: &entry.name,
                    bitmap,
                });
                json!({ "available": { "width": bitmap.width, "height": bitmap.height, "png": file } })
            }
            TipPreview::Unavailable(reason) => {
                unavailable += 1;
                json!({ "unavailable": format!("{reason:?}") })
            }
        };
        entries.push(json!({
            "index": entry.index,
            "name": entry.name,
            "tip": tip,
            "source_dimensions": source,
        }));
    }

    let config = ContactSheetConfig {
        show_names: true,
        ..ContactSheetConfig::default()
    };
    write(
        &out.join("sheet.png"),
        &preview::generate_contact_sheet_png(&sheet, &config)?,
    )?;

    Ok(json!({
        "call": head,
        "set_name": set.set_name,
        "counts": { "entries": set.entries.len(), "available": available, "unavailable": unavailable },
        "entries": entries,
        "sheet": "sheet.png",
    }))
}

fn dump(bytes: &[u8]) -> (bool, Value) {
    match abr::parse_abr(bytes) {
        Ok(pack) => match &pack.raw_desc_block {
            Some(desc) => {
                let dump = abr::dump::dump_descriptors(desc);
                let value = serde_json::to_value(&dump).expect("dump serializes");
                (true, value)
            }
            None => (true, json!({ "no_desc_block": pack.version.to_string() })),
        },
        Err(e) => (false, json!({ "error": e.to_string() })),
    }
}
