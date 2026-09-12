# CHZZK 로그인 보안 점검 — 2026-09-11

## 결론

Microsoft WebView2를 사용하지만, Atsumi 개발판을 Chrome과 동일하게 검증된 브라우저라고 보장할 수는 없다. 사이트의 보안뿐 아니라 호스트 앱, 앱에 주입한 스크립트, 연결한 네이버 확장·네이티브 연결 프로그램도 신뢰 범위에 포함된다. 이번 점검은 소스 검토와 제한된 회귀 검사이며 독립적인 보안 감사나 계정 침투 테스트가 아니다.

로그인이 불안하다면 당분간 주 계정은 공식 Chrome에서만 사용하고 앱은 비로그인으로 시험하는 선택이 가장 보수적이다. 앱에서 로그인하기로 결정했다면 네이버가 제공하는 QR 등 로그인 방식과 2단계 인증을 사용할 수 있다. 이들은 비밀번호 입력을 줄일 수 있어도 발급된 로그인 세션까지 앱으로부터 격리하지는 않는다.

## 현재 코드에서 확인한 보호와 처리

- 최상위 이동은 정확한 `https://chzzk.naver.com`, `https://nid.naver.com`만 허용한다. 다른 호스트·HTTP·사용자 정보가 포함된 URL은 거절한다. 설치 링크는 별도의 정확한 공식 확장 ID 목록으로 판별한다. 이것은 모든 하위 프레임·사이트 리소스의 도메인을 제한한다는 의미가 아니다.
- 로그인 폼의 비밀번호나 브라우저 쿠키를 읽어 Rust·앱 DB·원격 서버로 보내는 코드는 없다. Chrome의 기존 계정 쿠키나 프로필을 복사하지 않는다.
- 로그인 세션은 Atsumi 앱 데이터 아래 `chzzk-browser-profile`에 WebView2가 관리한다. 시청 창과 로그인 창이 같은 전용 프로필을 사용하므로 앱을 다시 열어도 로그인 상태가 남을 수 있다. 프로필은 비밀 정보로 취급해야 하며 진단 자료에 통째로 첨부하면 안 된다.
- 앱의 녹화·표시·채팅 스크립트는 CHZZK 라이브 페이지에서만 동작하도록 출처·경로를 검사한다. 로그인 전용 창에는 이 스크립트들을 등록하지 않는다.
- 원격 페이지의 Tauri 명령은 일반 명령과 내부 데이터 채널 모두 차단한다. 녹화 메시지는 별도의 출처·채널·사용자 시작 nonce·녹화 ID·크기·대기열 제한을 확인한다. 따라서 원격 페이지에 범용 파일 접근/명령 실행 API를 제공하지 않는다.
- 일반 채팅 저장은 녹화 시작 후 선택한 경우에만 진행하며 닉네임·문자·시간·표시 정보만 정규화한다. 인증 송신이나 전체 계정 프로필을 로그로 저장하지 않는다.
- 로그아웃은 사용자 확인 후 이 CHZZK 전용 프로필의 브라우징 데이터를 지운다. 다른 Chrome 계정·기존 녹화 파일을 삭제하지 않는다. 현재 로그인 상태를 API로 추출하지 않으므로 실제 계정 상태는 공식 페이지에서 확인한다.

## 남는 신뢰 경계와 한계

