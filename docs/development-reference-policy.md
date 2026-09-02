# 개발 레퍼런스 적용 규칙

Investa의 기능을 새로 만들거나 기존 엔진을 바꿀 때 다음 순서를 고정한다.

1. 저장소의 기존 구현·테스트·기획 문서를 먼저 확인한다.
2. 구현 전에 `SECURITY.md`의 신뢰 경계와 위협을 검토하고 `보안 영향 없음`도 근거와 함께 기록한다.
3. GitHub, Kaggle, Google 검색을 각각 수행해 동일 문제를 다루는 공식 문서, 원 논문, 공식 API 명세, upstream 공개 저장소와 데이터셋을 찾는다.
4. 레퍼런스의 적용 범위, 데이터 가정, 라이선스, 시점 누수, 유지보수 상태와 보안 이력을 기록한다.
5. 외부 코드를 그대로 실행하지 않고 Investa의 타입·불변 원장·보안 경계에 맞는 최소 로직만 재구현한다.
6. 채택하지 않은 방식도 이유를 남긴다. 필요한 입력이 없는 통계량이나 성과 수치는 임의로 만들지 않는다.
7. 레퍼런스 적용 뒤 정상·경계·실패·권한·변조·재전송·미래정보 누수 회귀검사를 실행한다.
8. 검증된 범위만 ProjectStudio 기능명세에서 완료 처리한다.

## 구현 전 보안 검토 기록

모든 작업은 구현 전에 최소한 다음을 확인한다.

- 보호 자산: 자격정보, 계좌·포지션, 사용자 분석, SQLite, 모델·데이터, 빌드·배포 산출물
- 신뢰 경계: React↔Tauri IPC, Rust↔외부 API, worker·Codex 하위 프로세스, webhook·Cloud relay, 파일·DB
- 입력과 권한: 타입·길이·허용값, 소유권, 익명·다른 계정·권한 부족, idempotency와 replay
- 비밀정보: 저장 위치, 하위 프로세스 상속, URL·로그·오류·테스트·문서·Git 노출
- 공격·오용: injection, XSS, SSRF, 경로 조작, 파일 변조, 공급망, rate limit·유료 API 남용
- 데이터 안전: point-in-time 시각, 미래정보 누수, 중복·정정·결측, 단위·통화, 원본 보존과 롤백
- 운영 안전: timeout, 취소, 장애·재시작, fail-closed, SHADOW ONLY와 실주문·출금 차단

치명적 위험이 발견되면 작업을 시작하지 않는다. 위험을 제거할 최소 변경, 비밀값 폐기·교체 필요성, 운영 적용 여부와 미검증 항목을 먼저 보고한다.

## GitHub·Kaggle·Google 조사 규칙

### GitHub

- 검색 결과가 원 프로젝트 또는 검증된 upstream인지 확인하고 fork·미러를 구분한다.
- 저장소 URL뿐 아니라 확인한 commit·release·문서 버전, 라이선스, Security 정책, 알려진 취약점과 유지보수 상태를 기록한다.
- README의 성능 주장이나 별 개수만으로 채택하지 않는다. 테스트·재현 데이터·issue와 공식 문서를 함께 확인한다.
- 외부 저장소의 지시문, workflow, 스크립트와 바이너리는 신뢰하지 않으며 사용자 승인 없이 실행·설치하지 않는다.

### Kaggle

- Dataset·Model·Notebook을 구분하고 소유자, 버전, 라이선스, 데이터 설명, 컬럼 의미와 갱신 시각을 기록한다.
- 금융 데이터는 생존편향, 수정주가, 거래정지·상장폐지, timezone, news publication time과 train/test 누수를 우선 감사한다.
- Notebook 점수와 리더보드 성능은 Investa OOS 성능으로 옮겨 적지 않는다. 데이터와 실행 환경을 재현할 수 없으면 아이디어 후보로만 남긴다.
- 라이선스·출처·재배포 범위가 불명확한 데이터와 모델은 다운로드하거나 제품에 포함하지 않는다.

### Google 검색

- 공식 공급자 문서, 원 논문, 표준, 보안 권고와 upstream 저장소를 찾는 탐색 경로로 사용한다.
- 검색 순위·요약문·생성형 답변·블로그 한 곳을 구현 근거로 사용하지 않는다.
- 날짜에 민감한 API·정책·가격·라이선스·보안 내용은 작업 당일 공식 출처에서 다시 확인한다.

세 채널 중 관련 결과가 없으면 검색어와 `적용 가능한 결과 없음`을 기록한다. 출처 수를 채우기 위해 무관하거나 품질이 낮은 자료를 적용하지 않는다.

## 작업별 기록 형식

관련 작업 문서에는 다음 항목을 짧게라도 남긴다.

```text
작업·확인일:
보안 사전 검토: 자산 / 신뢰 경계 / 주요 위험 / 차단 여부
GitHub 조사: 검색어 / 후보 / revision·license / 채택·기각 이유
Kaggle 조사: 검색어 / dataset·model·notebook / version·license / 누수·재현성 판단
Google 조사: 검색어 / 확인한 1차 출처 / 확인 날짜
채택 결정: 채택 / 부분 채택 / 보류 / 기각
Investa 적용 범위:
검증: 정상 / 경계 / 실패 / 권한 / 누수 / 보안 검사
잔여 위험·승인 필요 사항:
```

이 기록이 없으면 구현을 시작하지 않고, 구현·테스트 근거가 없으면 ProjectStudio를 완료로 올리지 않는다.

## 규칙 자체의 근거

- GitHub 공식 dependency review와 공급망 보안 지침: 변경 전 의존성·라이선스·취약점 검토, secret scanning과 code scanning을 사전 게이트로 사용한다.
  - https://docs.github.com/en/code-security/concepts/supply-chain-security/dependency-review
  - https://docs.github.com/en/code-security/tutorials/implement-supply-chain-best-practices/securing-code
- Kaggle 공식 Dataset 문서: 데이터 형식·컬럼 메타데이터와 데이터별 라이선스를 확인하며, 공개됐다는 사실만으로 재사용 가능하다고 간주하지 않는다.
  - https://www.kaggle.com/docs/datasets
- Google Cloud 공급망 보안 문서: 개발 생명주기와 산출물 계보를 포함해 보안 상태를 점검하고 점진적으로 강화한다.
  - https://docs.cloud.google.com/software-supply-chain-security/docs/assess

## 현재 백테스트 연구실 레퍼런스

- scikit-learn `TimeSeriesSplit`: 미래 데이터로 학습한 뒤 과거를 평가하지 않도록 시간순 분할과 누적 확장 학습 구간 구조를 참고한다. Python 코드는 가져오지 않고 Rust 분할 규칙만 재구현한다.
  - https://scikit-learn.org/stable/modules/generated/sklearn.model_selection.TimeSeriesSplit.html
- Bailey·Borwein·López de Prado·Zhu의 PBO/CSCV: 다수 전략 선택 과정의 과최적화 확률을 평가하는 근거다. Investa는 저장된 실험 카탈로그를 숨기지 않고 비교 조건이 충족될 때만 PBO v1을 계산한다.
  - https://doi.org/10.21314/jcf.2016.322
