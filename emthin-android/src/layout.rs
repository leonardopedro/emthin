//! Retained worker layout for the Android page stream.
//!
//! The Java render worker owns one layout at a time. Preparation compiles the
//! captured snapshot once and keeps the Typst document, source map and lazily
//! built glyph indexes. Later page requests rasterize from that layout without
//! importing the snapshot or compiling again. Overlay-only changes never call
//! this module, so caret/selection/search updates cost no layout work.
//!
//! Nothing here touches the live `DocModel` handle: `prepare` imports the
//! immutable snapshot bytes it is given.

use std::cell::RefCell;

use mathed_core::glyphs::{build_glyph_index, GlyphIndex};
use mathed_core::transform::{OffsetMap, TransformOptions};
use mathed_mini::{theme_prelude, MiniWorld};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use typst_layout::PagedDocument;

use crate::model::DocModel;
use crate::{byte_to_utf16, png_page};

struct RetainedLayout {
    id: String,
    snapshot_digest: [u8; 32],
    world: MiniWorld,
    document: PagedDocument,
    map: OffsetMap,
    theme_len: usize,
    text_size: f32,
    caption_start: Option<usize>,
    glyphs: Vec<Option<GlyphIndex>>,
    warnings: Vec<String>,
    diagnostics: Vec<Value>,
}

thread_local! {
    /// Thread-confined by construction: only the one Java render worker calls
    /// these entry points, and a later layout preparation replaces this value.
    static RETAINED: RefCell<Option<RetainedLayout>> = const { RefCell::new(None) };
}

