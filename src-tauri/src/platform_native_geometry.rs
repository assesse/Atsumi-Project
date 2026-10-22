//! Presentation-only native clipping for independent provider WebViews.
//! No account, receiver, recording, navigation or CHZZK mutable state.
use serde::{Deserialize, Serialize};
use std::sync::{atomic::{AtomicU64, Ordering}, Arc};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Rect { pub x: f64, pub y: f64, pub width: f64, pub height: f64 }
impl Rect {
    fn valid(&self) -> bool {
        [self.x, self.y, self.width, self.height].iter().all(|v| v.is_finite() && v.abs() <= 32768.0)
            && self.width >= 0.0 && self.height >= 0.0
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Hole {
    pub x: f64, pub y: f64, pub width: f64, pub height: f64,
    #[serde(default)] pub radii: [f64; 4],
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeViewport {
    pub sequence: u64, pub player: Option<Rect>, pub chat: Option<Rect>,
    pub visible: bool, pub muted: bool, #[serde(default)] pub occlusions: Vec<Hole>,
}
impl NativeViewport {
    pub fn validate(&self) -> Result<(), String> {
        if self.sequence == 0 || self.sequence > 9_007_199_254_740_991 || self.occlusions.len() > 16
            || self.player.iter().chain(self.chat.iter()).any(|r| !r.valid())
            || self.occlusions.iter().any(|h| !Rect { x: h.x, y: h.y, width: h.width, height: h.height }.valid()
                || h.radii.iter().any(|r| !r.is_finite() || *r < 0.0 || *r > h.width.min(h.height) / 2.0)) {
            return Err("방송 표시 영역이 올바르지 않습니다.".into());
        }
        Ok(())
    }
}

// Copy-free reuse of the pointer-only native repair. Every instance owns its
// exact expected URL, nonce and HWND; CHZZK's registry/profile is not imported.
#[path = "streaming/browser_pane_focus.rs"]
mod pointer_focus;
pub fn pointer_script(expected_url: &str, nonce: &str) -> String { pointer_focus::initialization_script(expected_url, nonce) }
pub fn install_pointer_focus(view: &tauri::Webview, expected_url: String, nonce: String) -> tauri::Result<()> { pointer_focus::install(view, expected_url, nonce) }

/// Read-only physical-intent gate, called synchronously inside a WebMessage
/// callback. `root` and `pane` must come from this exact native Window/controller.
/// For mouse actions send the bridge request on trusted pointerdown, not click
/// (click is delivered after release). This never moves focus or consumes input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputGate {
    InvalidInput, WindowUnavailable, KeyReleased, FocusMismatch, PointerOutside, Accepted,
}
impl InputGate {
    pub fn allowed(self) -> bool { self == Self::Accepted }
    pub fn diagnostic(self) -> &'static str { match self {
        Self::InvalidInput => "invalid_input", Self::WindowUnavailable => "window_unavailable",
        Self::KeyReleased => "key_released", Self::FocusMismatch => "focus_mismatch",
        Self::PointerOutside => "pointer_outside", Self::Accepted => "accepted",
    } }
}
fn evaluate_input_gate(valid: bool, window: impl FnOnce() -> bool, key: impl FnOnce() -> bool,
    focus: impl FnOnce() -> bool, pointer: impl FnOnce() -> bool) -> InputGate {
    if !valid { return InputGate::InvalidInput; }
    if !window() { return InputGate::WindowUnavailable; }
    if !key() { return InputGate::KeyReleased; }
    if !focus() { return InputGate::FocusMismatch; }
    if !pointer() { return InputGate::PointerOutside; }
    InputGate::Accepted
}
#[cfg(windows)]
pub fn trusted_input(root:usize,pane:usize,key:i32)->bool {
    trusted_input_gate(root,pane,key).allowed()
}
/// Fixed classification of the unchanged physical-intent checks. No raw input
/// value, window identity or page content is included in this result.
#[cfg(windows)]
pub fn trusted_input_gate(root:usize,pane:usize,key:i32)->InputGate {
    input_gate_with_receipt(root,pane,key,false)
}
/// Caller must already hold a single-use native Escape receipt bound to its
/// exact document/fullscreen grant. Recheck all window/pane/focus conditions.
#[cfg(windows)]
pub(crate) fn trusted_escape_receipt_context(root:usize,pane:usize)->InputGate {
    input_gate_with_receipt(root,pane,0x1B,true)
}
#[cfg(windows)]
fn input_gate_with_receipt(root:usize,pane:usize,key:i32,escape_receipt:bool)->InputGate {
    use windows::Win32::{Foundation::{HWND,POINT},UI::{Input::KeyboardAndMouse::{GetAsyncKeyState,IsWindowEnabled},WindowsAndMessaging::{GetForegroundWindow,GetGUIThreadInfo,GUITHREADINFO,IsChild,IsWindowVisible,GetParent,GetCursorPos,WindowFromPoint}}};
    let valid=matches!(key,1|0x1B|0x46|0x53)&&root!=0&&pane!=0&&root!=pane;
    let root=HWND(root as *mut std::ffi::c_void);let pane=HWND(pane as *mut std::ffi::c_void);
    unsafe {
        evaluate_input_gate(valid,
            || GetForegroundWindow()==root&&GetParent(pane).ok()==Some(root)&&IsWindowVisible(pane).as_bool()&&IsWindowEnabled(pane).as_bool()&&IsWindowEnabled(root).as_bool(),
            || escape_receipt&&key==0x1B || GetAsyncKeyState(key) as u16&0x8000!=0,
            || { let mut info=GUITHREADINFO{cbSize:std::mem::size_of::<GUITHREADINFO>() as u32,..Default::default()};
                GetGUIThreadInfo(0,&mut info).is_ok()&&(info.hwndFocus==pane||IsChild(pane,info.hwndFocus).as_bool()) },
            || { if key!=1{return true;} let mut point=POINT::default();if GetCursorPos(&mut point).is_err(){return false;}let hit=WindowFromPoint(point);hit==pane||IsChild(pane,hit).as_bool() })
    }
}