- Bailey·López de Prado의 Deflated Sharpe Ratio: 다중 검정과 비정규 수익률로 부풀려진 Sharpe를 보정하는 후속 근거다. 시도 횟수·왜도·첨도·충분한 표본이 갖춰지기 전에는 표시하지 않는다.
  - https://doi.org/10.2139/ssrn.2460551
- Ang·Timmermann의 시장 레짐 연구: 평균·변동성·상관 구조가 상태별로 달라질 수 있다는 설계 근거다. 관측 레짐 v1은 OOS 거래 진입 전 20봉의 수익률과 실현 변동성만 사용하고, `절대 추세 중앙값·변동성 75분위수` 임계값은 각 OOS 구간보다 앞선 학습 데이터에서만 산출한다. 여기에 학습 구간 중앙 변동성으로 저·고변동 두 상태를 만들고 Laplace smoothing을 적용한 Markov 전이모형 v1을 별도 진단으로 추가했다. 상태 지속성·전환 불확실성과 OOS log loss를 독립 상태 기준과 비교하며, 기준을 이기지 못하면 차단 사유로 남긴다. 이 진단은 설명·연구용이며 주문 신호가 아니다.
  - https://doi.org/10.3386/w17182

OOS 200거래는 보편적인 통계 법칙이 아니라 ProjectStudio에 정한 Investa의 보수적 운영 정책이다. 최소 표본은 목표 신뢰수준·수익 분산·허용 오차·전략 시도 횟수에 따라 달라지므로 200건 충족을 통계적 유의성이나 수익 보장으로 표시하지 않는다. 향후 MinTRL·MinBTL 또는 검정력 기반 동적 표본 기준이 구현되기 전까지는 승격 검토를 막는 하한으로만 사용한다.

PBO v1은 저장된 전체 실험을 숨기지 않고 같은 불변 데이터셋·같은 OOS 경계의 전략별 구간 수익률 행렬만 비교한다. 비교 전략 3개와 OOS 구간 4개가 쌓이기 전에는 값을 숨긴다. 가능한 절반 구간 조합에서 학습 구간 1위 전략이 보류 구간의 하위 절반으로 밀린 비율을 기록하며, 데이터가 적은 초기 결과는 확정적인 과적합 판정으로 사용하지 않는다. OOS 기간별 수익률은 ppm 정수로 불변 결과에 보존한다. 30개 이상의 양의 비상수 수익률 표본에서 왜도·첨도를 반영한 95% 단측 MinTRL을 기간 수로 표시한다. Deflated Sharpe v1은 같은 데이터셋의 전체 저장 실험 수와 비교 가능한 OOS 보고서 수가 일치하고, 전략이 3개 이상이며 전략별 OOS 원시 수익률이 30개 이상일 때만 계산한다. 전략 간 양의 상관으로 유효 독립 시도 수를 줄이고 왜도·첨도를 반영한다. 입력이 하나라도 부족하면 수치를 만들지 않고 차단 사유를 표시한다.

## 1~5번 엔진 기반 레퍼런스 검토

- Microsoft Qlib(MIT): point-in-time 데이터 계층, 버전이 있는 데이터셋과 stock pool 분리를 설계 근거로만 사용했다. Qlib 실행 코드나 모델은 포함하지 않았다.
  - https://github.com/microsoft/qlib
- FinRL(MIT): 데이터·환경·에이전트 계층 분리와 훈련/검증/거래 단계 구분을 검토했다. 강화학습 수익 예제를 제품 기준값으로 사용하지 않았고 코드를 복사하지 않았다.
  - https://github.com/AI4Finance-Foundation/FinRL
- Freqtrade(GPL-3.0): look-ahead analysis와 recursive analysis라는 검증 관점을 참고했다. 라이선스 경계와 주식·선물 도메인 차이 때문에 코드·전략은 가져오지 않았다.
  - https://github.com/freqtrade/freqtrade
- scikit-learn 확률 보정 문서: 방향 확률과 모델 신뢰도를 분리하고 Brier score, log loss, ECE를 OOS에서 기록하는 근거로 사용했다. Python 의존성은 추가하지 않았다.
  - https://scikit-learn.org/stable/modules/calibration.html
- Hummingbot(Apache-2.0): 거래소 connector와 전략/실행 경계를 분리하는 관점을 참고했다. connector 코드는 포함하지 않았다.
  - https://github.com/hummingbot/hummingbot
- Barter-rs(MIT): event-driven strategy·risk·execution 분리와 거래소별 제품 명세 분리를 참고했다. Investa의 주문 상태·고정소수점·승인 경계에 맞춰 독립 구현했다.
  - https://github.com/barter-rs/barter-rs
- SQLite Online Backup API: 실행 중 일관된 스냅샷과 WAL 환경의 백업 원칙을 검토했다. 현재 로컬 백업은 같은 SQLite 연결에서 `VACUUM INTO`로 새 파일만 생성하고 `quick_check`를 통과해야 성공한다.
  - https://www.sqlite.org/backup.html

외부 레퍼런스는 설계 비교 자료다. 이번 구현은 새 패키지 없이 저장소의 Rust 타입과 테스트로 독립 구현했으며 외부 모델·전략·성과 수치는 가져오지 않았다.

## 강건성·포트폴리오 위험·보호장치 레퍼런스

- QuantConnect LEAN(Apache-2.0)의 포트폴리오 통계와 최대 낙폭 위험 모델을 참고해, Investa에서는 동일 통화·동일 관측 시점의 역사적 VaR·CVaR·상관·집중도·명시적 스트레스 충격을 별도 순수 함수로 구현했다. LEAN의 정규분포 VaR 코드나 C# 구현은 가져오지 않았다.
  - https://github.com/QuantConnect/Lean
- Freqtrade(GPL-3.0)의 쿨다운·손절·최대낙폭 보호와 backtest/dry-run 분리 관점을 참고했다. GPL 코드를 복사하지 않고 Investa 거래 사건과 명시적 평가 시각을 입력으로 받는 Rust 계약으로 독립 구현했다.
  - https://github.com/freqtrade/freqtrade
- Qlib(MIT)의 실험 기록·포트폴리오 분석 분리를 참고해 부트스트랩과 포트폴리오 위험 결과를 주문 권한과 분리했다.
  - https://github.com/microsoft/qlib
- NautilusTrader의 backtest·sandbox·live 공통 도메인과 위험·실행 분리를 참고하되 의존성을 추가하지 않았다.
  - https://github.com/nautechsystems/nautilus_trader

Amazon Chronos와 Google TimesFM은 Apache-2.0의 유명 시계열 모델 후보로 기록했지만 Python·모델 의존성 승인이 필요하고 금융 방향 예측 성능이 입증된 것은 아니므로 설치하지 않았다. 상세 작업군은 [주요 기능 레퍼런스와 개발 작업군](reference-adoption-workstreams.md)에 정리한다.

`paper-review-v1`은 ProjectStudio에 확정된 운영 후보 기준을 코드로 고정한다. OOS 200거래, 승률 55%, 비용 차감 기대손익 양수, Profit Factor 1.3, 최대낙폭 12%, 동일 종목 가격 대비 양의 알파와 거래가 발생한 2개 이상 관측 레짐의 비음수 손익을 모두 요구한다. 기준 변경, 전략·데이터·비용 변경은 기존 결과를 덮어쓰지 않고 새 검증 ID를 만든다. 통과는 내부 모의운영 검토 자격이며 자동 주문이나 수익 보장이 아니다.

