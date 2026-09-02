# 공식 공개 실시간 시장 스트림

기준일: 2026-08-27

Investa의 첫 실시간 스트림은 계좌 키가 필요 없는 공개 시장 데이터만 사용한다. 개인 잔고·주문·체결 WebSocket은 연결하지 않으며 외부 주문 권한도 열지 않는다.

## 연결 범위

- Upbit `KRW-BTC` trade
- Binance Spot `BTCUSDT` aggregate trade
- Binance USDⓈ-M `BTCUSDT` aggregate trade + mark/index/funding
- Binance COIN-M `BTCUSD_PERP` aggregate trade + mark/index/funding

UI의 `운영 준비·근거·복구`에서 사용자가 스트림을 시작·중지·전체 재연결한다. 관측이 15초 이상 없으면 `stale`로 전환하고 지수 백오프로 재연결한다. Binance 연결은 공식 24시간 연결 제한 전에 선제 교체한다. 이벤트 순번이 제공되는 trade는 이전 값 이하의 중복·역행을 오류로 처리한다. 순번 증가 폭만으로 누락 체결을 추정하지 않는다.

## 보안과 데이터 품질

- CSP `connect-src`는 Upbit와 Binance의 공식 WebSocket 호스트 네 곳만 허용한다.
- 수신 종목, 가격의 유한·양수 여부와 공급자 이벤트 시각을 검사한다.
- 공급자 시각이 로컬 수신 시각보다 60초 넘게 미래면 수신 시각으로 낮추며 조작된 미래 시각을 신뢰하지 않는다.
- mark price, index price와 funding rate를 서로 다른 필드로 보존한다.
- 실시간 값은 연구·표시용이며 주문 가능 여부나 완료봉을 자동으로 증명하지 않는다.

## 공통 Tick과 완료 봉 집계 코어

`src-tauri/src/market_aggregation.rs`는 공급자 전송과 분리된 순수 Rust 계약이다.

- 공통 Tick은 공급자·자산군·종목·통화, 이벤트·수신 시각, 선택적 공급자 순번, 통화 최소 단위 가격, 고정소수점 수량과 scale을 보존한다.
- 서로 다른 원천·단위 혼합, 0 가격·수량, 미래 수신, 동일 Tick, 이벤트 시각 또는 공급자 순번 역행을 거부한다. 순번 증가 폭만으로 누락 체결을 추정하지 않는다.
- Tick은 UTC epoch에 정렬된 1분 OHLCV로 집계한다. watermark 이전에 끝난 봉만 완료로 내보내고 현재 분은 partial로 분리한다.
- 관측이 없는 분은 가짜 횡보 봉을 생성하지 않고 `MarketDataGap`으로 보존한다.
- 3·5·15·30·60·240분 봉은 정렬된 완료 1분봉이 구간 전체에 모두 있을 때만 생성한다. 한 분이라도 없으면 해당 상위 봉은 결과에서 제외하고 gap을 반환한다.
- 가격·거래량·Tick 수 overflow는 오류로 중단한다.

React 공개 WebSocket은 체결 가격·수량·순번을 공급자별로 검증한 뒤 스트림별 직렬 큐를 통해 Tauri IPC로 보낸다. Rust 런타임은 partial 봉, 최근 완료 1분봉 480개, 상위 봉 마지막 방출 시각, gap과 마지막 순번을 SQLite `market_stream_checkpoints`에 저장한다. 앱 재시작 뒤 첫 체결에서 체크포인트를 복원하며, UI는 보존된 완료 1분봉과 gap 수를 표시한다. mark/index/funding 메시지는 표시 근거로만 유지하고 OHLCV 체결 Tick으로 가장하지 않는다.

2026-08-30 Upbit·Binance 공개 스트림에는 공식 REST gap 복구를 연결했다. 체크포인트에 이미 기록된 첫 gap만 공급자별 단일 요청 상한 안에서 조회하며, 조회 중 상태 변경·요청 범위 밖 봉·미완료 봉·중복·역순·단위 불일치는 실패로 닫는다. Upbit 무거래 분처럼 공식 API에도 봉이 없으면 gap을 그대로 남기고 횡보 봉을 만들지 않는다. 복구는 공개 시세만 사용하며 자격정보와 주문 권한은 요구하지 않고 `liveOrderAllowed=false`를 유지한다.

