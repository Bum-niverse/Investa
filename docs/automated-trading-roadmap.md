# 자동매매·모델 개발 기준 로드맵

기준일: 2026-08-27

이 문서는 이후 사용자가 `다음 개발 단계`를 물을 때 사용하는 고정 우선순위다. 대화 중 다른 기능을 다뤘더라도 완료되지 않은 가장 앞 단계부터 다시 이어간다.

## 제품 경계

- LLM과 ML은 분석·확률·전략 제안까지만 담당한다.
- 빠른 주문 판단·위험 제한·체결 관리는 버전이 고정된 결정론적 Rust 알고리즘이 담당한다.
- 현재 제품은 `SHADOW ONLY`다. 외부 실주문과 출금 전송 함수는 추가하거나 활성화하지 않는다.
- 백테스트 통과는 연구 자격일 뿐 자동 배치 승인이 아니다. 사용자 승인 뒤 내부 섀도우·모의원장에만 배치한다.
- 판단 주기와 주문 실행 주기를 분리한다. 예를 들어 5분봉 전략도 체결 관리는 틱 단위로 수행할 수 있다.

## 고정 개발 순서

### 1. 전략 주기 계약

- 판단 주기: tick, 1m, 3m, 5m, 15m, 30m, 1h, 4h, 1d
- 실행 주기: tick 또는 고정 초 단위
- 자산·공급자·전략별 지원 여부와 불가 사유 표시
- 백테스트 interval과 운용 interval 불일치 차단

2026-08-30 판단 주기를 `tick·1m·3m·5m·15m·30m·1h·4h·1d`의 닫힌 Rust enum으로 고정하고, 실행 관리 주기를 `tick` 또는 `15~86,400초`로 분리했다. 현재 네 전략 플러그인은 완료 봉 기반이므로 tick 판단을 명시적으로 거부한다. 서버가 검증한 저장 PIT 데이터셋·로컬 완료 봉 집계·정규화 체결 스트림의 시장·주기 지원 목록만 사용하며, 백테스트·런타임·판단 interval이 다르면 실패로 닫는다. 성공 계약에도 `liveOrderAllowed=false`를 강제한다. 상세 근거는 [전략 판단·실행 주기 계약](strategy-cadence-contract.md)에 있다.

### 2. 실시간 Tick·봉 집계

- 공식 WebSocket 체결·호가를 공통 Tick으로 정규화
- Tick에서 1분 완료 봉 생성, 상위 분봉·시간봉 집계
- 중복·순서 역행·gap·stale·재연결·REST 백필 검사
- 국장·미장 세션과 휴장, 코인 24시간, 선물 만기·롤·펀딩·mark/index 경계 분리

2026-08-28 공급자 전송과 분리된 Rust 집계 코어를 추가했다. 공통 Tick 계약과 완료 1분봉, 3·5·15·30·60·240분 상위 봉을 결정론적으로 생성한다. 중복·역순·원천 혼합·overflow는 거부하고 빈 분은 생성하지 않으며 gap으로 남긴다. 상위 봉은 필요한 1분봉이 모두 있을 때만 확정한다.

같은 날 Upbit trade와 Binance Spot·USDⓈ-M·COIN-M aggregate trade를 공통 Tick으로 연결했다. Binance 선물 mark/index/funding은 같은 combined 연결에서 받되 체결 봉에는 섞지 않는다. partial·최근 완료 1분봉·gap·마지막 순번을 SQLite에 체크포인트하고 앱 재시작 뒤 복원한다.

2026-08-30 Upbit·Binance 공식 공개 REST로 체크포인트의 첫 gap을 제한적으로 복구하는 경로를 추가했다. 복구 중 스트림 상태가 바뀌면 재시도를 요구하고, 완료·범위·중복·단위 검사를 통과한 1분봉만 저장한다. 무거래 분은 보간하지 않는다. 토스 인증 WebSocket과 자산별 장시간 실제 왕복은 아직 남아 있다.

2026-08-31 토스 인증 WebSocket Rust 전송을 구현했다. OAuth access token은 handshake 헤더 내부에서만 사용하고, 체결·호가 topic만 허용하며 개인 주문 topic은 생성·수신하지 않는다. 60초 PING, ack timeout, backoff+jitter 재연결과 체결 Tick의 SQLite 완료 봉 집계를 연결했고 공식 101 handshake·국장 구독 ack를 통과했다. 공개 코인 스트림은 재연결 뒤 저장된 gap이 있을 때 기존 공식 REST 제한 복구를 자동 시도한다. 국장·미장 장중 체결·호가와 자산별 24시간 실제 내구 검증은 계속 미완료다.