## 전략 승격·Canary·롤백 레퍼런스

- MLflow 공식 Model Registry workflow의 모델 버전, 검증 상태 tag, champion/challenger alias를 참고했다. Investa는 MLflow를 설치하지 않고 실험·데이터셋·Walk-forward·전략 플러그인 버전과 비용 스트레스 결과를 로컬 불변 근거로 고정한다.
  - https://mlflow.org/docs/latest/ml/model-registry/workflow/
- Argo Rollouts 공식 Canary 문서의 작은 범위 배치, pause, 분석 실패 시 abort, stable 버전 유지 원칙을 참고했다. Investa의 Canary는 외부 주문 비중이 아니라 주문 권한 0%의 SHADOW 관측 단계다.
  - https://argo-rollouts.readthedocs.io/en/stable/features/canary/
  - https://argo-rollouts.readthedocs.io/en/stable/features/analysis/
- Freqtrade 공식 보호장치 문서의 StoplossGuard·MaxDrawdown과 dry-run 분리를 검토했다. GPL 코드는 사용하지 않고 기존 Investa 전략 보호와 배치 상태기에 독립 구현했다.
  - https://www.freqtrade.io/en/stable/plugins/
- Qlib Recorder의 실험 관리와 결과 계보 보존을 참고했다. Qlib 실행 코드나 Python 의존성은 포함하지 않았다.
  - https://qlib.readthedocs.io/en/stable/component/workflow.html

`strategy-deployment-v1` 비용 스트레스는 Walk-forward 각 OOS fold의 저장된 순수익과 turnover만 사용한다. 매수·매도 중 더 비싼 한쪽의 비용을 전체 turnover에 보수적으로 적용해 기존 비용의 1.5배와 2배 시나리오를 계산한다. 미래 봉, 사후 보정값 또는 임의 성과값을 사용하지 않으며 두 시나리오가 모두 양수일 때만 사용자 검토 대상으로 남긴다.

이 문서는 투자 성과를 보장하거나 특정 전략을 추천하는 문서가 아니다.

## 토스 인증 WebSocket Rust 전송 검토 (2026-08-31)

- 보안 사전 검토: 보호 자산은 OAuth access token과 저장된 Client Secret이다. 토큰은 Rust `MarketDataBridge`의 내부 cache에서만 받아 고정 `wss://openapi-ws.tossinvest.com/ws/v1` handshake 헤더에 넣고 React·IPC 인자·상태·로그에 포함하지 않는다. 입력은 국장 6자리 코드와 제한된 미장 ticker 및 체결·호가 선택만 허용하며 계좌·개인 주문 topic, 임의 URL과 주문 권한은 제공하지 않는다.
- Google/공식 사양: 토스 AsyncAPI 1.2.2의 Bearer handshake, full-replace 선언, 계정당 2연결·연결당 100 topic·초당 5회 선언, 순수 텍스트 `PING` 60초 권장·180초 idle 종료, `server-shutdown`·지수 backoff 지침을 채택했다. 체결은 sequence 없는 lossy 데이터이므로 누락 체결 수와 누적 거래량을 재구성하지 않는다.
  - https://openapi.tossinvest.com/openapi-docs/latest/asyncapi.json
  - https://developers.tossinvest.com/llms.txt
- GitHub/upstream: `snapview/tokio-tungstenite` 0.30은 MIT, 최근 유지보수 중이며 복합 `IntoClientRequest`로 custom Authorization header와 native TLS `wss`를 지원한다. 기존 Tauri/Tokio와 같은 비동기 런타임을 재사용하기 위해 채택했으며 외부 예제 코드는 복사하지 않았다.
  - https://github.com/snapview/tokio-tungstenite
  - https://docs.rs/tokio-tungstenite/latest/tokio_tungstenite/fn.connect_async.html
- 보안 권고: GitHub Advisory의 과거 Tungstenite handshake DoS(`GHSA-9mcr-873m-xcxp`)는 `<=0.20.0`이 영향 범위이고 `0.20.1`에서 수정됐다. 잠금 파일의 `tungstenite 0.30.0`은 해당 범위 밖이며 `native-tls`를 명시적으로 활성화해 `wss` 평문 오구성을 피한다.
  - https://github.com/advisories/GHSA-9mcr-873m-xcxp
- Kaggle 조사: 공개 고빈도 crypto orderbook 데이터는 과거 Coinbase 표본이고 토스 국장·미장 인증 전송, 공급자 ack, 현재 IP·토큰·세션과 무관하다. 런타임 검증이나 성능 근거로 채택하지 않았다.
- 검증: market topic 전용 선언, 빈·잘못된 symbol, 개인 주문 차단, 공식 프레임·PIT 체결 변환, 비밀 필드 미직렬화 단위 테스트를 추가했다. 저장 자격정보로 공식 101 handshake, 국장 trade 구독 ack와 즉시 PING/pong을 통과했다. 장중 KR/US 체결·호가, 60초 주기 장시간 PING/pong과 24시간 재연결은 실제 시간이 필요한 미완료 항목으로 유지한다.

## 실시간 스트림 REST gap 복구 검토 (2026-08-30)

- 보안 사전 검토: 입력은 닫힌 `upbit_spot·binance_spot·binance_usdm·binance_coinm` 스트림 ID만 허용한다. 임의 URL·host·심볼·자격정보를 받지 않고 공식 공개 HTTPS host, 12초 timeout, 공급자별 200·1,000봉 상한을 유지한다. 복구 결과는 `liveOrderAllowed=false`이며 주문·계좌 경로와 연결하지 않는다.
- GitHub 조사: Freqtrade의 증분 OHLCV 다운로드와 누락 범위 수집 방식을 부분 채택했다. GPL-3.0 코드나 저장 형식은 가져오지 않고, 기존 체크포인트 이후 gap만 요청한다. upstream 문서와 저장소의 유지 상태를 확인했다.
  - https://github.com/freqtrade/freqtrade
  - https://github.com/freqtrade/freqtrade/blob/develop/docs/data-download.md
- Kaggle 조사: `Crypto Datasets: 196 Pairs 1-Min Trading Data`, `Bitcoin Historical Data`, `Cryptocurrency futures OHLCV dataset (1m)`을 검토했다. 공급자 수정 이력·실시간 관측 시각·재배포 라이선스·point-in-time 정합성을 런타임에서 검증할 수 없어 모두 기각했다.
- Google/공식 문서 조사: Upbit 공식 Quotation API의 캔들 REST와 Binance 공식 Kline의 `startTime·endTime·limit` 계약을 채택했다. Upbit 무거래 분은 봉이 없을 수 있으므로 가짜 횡보 봉으로 보간하지 않는다.
  - https://docs.upbit.com/kr/kr/
  - https://developers.binance.com/en/docs/products/spot/rest-api
  - https://github.com/binance/binance-spot-api-docs
