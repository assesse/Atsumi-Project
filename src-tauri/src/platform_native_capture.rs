//! User-triggered frame capture from an already playing official document.
//! Never fetches media, reads page text/cookies or captures the desktop/chat.
use base64::{engine::general_purpose::STANDARD, Engine};
use tauri::Webview;
const MAX_PNG:usize=16*1024*1024;
const ERROR:&str="현재 영상의 스크린샷을 만들지 못했습니다.";
pub enum VideoSurface { Soop, YouTube }
impl VideoSurface { fn selector(&self)->&'static str { match self { Self::Soop=>"video#livePlayer",Self::YouTube=>"#movie_player video" } } }
fn script(surface:VideoSurface)->String {
    // Selector comes exclusively from this closed enum, not remote/user data.
    format!(r#"(()=>{{try{{const v=document.querySelector('{}');if(!v||v.readyState<2||v.videoWidth<1||v.videoHeight<1||v.videoWidth*v.videoHeight>16777216||v.mediaKeys)return {{code:'not_ready'}};const c=document.createElement('canvas');c.width=v.videoWidth;c.height=v.videoHeight;const x=c.getContext('2d');if(!x)return {{code:'not_ready'}};x.drawImage(v,0,0);const data=c.toDataURL('image/png');if(data.length>22369700)return {{code:'too_large'}};return {{data}};}}catch{{return {{code:'capture_restricted'}};}}}})()"#,surface.selector())
}
fn decode(raw:&str)->Result<Vec<u8>,String>{
    if raw.len()>22_370_000{return Err(ERROR.into());}
    let result:serde_json::Value=serde_json::from_str(raw).map_err(|_|ERROR)?;
    let data=result["data"].as_str().and_then(|d|d.strip_prefix("data:image/png;base64,")).ok_or_else(||{
        if result["code"]=="capture_restricted" {"플랫폼이 현재 영상의 이미지 저장을 허용하지 않습니다."}else{ERROR}
    })?;
    let bytes=STANDARD.decode(data).map_err(|_|ERROR)?;
    if bytes.is_empty()||bytes.len()>MAX_PNG||!bytes.starts_with(b"\x89PNG\r\n\x1a\n"){return Err(ERROR.into());}
    // The shared screenshot store still fully decodes and validates dimensions.
    Ok(bytes)
}
/// Call on a blocking worker, only after trusted-main intent/visible-pane checks.
/// The caller must also recheck its native session epoch after this returns.
pub fn video_png(view:&Webview,expected_url:&str,surface:VideoSurface)->Result<Vec<u8>,String>{
    if view.url().map_err(|_|ERROR)?.as_str()!=expected_url{return Err(ERROR.into());}
    let (tx,rx)=std::sync::mpsc::sync_channel(1);
    view.eval_with_callback(script(surface),move|value|{let _=tx.send(value);}).map_err(|_|ERROR)?;
    let result=rx.recv_timeout(std::time::Duration::from_secs(5)).map_err(|_|ERROR)?;
    if view.url().map_err(|_|ERROR)?.as_str()!=expected_url{return Err(ERROR.into());}
    decode(&result)
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn never_accepts_other_payloads_or_unbounded_images(){
        for value in [r#"{"data":"https://example.test/picture.png"}"#,r#"{"data":"data:image/jpeg;base64,aGVsbG8="}"#,r#"{"data":"data:image/png;base64,aGVsbG8="}"#,r#"{"code":"capture_restricted"}"#]{assert!(decode(value).is_err());}
        assert!(decode(&" ".repeat(22_370_001)).is_err());
    }
    #[test]fn capture_is_video_only_and_has_no_network_or_account_access(){for surface in [VideoSurface::Soop,VideoSurface::YouTube]{let s=script(surface);assert!(s.contains("drawImage(v,0,0)"));for forbidden in ["fetch(","XMLHttpRequest","cookie","localStorage","innerText","getDisplayMedia"]{assert!(!s.contains(forbidden));}}}
}
