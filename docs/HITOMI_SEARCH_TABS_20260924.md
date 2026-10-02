# Hitomi search behavior — 2026-09-24

## Implemented

- Global required/excluded tags apply to Auto Find snapshots, cards, counts, selection and random-open candidates, including already saved discoveries. All required tags must match; any excluded tag rejects the candidate. Namespaces are distinct and spaces/underscores are normalized.
- Keep discovery records and incremental checkpoints intact. Settings changes reproject saved metadata; removing a rule restores eligible candidates without another full network scan or permanent album exclusions.
- Every submitted Explore search gets a normal, named, closable tab from the first search onward. No special protected root/“전체 탐색” tab. Equivalent open searches are reused, including pending searches; refresh/retry replaces only its own request.
- Up to 64 tabs, without silently evicting older work. The native query cache holds 128 snapshots to cover open tabs and refresh headroom. Existing shared network concurrency/rate limits remain in force.
- Submission validity is per tab/request, not per currently focused tab. Requested page loads and adjacent metadata prefetch continue when switching searches, Downloads/Auto Find, or content sources. Background completion never selects a tab or changes another tab’s results/scroll position. Invisible thumbnail warmups are parked to avoid decoding every hidden tab’s images; search/page metadata is not cancelled.
- Page request IDs include a unique session prefix so closing one tab cannot cancel another tab’s request with the same sequence number. Closed/replaced/unmounted contexts reject late responses.

## Verification

- 880 native backend tests passed (25 opt-in tests ignored), including persisted tag-rule filtering, incremental carry-forward and restoring candidates after relaxing rules.
- 220 targeted frontend tests passed. Coverage includes 64 pending search tabs, refusal of a 65th without eviction, background completion while Downloads is selected, focus isolation, closed-tab late responses, background page navigation, single-tab visibility and result-viewport layout.
- TypeScript checking and the production frontend build passed. No production database edits, live network refresh, commit, push or release were performed.

## Investigation only: Downloads “정보 불러오는 중” artist folder

The virtual grouping uses `gallery.artist` (primary artist), not the list of all artists. Downloads initially load persisted summaries in batches of 200 with a yield between batches. Missing primary artist values become the placeholder `정보 불러오는 중`; albums sharing that string consequently occupy one temporary folder.

The SQLite library projection reads primary artist from `galleries.primary_artist`. It joins the durable `gallery_summary_cache`, but currently uses that full summary only for tags/all artists and does not use `summary.artist` to fill a missing primary artist. Loading a detailed summary later updates the frontend projection and moves that card into its real artist folder. Card/hover hydration uses up to six workers; the artist-view preloader prioritizes missing tags. There is no guaranteed one-minute completion deadline, and a present tag cache does not itself guarantee that a missing primary artist is repaired.

Read-only inspection of the actual DB on 2026-09-24 found 6,945 distinct download IDs with at least one non-cancelled/non-quarantined entry. Of these, 1,662 had no basic primary artist while their cached summary contained an artist (1,372 queued, 139 interrupted and 151 completed entries in the state-level check). These are diagnostic counts before all UI exclusion/deduplication filters, not a claim about the currently visible folder count.

This can recur on app startup (and on a UI reload that rebuilds in-memory state), not only after first installation. Within the same session, resolved frontend metadata is normally retained across tab changes. Rebuilding or clearing caches, missing summaries and the entries-only fallback can also produce placeholders. The requested investigation did not modify download grouping, cached data or the real database. A follow-up fix would fill missing primary artist from the already persisted full summary before returning each library page; it would not require scanning every album again.