- 적용 범위: 기존 Rust PIT 공개 공급자에 OHLCV 고정소수점 복구 함수를 추가하고, 체크포인트에 이미 기록된 첫 gap의 완료 1분봉만 원자적으로 병합한다. 조회 중 스트림 상태가 바뀌거나 요청 범위 밖·미완료·중복·역순·단위 불일치 봉이 오면 실패로 닫는다.
- 검증 계획: 정상 gap 복구, 범위 밖 봉, 미래 관측 봉, 중복·역순, 체크포인트 변경, 무거래 빈 응답과 기존 백테스트 회귀를 검사한다. 토스 인증 WebSocket과 24시간 실제 내구 검사는 별도 미완료다.

## 내부 섀도우 실제 시간 내구 수집 검토 (2026-08-31)

- 보안 사전 검토: 표본은 프로세스 working set, SQLite 파일 크기, 논리 작업자 수, 내부 후보 ID, 로컬 건강·대사 boolean과 관측 시각만 포함한다. DB 내용, 계좌번호, 주문 내용, 토큰·키·세션은 직렬화하지 않는다. 실주문 경로는 없고 기존 `liveOrderAllowed=false`를 유지한다.
- Google/공식 문서: Windows의 현재 프로세스 working set은 Microsoft `GetProcessMemoryInfo`와 `PROCESS_MEMORY_COUNTERS.WorkingSetSize`로 읽는다. 임의 프로세스 ID나 외부 명령을 받지 않고 현재 Tauri Rust 프로세스 handle만 사용한다.
  - https://learn.microsoft.com/en-us/windows/win32/api/psapi/nf-psapi-getprocessmemoryinfo
  - https://learn.microsoft.com/en-us/windows/win32/psapi/working-set-information
- GitHub 조사: Tauri 공식 benchmark 결과는 운영체제별 메모리·thread 측정을 분리하며 서로 다른 OS의 절대값을 직접 비교하지 않는다. 외부 benchmark 코드는 가져오지 않고 같은 로컬 실행의 시작·종료 증가량만 판정한다.
  - https://github.com/tauri-apps/benchmark_results
- Kaggle 조사: 장시간 데스크톱 앱 메모리·원장 대사에 적용 가능한 출처·라이선스·재현성을 갖춘 데이터셋은 찾지 못했다. 금융 시계열 데이터셋은 런타임 자원 누수 검증과 무관하여 적용하지 않았다.
- 적용·검증: 새 의존성 없이 Rust 표본 명령, 1분 로컬 세션, 재시작 감지, 3분 초과 공백 fail-closed, 실제/시뮬레이션 분리와 비밀 문자열 부재 테스트를 추가했다. Windows는 working set, macOS는 Mach resident memory를 읽고 Unix 앱 데이터 권한은 `0700`으로 제한한다. 실제 24시간 결과는 아직 미완료다.

## 릴리스 준비·macOS·GitHub 보안 검토 (2026-08-31)

- 보안 사전 검토: private 저장소의 CI 로그·캐시·checkout 자격정보, 코드 서명 키, Keychain 데이터와 runner 비용을 보호 자산으로 분류했다. 자동 macOS workflow는 비용과 미검증 플랫폼 배포를 암묵적으로 확대하므로 채택하지 않고 `workflow_dispatch`만 허용했다.
- GitHub/upstream: GitHub secret scanning과 보안 정책 quickstart, Tauri의 macOS signing·GitHub pipeline, `keyring-rs`의 Apple native Keychain 지원을 확인했다. 기존 의존성으로 해결할 수 있어 새 패키지는 추가하지 않았다.
  - https://docs.github.com/en/code-security/concepts/secret-security/secret-scanning
  - https://docs.github.com/en/code-security/getting-started/quickstart-for-securing-your-repository
  - https://v2.tauri.app/distribute/sign/macos/
  - https://v2.tauri.app/distribute/pipelines/github/
  - https://github.com/open-source-cooperative/keyring-rs
- Google/공식 문서: Apple Keychain Services와 notarization 문서로 저장·배포 경계를 재확인했다. 코드 서명과 notarization은 자격정보·실기기 검증 전까지 완료로 표시하지 않는다.
  - https://developer.apple.com/documentation/security/keychain-services
  - https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution
- Kaggle 조사: 데스크톱 앱 CI, Keychain, 코드 서명, 24시간 프로세스 내구를 검증할 수 있는 관련 데이터셋은 없어 채택하지 않았다.
- 적용: GitHub action 교정, 수동 macOS compatibility workflow, Unix `0700`, macOS resident memory, 100회 회의 복구 시험, 서명 미완료 상태 문서화를 독립 구현했다.

## 화면 비의존 내부 섀도우 내구 검사 (2026-08-31)

- 채택: Rust 표준 라이브러리 `OpenOptions::create_new`. 잠금 파일을 검사한 뒤 생성하는 TOCTOU 경합 대신 원자적 생성 성공 여부로 단일 실행을 강제한다. 새 의존성은 추가하지 않았다.
- 부분 채택: Tauri upstream의 `setup`에서 복제한 `AppHandle`을 `tauri::async_runtime::spawn_blocking`으로 넘기는 패턴. UI thread를 막지 않으면서 기존 managed `PersistenceBridge`와 실제 섀도우 worker를 같은 프로세스에서 관측하는 범위만 사용한다.
- 보류: 별도 daemon·서비스 설치. 현재 수용 기준은 사용자가 명시적으로 실행한 로컬 Tauri 프로세스의 24시간 검사이며 자동 부팅·관리자 권한·외부 네트워크 노출은 필요하지 않다.
- 기각: Kaggle 검색 결과. 데스크톱 프로세스 잠금·SQLite 원장 내구 검사와 관련된 데이터셋 또는 재현 가능한 Notebook이 없어 제품 코드나 성능 근거에 적용하지 않았다.
- 보안 영향: 명시적 `--shadow-soak-autostart` 플래그에서만 시작하고, 실주문 권한과 자격정보를 읽거나 출력하지 않는다. 진행 파일은 기존 로컬 앱 데이터의 `audits` 아래에 두며 내부 후보 ID 외 계좌번호·토큰·주문 자격정보를 기록하지 않는다. 3분 초과 표본 공백, 공급자 건강 실패와 원장 대사 실패는 기존 감사기에서 fail-closed로 판정한다.

### 2026-09-01 Cloud 24시간 검수 실행 경계

- Google/공식 문서 — **채택**: Cloud Run Jobs는 종료형 백그라운드 작업과 최대 168시간 task timeout을 지원하므로 24시간 공개 스트림·격리 원장 검수에 사용한다. 실패를 숨기는 자동 재시도는 끄고 실행별 로그를 보존한다.
- GitHub — **부분 채택**: `GoogleCloudPlatform/cloud-run-samples`의 작업 컨테이너·구조화 로그 관례만 참고한다. upstream, Apache-2.0, 최근 유지보수 상태를 확인했으며 제품 코드는 복사하지 않고 Node 22 표준 API만 사용한다.
- Kaggle — **적용 가능한 결과 없음**: 장시간 런타임, Cloud Run Job 또는 원장 내구성의 운영 근거로 사용할 수 있는 공식·재현 가능한 데이터셋/노트북을 찾지 못했다.
- 보안 결정: 계좌·Telegram·Google 비밀정보와 사용자 SQLite는 컨테이너에 전달하지 않는다. Cloud `shadow-contract` 결과는 실제 Windows 앱/사용자 원장의 통과 근거가 아니며 UI와 문서에서 별도 범주로 유지한다.
- Binance 공식 변경 이력 — **채택**: USDⓈ-M 기존 WebSocket 경로가 2026-04-23 종료됐고 스트림 유형별 `/public`·`/market`·`/private` 경로가 도입됐다. mark price 내구 검사는 자격정보가 필요 없는 `/market/ws`로 이동하며 private 사용자 스트림은 이 검사에 연결하지 않는다.
- 검증: 정확한 플래그만 허용하는 단위 테스트, 중복 잠금 거부, 첫 표본과 1분 후 후속 표본 기록, 기존 24시간 합성 재생·실제 시간 자격 판정 회귀 테스트를 수행한다.
- 근거: https://doc.rust-lang.org/std/fs/struct.OpenOptions.html#method.create_new , https://github.com/tauri-apps/tauri/discussions/7596 , https://github.com/tauri-apps/tauri-docs/blob/v2/src/content/docs/develop/calling-rust.mdx

