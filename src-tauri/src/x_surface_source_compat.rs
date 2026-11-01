//! A source-only compatibility correction for X's already-permitted media.
//! X renders `showContent || displaySensitiveMedia`, but its blurred-media
//! wrapper grants playback priority only for `showContent`. With the account's
//! existing permission there is no Show button to set that state. The original
//! coordinator consequently skips autoplay and stops manual playback on scroll.
//!
//! Correct that predicate before the public script executes. No player calls,
//! React/account state access, private API, account setting or security change.

use tauri::Url;

const MAX_SCRIPT: usize = 4 * 1024 * 1024;
const ASSET_PATTERNS: [&str; 1] =
    ["https://abs.twimg.com/responsive-web/client-web/shared~bundle.AudioSpaceAnalytics~*.js"];

fn identifier(value: &str) -> Option<&str> {
    let first = value.as_bytes().first()?;
    if !first.is_ascii_alphabetic() && !b"_$".contains(first) {
        return None;
    }
    let end = value
        .bytes()
        .position(|c| !c.is_ascii_alphanumeric() && !b"_$".contains(&c))
        .unwrap_or(value.len());
    Some(&value[..end])
}

fn field<'a>(value: &'a str, key: &str) -> Option<&'a str> {
    if value.matches(key).count() != 1 {
        return None;
    }
    identifier(value.split_once(key)?.1)
}

fn repair_permitted_media(source: &str) -> Option<String> {
    const START: &str = "_renderBlurredMediaTombstone=()=>{";
    const END: &str = "_renderAgeVerificationModal=";
    if source.len() > MAX_SCRIPT || source.matches(START).count() != 1 {
        return None;
    }
    let start = source.find(START)? + START.len();
    let length = source[start..].find(END)?;
    if length > 8192 {
        return None;
    }
    let method = &source[start..start + length];
    let prelude = method.split_once(";return")?.0;
    let permitted = field(prelude, "displaySensitiveMedia:")?;
    let shown = field(prelude, "showContent:")?;
    let visibility = field(prelude, "mediaVisibilityResults:")?;
    if !prelude.starts_with("let{children:")
        || !prelude.contains(&format!("{{showContent:{shown}}}=this.state"))
        || !prelude.contains(&format!("={shown}||{permitted},"))
    {
        return None;
    }
    // Require the actual upstream age gate, not merely a similarly named
    // boolean. Unknown/changed source leaves the original script untouched.
    let gate = format!("={visibility}?.blurred_image_interstitial?.interstitial_action&&this.context.featureSwitches.isTrue(\"rweb_age_assurance_flow_enabled\")");
    if prelude.matches(&gate).count() != 1 {
        return None;
    }
    let before_gate = &prelude[..prelude.find(&gate)?];
    let age = before_gate.rsplit(',').next()?;
    if identifier(age) != Some(age)
        || [shown, permitted, visibility].contains(&age)
        || !prelude.contains(&format!("!{permitted}||!!{age}"))
    {
        return None;
    }
    let priority = format!("playbackPriority:{shown}?");
    if method.matches(&priority).count() != 1 {
        return None;
    }
    let offset = method.find(&priority)?;
    let enum_name = identifier(&method[offset + priority.len()..])?;
    let original = format!("{priority}{enum_name}.c.NORMAL:{enum_name}.c.INELIGIBLE");
    if !method[offset..].starts_with(&original) {
        return None;
    }
    let fixed = format!("playbackPriority:({shown}||{permitted}&&!{age})?{enum_name}.c.NORMAL:{enum_name}.c.INELIGIBLE");
    let mut result = source.to_owned();
    result.replace_range(start + offset..start + offset + original.len(), &fixed);
    Some(result)
}

// Public source fixture, not proof that its optional route is registered.
const VIDEO_ENTRY: &str = "if(!e||!t||!r)return e;let i=e.indexOf(\"?\"),n=-1===i?\"\":e.slice(i);return`/${t}/status/${r}/mediaviewer${n}`";
fn repair_source(source: &str) -> Option<String> {
    let fixed = repair_permitted_media(source)?;
    if source.matches(VIDEO_ENTRY).count() != 1
        || !source.contains("tweetMediaItemImmersiveViewerEntryPoint")
        || !source.contains("_handleEnterImmersiveViewer")
    {
        return Some(fixed);
    }
    // An absent entry alone is insufficient. The original route factory must
    // first execute with both verified presentation imports in this document.
    Some(fixed.replacen(VIDEO_ENTRY,&format!("if(!e&&t&&r&&window.__atsumiXSurface?.sourceMediaViewerAvailable?.()===true)e=\"/\";{VIDEO_ENTRY}"),1))
}

