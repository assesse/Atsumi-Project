# Atsumi Community v1

## 이용 방식

- 좌측 **커뮤니티**에서 최신 후기 또는 사이트·작품번호로 후기를 찾는다. 읽기만 할 때는 개인 계정·키를 만들거나 요구하지 않는다.
- **후기 작성 / 수정**을 처음 누를 때 Supabase 익명 작성자를 발급한다. 이메일, 비밀번호, 기기 지문은 필요 없다.
- Hitomi **FLOATING DETAIL**의 다운로드 왼쪽과 **PAGE PREVIEW** 우측 상단의 말풍선 **후기 남기기**를 누르면 해당 작품번호의 작성 화면으로 바로 이동한다. 내 기존 후기가 있으면 수정 상태로 불러온다. 이 버튼도 명시적인 첫 작성 시도로 취급하며, 버튼을 표시하는 것만으로 키를 발급하지 않는다. 커뮤니티에서 Explore 등으로 돌아오면 기존 탐색 결과와 상세 탭을 유지한다.
- Hitomi / Danbooru의 작품마다 작성자별 후기 한 개. 별점 1~5, 추천 여부, 500자 이하 후기, 2~24자 닉네임을 지원한다. 별점만 등록해도 된다.
- 수정·삭제 대상은 서버가 인증된 작성자로 결정한다. 다른 사람의 후기 카드에서도 버튼은 **내 후기 작성 / 수정**으로 표시한다.
- 이미 발급한 키가 있으면 신고할 수 있다. 신고는 운영자의 비공개 검토 대상으로 저장하며, 신고만으로 자동 삭제하지 않는다.
- 앱 초기화·캐시 초기화는 키나 서버 후기를 삭제하지 않는다. 닉네임을 바꾸면 이전 후기에 표시되는 닉네임도 바뀐다.

## 키 보관과 한계

Windows `%LOCALAPPDATA%\local.atsumi.next\community-identity\yfpgshvflnawmrimyfzo.v1.dpapi`에 사용자 범위 DPAPI로 암호화한다. 라이브러리 SQLite와 설정 초기화가 사용하는 Roaming 경로에서 분리했다. 파일 교체는 같은 디렉터리에서 원자적으로 수행하며, 파일 잠금으로 중복 발급·동시 토큰 갱신을 막는다.

- 토큰은 Rust 안에서만 처리한다. 웹뷰 IPC·JS·localStorage에 전달하지 않는다.
- 유효한 키를 재사용하고 만료 직전에 refresh token으로 갱신한다. 갱신된 키를 저장하기 전에는 후기를 전송하지 않는다.
- 최초 발급 전에 `Issuing` 상태를 먼저 기록한다. 저장 불가·손상·서버 응답 불명확·중간 종료는 **새 작성자를 무한 재발급하지 않고 중단**한다.
- `Issuing`에 머문 경우 문의가 필요하다. 특히 서버 응답 수신 후 디스크 저장 실패가 생기면 앱이 기존 작성자 권한을 안전하게 복구할 수 없을 수 있다. 상태 파일을 임의 삭제하는 해결책을 안내하지 않는다.
- Windows 재설치/사용자 계정 변경/보관 파일 직접 삭제에는 자동 복구가 없다. 현재 기기 이전·키 내보내기·계정 연결 UI는 구현하지 않았다.
- 앱 브라우저 미리보기는 공개 열람만 지원한다. 안전한 Windows 보관 기능이 없으면 임시 브라우저 작성자를 발급하지 않는다.

익명은 서버에 접속 흔적도 없다는 뜻이 아니다. Supabase가 운영에 필요한 IP 등의 접속 정보를 처리할 수 있다. 닉네임은 중복 가능하며, 한 사람당 한 계정을 강제하지 않는다.

## 공개 범위 / 보안

공개: 사이트, 작품번호, 닉네임, 별점, 추천, 후기, 작성/수정 시각. 앱은 이미지·다운로드 내역·즐겨찾기·파일 경로·하드웨어 ID를 업로드하지 않는다.

`atsumi_community` 스키마는 Data API에 노출하지 않는다. 모든 테이블은 RLS 활성화 + 클라이언트 직접 접근 금지. 공개 `community_v1_*` RPC만 제한적으로 허용한다. SECURITY DEFINER의 소유자 권한은 RLS를 우회할 수 있으므로 **함수 안에서 auth.uid(), 활성 작성자, 소유권을 직접 검사**한다. 모든 함수는 빈 search_path와 정규화된 테이블 이름을 사용하고 기본 PUBLIC 실행 권한을 회수한다.

- 공개 읽기: feed / reviews / summaries. 한 페이지 20개, keyset cursor, 요약 최대 100작품.
- 익명 인증 후 쓰기: profile / save_review / delete_review / report.
- 작성자별 3초 간격, 서버 일자 기준 하루 100회 쓰기. DB 행 잠금으로 병렬 요청도 제한한다.
- 후기/집계 변경은 동일 트랜잭션. 숨김 후기는 목록·집계에서 빠지며 작성자가 수정해도 숨김 상태를 유지한다.
- 공개 앱 키는 소스에 포함되지만 비밀 키가 아니다. service_role / secret key / DB 비밀번호는 앱에 넣지 않았다.
- 전역 CSP는 확장하지 않았다. 실제 앱의 네트워크는 고정 Supabase 주소로만 연결하는 Rust 어댑터에서 수행한다. 신뢰할 수 있는 main 창만 명령을 호출할 수 있다.

