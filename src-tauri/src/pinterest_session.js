// Bounded content projection from one officially loaded document. No cookies,
// storage, credentials, arbitrary fetch or remote IPC. Only the owner query
// returns the signed-in profile path required for the requested own boards.
(() => {
  const request = __ATSUMI_PINTEREST_REQUEST__;
  // Closed shapes only: never retain option names/values, identifiers or bookmarks here.
  const diagnostics = {
    sourceBranch: "none", initialShape: "absent", knownResourceMask: 0, resourceEntryCount: 0,
    structuredPinCount: 0, domPinCount: 0, dataShape: "missing", entryShape: "empty",
    keyShape: "other", bookmarkShape: "absent", bookmarkLocationMask: 0, planReject: "missing_bookmark",
  };
  const diagnosticKind = ["home", "keyword", "profile"].includes(request.kind);
  const result = (state, extra = {}) => {
    const projected = { nonce: request.nonce, state, ...extra };
    // Do not push an otherwise accepted document over the unchanged native cap.
    if (diagnosticKind) try {
      if (new TextEncoder().encode(JSON.stringify(projected)).length <= 512 * 1024 - 2048) projected.diagnostics = diagnostics;
    } catch { /* diagnostics are optional; product projection is unchanged */ }
    return projected;
  };
  if (window !== window.top || location.protocol !== "https:" || !["pinterest.com", "www.pinterest.com", "kr.pinterest.com"].includes(location.hostname)) return result("unavailable");
  if (document.readyState !== "complete") return result("pending");
  const bounded = (value, limit) => typeof value === "string" ? value.slice(0, limit) : "";
  const id = value => typeof value === "string" && /^[1-9]\d{4,24}$/.test(value) ? value : null;
  const media = value => {
    if (!value || !["object", "string"].includes(typeof value)) return null;
    const raw = typeof value === "string" ? value : value.url ?? value.contentUrl;
    try {
      const url = new URL(raw);
      if (url.protocol !== "https:" || url.username || url.password || url.port || !["i.pinimg.com", "i2.pinimg.com"].includes(url.hostname)) return null;
      const dimension = n => Number.isInteger(n) && n > 0 && n <= 16384 ? n : null;
      return { url: url.href, width: dimension(value.width), height: dimension(value.height) };
    } catch { return null; }
  };
  const pin = (value, summary = true) => {
    const pinId = id(value?.id);
    if (!pinId) return null;
    const images = value.images;
    const original = images?.orig ?? images?.originals;
    const image = media(original ?? (summary ? images?.["736x"] ?? images?.["564x"] ?? images?.["236x"] : null));
    if (!image) return null;
    const author = value.pinner ?? value.native_creator ?? value.user ?? {};
    const username = bounded(author.username, 100);
    return { id: pinId, title: bounded(value.title || value.grid_title || `핀 #${pinId}`, 300), description: bounded(value.description, 3000),
      creatorName: bounded(author.full_name || author.username, 120), creatorPath: /^[\w-]{1,100}$/.test(username) ? `/${username}/` : null,
      summaryOnly: summary, sourceUrl: `https://www.pinterest.com/pin/${pinId}/`, media: [image],
      unsupportedVideo: !!value.videos || !!value.story_pin_data };
  };
  let initial = null;
  const initialText = document.getElementById("__PWS_INITIAL_PROPS__")?.textContent;
  if (initialText && initialText.length <= 5 * 1024 * 1024) {
    try {
      initial = JSON.parse(initialText)?.initialReduxState;
      diagnostics.initialShape = initial && typeof initial === "object" && !Array.isArray(initial) ? "redux_object" : "redux_missing";
    } catch { diagnostics.initialShape = "invalid_json"; }
  } else if (initialText) diagnostics.initialShape = "over_limit";
  const authenticated = typeof initial?.session?.isAuthenticated === "boolean" ? initial.session.isAuthenticated : null;
  if (request.kind === "status") return result("ready", { authenticated });
  if (request.kind === "owner") {
    // Recommended UserResource/users/pinners are NOT evidence of ownership.
    const username = initial?.session?.user?.username;
    const ownerPath = authenticated === true && typeof username === "string" && /^[A-Za-z0-9_-]{1,100}$/.test(username)
      ? `/${username}/_saved/` : null;
    return result("ready", { authenticated, ownerPath });
  }
  const pins = [], boards = [], seen = new Set();
  let exhausted = false;
  let observedTerminal = 0, observedNonterminal = 0;
  const boardPath = value => {
    try {
      const url = new URL(value, location.origin);
      return ["pinterest.com", "www.pinterest.com", "kr.pinterest.com"].includes(url.hostname) && url.protocol === "https:" && !url.username && !url.password && !url.port && url.pathname.length <= 1804 && /^\/[^/?#]+\/[^/?#]+\/$/.test(url.pathname) ? url.pathname : null;
    } catch { return null; }
  };
  const addBoard = raw => {
    const value = typeof raw === "string" ? initial?.boards?.[raw] : raw;
    const path = boardPath(value?.url);
    if (path && boards.length < 40 && !boards.some(board => board.path === path)) boards.push({ title: bounded(value.name, 200), path });
  };
  if (request.kind !== "saved") for (const resource of Object.values(initial?.resources?.BoardsFeedResource ?? {}).slice(0, 1)) {
    if (Array.isArray(resource?.data)) for (const value of resource.data.slice(0, 40)) addBoard(value);
  }
  const add = value => { if (value && !seen.has(value.id) && pins.length < 60) { seen.add(value.id); pins.push(value); } };
  const resourceNames = request.kind === "home" ? ["HomefeedResource", "UserHomefeedResource"] : request.kind === "keyword" ? ["BaseSearchResource"] : request.kind === "saved" ? ["BoardsFeedResource"] : ["UserPinsResource", "BoardFeedResource"];
  const knownResources = ["HomefeedResource", "UserHomefeedResource", "BaseSearchResource", "UserPinsResource", "BoardFeedResource"];
  for (let index = 0; index < knownResources.length; index++) {
    if (initial?.resources?.[knownResources[index]] != null) diagnostics.knownResourceMask |= 1 << index;
  }
  const bookmarkShape = value => value == null ? "absent" : Array.isArray(value) ? "array" : typeof value !== "string" ? "other" : value === "" ? "empty" : value === "-end-" ? "end" : "string";
  const dataShapes = raw => {
    const data = Array.isArray(raw) ? raw : raw?.results;
    diagnostics.dataShape = Array.isArray(raw) ? "array" : Array.isArray(raw?.results) ? "results_array" : raw == null ? "missing" : "other";
    const entries = Array.isArray(data) ? data.slice(0, 60) : [];
    const shapes = new Set(entries.map(value => value && typeof value === "object" && id(value.id) ? "pin_objects" : ["string", "number"].includes(typeof value) ? "id_only" : "other"));
    diagnostics.entryShape = !shapes.size ? "empty" : shapes.size > 1 ? "mixed" : [...shapes][0];
  };
  let continuation = null;
  const plan = (resource, key, bookmark) => {
    diagnostics.bookmarkShape = bookmarkShape(bookmark);
    diagnostics.keyShape = "other";
    diagnostics.planReject = "none";
    const reject = reason => { diagnostics.planReject = reason; return null; };
    // Keep the original admission order and limits. Diagnostics do not repair a plan.
    if (typeof bookmark !== "string" || !bookmark) return reject("missing_bookmark");
    if (bookmark === "-end-") return reject("end");
    if (bookmark.length > 8192) return reject("option_size");
    if (!resourceNames.includes(resource)) return reject("key_shape");
    try {
      const pairs = JSON.parse(key);
      diagnostics.keyShape = Array.isArray(pairs) ? "pairs" : pairs && typeof pairs === "object" ? "object" : "other";
      if (!Array.isArray(pairs)) return reject("key_shape");
      if (pairs.length > 32) return reject("too_many_options");
      const allowed = new Set(["add_vase", "field_set_key", "gated", "is_own_profile_pins", "page_size", "username", "board_id", "query", "scope", "rs", "source_pos", "redux_normalize_feed", "sort", "filter_stories", "in_nux", "prepend", "is_prefetch", "num_pins", "bookmarks"]);
      const options = {}, keys = new Set();
      const canProjectUnknown = request.kind === "home" && resource === "UserHomefeedResource"
        || request.kind === "keyword" && resource === "BaseSearchResource";
      let omitted = false;
      for (const pair of pairs) {
        if (!Array.isArray(pair) || pair.length !== 2 || typeof pair[0] !== "string" || !pair[0] || keys.has(pair[0])) return reject("key_shape");
        keys.add(pair[0]);
        if (!allowed.has(pair[0])) {
          if (!canProjectUnknown || pair[0] === "access_token") return reject("unknown_option");
          omitted = true;
          continue;
        }
        const value = pair[1];
        if (pair[0] === "bookmarks") continue;
        if (!(typeof value === "boolean" || typeof value === "number" && Number.isFinite(value) || typeof value === "string" && value.length <= 300)) return reject(typeof value === "string" ? "option_size" : "option_type");
        options[pair[0]] = value;
      }
      if (omitted && !Object.keys(options).length) return reject("key_shape");
      if (request.kind === "keyword" && (resource !== "BaseSearchResource" || typeof request.input !== "string"
        || options.query !== request.input.trim() || options.scope !== "pins")) return reject("context_mismatch");
      if (request.kind === "saved" && (resource !== "BoardsFeedResource" || options.username !== request.owner)) return reject("context_mismatch");
      return { resource, options, bookmark };
    } catch { diagnostics.keyShape = "invalid_json"; return reject("key_shape"); }
  };
  if (request.kind === "pin") {
    // Prefer the canonical page's explicitly identified image schema. Do not
    // treat an arbitrary recommendation or thumbnail as this pin's original.
    const nodes = [];
    const collect = (value, depth = 0) => {
      if (!value || depth > 3 || nodes.length >= 60) return;
      if (Array.isArray(value)) { for (const item of value.slice(0, 60)) collect(item, depth + 1); }
      else if (typeof value === "object") { nodes.push(value); collect(value["@graph"], depth + 1); }
    };
    for (const script of [...document.querySelectorAll('script[type="application/ld+json"]')].slice(0, 8)) {
      if (script.textContent?.length > 512 * 1024) continue;
      try { collect(JSON.parse(script.textContent)); } catch { /* malformed optional schema */ }
    }
    for (const value of nodes) {
      if (!["SocialMediaPosting", "CreativeWork", "ImageObject"].includes(value["@type"])) continue;
      const source = value.mainEntityOfPage?.["@id"] ?? value.mainEntityOfPage ?? value.url;
      try {
        const url = new URL(source);
        if (!["pinterest.com", "www.pinterest.com", "kr.pinterest.com"].includes(url.hostname) || url.protocol !== "https:" || url.username || url.password || url.port || !new RegExp(`^/pin/(?:[^/]*--)?${request.id}/?$`).test(url.pathname)) continue;
      } catch { continue; }
      const raw = value.image ?? (value["@type"] === "ImageObject" ? value : null);
      const values = Array.isArray(raw) ? raw : [raw];
      if (!values.length || values.length > 50) continue;
      const images = values.map(media);
      if (images.some(image => !image || !new URL(image.url).pathname.startsWith("/originals/"))) continue;
      add({ id: request.id, title: bounded(value.headline || value.name || `핀 #${request.id}`, 300), description: bounded(value.articleBody || value.description, 3000),
        creatorName: bounded(value.author?.name, 120), creatorPath: null, summaryOnly: false, sourceUrl: `https://www.pinterest.com/pin/${request.id}/`, media: images,
        unsupportedVideo: !!value.video || value.sharedContent?.["@type"] === "VideoObject" });
    }
    for (const resource of Object.values(initial?.resources?.PinResource ?? {}).slice(0, 4)) {
      if (resource?.data?.id === request.id) add(pin(resource.data, false));
    }
    const exact = initial?.pins?.[request.id];
    if (exact?.id === request.id) add(pin(exact, false));
  } else if (request.resource) {
    diagnostics.sourceBranch = "resource_response";
    const resourceIndex = knownResources.indexOf(request.resource);
    diagnostics.knownResourceMask = resourceIndex < 0 ? 0 : 1 << resourceIndex;
    diagnostics.resourceEntryCount = 1;
    const text = document.body?.textContent;
    if (!text || text.length > 5 * 1024 * 1024) return result("unavailable");
    try {
      const payload = JSON.parse(text), response = payload.resource_response;
      if (!response || response.status === "failure" || response.error) return result("unavailable");
      dataShapes(response.data);
      diagnostics.bookmarkLocationMask = (response.bookmark != null ? 8 : 0) | (response.data?.bookmark != null ? 2 : 0) | (payload.resource?.options?.bookmarks != null ? 4 : 0);
      const data = Array.isArray(response.data) ? response.data : response.data?.results;
      if (!Array.isArray(data)) return result("unavailable");
      for (const value of data.slice(0, 60)) { if (request.kind === "saved") addBoard(value); else add(pin(value)); }
      diagnostics.structuredPinCount = pins.length;
      // The resource.options bookmark may merely echo the request. It is not
      // evidence of a new page or a terminal response.
      const bookmark = response.bookmark ?? response.data?.bookmark;
      continuation = plan(request.resource, JSON.stringify(Object.entries(request.options ?? {})), bookmark);
      exhausted = bookmark === "-end-";
      if (typeof bookmark === "string" && bookmark !== "-end-" && bookmark === request.bookmark) {
        continuation = null;
        diagnostics.planReject = "same_bookmark";
      }
    } catch { return result("unavailable"); }
  } else {
    for (const name of resourceNames) for (const [key, resource] of Object.entries(initial?.resources?.[name] ?? {}).slice(0, 2)) {
      diagnostics.resourceEntryCount = Math.min(4, diagnostics.resourceEntryCount + 1);
      if (!continuation) {
        dataShapes(resource?.data);
        diagnostics.bookmarkLocationMask = (resource?.nextBookmark != null ? 1 : 0) | (resource?.data?.bookmark != null ? 2 : 0) | (resource?.options?.bookmarks != null ? 4 : 0);
      }
      const data = Array.isArray(resource?.data) ? resource.data : resource?.data?.results;
      if (!Array.isArray(data)) { observedNonterminal++; continue; }
      if (resource.nextBookmark === "-end-") observedTerminal++;
      else observedNonterminal++;
      for (const value of data.slice(0, 60)) { if (request.kind === "saved") addBoard(value); else add(pin(typeof value === "string" ? initial?.pins?.[value] : value)); }
      continuation ??= plan(name, key, resource.nextBookmark);
    }
    exhausted = !continuation && observedTerminal > 0 && observedNonterminal === 0;
    diagnostics.structuredPinCount = pins.length;
    diagnostics.sourceBranch = pins.length ? "hydration" : "dom_fallback";
    // Some official home documents hydrate only on the client. Observe rendered
    // pin cards in that same document; never call their recommendations originals.
    if (!pins.length && request.kind !== "saved") for (const anchor of [...document.querySelectorAll('a[href*="/pin/"]')].slice(0, 120)) {
      let target;
      try { target = new URL(anchor.href, location.href); } catch { continue; }
      if (!["pinterest.com", "www.pinterest.com", "kr.pinterest.com"].includes(target.hostname) || target.protocol !== "https:" || target.username || target.password || target.port) continue;
      const match = /^\/pin\/(?:[^/]*--)?([1-9]\d{4,24})\/?$/.exec(target.pathname);
      const img = anchor.querySelector("img");
      const image = media({ url: img?.currentSrc || img?.src, width: img?.naturalWidth, height: img?.naturalHeight });
      if (match && image) add({ id: match[1], title: bounded(img?.alt || `핀 #${match[1]}`, 300), description: "", creatorName: "", creatorPath: null,
        summaryOnly: true, sourceUrl: `https://www.pinterest.com/pin/${match[1]}/`, media: [image], unsupportedVideo: !!anchor.querySelector("video") });
    }
    diagnostics.domPinCount = pins.length - diagnostics.structuredPinCount;
  }
  // A valid but nonterminal empty follow-up must preserve earlier cards as
  // limited, not falsely report end. Initial empty documents still wait/error.
  if (!pins.length && !boards.length && !exhausted && !request.resource) return result(request.kind === "pin" ? "unavailable" : "pending", { authenticated });
  const profile = Object.values(initial?.resources?.UserResource ?? {})[0]?.data;
  const board = Object.values(initial?.resources?.BoardResource ?? {})[0]?.data;
  return result("ready", { authenticated, pins, boards, title: bounded(board?.name || profile?.full_name, 200), continuation, exhausted, limited: !continuation && !exhausted });
})()