- 네이티브 호스트는 기술적으로 페이지 스크립트 실행·프로필 접근 기능을 사용할 수 있다. 현재 구현이 비밀번호를 추출하지 않는다는 사실과, 앱이 원천적으로 접근할 수 없다는 주장은 다르다.
- 이 PC의 공식 네이버 Chrome 확장 1.0.2.4는 `activeTab`, `nativeMessaging` 권한과 `*.naver.com`, `*.navercorp.com` 범위의 content script/host 권한을 가진다. 따라서 네이버 로그인 도메인도 확장의 접근 범위다. 공식 확장 한 개만 연결하더라도 추가 신뢰가 필요하다.
- 연결 허용 목록은 Chrome용 `ooadnieabchijkibjpeieeliohjidnjj`, Edge용 `jedbgfnhnpbfcbplibkacnmiafbojobk` 두 ID다. 위 권한 확인은 이 PC에 설치된 Chrome용 manifest에 대한 것이며 모든 미래 버전의 권한을 보증하지 않는다.
- 사용자가 연결한 선택을 비민감한 별도 설정으로 기억하면 다음 앱/시청 영역 생성 때 다시 검증하고 연결한다. 따라서 매번 버튼을 누르지 않아도 확장 권한이 활성화될 수 있다. 자동 녹화나 Chrome 계정 복사를 의미하지는 않는다.
- manifest 공개키와 확장 ID 일치 검사는 엉뚱한 확장 연결을 줄이지만, 설치된 모든 파일의 서명·변조 여부를 독립적으로 보증하는 검사는 아니다. 원본 확장 폴더와 네이티브 연결 프로그램, Windows 계정을 신뢰해야 한다.
- 전용 프로필은 Chrome과의 세션 혼합을 막는 분리이지, Atsumi나 동일 Windows 계정의 악성 프로그램으로부터 완전한 비밀 저장소가 되는 것은 아니다.
- WebView2의 새 비밀번호 자동 저장 기본값은 false지만 앱이 현재 이를 별도로 강제 설정하지 않는다. 일반 입력 자동완성 기본값은 true다. 비밀번호 저장을 꺼도 로그인 쿠키 저장과는 별개이며, 기존에 저장된 비밀번호를 자동 삭제하는 것도 아니다.
- 개발판은 로컬 개발 서버·변경 가능한 소스를 사용한다. WebView2 런타임·네이버 확장·Atsumi 코드의 변경과 로컬 환경 모두가 보안에 영향을 준다. 인증서 검증을 끄거나 브라우저 sandbox를 해제하는 제품 코드는 확인되지 않았지만, 모든 PC 정책·환경 변수·외부 프로그램까지 감사한 것은 아니다.
- 실제 계정 로그인·19세 인증·장시간 녹화는 이번 보안 점검에서 수행하지 않았다. 비밀번호·쿠키·로그인 DB도 열어보지 않았다.

## 근거

- [Microsoft: WebView2 보안 권장사항](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/security) — 웹 콘텐츠 출처 확인, 최소 권한, 네이티브 인터페이스 격리.
- [Microsoft: 사용자 데이터 폴더](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/user-data-folder) — 쿠키·권한·캐시와 프로필 공유 범위.
- [Microsoft: 자동완성·비밀번호 저장 설정](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2settings4) — 기본값과 기존 저장 자료에 대한 한계.
- 로컬 검토: `browser.rs`, `browser_host.rs`, `browser_extension.rs`, `browser_capture.js`, `browser_page_chat.js`, `browser_ipc_tests.rs`, `vendor/ATSUMI_PATCHES.md`.

## 2026-09-11 추가 검토(일반모델, DAYBREAK 미실행)

### 범위와 판정 기준

현재 사용 가능한 일반 모델이 기존 결론과 별도로 소스를 읽어 검토했다. 요청된 DAYBREAK 모델은 이 협업 도구에서 지원되지 않아 실행되지 않았다. 이 절은 DAYBREAK 결과나 외부 전문 보안 감사가 아니다.

`2849` 작업 공간의 원격 Tauri IPC, 공식 WebView 메시지, 이동·팝업, 초기 주입 스크립트, 확장 연결, 전용 로그인 프로필 및 전역 미디어 프로토콜을 확인했다. 이 추가 검토에서는 앱·실제 로그인·공격 스크립트·방송 녹화·테스트를 실행하지 않았고, 사용자 파일 내용·쿠키·비밀번호·로그인 DB·설치된 확장의 실제 파일을 열지 않았다. 아래 줄 번호는 검토 시점 소스 기준이며, 공동 작업으로 이후 바뀔 수 있다.

원격 페이지에서 일반 네이티브 명령 실행이나 임의 사용자 파일 읽기로 이어지는 우회는 이번 검토에서 확인하지 못했다. 다만 아래 두 경로는 명시적 격리를 보완할 필요가 있는 **소스상 발견 사항**이다. 실제 악용 성공 또는 모든 공격 경로 부재를 입증한 결과는 아니다.

### 발견 1 — P2: 페이지 이동이 사용자 승인 없이 외부 브라우저 실행으로 전환됨