### 3. 전략 플러그인

- 이동평균 교차, 가격 채널 돌파, 평균 이격 회귀, ATR 변동성 확장을 버전형 순수 Rust 플러그인으로 실행
- 각 플러그인이 지원 시장·주기·필요 가격봉 필드·진입·청산 방향을 명시
- 백테스트와 섀도우 최신 신호가 같은 디스패처를 사용하고, 혼합 플러그인·미지원 주기·필드 누락은 실행 전 거부

2026-08-28 네 플러그인의 v1 계약과 카탈로그·사전 검증 IPC를 구현했다. 기존 이동평균 `StrategySpec` JSON은 schema v1 그대로 읽으며, 신규 전략도 완료된 OHLCV와 `availableAt`·`ingestedAt`만 사용한다. RSI/MACD와 호가 불균형은 아직 구현하지 않았고, 특히 호가 불균형은 체결 봉이 아닌 시점 정합 호가 데이터 계약이 먼저 필요하다.

### 4. Rust 상시 스케줄러

- React 화면 타이머에 의존하지 않고 Tauri 프로세스에서 감시
- SQLite의 활성 감시를 앱 재시작 뒤 자동 재개
- 동시 tick·중복 신호·중복 후보 차단
- 화면은 상태를 읽기만 하고 주문 판단을 수행하지 않음

2026-08-27 첫 수직 기능으로 이동평균 전략 감시를 Rust 백그라운드 worker로 이전했다. 저장된 백테스트 interval을 단일 원본으로 사용해 `1m`·`1d` fresh bar를 구분하며 다른 주기는 거부한다.

2026-08-28 연구원 패널에서 저장 보고서를 `1m` 또는 `1d`로 재검증할 수 있게 했다. 재검증은 기존 결과를 수정하지 않고 interval이 포함된 새 experiment·dataset ID로 저장한다. 주식과 코인은 완료 봉만 사용하며 1분봉 200개는 약 3시간 20분에 불과하므로 승격 근거가 아닌 연결·탐색 결과로 경고한다. 새 실험에서 시작한 섀도우 감시는 저장된 동일 interval을 사용한다. 아직 `3m` 이상 집계, tick 전략, 앱 프로세스 종료 뒤 서버 실행을 뜻하지 않는다.

### 5. 내부 주문 실행 알고리즘

- 지정가·시장성 지정가, 재호가, 취소·재주문, 부분체결, 만료
- 최대 슬리피지·분할 수량·최소 주문 단위·충돌 방지
- 증권 선물과 코인 무기한선물의 레버리지·증거금·reduce-only·청산가 경계
- 내부 모의원장에서 먼저 검증하고 외부 모의계좌는 별도 승인 뒤 연결

2026-08-28 `internal-execution-v1` 결정론적 Rust 계약을 구현했다. 최초 기준가와 최대 슬리피지, 최소 수량 단위, 자식 주문 수·수량, 재호가 횟수와 만료 시각을 계획 생성 시 고정한다. 부분체결은 거래량으로 추정하지 않고 명시적으로 입력된 체결 사건만 누적하며, 동일 사건은 멱등 처리하고 다른 내용의 중복 ID와 동시 상태 변경은 거부한다. 증권 선물·코인 무기한선물은 최대 2배 격리증거금, 유지·초기증거금, mark 대비 청산 완충과 reduce-only 방향·잔여 포지션 한도를 통과해야 한다. 모든 응답은 `liveOrderAllowed=false`이며 외부 주문 전송은 없다.

설계 비교 자료:

- QuantConnect LEAN 주문 사건과 비동기 부분체결 상태: https://www.quantconnect.com/docs/v2/writing-algorithms/trading-and-orders/order-events
- QuantConnect Time In Force: https://www.quantconnect.com/docs/v2/writing-algorithms/trading-and-orders/order-properties
- NautilusTrader `OrderFilled`와 사건 기반 실행·대사: https://nautilustrader.io/docs/latest/concepts/events/order_filled/ , https://nautilustrader.io/docs/latest/concepts/reconciliation/
- Hummingbot executor의 전략 판단과 실행 수명주기 분리: https://hummingbot.org/strategies/v2-strategies/executors/
- Binance USDⓈ-M 공식 주문 필드의 `executedQty`, `reduceOnly`, `timeInForce`: https://developers.binance.com/docs/derivatives/usds-margined-futures/trade/rest-api/New-Order

