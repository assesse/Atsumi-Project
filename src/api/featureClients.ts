import type { BackendClient } from "./backend";

/** The transport operations available to the Danbooru workspace. */
export type DanbooruApi = Pick<BackendClient,
  | "runtime"
  | "danbooruSearch"
  | "danbooruRandom"
  | "danbooruRelated"
  | "danbooruAutocomplete"
  | "danbooruDownload"
  | "danbooruDownloadsList"
>;

/** Keep transport state on its owner and expose only this feature's operations. */
export function createDanbooruApi(client: DanbooruApi): DanbooruApi {
  return {
    runtime: client.runtime,
    danbooruSearch: (request) => client.danbooruSearch(request),
    danbooruRandom: () => client.danbooruRandom(),
    danbooruRelated: (request) => client.danbooruRelated(request),
    danbooruAutocomplete: (query, limit) => client.danbooruAutocomplete(query, limit),
    danbooruDownload: (postId) => client.danbooruDownload(postId),
    danbooruDownloadsList: (request) => client.danbooruDownloadsList(request),
  };
}
