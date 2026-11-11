//! Shared bibliography formatting for desktop and Android.

/// Resolve the selected cite against its actual bibliography binding and
/// include Hayagriva's full reference entries, not just its short label.
pub fn bibliography_detail(text: &str, number: u64) -> Result<(String, Vec<String>), String> {
    use mathed_core::markers::{Arg, PropKind, ReferenceKind};
    use mathed_core::semantics::SemanticIndex;
    let scan = mathed_core::markers::scan(text);
    let references = super::model::citation_references(&scan);
    let reference = references.iter().find(|r| r.numbers.contains(&number))
        .ok_or_else(|| "Citation no longer exists".to_string())?;
    if !matches!(&reference.kind, ReferenceKind::Bibliography { .. }) {
        return Err("This is a document reference".to_string());
    }
    let stmt = &scan.stmts[reference.stmt_idx];
    let mut keys = Vec::new();
    let mut bib_name = None;
    let mut cite_style = None;
    for arg in &stmt.args {
        let Arg::Literal { text, .. } = arg else { continue };
        let value = text.trim();
        if let Some((name, value)) = value.split_once(':') {
            let value = value.trim().trim_matches('"').to_owned();
            match name.trim() {
                "bib" => bib_name = Some(value),
                "style" => cite_style = Some(value),
                _ => keys.push(text.trim().trim_matches('"').to_owned()),
            }
        } else {
            keys.push(value.trim_matches('"').to_owned());
        }
    }
    if keys.is_empty() { return Err("Citation has no bibliography key".to_string()); }
    let statements = SemanticIndex::scan_biblio_statements(text);
    let bibliographies: Vec<_> = statements.iter().filter(|s| s.kind == PropKind::Bibliography).collect();
    let bib = match bib_name.as_deref() {
        Some(name) => bibliographies.iter().find(|b| b.name.as_deref() == Some(name)).copied(),
        None if bibliographies.len() == 1 => bibliographies.first().copied(),
        _ => None,
    }.ok_or_else(|| "No matching bibliography is bound to this citation".to_string())?;
    let library = if bib.format.as_deref() == Some("bibtex") {
        mathed_biblio::load_bibtex(&bib.body_text)
    } else { mathed_biblio::load_yaml(&bib.body_text) }.map_err(|e| e.to_string())?;
    let style_name = cite_style.as_deref().or(bib.style.as_deref()).unwrap_or(mathed_biblio::DEFAULT_STYLE);
    let style = hayagriva::archive::ArchivedStyle::by_name(style_name)
        .ok_or_else(|| format!("Unknown citation style: {style_name}"))?;
    let hayagriva::citationberg::Style::Independent(style) = style.get() else {
        return Err(format!("Citation style {style_name} is dependent; select its parent style"));
    };
    let locale = hayagriva::citationberg::LocaleFile::from_xml(include_str!("../../../../emthin-android/assets/locales-en-US.xml"))
        .map_err(|e| format!("Invalid CSL locale: {e}"))?.into();
    let locales = [locale];
    let selected = keys.iter().map(|key| library.get(key)
        .ok_or_else(|| {
            let suggestions = bibliography_suggestions(&library, key);
            if suggestions.is_empty() { format!("Unknown bibliography entry key: {key}") }
            else { format!("Unknown bibliography entry key: {key}. Possible keys: {}", suggestions.join(", ")) }
        }))
        .collect::<Result<Vec<_>, _>>()?;
    let items = selected.iter().map(|&entry| hayagriva::CitationItem::with_entry(entry)).collect();
    let request = hayagriva::CitationRequest::new(items, &style, None, &locales, None);
    let mut citation = String::new();
    hayagriva::standalone_citation(request).write_buf(&mut citation, hayagriva::BufWriteFormat::Plain)
        .map_err(|e| e.to_string())?;
    let mut driver = hayagriva::BibliographyDriver::new();
    for entry in selected {
        driver.citation(hayagriva::CitationRequest::new(
            vec![hayagriva::CitationItem::with_entry(entry)], &style, None, &locales, None));
    }
    let result = driver.finish(hayagriva::BibliographyRequest { style: &style, locale: None, locale_files: &locales });
    let bibliography = result.bibliography.ok_or_else(|| "This style produces no bibliography".to_string())?;
    let entries = bibliography.items.into_iter().map(|item| {
        let mut text = String::new();
        item.content.write_buf(&mut text, hayagriva::BufWriteFormat::Plain).map_err(|e| e.to_string())?;
        Ok(text)
    }).collect::<Result<Vec<_>, String>>()?;
    Ok((citation, entries))
}

/// Offer, but never silently select, nearby author/year entries. This follows
/// markdown_bib_processor's surname + two-digit-year lookup while retaining
/// explicit keys when more than one entry could match.
fn bibliography_suggestions(library: &hayagriva::Library, key: &str) -> Vec<String> {
    let split = key.find(|c: char| c.is_ascii_digit()).unwrap_or(key.len());
    let (author, rest) = key.split_at(split);
    if author.is_empty() || rest.len() < 2 || !rest.as_bytes()[..2].iter().all(u8::is_ascii_digit)
        || !author.chars().all(|c| c.is_alphabetic()) { return Vec::new(); }
    let year = &rest[..2];
    let author = author.to_lowercase();
    let mut ranked = Vec::new();
    for entry in library.iter() {
        let Some(person) = entry.authors().and_then(|authors| authors.first()) else { continue };
        let Some(date) = entry.date() else { continue };
        if format!("{:02}", date.year.rem_euclid(100)) != year { continue; }
        let candidate = person.name.to_lowercase();
        let mut previous: Vec<usize> = (0..=candidate.chars().count()).collect();
        for (i, a) in author.chars().enumerate() {
            let mut row = vec![i + 1];
            for (j, b) in candidate.chars().enumerate() {
                row.push((previous[j + 1] + 1).min(row[j] + 1).min(previous[j] + usize::from(a != b)));
            }
            previous = row;
        }
        let distance = *previous.last().unwrap_or(&usize::MAX);
        if distance <= 2 { ranked.push((distance, entry.key().to_string())); }
    }
    ranked.sort();
    ranked.into_iter().take(3).map(|(_, key)| key).collect()
}


