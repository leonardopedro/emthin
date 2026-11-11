# emthin pad

The pad displays the rendered mathed document as its editing surface. Tap text
to open its editable source below the document; press **Esc** to close it.
The caret moves over the rendered text with the arrow keys and follows the
source caret while editing. Moving beyond the source closes its columns and
continues in the rendered document. Tap an empty document to start writing.
The shortcut guide can also be tapped on a touchscreen. Typing an
unescaped `#` creates a fresh marker named using mathed's RFC 1751 word table;
type `\#` for a literal hash. Text and Loro marks are saved together between
launches. Press **Tab** to show or hide markers in the document and source;
the columns for the text at the caret open and close with them. The **Tab Show
markers** shortcut provides the same control on touchscreens.

Press **Ctrl+F** (or foot's **Ctrl+Shift+R**) to search the document. Matches
are highlighted in the rendered page; **Ctrl+S** and **Ctrl+R** select the
next and previous match, **Enter** accepts it, and **Esc** returns to the
original caret. Lowercase queries ignore case; queries containing uppercase
letters match case. **Ctrl+Left/Right** moves by mathed's word boundaries.
Inside `$...$`, mathed's ASCII-to-Unicode completions are shown as a hint;
typing a delimiter commits the pending completion. Ordinary prose is unchanged.
Tap a numbered citation to expand its referenced document text or its
Hayagriva-formatted citation and bibliography entry. **Ctrl+K** lists citations;
**Ctrl+1** through **Ctrl+9** open numbered entries. Document references may
contain further expandable citations. Tap a collapsed translator or template
title to expand its code; **Ctrl+L** folds or unfolds the block at the caret.
On the rendered page, **Shift+arrows** selects text and **Ctrl+A/C/X/V**
selects all, copies, cuts and pastes. Hidden marker names are omitted from a
copy and retained when visible text is cut.
**Ctrl+O** opens a Loro `.mathed` snapshot or UTF-8 Typst source. **Ctrl+S**
saves a Loro snapshot, including modifier marks; an opened plain text file is
saved as a new `.mathed` file so its marks are preserved.

To use marked text as a modifier, tap the corresponding rendered text. The
panel below the document shows the
editable source text, modifiers affecting it, and text it modifies. Each
comparison is rendered with the selected modifier removed on the right.

In the right-hand field, enter pairs of marker names separated by commas,
such as `a at, be e`. Each pair is applied when the caret moves out of it,
including when the field loses focus. Each pair
identifies the text between those two markers; `#` prefixes are optional.
The links are Loro marks and follow the named markers as the text changes;
they do not insert property statements into the document. Clear the list to
remove these links. Existing selection-based links without matching marker
pairs remain listed separately and can be removed there. Applications
participate in Undo and Redo, can overlap, and may cover several ranges.

Modifier text follows Typst's content-function convention. `bold` is an alias
for `strong`, and `italic` is an alias for `emph`; Typst function expressions
such as `text.with(fill: red)` are also accepted. The expression is applied to
the selected content. A malformed expression produces a rendering diagnostic
while the source remains editable.

The Android pad does not yet expose every mathed feature; in particular, this
interface does not implement its kernel execution and notebook workflows.
