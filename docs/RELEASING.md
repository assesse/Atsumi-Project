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
4. workflow는 서명을 검증한 `Atsumi-Portable.zip`(권장), `Atsumi-Setup.exe`, `latest.json`을 **draft release**에 올린다. 별도의 `.sig`와 MSI는 업로드하지 않는다. 이미 공개된 같은 버전은 덮어쓰지 않는다.
5. draft의 파일과 설명을 확인한 뒤 GitHub에서 Publish한다. draft 상태에서는 앱의 `releases/latest/download/latest.json` 주소에 새 버전이 노출되지 않는다.

앱은 시작할 때 최신 release 정보를 확인한다. 새 버전이 있으면 사용자에게 묻고, 동의할 때만 다운로드·서명 검증·passive 설치 후 재시작한다. 거절한 사용자는 `설정 > 일반 > 프로그램 정보 > 업데이트 확인`에서 다시 확인할 수 있다. 시작 확인의 네트워크 실패는 앱 실행을 방해하지 않는다.

## 무설치 업데이트 (2.1.1부터)

- 포터블과 무설치는 같은 배포 방식이다. `atsumi-portable.json`으로 식별하고 `windows-x86_64-portable` 대상으로 ZIP만 받는다. 설치형은 기존 `windows-x86_64`/`windows-x86_64-nsis` 경로를 유지한다. 개발 앱에는 릴리즈 업데이트를 제안하지 않는다.
- 다운로드·중복 검사·Auto Find·녹화가 진행 중이면 적용을 보류한다. 예약 이후 새 작업 시작도 막는다.
- ZIP 서명을 다운로드 후 및 독립 교체 도우미에서 재검증한다. 압축 경로·링크·용량 제한을 검사한 다음 정상 종료를 기다린다. 도우미는 앱을 강제 종료하지 않는다.
- 프로그램이 설치된 쓰기 가능한 로컬 폴더에서 `atsumi.exe`, `media-tools`, 실행 스크립트·배포 안내·표시 파일만 교체한다. AppData, 다운로드, 녹화 및 선택적으로 제공된 `WebView2/`는 변경하지 않는다. 정션·심볼릭 링크 경로는 거부한다.
- 교체 실패 시 이전 프로그램 파일을 되돌린다. `.atsumi-update/<id>/backup`과 `result.txt`를 남겨 정전 등 비정상 중단 시에도 수동 복구할 수 있다. 이 백업은 자동 삭제하지 않는다.
- **2.1.0 이하의 무설치 앱에는 이 기능이 없으므로 최초 한 번은 새 ZIP을 새 폴더에 풀어 실행해야 한다.** 그 이후부터 앱 내부 업데이트를 지원한다.
- 로컬 배포: `tauri build --bundles nsis` → `tools/prepare_portable.ps1` → `tauri signer sign .runtime/portable-v<version>/Atsumi-Portable.zip` → `node tools/prepare_release.mjs`. 같은 기존 개인 키를 환경 변수로 제공하되 출력하지 않는다.

## 운영 경계

- GitHub release를 삭제하거나 `latest.json`만 수동 편집하지 않는다. 설치 파일, 서명, 플랫폼 URL이 함께 맞아야 한다.
- updater 개인 키를 교체하면 기존 앱은 새 키로 서명한 업데이트를 거부한다. 키 교체는 기존 키로 서명한 전환 release를 별도로 설계한 뒤 진행한다.
- 이 workflow는 Windows Authenticode 서명을 하지 않는다. 나중에 인증서를 도입하기 전까지 `TAURI_SIGNING_PRIVATE_KEY` 외의 Windows 인증서 secret은 필요 없다.
- release는 자동 publish하지 않는다. workflow가 만든 draft를 사람이 최종 확인한 뒤 공개한다.
