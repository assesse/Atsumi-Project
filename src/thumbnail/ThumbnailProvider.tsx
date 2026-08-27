import { createContext, useContext, type ReactNode } from "react";
import { ThumbnailClient } from "./client";

const ThumbnailClientContext = createContext<ThumbnailClient | null>(null);

export function ThumbnailProvider({ client, children }: { client: ThumbnailClient; children: ReactNode }) {
  return <ThumbnailClientContext.Provider value={client}>{children}</ThumbnailClientContext.Provider>;
}

export function useThumbnailClient(override?: ThumbnailClient): ThumbnailClient {
  const contextClient = useContext(ThumbnailClientContext);
  const client = override ?? contextClient;
  if (!client) throw new Error("ThumbnailProvider is required");
  return client;
}