fn asset_kind(value: &str) -> Option<&'static str> {
    let name = value.strip_prefix("https://abs.twimg.com/responsive-web/client-web/")?;
    let kind = if name.starts_with("shared~bundle.AudioSpaceAnalytics~") {
        "media"
    } else {
        return None;
    };
    (value.len() <= 512
        && name.ends_with(".js")
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"~._-".contains(&c))
        && Url::parse(value).is_ok_and(|url| {
            url.host_str() == Some("abs.twimg.com")
                && url.port().is_none()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
        }))
    .then_some(kind)
}
fn public_asset(value: &str) -> bool {
    asset_kind(value).is_some()
}

#[cfg(windows)]
#[path = "x_surface_source_delivery.rs"]
mod delivery;
pub(super) fn initialization(config: &str) -> String {
    [
        include_str!("x_surface_route_adapter.js"),
        include_str!("x_surface_source_delivery.js"),
    ]
    .join("\n")
    .replace("__ATSUMI_X_SOURCE_COMPAT__", config)
}
#[cfg(windows)]
pub(super) fn install(
    view: &tauri::Webview,
    session: super::XSession,
    pane: std::sync::Arc<super::Pane>,
) {
    delivery::install(view, session, pane);
}
#[cfg(not(windows))]
pub(super) fn install(_: &tauri::Webview, _: super::XSession, _: std::sync::Arc<super::Pane>) {}

#[cfg(test)]
mod tests {
    use super::*;