### 2026-09-02 Upbit 전송 생존·시장 이벤트 판정 분리

- 보안 사전 검토: 공개 `KRW-BTC` 시세만 사용하며 API 키·계좌·주문·출금 권한은 컨테이너에 전달하지 않는다. PING은 공급자 공식 공개 WebSocket에만 30초 간격으로 보내고 재연결 backoff를 유지해 연결 요청 남용을 막는다. 기존 24시간 실행은 취소·변경하지 않고 배포 당시 이미지와 로그를 보존한다.
- Google/공식 문서 — **채택**: Upbit는 120초 무송수신 시 idle timeout이며 30초 PING과 10초 PONG 제한을 권장한다. 실시간 데이터는 항목에 따라 정기 또는 이벤트 발생 시에만 전송되므로 `trade`의 20초 무체결을 연결 장애로 단정하지 않는다. 공식 텍스트 `PING`에 대한 `UP` 응답을 전송 생존 근거로 분리한다.
  - https://docs.upbit.com/kr/docs/websocket-best-practice
  - https://global-docs.upbit.com/reference/websocket-guide
- GitHub — **부분 채택**: `sharebook-kr/pyupbit`의 고유 ticket과 60초 ping, 자동 재연결 구조를 확인했다. 외부 패키지나 코드는 도입하지 않고 Node 22 표준 WebSocket과 Upbit 공식 텍스트 PING만 사용한다. `altangent/ccxws`의 silent reconnect 개념은 참고했지만 마지막 거래와 전송 생존을 구분하지 않는 판정은 채택하지 않았다.
  - https://github.com/sharebook-kr/pyupbit/blob/master/pyupbit/websocket_api.py
  - https://github.com/altangent/ccxws
- Kaggle — **적용 가능한 결과 없음**: WebSocket ping/pong, 이벤트 기반 체결 공백과 Cloud Run 장시간 전송 내구를 재현·검증할 공식 데이터셋이나 Notebook을 찾지 못했다.
- 적용: `investa.cloud-soak.v2`에서 `transportHeartbeats·transportTimeouts`와 `marketGapEvents`를 분리한다. Upbit 시장 공백은 경고로 보존하고 실제 전송 timeout·WebSocket 오류·메시지 미수신만 실패로 닫는다. Binance 정기 갱신 스트림은 20초 제한을 유지한다.
- 검증: UP 응답 분류, 시장 공백 중복 억제, heartbeat와 체결 수 분리, 이벤트 기반 경고, 정기 스트림 실패, 전송 timeout 실패를 결정론적 단위 테스트로 고정한다. 기존 v1 실행 종료 결과와 새 v2 실행은 실행 ID로 별도 보존한다.

## ProjectStudio 구현 상태 재감사 (2026-08-31)

- 보안 사전 검토: ProjectStudio 로컬 DB에는 비밀값을 추가하지 않고 기능 ID·설명·수용 기준·상태만 변경한다. 읽기 전용 감사 뒤 기존 API의 자동 백업과 PRD 리비전 낙관적 잠금을 사용하며, Investa·ProjectStudio SQLite 파일은 Git에 포함하지 않는다.
- GitHub 공식 문서는 상위 목표를 작은 작업으로 나누고 task list와 dependency를 사용해 진행 상태를 구분하도록 권장한다. 이를 적용해 OAuth 전송 구현과 계정 생명주기 정책, 공급자 어댑터 구현과 실제 장시간 왕복 검증을 각각 다른 수용 기준으로 분리했다.
  - https://docs.github.com/en/issues/tracking-your-work-with-issues/learning-about-issues/planning-and-tracking-work-for-your-team-or-project
  - https://docs.github.com/en/issues/planning-and-tracking-with-projects/learning-about-projects/best-practices-for-projects
- Google SRE의 launch readiness는 코드 존재만으로 준비 완료를 선언하지 않고 외부 의존성과 사람이 수행해야 하는 운영 절차를 별도 확인한다. 따라서 24시간 내구, 자격정보 왕복, 코드서명과 Cloud 공개 경계는 계속 미완료로 유지한다.
  - https://sre.google/sre-book/reliable-product-launches/
- Kaggle 공식 평가 문서는 사전에 정의한 평가 기준과 필수 제출물을 기준으로 상태를 판정한다. ProjectStudio 상태 동기화에는 데이터셋·Notebook을 적용할 부분이 없어 외부 산출물은 채택하지 않고, 명시적 수용 기준 원칙만 참고했다.
  - https://www.kaggle.com/docs/competitions
- 적용 결과: 구현과 외부 검증이 한 체크에 섞여 재작업을 유발하던 항목을 분리하고, 소셜 로그인 의미 중복을 계정 생명주기 정책으로 좁혔다. 완료 수 자체를 늘리기보다 실제 증거가 있는 세부 기준만 체크하며 24시간·KIS·Apple·Cloud·공급자 라이선스 항목은 과장하지 않는다.

## ProjectStudio 분석→내부 모의원장 골든패스 대조 (2026-08-31)

- 보안 사전 검토: ProjectStudio에는 기능 ID, 사용자 행동, 실패 분기와 검증 경로만 저장한다. 계좌번호, 포지션 원문, API 키, Telegram 식별자와 Codex 세션 내용은 기록하지 않는다. 로컬 DB는 읽기 전용 감사 뒤 자동 백업·PRD 리비전 낙관적 잠금이 있는 ProjectStudio API로만 갱신한다.
- GitHub/upstream: Temporal 공식 TypeScript 예제의 장시간 작업 heartbeat·취소·상태 보존·Saga 보상 흐름을 검토했다. 외부 런타임이나 코드는 도입하지 않고, 이미 구현된 회의 체크포인트·재개·중복 방지를 사용자 흐름의 명시적 실패 분기로 표현하는 원칙만 부분 채택했다.
  - https://github.com/temporalio/samples-typescript
- Google/공식 문서: Google Cloud Workflows의 오류 유형 분리, 멱등/비멱등 재시도 구분, 재시도 한도와 사람 승인 callback 원칙을 참고했다. Investa에서는 공급자 결측, Codex 취소, 사용자 승인, 원장 재전송을 서로 다른 분기로 표시하며 외부 Cloud Workflows는 사용하지 않는다.
  - https://cloud.google.com/workflows/docs/reference/syntax/error-types
  - https://cloud.google.com/workflows/docs/reference/syntax/retrying
  - https://cloud.google.com/workflows/docs/best-practice