#[cfg(windows)]
mod region {
    use super::*;
    use windows::Win32::{Foundation::HWND, Graphics::Gdi::{CombineRgn, CreateEllipticRgn, CreateRectRgn, DeleteObject, EqualRgn, GetWindowRgn, SetWindowRgn, HGDIOBJ, HRGN, RGN_DIFF, RGN_OR}};
    pub struct Region(HRGN);
    impl Region {
        fn rect(x: i32, y: i32, r: i32, b: i32) -> Result<Self, ()> {
            let h = unsafe { CreateRectRgn(x, y, r, b) }; if h.0.is_null() { Err(()) } else { Ok(Self(h)) }
        }
        fn combine(&self, other: &Self, subtract: bool) -> Result<(), ()> {
            if unsafe { CombineRgn(Some(self.0), Some(self.0), Some(other.0), if subtract { RGN_DIFF } else { RGN_OR }) }.0 == 0 { Err(()) } else { Ok(()) }
        }
        pub fn build(v: &NativeViewport, scale: f64, width: u32, height: u32) -> Result<Self, ()> {
            let region = Self::rect(0, 0, 0, 0)?;
            if !v.visible { return Ok(region); }
            let px = |n: f64| (n * scale).round() as i32;
            for r in v.player.iter().chain(v.chat.iter()) {
                let left = px(r.x).clamp(0, width as i32); let top = px(r.y).clamp(0, height as i32);
                let right = px(r.x + r.width).clamp(0, width as i32); let bottom = px(r.y + r.height).clamp(0, height as i32);
                if right > left && bottom > top { region.combine(&Self::rect(left, top, right, bottom)?, false)?; }
            }
            for h in &v.occlusions {
                let (l,t,r,b) = (px(h.x), px(h.y), px(h.x+h.width), px(h.y+h.height));
                let mask = Self::rect(l,t,r,b)?;
                for (corner, radius) in h.radii.iter().map(|v| px(*v)).enumerate() {
                    if radius <= 0 { continue; }
                    let x = if corner == 0 || corner == 3 { l } else { r-radius };
                    let y = if corner < 2 { t } else { b-radius };
                    let outside = Self::rect(x,y,x+radius,y+radius)?;
                    let ex = if corner == 0 || corner == 3 { l } else { r-2*radius };
                    let ey = if corner < 2 { t } else { b-2*radius };
                    let ellipse = unsafe { CreateEllipticRgn(ex,ey,ex+2*radius,ey+2*radius) };
                    if ellipse.0.is_null() { return Err(()); }
                    outside.combine(&Self(ellipse), true)?; mask.combine(&outside, true)?;
                }
                region.combine(&mask, true)?;
            }
            Ok(region)
        }
        pub fn apply(self, hwnd: HWND) -> Result<(), ()> {
            let old = Self::rect(0,0,0,0)?;
            if unsafe { GetWindowRgn(hwnd, old.0) }.0 != 0 && unsafe { EqualRgn(old.0,self.0) }.as_bool() { return Ok(()); }
            if unsafe { SetWindowRgn(hwnd,Some(self.0),true) } == 0 { return Err(()); }
            std::mem::forget(self); Ok(())
        }
    }
    impl Drop for Region { fn drop(&mut self) { unsafe { let _ = DeleteObject(HGDIOBJ(self.0.0)); } } }
    #[cfg(test)]
    mod tests {
        use super::*;
        use windows::Win32::Graphics::Gdi::PtInRegion;
        fn inside(region: &Region, x:i32, y:i32)->bool { unsafe { PtInRegion(region.0,x,y).as_bool() } }
        #[test]
        fn gaps_and_rounded_overlays_leave_input_with_main_renderer() {
            let mut v=NativeViewport { sequence:1,player:Some(Rect{x:0.,y:0.,width:200.,height:200.}),
                chat:Some(Rect{x:250.,y:0.,width:100.,height:200.}),visible:true,muted:false,
                occlusions:vec![Hole{x:20.,y:20.,width:100.,height:100.,radii:[20.;4]}] };
            let r=Region::build(&v,1.,400,250).unwrap();
            assert!(inside(&r,10,10)); assert!(!inside(&r,220,10)); assert!(inside(&r,280,10));
            assert!(!inside(&r,70,70)); assert!(inside(&r,20,20)); assert!(!inside(&r,70,20));
            v.visible=false;
            let hidden=Region::build(&v,1.,400,250).unwrap(); assert!(!inside(&hidden,10,10));
            v.visible=true;
            let scaled=Region::build(&v,2.,800,500).unwrap();
            assert!(inside(&scaled,560,20)); assert!(!inside(&scaled,440,20)); assert!(!inside(&scaled,140,140));
        }
    }
}

