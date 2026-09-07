import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { BackendClient } from "../api/backend";
import type { ArtistPreview, Gallery, GalleryId, GalleryPreview } from "../core/types";

/** Saved selections only: opening or hovering a folder never starts image analysis. */
export function useSavedGalleryPreviews(backend: BackendClient, galleries: ReadonlyMap<GalleryId, Gallery>) {
  const [previews, setPreviews] = useState<ReadonlyMap<GalleryId, GalleryPreview>>(new Map());
  const [artists, setArtists] = useState<ReadonlyMap<string, ArtistPreview>>(new Map());
  const requestedGalleries = useRef(new Set<GalleryId>());
  const requestedArtists = useRef(new Set<string>());
  const mounted = useRef(false);
  const subscriptionsReady = useRef(Promise.resolve());
  const completed = [...galleries.values()].filter((gallery) => gallery.download?.state === "completed");
  const gallerySignature = completed.map((gallery) => gallery.id).sort((a, b) => a - b).join(",");
  const artistSignature = JSON.stringify([...new Set(completed.map((gallery) => gallery.artist.trim().toLocaleLowerCase()).filter(Boolean))].sort());

  useEffect(() => {
    mounted.current = true;
    let disposed = false;
    const cleanup: Array<() => void> = [];
    const listen = async () => {
      for (const registration of [
        () => backend.on("gallery-preview:updated", (preview) => {
          if (!disposed) setPreviews((current) => new Map(current).set(preview.galleryId, preview));
        }),
        () => backend.on("artist-preview:updated", (preview) => {
          if (!disposed) setArtists((current) => new Map(current).set(preview.artist.trim().toLocaleLowerCase(), preview));
        }),
      ]) {
        try {
          const unsubscribe = await registration();
          if (disposed) unsubscribe(); else cleanup.push(unsubscribe);
        } catch { /* An unavailable preview service must not block the library. */ }
      }
    };
    subscriptionsReady.current = listen();
    return () => { mounted.current = false; disposed = true; cleanup.forEach((unsubscribe) => unsubscribe()); };
  }, [backend]);

  useEffect(() => {
    const ids = gallerySignature ? gallerySignature.split(",").map((id) => Number(id) as GalleryId) : [];
    const names = JSON.parse(artistSignature) as string[];
    const load = async () => {
      await subscriptionsReady.current;
      if (!mounted.current) return;
      const missingIds = ids.filter((id) => !requestedGalleries.current.has(id));
      const missingArtists = names.filter((artist) => !requestedArtists.current.has(artist));
      // Claim before awaiting so progressive library/artist hydration cannot
      // issue duplicate reads while a previous batch is still in flight.
      missingIds.forEach((id) => requestedGalleries.current.add(id));
      missingArtists.forEach((artist) => requestedArtists.current.add(artist));
      for (let offset = 0; offset < missingIds.length && mounted.current; offset += 200) {
        const batch = missingIds.slice(offset, offset + 200);
        try {
          const result = await backend.galleryPreviewList(batch);
          if (!mounted.current) return;
          if (result.ok) {
            setPreviews((current) => {
              const next = new Map(current);
              // A newer completion/manual-selection event may have beaten this snapshot.
              result.data.forEach((preview) => { if (!next.has(preview.galleryId)) next.set(preview.galleryId, preview); });
              return next;
            });
          } else batch.forEach((id) => requestedGalleries.current.delete(id));
        } catch { batch.forEach((id) => requestedGalleries.current.delete(id)); }
      }
      for (let offset = 0; offset < missingArtists.length && mounted.current; offset += 200) {
        const batch = missingArtists.slice(offset, offset + 200);
        try {
          const result = await backend.artistPreviewList(batch);
          if (!mounted.current) return;
          if (result.ok) {
            setArtists((current) => {
              const next = new Map(current);
              result.data.forEach((preview) => {
                const key = preview.artist.trim().toLocaleLowerCase();
                if (!next.has(key)) next.set(key, preview);
              });
              return next;
            });
          } else batch.forEach((artist) => requestedArtists.current.delete(artist));
        } catch { batch.forEach((artist) => requestedArtists.current.delete(artist)); }
      }
    };
    void load();
  }, [backend, gallerySignature, artistSignature]);

  const save = useCallback(async (galleryId: GalleryId, sourcePage: number | null) => {
    const result = await backend.galleryPreviewSet({ galleryId, sourcePage });
    if (!result.ok) throw new Error(result.error.message);
    setPreviews((current) => new Map(current).set(galleryId, result.data));
  }, [backend]);
  const artistGalleryIds = useMemo(() => new Map([...artists].map(([key, value]) => [key, value.galleryIds])), [artists]);
  return { previews, artistGalleryIds, save };
}