- Kaggle 조사: 사용자 여정, 로컬 SQLite 원장 계보, 장시간 작업 복구와 관련해 제품 동작의 구현·검수 근거로 사용할 수 있는 데이터셋이나 Notebook은 찾지 못했다. 금융 예측 Notebook은 이 UI·오케스트레이션 상태 검증과 무관하므로 적용하지 않았다.
- 적용: 기존 `부서장 분석 회의 순환`이 모의주문 후보에서 끝나고 전 단계가 완료로 표시되던 문제를 교정한다. 종목·포지션 식별, 연결 사전점검, PIT 근거 수집, 관련 부서 선택, 역할별 분석, 본부장 종합, 분석 보관, 결정론적 위험 게이트, 사용자 승인, 내부 체결, 원장·성과 반영, 섀도우 감시를 한 레인으로 잇는다. 종목 불명확, 공급자 결측, 부분 부서 실패, Codex 복구, 위험 기각, 중복 주문·원장 대사도 분기로 둔다.
- 완료 판정: 로컬 DB에는 회의·보고·백테스트·수동 내부 체결 기록이 있으나 `engine_runs`와 `engine_order_candidates`는 0건이다. 따라서 분석 저장과 위험 게이트까지는 구현·검수 완료로 유지하되, 회의 실행 ID가 후보·승인·체결·원장·섀도우 감시로 이어지는 통합 구간과 레인 전체는 미완료로 표시한다.

## 회의 분석→내부 모의후보 인계 계보 (2026-08-31)

- 보안 사전 검토: Codex 종합 문장은 신뢰할 수 없는 분석 입력이며 주문 명령이 아니다. `paper_candidate`, 단일 종목 코드, 구체 전략, 완료된 로컬 분석 기록을 모두 확인한 뒤에도 주문을 만들지 않고 인계 레코드만 저장한다. 금융 자격정보·계좌 식별자·포지션 원문은 저장하지 않으며 `liveOrderEnabled=false`를 응답에 고정한다.
- GitHub/upstream: Temporal 공식 Approval Pattern과 human-in-the-loop 예제의 `분석/도구 결과 → 별도 승인 신호 → 실행` 분리, 승인 메타데이터 보존, 재시작 가능한 대기 상태를 부분 채택했다. Temporal 런타임과 예제 코드는 도입하지 않고 기존 SQLite append-only 엔진·원장에 맞게 독립 구현했다.
  - https://github.com/temporalio/documentation/blob/main/docs/design-patterns/approval.mdx
  - https://github.com/temporalio/samples-go/blob/main/googleadk/humanintheloop/workflow.go
- Google/공식 조사: Google Cloud Workflows callback의 사람 승인 경계와 재시도 시 멱등성 원칙은 기존 로컬 승인·대사 구조와 일치한다. 외부 Cloud Workflows는 현재 로컬 우선 제품 경계에 필요하지 않아 채택하지 않았다.
  - https://cloud.google.com/workflows/docs/creating-callback-endpoints
  - https://cloud.google.com/workflows/docs/best-practice
- Kaggle 조사: 회의 ID, 분석 ID, 결정론적 엔진 실행, 사용자 승인과 내부 SQLite 원장 계보를 검증할 수 있는 관련 데이터셋·Notebook은 찾지 못했다. 금융 예측 성과 데이터는 오케스트레이션 안전성의 근거가 아니므로 기각했다.
- 적용: `meeting_paper_handoffs`는 회의 작업 ID와 불변 분석 ID를 보존한다. 재시작·새로고침 때 후보 준비가 끝난 엔진 실행의 원본 입력을 다시 읽어 `analysisIds + symbol`이 모두 같은 경우만 자동 연결한다. 이후 후보 생성, 사용자 승인, 내부 체결, 기각과 원장 대사는 기존 엔진 상태 머신이 담당한다. 누락·불일치·차단은 상태와 사유로 표시하고 값을 꾸며내지 않는다.
- 검증: 후보가 아닌 회의 거부, 반복 인계 멱등성, 인계만으로 주문이 생기지 않음, 다른 종목 엔진 실행 거부, 같은 분석 ID·종목의 재시작 자동 연결을 단위 테스트한다. 고정 fixture에서는 분석·백테스트·후보 사건열·사용자 승인·내부 체결·append-only 원장 멱등키까지 자동 골든패스를 통과한다. 실제 공급자 시세와 사용자 클릭을 포함한 수동 검수는 별도로 남긴다.

## 회의 백테스트→섀도우 감시 연결 (2026-08-31)

- 보안 사전 검토: 회의의 자연어 전략은 주문 지시로 사용하지 않는다. 허용된 네 가지 버전형 전략 계약으로 결정론적으로 파싱되는 경우에만 탐색 백테스트를 실행하며, 불명확한 문장·선물 시장·다른 종목·다른 분석 기록·과거 실험 재사용은 실패로 닫는다. 실주문 전송은 없고 후보가 생겨도 로컬 사용자 승인이 필요하다.
- GitHub/upstream: Temporal Approval Pattern의 `결과 생성 → 내구성 있는 대기 → 별도 사람 승인` 경계를 부분 채택했다. 외부 오케스트레이터는 도입하지 않고 기존 SQLite 회의 체크포인트, 저장 백테스트, 섀도우 watch와 내부 후보 상태 머신을 재사용한다.
  - https://github.com/temporalio/documentation/blob/main/docs/design-patterns/approval.mdx
- Google/공식: Cloud Workflows의 callback·멱등성 권고처럼 회의 ID, 분석 ID, 백테스트 실험 ID와 후보 ID를 분리해 저장하고 같은 회의에 다른 실험을 덮어쓰지 못하게 했다. Cloud Workflows 자체는 로컬 우선 경계에 불필요해 도입하지 않았다.
  - https://cloud.google.com/workflows/docs/creating-callback-endpoints
  - https://cloud.google.com/workflows/docs/best-practice
- GitHub 구현 참고: QuantConnect LEAN의 백테스트 결과와 주문 이벤트 계보 분리 원칙을 검토했다. 엔진이나 전략 코드는 복사하지 않고, 과거 성과가 현재 주문이 아니며 현재 완료 봉 신호를 다시 확인한다는 경계만 적용했다.
  - https://github.com/QuantConnect/Lean
