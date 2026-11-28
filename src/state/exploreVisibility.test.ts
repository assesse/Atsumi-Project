import { afterEach, expect, it } from "vitest";
import { mockGalleries } from "../data/mockGalleries";
import type { Gallery } from "../core/types";
import { omittedExploreIds, readExploreVisibility, saveExploreVisibility } from "./exploreVisibility";

afterEach(() => localStorage.clear());

it("uses the displayed status when a downloaded album is also excluded or awaiting duplicate review", () => {
  const rows: Gallery[] = mockGalleries.slice(0, 5).map(g => ({...g, download: { entryId: String(g.id), state: "completed", revision: 1 }}));
  const [excluded, candidate, complete, quarantined, fresh] = rows as [Gallery, Gallery, Gallery, Gallery, Gallery];
  quarantined.download = {...quarantined.download!, state: "quarantined"};
  fresh.download = undefined;
  const albums = new Map(rows.map(g => [g.id, g]));
  const excludedIds = new Set([excluded.id]);
  const duplicates = new Map([excluded, candidate, quarantined].map(g => [g.id, 1]));
  const options = readExploreVisibility();
  expect(omittedExploreIds(options, albums, excludedIds, duplicates)).toEqual([]);
  expect(omittedExploreIds({...options, completed:false}, albums, excludedIds, duplicates)).toEqual([complete.id]);
  expect(omittedExploreIds({...options, duplicate:false}, albums, excludedIds, duplicates)).toEqual([candidate.id]);
  const choice = {...options, excluded:false};
  saveExploreVisibility(choice);
  expect(readExploreVisibility()).toEqual(choice);
  expect(omittedExploreIds(readExploreVisibility(), albums, excludedIds, duplicates)).toEqual([excluded.id, quarantined.id].sort((a,b)=>a-b));
});