- **근거:** `src-tauri/src/streaming/browser_host.rs:231–234, 272–275`는 허용된 확장 설치 URL의 navigation/new-window를 `install_from_page`로 전달한다. `:813–814`에서 별도 클릭 확인·일회 승인·재호출 간격 검사 없이 실행 함수로 이어지고, `:1075–1093`는 로컬 Chrome/Edge 실행 파일에 고정된 공식 설치 URL을 인자로 주어 `Command::spawn`한다.
- **악용 전제와 영향:** 공식 CHZZK 최상위 문맥에서 스크립트를 실행할 수 있는 공격자(예: 사이트 XSS 또는 사이트 스크립트 공급망 손상)가 해당 허용 URL로 이동을 반복 요청할 수 있어야 한다. 이동 자체가 취소되어도 호스트의 외부 브라우저 실행 부작용은 이미 요청된다. 설치 탭 반복 생성과 네이티브 프로세스 실행에 따른 방해·자원 소모 가능성이 있다. 허용 URL·실행 파일·인자는 제한되므로 **임의 URL 열기나 임의 명령 실행을 발견했다는 뜻은 아니다.**
- **미재현 범위:** 악성 반복 이동, 외부 브라우저 반복 실행, 실제 자원 고갈을 시험하지 않았다. 실제 프로세스·탭 개수는 Chrome/Edge의 기존 실행 상태에도 좌우된다.
- **개선안:** 외부 설치 페이지 열기는 Atsumi의 신뢰된 설치 버튼으로만 허용하거나, 페이지 요청을 사용자 확인이 필요한 상태로 바꾸고 네이티브 일회 승인 및 짧은 재호출 제한을 적용한다. URL allowlist는 그대로 유지한다. 새 창 생성 이벤트의 사용자 제스처 정보만 추가하는 경우에도 navigation 경로가 별도로 남지 않도록 함께 처리해야 한다.

### 발견 2 — P2(브라우저 도달 조건부): 전역 미디어 프로토콜에 원격 WebView 차단이 없고 대기 스레드가 제한되지 않음

- **근거:** `src-tauri/vendor/tauri-2.11.5/src/manager/webview.rs:230–242`는 등록된 사용자 프로토콜을 각 WebView에 연결한다. 사용 중인 Wry 0.55.1의 `src/webview2/mod.rs:942–946`는 지원 런타임에서 iframe·worker를 포함한 모든 요청 출처를 필터에 포함한다. 앱의 `src-tauri/src/lib.rs:509, 516, 538` 콜백은 `context.webview_label()`을 검사하지 않는다. 일반 Tauri invoke 거부와 이 프로토콜 처리는 별도 경로다.
- **특히 추가 보호가 필요한 경로:** `danbooru-media`의 토큰은 비밀 난수가 아니라 CDN URL을 base64url로 표현한 값이다(`src-tauri/src/interface/danbooru.rs:1032–1051`). `lib.rs:547–548`은 요청마다 OS 스레드를 만든 다음 `DanbooruClient::media`를 호출한다. `danbooru.rs:263–273, 620–622`의 gate는 실제 다운로드를 6개로 제한하지만, 이미 만들어져 기다리는 스레드 수에는 상한이 없다. 요청 출처·WebView label도 검사하지 않는다.
- **악용 전제와 영향:** 원격 공식 페이지 또는 하위 프레임의 공격자 스크립트가 해당 WebView에서 사용자 프로토콜 subresource 요청을 전송할 수 있어야 한다. 이 조건에서는 비밀 토큰 없이 알려진 CDN 이미지 URL로 호스트의 네트워크 작업·대기 스레드를 반복 생성할 수 있다. CORS가 응답 내용을 읽지 못하게 하더라도 그 전에 시작하는 네이티브 작업의 승인 검사는 아니다. 실제 CDN 요청은 HTTPS 및 `cdn.donmai.us`로 제한되고 응답은 이미지 형식·32 MiB 상한을 검사하므로, 이 발견을 **임의 호스트 SSRF·계정 정보 유출·임의 파일 읽기**로 확대 해석하면 안 된다.
- **미재현 범위:** 현재 CHZZK CSP 및 실제 WebView에서의 요청 전송·혼합 콘텐츠 처리·메모리 고갈은 시험하지 않았다. 소스상 처리기가 원격 WebView에도 등록되고 출처 검사 없이 작업을 만드는 사실과, 실제 브라우저에서의 악용 성공은 구분한다. 일반 Chrome의 외부 웹사이트에서 이 가상 호스트에 접속해 Atsumi를 제어할 수 있다는 주장도 아니다.
- **개선안:** 세 미디어 처리기 모두 작업 생성 전에 신뢰된 `main` WebView label만 허용하고, 원격/로그인 WebView는 403으로 종료한다. 적절한 Origin 검사도 유지하되 이미지·미디어 GET의 Origin 부재를 고려해 label 검사를 기본 경계로 삼는다. `danbooru-media`에는 요청마다 스레드를 만드는 대신 bounded 작업 큐/동시 실행 상한과 큐 포화 시 거부를 적용한다. 합성 회귀 검사는 `main` 정상 요청, `chzzk-official`·로그인 label 및 외부/누락 Origin 요청, 포화 시 새 작업이 생기지 않는 경우를 다뤄야 한다.

### 미디어 토큰 방어의 실제 범위

