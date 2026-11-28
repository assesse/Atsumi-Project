use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io,
    net::{IpAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
const ENGINE_HASH: &str = "b176b561fd8bf15d828fcab484cfd5b4fb941cb9f61807901ca64b955af27e1f";
#[derive(Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub config: String,
}

const MAX_PROFILES: usize = 32;
#[derive(Serialize, Deserialize)]
pub struct SavedProfile {
    pub id: String,
    pub profile: Profile,
}
#[derive(Serialize, Deserialize, Default)]
pub struct Profiles {
    pub profiles: Vec<SavedProfile>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSummary {
    pub id: String,
    pub name: String,
    // A filename hint for the picker, never proof of the VPN's exit location.
    pub country_hint: Option<String>,
}
impl Profiles {
    pub fn summaries(&self) -> Vec<ProfileSummary> {
        self.profiles
            .iter()
            .map(|p| ProfileSummary {
                id: p.id.clone(),
                name: p.profile.name.clone(),
                country_hint: country_hint(&p.profile.name),
            })
            .collect()
    }
    pub fn selected(&self, id: Option<&str>) -> Result<&SavedProfile, String> {
        match id {
            Some(id) => self.profiles.iter().find(|p| p.id == id).ok_or_else(|| {
                "선택한 VPN 설정을 찾지 못했습니다. 국가를 다시 선택해 주세요.".into()
            }),
            None => self
                .profiles
                .first()
                .ok_or_else(|| "먼저 Proton WireGuard 설정 파일을 가져와 주세요.".into()),
        }
    }
    pub fn add(&mut self, mut profile: Profile) -> Result<String, String> {
        profile.config = sanitize(&profile.config)?;
        if let Some(existing) = self
            .profiles
            .iter()
            .find(|p| p.profile.config == profile.config)
        {
            return Ok(existing.id.clone());
        }
        if self.profiles.len() >= MAX_PROFILES {
            return Err("저장 가능한 VPN 설정은 32개입니다.".into());
        }
        profile.name = profile
            .name
            .chars()
            .filter(|c| !c.is_control())
            .take(80)
            .collect();
        let id = uuid::Uuid::new_v4().simple().to_string();
        self.profiles.push(SavedProfile {
            id: id.clone(),
            profile,
        });
        Ok(id)
    }
}
fn country_hint(name: &str) -> Option<String> {
    let upper = name.to_ascii_uppercase();
    for code in ["JP", "US", "NL", "SG", "CA", "MX", "NO", "PL", "RO", "CH"] {
        let marker = format!("{code}-FREE");
        if upper
            .match_indices(&marker)
            .any(|(index, _)| index == 0 || !upper.as_bytes()[index - 1].is_ascii_alphabetic())
        {
            return Some(code.into());
        }
    }
    None
}
pub fn load_profiles(root: &Path) -> Result<Profiles, String> {
    let bytes = match std::fs::read(root.join("vpn-profiles.dpapi")) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // Preserve the old file; the first collection save migrates without deleting it.
            return Ok(Profiles {
                profiles: load(&root.join("vpn.dpapi"))?
                    .map(|profile| {
                        vec![SavedProfile {
                            id: "legacy".into(),
                            profile,
                        }]
                    })
                    .unwrap_or_default(),
            });
        }
        Err(_) => return Err("저장된 국가 설정을 읽지 못했습니다.".into()),
    };
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("국가 설정 파일이 손상됐습니다.".into());
    }
    let plain = crypt(&bytes, false)?;
    let mut profiles: Profiles =
        serde_json::from_slice(&plain).map_err(|_| "저장된 국가 설정을 읽지 못했습니다.")?;
    if profiles.profiles.len() > MAX_PROFILES {
        return Err("국가 설정 파일이 손상됐습니다.".into());
    }
    let mut ids = std::collections::HashSet::new();
    for p in &mut profiles.profiles {
        if p.id.is_empty()
            || p.id.len() > 64
            || !p.id.chars().all(|c| c.is_ascii_alphanumeric())
            || !ids.insert(p.id.clone())
            || p.profile.name.len() > 320
            || p.profile.name.chars().any(char::is_control)
        {
            return Err("국가 설정 파일이 손상됐습니다.".into());
        }
        p.profile.config = sanitize(&p.profile.config)?;
    }
    Ok(profiles)
}
pub fn save_profiles(root: &Path, profiles: &Profiles) -> Result<(), String> {
    let bytes = serde_json::to_vec(profiles).map_err(|_| "국가 설정을 저장하지 못했습니다.")?;
    let encrypted = crypt(&bytes, true)?;
    let temporary = root.join("vpn-profiles.pending");
    std::fs::write(&temporary, encrypted).map_err(|_| "국가 설정을 저장하지 못했습니다.")?;
    replace(&temporary, &root.join("vpn-profiles.dpapi"))
}