    // Reduced public source shape, not an account or media response fixture.
    const SOURCE: &str = r#"class V{_renderBlurredMediaTombstone=()=>{let{children:e,displaySensitiveMedia:t,iconOnly:r,mediaVisibilityResults:i}=this.props,{showContent:c}=this.state,d=c||t,p=i?.blurred_image_interstitial?.interstitial_action&&this.context.featureSwitches.isTrue("rweb_age_assurance_flow_enabled"),h=!t||!!p;return jsx(P,{value:{playbackPriority:c?a.c.NORMAL:a.c.INELIGIBLE},children:e})};_renderAgeVerificationModal=()=>{}}"#;
    fn video_source() -> String {
        format!("const entryName='tweetMediaItemImmersiveViewerEntryPoint';const route=(f,e,c)=>((e,t,r)=>{{{VIDEO_ENTRY}}})(f(),e,c);class Viewer{{_handleEnterImmersiveViewer=()=>{{this.context.history.push(this.props.immersiveViewerUrl||this.context.history.location.pathname)}}}}")
    }
    #[test]
    fn absent_original_viewer_entry_is_never_fabricated_from_app_flags() {
        use std::{
            io::Write,
            process::{Command, Stdio},
        };
        let source = repair_source(&format!("{SOURCE};{}", video_source())).unwrap();
        let check = r#"const fs=require('fs'),vm=require('vm'),assert=require('assert');const source=fs.readFileSync(0,'utf8');for(const enabled of [false,true]){const context=vm.createContext({window:{__atsumiXSurface:{feedVideoMode:enabled}}});vm.runInContext(source,context);assert.equal(vm.runInContext('route(()=>undefined,"author","123456")',context),undefined);assert.equal(vm.runInContext('route(()=>"/old?context=feed","author","123456")',context),'/author/status/123456/mediaviewer?context=feed');assert.equal(vm.runInContext('route(()=>undefined,undefined,"123456")',context),undefined);}console.log('original source entry preserved');"#;
        let mut child = Command::new("node")
            .args(["-e", check])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Project Node.js is required");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(source.as_bytes())
            .unwrap();
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    #[test]
    fn verified_source_route_availability_is_required_for_the_optional_entry() {
        let entry = repair_source(&format!("{SOURCE};{}", video_source())).unwrap();
        let program = format!(
            r#"const vm=require('vm'),assert=require('assert');for(const available of [false,true]){{const c=vm.createContext({{window:{{__atsumiXSurface:{{sourceMediaViewerAvailable:()=>available}}}}}});vm.runInContext({entry},c);assert.equal(vm.runInContext(`route(()=>undefined,'author','123456')`,c),available?'/author/status/123456/mediaviewer':undefined);assert.equal(vm.runInContext(`route(()=>'/old?source=original','author','123456')`,c),'/author/status/123456/mediaviewer?source=original');}}"#,
            entry = serde_json::to_string(&entry).unwrap()
        );
        let result = std::process::Command::new("node")
            .args(["-e", &program])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    #[test]
    fn viewer_source_drift_is_untouched_and_does_not_change_access_or_playback_logic() {
        let source = video_source();
        let combined = format!("{SOURCE};{source}");
        let fixed = repair_source(&combined).unwrap();
        assert!(!fixed.contains("feedVideoMode"));
        assert!(fixed.contains(VIDEO_ENTRY));
        assert!(fixed.contains("playbackPriority:(c||t&&!p)?"));
        assert!(fixed.contains("rweb_age_assurance_flow_enabled"));
        for forbidden in [
            ".play(",
            "currentTime=",
            "fetch(",
            "XMLHttpRequest",
            "document.cookie",
            "localStorage",
        ] {
            assert!(!fixed.contains(forbidden));
        }
    }
    #[test]
    #[ignore = "Explicit captured public-code shape check; fixture is not a release asset"]
    fn captured_public_video_entry_shape() {
        let file =
            std::env::var("ATSUMI_PUBLIC_SOURCE_FIXTURE").expect("public fixture path required");
        let source = std::fs::read_to_string(file).unwrap();
        assert!(source.contains(VIDEO_ENTRY));
        assert!(!repair_source(&source).unwrap().contains("feedVideoMode"));
    }
    #[test]
    fn account_permitted_visible_media_remains_eligible_without_a_show_click() {
        let fixed =
            repair_permitted_media(SOURCE).expect("known source predicate must be corrected");
        assert_eq!(
            fixed,
            SOURCE.replace("playbackPriority:c?", "playbackPriority:(c||t&&!p)?")
        );
    }

    #[test]
    fn age_gate_is_mandatory_and_explicit_reveal_still_works() {
        let fixed = repair_permitted_media(SOURCE).unwrap();
        assert!(fixed.contains("playbackPriority:(c||t&&!p)?"));
        for original in [
            SOURCE.replace("rweb_age_assurance_flow_enabled", "different_gate"),
            SOURCE.replace("h=!t||!!p", "h=!t"),
            SOURCE.replace(
                "p=i?.blurred_image_interstitial?.interstitial_action",
                "p=i?.unrelated_flag",
            ),
        ] {
            assert!(repair_permitted_media(&original).is_none());
        }
    }

    #[test]
    fn renamed_minified_flags_work_but_ambiguous_or_changed_source_is_untouched() {
        let renamed = SOURCE
            .replace("displaySensitiveMedia:t", "displaySensitiveMedia:allowed")
            .replace("showContent:c", "showContent:revealed")
            .replace("c||t", "revealed||allowed")
            .replace("p=i?", "blocked=i?")
            .replace("!t||!!p", "!allowed||!!blocked")
            .replace("playbackPriority:c?", "playbackPriority:revealed?");
        assert!(repair_permitted_media(&renamed)
            .unwrap()
            .contains("(revealed||allowed&&!blocked)?"));
        assert!(repair_permitted_media(&SOURCE.repeat(2)).is_none());
        assert!(repair_permitted_media(
            &SOURCE.replace("playbackPriority:c?", "playbackPriority:d?")
        )
        .is_none());
        let fixed = repair_permitted_media(SOURCE).unwrap();
        assert!(repair_permitted_media(&fixed).is_none());
        assert!(repair_permitted_media(&"x".repeat(MAX_SCRIPT + 1)).is_none());
    }

    #[test]
    fn only_canonical_public_script_urls_are_candidates() {
        let url = "https://abs.twimg.com/responsive-web/client-web/shared~bundle.AudioSpaceAnalytics~bundle.fixture.hash.js";
        assert!(public_asset(url));
        assert!(!public_asset(
            "https://abs.twimg.com/responsive-web/client-web/bundle.Routes.observed.js"
        ));
        for value in [
            url.replace("https:", "http:"),
            url.replace("abs.twimg.com", "abs.twimg.com.evil.test"),
            url.replace("abs.twimg.com", "user@abs.twimg.com"),
            url.replace("abs.twimg.com", "abs.twimg.com:444"),
            format!("{url}?token=private"),
            format!("{url}#fragment"),
            url.replace("fixture", "../fixture"),
            url.replace("fixture", "%66ixture"),
            url.replace(".js", ".json"),
        ] {
            assert!(!public_asset(&value), "invalid public asset accepted");
        }
    }
}