Supabase 기본 익명 가입 IP 제한(공식 문서 기준 30회/시간)을 유지했다. CAPTCHA/Turnstile는 추가하지 않았다. **공개 배포 전 봇 방지와 운영 정책을 추가하는 것을 권장**한다. 계정별 쓰기 제한은 계정을 여러 개 만드는 공격을 막지 못한다. 무료 프로젝트 용량·휴면 여부는 운영자가 확인해야 한다. 후기가 있는 auth.users를 정리하면 CASCADE로 후기도 지워질 수 있으므로 익명 사용자 일괄 삭제를 하지 않는다.

운영자는 Supabase SQL Editor에서 `atsumi_community.reports`를 확인하고, 대상 리뷰의 `hidden=true` 또는 작성자의 `enabled=false`를 명시적으로 적용한다. 신고 검토 대시보드·자동 필터·사용자 전체 탈퇴 UI는 후속 범위다. 기존 후기 공개 중단은 해당 리뷰의 hidden을 별도로 적용한다.

## 확장 경계

- UI: `src/features/community/CommunityWorkspace.tsx`
- 기능 계약: `src/features/community/api.ts`의 CommunityApi / WorkKey. UI는 Supabase SDK를 직접 사용하지 않는다.
- Windows 네트워크/익명 인증: `src-tauri/src/community/mod.rs`
- 키 보관: `src-tauri/src/community/vault.rs`
- 공통 메뉴: `CommonNavigationContext`. 커뮤니티는 새 콘텐츠 소스가 아니며 기존 Hitomi·CHZZK 작업의 수명과 분리한다.
- 서버: `supabase/migrations/202609140001_community.sql`

서버를 이전할 때 CommunityApi 계약을 유지하고 네트워크·인증 어댑터와 데이터 마이그레이션을 교체한다. 커뮤니티의 member UUID는 Supabase Auth UUID와 별도다. 인증 제공자 이전에는 기존 작성자 권한을 새 인증으로 연결하는 이관 절차가 필요하며, DB만 복사하면 끝나는 것은 아니다.

## 배포 / 검증 기록

2026-09-15 KST: Free 프로젝트 `yfpgshvflnawmrimyfzo`에 SQL Editor로 최초 적용하고 익명 인증을 활성화했다. DB 비밀번호나 관리자 키는 취급하지 않았다. SQL Editor로 적용했으므로 Supabase CLI의 migration history를 자동 기록하지 않았다. 향후 CLI를 연결하면 실제 스키마와 파일을 대조하고 먼저 migration repair로 기준 버전을 맞춘 뒤 db push를 사용해야 한다.

`supabase/tests/community_security.sql`은 실제 PostgreSQL에서 공개 읽기, 익명 쓰기, 직접 테이블 접근 거부, 타 작성자 삭제 거부, 숨김 상태, 증분 집계, pagination, 서버 검증·rate limit를 점검하고 모두 ROLLBACK한다. 운영 데이터에 영향을 주지 않도록 합성 UUID/작품번호를 사용한다. 대규모 실사용 데이터가 있는 DB에서는 별도 테스트 프로젝트에서 실행한다.

개인 작성자 키는 실제로 발급하지 않고 Auth 설정/공개 API와 롤백형 DB 테스트로 검증했다. Windows 키 발급·재사용·갱신 흐름은 가짜 Auth 응답과 실제 DPAPI 임시 파일 테스트로 검증한다.

이번 검증 결과:

- 현재 `src` 단위테스트 107파일 / 1,070개 통과. `--dir src`로 검증용 복사본을 제외하고, 별도 Edge 실행이 필요한 3개 테스트 파일(`*.layout.test.tsx`, `browserPlayerGeometry.test.ts`)은 제외했다.
- 관련 App/설정/메뉴/소스 분리 테스트 144개 통과, 새 커뮤니티 단위·통합 테스트 9개 통과(전체 1,070개에도 포함).
- Rust 커뮤니티 테스트 9개 통과. Windows DPAPI 테스트는 일반 Windows 권한에서 임시 파일로 실행했다. 샌드박스에서는 사용자 암호화 기능이 거부된다.
- 앱/도구 TypeScript 검사, 프런트엔드 빌드, 개발 실행 파일 빌드, 추가 Rust 파일 포맷 검사 통과.
- 합성 후기 화면으로 1280×820 / 960×640 배치와 등록 후 목록 갱신을 브라우저에서 확인했다. 실제 후기나 작성자 키는 생성하지 않았다.
- 필터 없는 전체 저장소 테스트는 `.runtime`·`checkpoints`의 과거 복사본과 CHZZK 오프라인 플레이어 증명 테스트까지 수집했고, 이 실행은 통과하지 않았다(206파일 중 9파일 실패). 별도 Edge 테스트에서는 샌드박스/GPU 실행 오류도 발생했다. 이 작업에서 해당 CHZZK 코드·백업 또는 전역 테스트 설정을 수정하지 않았다.
- 바탕화면 `Atsumi.lnk`는 이 워크스페이스의 `tools/start_debug_app_hidden.ps1`에 연결된 것을 읽기 전용으로 확인했다. 앱을 자동 실행하거나 사용자 라이브러리 DB를 초기화하지 않았다.

참고: [익명 인증](https://supabase.com/docs/guides/auth/auth-anonymous), [API 키](https://supabase.com/docs/guides/getting-started/api-keys), [Windows DPAPI](https://learn.microsoft.com/en-us/windows/win32/api/dpapi/nf-dpapi-cryptprotectdata).