pub fn sanitize(input: &str) -> Result<String, String> {
    let bad = || {
        "유효한 Proton WireGuard 설정 파일이 아닙니다. 공식 사이트에서 받은 .conf 파일을 선택해 주세요.".to_string()
    };
    if input.len() > 32768 || input.contains('\0') {
        return Err(bad());
    }
    let mut values = BTreeMap::new();
    let mut section = "";
    let mut seen = std::collections::HashSet::new();
    for line in input.lines() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            section = match line {
                "[Interface]" => "Interface",
                "[Peer]" => "Peer",
                _ => return Err(bad()),
            };
            if !seen.insert(section) {
                return Err(bad());
            }
            continue;
        }
        let (key, value) = line.split_once('=').ok_or_else(bad)?;
        let key = key.trim();
        let value = value.split('#').next().unwrap_or("").trim();
        let allowed = match section {
            "Interface" => matches!(key, "PrivateKey" | "Address" | "DNS" | "MTU"),
            "Peer" => matches!(
                key,
                "PublicKey" | "PresharedKey" | "AllowedIPs" | "Endpoint" | "PersistentKeepalive"
            ),
            _ => false,
        };
        if !allowed || value.is_empty() || values.insert((section, key), value).is_some() {
            return Err(bad());
        }
    }
    for (s, k) in [("Interface", "PrivateKey"), ("Peer", "PublicKey")] {
        if STANDARD
            .decode(values.get(&(s, k)).ok_or_else(bad)?)
            .map_err(|_| bad())?
            .len()
            != 32
        {
            return Err(bad());
        }
    }
    if let Some(key) = values.get(&("Peer", "PresharedKey")) {
        if STANDARD.decode(key).map_err(|_| bad())?.len() != 32 {
            return Err(bad());
        }
    }
    for (s, k, cidr) in [
        ("Interface", "Address", true),
        ("Interface", "DNS", false),
        ("Peer", "AllowedIPs", true),
    ] {
        let list = values.get(&(s, k)).ok_or_else(bad)?;
        if list.split(',').count() > 8 {
            return Err(bad());
        }
        for value in list.split(',').map(str::trim) {
            if cidr {
                let (ip, bits) = value.split_once('/').ok_or_else(bad)?;
                let ip = ip.parse::<IpAddr>().map_err(|_| bad())?;
                if bits.parse::<u8>().map_err(|_| bad())? > if ip.is_ipv4() { 32 } else { 128 } {
                    return Err(bad());
                }
            } else {
                value.parse::<IpAddr>().map_err(|_| bad())?;
            }
        }
    }
    if !values
        .get(&("Peer", "AllowedIPs"))
        .unwrap()
        .split(',')
        .any(|s| s.trim() == "0.0.0.0/0")
    {
        return Err(bad());
    }
    let endpoint = values.get(&("Peer", "Endpoint")).ok_or_else(bad)?;
    let url = tauri::Url::parse(&format!("http://{endpoint}")).map_err(|_| bad())?;
    if url.port().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || !super::proxy::public_host(url.host_str().ok_or_else(bad)?)
    {
        return Err(bad());
    }
    for (s, k, min, max) in [
        ("Interface", "MTU", 576, 1500),
        ("Peer", "PersistentKeepalive", 0, 65535),
    ] {
        if let Some(value) = values.get(&(s, k)) {
            let n = value.parse::<u32>().map_err(|_| bad())?;
            if n < min || n > max {
                return Err(bad());
            }
        }
    }
    let mut output = String::new();
    for section in ["Interface", "Peer"] {
        output.push_str(&format!("[{section}]\n"));
        for ((s, k), v) in &values {
            if *s == section {
                output.push_str(&format!("{k} = {v}\n"));
            }
        }
    }
    Ok(output)
}
pub fn save(path: &Path, profile: &Profile) -> Result<(), String> {
    let bytes = serde_json::to_vec(profile).map_err(|_| "설정을 저장하지 못했습니다.")?;
    let protected = crypt(&bytes, true)?;
    let tmp = path.with_extension("pending");
    std::fs::write(&tmp, protected).map_err(|_| "VPN 설정을 저장하지 못했습니다.")?;
    replace(&tmp, path).map_err(|_| "VPN 설정을 저장하지 못했습니다.".into())
}
pub fn load(path: &Path) -> Result<Option<Profile>, String> {
    let bytes = match std::fs::read(path) {
        Ok(v) => v,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("저장된 VPN 설정을 읽지 못했습니다.".into()),
    };
    if bytes.len() > 65536 {
        return Err("VPN 설정 파일이 손상됐습니다.".into());
    }
    let plain = crypt(&bytes, false)?;
    let mut profile: Profile =
        serde_json::from_slice(&plain).map_err(|_| "저장된 VPN 설정을 읽지 못했습니다.")?;
    profile.config = sanitize(&profile.config)?;
    Ok(Some(profile))
}
#[cfg(windows)]
fn crypt(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    use windows::{
        core::w,
        Win32::{
            Foundation::{LocalFree, HLOCAL},
            Security::Cryptography::{
                CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
            },
        },
    };
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        let result = if encrypt {
            CryptProtectData(
                &input,
                w!("Atsumi browser VPN"),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        result.map_err(|_| "Windows가 VPN 설정을 암호화하거나 복원하지 못했습니다.")?;
        if output.pbData.is_null() {
            return Err("VPN 설정을 보호하지 못했습니다.".into());
        }
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
        Ok(bytes)
    }
}
#[cfg(not(windows))]
fn crypt(_: &[u8], _: bool) -> Result<Vec<u8>, String> {
    Err("Windows 전용 기능입니다.".into())
}
#[cfg(windows)]
pub(super) fn replace(from: &Path, to: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        },
    };
    let f: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let t: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        MoveFileExW(
            PCWSTR(f.as_ptr()),
            PCWSTR(t.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|_| "VPN 설정을 저장하지 못했습니다.".into())
}
#[cfg(not(windows))]
pub(super) fn replace(_: &Path, _: &Path) -> Result<(), String> {
    Err("Windows 전용 기능입니다.".into())
}
pub struct Engine {
    child: Child,
    pub port: u16,
}
impl Engine {
    pub fn start(exe: &Path, config: &str) -> Result<Self, String> {
        let bytes = std::fs::read(exe).map_err(|_| {
            "VPN 연결 엔진이 없습니다. 브라우저 도구가 포함된 빌드를 사용해 주세요."
        })?;
        if format!("{:x}", Sha256::digest(&bytes)) != ENGINE_HASH {
            return Err("VPN 연결 엔진의 무결성 검사에 실패했습니다.".into());
        }
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|_| "VPN 연결 포트를 준비하지 못했습니다.")?;
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let pipe = ConfigPipe::new()?;
        let mut cmd = Command::new(exe);
        cmd.args(["-s", "-c", &pipe.name])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        let child = cmd
            .spawn()
            .map_err(|_| "VPN 연결 엔진을 시작하지 못했습니다.")?;
        let mut engine = Self { child, port };
        let mut content = format!(
            "{}\n[Socks5]\nBindAddress = 127.0.0.1:{port}\n",
            sanitize(config)?
        )
        .into_bytes();
        let sent = pipe.send(engine.child.id(), &content);
        content.fill(0);
        sent?;
        let until = Instant::now() + Duration::from_secs(8);
        while Instant::now() < until {
            if !engine.alive() {
                return Err("VPN 연결 엔진이 종료됐습니다. 설정 파일을 다시 확인해 주세요.".into());
            }
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return Ok(engine);
            }
            thread::sleep(Duration::from_millis(50));
        }
        Err("VPN 연결 엔진이 응답하지 않습니다.".into())
    }
    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
