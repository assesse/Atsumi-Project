# Windows 배포와 앱 업데이트

Atsumi는 유료 Windows 코드 서명(Authenticode) 인증서 없이 배포한다. 따라서 새 PC에서 설치 프로그램을 직접 실행하면 Windows가 `알 수 없는 게시자` 또는 SmartScreen 안내를 표시할 수 있다. 이 안내를 숨기는 기능은 구현하지 않는다.

앱 내부 업데이트는 별도의 무료 Tauri 업데이트 키로 서명을 검증한다. 이 서명은 Windows 게시자 신원을 인증하지는 않지만, 앱이 받은 설치 파일이 프로젝트에서 만든 파일이며 전송 중 바뀌지 않았는지는 검증한다. 업데이트 서명 검증은 끌 수 없다.

## 최초 1회 준비

1. 현재 공개 키와 짝을 이루는 updater 개인 키를 저장소 밖의 안전한 위치에 보관한다. 로컬 빌드에서 `.runtime/updater-secrets/atsumi.key`를 사용할 수 있지만 `.runtime/`은 Git에서 제외된다.
2. 개인 키 파일을 암호화된 별도 저장소에 백업한다. 이 키를 잃으면 기존 설치본이 이후 업데이트를 받아들이지 못하므로 새 키를 임의로 생성하지 않는다.
3. 개인 키의 **내용**을 GitHub 저장소 Actions secret `TAURI_SIGNING_PRIVATE_KEY`로 등록한다. 파일 내용은 이 문서, issue, 로그, commit에 복사하지 않는다.
4. 공개 키는 `src-tauri/tauri.conf.json`에 들어 있으며 공개되어도 안전하다.

## 버전 배포 절차

1. `package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`의 버전을 같은 값으로 올린다.
2. `tools/verify.ps1 -SkipInstall`로 현재 소스를 검증한다.
3. 검토가 끝난 commit에 정확히 `v<버전>` 태그를 push하거나 GitHub Actions의 `Windows Release`를 수동 실행한다.
4. workflow는 성공 빌드 기록과 서명을 검증한 `Atsumi-Portable.zip`(권장), `Atsumi-Setup.exe`, `latest.json`, `build-receipt.json`을 **draft release**에 올린다. 별도의 `.sig`와 MSI는 업로드하지 않는다. 이미 공개된 같은 버전은 덮어쓰지 않는다.
5. draft의 파일과 설명을 확인한 뒤 GitHub에서 Publish한다. draft 상태에서는 앱의 `releases/latest/download/latest.json` 주소에 새 버전이 노출되지 않는다.

앱은 시작할 때 최신 release 정보를 확인한다. 새 버전이 있으면 사용자에게 묻고, 동의할 때만 다운로드·서명 검증·passive 설치 후 재시작한다. 거절한 사용자는 `설정 > 일반 > 프로그램 정보 > 업데이트 확인`에서 다시 확인할 수 있다. 시작 확인의 네트워크 실패는 앱 실행을 방해하지 않는다.

## 무설치 업데이트 (2.1.1부터)

- 포터블과 무설치는 같은 배포 방식이다. `atsumi-portable.json`으로 식별하고 `windows-x86_64-portable` 대상으로 ZIP만 받는다. 설치형은 기존 `windows-x86_64`/`windows-x86_64-nsis` 경로를 유지한다. 개발 앱에는 릴리즈 업데이트를 제안하지 않는다.
- 다운로드·중복 검사·Auto Find·녹화가 진행 중이면 적용을 보류한다. 예약 이후 새 작업 시작도 막는다.
- ZIP 서명을 다운로드 후 및 독립 교체 도우미에서 재검증한다. 압축 경로·링크·용량 제한을 검사한 다음 정상 종료를 기다린다. 도우미는 앱을 강제 종료하지 않는다.
- 프로그램이 설치된 쓰기 가능한 로컬 폴더에서 `atsumi.exe`, `media-tools`, 실행 스크립트·배포 안내·표시 파일만 교체한다. AppData, 다운로드, 녹화 및 선택적으로 제공된 `WebView2/`는 변경하지 않는다. 정션·심볼릭 링크 경로는 거부한다.
- 교체 실패 시 이전 프로그램 파일을 되돌린다. `.atsumi-update/<id>/backup`과 `result.txt`를 남겨 정전 등 비정상 중단 시에도 수동 복구할 수 있다. 이 백업은 자동 삭제하지 않는다.
- **2.1.0 이하의 무설치 앱에는 이 기능이 없으므로 최초 한 번은 새 ZIP을 새 폴더에 풀어 실행해야 한다.** 그 이후부터 앱 내부 업데이트를 지원한다.
- 로컬 배포는 아래 성공 빌드 기록을 반드시 사용한다. 기존 `target/release/atsumi.exe`를 직접 포장하는 경로는 지원하지 않는다. 같은 기존 개인 키를 환경 변수로 제공하되 출력하지 않는다.

