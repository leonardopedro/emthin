# emthin pad

The pad displays the rendered mathed document as its editing surface. Tap text
to open its editable source below the document; press **Esc** to close it.
The caret moves over the rendered text with the arrow keys and follows the
source caret while editing. Moving beyond the source closes its columns and
continues in the rendered document. Tap an empty document to start writing.
The shortcut guide can also be tapped on a touchscreen. Typing an
unescaped `#` creates a fresh marker named using mathed's RFC 1751 word table;
type `\#` for a literal hash. IME candidates stay in the source field until
they are committed, and rendered movement, selection and deletion follow
mathed's grapheme boundaries. Pages are laid out once on a background worker for
each document/marker/fold state: page placeholders keep the full document, but
only the visible page and one neighbour keep bitmaps; distant pages are
rehydrated from the retained layout on demand. Caret and search overlays only
repaint, and PNG decoding stays off the UI thread. **A−/A+** change the reading
size (8–32 pt) and **Zoom−/Zoom+** magnify the page from 1× to 4× with
horizontal panning; both settings persist. The selection's fixed end and the
reading position are stored as Loro cursor anchors, so they follow edits and
survive relaunch. Text and Loro marks are
saved together between launches. Press **Tab** to show or hide markers in the document and source;
the columns for the text at the caret open and close with them. The **Tab Show
markers** shortcut provides the same control on touchscreens.

Press **Ctrl+F** (or foot's **Ctrl+Shift+R**) to search the document. The
scope button selects **Text** (prose and math, excluding markers and property
statements), **Source** (all source, including structure), or **Selection**
(frozen to the non-empty selection). Matches are highlighted in the rendered
page and listed as bounded 200-result pages with 80-grapheme snippets;
**Ctrl+S** and **Ctrl+R** select the next and previous match while Find is
open; the Save shortcut changes to **^S Find next (Save: close Find)** so the
context is visible. While Find is open, the main status repeats the current/total match
count. **Enter** accepts a match, and **Esc** or Cancel restores the original
selection, folds and viewport. Lowercase queries ignore case; queries containing uppercase letters
match case. **F3/Shift+F3** reuse the last accepted query.

**Ctrl+H** opens the Replace row. **Replace** applies one eligible match;
**Replace all…** first shows the replaceable/protected counts and up to 20
before/after snippets, then applies one checked plan. Marker and property
statement ranges are never replaced, plans over 10,000 edits are refused, and
one Undo reverses the whole batch. **Ctrl+Left/Right** moves by mathed's word
boundaries.
Inside `$...$`, mathed's ASCII-to-Unicode completions are shown as a hint;
typing a delimiter commits the pending completion. Ordinary prose is unchanged.
Tap a numbered citation to expand its referenced document text or its
Hayagriva-formatted citation and bibliography entry. Citation details show
the source snippet and a breadcrumb for nested document references; repeated
references stop with a source action, and nesting is limited to 16 levels.
Style and locale selectors use the shared CSL choices, apply only to detail
formatting, persist per document, and **Reset style and locale** returns to
the document/default choices. The References list shows resolved/unresolved
status and source snippets and filters by number, label, key or snippet.
**Ctrl+K** opens the **Navigate** panel on References; **Ctrl+G** opens its
Sections tab.
Definitions list their resolved use count, open their source, and expose
Go to definition/Find uses/Rename definition in the source inspector. Rename
validates one math identifier, rejects duplicate or unresolved names outside
its plan, and applies the definition plus resolved uses as one grouped Undo
step. **Ctrl+1** through **Ctrl+9** open numbered entries.
Accepted search, section/definition/use and reference jumps enter a bounded
30-place return history; use **Alt+Left/Right** or the toolbar Back/Forward
actions. New/Open clear that history. Document references may
contain further expandable citations. Tap a collapsed translator or template
title to expand its code; **Ctrl+L** folds or unfolds the block at the caret.
On the rendered page, **Shift+arrows** selects text and **Ctrl+A/C/X/V**
selects all, copies, cuts and pastes. Long-press selects a word through
mathed's word boundaries and dragging extends it; start/end handles remain
visible and can be dragged independently. A contextual
Copy/Cut/Paste/Select-all bar provides the same touch-only path. Hidden marker
names are omitted from a copy and retained when visible text is cut. Loaded
pages expose their text as virtual accessibility nodes in reading order, with
source-backed bounds and actions for citations, folds and source editing;
unloaded pages request only their own page when accessibility focus reaches
them.
**Ctrl+O** opens a Loro `.mathed` snapshot or UTF-8 Typst source. Outside
Find, **Ctrl+S** saves a Loro snapshot, including modifier marks;
**Ctrl+Shift+S** saves a copy
without changing the active file. An opened plain text file is saved as a new
`.mathed` file so its marks are preserved. Saving over a file that changed
externally offers a copy, an intentional replacement, or cancel.

To use marked text as a modifier, tap the corresponding rendered text. The
panel below the document shows the editable source text, modifiers affecting
it, and text it modifies. Each link row names both marker pairs, shows a
snippet and resolved state, and can jump to either endpoint or remove the
link in either direction. Only one comparison row expands at a time; its
right side renders the selected modifier removed. Six row results are cached
without touching the document.

In the right-hand field, enter pairs of marker names separated by commas,
such as `a at, be e`. `#` prefixes are optional, and the field suggests the
markers that can complete the pair being typed. A complete pair is applied
when the caret leaves it, when the field loses focus, or when the IME's **Done** action
is used; an active partial pair is preserved. Missing or reversed endpoints
report an error and leave existing links unchanged, while duplicate pairs
collapse. The links are Loro marks and follow the named markers as the text
changes; they do not insert property statements into the document. Clear the
list to remove these links. Existing selection-based links without matching
marker pairs remain listed separately and can be removed there. Applications
participate in Undo and Redo, can overlap, and may cover several ranges. At
600 dp and wider, source, relationships and comparisons use aligned columns;
below that they stack, and **Show document** keeps the editor reachable.

Modifier text follows Typst's content-function convention. `bold` is an alias
for `strong`, and `italic` is an alias for `emph`; Typst function expressions
such as `text.with(fill: red)` are also accepted. The expression is applied to
the selected content. A malformed expression produces a rendering diagnostic
while the source remains editable.

The status line names the document (or **Untitled**), shows whether it is saved,
unsaved or currently being written, and reports the latest operation or
diagnostic summary. Problems found during layout are counted there. Tap the
count to open the diagnostics list; each entry identifies whether it points at exact
copied source, at the modifier or statement that produced the text, or at the
document as a whole. Selecting an exact or owner entry opens that source and
**Back** returns to the document. Unresolved semantic names, incomplete marker
pairs and kernel/execution statements are reported here too; execution
statements say explicitly that execution is unavailable on this Android
frontend, and their source remains editable.

The Android pad does not yet expose every mathed feature; in particular, this
interface does not implement its kernel execution and notebook workflows.

The [Android development plan](DEVELOPMENT_PLAN.md) records the current review,
priorities, and scoped implementation tasks. The [feature specification](FEATURE_SPEC.md)
defines the selected future behavior; the [sibling study](SIBLING_STUDY.md)
records its source evidence and adaptation decisions.

The [native terminal figure specification](../docs/HERDR_INTEGRATION_SPEC.md)
and [implementation plan](../docs/HERDR_INTEGRATION_PLAN.md) define the next
Android integration with remote Herdr Core sessions and the OpenShell fleet.
