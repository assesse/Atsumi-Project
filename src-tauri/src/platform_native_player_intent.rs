//! Native evidence for a visible player's narrow UI actions, not page IPC authority.
use crate::platform_native_geometry::NativeViewport;
use std::sync::{atomic::{AtomicBool, AtomicU64, Ordering}, Arc, Mutex};
use std::time::{Duration, Instant};

const AGE: Duration = Duration::from_millis(500);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rejection { StateBusy, Missing, BindingChanged, Expired, Window, Focus, PointerUnavailable, PointerOutside, Geometry, Dpi }
impl Rejection {
    pub(crate) fn diagnostic(self)->&'static str {match self{
        Self::StateBusy=>"state_busy",Self::Missing=>"receipt_missing",Self::BindingChanged=>"binding_changed",Self::Expired=>"receipt_expired",
        Self::Window=>"window_unavailable",Self::Focus=>"focus_mismatch",Self::PointerUnavailable=>"pointer_unavailable",
        Self::PointerOutside=>"pointer_outside",Self::Geometry=>"geometry_mismatch",Self::Dpi=>"dpi_changed",
    }}
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Binding {
    pub token: String,
    pub epoch: u64,
    revision: u64,
    player: [f64; 4],
    chat: Option<[f64; 4]>,
    holes: Vec<[f64; 4]>,
}
impl Binding {
    fn contains(&self, x: i32, y: i32, dpi: u32) -> bool {
        let (x,y)=(f64::from(x)*96.0/f64::from(dpi),f64::from(y)*96.0/f64::from(dpi));
        let inside=|r: &[f64;4]| x>=r[0]&&y>=r[1]&&x<r[0]+r[2]&&y<r[1]+r[3];
        inside(&self.player)&&!self.chat.as_ref().is_some_and(inside)&&!self.holes.iter().any(inside)
    }
}
struct Receipt { binding: Binding, at: Instant, dpi: u32 }
#[derive(Default)]
pub(crate) struct Intent {
    presenter: Mutex<Option<Binding>>,
    receipt: Mutex<Option<Receipt>>,
    revision: AtomicU64,
    active: AtomicBool,
}
impl Intent {
    pub(crate) fn document(&self,epoch:u64){let mut slot=self.presenter.lock().unwrap_or_else(|e|e.into_inner());if let Some(value)=slot.as_mut(){value.epoch=epoch;}self.clear();}
    pub(crate) fn present(&self, token: &str, epoch: u64, viewport: &NativeViewport) {
        if viewport.validate().is_err(){self.hide(token);return;}
        if !viewport.visible || viewport.player.is_none() { self.hide(token); return; }
        let p=viewport.player.as_ref().unwrap();
        let mut next=Binding { token:token.into(),epoch,revision:0,player:[p.x,p.y,p.width,p.height],
            chat:viewport.chat.as_ref().map(|r|[r.x,r.y,r.width,r.height]),
            holes:viewport.occlusions.iter().map(|r|[r.x,r.y,r.width,r.height]).collect() };
        let mut slot=self.presenter.lock().unwrap_or_else(|e|e.into_inner());
        if let Some(old)=slot.as_ref(){next.revision=old.revision;if old==&next{return;}}
        next.revision=self.revision.fetch_add(1,Ordering::AcqRel)+1;
        *slot=Some(next);self.clear();
    }
    pub(crate) fn hide(&self, token: &str) {
        let mut slot=self.presenter.lock().unwrap_or_else(|e|e.into_inner());
        if slot.as_ref().is_some_and(|v|v.token==token){*slot=None;self.revision.fetch_add(1,Ordering::AcqRel);self.clear();}
    }
    pub(crate) fn current(&self, epoch: u64) -> Option<Binding> {
        let mut value=self.presenter.try_lock().ok()?.as_ref().filter(|b|b.epoch==epoch).cloned()?;
        value.revision=self.revision.load(Ordering::Acquire);Some(value)
    }
    pub(crate) fn matches(&self, binding: &Binding, epoch: u64) -> bool { self.current(epoch).as_ref()==Some(binding) }
    pub(crate) fn clear(&self) { self.revision.fetch_add(1,Ordering::AcqRel);self.receipt.lock().unwrap_or_else(|e|e.into_inner()).take(); }
    fn take(&self, current: Option<&Binding>, now: Instant) -> Option<(Binding,u32)> {
        self.take_diagnosed(current,now).ok()
    }
    fn take_diagnosed(&self,current:Option<&Binding>,now:Instant)->Result<(Binding,u32),Rejection>{
        let receipt=self.receipt.try_lock().map_err(|_|Rejection::StateBusy)?.take().ok_or(Rejection::Missing)?;
        if current!=Some(&receipt.binding){return Err(Rejection::BindingChanged);}
        if !now.checked_duration_since(receipt.at).is_some_and(|age|age<=AGE){return Err(Rejection::Expired);}
        Ok((receipt.binding,receipt.dpi))
    }
}
pub(crate) type Current = Arc<dyn Fn()->Option<Binding>+Send+Sync>;