2026-08-31 토스 인증 WebSocket은 Rust 전송으로 연결했다. 토큰은 Windows 자격 증명 관리자에서 읽어 OAuth cache를 거친 뒤 handshake의 `Authorization` 헤더에만 사용하며 React·상태 DTO·로그로 내보내지 않는다. 사용자가 입력한 국장·미장 종목의 체결·호가 market topic만 선언하고 `personal:order`는 빌더와 파서 모두 거부한다. 공식 권장대로 순수 텍스트 `PING`을 60초마다 보내고 15초 ack timeout, 최대 30초 지수 backoff+jitter, `server-shutdown` 재연결과 구독 재선언을 적용했다. 저장 자격정보로 공식 101 handshake와 `trade:kr:005930` 구독 ack를 실제 확인했다.

토스 체결에는 sequence가 없고 공식 문서가 lossy 스트림이라고 명시하므로 누락 체결 수를 꾸며내지 않는다. Rust 집계 코어는 체결 시각의 빈 분을 gap으로 남기며 체결 프레임 합산을 공식 누적 거래량으로 표시하지 않는다. 공식 handshake 뒤 즉시 PING/pong은 확인했지만 국장·미장 장중 체결·호가, 60초 주기 장시간 PING/pong, 재연결과 전체 자산 24시간 실제 왕복은 아직 남아 있어 전체 실시간 전략 배치 완료로 올리지 않는다.

## 실제 검증

2026-08-27 로컬 네트워크에서 Upbit, Binance Spot, Binance COIN-M은 실제 메시지를 수신했다. Binance USDⓈ-M 호스트는 연결은 열렸지만 제한 시간 안에 메시지가 없어 장시간 재검사가 필요하다. 따라서 ProjectStudio의 스트림 구현 기준은 체크하되 장시간 gap·rate-limit·재시작 대사 기준은 미완료로 유지한다.

2026-08-28 체결 스트림 전환 뒤 25초 smoke에서는 Upbit trade 31건, Binance Spot aggregate trade 25건, COIN-M combined stream 25건을 수신했다. USDⓈ-M combined stream은 0건이고 stale 재연결이 1회 발생했다. 이 결과는 짧은 연결 검사일 뿐 24시간 내구 검증이 아니며 USDⓈ-M 실제 왕복은 계속 미완료다.

2026-08-31 60초 재검사에서는 Upbit 50건, Binance Spot 676건, COIN-M 104건을 받았고 오류·재연결은 없었다. USDⓈ-M은 0건, stale·재연결 5회로 다시 실패했다. raw aggTrade, combined aggTrade, raw markPrice, combined aggTrade+markPrice 네 공식 URL과 구독 메시지 방식 모두 handshake/ack 뒤 시장 프레임이 0건이라 애플리케이션 파서보다 공급자 또는 현재 네트워크 경로 문제로 분리했다. 이 상태에서는 24시간 검사를 시작하더라도 USDⓈ-M 수용 기준을 통과한 것으로 표시하지 않는다.

토스증권 최신 공식 AsyncAPI에는 인증형 WebSocket(`wss://openapi-ws.tossinvest.com/ws/v1`)이 공개되어 있다. 브라우저 `WebSocket`은 `Authorization` 헤더를 넣을 수 없으므로 인증 연결은 Rust에서만 수행하고, 국장·미장 화면의 REST polling과 공식 `market-calendar/KR`, `market-calendar/US`도 기준 snapshot 경로로 유지한다.

Rust의 `toss_stream` 모듈에는 다음 전송 독립 계약을 구현했다.

- `trade:kr`, `trade:us`, `orderbook:kr`, `orderbook:us` 전체 교체 구독 선언
- 연결당 100개 제한, 종목 형식, 중복 topic 검증
- subscriptions ack·부분 거부·error·server-shutdown·pong 파싱
- 체결·호가의 종목, 통화, RFC 3339 시각, 0 이상 decimal 문자열 검증
- `personal:order` 선언·수신의 명시적 차단