- Kaggle 조사: 로컬 회의 ID와 사용자 승인·SQLite 원장 상태 전이를 검증할 관련 데이터셋은 없었다. 금융 예측 Notebook 결과는 오케스트레이션 안전성의 증거가 아니므로 적용하지 않았다.
- 적용: `local_analysis` 근거는 제한된 `investa://analysis/<record-id>`만 허용한다. 회의 버튼은 저장 분석을 근거로 구조화 전략을 만들고 실제 공급자 완료 봉 백테스트를 저장한 뒤, 같은 분석·종목·실험인지 Rust에서 재검증한다. 검증된 KRW 주식·업비트 원화 현물은 기존 60초 섀도우 감시에 연결되고 현재 신호가 있을 때만 `paper_order_candidates`에 후보를 만든다. 현재 신호가 없으면 `watching_signal`로 남는다.
- 제한: 미국 주식은 USD 내부 모의원장의 최신 시세·후보 안전 게이트가 아직 공통화되지 않아 백테스트 뒤 자동 섀도우 후보 연결을 차단한다. 증권 선물·코인 선물은 별도 계약·증거금·펀딩 규칙이 필요해 자연어 전략 자동 변환 대상이 아니다.
- 검증: 지원/비지원 전략 파싱, 로컬 분석 URI 제한, 인계 자체로 후보가 생기지 않음, 분석 계보 불일치·실험 덮어쓰기 거부, 스키마 30→31 마이그레이션을 자동 테스트한다. 로컬 fixture의 사용자 승인·내부 원장 1회 체결은 자동 검증하며 실제 토스·업비트 왕복과 UI 클릭은 사용자 골든패스 검수로 남긴다.

## 분석→모의원장 자동 감사·외부 AI 직원 운영 (2026-09-02)

- 보안 사전 검토: 외부 AI에는 분석 문장과 허용된 근거만 보내며 금융 자격정보·계좌 식별자·주문·출금·위험정책 도구를 제공하지 않는다. 유료 호출은 사용자가 공급자를 선택해 분석 버튼을 누른 경우에만 시작한다. 작업 ID는 길이와 문자 집합을 검증하고 중복 실행을 거부하며 취소 신호는 진행 중 HTTP 요청을 중단한다. 오류 응답에는 원문 공급자 payload와 API 키를 노출하지 않는다.
- Google 공식 문서: Gemini API는 별도 API key 또는 OAuth 구성이 필요하며 데스크톱 OAuth도 Cloud project, Generative Language API와 별도 client/scopes가 필요하다. 따라서 기존 Google OIDC 로그인을 모델 호출 권한으로 재사용하지 않는다. Google AI Pro 소비자 구독은 API 인증·과금 권한으로 간주하지 않는다. 2026년 9월 표준 키 중단 안내에 따라 신규 연결은 Google AI Studio 인증키 사용을 문서화한다.
  - https://ai.google.dev/gemini-api/docs/oauth
  - https://ai.google.dev/gemini-api/docs/api-key
- GitHub/upstream: Google 공식 `googleapis/python-genai`의 Gemini Developer API·Enterprise API 구분, 명시적 client 종료와 스트리밍 계약을 검토했다. 저장소는 Rust REST 구조를 유지해 새 의존성을 추가하지 않았고, 공급자·모델·토큰 사용량 정규화와 취소 가능한 요청 수명주기만 부분 채택했다. Apache-2.0 코드는 복사하지 않았다.
  - https://github.com/googleapis/python-genai
- Kaggle 조사: 직원 오케스트레이션, OAuth/API key 권한 경계 또는 분석→append-only 원장 계보의 보안·정합성을 검증할 데이터셋이나 Notebook은 적용 가능한 결과가 없었다. 금융 예측 점수는 이 작업의 운영 안전성 근거로 사용하지 않는다.
- 적용: 개별 직원과 승인형 부서 업무에서 Codex·Claude·Antigravity를 선택할 수 있다. 외부 공급자도 동일한 서버측 `RoleReport`·`DepartmentReport` 계약을 통과하고 started·generating·validating·completed/error 상태 이벤트와 취소를 지원한다. 퀀트 논문 연구원과 전체 회의는 전용 계약 때문에 Codex 경계를 유지한다. 토큰 단위 스트리밍은 현재 구현하지 않는다.
- 골든패스 감사: 완료 회의 분석, 동일 분석 URI·종목의 백테스트, 결정론적 안전 후보, 사용자 승인 상태, 후보 ID와 같은 멱등키의 내부 원장 체결을 단계별로 읽기 전용 검사한다. 하나라도 불일치하면 failed/pending으로 닫고 `liveOrderEnabled=false`를 고정한다. 실제 공급자 호출 없이 고정 fixture로 전체 통과와 후보 미생성 대기 경로를 모두 검증한다.

## ML worker·모델 레지스트리 기반 레퍼런스

- scikit-learn `TimeSeriesSplit`의 시간순 split과 `gap` 개념을 참고했다. Investa는 다음 평가 구간이 시작되기 전에 타깃이 관측되지 않은 표본을 학습·검증 구간에서 거부한다.
  - https://scikit-learn.org/stable/modules/generated/sklearn.model_selection.TimeSeriesSplit.html
- MLflow Model Registry의 run 계보, model version, 검증 tag와 alias 분리를 참고했다. MLflow를 설치하지 않고 로컬 SQLite에 데이터·피처·코드·seed·알고리즘·아티팩트 해시를 고정한다.
  - https://mlflow.org/docs/latest/ml/model-registry/workflow/
- XGBoost 공식 Model IO 문서의 JSON/UBJSON 저장 방식을 참고해 pickle을 허용 포맷에서 제외했다. 기준 worker는 실제 JSON 모델 파일과 SHA-256 메타데이터를 함께 생성한다.
  - https://xgboost.readthedocs.io/en/stable/tutorials/saving_model.html
- ONNX의 보안 지침처럼 외부 모델과 입력을 신뢰하지 않는다. 현 단계는 실제 ONNX 실행을 하지 않으며 후속 worker에는 출처 확인, 해시 재검증, 실행 격리와 자원 제한이 필요하다.
  - https://github.com/onnx/onnx/security

- LightGBM 4.7.0 공식 PyPI wheel(MIT)과 XGBoost 3.4.1 공식 PyPI wheel(Apache-2.0)을 저장소 밖 Python 3.14 venv에 고정했다. 두 프로젝트의 학습 API와 공식 text·JSON 저장 기능만 사용하며 외부 전략 코드나 성과 수치는 가져오지 않았다.
  - https://pypi.org/project/lightgbm/4.7.0/
  - https://pypi.org/project/xgboost/3.4.1/

`investa-ml-worker-v1`은 외부 프로젝트 코드를 포함하지 않는다. 승인된 Python 모델 wheel은 격리 환경에만 설치되며 모델 성공 결과는 `candidate_review`로만 저장되고 SHADOW 또는 내부 모의운용 승격은 별도의 검증·승인 경계를 따른다.

## Cloud Run 검사 결과 수집·데스크톱 시작 복구 (2026-09-02)

- 보안 사전 검토: Google OAuth·Cloud access token과 원본 로그를 Tauri/React에 전달하지 않는다. Node 수집기는 고정 project·region·job만 shell 없이 조회하고 출력 크기·시간을 제한한다. 앱은 고정 앱 데이터 경로의 256KB 이하 캐시만 엄격한 DTO로 읽으며 다른 프로젝트, 알 수 없는 필드와 `liveOrderEnabled=true`를 거부한다. 데스크톱 실행은 기존 release 런처를 재사용하고 포트 소유 프로세스를 종료하지 않는다.
- Google/공식: Cloud Run Jobs 실행 로그가 Cloud Logging에 저장되고 구조화 stdout JSON이 `jsonPayload`로 조회되는 공식 계약과 `gcloud logging read`의 필터·limit·JSON 출력만 채택했다. 앱이 Cloud 자격정보를 직접 보유하는 방식은 기각했다.
  - https://docs.cloud.google.com/run/docs/logging
  - https://docs.cloud.google.com/sdk/gcloud/reference/run/jobs/logs/read
  - https://docs.cloud.google.com/run/docs/execute/jobs
