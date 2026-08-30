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

2026-08-31 공개 스트림 24시간 실제 시간 검사를 백그라운드로 시작했다. 결과는 `%LOCALAPPDATA%\Investa\audits\market-stream-soak-24h-20260831.json`에 원자적으로 생성되며 종료 전에는 완료로 판정하지 않는다. 표준 출력·오류도 같은 폴더에 분리 저장한다.