// A one-shot named pipe keeps the decrypted private key off disk and argv.
#[cfg(windows)]
struct ConfigPipe {
    name: String,
    file: std::fs::File,
}
#[cfg(windows)]
impl ConfigPipe {
    fn new() -> Result<Self, String> {
        use std::os::windows::io::FromRawHandle;
        use windows::{
            core::PCWSTR,
            Win32::{
                Storage::FileSystem::{
                    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_OUTBOUND,
                },
                System::Pipes::{
                    CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
                    PIPE_TYPE_BYTE, PIPE_WAIT,
                },
            },
        };
        let name = format!(
            r"\\.\pipe\atsumi-browser-vpn-{}",
            uuid::Uuid::new_v4().simple()
        );
        let wide: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
        let h = unsafe {
            CreateNamedPipeW(
                PCWSTR(wide.as_ptr()),
                PIPE_ACCESS_OUTBOUND | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                65536,
                0,
                5000,
                None,
            )
        };
        if h.is_invalid() {
            return Err("VPN 설정 전송을 준비하지 못했습니다.".into());
        }
        Ok(Self {
            name,
            file: unsafe { std::fs::File::from_raw_handle(h.0) },
        })
    }
    fn send(self, pid: u32, bytes: &[u8]) -> Result<(), String> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::{CloseHandle, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, HANDLE},
            Storage::FileSystem::{FlushFileBuffers, WriteFile},
            System::{
                Pipes::{ConnectNamedPipe, GetNamedPipeClientProcessId},
                Threading::CreateEventW,
                IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
            },
        };
        let fail = || "VPN 설정을 안전하게 전달하지 못했습니다.".to_string();
        let handle = HANDLE(self.file.as_raw_handle());
        unsafe {
            let event = CreateEventW(None, true, false, None).map_err(|_| fail())?;
            let mut overlap = OVERLAPPED {
                hEvent: event,
                ..Default::default()
            };
            let mut count = 0;
            let result = (|| {
                match ConnectNamedPipe(handle, Some(&mut overlap)) {
                    Ok(()) => {}
                    Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => {}
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {
                        use windows::Win32::Foundation::WAIT_OBJECT_0;
                        use windows::Win32::System::Threading::WaitForSingleObject;
                        if WaitForSingleObject(event, 5000) != WAIT_OBJECT_0 {
                            let _ = CancelIoEx(handle, Some(&overlap));
                            let _ = GetOverlappedResult(handle, &overlap, &mut count, true);
                            return Err(fail());
                        }
                        GetOverlappedResult(handle, &overlap, &mut count, true)
                            .map_err(|_| fail())?;
                    }
                    Err(_) => return Err(fail()),
                }
                let mut owner = 0;
                GetNamedPipeClientProcessId(handle, &mut owner).map_err(|_| fail())?;
                if owner != pid {
                    return Err(fail());
                }
                // <=32KiB configuration fits the 64KiB pipe buffer; complete the overlapped write.
                windows::Win32::System::Threading::ResetEvent(event).map_err(|_| fail())?;
                match WriteFile(handle, Some(bytes), Some(&mut count), Some(&mut overlap)) {
                    Ok(()) => {}
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {
                        use windows::Win32::System::IO::GetOverlappedResultEx;
                        if GetOverlappedResultEx(handle, &overlap, &mut count, 5000, false).is_err()
                        {
                            let _ = CancelIoEx(handle, Some(&overlap));
                            let _ = GetOverlappedResult(handle, &overlap, &mut count, true);
                            return Err(fail());
                        }
                    }
                    Err(_) => return Err(fail()),
                }
                if count != bytes.len() as u32 {
                    return Err(fail());
                } // Closing delivers EOF after buffered bytes.
                FlushFileBuffers(handle).map_err(|_| fail())?;
                Ok(())
            })();
            let _ = CloseHandle(event);
            result
        }
    }
}
#[cfg(not(windows))]
struct ConfigPipe {
    name: String,
}
#[cfg(not(windows))]
impl ConfigPipe {
    fn new() -> Result<Self, String> {
        Err("Windows 전용 기능입니다.".into())
    }
    fn send(self, _: u32, _: &[u8]) -> Result<(), String> {
        Err("Windows 전용 기능입니다.".into())
    }
}
pub fn pick() -> Result<Option<PathBuf>, String> {
    #[cfg(windows)]
    {
        use windows::{
            core::HRESULT,
            Win32::{
                System::Com::{
                    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
                    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
                },
                UI::Shell::{
                    FileOpenDialog, IFileOpenDialog, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM,
                    SIGDN_FILESYSPATH,
                },
            },
        };
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|_| "파일 선택기를 준비하지 못했습니다.")?;
        }
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe { CoUninitialize() }
            }
        }
        let _apartment = Apartment;
        unsafe {
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
                    .map_err(|_| "파일 선택기를 열지 못했습니다.")?;
            dialog
                .SetOptions(FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST)
                .map_err(|_| "파일 선택기를 준비하지 못했습니다.")?;
            if let Err(e) = dialog.Show(None) {
                if e.code() == HRESULT::from_win32(1223) {
                    return Ok(None);
                }
                return Err("파일을 선택하지 못했습니다.".into());
            }
            let item = dialog
                .GetResult()
                .map_err(|_| "선택한 파일을 읽지 못했습니다.")?;
            let path = item
                .GetDisplayName(SIGDN_FILESYSPATH)
                .map_err(|_| "파일 경로를 읽지 못했습니다.")?;
            let value = path
                .to_string()
                .map(PathBuf::from)
                .map_err(|_| "파일 경로가 올바르지 않습니다.".into());
            CoTaskMemFree(Some(path.0.cast()));
            value.map(Some)
        }
    }
    #[cfg(not(windows))]
    {
        Err("Windows 전용 기능입니다.".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> String {
        format!("[Interface]\nPrivateKey = {}\nAddress = 10.2.0.2/32\nDNS = 10.2.0.1\n[Peer]\nPublicKey = {}\nAllowedIPs = 0.0.0.0/0, ::/0\nEndpoint = 185.159.157.1:51820\n",STANDARD.encode([1;32]),STANDARD.encode([2;32]))
    }
    #[test]
    fn import_accepts_standard_proton_configuration() {
        let source = config();
        let parsed = sanitize(&source).unwrap();
        assert_eq!(sanitize(&parsed).unwrap(), parsed);
    }
    #[test]
    fn import_rejects_hooks_includes_routes_and_multiple_peers() {
        for extra in [
            "PostUp = cmd /c bad",
            "WGConfig = secret",
            "[Socks5]\nBindAddress = 0.0.0.0:1",
            "[Peer]\nPublicKey = bad",
        ] {
            assert!(sanitize(&format!("{}{extra}", config())).is_err());
        }
        assert!(sanitize(&config().replace("0.0.0.0/0", "10.0.0.0/8")).is_err());
        assert!(sanitize(&config().replace("185.159.157.1", "127.0.0.1")).is_err());
    }
    #[test]
    fn protected_profile_roundtrip_has_no_plaintext_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.bin");
        let p = Profile {
            name: "JP Free".into(),
            config: config(),
        };
        save(&path, &p).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("PrivateKey"));
        assert_eq!(
            load(&path).unwrap().unwrap().config,
            sanitize(&p.config).unwrap()
        );
    }
    #[test]
    fn saved_countries_keep_legacy_key_and_roundtrip_without_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("vpn.dpapi");
        save(
            &legacy,
            &Profile {
                name: "Atsumi_Browser_JP-JP-FREE-1".into(),
                config: config(),
            },
        )
        .unwrap();
        let original = std::fs::read(&legacy).unwrap();
        let mut profiles = load_profiles(dir.path()).unwrap();
        assert_eq!(profiles.selected(None).unwrap().id, "legacy");
        let us = profiles
            .add(Profile {
                name: "Atsumi_Browser_US-US-FREE-1".into(),
                config: config().replace("185.159.157.1", "185.159.157.2"),
            })
            .unwrap();
        save_profiles(dir.path(), &profiles).unwrap();
        let reopened = load_profiles(dir.path()).unwrap();
        assert_eq!(reopened.profiles.len(), 2);
        assert_eq!(
            reopened.selected(Some(&us)).unwrap().profile.name,
            "Atsumi_Browser_US-US-FREE-1"
        );
        assert_eq!(
            reopened.selected(Some("legacy")).unwrap().profile.config,
            sanitize(&config()).unwrap()
        );
        assert_eq!(std::fs::read(&legacy).unwrap(), original);
        let cipher = std::fs::read(dir.path().join("vpn-profiles.dpapi")).unwrap();
        assert!(!String::from_utf8_lossy(&cipher).contains("PrivateKey"));
        assert!(!String::from_utf8_lossy(&cipher).contains(&STANDARD.encode([1; 32])));
        let metadata = serde_json::to_string(&reopened.summaries()).unwrap();
        assert!(!metadata.contains("config") && !metadata.contains("PrivateKey"));
        assert_eq!(reopened.summaries()[1].country_hint.as_deref(), Some("US"));
    }
    #[test]
    fn duplicate_country_import_preserves_id_and_other_selections() {
        let mut profiles = Profiles::default();
        let first = profiles
            .add(Profile {
                name: "JP-FREE-1".into(),
                config: config(),
            })
            .unwrap();
        let duplicate = profiles
            .add(Profile {
                name: "renamed".into(),
                config: format!("# comment\n{}", config()),
            })
            .unwrap();
        assert_eq!(first, duplicate);
        assert_eq!(profiles.profiles.len(), 1);
        assert_eq!(
            profiles.selected(Some(&first)).unwrap().profile.name,
            "JP-FREE-1"
        );
        assert!(profiles.selected(Some("missing")).is_err());
        assert!(Profiles::default().selected(None).is_err());
    }
    #[test]
    fn country_labels_are_only_bounded_filename_hints() {
        assert_eq!(
            country_hint("Atsumi_Browser_jp-jp-free-1"),
            Some("JP".into())
        );
        assert_eq!(country_hint("NL-FREE#5"), Some("NL".into()));
        assert_eq!(country_hint("myus-free-guide"), None);
        assert_eq!(country_hint("personal config"), None);
    }
    #[test]
    fn damaged_collection_does_not_silently_fall_back_to_legacy_country() {
        let dir = tempfile::tempdir().unwrap();
        save(
            &dir.path().join("vpn.dpapi"),
            &Profile {
                name: "JP-FREE-1".into(),
                config: config(),
            },
        )
        .unwrap();
        std::fs::write(dir.path().join("vpn-profiles.dpapi"), b"damaged").unwrap();
        assert!(load_profiles(dir.path()).is_err());
    }
    #[test]
    fn collection_rejects_ambiguous_profile_ids() {
        let dir = tempfile::tempdir().unwrap();
        let profiles = Profiles {
            profiles: (0..2)
                .map(|_| SavedProfile {
                    id: "same".into(),
                    profile: Profile {
                        name: "JP-FREE-1".into(),
                        config: config(),
                    },
                })
                .collect(),
        };
        save_profiles(dir.path(), &profiles).unwrap();
        assert!(load_profiles(dir.path()).is_err());
    }
    #[test]
    fn full_collection_rejects_new_profile_but_accepts_existing_one() {
        let mut profiles = Profiles::default();
        let mut first = String::new();
        for n in 1..=MAX_PROFILES {
            let id = profiles
                .add(Profile {
                    name: format!("JP-FREE-{n}"),
                    config: config().replace("185.159.157.1", &format!("185.159.157.{n}")),
                })
                .unwrap();
            if n == 1 {
                first = id;
            }
        }
        assert!(profiles
            .add(Profile {
                name: "US-FREE-1".into(),
                config: config().replace("185.159.157.1", "185.159.158.1")
            })
            .is_err());
        assert_eq!(
            profiles
                .add(Profile {
                    name: "same".into(),
                    config: config()
                })
                .unwrap(),
            first
        );
    }
    #[test]
    fn generated_test_config_engine_accepts_named_pipe() {
        let exe =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../.runtime/browser-tools/wireproxy.exe");
        if !exe.exists() {
            return;
        }
        let engine = Engine::start(&exe, &config()).unwrap();
        assert!(engine.port > 0);
        drop(engine);
    }
}