실제 전송은 `tokio-tungstenite 0.30`의 복합 handshake request와 Windows native TLS를 사용한다. 앱은 연결 상태·확정 및 거부 topic 수·체결/호가 수·완료 봉 수만 노출하고 access token과 Authorization 값을 직렬화하지 않는다. REST polling은 연결 전·장중 체결이 없는 구간의 기준 snapshot 경로로 계속 유지한다.

### 실제 시간 내구 검사

`scripts/market_stream_soak.mjs`는 계좌 키 없이 네 공개 스트림을 독립 연결하고 20초 stale, 재연결 횟수, 오류, 수신 수와 최대 수신 간격을 JSON으로 남긴다. `simulatedTimeline=false`와 실제 경과 시간이 함께 저장되며 24시간을 실제로 경과하지 않은 실행은 `actualElapsed24hQualified=false`다.

```powershell
node scripts/market_stream_soak.mjs --duration-seconds 86400 --output "$env:LOCALAPPDATA\Investa\audit\market-stream-soak-24h.json"
```

이 검사는 공개 시세 전송 내구성만 검증한다. 내부 원장·후보·재시작 대사는 기존 `shadow_soak_audit_save`의 실제 시간 표본으로 별도 검증하며, 두 결과 중 하나라도 실패하면 장시간 운용 완료로 올리지 않는다.

내부 섀도우 검사는 `운영 준비·근거·복구`의 `24시간 내부 섀도우 내구 검사`에서 시작한다. 1분마다 Windows 프로세스 working set, SQLite 파일 크기, 활성 섀도우 작업자 수, 최근 내부 후보, SQLite·KRW·USD 원장 건강 상태와 재시작 대사 결과를 수집한다. 진행 세션은 자격정보 없이 로컬 저장소에 보존되어 앱 재시작 뒤 이어지지만, 실제 표본 간격이 3분을 넘거나 재시작 대사가 실패하면 결과는 fail-closed다. 실행 중 앱을 오래 종료한 시간을 24시간 운용으로 꾸며내지 않는다.

화면을 열지 않고 같은 검사를 수행할 때는 빌드된 Tauri 실행 파일에 `--shadow-soak-autostart`를 전달한다. 이 플래그는 메인 창을 숨긴 뒤 실제 Rust 섀도우 worker와 동일 프로세스의 메모리·SQLite·원장을 1분마다 관측한다. `%APPDATA%\com.bumniverse.investa\audits\shadow-soak-<시작시각>.jsonl`에는 진행 표본을 append-only로 남기고, 24시간을 실제로 경과한 뒤에만 `.result.json`과 SQLite 감사 결과를 확정한다. `shadow-soak-24h.lock`은 동시에 두 검사가 원장과 진행 파일을 공유하지 못하게 원자적으로 차단한다. 비정상 종료로 잠금 파일이 남으면 자동 성공 처리하거나 이어 붙이지 않고, 원장 대사와 실패 원인을 확인한 뒤 새 실행으로 시작한다.

2026-08-31 공개 스트림 24시간 실제 시간 검사를 백그라운드로 시작했다. 결과는 `%LOCALAPPDATA%\Investa\audits\market-stream-soak-24h-20260831.json`에 원자적으로 생성되며 종료 전에는 완료로 판정하지 않는다. 표준 출력·오류도 같은 폴더에 분리 저장한다.

2026-08-31 06:53 KST에는 `--shadow-soak-autostart`로 화면 비의존 내부 섀도우 실제 시간 검사를 함께 시작했다. 첫 표본과 60초 뒤 두 번째 표본에서 공급자 건강과 원장 대사가 모두 통과했고, 중복 실행 프로세스는 DB 상태를 변경하지 않고 종료됐다. 이 짧은 확인은 시작 경로 검증일 뿐이며 실제 24시간 결과는 계속 미완료다.

## Cloud Run Job 분리 실행

PC 전원과 무관한 검수는 `server/cloud-soak` 이미지의 두 Cloud Run Job으로 분리한다. `market`은 네 공개 WebSocket의 실제 장시간 수신을, `shadow-contract`는 비밀정보와 사용자 DB를 반출하지 않은 격리 SQLite의 append-only·트랜잭션·대사 계약을 검증한다. 두 실행은 60초 heartbeat와 최종 `actualElapsed24hQualified`를 Cloud Logging에 남긴다.