#[cfg(windows)]
mod native {
    use super::*;
    use std::{cell::Cell,rc::Rc};
    use webview2_com::{FocusChangedEventHandler,Microsoft::Web::WebView2::Win32::ICoreWebView2Controller};
    use windows::Win32::{Foundation::{HWND,LPARAM,LRESULT,POINT,WPARAM},Graphics::Gdi::ScreenToClient,
        UI::{HiDpi::GetDpiForWindow,Input::KeyboardAndMouse::IsWindowEnabled,
        Shell::{DefSubclassProc,GetWindowSubclass,RemoveWindowSubclass,SetWindowSubclass},
        WindowsAndMessaging::{GetCursorPos,GetForegroundWindow,GetGUIThreadInfo,GetParent,IsChild,IsWindowVisible,WindowFromPoint,GUITHREADINFO,WM_KILLFOCUS,WM_LBUTTONDOWN,WM_NCDESTROY,WM_PARENTNOTIFY,WM_SHOWWINDOW}}};
    const ID:usize=0x41535049;
    struct State {root:HWND,pane:HWND,intent:Arc<Intent>,current:Current,closed:Cell<bool>}
    struct Active<'a>(&'a AtomicBool);
    impl Drop for Active<'_>{fn drop(&mut self){self.0.store(false,Ordering::Release);}}
    unsafe fn context(root:HWND,pane:HWND,b:&Binding,focus:bool)->Result<u32,Rejection>{
        if root.0.is_null()||pane.0.is_null()||root==pane||GetForegroundWindow()!=root||GetParent(pane).ok()!=Some(root)
            ||!IsWindowVisible(root).as_bool()||!IsWindowVisible(pane).as_bool()||!IsWindowEnabled(root).as_bool()||!IsWindowEnabled(pane).as_bool(){return Err(Rejection::Window);}
        if focus {let mut i=GUITHREADINFO{cbSize:std::mem::size_of::<GUITHREADINFO>() as u32,..Default::default()};
            if GetGUIThreadInfo(0,&mut i).is_err()||!(i.hwndFocus==pane||IsChild(pane,i.hwndFocus).as_bool()){return Err(Rejection::Focus);}}
        let mut p=POINT::default();if GetCursorPos(&mut p).is_err(){return Err(Rejection::PointerUnavailable);}
        let hit=WindowFromPoint(p);if hit!=pane&&!IsChild(pane,hit).as_bool(){return Err(Rejection::PointerOutside);}
        let dpi=GetDpiForWindow(root);if !(24..=768).contains(&dpi){return Err(Rejection::Dpi);}
        if !ScreenToClient(root,&mut p).as_bool()||!b.contains(p.x,p.y,dpi){return Err(Rejection::Geometry);}Ok(dpi)
    }
    unsafe extern "system" fn procedure(hwnd:HWND,message:u32,w:WPARAM,l:LPARAM,id:usize,data:usize)->LRESULT{
        if message==WM_NCDESTROY {let _=RemoveWindowSubclass(hwnd,Some(procedure),id);let s=Rc::from_raw(data as *const State);s.closed.set(true);s.intent.clear();return DefSubclassProc(hwnd,message,w,l);}
        Rc::increment_strong_count(data as *const State);let s=Rc::from_raw(data as *const State);
        if message==WM_KILLFOCUS||message==WM_SHOWWINDOW&&w.0==0{s.intent.clear();}
        let active=if message==WM_PARENTNOTIFY&&(w.0&0xffff) as u32==WM_LBUTTONDOWN&&!s.closed.get()
            &&s.intent.active.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_ok(){Some(Active(&s.intent.active))}else{None};
        let at=Instant::now();
        let observed=active.as_ref().and_then(|_|{s.intent.clear();let b=(s.current)()?;let dpi=context(s.root,s.pane,&b,false).ok()?;Some((b,dpi))});
        // The physical notification is evidence even if the button is already up.
        // No lock survives native forwarding; nested notifications cannot rearm.
        let result=DefSubclassProc(hwnd,message,w,l);
        if let Some((b,dpi))=observed {if !s.closed.get()&&(s.current)().as_ref()==Some(&b)&&context(s.root,s.pane,&b,true)==Ok(dpi){
            if let Ok(mut receipt)=s.intent.receipt.try_lock(){*receipt=Some(Receipt{binding:b,at,dpi});}
        }}
        drop(active);result
    }
    pub(crate) unsafe fn install(controller:&ICoreWebView2Controller,root:usize,pane:usize,intent:Arc<Intent>,current:Current)->windows::core::Result<()>{
        let root=HWND(root as *mut _);let pane=HWND(pane as *mut _);let mut existing=0;
        if root==pane||pane.0.is_null()||GetParent(pane).ok()!=Some(root)||GetWindowSubclass(pane,Some(procedure),ID,Some(&mut existing)).as_bool(){return Err(windows::core::Error::from_win32());}
        let state=Rc::new(State{root,pane,intent:intent.clone(),current,closed:Cell::new(false)});let raw=Rc::into_raw(state.clone());
        if !SetWindowSubclass(pane,Some(procedure),ID,raw as usize).as_bool(){drop(Rc::from_raw(raw));return Err(windows::core::Error::from_win32());}
        let mut token=0;
        if let Err(error)=controller.add_LostFocus(&FocusChangedEventHandler::create(Box::new(move|_,_|{intent.clear();Ok(())})),&mut token){
            state.closed.set(true);state.intent.clear();if RemoveWindowSubclass(pane,Some(procedure),ID).as_bool(){drop(Rc::from_raw(raw));}return Err(error);
        }Ok(())
    }
    pub(crate) fn consume(intent:&Intent,current:Option<&Binding>,root:usize,pane:usize)->Option<Binding>{
        consume_diagnosed(intent,current,root,pane).ok()
    }
    pub(crate) fn consume_diagnosed(intent:&Intent,current:Option<&Binding>,root:usize,pane:usize)->Result<Binding,Rejection>{
        let (binding,dpi)=intent.take_diagnosed(current,Instant::now())?;
        let current_dpi=unsafe{context(HWND(root as *mut _),HWND(pane as *mut _),&binding,true)}?;
        if current_dpi!=dpi{return Err(Rejection::Dpi);}Ok(binding)
    }
}
#[cfg(windows)]
pub(crate) use native::{consume,consume_diagnosed,install};