/// Queue one revision-fenced presentation update. Never navigates, activates,
/// focuses or changes playback state. Hidden surfaces keep their audio policy.
#[cfg(windows)]
pub fn paint(view: &tauri::Webview, viewport: &NativeViewport, revision: Arc<AtomicU64>, expected: u64) -> Result<(), String> {
    use windows::{core::Interface, Win32::{Foundation::{HWND, RECT}, UI::WindowsAndMessaging::{SetWindowPos, ShowWindow, SW_HIDE, SW_SHOWNA, SWP_NOACTIVATE, SWP_NOZORDER}}};
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_8;
    viewport.validate()?;
    let v = viewport.clone();
    let scale = view.window().scale_factor().map_err(|_| "방송 배율을 확인하지 못했습니다.")?;
    let size = view.window().inner_size().map_err(|_| "방송 창 크기를 확인하지 못했습니다.")?;
    if !scale.is_finite() || !(0.25..=8.0).contains(&scale) || size.width > 65536 || size.height > 65536 { return Err("방송 창 크기가 올바르지 않습니다.".into()); }
    view.with_webview(move |platform| unsafe {
        if revision.load(Ordering::Acquire) != expected { return; }
        let controller = platform.controller();
        let mut hwnd = HWND::default(); if controller.ParentWindow(&mut hwnd).is_err() { return; }
        let apply = || -> Result<(), ()> {
            let core = controller.CoreWebView2().map_err(|_| ())?;
            core.cast::<ICoreWebView2_8>().map_err(|_| ())?.SetIsMuted(v.muted).map_err(|_| ())?;
            let region = region::Region::build(&v, scale, size.width, size.height)?;
            region.apply(hwnd)?;
            // Page roots use main-client coordinates so nonadjacent player/chat
            // can share one receiver while their union alone receives input.
            SetWindowPos(hwnd,None,0,0,size.width as i32,size.height as i32,SWP_NOACTIVATE|SWP_NOZORDER).map_err(|_| ())?;
            controller.SetBounds(RECT{left:0,top:0,right:size.width as i32,bottom:size.height as i32}).map_err(|_| ())?;
            if revision.load(Ordering::Acquire) != expected { return Ok(()); }
            // Hiding pixels is independent from receiver lifetime and mute.
            let _ = ShowWindow(hwnd, if v.visible { SW_SHOWNA } else { SW_HIDE });
            controller.SetIsVisible(true).map_err(|_| ())?;
            Ok(())
        };
        if apply().is_err() {
            let _ = ShowWindow(hwnd,SW_HIDE);
            tracing::warn!(diag_stage="platform_native_geometry_failed");
        }
    }).map_err(|_| "방송 표시 영역을 적용하지 못했습니다.".into())
}
#[cfg(not(windows))]
pub fn paint(_: &tauri::Webview, _: &NativeViewport, _: Arc<AtomicU64>, _: u64) -> Result<(), String> { Err("공식 방송 창은 Windows 앱에서 지원합니다.".into()) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn input_gate_preserves_every_deny_condition_and_short_circuits_in_order() {
        for failure in 0..=5 {
            let visited=std::cell::RefCell::new(Vec::new());
            let observe=|index| { visited.borrow_mut().push(index); failure!=index };
            let gate=evaluate_input_gate(failure!=0,||observe(1),||observe(2),||observe(3),||observe(4));
            assert_eq!(gate,[InputGate::InvalidInput,InputGate::WindowUnavailable,InputGate::KeyReleased,InputGate::FocusMismatch,InputGate::PointerOutside,InputGate::Accepted][failure]);
            assert_eq!(gate.allowed(),failure==5);
            assert_eq!(*visited.borrow(),(1..=failure.min(4)).collect::<Vec<_>>());
        }
    }
    #[test] fn input_gate_diagnostics_are_fixed_and_key_up_never_becomes_permission() {
        assert_eq!(evaluate_input_gate(true,||true,||false,||true,||true),InputGate::KeyReleased);
        for (gate,text) in [(InputGate::InvalidInput,"invalid_input"),(InputGate::WindowUnavailable,"window_unavailable"),(InputGate::KeyReleased,"key_released"),(InputGate::FocusMismatch,"focus_mismatch"),(InputGate::PointerOutside,"pointer_outside"),(InputGate::Accepted,"accepted")] {assert_eq!(gate.diagnostic(),text);}
    }
    fn viewport() -> NativeViewport { NativeViewport { sequence:1, player:Some(Rect{x:0.0,y:0.0,width:500.0,height:300.0}),chat:None,visible:true,muted:false,occlusions:vec![] } }
    #[test] fn rejects_unbounded_or_nonfinite_geometry() {
        let mut v=viewport(); assert!(v.validate().is_ok()); v.sequence=0; assert!(v.validate().is_err());
        v=viewport(); v.player.as_mut().unwrap().x=f64::NAN; assert!(v.validate().is_err());
        v=viewport(); v.player.as_mut().unwrap().height=-1.0; assert!(v.validate().is_err());
        v=viewport(); v.occlusions=vec![Hole{x:0.0,y:0.0,width:10.0,height:10.0,radii:[6.0;4]}]; assert!(v.validate().is_err());
        v.occlusions[0].radii=[0.0;4]; v.occlusions=vec![v.occlusions[0].clone();17]; assert!(v.validate().is_err());
    }
    #[test] fn hidden_is_not_muted_and_empty_presentation_is_valid() {
        let mut v=viewport(); v.visible=false; v.player=None; assert!(v.validate().is_ok()); assert!(!v.muted);
    }
}
