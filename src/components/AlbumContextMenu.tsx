import type { ComponentProps } from "react";
import type { Gallery } from "../core/types";
import { CardContextMenu } from "./CardContextMenu";
import { useAlbumMenuItems } from "./albumMenuItems";

// Subscribe to bookmark state only while a menu is open. A bookmark update must
// not rerender every full card (their small bookmark buttons already subscribe).
export function AlbumContextMenu({ gallery, actions, ...menu }: Pick<ComponentProps<typeof CardContextMenu>, "anchor" | "close" | "label"> & {
  gallery: Gallery | undefined; actions: Parameters<typeof useAlbumMenuItems>[1];
}) {
  return <CardContextMenu {...menu} items={useAlbumMenuItems(gallery, actions)} />;
}