| 경로 | 확인한 기존 보호 | 이번 판정 |
| --- | --- | --- |
| `chzzk-stream` | `streaming/protocol.rs:4–16`은 외부 Origin을 거부하지만 Origin이 없으면 허용한다. `service.rs:107, 219, 449–469`는 세션별 임의 UUID 토큰을 만들고 현재 세션 목록에서 정확히 찾는다. `store.rs:445–487`은 해당 세션의 manifest·segment·init 파일 이름, 파일 유형 및 읽기 크기를 제한한다. | 토큰 없이 임의 녹화·경로를 읽는 경로는 확인하지 못했다. 토큰 유출이 선행된다면 Origin 없는 미디어 요청은 별도 문제이므로 main-only label 제한을 추가하는 것이 명확하다. |
| `detail-original` | `ProgressiveDetailHero.tsx:22` 및 `ProgressivePagePreview.tsx:29`는 `crypto.randomUUID()`를 사용한다. `application/detail_original.rs:307–323`은 현재 활성 요청 맵에 있고 취소되지 않은 원본만 찾고, 정규화한 파일 경로가 전용 루트 아래인지 확인한다. | 원격 페이지에 이 토큰이 노출되는 경로는 찾지 못했으며 개인 이미지 유출을 입증하지 않았다. Origin/label 검사는 없으므로 토큰 지식에만 의존하지 않도록 원격 WebView를 차단하는 방어가 적절하다. |
| `danbooru-media` | HTTPS·정확한 CDN 호스트, 사용자 정보 없는 URL, 토큰 길이·이미지 형식·32 MiB 응답 한도와 다운로드 6개 제한이 있다. | URL 인코딩은 권한 토큰이 아니다. 원격 출처 차단 및 작업 대기열 상한이 추가로 필요하다. |

제품 코드에서 별도의 TCP loopback 수신 서버는 찾지 못했다. `*.localhost` 미디어 주소는 위 Wry 사용자 프로토콜 변환이며, 개발 모드의 `127.0.0.1:1420` Vite 서버와 구분해야 한다. 계정 쿠키를 넘기는 별도 loopback 로그인 bridge가 존재한다고 확인한 것은 아니다.

### 다시 확인한 주요 신뢰 경계

- **일반 IPC와 전용 bridge:** vendored `webview/mod.rs:1744, 1776–1782`의 remote 거부는 앱 명령·플러그인 및 내부 `__TAURI_CHANNEL__|fetch` 처리보다 앞선다. `ipc/protocol.rs:185–196`의 예약 prefix 무시는 전용 메시지를 일반 IPC로 처리하지 않게 할 뿐, 네이티브 권한을 부여하지 않는다. 전용 수신기는 WebView2가 제공한 `Source`를 exact HTTPS CHZZK live URL과 대조하고, 메시지 크기·UUID·4개 대기열을 검사한다(`browser.rs:222–249, 1023–1099`). `begin`은 main-only 명령에서 만든 사용자 승인 nonce와 20초 유효기간을 요구하며 후속 쓰기는 활성 녹화 ID·채널에 묶인다(`browser.rs:373–464, 625–703, 1139` 전후). JS의 DOM 이벤트나 공개 전역 객체 자체는 권한 경계가 아니다. Microsoft도 웹 메시지의 Source와 매개변수 검사를 권고한다. [WebView2 보안 권장사항](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/security)
- **로그인 및 채팅:** watch/login은 전용 `chzzk-browser-profile`을 공유한다. 로그인 전용 builder에는 Atsumi의 표시·채팅·녹화 초기 스크립트가 없다(`browser_host.rs:224–230, 667–694`). 스크립트 자체도 exact live origin/path와 최상위 문맥을 검사한다. 채팅 수신 관측은 `begin` ACK의 `captureChat` 이후에만 저장을 시작하고, 지정된 공식 WSS·일반 실시간 채팅 명령 93101·필드 allowlist·UTF-8 크기/큐 한도를 적용한다(`browser_capture.js:365–377`, `browser_page_chat.js:5–13, 45–97, 142–210`). 로그인 송신·쿠키·전체 raw profile을 네이티브에 넘기는 구현은 확인하지 못했다. 공식 페이지/연결 확장이 악성으로 바뀌지 않았다는 보증이나 네이티브 앱이 기술적으로 프로필에 접근할 수 없다는 뜻은 아니다.
- **확장 신뢰에 대한 보정:** `browser_extension.rs:894–921`은 manifest 버전과 공개키에서 계산한 허용 ID를 검사하고, 경로·reparse·파일 수/크기를 제한하지만 확장 전체 내용의 서명을 검증하지 않는다. `:694`에서 unpacked 경로를 `AddBrowserExtension`에 넘기며 로그인 창도 확장을 활성화한다(`browser_host.rs:291, 678`). Microsoft 문서에 따르면 **설치 후 내용이 변경되면 해당 확장은 프로필에서 제거**된다. 그러므로 변경된 코드가 즉시 계속 실행된다고 단정하면 잘못이다. 남는 조건은 공격자가 연결 전에 허용 ID의 공개키를 유지한 채 원본 디렉터리를 바꾸거나 공급망이 손상된 경우이며, 이후 명시적/자동 재연결이 내용 서명 검증 없이 그 디렉터리를 다시 받아들일 수 있다는 것이다. 공개키는 비밀이 아니므로 ID 일치는 변조 방지 증명이 아니다. 이번에는 실제 설치 파일을 재검사하지 않았다. [Microsoft: AddBrowserExtension](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2profile7)
- **기본 설정과 미확인 항목:** 검토한 builder에는 PasswordAutosave/GeneralAutofill 및 PermissionRequested/DownloadStarting의 별도 강제 정책이 없었다. 이것만으로 카메라·마이크·다운로드가 자동 허용된다고 결론 내리지 않는다. 문서상 새 비밀번호 저장 기본값은 false, 일반 입력 자동완성은 true이며 기존 저장 자료와는 별개다. 의도한 제품 정책을 명시적으로 설정하고 별도 합성 테스트로 확인하는 것이 좋다. 실제 OS 권한, 런타임 정책, 기존 프로필 상태, 확장 네이티브 연결 프로그램, 로그인 인증 흐름은 이번 추가 검토 범위 밖이다. [Microsoft: Settings4](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2settings4)

