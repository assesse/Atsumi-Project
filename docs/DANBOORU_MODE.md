# Danbooru 모드 명세

## 제품 경계

Danbooru는 앨범 중심인 Hitomi와 달리 post(이미지·영상) 중심이다. 두 소스의 숫자 ID는
겹칠 수 있으므로 기존 `GalleryId`, 다운로드 대기열, 중복 판정 DB에 Danbooru post를
끼워 넣지 않는다. 좌측 상단 Atsumi 배너에서 소스를 전환하고, 각 소스의 탐색 상태와
다운로드 기록은 독립적으로 유지한다.

## 첫 실사용 범위

- Atsumi 배너를 누르면 `Hitomi library`와 `Danbooru posts`를 선택할 수 있다.
- 마지막으로 선택한 소스는 이 PC에 저장하고 다음 실행 때 복원한다.
- Danbooru Explore는 최신 post, 태그 검색, 숫자 post ID 검색, 자동완성, 이전/다음
  페이지, 랜덤 post 열기를 지원한다.
- 카드와 상세 화면에는 rating, 크기, 점수, 즐겨찾기 수, 작가·작품·캐릭터 태그를
  구분해 표시한다.
- 상세 화면에서 원본 파일을 Atsumi 다운로드 루트의 `Danbooru` 폴더에 저장한다.
  저장 중 임시 파일을 사용하고, 완료 전에 응답 크기와 Danbooru MD5를 확인한다.
- Danbooru Downloads는 로컬 인덱스를 먼저 읽어 최신 저장 순으로 표시한다. 원본과
  sidecar가 남아 있어 인덱스가 없어져도 다시 구성할 수 있다.
- Hitomi 화면을 떠나도 현재 검색 탭과 스크롤을 가진 React 상태는 유지한다.

## API와 안전 경계

- API 기준 주소는 `https://danbooru.donmai.us`로 고정한다.
- 미디어는 HTTPS와 `cdn.donmai.us` 호스트만 허용한다.
- API 요청은 식별 가능한 Atsumi User-Agent, 30초 timeout, 직렬 시작 간격을 사용한다.
- 공개 API의 현재 비로그인 제한에 맞춰 검색 태그는 최대 2개다. 서버가 제한을
  변경하거나 429를 반환하면 안정된 한국어 오류로 변환한다.
- 다운로드는 512 MiB 상한과 Danbooru가 제공한 파일 크기·MD5를 검증한다.
- 원격 오류 본문, 사용자 경로, 전체 URL은 공개 오류에 포함하지 않는다.

## 의도적으로 분리한 후속 범위

- Danbooru 계정/API key, 투표·사이트 즐겨찾기 쓰기 작업
- pool을 앨범처럼 묶어 받는 기능
- Danbooru post와 Hitomi 앨범 사이의 이미지 중복 분석
- 영상 재생 최적화와 대용량 background 다운로드 대기열

이 항목들은 source-aware 영속 ID와 자격 증명 저장 정책을 먼저 확정한 뒤 확장한다.
현재 모드는 계정 없이 검색·검토·원본 보관이 가능한 읽기 중심 경계를 완성한다.
