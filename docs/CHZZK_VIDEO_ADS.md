# CHZZK 영상 광고 필터

2026-09-12. Windows 앱 내부의 단일 공식 시청 WebView와 마도 **영상** WebView에 적용하는 제한적인 요청 필터다. WebView2 네이티브 `WebResourceRequested`를 사용하며 AdGuard 제품이나 List-KR 전체 필터 엔진을 설치한 것은 아니다.

## 적용 범위

현재 메인 문서가 `https://chzzk.naver.com/live/{32자리 채널 ID}`이고 요청이 GET + XHR/fetch일 때 다음 세 경로만 빈 HTTP 204 응답으로 처리한다.

| 호스트 | 정확한 경로 | 추가 조건 |
| --- | --- | --- |
| `api.chzzk.naver.com` | `/service/v1/lives/{liveId}/ads/current` | liveId는 1–20자리 숫자 |
| `api.chzzk.naver.com` | `/ad-polling/v1/lives/{liveId}/ad` | liveId는 1–20자리 숫자 |
| `nam.veta.naver.com` | `/gfp/v1/vas/vas` | `vsi`가 정확히 한 개이고 `LIVE_CHZZK_NDP_SCH` |

호스트·경로의 부분 문자열 검색으로 차단하지 않는다. 계정 창과 마도 채팅 전용 창에는 필터를 설치하지 않는다. 미디어 요청, 방송 CDN, 인증, 채팅, 확장·그리드, 추적·배너 경로는 이 필터의 대상이 아니다. OPTIONS/POST 등 다른 메서드와 알려지지 않은 주소·형식도 통과한다. 요청 본문·헤더·쿠키를 읽거나 기록하지 않으며, 프록시·인증서 설치·TLS 중간 처리는 없다.

단일 시청은 초기 `about:blank`에서 필터와 기존 캡처 연결을 설치한 뒤 CHZZK로 이동한다. 마도도 최초 CHZZK 이동 전에 같은 필터를 설치한다. 필터 초기화 실패 시 차단은 활성화되지 않고 시청을 계속 사용할 수 있다. 문제 분리용으로 프로세스 시작 환경 변수 `ATSUMI_CHZZK_VIDEO_AD_FILTER=0`을 지정하면 필터를 끈다. 실행 중 환경 변수 변경이나 일반 새로고침으로는 전환되지 않는다.

## 확인한 공개 근거와 한계

- [List-KR의 CHZZK 요청 규칙](https://github.com/List-KR/List-KR/blob/master/filterslists/adblocking/filters-share/specific_URL.txt)은 두 광고 상태 API를 CHZZK XHR 차단 대상으로 지정한다. 이 구현은 해당 경로를 더 좁게 해석하며 List-KR 필터 내용이나 스크립트를 내려받아 실행하지 않는다.
- [Brave 공식 지원 게시판의 원본 광고 제보](https://community.brave.app/t/brave-shield-javascript-and-ad-blocking-at-naver-com/644758)는 위 NAVER URL과 `LIVE_CHZZK_NDP_SCH` 스케줄을 함께 제시한다. [Crackle의 공개 영상 광고 스케줄 코드](https://github.com/FilteringDev/crackle/blob/main/userscript/source/resource.ts)에서도 같은 CHZZK 라이브 스케줄 ID를 사용한다. 다른 스케줄·광고 구좌까지 확대하지 않았다.
- [List-KR의 NAVER 호환 규칙](https://github.com/List-KR/List-KR/blob/master/filterslists/adblocking/filters-AG/antiadblock.txt)은 더 넓은 `/gfp`, `/nac` 등의 처리와 페이지 스크립트 수정도 포함한다. 이 앱 필터에는 포함하지 않았다.
- 최신 광고에는 암호화된 GFP/Waterfall 응답이나 `seoraksan` 터널이 있다. [Crackle의 응답 스키마](https://github.com/FilteringDev/crackle/blob/main/userscript/source/tunneled-schema.ts)는 터널 응답에 `livePlaybackJson`이 함께 있음을 보여준다. 이 구현은 터널·복호화·플레이어 데이터에 개입하지 않으므로 해당 방식의 광고는 남을 수 있다.
- [uBlock Origin의 2026-08-20 원본 제보](https://github.com/uBlockOrigin/uAssets/issues/34177)는 페이지의 XHR 속성 덮어쓰기에 대한 CHZZK 감지를 기록한다. 이 구현은 페이지 XHR, MSE, 영상 캡처 훅을 변경하지 않는다. 네이티브 처리만으로 모든 광고 감지를 피한다고 보장하지 않는다.

## 검증 범위

`browser_video_ads.rs`의 합성 단위 테스트는 허용된 세 경로, 다른 호스트·스케줄·메서드·리소스 유형, 계정·채팅 문서, 위장 호스트, 잘못된 경로, 중복 스케줄·과도한 길이를 확인한다. 이는 규칙 경계 검증이며 실제 CHZZK 광고·계정·방송·녹화 검증이 아니다. 15초 광고 전체 차단이나 재생·녹화 연속성을 확인한 결과로 표현하지 않는다.

`cargo run --offline --example chzzk_video_ad_probe`는 임시 WebView2 프로필과 숨겨진 창에서 같은 운영 코드의 네이티브 요청 처리를 검증하는 격리 fixture다. 모든 요청에 먼저 로컬 응답을 설정하고 DNS도 차단하므로 실제 NAVER 서버나 계정·사용자 DB를 사용하지 않는다. 첫 라이브 문서 이동에서 세 광고 요청만 네이티브 204가 되는지, 미디어·계정·채팅·공용 GFP·다른 스케줄·POST·알 수 없는 경로가 로컬 200으로 유지되는지, 이후 채팅/계정 문서에서는 광고 경로도 통과하는지 확인한다. 40초 제한에서 네이티브 창을 닫고 종료한다.

필터의 빈 204 응답은 `https://chzzk.naver.com` 하나만 허용하는 고정 CORS 헤더를 사용한다. Origin 반사나 와일드카드는 없고 원래 서버 응답·개인 데이터도 포함하지 않는다. 격리 WebView2 실행에서 CORS 헤더가 없는 응답은 JS 상태 0으로 거절되고 네이티브 응답 완료 관측에도 나타나지 않아, 정확한 출처의 페이지가 빈 응답을 읽을 수 있도록 했다. probe는 JS의 실제 204와 네이티브 `WebResourceResponseReceived`의 204를 모두 요구한다. 실제 CHZZK가 빈 응답을 처리하는 방식과 광고 대체·재시도·재생 연속성은 별도 실사용 검증이 필요하다.

2026-09-12 격리 네이티브 실행은 약 3.4초에 exit 0 / `success: true`로 통과했다. 최초 라이브 문서에서 광고 3건의 JS·네이티브 204, 정상 요청 9건의 네이티브 200(합성 WAV 미디어 2건 로드 포함), 이후 채팅·계정 문서에서 각각 광고 경로의 200을 확인했다. 총 14개 요청 상태 검증이며, 종료 후 probe와 해당 WebView2 자식 프로세스는 남지 않았다. 종료 시 WebView2의 창 클래스 정리 경고 1412가 한 번 기록됐지만 요청 검증은 통과했다. 실제 CHZZK 광고·방송의 결과는 아니다.