#[cfg(test)]
mod tests {
    use super::*;
    fn view()->NativeViewport{serde_json::from_value(serde_json::json!({"sequence":1,"player":{"x":0,"y":0,"width":100,"height":100},"chat":{"x":80,"y":0,"width":20,"height":100},"visible":true,"muted":false,"occlusions":[]})).unwrap()}
    #[test]fn exact_one_use_expiry_and_replacement(){let i=Intent::default();i.present("one",1,&view());let a=i.current(1).unwrap();let now=Instant::now();
        *i.receipt.lock().unwrap()=Some(Receipt{binding:a.clone(),at:now,dpi:96});assert!(i.take(Some(&a),now).is_some());assert!(i.take(Some(&a),now).is_none());
        *i.receipt.lock().unwrap()=Some(Receipt{binding:a.clone(),at:now,dpi:96});assert!(i.take(Some(&a),now+AGE+Duration::from_millis(1)).is_none());
        i.present("two",1,&view());assert!(!i.matches(&a,1));i.hide("one");assert_eq!(i.current(1).unwrap().token,"two");i.hide("two");assert!(i.current(1).is_none());}
    #[test]fn epoch_geometry_and_chat_are_fenced(){let i=Intent::default();i.present("one",1,&view());let a=i.current(1).unwrap();assert!(a.contains(10,10,96));assert!(!a.contains(90,10,96));assert!(i.current(2).is_none());let mut v=view();v.player.as_mut().unwrap().width=90.0;i.present("one",1,&v);assert!(!i.matches(&a,1));}
    #[test]fn lost_focus_and_document_replace_reject_queued_binding_even_after_reveal(){let i=Intent::default();i.present("one",1,&view());let a=i.current(1).unwrap();i.clear();assert!(!i.matches(&a,1));let b=i.current(1).unwrap();i.hide("one");i.present("one",1,&view());assert!(!i.matches(&b,1));let c=i.current(1).unwrap();i.document(2);assert!(i.current(1).is_none());assert!(!i.matches(&c,2));}
    #[test]fn mismatched_receipt_is_consumed_without_a_held_mouse_fallback(){let i=Intent::default();i.present("one",1,&view());let a=i.current(1).unwrap();let mut wrong=a.clone();wrong.token="other".into();*i.receipt.lock().unwrap()=Some(Receipt{binding:a.clone(),at:Instant::now(),dpi:96});assert!(i.take(Some(&wrong),Instant::now()).is_none());assert!(i.take(Some(&a),Instant::now()).is_none());}
    #[test]fn diagnosis_is_fixed_and_preserves_consumption_and_expiry(){let i=Intent::default();i.present("one",1,&view());let a=i.current(1).unwrap();let now=Instant::now();
        assert_eq!(i.take_diagnosed(Some(&a),now).unwrap_err(),Rejection::Missing);
        *i.receipt.lock().unwrap()=Some(Receipt{binding:a.clone(),at:now,dpi:96});assert_eq!(i.take_diagnosed(None,now).unwrap_err(),Rejection::BindingChanged);assert!(i.take(Some(&a),now).is_none());
        *i.receipt.lock().unwrap()=Some(Receipt{binding:a.clone(),at:now,dpi:96});assert_eq!(i.take_diagnosed(Some(&a),now+AGE+Duration::from_millis(1)).unwrap_err(),Rejection::Expired);
        *i.receipt.lock().unwrap()=Some(Receipt{binding:a.clone(),at:now,dpi:96});assert_eq!(i.take_diagnosed(Some(&a),now+AGE).unwrap().0,a);
        for reason in [Rejection::StateBusy,Rejection::Missing,Rejection::BindingChanged,Rejection::Expired,Rejection::Window,Rejection::Focus,Rejection::PointerUnavailable,Rejection::PointerOutside,Rejection::Geometry,Rejection::Dpi]{assert!(reason.diagnostic().bytes().all(|c|c.is_ascii_lowercase()||c==b'_'));}
    }
}
