//! Community is independent of the library DB and its reset lifecycle.
//! Public reads carry no user token. Only BeginWriting may issue an identity.
//! Tokens never cross IPC into the webview, and no device identifier is sent.
#[cfg(test)]
mod tests;
mod vault;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{io::Read, path::PathBuf, time::Duration};
use tauri::Manager;

const PROJECT_URL: &str = "https://yfpgshvflnawmrimyfzo.supabase.co";
// Public application identifier, not a service-role/secret key.
const PUBLISHABLE_KEY: &str = "sb_publishable_oRRDkcQgUf1LMzM6pe9blQ_0AGjcS7z";
const MAX_RESPONSE: u64 = 1_048_576;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ReadRequest {
    Feed {
        source: Option<String>,
        cursor: Option<Value>,
    },
    Work {
        source: String,
        #[serde(rename = "workId")]
        work_id: String,
        cursor: Option<Value>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum WriteRequest {
    BeginWriting {
        source: String,
        #[serde(rename = "workId")]
        work_id: String,
    },
    Save {
        source: String,
        #[serde(rename = "workId")]
        work_id: String,
        rating: u8,
        recommended: bool,
        comment: String,
        nickname: String,
    },
    Delete {
        source: String,
        #[serde(rename = "workId")]
        work_id: String,
    },
    Report {
        #[serde(rename = "reviewId")]
        review_id: String,
        reason: String,
    },
}

// Deliberately no Debug implementation: tokens must never enter logs/errors.
#[derive(Clone, Serialize, Deserialize)]
struct Session {
    access_token: String,
    refresh_token: String,
    user_id: String,
    expires_at: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "state")]
enum Identity {
    Unissued,
    Issuing,
    Ready { session: Session },
}

trait IdentityStore {
    fn load(&self) -> Result<Identity, String>;
    fn save(&self, identity: &Identity) -> Result<(), String>;
}

struct RequestFailure {
    message: String,
    definitely_rejected: bool,
}
trait Transport {
    fn post(&self, route: &str, body: Value, token: Option<&str>) -> Result<Value, RequestFailure>;
}

struct SupabaseTransport(reqwest::blocking::Client);
impl SupabaseTransport {
    fn new() -> Result<Self, String> {
        reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map(Self)
            .map_err(|_| "커뮤니티 연결을 준비하지 못했습니다.".into())
    }
}

fn safe_server_error(status: u16, value: &Value) -> String {
    let code = value
        .get("message")
        .or_else(|| value.get("error_code"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let message = match code {
        "COMMUNITY_WRITE_TOO_FAST" => {
            "잠시 후 다시 저장해 주세요. 쓰기 요청은 3초 간격으로 할 수 있습니다."
        }
        "COMMUNITY_DAILY_LIMIT" => "오늘의 커뮤니티 쓰기 한도(100회)에 도달했습니다.",
        "COMMUNITY_ACCOUNT_DISABLED" | "COMMUNITY_PROFILE_REQUIRED" => {
            "작성자 권한을 확인할 수 없습니다. 기존 키는 보존했습니다."
        }
        "COMMUNITY_INVALID_REVIEW" => {
            "별점(1~5), 닉네임(2~24자), 후기(500자 이하)를 확인해 주세요."
        }
        "COMMUNITY_REVIEW_NOT_REPORTABLE" => {
            "삭제·숨김 처리되었거나 본인이 작성한 후기는 신고할 수 없습니다."
        }
        "anonymous_provider_disabled" => "서버의 익명 작성 기능이 아직 활성화되지 않았습니다.",
        "captcha_failed" => {
            "서버에서 봇 방지 확인을 요구합니다. 현재 앱의 작성 기능으로는 확인할 수 없습니다."
        }
        _ if status == 429 => "요청이 많습니다. 잠시 후 다시 시도해 주세요.",
        _ if status == 401 || status == 403 => {
            "작성자 인증을 확인하지 못했습니다. 기존 키를 지우거나 새로 발급하지 않았습니다."
        }
        _ if status == 404 => "커뮤니티 서버 준비가 필요합니다. 잠시 후 다시 시도해 주세요.",
        _ => "커뮤니티 요청을 완료하지 못했습니다. 잠시 후 다시 시도해 주세요.",
    };
    message.to_string()
}

impl Transport for SupabaseTransport {
    fn post(&self, route: &str, body: Value, token: Option<&str>) -> Result<Value, RequestFailure> {
        let uncertain = || RequestFailure {
            message: "커뮤니티 연결이 지연되거나 끊어졌습니다. 기존 키와 작성 내용은 유지됩니다."
                .into(),
            definitely_rejected: false,
        };
        let mut request = self
            .0
            .post(format!("{PROJECT_URL}/{route}"))
            .header("apikey", PUBLISHABLE_KEY)
            .header("Content-Type", "application/json")
            .body(body.to_string());
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().map_err(|_| uncertain())?;
        let status = response.status();
        let mut bytes = Vec::new();
        response
            .take(MAX_RESPONSE + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| uncertain())?;
        if bytes.len() as u64 > MAX_RESPONSE {
            return Err(uncertain());
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| uncertain())?;
        if !status.is_success() {
            return Err(RequestFailure {
                message: safe_server_error(status.as_u16(), &value),
                definitely_rejected: status.is_client_error(),
            });
        }
        Ok(value)
    }
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn parse_session(value: Value, now: u64) -> Result<Session, String> {
    let string = |name: &str| {
        value
            .get(name)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let user_id = value
        .pointer("/user/id")
        .and_then(Value::as_str)
        .filter(|id| uuid::Uuid::parse_str(id).is_ok());
    match (
        string("access_token"),
        string("refresh_token"),
        user_id,
        value.get("expires_in").and_then(Value::as_u64),
    ) {
        (Some(access_token), Some(refresh_token), Some(user_id), Some(expires)) if expires > 0 => {
            Ok(Session {
                access_token,
                refresh_token,
                user_id: user_id.into(),
                expires_at: now.saturating_add(expires),
            })
        }
        _ => Err(
            "서버가 유효한 작성자 키를 반환하지 않았습니다. 중복 발급을 막기 위해 중단했습니다."
                .into(),
        ),
    }
}

fn ensure_session(
    store: &impl IdentityStore,
    transport: &impl Transport,
    now: u64,
    may_issue: bool,
) -> Result<Session, String> {
    match store.load()? {
        Identity::Unissued if may_issue => {
            // Persist the intent first. A crash/ambiguous response must not silently
            // create another account next time. Storage failure precedes signup.
            store.save(&Identity::Issuing)?;
            let response = match transport.post("auth/v1/signup", json!({"data": {}}), None) {
                Ok(value) => value,
                Err(error) => {
                    if error.definitely_rejected { store.save(&Identity::Unissued)?; }
                    return Err(error.message);
                }
            };
            let session = parse_session(response, now)?;
            store.save(&Identity::Ready { session: session.clone() })?;
            Ok(session)
        }
        Identity::Unissued => Err("먼저 ‘후기 작성’을 눌러 작성자 키를 발급해 주세요.".into()),
        Identity::Issuing => Err("이전 키 발급이 중단되어 결과를 확인할 수 없습니다. 중복 발급을 막기 위해 기존 상태를 보존했습니다. 개발자에게 문의해 주세요.".into()),
        Identity::Ready { session } if session.expires_at > now.saturating_add(60) => Ok(session),
        Identity::Ready { session } => {
            let response = transport.post("auth/v1/token?grant_type=refresh_token", json!({"refresh_token": session.refresh_token}), None).map_err(|e| e.message)?;
            let refreshed = parse_session(response, now)?;
            if refreshed.user_id != session.user_id { return Err("작성자 식별 정보가 달라 갱신을 중단했습니다. 기존 키는 보존했습니다.".into()); }
            // Do not send mutations until the rotated refresh token is durable.
            store.save(&Identity::Ready { session: refreshed.clone() })?;
            Ok(refreshed)
        }
    }
}

fn validate_work(source: &str, id: &str) -> Result<(), String> {
    if !matches!(source, "hitomi" | "danbooru")
        || id.is_empty()
        || id.len() > 20
        || id.starts_with('0')
        || !id.bytes().all(|c| c.is_ascii_digit())
    {
        return Err("Hitomi 또는 Danbooru와 올바른 작품번호를 선택해 주세요.".into());
    }
    Ok(())
}

fn read_request(transport: &impl Transport, request: ReadRequest) -> Result<Value, String> {
    match request {
        ReadRequest::Feed { source, cursor } => {
            if let Some(ref source) = source {
                validate_work(source, "1")?;
            }
            transport
                .post(
                    "rest/v1/rpc/community_v1_feed",
                    json!({"p_source": source, "p_cursor": cursor}),
                    None,
                )
                .map_err(|e| e.message)
        }
        ReadRequest::Work {
            source,
            work_id,
            cursor,
        } => {
            validate_work(&source, &work_id)?;
            let mut page = transport
                .post(
                    "rest/v1/rpc/community_v1_reviews",
                    json!({"p_source": source, "p_work_id": work_id, "p_cursor": cursor}),
                    None,
                )
                .map_err(|e| e.message)?;
            let summaries = transport
                .post(
                    "rest/v1/rpc/community_v1_summaries",
                    json!({"p_works": [{"source": source, "workId": work_id}]}),
                    None,
                )
                .map_err(|e| e.message)?;
            page["summary"] = summaries.get(0).cloned().unwrap_or(Value::Null);
            Ok(page)
        }
    }
}

fn write_request(
    store: &impl IdentityStore,
    transport: &impl Transport,
    request: WriteRequest,
) -> Result<Value, String> {
    match &request {
        WriteRequest::BeginWriting { source, work_id }
        | WriteRequest::Save {
            source, work_id, ..
        }
        | WriteRequest::Delete { source, work_id } => validate_work(source, work_id)?,
        WriteRequest::Report { review_id, reason } => {
            if uuid::Uuid::parse_str(review_id).is_err()
                || !(1..=300).contains(&reason.trim().chars().count())
            {
                return Err("신고 사유를 1~300자로 입력해 주세요.".into());
            }
        }
    }
    if let WriteRequest::Save {
        rating,
        comment,
        nickname,
        ..
    } = &request
    {
        if !(1..=5).contains(rating)
            || comment.chars().count() > 500
            || !(2..=24).contains(&nickname.trim().chars().count())
        {
            return Err("별점, 닉네임, 후기 길이를 확인해 주세요.".into());
        }
    }
    let session = ensure_session(
        store,
        transport,
        now_seconds(),
        matches!(request, WriteRequest::BeginWriting { .. }),
    )?;
    let post = |name: &str, body: Value| {
        transport
            .post(
                &format!("rest/v1/rpc/community_v1_{name}"),
                body,
                Some(&session.access_token),
            )
            .map_err(|e| e.message)
    };
    match request {
        WriteRequest::BeginWriting { source, work_id } => {
            let profile = post("profile", json!({}))?;
            let page = post(
                "reviews",
                json!({"p_source": source, "p_work_id": work_id, "p_cursor": null}),
            )?;
            Ok(json!({"profile": profile, "mine": page.get("mine")}))
        }
        WriteRequest::Save {
            source,
            work_id,
            rating,
            recommended,
            comment,
            nickname,
        } => post(
            "save_review",
            json!({"p_source": source, "p_work_id": work_id, "p_rating": rating, "p_recommended": recommended, "p_comment": comment, "p_nickname": nickname}),
        ),
        WriteRequest::Delete { source, work_id } => post(
            "delete_review",
            json!({"p_source": source, "p_work_id": work_id}),
        ),
        WriteRequest::Report { review_id, reason } => post(
            "report",
            json!({"p_review_id": review_id, "p_reason": reason}),
        ),
    }
}

fn trusted_main(label: &str, url: &tauri::Url, development: bool) -> bool {
    label == "main"
        && url.username().is_empty()
        && url.password().is_none()
        && (matches!(
            (url.scheme(), url.host_str(), url.port()),
            ("tauri", Some("localhost"), None) | ("http" | "https", Some("tauri.localhost"), None)
        ) || development
            && url.scheme() == "http"
            && url.host_str() == Some("127.0.0.1")
            && url.port() == Some(1420))
}

fn require_main(window: &tauri::WebviewWindow) -> Result<(), String> {
    let url = window.url().map_err(|_| "앱 창을 확인하지 못했습니다.")?;
    if !trusted_main(window.label(), &url, cfg!(debug_assertions)) {
        return Err("커뮤니티는 Atsumi 기본 창에서만 사용할 수 있습니다.".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn community_read(
    window: tauri::WebviewWindow,
    request: ReadRequest,
) -> Result<Value, String> {
    require_main(&window)?;
    tauri::async_runtime::spawn_blocking(move || read_request(&SupabaseTransport::new()?, request))
        .await
        .map_err(|_| "후기 조회 작업을 완료하지 못했습니다.")?
}

#[tauri::command]
pub async fn community_write(
    window: tauri::WebviewWindow,
    request: WriteRequest,
) -> Result<Value, String> {
    require_main(&window)?;
    // LocalAppData, not the roaming library DB/reset directory. No configurable
    // path or token is accepted from the client. A per-file lock also serializes
    // development/release processes during refresh-token rotation.
    let path: PathBuf = window
        .app_handle()
        .path()
        .app_local_data_dir()
        .map_err(|_| "작성자 키 저장 위치를 확인하지 못했습니다.")?
        .join("community-identity")
        .join("yfpgshvflnawmrimyfzo.v1.dpapi");
    tauri::async_runtime::spawn_blocking(move || {
        let store = vault::FileVault::open(path)?;
        write_request(&store, &SupabaseTransport::new()?, request)
    })
    .await
    .map_err(|_| "후기 작성 작업을 완료하지 못했습니다.")?
}