/// A citation overlay is derived presentation state; it never changes the CRDT.
#[derive(Default)]
pub struct CitationPopup {
    stack: Vec<PopupLevel>,
    pub raster: Option<PopupRaster>,
    pub revision: u64,
    document_revision: u64,
}

struct PopupLevel {
    number: u64,
    /// The cited document body is the scope for nested citations.
    child_scope: Option<String>,
    markup: String,
}

pub struct PopupRaster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl CitationPopup {
    pub fn is_open(&self) -> bool { !self.stack.is_empty() }

    pub fn close(&mut self) -> bool {
        if self.stack.pop().is_none() { return false; }
        self.refresh();
        true
    }

    pub fn clear(&mut self) {
        if !self.stack.is_empty() {
            self.stack.clear();
            self.refresh();
        }
    }

    /// Toggle a numbered citation; nested numbers resolve within the open
    /// document reference, as they do in mathed's citation stack.
    pub fn toggle(&mut self, document: &str, document_revision: u64, number: u64) -> Result<(), String> {
        if self.is_open() && self.document_revision != document_revision { self.clear(); }
        self.document_revision = document_revision;
        if self.stack.last().is_some_and(|top| top.number == number) {
            self.close();
            return Ok(());
        }
        let scope = match self.stack.last() {
            Some(level) => level.child_scope.as_deref().ok_or("This citation contains no nested document")?,
            None => document,
        };
        let scan = mathed_core::markers::scan(scope);
        let normalized = super::model::citation_references(&scan);
        let target = normalized.iter().find(|r| r.numbers.contains(&number))
            .ok_or_else(|| format!("No citation [{number}]"))?;
        let upstream = mathed_core::markers::scan_references(&scan);
        let upstream_number = upstream.iter().find(|r| r.stmt_idx == target.stmt_idx)
            .and_then(|r| r.numbers.first()).copied().ok_or("Citation no longer exists")?;
        let body = mathed_mini::cite_popup::resolve_popup_body(scope, upstream_number)
            .ok_or("Citation no longer exists")?;
        let (child_scope, body_markup) = match body {
            mathed_mini::cite_popup::PopupBody::DocumentRef { body_text, body_markup } => {
                let markup = if body_text.is_empty() { body_markup } else {
                    let scan = mathed_core::markers::scan(&body_text);
                    let segments = mathed_core::markers::resolve_segments(&scan);
                    let options = mathed_core::TransformOptions {
                        references: super::model::citation_references(&scan), ..Default::default()
                    };
                    mathed_core::transform::to_render_text(&body_text, &scan, &segments, &options).text
                };
                (Some(body_text), markup)
            }
            mathed_mini::cite_popup::PopupBody::Bibliography { .. } => {
                let detail = bibliography_detail(scope, number)
                    .map(|(citation, entries)| format!("{}\n\n{}", citation, entries.join("\n\n")))
                    .unwrap_or_else(|error| error);
                let quoted = serde_json::to_string(&detail).map_err(|e| e.to_string())?;
                (None, format!("#text({quoted})"))
            }
        };
        self.stack.push(PopupLevel { number, child_scope, markup: body_markup });
        self.refresh();
        Ok(())
    }

    pub fn clear_if_stale(&mut self, document_revision: u64) {
        if self.is_open() && self.document_revision != document_revision { self.clear(); }
    }

    fn refresh(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.raster = self.stack.last().and_then(|level| {
            let markup = format!("#set text(fill: white, size: 14pt, font: \"DejaVu Sans Mono\")\n#text(fill: rgb(\"#b7d5ea\"))[Citation [{}] · Esc close]\n\n{}", level.number, level.markup);
            let image = mathed_mini::render_markup(&markup, 400.0).ok()?;
            Some(PopupRaster { width: image.width, height: image.height, rgba: image.data })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_citation_opens_and_closes_without_changing_text() {
        let text = "#a reference text #b \\cite(#a,#b)";
        let mut popup = CitationPopup::default();
        popup.toggle(text, 1, 1).unwrap();
        assert!(popup.is_open());
        assert!(popup.raster.as_ref().is_some_and(|r| r.width > 0 && r.height > 0));
        assert!(popup.close());
        assert!(!popup.is_open());
        assert!(popup.raster.is_none());
    }

    #[test]
    fn popup_closes_when_its_source_document_changes() {
        let mut popup = CitationPopup::default();
        popup.toggle("#a body #b \\cite(#a,#b)", 7, 1).unwrap();
        popup.clear_if_stale(7);
        assert!(popup.is_open());
        popup.clear_if_stale(8);
        assert!(!popup.is_open());
    }

    #[test]
    fn bibliography_uses_locale_for_year_and_full_entry() {
        let text = "#a source:\n  type: Book\n  title: A Book\n  author: Smith, Jane\n  date: 2020\n#b \\bibliography(#a,#b, refs) \\cite(\"source\", bib: \"refs\")";
        let (citation, entries) = bibliography_detail(text, 1).unwrap();
        assert!(citation.contains("2020"), "{citation}");
        assert!(entries[0].contains("A Book"));
    }
}
