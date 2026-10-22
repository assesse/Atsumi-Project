(() => {
  const nonce = __ATSUMI_SOOP_DISCOVERY_NONCE__;
  const result = (state, extra = {}) => ({ nonce, state, ...extra });
  if (window !== window.top || location.href !== "https://live.sooplive.com/api/myplus/preferbjLiveVodController.php") return result("unavailable");
  if (document.readyState !== "complete") return result("pending");
  const text = document.body?.textContent;
  if (!text || text.length > 2 * 1024 * 1024) return result("unavailable");
  try {
    const value = JSON.parse(text);
    const get = (object, snake, camel = snake) => object?.[snake] ?? object?.[snake.toUpperCase()] ?? object?.[camel];
    if (String(get(value, "result")) !== "1") return result("unavailable");
    const data = get(value, "data");
    if (!data || typeof data !== "object" || Array.isArray(data)) return result("unavailable");
    const rows = (list, kind) => Array.isArray(list) ? list.slice(0, 40).map(row => {
      const clean = (snake, camel = snake, limit = 300) => {
        const item = get(row, snake, camel);
        return typeof item === "string" ? item.slice(0, limit) : typeof item === "number" && Number.isFinite(item) ? item : null;
      };
      const tags = key => {
        const list = get(row, key);
        return Array.isArray(list) ? list.slice(0, 8).filter(tag => typeof tag === "string" && !/[\u0000-\u001f\u007f]/.test(tag)).map(tag => tag.slice(0, 32)) : [];
      };
      return { kind, user_id: clean("user_id", "userId", 40), user_nick: clean("user_nick", "userNick", 120),
        bj_id: clean("bj_id", "bjId", 40), bj_nick: clean("bj_nick", "bjNick", 120),
        broad_no: clean("broad_no", "broadNo", 20), title_no: clean("title_no", "titleNo", 20),
        broad_title: clean("broad_title", "broadTitle"), title_name: clean("title_name", "titleName"), title: clean("title"),
        broad_thumb: clean("broad_thumb", "broadThumb", 2048), broad_img: clean("broad_img", "broadImg", 2048), thumb: clean("thumb", "thumb", 2048),
        thumbnail: clean("thumbnail", "thumbnail", 2048),
        lang_tags: tags("lang_tags"), category_tags: tags("category_tags"), hash_tags: tags("hash_tags"),
        total_view_cnt: clean("total_view_cnt", "totalViewCnt", 20), view_cnt: clean("view_cnt", "viewCnt", 20), duration: clean("duration", "duration", 30),
        vod_duration: clean("vod_duration", "vodDuration", 30), ppv: get(row, "ppv") === true || get(row, "ppv") === "Y" || get(row, "ppv") === 1,
        is_password: clean("is_password", "isPassword", 5), subscription_only: clean("subscription_only", "subscriptionOnly", 5), ppv_yn: clean("ppv_yn", "ppvYn", 5), grade: clean("grade", "grade", 5) };
    }) : [];
    return result("ready", { live: rows(get(data, "live_list", "liveList"), "live"), video: rows(get(data, "vod_list", "vodList"), "video") });
  } catch { return result("unavailable"); }
})()
