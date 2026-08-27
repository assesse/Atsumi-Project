# Windows 배포와 앱 업데이트

Atsumi Next는 유료 Windows 코드 서명(Authenticode) 인증서 없이 배포한다. 따라서 새 PC에서 설치 프로그램을 직접 실행하면 Windows가 `알 수 없는 게시자` 또는 SmartScreen 안내를 표시할 수 있다. 이 안내를 숨기는 기능은 구현하지 않는다.

앱 내부 업데이트는 별도의 무료 Tauri 업데이트 키로 서명을 검증한다. 이 서명은 Windows 게시자 신원을 인증하지는 않지만, 앱이 받은 설치 파일이 프로젝트에서 만든 파일이며 전송 중 바뀌지 않았는지는 검증한다. 업데이트 서명 검증은 끌 수 없다.

## 최초 1회 준비

1. 현재 공개 키와 짝을 이루는 updater 개인 키를 저장소 밖의 안전한 위치에 보관한다. 로컬 빌드에서 `.runtime/updater-secrets/atsumi-next.key`를 사용할 수 있지만 `.runtime/`은 Git에서 제외된다.
2. 개인 키 파일을 암호화된 별도 저장소에 백업한다. 이 키를 잃으면 기존 설치본이 이후 업데이트를 받아들이지 못하므로 새 키를 임의로 생성하지 않는다.
3. 개인 키의 **내용**을 GitHub 저장소 Actions secret `TAURI_SIGNING_PRIVATE_KEY`로 등록한다. 파일 내용은 이 문서, issue, 로그, commit에 복사하지 않는다.
4. 공개 키는 `src-tauri/tauri.conf.json`에 들어 있으며 공개되어도 안전하다.

## 버전 배포 절차

1. `package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`의 버전을 같은 값으로 올린다.
2. `tools/verify.ps1 -SkipInstall`로 현재 소스를 검증한다.
3. 검토가 끝난 commit에 정확히 `v<버전>` 태그를 push하거나 GitHub Actions의 `Windows Release`를 수동 실행한다.
4. workflow는 Windows NSIS 설치 파일, Tauri 업데이트 서명과 `latest.json`을 포함한 **draft release**를 만든다.
5. draft의 파일과 설명을 확인한 뒤 GitHub에서 Publish한다. draft 상태에서는 앱의 `releases/latest/download/latest.json` 주소에 새 버전이 노출되지 않는다.

앱은 시작할 때 최신 release 정보를 확인한다. 새 버전이 있으면 사용자에게 묻고, 동의할 때만 다운로드·서명 검증·passive 설치 후 재시작한다. 거절한 사용자는 `설정 > 일반 > 프로그램 정보 > 업데이트 확인`에서 다시 확인할 수 있다. 시작 확인의 네트워크 실패는 앱 실행을 방해하지 않는다.

## 운영 경계

- GitHub release를 삭제하거나 `latest.json`만 수동 편집하지 않는다. 설치 파일, 서명, 플랫폼 URL이 함께 맞아야 한다.
- updater 개인 키를 교체하면 기존 앱은 새 키로 서명한 업데이트를 거부한다. 키 교체는 기존 키로 서명한 전환 release를 별도로 설계한 뒤 진행한다.
- 이 workflow는 Windows Authenticode 서명을 하지 않는다. 나중에 인증서를 도입하기 전까지 `TAURI_SIGNING_PRIVATE_KEY` 외의 Windows 인증서 secret은 필요 없다.
- release는 자동 publish하지 않는다. workflow가 만든 draft를 사람이 최종 확인한 뒤 공개한다.