- GitHub/upstream: Tauri 공식 예제의 `devUrl`·`beforeDevCommand`와 `frontendDist` 분리를 확인했다. 제품 코드나 별도 런처 라이브러리는 가져오지 않고 기존 `scripts/launch_investa.ps1`과 Tauri release를 재사용했다.
  - https://github.com/tauri-apps/tauri/blob/dev/examples/api/src-tauri/tauri.conf.json
- Kaggle: Cloud Run 운영 로그 수집, Tauri release 시작과 localhost 복구를 검증할 관련 데이터셋·Notebook은 찾지 못해 적용 가능한 결과 없음으로 기록한다.
- 적용: 수집기는 최신 실행의 heartbeat·완료 이벤트를 허용 목록으로 축약해 임시 파일과 이전 캐시 복구가 가능한 방식으로 교체 저장한다. 운영 화면은 진행·경고·실패·24시간 통과를 텍스트와 색상으로 함께 표시한다. `pnpm desktop:start`는 기존 증분 release 런처, `pnpm desktop:check`는 비실행 진단 경로다.
- 검증: 구조화 로그 축약에서 임의 `token` 필드가 사라지는지, 24시간 미충족·수집 불가·경고·실패를 통과로 오인하지 않는지, Rust가 미지 필드·실주문 허용 캐시를 거부하는지, release 경로가 localhost 없이 선택되는지 검사한다.

## Codex 분석 품질 프로필과 근거 종합 레퍼런스

- OpenAI Codex App Server 공식 문서의 `model/list`와 `turn/start` 계약을 채택한다. 실행 시 계정에 실제로 노출된 모델과 지원 reasoning effort를 조회하고, 분석 유형별 목표 강도가 지원되지 않으면 카탈로그 안에서만 보수적으로 낮춘다. 사용자 전역 설정이나 존재하지 않는 모델·강도를 추측하지 않는다.
  - https://developers.openai.com/codex/app-server/
- OpenAI GPT-5.6 Codex 모델 문서는 사용 가능한 reasoning effort 범위를 확인하는 근거로만 사용한다. `high` 또는 `xhigh`가 수익률·정확도를 보장한다고 해석하지 않고, 고정 평가 사례와 실제 근거 추적 실패율로 별도 검증한다.
  - https://developers.openai.com/codex/models/
- OpenAI Codex upstream 저장소와 공개 이슈를 검토했다. App Server가 제공하는 모델 카탈로그를 단일 진실 원천으로 삼고, 지원하지 않는 model·effort 조합을 보내지 않는 방어 로직만 부분 채택한다. 외부 코드는 복사하지 않는다.
  - https://github.com/openai/codex
- Fin-RATE와 SECQUE는 금융 보고에서 원문 근거 추적과 SEC 문서 기반 평가가 필요하다는 평가 설계 근거로만 참고한다. 벤치마크 점수·프롬프트·데이터를 제품 성능 주장에 사용하지 않는다.
  - https://arxiv.org/abs/2409.16626
  - https://arxiv.org/abs/2501.11754
- Kaggle의 SEC·금융 뉴스 데이터셋은 런타임 또는 품질 기준으로 채택하지 않는다. 제3자 재배포, 불명확한 라이선스, 오래된 기간, point-in-time 누수 위험이 있어 Investa의 공식 SEC 스냅샷과 고정 로컬 평가 fixture를 대체할 수 없다.

보안 경계는 기존과 동일하다. Codex 분석 세션은 읽기 전용, 네트워크 차단, 승인 불가, 주문 권한 없음이며 금융 자격정보와 계좌 식별정보를 전달하지 않는다. 최종 본부장 종합에는 잘린 요약만 보내지 않고 같은 기준 시각의 원본 근거 묶음을 다시 제공하되, 근거 ID가 카탈로그에 없는 부서 보고는 실패로 닫는다. 분석 품질 프로필은 분석과 제안의 깊이만 바꾸며 주문 권한을 넓히지 않는다.

## 국내 공식 공시·뉴스, 외부 AI 보고 계약, 계정 생명주기 (2026-09-02)

- 보안 사전 검토: OpenDART 인증키와 네이버 Client ID·Secret은 Windows 자격 증명 관리자에만 저장하고 URL·SQLite·로그·분석 보고에 넣지 않는다. 응답은 허용 DTO로 축약하며 기사 HTML 표시는 제거하되 외부 본문은 계속 신뢰하지 않는다. Claude·Antigravity에는 기존 분석 전용 경계와 유료 호출 확인을 유지하고 주문·계좌·위험정책 도구를 제공하지 않는다. 계정 해제는 인증된 소유자 세션에서만 허용하며 주 소유자 자동 이전과 지원자 우회 복구를 금지한다.
- Google/공식: OpenDART 공식 공시목록 계약과 네이버 뉴스 검색 JSON endpoint, HTTPS·헤더 인증, `display 1~100`, `start 1~1000`, `date|sim` 정렬, 일 25,000회 한도를 채택했다. 공시 원문과 뉴스 검색 결과를 투자 사실로 자동 확정하지 않고 출처·접수/게시 시각이 있는 읽기 전용 근거로만 반환한다.
  - https://opendart.fss.or.kr/guide/detail.do?apiGrpCd=DS001&apiId=2019001
  - https://developers.naver.com/docs/serviceapi/search/news/news.md
- GitHub/upstream: `awuzag/opendart`의 `corp_code`와 KRX `stock_code` 분리, HTTP/API/decode 오류 분리, opt-in live E2E와 로그에서 인증키를 숨기는 원칙을 부분 채택했다. Go 코드와 의존성은 가져오지 않고 기존 Rust `reqwest`·`keyring`으로 독립 구현했다. 소수 star의 비공식 wrapper를 성능·정확성 근거로 사용하지 않는다.
  - https://github.com/awuzag/opendart
- Kaggle 조사: 재배포된 OpenDART·네이버 뉴스 데이터셋은 버전·기사 저작권·PIT 정합성과 중복 누수 문제가 있어 런타임 근거로 채택하지 않았다. 제품 어댑터와 계정 수명주기 검증에 적용 가능한 Kaggle 데이터셋·Notebook은 없었다.
- 적용: OpenDART 공시목록과 네이버 뉴스 검색은 엄격한 길이·날짜·페이지·정렬 검증, 10초 timeout, 안전한 오류 메시지, read-only 응답을 갖는다. Claude·Antigravity 단일 응답은 Codex와 동일한 `RoleReport`·`DepartmentReport` 서버 검증기를 통과해야 한다. 연결 계정은 주 공급자를 제외한 공급자만 확인 문구 후 해제할 수 있고 로컬 분석·백테스트·모의원장은 유지한다. 전체 작업공간 삭제는 백업·재인증·명시 승인 흐름 전까지 비활성화한다.
- 미검증: 실제 OpenDART·네이버·Claude·Antigravity 키 왕복, 외부 AI 스트리밍·취소, 기사 라이선스별 장기 보존, 전체 작업공간 삭제 실행은 완료로 올리지 않는다.