격리 원장 검사는 Windows Tauri 프로세스, 사용자 SQLite, Windows 자격 증명 관리자와 실제 계좌 연결을 검증하지 않는다. 따라서 클라우드 두 작업이 통과해도 데스크톱 통합 24시간 검사는 별도 미완료로 유지한다. 이전 `--shadow-soak-autostart` 실행에서 확인된 Windows UI 런타임 종료 충돌을 성공으로 간주하거나 클라우드 계약 검사로 대체하지 않는다.

2026-09-01 최초 Cloud heartbeat에서 USDⓈ-M만 메시지 0건과 stale 재연결이 반복됐다. Binance 공식 변경 이력상 2026-04-23 기존 USDⓈ-M WebSocket 경로가 종료됐으므로 일반 mark price 스트림을 `wss://fstream.binance.com/market/ws/<stream>`으로 교체했다. 공개 고빈도 호가의 `/public`, 일반 시세의 `/market`, 사용자 데이터의 `/private` 경계를 섞지 않는다.

2026-09-02 구버전 `v1` 시장 실행은 약 7시간 46분 시점에 Upbit 체결 44,441건·WebSocket 오류 0건이었지만, 20초 무체결을 연결 장애로 처리해 stale·재연결을 각각 137회 만들었다. 이 실행은 취소하지 않고 최종 결과를 원본 그대로 보존한다. 수정판 `v2`는 30초 텍스트 PING과 `UP` 응답으로 전송 생존을 별도 확인하고, 이벤트 기반 `trade`의 20초 체결 공백은 경고로만 남긴다. 실제 전송 timeout·close·error와 시장 이벤트 공백을 서로 다른 필드와 판정으로 유지하며 새 실행 ID에서 다시 24시간 검증한다.

같은 날 기존 실행 `investa-market-soak-24h-5xp8d`와 `investa-shadow-contract-soak-24h-x95sr`가 각각 `runningCount=1`인 상태를 다시 확인한 뒤 별도 Job `investa-market-soak-24h-v2`를 만들었다. 새 이미지는 `20260902-1`(digest `sha256:6a1081a94c1d256746bae50987189d590e2b2894c64580958b1c1132318ca657`), 새 실행은 `investa-market-soak-24h-v2-mkr74`다. 이로써 v1 결과와 v2 결과는 Job·이미지·실행 ID가 모두 분리되어 서로 덮어쓰지 않는다.

v2 첫 60초 heartbeat(`investa.cloud-soak.v2`)에서 Upbit 체결 183건, 전송 heartbeat 3건, 오류·재연결·전송 timeout·시장 공백 이벤트 0건을 확인했다. 같은 표본에서 Binance 현물 648건, USDⓈ-M 60건, COIN-M 60건을 수신했고 세 스트림 모두 오류·재연결·전송 timeout 0건이었다. 이는 새 판정과 실행 경로의 초기 검증일 뿐이며 24시간 최종 통과는 `actualElapsed24hQualified=true`와 최종 `completed` 로그가 생성된 뒤에만 확정한다.

## 읽기 전용 결과 수집과 앱 표시

`scripts/cloud_soak_report.mjs`는 고정 프로젝트 `investa-remote-bumniverse`, 리전 `asia-northeast3`와 허용된 시장·섀도우 작업만 Google Cloud CLI로 읽는다. Cloud Run execution과 `investa.cloud-soak.v2` 구조화 로그에서 실행 시각, 최근 heartbeat, 허용된 카운터와 최종 판정만 추리고 원본 로그·토큰·환경변수는 복사하지 않는다. 결과는 앱 데이터 `audits/cloud-soak-status.json`과 사람이 읽는 Markdown에 임시 파일·이전 캐시 복구가 가능한 교체 방식으로 저장한다.

운영 패널은 Tauri가 고정 경로의 256KB 이하 캐시를 엄격한 스키마로 검증한 결과만 표시한다. 사용자가 누르는 `저장된 검사 결과 다시 읽기`는 Cloud API를 호출하거나 인증을 요구하지 않는다. 24시간 실측, 성공 종료와 이슈 0건을 모두 충족해야 `24시간 검사 통과`로 표시하며 수집 실패·구버전 로그·짧은 실행을 성공으로 해석하지 않는다.
