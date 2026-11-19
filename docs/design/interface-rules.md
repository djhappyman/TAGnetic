# Interface rules

For corner, spacing, typography or font changes, read the shared [Visual system](visual-system.md) and edit the named style owner.

Current direction agreed during the Settings and workbench restyles. Apply these rules when
iterating on the personal interface; older design briefs are historical context.

- Use the shared [surface palettes](palettes.md), typography and familiar control
  shapes. A Theme choice applies coordinated surfaces and a stronger primary
  from the same colour family in both modes. Keep older custom preferences
  stored for recovery when migrating to complete presets.
- Use spacing, tone and rounded surfaces to group content. Avoid outlining every
  field, row or section. Add a divider only when it marks a useful boundary.
- Use the shared `--input-radius` for standard inputs, selects and compound
  fields. It is currently 10px. Keep action buttons' established pill shapes.
- Give a focused field one clean accent outline. Compound fields own that
  outline around the whole group; avoid a second frame on the inner input.
  Use the error colour for invalid fields.
- Prefer a quiet filled input over a strong resting border when its surrounding
  surface gives enough contrast. Retain clear hover, focus and error states.
- Shadows represent depth and use dark translucent colours in both themes.
  Never derive a shadow from the text colour, which creates a pale dark-mode glow.
- Footer action buttons have no borders or inset shadows. Retain visible keyboard
  focus outlines, clear primary actions and disabled states.
- Keep a quiet border around the outer dialog to separate it from the app.
  Inset a recessed content area from the outer edge and round its corners. Avoid
  combining that surface change with a redundant separator line.
- Build hierarchy with page headings, section headings, labels and concise help.
  Keep related help close to its setting. Avoid adding a card around every group.
- Keep Tags, Online, Rename, Duplicates and Export beside the logo with a theme-coloured
  rounded fill on the selected tab, matching the sidebar tabs without underline rules. Put the flexible library search after the tabs, separated by quiet dividers;
  keep search presets inside its trailing edge.
  Tags owns manual sources; Online opens release search without the manual subtabs.
- Keep the file pane's outer bottom edge fixed when moving search into the header.
  Rows fill the freed space; an 8px bottom inset and short 8px fade soften the rounded edge.
- Keep the folder bar with the file table and common commands in the toolbar.
  Keep the established panel arrangement fixed, with a 420px desktop overlay sidebar and collapse in place of pane resizing. Keep table geometry fixed underneath it, reserving terminal horizontal scroll space so every column remains reachable; move only small controls and feedback into the uncovered area. Retain the
  Transformations source tab without exposing panel-movement menus.
- Align the permanent player beneath the file pane as the docks resize. Anchor
  Preview and Apply beneath the inspector; keep the draft-clear control in the
  bottom-pinned staged summary, visible only while edits exist. Keep Preview and Apply visible and disabled while there is no work. Put the affected file count
  in the primary action's natural-language label. Keep feedback from resizing
  the table during an active edit.
- Grouping leads with Folder, Album, Album Artist, Artist, Disc, Year, Genre and Media;
  secondary fields live under More. Reuse each grouping icon in the menu and table.
  Keep icons muted, counts in a themed chip immediately after the group name, and folder
  labels relative to the shared library prefix, with full paths available on hover.
- Use barely different row fills without divider borders; keep selection, staged
  changes, invalid values and keyboard focus distinct. Keep artwork centred.
- Preserve visible labels, keyboard access, selection and validation feedback.
  Check both themes and narrow windows. Quiet styling must stay discoverable.

These are shared visual defaults, not a reason to remove meaningful table,
selection, drag-position or error boundaries. Apply changes in reviewable steps.
