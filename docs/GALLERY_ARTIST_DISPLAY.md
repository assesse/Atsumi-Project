# Gallery artist display

Gallery cards keep the existing primary `artist` field for grouping and selection rules. The additive, optional `artists` list contains known participants; it is not inferred from an Auto Find match, title, or a delimited primary name.

- Detail cards fit up to three names on one line, favorite artists first, with `+N명` for hidden participants.
- Compact cards show one favorite-first name and `외 N명` without increasing card height or changing favorite-tag badges.
- Hover or click the count to open a viewport-clamped participant popover. Names search; stars toggle each artist's favorite. The popover keeps source order so rows do not jump after a star changes.
- Ctrl/Shift clicks retain gallery-selection behavior. Excluded/blinded cards cannot expose the popover. Keyboard activation, Escape, outside click, and focus departure are supported.

Migration 46 preserves participant lists in Auto Find and backfills only available local summaries/owned-gallery artist records. Download list lookups read local artists after paging; there is no new all-library or network scan. Previously cached rows with only a primary artist remain unknown until normal metadata retrieval (for example opening Detail) supplies the full list. Fresh summaries also enrich the corresponding saved Auto Find records, surviving restart and summary-cache cleanup.

Floating Detail also shows all known participants in its artist metadata box, with a participant count and favorite-first chips. Each artist keeps its own search and favorite action. Legacy records without a full list still display the primary artist without inventing a count.

This does not change Auto Find recommendation/grouping logic, the primary artist identity, or random selection.
