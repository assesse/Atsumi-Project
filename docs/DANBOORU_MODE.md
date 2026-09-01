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
- 이미지 post는 large/sample을 사용하고, MP4·WebM·ugoira는 `media_asset.variants`
  중 가장 큰 정적 이미지 poster를 선택한다. 정적 변형이 없는 경우에만 180px
  preview로 대체한다. 목록에서는 poster만 표시하고 MP4·WebM은 플로팅 상세에서
  원본을 스트리밍 재생한다.
- 상세 화면에서 원본 파일을 Atsumi 다운로드 루트의 `Danbooru` 폴더에 저장한다.
  저장 중 임시 파일을 사용하고, 완료 전에 응답 크기와 Danbooru MD5를 확인한다.
- Danbooru Downloads는 로컬 인덱스를 먼저 읽어 최신 저장 순으로 표시한다. 원본과
  sidecar가 남아 있어 인덱스가 없어져도 다시 구성할 수 있다.
- Hitomi 화면을 떠나도 현재 검색 탭과 스크롤을 가진 React 상태는 유지한다.

## API와 안전 경계

- API 기준 주소는 `https://danbooru.donmai.us`로 고정한다.
- 미디어는 HTTPS와 `cdn.donmai.us` 호스트만 허용한다.
- API 요청은 식별 가능한 Atsumi User-Agent, 30초 timeout, 직렬 시작 간격을 사용한다.
- 공개 API의 현재 비로그인·무료 Member 제한에 맞춰 제한 대상 검색 조건은 최대
  2개다. 일반 태그와 `order:`는 이 수에 포함되지만 `rating:`, `date:`, `score:`,
  `filetype:`, `parent:`, `child:` 등은 포함되지 않는다. 서버가 제한을 변경하거나
  429를 반환하면 안정된 한국어 오류로 변환한다.
- 다운로드는 512 MiB 상한과 Danbooru가 제공한 파일 크기·MD5를 검증한다.
- 원격 오류 본문, 사용자 경로, 전체 URL은 공개 오류에 포함하지 않는다.

## 의도적으로 분리한 후속 범위

- Danbooru 계정/API key, 투표·사이트 즐겨찾기 쓰기 작업
- pool을 앨범처럼 묶어 받는 기능
- Danbooru post와 Hitomi 앨범 사이의 이미지 중복 분석
- 영상 재생 최적화와 대용량 background 다운로드 대기열

이 항목들은 source-aware 영속 ID와 자격 증명 저장 정책을 먼저 확정한 뒤 확장한다.
현재 모드는 계정 없이 검색·검토·원본 보관이 가능한 읽기 중심 경계를 완성한다.

## 설정과 기능 소유권

| 분류 | 소유하는 설정과 기능 |
| --- | --- |
| 일반 | 다운로드 루트, 저장공간 현황, 프라이버시 모드, 업데이트·초기화·앱 정보 |
| Hitomi | 페이지당 앨범 수, 카드 크기, 앨범 폴더 템플릿, Auto Find 기록, 판본 자동 판정, 앨범 열 수, Related galleries, Hitomi 요청 속도, 태그 카탈로그·전역 검색 규칙·탐색 제외 복원·라이브러리 유지보수 |
| Danbooru | 페이지당 post 수, 카드 크기, 기본 rating, 기본 파일 형식, 기본 정렬, post 검색 메타데이터 안내, large/sample 카드 품질 정책 |

일반 설정은 두 소스가 같은 값과 동작을 사용한다. 프라이버시 모드는 두 소스의 카드와
플로팅 상세 이미지를 모두 가린다. 페이지당 로드 수와 카드 미리보기 크기는 소스별로
독립 저장하며, 현재 열 수의 배수로 요청량을 조정해 마지막 행을 가능한 한 채운다.
Hitomi 또는 Danbooru 탭의 설정은 다른 소스의 검색 상태나 다운로드 판정에는 영향을
주지 않는다.

## 통합·템플릿 공유·분리 원칙

### 실제로 통합하는 기능

- Atsumi 배너와 좌측 rail, Explore/Downloads 전환, 랜덤 열기, 설정 진입
- 페이지당 로드 수, 카드 미리보기 크기, 다운로드 루트·디스크 현황, 프라이버시 모드
- 카드를 누르면 하단 sheet가 아닌 중앙 플로팅 상세를 여는 상호작용
- 다운로드 완료 배지와 로컬 목록을 우선 보여 주는 탐색 흐름

### UI 템플릿과 관례만 공유하는 기능

- 카드 표면·상태 배지·스켈레톤·페이지 이동의 시각 언어
- 플로팅 상세의 header/body/footer 구조와 닫기 동작
- 검색 자동완성, 조건 패널, 정렬 선택기의 입력 관례

같은 템플릿을 쓰더라도 Hitomi 카드는 앨범, Danbooru 카드는 post를 표현하므로 정보
밀도와 동작을 억지로 동일하게 만들지 않는다.

### 완전히 분리하는 기능

- Hitomi 앨범 ID/manifest/페이지와 Danbooru post ID/file/sidecar 저장 모델
- Hitomi Auto Find·판본 중복 판정·격리와 Danbooru parent/child·pool 관계
- Hitomi 여성/남성 태그 문법과 Danbooru category/rating/metatag 문법
- 향후 Danbooru 계정 자격 증명, 투표·즐겨찾기 쓰기, pool 일괄 저장

소스 간 ID 충돌과 잘못된 중복 제거를 막기 위해 Danbooru post를 기존 Hitomi
`GalleryId`, 다운로드 supervisor, 판본 중복 DB에 넣지 않는다.

## Danbooru 메타데이터 검색 명세

rating은 `g`(General), `s`(Sensitive), `q`(Questionable), `e`(Explicit) 네 값이다.
사이트 별칭으로 `safe`는 `s`, `sfw`는 `g,s`, `nsfw`는 `q,e`에 대응한다.

제한 수에 포함되지 않는 메타태그는 다음과 같다.

```text
status rating limit is id date age filesize filetype parent child md5 width height
duration mpixels ratio score upvote downvotes favcount embedded tagcount pixiv_id pixiv
```

날짜는 `date:2026-08-01..2026-08-31`, 수치는 `score:>=20`처럼 범위·비교 연산자를
사용할 수 있다. Atsumi 상세 조건 UI는 자주 쓰는 rating, filetype, date, score,
favcount, parent/child를 체크박스·날짜·숫자·드롭다운으로 제공하고 나머지는 고급 검색
문자열로 직접 입력할 수 있게 둔다.

정렬은 검색당 하나만 사용하며 `order:` 자체가 제한 대상 조건 하나를 차지한다.
Atsumi는 최신 등록순(메타태그 없음), 오래된 등록순, 점수, 즐겨찾기, 해상도, 파일
크기, 태그 수, 세로 우선, 가로 우선을 제공한다. 기간을 선택하고 점수순으로 정렬하면
특정 기간 인기순에 해당하는 결과를 얻는다.