## 성공 빌드 기록과 포장

배포 권한을 확인한 뒤, 준비된 의존성과 고정 도구가 있는 Windows 작업 폴더에서 다음 순서로 진행한다.

```powershell
./tools/prepare_media_tools.ps1
./tools/prepare_platform_tools.ps1
$receipt = ./tools/build_release.ps1
./tools/prepare_portable.ps1 -BuildReceipt $receipt
$version = (Get-Content package.json -Raw | ConvertFrom-Json).version
pnpm exec tauri signer sign --password= ".runtime/portable-v$version/Atsumi-Portable.zip"
node tools/prepare_release.mjs $receipt
```

- 빌드마다 새 `.runtime/release-builds/<UUID>/target`을 사용한다. 빌드 종료 코드가 0이고 소스·설정·잠금 파일·공개 자산·vendor·도구 리소스의 전후 전체 목록과 해시가 같아야 성공 기록을 만든다. 빌드 중 다른 개발 작업이 파일을 바꾸면 실패하므로, 안정된 소스에서 다시 빌드한다.
- 성공 결과는 새 `.runtime/release-inputs/<UUID>`에 보존한다. Cargo의 정확한 실행 파일/`deps/atsumi.exe` 하드링크 쌍만 빌드 소유 경로에서 확인해 새 단일링크 파일로 복사한다. 일반 소스·도구·포장 입력의 하드링크 금지는 유지한다. 기존 Cargo 출력이나 이전 배포물을 삭제·수정하지 않는다.
- `build-receipt.json`에는 버전·대상·빌드 인자·제한된 컴파일러 버전 정보·입력/출력 해시만 들어간다. 사용자 프로필, 임의 `.runtime`, 미디어, 환경 변수 전체, 개인 키는 넣지 않는다. 빌드 환경에서 SDK 버전이 제공되지 않으면 미확인으로 기록하며 추정하지 않는다.
- 포터블 준비와 릴리즈 준비는 명시한 동일 기록을 검증한다. 누락된 기록, 현재 소스와 불일치, 같은 버전의 다른 실행 파일, 변조된 캡처 파일/포터블 연결은 거부한다. 검증 단계에서 도구를 다운로드하거나 손상된 파일을 복구하지 않는다.
- 포장이 실패한 부분 결과와 이전 출력은 남긴다. 새 출력 경로를 사용하며, 자동 정리·덮어쓰기를 하지 않는다. 빌드 기록은 신뢰할 수 없는 빌드 호스트에 대한 외부 보안 인증이나 모든 의존성 라이선스 충족 증명이 아니다.
- WebView2 포함 초기 설치용 ZIP은 `Atsumi-Portable-With-WebView2.zip`으로 별도 구분하며 updater 대상에 넣지 않는다. PKG02/04의 정확한 도구 소스·라이선스/고지와 CHZZK 원본 자산 권리 검토는 별도 미완료 항목이다. 기록/서명이 통과해도 이 검토와 실제 빈 PC 검증 없이 배포 준비가 모두 끝났다고 판단하지 않는다.

## 운영 경계

- GitHub release를 삭제하거나 `latest.json`만 수동 편집하지 않는다. 설치 파일, 서명, 플랫폼 URL이 함께 맞아야 한다.
- updater 개인 키를 교체하면 기존 앱은 새 키로 서명한 업데이트를 거부한다. 키 교체는 기존 키로 서명한 전환 release를 별도로 설계한 뒤 진행한다.
- 이 workflow는 Windows Authenticode 서명을 하지 않는다. 나중에 인증서를 도입하기 전까지 `TAURI_SIGNING_PRIVATE_KEY` 외의 Windows 인증서 secret은 필요 없다.
- release는 자동 publish하지 않는다. workflow가 만든 draft를 사람이 최종 확인한 뒤 공개한다.