외부 코드는 복사하지 않았다. Investa의 고정소수점·append-only 사건·사용자 승인·SHADOW ONLY 경계에 맞춰 새 의존성 없이 독립 구현했다.

### 6. 전략 승격·자동 배치·롤백

`백테스트 → OOS/Walk-forward/비용 스트레스 → 승격 후보 → 사용자 승인 → 섀도우 → 내부 모의운용 → 유지·중지·롤백`

- 전략·데이터·파라미터·비용 버전 고정
- Canary 배치와 성과 악화 자동 정지
- 승인·배치·중지·롤백 감사 사건 보존

2026-08-28 `strategy-deployment-v1`을 구현했다. 승격 후보를 만들 때 저장된 백테스트와 해당 Walk-forward 결과의 실제 연결을 다시 확인하고 `paper-review-v1` 전 항목 통과를 요구한다. 원본 experiment·dataset·Walk-forward ID, 전략 스키마, 진입·청산 플러그인 ID와 버전, 코드·공급자·봉 주기, 원래 비용과 1.5배·2배 비용 스트레스 결과를 SHA-256 근거 묶음으로 고정한다. 비용 0 또는 스트레스 수익이 0 이하인 결과는 배치할 수 없다.

수명주기는 `승인 대기 → SHADOW Canary → Canary 통과 → 내부 모의운용`이며 각 승격에 서로 다른 명시적 사용자 승인 문구가 필요하다. Canary와 내부 모의운용 관측은 최소 표본, 순손익, 최대낙폭, 평균 슬리피지와 오류 수를 결정론적으로 평가한다. 성과 악화나 운영 오류가 발생하면 새 진입을 위해 사용하던 섀도우 감시를 즉시 중지한다. 새 버전을 내부 모의운용으로 올릴 때 직전 버전은 삭제하지 않고 `superseded`로 보존하며, 같은 전략 슬롯의 바로 이전 버전만 별도 승인 후 롤백할 수 있다. 모든 상태와 사건은 SQLite에 append-only로 남고 모든 응답은 `liveOrderAllowed=false`다. 외부 주문 전송은 추가하지 않았다.

설계 비교 자료는 MLflow Model Registry의 버전·검증 상태·champion/challenger alias, Argo Rollouts의 작은 Canary 단계·pause·분석 실패 abort, Freqtrade의 dry-run 보호장치, Qlib Recorder의 실험 계보다. 외부 코드는 복사하지 않았고 새 패키지도 추가하지 않았다.

### 7. ML 모델 개발

- 수년치 OHLCV와 시점 정합 뉴스·공시·재무·커뮤니티 데이터셋
- 누수 방지 split과 자산·주기별 피처
- Python 격리 worker와 LightGBM/XGBoost 기준 모델
- Chronos·TimesFM 비교 어댑터
- 상승·하락·횡보 확률, 보정, OOS 평가, 모델 레지스트리와 재학습
- 범용 기본 모델 + 자산군별 모델 + 주기별 head를 기본 구조로 사용하고 필요한 종목만 미세조정

2026-08-28 `investa-ml-worker-v1` 기반을 구현했다. 기존 PIT 품질 감사와 동일한 데이터만 불변 매니페스트로 만들고, 표본·피처의 정렬된 payload 및 피처 스키마를 각각 SHA-256으로 고정한다. train·validation·test에 최소 표본을 요구하며 학습·검증 타깃 관측 시각이 다음 구간에 닿으면 누수로 거부한다.

같은 날 `pit-dataset-builder-v1`을 추가했다. 주식 adjusted close·코인 현물 close·증권선물 settlement·코인 무기한선물 mark의 라벨 정책과 비중첩 수집 창을 고정한다. 기업행사·만기·롤오버·펀딩 경계, 24시간 시장 gap, 중복·단위 혼합을 검사하고 결정 시각에 이용 가능했던 최신 리비전만 as-of 조인한다. 유효 결과만 기존 Forecast 감사와 ML 불변 매니페스트로 저장하며 외부 주문 권한은 없다. Upbit·Binance 공개 가격의 장기 수집 작업 오케스트레이터는 멱등 생성, 원자적 lease, 페이지 체크포인트, 제한된 재시도와 취소·재개 이벤트까지 구현했다. 토스 주식 수년 이력과 앱 밖 반복 스케줄링은 아직 남아 있다.

