# Atsumi Next

Atsumi Next는 Windows용 Hitomi 갤러리 탐색·다운로드·라이브러리 관리 앱입니다. 검색, 즐겨찾기 작가 자동 탐색, 다운로드 재개, 작품 간 판본 비교, 앨범 내부 중복 페이지 검토와 복구 가능한 격리를 하나의 로컬 라이브러리에서 제공합니다.

현재 공개 버전은 **1.0.0**입니다.

## 주요 기능

- Hitomi 검색, 언어·정렬 필터와 구조화 태그 검색
- 작가·그룹·시리즈·캐릭터·태그 즐겨찾기
- 즐겨찾기 작가 기반 Auto Find
- 검증 checkpoint를 사용하는 다운로드 재개와 복구
- 다운로드 직전 기존 판본과 신규 판본 비교
- 작품 간 중복 및 앨범 내부 중복 페이지 검토
- 파일을 즉시 삭제하지 않는 quarantine·undo
- 시작 시 업데이트 확인과 설정의 수동 업데이트 확인

앨범·다운로드·사용자 판정은 로컬 SQLite와 manifest에 저장됩니다. 앱은 판정만으로 사용자 파일을 자동 영구 삭제하지 않습니다.

## 실행

탐색기에서 `start-app.vbs`를 실행하면 현재 소스로 만든 Windows 앱이 열립니다. 첫 실행 또는 소스 변경 뒤에는 release 실행 파일을 먼저 빌드하므로 시간이 걸릴 수 있습니다.

직접 빌드하려면 Node.js 24, pnpm 11.16.0, Rust stable, MSVC C++ Build Tools와 Windows SDK가 필요합니다.

```powershell
pnpm install --frozen-lockfile
./tools/verify.ps1 -SkipInstall
pnpm tauri dev
```

설치 파일을 만드는 배포 절차는 [docs/RELEASING.md](docs/RELEASING.md)를 참고하세요.

## 프로젝트 구성

- `src/`: React 사용자 인터페이스와 브라우저 fixture 테스트
- `src-tauri/`: Rust application/domain/infrastructure와 Tauri Windows 앱
- `shared/`: 프런트엔드와 백엔드가 공유하는 정적 설정
- `tools/`: 빌드·검증·실행 도구
- `.github/workflows/`: Windows CI와 draft release 자동화

## 안전 원칙

- manifest와 파일 검증 전에는 다운로드를 완료로 표시하지 않습니다.
- 모호하거나 손상된 파일은 덮어쓰거나 자동 삭제하지 않습니다.
- quarantine과 내부 페이지 격리는 복원 이력을 유지합니다.
- updater 개인 키, 로컬 DB, 다운로드 파일과 검증 로그는 Git에 포함하지 않습니다.
- Windows Authenticode 인증서는 사용하지 않지만 앱 내부 업데이트 파일은 전용 updater 키로 검증합니다.

## 피드백

버그와 기능 제안은 [GitHub Issues](https://github.com/assesse/Atsumi-Project/issues)에서 받습니다.

외부 라이브러리와 포함 자산의 고지는 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)에 있습니다.