위 발견 사항은 이 절 작성 시점에는 보고만 했으며 제품 코드를 변경하지 않았다. 후속 수정 시에는 해당 경로와 회귀 검사를 다시 검토해야 한다.

## QR·일회용 로그인과 세션 탈취 — 후속 검토

**QR 로그인은 비밀번호를 PC에 입력하는 위험을 줄이지만, 로그인한 세션을 보호하는 만능 수단은 아니다.** QR 인증이 성공하면 해당 WebView 프로필에 로그인 세션이 생긴다. 앱/확장/동일 Windows 계정의 악성 프로그램이 이미 신뢰 경계를 넘었다면 QR만으로 그 세션의 사용·탈취를 차단하지 못한다. 네이버 도움말은 일회용 번호 로그인 종료와 QR 전환을 안내하고 있으므로 일회용 번호를 장기적인 별도 보안 대안으로 설계하지 않는다. [네이버 QR 안내](https://help.naver.com/service/5640/contents/19014?lang=ko&osType=COMMONOS), [일회용 번호 안내](https://help.naver.com/service/5640/contents/1546?lang=ko&osType=COMMONOS)

현재 코드 점검에서 쿠키를 추출·전송하는 경로는 찾지 못했다. 그러나 WebView2 호스트에는 쿠키 관리 API가 있고 전용 프로필도 디스크에 존재한다. 따라서 '현재 추출하지 않는다'와 '기술적으로 탈취할 수 없다'는 다른 주장이다. `HttpOnly`는 페이지 JavaScript의 직접 읽기를 제한하고 `Secure`는 전송 방식, `SameSite`는 교차 사이트 요청을 제한한다. 이 속성들이 신뢰된 네이티브 호스트나 같은 출처의 인증 요청까지 무력화하는 것은 아니다. [Microsoft 쿠키 관리 API](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2cookiemanager), [쿠키 속성](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Set-Cookie)

앱의 로그아웃은 전용 프로필의 로컬 데이터를 지운다. 이미 탈취된 복사본까지 서버에서 무효화했다고 보장할 수 없다. 의심될 때는 신뢰할 수 있는 기기에서 네이버 로그인 기기 관리로 해당 세션을 로그아웃하고 계정 보안 상태를 점검해야 한다. [네이버 로그인 기기 관리](https://help.naver.com/service/5640/contents/19022?lang=ko&osType=COMMONOS)

앞서 보고한 설치 페이지 반복 실행 가능성과 미디어 프로토콜 자원 소모 가능성 두 P2는 QR 로그인으로 해결되지 않는다. 이번 후속 작업에서도 이 두 경로는 별도 보안 보완 대상으로 남는다. 이들이 곧 쿠키 탈취 성공을 뜻하지는 않는다. 실제 계정·쿠키·로그인 DB·설치 확장 파일을 이번 후속 검증에서 읽지 않았다.