LightGBM·XGBoost·Chronos·TimesFM worker 요청은 코드 버전·seed·horizon·자원 상한·제한된 scalar 파라미터만 받으며 경로나 쉘 명령을 받지 않는다. worker bundle 조회 시 데이터 해시를 다시 검사하고, 성공 결과도 허용 포맷·파일명·크기·SHA-256·OOS 지표를 갖춘 `candidate_review`로만 등록한다. pickle과 자동 배치, 외부 주문 권한은 없다. 2026-08-28 저장소 밖 격리 Python 3.14 환경에 LightGBM 4.7.0·XGBoost 3.4.1 기준 worker를 구현하고 synthetic PIT 데이터로 두 포맷의 실제 학습·CLI 왕복을 검증했다. worker가 반환한 test 표본별 백만분율 방향 확률을 Rust가 다시 계산해 지표·fold·표본 수 불일치를 등록 전에 거부한다. 수년치 공식 PIT 데이터 수집은 아직 완료하지 않았다.

2026-08-28 `ml-worker-runner-v1`을 구현했다. 저장된 prepared 작업만 개발·패키지의 고정 worker resource로 실행하고 금융·Cloud·GitHub·Telegram 환경 비밀을 상속하지 않는다. timeout, stdout·stderr·결과 크기, 비정상 종료와 잘못된 JSON을 제한된 실패 코드로 저장한다. Windows Job Object가 프로세스와 전체 자식 작업의 메모리 상한 및 `KILL_ON_JOB_CLOSE`를 강제하며, 실제 아티팩트 크기·SHA-256도 Rust가 파일에서 다시 확인한다. 이 완료는 synthetic 기준 학습 실행 안전성을 뜻하며 실제 시장 예측 성능을 뜻하지 않는다.

2026-08-30 Binance BTC·ETH 현물·USDⓈ-M의 730일 `1h`·`4h`·`1d`를 48개 expanding walk-forward 모델로 검증했다. 레짐 임계값은 각 fold의 과거 학습 표본만 사용해 산출하고 OOS 거래를 6개 관측 상태로 분리했다. 12개 조합 모두 기본 비용에서 순손실이므로 모델은 계속 기각하며 자동 배치와 주문 권한은 열지 않는다.

세부 계약은 [ML 모델 파이프라인 기반](ml-model-pipeline.md)에 기록한다.

### 8. 외부 연결·장시간 검증

- Toss 인증 WebSocket, KIS 모의계좌·선물, Binance Testnet
- SEC·Telegram·OpenDART·네이버 뉴스와 선택 커뮤니티
- Claude·Antigravity 실제 분석 왕복
- 24시간 스트림·섀도우·재시작·네트워크 단절 soak test

2026-09-02 OpenDART 공시목록과 네이버 뉴스 검색의 읽기 전용 Rust 어댑터를 추가했다. 자격정보는 Windows 자격 증명 관리자에만 저장하며 bounded query, 10초 timeout, 안전한 오류와 허용 DTO를 적용한다. Claude·Antigravity 단일 응답은 Codex와 같은 `RoleReport`·`DepartmentReport` 서버 검증을 통과해야 한다. 실제 API 키 왕복과 외부 AI의 직원별 스트리밍·취소·부서 집계 실행은 계속 미완료다.
- Cloud relay는 `run.app` 라우팅 문제 해결 뒤 마지막에 재개

2026-08-31 내부 섀도우 실제 시간 수집 경로를 운영 패널에 연결했다. 프로세스 working set은 Windows 공식 `GetProcessMemoryInfo`로 읽고, SQLite 파일 크기·활성 섀도우 작업자·내부 후보·로컬 원장 건강·재시작 대사를 Rust에서 수집한다. 세션은 1분 표본으로 재개할 수 있지만 3분 초과 공백과 대사 실패는 통과시키지 않는다. 이는 검사 기반 구현 완료이며 실제 24시간 통과를 의미하지 않는다.

## 다음 개발 단계 판정

1. 이 문서의 앞 단계부터 수용 기준을 확인한다.
2. 외부 계정·유료 API·새 라이브러리 승인이 필요한 항목은 건너뛰지 말고 `blocked`로 기록한다.
3. 승인 없이 가능한 하위 작업을 먼저 완료한다.
4. 코드·테스트·문서·ProjectStudio 상태가 모두 일치해야 해당 기능을 완료로 올린다.