#[cfg(test)]
thread_local! {
    static COMPILES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn compiles() -> u64 {
    COMPILES.with(|count| count.get())
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn compile(
    doc_text: &str,
    opts: &TransformOptions,
    text_size: f32,
    model: &DocModel,
    capture: &str,
) -> Result<(MiniWorld, PagedDocument, OffsetMap, usize, Vec<Value>), (String, Vec<Value>)> {
    if !text_size.is_finite() || text_size <= 0.0 {
        return Err((
            "Reading size must be a positive number".to_string(),
            Vec::new(),
        ));
    }
    let render = mathed_mini::doc_to_render_with(doc_text, opts);
    let map = render.map;
    let prelude = theme_prelude(text_size);
    let world = MiniWorld::new(format!("{prelude}{}", render.text));
    let warned = typst::compile::<PagedDocument>(&world);
    let mut diagnostics = Vec::new();
    for (index, warning) in warned.warnings.iter().enumerate() {
        diagnostics.push(crate::diagnostics::typst_diagnostic(
            &world,
            warning,
            &map,
            prelude.len(),
            model,
            capture,
            index,
        ));
    }
    let document = match warned.output {
        Ok(document) => document,
        Err(errors) => {
            let offset = diagnostics.len();
            for (index, error) in errors.iter().enumerate() {
                diagnostics.push(crate::diagnostics::typst_diagnostic(
                    &world,
                    error,
                    &map,
                    prelude.len(),
                    model,
                    capture,
                    offset + index,
                ));
            }
            return Err((
                "Could not typeset the document (Eval)".to_string(),
                diagnostics,
            ));
        }
    };
    #[cfg(test)]
    COMPILES.with(|count| count.set(count.get() + 1));
    Ok((world, document, map, prelude.len(), diagnostics))
}

fn presentation_options(
    doc_text: &str,
    show_markers: bool,
    expanded_pairs: &[(String, String)],
    model: &DocModel,
) -> (TransformOptions, Option<usize>) {
    let scan = mathed_core::markers::scan(doc_text);
    let segments = mathed_core::markers::resolve_segments(&scan);
    let expand = segments
        .iter()
        .filter(|s| {
            expanded_pairs
                .iter()
                .any(|(a, b)| &s.start_id == a && &s.end_id == b)
        })
        .filter_map(|s| s.span.clone())
        .collect();
    let reveal = if show_markers {
        scan.markers.iter().map(|m| m.range.clone()).collect()
    } else {
        Vec::new()
    };
    let options = TransformOptions {
        reveal,
        expand,
        references: crate::model::citation_references(&scan),
        block_splices: crate::modifiers::typst_splices(model),
        ..TransformOptions::default()
    };
    let caption_start = segments
        .iter()
        .filter_map(|s| s.span.as_ref().map(|span| span.start))
        .min();
    (options, caption_start)
}

fn metadata(layout: &RetainedLayout) -> Value {
    let pages: Vec<Value> = layout
        .document
        .pages()
        .iter()
        .map(|page| {
            let size = page.frame.size();
            json!({
                "widthPt": size.x.to_pt(),
                "heightPt": size.y.to_pt(),
            })
        })
        .collect();
    json!({
        "layoutId": layout.id,
        "total": layout.document.pages().len(),
        "captionStart": layout.caption_start,
        "pages": pages,
        "textSize": layout.text_size,
        "warnings": layout.warnings,
        "diagnostics": layout.diagnostics,
    })
}

/// Compile or reuse the one retained layout for `layout_id`.
pub fn prepare(
    snapshot: &[u8],
    doc_text: &str,
    layout_id: &str,
    show_markers: bool,
    expanded_pairs: &[(String, String)],
    text_size: f32,
) -> Result<Value, Value> {
    let snapshot_digest = digest(snapshot);
    if let Some(value) = RETAINED.with(|slot| {
        let slot = slot.borrow();
        slot.as_ref()
            .filter(|layout| {
                layout.id == layout_id
                    && layout.snapshot_digest == snapshot_digest
                    && (layout.text_size - text_size).abs() < f32::EPSILON
            })
            .map(metadata)
    }) {
        return Ok(value);
    }

    let model = DocModel::from_snapshot(snapshot)
        .ok()
        .filter(|model| model.text() == doc_text)
        .ok_or_else(|| json!({"error": "Could not restore document for preview"}))?;
    let capture = layout_id;
    let mut diagnostics = crate::diagnostics::source_diagnostics(&model, capture);
    let (mut options, caption_start) =
        presentation_options(doc_text, show_markers, expanded_pairs, &model);

    let mut warnings = Vec::new();
    let (world, document, map, theme_len, typst) = match compile(
        doc_text, &options, text_size, &model, capture,
    ) {
        Ok(compiled) => compiled,
        Err((original, failures)) => {
            let pairs: std::collections::BTreeSet<(String, String)> =
                crate::modifiers::links(&model)
                    .into_iter()
                    .map(|link| (link.start_id, link.end_id))
                    .collect();
            if pairs.is_empty() {
                diagnostics.extend(failures);
                return Err(json!({"error": original, "diagnostics": diagnostics}));
            }
            // Isolate failing modifiers against this exact snapshot. The
            // successful exclusion set is retained with the layout, so later
            // pages do not retry any of these compiles.
            let mut excluded: std::collections::HashSet<_> = pairs.iter().cloned().collect();
            options.block_splices.clear();
            let mut compiled = compile(doc_text, &options, text_size, &model, capture).map_err(
                |(message, more)| {
                    diagnostics.extend(failures);
                    diagnostics.extend(more);
                    json!({"error": message, "diagnostics": diagnostics})
                },
            )?;
            for pair in pairs {
                excluded.remove(&pair);
                options.block_splices =
                    crate::modifiers::typst_splices_excluding(&model, 0..model.len(), &excluded);
                match compile(doc_text, &options, text_size, &model, capture) {
                    Ok(next) => compiled = next,
                    Err((_, more)) => {
                        excluded.insert(pair.clone());
                        diagnostics.extend(more);
                        let name = crate::modifiers::source_range(&model, &pair.0, &pair.1)
                            .map(|r| model.text()[r].trim().to_owned())
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| format!("#{} … #{}", pair.0, pair.1));
                        let source = crate::modifiers::source_range(&model, &pair.0, &pair.1);
                        let index = diagnostics.len();
                        diagnostics.push(json!({
                            "id": format!("{capture}:modifier-application:{index}"),
                            "severity": "error",
                            "code": "modifier-application",
                            "message": format!(
                                "Could not apply modifier ‘{name}’; expected a valid Typst content function."
                            ),
                            "captureId": capture,
                            "start": source.as_ref().map(|r| byte_to_utf16(&model.text()[..], r.start)),
                            "end": source.as_ref().map(|r| byte_to_utf16(&model.text()[..], r.end)),
                            "markerPair": [pair.0, pair.1],
                            "provenance": "owner",
                        }));
                        warnings.push(format!(
                            "Could not apply modifier ‘{name}’; expected a valid Typst content function."
                        ));
                    }
                }
            }
            compiled
        }
    };
    diagnostics.extend(typst);
    diagnostics.sort_by(|a, b| {
        let a = (
            a["severity"].as_str().unwrap_or(""),
            a["start"].as_u64().unwrap_or(u64::MAX),
        );
        let b = (
            b["severity"].as_str().unwrap_or(""),
            b["start"].as_u64().unwrap_or(u64::MAX),
        );
        a.cmp(&b)
    });
    let total = document.pages().len();
    let layout = RetainedLayout {
        id: layout_id.to_string(),
        snapshot_digest,
        world,
        document,
        map,
        theme_len,
        text_size,
        caption_start,
        glyphs: std::iter::repeat_with(|| None).take(total).collect(),
        warnings,
        diagnostics,
    };
    let value = metadata(&layout);
    RETAINED.with(|slot| *slot.borrow_mut() = Some(layout));
    Ok(value)
}

fn ensure_glyphs(layout: &mut RetainedLayout, page: usize) {
    if layout.glyphs[page].is_some() {
        return;
    }
    let frame = &layout.document.pages()[page].frame;
    let index = build_glyph_index(
        frame,
        layout.world.main_source(),
        &layout.map,
        layout.theme_len,
    );
    layout.glyphs[page] = Some(index);
}

fn text_top(layout: &RetainedLayout, page: usize) -> Option<f32> {
    let glyphs = layout.glyphs[page].as_ref()?;
    let entry = match layout.caption_start {
        Some(start) => glyphs.entries.iter().find(|e| e.doc_byte >= start),
        None => glyphs.entries.first(),
    }?;
    glyphs.bands.get(entry.band as usize).map(|band| band.top)
}

/// Resolve the terminal binding for each figure key in one document pass.
/// The page frame only carries the key, so the pad needs this side table to
/// tell a native terminal slot from an ordinary dormant/Wayland figure.
pub(crate) fn figure_bindings(
    doc_text: &str,
) -> std::collections::HashMap<String, (Option<String>, Option<String>)> {
    let scan = mathed_core::markers::scan(doc_text);
    let mut bindings = std::collections::HashMap::new();
    for segment in mathed_core::markers::resolve_segments(&scan) {
        let Some(figure) = mathed_core::figures::resolve_figure(&segment) else {
            continue;
        };
        bindings.insert(
            figure.key,
            (
                figure
                    .spec
                    .terminal
                    .as_ref()
                    .map(|reference| reference.canonical()),
                figure.spec.binding_error,
            ),
        );
    }
    bindings
}

fn page_hits(doc_text: &str, glyphs: &GlyphIndex) -> Vec<Value> {
    glyphs
        .entries
        .iter()
        .filter_map(|entry| {
            let band = glyphs.bands.get(entry.band as usize)?;
            Some(json!([
                entry.x,
                band.top,
                entry.x + entry.advance,
                band.bottom,
                byte_to_utf16(doc_text, entry.doc_byte)
            ]))
        })
        .collect()
}

/// Rasterize one page from the retained layout. `pixel_limit` caps the
/// working RGBA page; density is reduced to fit while point geometry stays
/// unchanged.
pub fn render_page(
    layout_id: &str,
    doc_text: &str,
    page: usize,
    scale: f32,
    pixel_limit: u64,
    retain_from: usize,
    retain_to: usize,
) -> Result<Value, String> {
    RETAINED.with(|slot| {
        let mut slot = slot.borrow_mut();
        let layout = slot
            .as_mut()
            .filter(|layout| layout.id == layout_id)
            .ok_or_else(|| "Render layout was replaced; request a new frame".to_string())?;
        if page >= layout.document.pages().len() {
            return Err(format!("Page {} is outside the layout", page));
        }
        ensure_glyphs(layout, page);

        let frame = &layout.document.pages()[page].frame;
        let size = frame.size();
        let mut scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        let limit = pixel_limit.max(1);
        let requested = (size.x.to_pt() as f64 * scale as f64).ceil().max(1.0)
            * (size.y.to_pt() as f64 * scale as f64).ceil().max(1.0);
        if requested > limit as f64 {
            scale = (scale as f64 * (limit as f64 / requested).sqrt()) as f32;
        }
        let image = mathed_mini::rasterize_at(frame, scale)
            .map_err(|error| format!("Could not rasterize page: {error:?}"))?;
        let (width, height) = (image.width, image.height);
        let png = png_page(width, height, &image.data)?;
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(png);
        let glyphs = layout.glyphs[page]
            .as_ref()
            .expect("glyph index built above");
        let bindings = figure_bindings(doc_text);
        let figures: Vec<Value> = mathed_core::figures::figures_in_frame(frame)
            .into_iter()
            .map(|figure| {
                let (reference, binding_error) =
                    bindings.get(&figure.key).cloned().unwrap_or((None, None));
                json!({
                    "key": figure.key,
                    "x": figure.rect.x0,
                    "y": figure.rect.y0,
                    "w": figure.rect.x1 - figure.rect.x0,
                    "h": figure.rect.y1 - figure.rect.y0,
                    "reference": reference,
                    "bindingError": binding_error,
                })
            })
            .collect();
        let value = json!({
            "page": page,
            "width": width,
            "height": height,
            "widthPt": size.x.to_pt(),
            "heightPt": size.y.to_pt(),
            "scale": scale,
            "textTop": text_top(layout, page),
            "hasFigures": !figures.is_empty(),
            "figures": figures,
            "hits": page_hits(doc_text, glyphs),
            "png": encoded,
        });
        let from = retain_from.min(page);
        let to = retain_to
            .max(page)
            .min(layout.glyphs.len().saturating_sub(1));
        for (index, glyphs) in layout.glyphs.iter_mut().enumerate() {
            if index < from || index > to {
                *glyphs = None;
            }
        }
        Ok(value)
    })
}

/// Locate a UTF-16 source range without rasterizing any intervening page.
pub fn locate_range(
    layout_id: &str,
    doc_text: &str,
    start_utf16: usize,
    end_utf16: usize,
) -> Result<Value, String> {
    let start = crate::utf16_to_byte(doc_text, start_utf16);
    let end = crate::utf16_to_byte(doc_text, end_utf16.max(start_utf16)).max(start);
    RETAINED.with(|slot| {
        let mut slot = slot.borrow_mut();
        let layout = slot
            .as_mut()
            .filter(|layout| layout.id == layout_id)
            .ok_or_else(|| "Render layout was replaced; request a new frame".to_string())?;
        let total = layout.document.pages().len();
        for page in 0..total {
            // Build only the page being searched. Keeping every page's glyph
            // index here would defeat the per-page residency bound before the
            // first raster request, especially for a jump to a late page.
            let had_index = layout.glyphs[page].is_some();
            ensure_glyphs(layout, page);
            let found = layout.glyphs[page].as_ref().and_then(|glyphs| {
                glyphs
                    .entries
                    .iter()
                    .find(|entry| entry.doc_byte >= start && (start == end || entry.doc_byte < end))
                    .map(|entry| byte_to_utf16(doc_text, entry.doc_byte))
            });
            if let Some(found_start) = found {
                return Ok(json!({
                    "page": page,
                    "found": true,
                    "start": found_start,
                    "end": end_utf16.max(start_utf16),
                }));
            }
            if !had_index {
                layout.glyphs[page] = None;
            }
        }
        Ok(json!({
            "page": total.saturating_sub(1),
            "found": false,
            "start": start_utf16,
            "end": end_utf16.max(start_utf16),
        }))
    })
}
