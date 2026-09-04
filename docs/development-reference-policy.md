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

### Windows Cloud CLI 수집 경로 보강 (2026-09-03)

- 보안 사전 검토: `gcloud` 인자는 고정 project·region·job과 읽기 전용 조회만 허용한다. Windows batch 실행에는 shell이 필요하므로 PATH 또는 공식 기본 설치 경로에서 확인한 절대 `gcloud.cmd`만 고정 PowerShell 래퍼에 인자 배열로 전달하고, 환경변수 override도 존재하는 절대 경로만 허용한다. 명령 문자열을 조립하지 않으며 인자의 NUL·줄바꿈과 과도한 길이를 거부한다. Cloud 응답의 execution name은 shell 필터에 삽입하지 않고 로컬에서 일치 여부를 확인한다. stderr 원문과 OAuth 자격정보는 캐시·UI에 기록하지 않는다.
- Google 공식 문서: Windows Google Cloud CLI의 공식 설치 경로·PATH 갱신 요구와 Cloud Run job 실행 목록·Cloud Logging 조회 계약을 재확인했다. 앱에 Cloud 인증을 내장하지 않고 외부 CLI 세션을 계속 사용한다.
  - https://docs.cloud.google.com/sdk/docs/install-sdk
  - https://docs.cloud.google.com/sdk/gcloud/reference/run/jobs/executions
  - https://docs.cloud.google.com/run/docs/logging
- GitHub/upstream: Node.js `child_process` 공식 문서와 upstream 원문은 Windows `.cmd`가 직접 실행 가능한 네이티브 실행 파일이 아니며 shell 경계에서 입력을 정제해야 한다고 설명한다. 별도 패키지나 코드는 도입하지 않고 기존 Node 표준 라이브러리로 독립 구현했다.
  - https://nodejs.org/api/child_process.html#spawning-bat-and-cmd-files-on-windows
  - https://github.com/nodejs/node/blob/main/doc/api/child_process.md
- Kaggle 조사: Kaggle Notebook의 gcloud 연동 안내는 노트북 비밀 저장소와 클라우드 런타임을 전제로 하므로 로컬 Windows 수집기 구현·검증에 적용하지 않았다. 관련 데이터셋이나 Notebook 기반 성능 증거는 적용 가능한 결과 없음이다.
- 적용·검증: Windows 경로 확인과 안전한 명령 생성, CLI 부재·인증·권한·비정상 JSON 오류 분류를 추가한다. Cloud Logging 조회는 job 단위의 고정 필터로 가져온 뒤 최신 execution label을 로컬에서 재검증한다. 단위 테스트와 실제 수집 재실행으로 기존 `EINVAL`이 구체적인 수집 불가 사유로 바뀌는지 확인한다.
- 호환성 판정: 이미 완료된 Cloud Run 실행을 재배포하지 않고 검증하기 위해 고정된 `investa.cloud-soak.v1`·`v2` 구조화 스키마만 허용한다. 자유 형식 로그나 알 수 없는 스키마는 성공 근거로 승격하지 않는다.
- 조기 종료 판정: Cloud Run의 `cancelledCount`와 `Completed=False, reason=Cancelled`는 런타임 오류와 분리한다. 구조화 완료 로그가 통과여도 24시간 실측이 부족하면 종합 경고로 남기고 24시간 완료로 승격하지 않는다.

### Windows 바탕화면 런처의 Node 런타임 복구 (2026-09-03)

- 보안 사전 검토: 바탕화면 바로가기는 저장소 내부의 고정 PowerShell 런처만 실행하고, 외부 입력·URL·환경변수 명령 문자열을 평가하지 않는다. 런타임 탐색은 현재 `PATH`, 공식적인 일반 Node 설치 위치와 Codex 번들 런타임의 존재하는 `node.exe`로 제한한다. 경로 자체만 진단 결과에 표시하며 환경변수 값이나 금융 자격정보는 로그에 남기지 않는다.
- Google 검색·공식 문서: Microsoft PowerShell 문서에 따라 자식 프로세스가 현재 프로세스의 환경변수를 상속하고 `PATH`가 실행 파일 탐색 위치를 결정한다는 계약을 채택했다. Node가 없는 셸에서 `pnpm.cmd`만 발견되는 상태는 실행 가능 상태가 아니므로 두 실행 파일을 함께 검증한다.
  - https://learn.microsoft.com/powershell/module/microsoft.powershell.core/about/about_environment_variables
  - https://learn.microsoft.com/powershell/module/microsoft.powershell.management/start-process
- GitHub/upstream: Tauri v2 공식 문서 저장소의 `pnpm tauri build --no-bundle` 계약을 유지한다. 별도 런처·자동 업데이트 라이브러리는 도입하지 않는다.
  - https://github.com/tauri-apps/tauri-docs/blob/v2/src/content/docs/ja/distribute/index.mdx
- Kaggle 조사: Windows 데스크톱 런처의 Node/PATH 복구를 검증할 데이터셋·Notebook은 관련성이 없어 적용 가능한 결과 없음으로 기록한다.
- 적용·검증: `pnpm.cmd`가 이미 발견되더라도 `node.exe`를 별도로 확인하고, 없으면 검증된 로컬 후보 경로를 현재 런처 프로세스의 `PATH` 앞에 추가한다. `desktop:check`는 빌드 필요 여부뿐 아니라 실제 선택된 Node·pnpm 경로와 런타임 준비 상태를 출력한다. 현재처럼 Node 없는 셸, 최신 release 재빌드, localhost 없이 실행되는 release 프로세스를 순서대로 검증한다.

### 보유종목 자연어 분석 사전 해석 (2026-09-03)

- 보안 사전 검토: “내 보유 종목” 요청은 종목 검색 실패 뒤에만 토스증권의 기존 읽기 전용 보유자산 명령으로 해석한다. 계좌번호·별칭은 분석 프롬프트에 전달하지 않고 종목코드·종목명·통화·수량·현재가·평단가만 기존 포지션 근거 DTO로 제한한다. 실제 주문·출금과 위험정책 변경 권한은 계속 제공하지 않는다.
- Google/공식: 토스증권 공식 Open API의 계좌·자산 문서가 `GET /api/v1/holdings`에서 종목별 보유내역과 평가 합계를 제공한다는 계약을 재확인했다. 기존 구현의 OAuth·읽기 전용 자격정보 경계를 그대로 사용한다.
  - https://developers.tossinvest.com/docs
- GitHub 조사: 공개 토스 API 봇 구현은 holdings 응답의 종목·수량·현재가·평단가 필드 확인에만 참고했다. 자동주문 구현이나 코드는 채택하지 않으며 공식 문서를 우선한다.
  - https://github.com/chanjoongx/toss-invest-bot
- Kaggle 조사: 일반 포트폴리오 분석 Notebook은 임의 CSV와 과거 수익률을 전제로 하므로 실계좌 읽기 전용 라우팅, PIT 근거와 계좌 식별정보 제거를 검증하는 자료로 채택하지 않았다.
- 적용·제한: 보유종목 요청에서 단일 보유종목이 확인되면 그 종목코드로 기존 단일 종목 PIT 스냅샷을 다시 생성한다. 보유종목이 없거나 둘 이상이면 임의 선택·추천을 하지 않고 각각 빈 보유상태 또는 종목 선택 필요 사유로 중단한다. 전체 포트폴리오 동시 분석과 미보유 종목 발굴은 별도 다종목 스냅샷·스크리너 계약이 필요하다.

### 전체 보유 포트폴리오 분석과 사용자 운용 원칙 (2026-09-03)

- 보안 사전 검토: 복수 보유종목 분석은 토스증권의 기존 읽기 전용 보유자산 DTO에서 종목·통화·수량·현재가·평단가만 사용한다. 계좌번호·별칭·자격정보는 Codex에 전달하지 않는다. 종목별 PIT 스냅샷은 제한된 동시성으로 만들고 부분 실패를 전체 성공으로 숨기지 않는다. 다종목 회의 결과는 특정 종목을 사용자가 다시 선택하기 전까지 백테스트·모의주문 후보로 직접 인계하지 않는다.
- 공식 근거: CFA Institute의 적합성 원칙은 포트폴리오 판단이 투자자의 명시된 목적·mandate·제약과 일치해야 하며 전체 포트폴리오 맥락에서 이뤄져야 한다고 설명한다. 따라서 기술주·테마주 집중을 앱의 임의 기본값으로 위반 처리하지 않고, 사용자가 저장한 운용 원칙과 활성화한 한도를 판정 기준으로 사용한다.
  - https://www.cfainstitute.org/standards/professionals/code-ethics-standards/standards-of-practice-iii-c
  - https://www.cfainstitute.org/insights/professional-learning/refresher-readings/2026/basics-of-portfolio-planning-and-construction
- GitHub/upstream: QuantConnect LEAN의 `MaximumSectorExposureRiskManagementModel`은 명시적으로 전달된 최대 섹터 비중을 기준으로만 축소 목표를 만든다. Apache-2.0 코드는 복사하지 않고 `측정값`과 `사용자 활성 제약`을 분리하는 계약만 부분 채택한다.
  - https://github.com/QuantConnect/Lean/blob/master/Algorithm.Framework/Risk/MaximumSectorExposureRiskManagementModel.py
- Kaggle 조사: 일반 포트폴리오 최적화 Notebook은 임의 CSV·고정 분산 선호·사후 수익률을 전제로 하며 사용자의 실제 운용 원칙, 실계좌 PIT 근거와 계정 식별정보 제거를 검증하지 못한다. 제품 판정이나 기본 집중 한도의 근거로 채택하지 않았다.
- 적용: `관측 전용`, `집중 투자`, `테마 투자`, `분산 투자`, `사용자 정의` 운용 원칙을 워크스페이스 설정에 저장한다. 종목·섹터·시장 비중은 항상 관측 사실로 표시할 수 있지만, 명시적 집중 한도 스위치가 꺼져 있으면 초과·감축·매도 판정을 만들지 않는다. 일일 손실·낙폭·실주문 잠금 같은 필수 안전 경계는 이 선택과 별도로 유지한다.
- 데이터 품질: 각 종목의 공급자·기준 시각·수집 시각·통화와 결측을 독립 보존한다. 서로 다른 통화를 환율 근거 없이 합산하지 않고, 종목별 실패와 처리 개수를 보고한다. 전체 보유종목 수가 안전 상한을 넘으면 일부만 몰래 분석하지 않고 범위 축소를 요구한다.

### 실계좌 포트폴리오 분석 재검수와 공급자 제한 복구 (2026-09-03)

- 보안 사전 검토: 실계좌 재검수는 토스증권의 읽기 전용 `holdings`·`buying-power`·완료 일봉만 사용한다. 계좌번호·별칭·accountSeq·OAuth 자격정보는 Codex 입력과 로그에서 제거한다. 매수 가능 금액은 현금 잔고와 동일하다고 과장하지 않고 공급자가 정의한 `cashBuyingPower`로 표시하며, 실주문 권한은 계속 비활성화한다.
- 토스 공식 문서: `GET /api/v1/stocks/all`은 일 배치 저변동 데이터라 하루 1회 캐시가 권장되고 `STOCK_ALL` 호출 제한을 따른다. `GET /api/v1/buying-power`는 계좌 헤더와 KRW 또는 USD 통화를 받아 미수 제외 현금 기반 매수 가능 금액을 반환한다. 따라서 계좌 스냅샷에 이미 포함된 종목코드가 있는 보유분석에서는 전체 종목 목록을 재조회하지 않고, 통화별 매수 가능 금액은 별도 읽기 전용 근거로 보존한다.
  - https://developers.tossinvest.com/docs
  - https://openapi.tossinvest.com/openapi-docs/latest/openapi.json
- GitHub/upstream: WorkOS Rust SDK의 안전한 GET 재시도 구현은 429에서 `Retry-After`를 우선하고 지수 backoff·jitter와 최대 시도 수를 제한한다. 외부 코드는 복사하거나 새 의존성을 추가하지 않았다. Investa는 공급자 호출을 줄이는 구조화 보유종목 경로와 기존 라이브러리만 사용한 전역 `STOCK_ALL` 1초 간격 예약을 채택했다.
  - https://github.com/workos/workos-rust
- Kaggle 조사: 공개 S&P 500 historical holdings 데이터는 과거 구성종목을 보존해 생존편향을 피해야 한다는 데이터 품질 사례로만 검토했다. 실계좌 현재 보유분, 토스 호출 제한 또는 매수 가능 금액 검증에는 적용 가능한 Notebook·모델이 없어 런타임 의존성으로 채택하지 않았다.
- 적용·검증: 계좌 보유분은 종목코드·종목명·시장·통화 구조체로 Rust에 전달해 영문이 섞인 국내 종목코드도 전체 카탈로그 우회 없이 검증한다. 보유종목별 스냅샷은 순차 수집하고 부분 실패를 보존한다. 완료 일봉이 20개 미만인 신규·파생 종목은 가격과 포지션을 버리지 않고 기술지표만 `insufficient_history`로 표시한다. 긴 회의 안건은 전문을 분석 본문에 그대로 남기되 저장 목록 제목만 240자로 축약한다. 실계좌 검사는 금액 원문을 출력하지 않고 KRW·USD 응답 존재, 마스킹, 읽기 전용 경계만 확인한다.

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

## 토스 시장 랭킹 기반 다단계 후보 탐색 (2026-09-03)

- 보안 사전 검토: 저장된 토스 Client ID·Secret은 기존 Windows 자격 증명 관리자와 Rust 내부 OAuth 경계 밖으로 전달하지 않는다. 랭킹·활성 종목·완료 일봉만 읽고 계좌·주문 API를 호출하지 않는다. 요청 시장·preset·후보 수는 enum과 20종목 상한으로 제한하며 공급자 오류 원문, 응답 헤더와 토큰을 UI·로그에 노출하지 않는다. 결과의 `liveOrderEnabled`는 항상 false여야 하며 후보 선택은 회의 안건 작성까지만 연결한다.
- 토스 공식 문서: `GET /api/v1/rankings`가 시장 전체 거래대금·거래량·급등락 상위 100개와 집계 시각을 제공하고, `GET /api/v1/prices`가 콤마 구분 최대 200종목을 지원하는 계약을 확인했다. 랭킹은 별도 `RANKING`, 일봉은 `MARKET_DATA_CHART` 호출 그룹이므로 호출 간격을 제한하고 모든 활성 종목의 일봉을 전수 요청하지 않는다.
  - https://developers.tossinvest.com/docs
  - https://developers.tossinvest.com/docs/market-data
  - https://openapi.tossinvest.com/openapi-docs/latest/openapi.json
- GitHub/upstream: QuantConnect LEAN의 coarse/fine universe selection은 가격·거래량 같은 저비용 데이터로 넓은 유니버스를 줄인 뒤 제한된 후보에만 상세 데이터를 요청한다. Apache-2.0 코드를 복사하지 않고 다단계 예산 계약과 결정론적 제외 사유만 채택했다.
  - https://github.com/QuantConnect/Lean/blob/master/Algorithm.CSharp/RawPricesCoarseUniverseAlgorithm.cs
- Kaggle 조사: 현재 구성 종목만 포함한 공개 주식 데이터셋은 생존편향과 미래정보 누수 한계를 스스로 명시하는 사례가 확인됐다. 정적 Kaggle CSV·Notebook은 실시간 후보 공급자나 성능 근거로 채택하지 않는다.
  - https://www.kaggle.com/datasets/baidalinadilzhan/advanced-stock-dataset
- 적용: 국장·미장별 시장 거래대금·거래량과 preset에 맞는 급등·급락 랭킹을 합쳐 1차 점수를 만든다. 상위 12종목에만 완료된 수정주가 일봉 80개를 요청하고 기존 결정론적 스크리너로 20일 수익률·20일 평균 거래량·RSI·20일선 조건을 검증한다. `균형`, `추세`, `반전 관찰`은 명시적으로 다른 규칙 버전을 사용하며 조건 미달을 자동 완화하지 않는다.
- 데이터 품질: 랭킹 집계 시각, 앱 관측 시각, 1차 유니버스 수, 기술 검토 수, 조건 제외와 종목별 부분 실패를 분리한다. 현재 활성 종목과 당일 랭킹이므로 과거 시점 유니버스나 백테스트로 표시하지 않는다. 후보 탐색 단계에서는 호가 스프레드를 명시적으로 유예하며 주문 적격성·장 상태·실제 호가는 이후 별도 게이트에서 확인한다. 테마·업종 집중은 사용자가 한도를 켠 경우에만 위반 판단한다.

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

## 실계좌 보유자산 시각화와 Telegram 근거 자동 연결 (2026-09-03)

- 보안 사전 검토: 실계좌 화면과 분석 기록에는 읽기 전용 종목코드·종목명·통화·수량·현재가·평단가와 통화별 매수 가능 금액만 사용한다. 분석 기록에는 계좌 별칭·마스킹 번호도 저장하지 않으며, 서로 다른 통화는 환율 근거 없이 합산하지 않는다. Telegram 동기화는 사용자가 이미 승인하고 선택한 채널의 읽기만 수행하고 메시지 안의 명령·링크는 신뢰하지 않는다. 동기화 실패는 분석 실패로 숨기거나 전체 분석을 중단하지 않고 근거 공백으로 기록한다.
- Google/공식: Google Charts 문서는 도넛 차트를 하나의 데이터 계열에서 전체 대비 부분을 비교하는 표현으로 정의하고 0·음수 값을 조각으로 표시하지 않는다고 명시한다. 따라서 통화별 양수 평가액만 비중으로 계산하고, 작은 종목은 `기타`로 묶되 상세 표에서 모두 확인할 수 있게 한다. Google 검색 결과 자체는 근거로 쓰지 않았다.
  - https://developers.google.com/chart/interactive/docs/gallery/piechart
- Telegram 공식: MTProto 검색 계약은 query와 날짜 범위를 별도로 받는다. Investa는 원격 검색 결과를 바로 보고에 넣지 않고, 사용자가 선택한 채널을 로컬 리비전 저장소에 먼저 동기화한 뒤 분석 요청 시작 시각 이하의 메시지만 근거 후보로 조회한다.
  - https://core.telegram.org/method/messages.search
- GitHub/upstream: OpenStocky는 보유종목 비중 도넛에서 상위 종목과 `Other`를 분리하고 상세 보유내역을 함께 제공하는 정보 구조만 참고했다. 프로젝트 코드와 의존성은 가져오지 않는다. `react-minimal-pie-chart`도 검토했으나 단일 화면을 위해 새 패키지를 추가하지 않고 기존 React·CSS로 구현한다.
  - https://github.com/marketcalls/openstocky
  - https://github.com/toomuchdesign/react-minimal-pie-chart
- Kaggle 조사: 공개 포트폴리오 배분 Notebook은 임의 CSV와 사후 수익률을 사용해 현재 실계좌의 시점 정합성·통화 분리·계정 비식별화를 보증하지 못한다. 런타임 데이터나 성과 근거로 채택하지 않는다.
- 적용: 원장 첫 화면에 `실계좌 보유자산 · 읽기 전용`을 먼저 표시하고 내부 모의원장은 별도 출처 탭으로 유지한다. 계좌 분석 기록에도 당시의 익명화 보유자산 스냅샷을 불변 기록으로 저장해 통화별 평가액, 손익, 비중과 매수 가능 금액을 재현한다. 업종·배당 데이터가 공급되지 않으면 추정 그래프를 만들지 않고 미제공 사실을 표시한다.
- Telegram 장애 판정: `연결됨`, `선택 채널 존재`, `이번 동기화 성공`, `기준 시각 범위의 저장 메시지 존재`, `현재 안건에 포함된 근거 수`를 서로 다른 상태로 보고한다. 분석 시작 시 선택 채널을 동기화하고 완료 일봉 시각이 아니라 분석 요청 시작 시각을 뉴스 근거 cutoff로 사용한다.

### 리서치부 직원별 Agent v2와 학술 메타데이터 브로커 (2026-09-03)

- 보안 사전 검토: Codex 직원에게 네트워크·파일 수정·명령·주문 권한을 직접 주지 않는다. 외부 조회는 Rust가 고정 HTTPS 호스트, 8초 timeout, 최대 5건, redirect 금지로 수행하고 결과를 신뢰할 수 없는 입력 구간으로 격리한다. Crossref 결과는 서지 메타데이터 후보일 뿐 논문 원문 검증·전략 수익·재현 성공으로 승격하지 않는다. 계좌 식별자와 금융 자격정보는 전달하지 않으며 `SHADOW ONLY`를 유지한다.
- GitHub/upstream: OpenAI Agents SDK의 manager/handoff, specialist-as-tool, 구조화 도구와 tool guardrail 원칙을 구조 참고로 채택했다. SDK나 코드는 도입하지 않고 기존 Codex App Server, Rust 계약 검증과 React 오케스트레이션을 재사용한다. FinRobot은 금융 전문 Agent가 데이터 수집과 역할별 보고를 분리하는 사례로만 검토했으며 외부 API 키·자동 거래·프레임워크 코드는 가져오지 않았다.
  - https://github.com/openai/openai-agents-python
  - https://github.com/AI4Finance-Foundation/FinRobot
- Google 검색·공식: Crossref 공개 REST API의 `/works` 서지 메타데이터 검색 계약을 채택했다. 원문 저작권과 전략 규칙은 메타데이터만으로 확인할 수 없으므로 DOI·제목·저자·발행일·학술지·인용 메타데이터만 제공한다.
  - https://www.crossref.org/documentation/retrieve-metadata/rest-api/
- Kaggle 조사: 다중 Agent 금융 Notebook은 임의 데이터·API key·성과 예시를 사용하며 Investa의 역할 계약, PIT 근거, 주문 금지와 직원별 실행 무결성을 증명하지 못해 적용 가능한 결과 없음으로 기록한다.
- 적용: 전체 회의의 리서치부는 기술·펀더멘털·뉴스·거시·논문 연구원에게 각각 별도 `RoleReport` turn을 배정한다. 리서치 총괄은 직원별 실제 성공·실패 결과만 받아 `DepartmentReport`로 취합한다. 다른 부서는 회귀 위험을 줄이기 위해 기존 `department_batch_v1`을 유지한다. 논문 요청은 Crossref 후보를 자동 첨부하고 `crossref-paper-N` 근거 ID를 직원→부장→본부장까지 추적한다.
- 도구 선택 적용: 각 리서치 직원은 최종 보고 전에 구조화된 `AgentToolPlan`을 만들고 역할별 읽기 전용 허용 목록에서 최대 3개만 선택한다. Rust가 계획과 두 번째 보고 turn의 `approvedToolIds`를 각각 검증하고, React 브로커도 동일 카탈로그를 다시 확인한 뒤 선택한 결과만 `RoleReport`에 전달한다. 기술·재무·공시·Telegram·시장 레짐·Crossref·공개 GitHub 메타데이터 외 임의 URL, 파일·명령, 자격정보와 주문 도구는 제공하지 않는다. 도구 계획과 실행 상태는 분석 노트에 trace로 보존한다.
- Telegram 오프라인 경계: 인터넷이 없으면 새 Telegram 메시지를 동기화할 수 없다. 이전에 Investa SQLite에 저장된 선택 채널 메시지만 `cached_offline`으로 제공하고 마지막 관측 시각과 동기화 실패를 함께 표시한다. Telegram 데스크톱·웹 앱의 자체 캐시는 읽지 않으며, 저장 근거를 최신 뉴스라고 표현하지 않는다.
- 호출 예산: 리서치부는 직원별 도구 계획 5회, 직원 보고 5회와 부장 취합 1회를 실제 비용으로 계산한다. 분류·본부장 종합 2회를 포함한 중요 안건 상한은 13회이며, 예산을 넘는 후순위 부서는 생략 사실을 UI에 표시한다.
- 당시 남은 범위였던 전략·리스크와 다른 5개 전문 부서 전환은 2026-09-04 완료했다. 실제 Codex App Server에서는 한화 기업행위 안건으로 리서치·리스크 직원 10명, 두 부서장과 본부장까지 계획→도구→보고 E2E를 통과했다. 나머지 6개 전문 부서와 장시간 반복은 계속 남아 있다. 현재도 모든 직원에게 범용 웹 브라우저나 임의 API 호출 권한을 주지 않는다.

### 국내 기업행위 조사 Agent 회귀 수정 (2026-09-03)

- 보안 사전 검토: 직원 Codex에는 계속 네트워크를 열지 않고, Rust 브로커만 `dart.fss.or.kr`, `opendart.fss.or.kr`, `openapi.naver.com`의 고정 HTTPS endpoint를 호출한다. 종목코드는 숫자 6자리, DART 고유번호는 숫자 8자리, 응답은 1MiB, timeout은 10초로 제한하며 자격정보는 Windows 자격 증명 관리자 밖으로 전달하지 않는다. 공시·기사 본문은 신뢰할 수 없는 외부 입력으로 취급하고 파일·명령·주문 도구는 제공하지 않는다.
- OpenAI 공식: 구조화 출력의 중첩 객체도 모든 필드를 required로 선언하고 `additionalProperties: false`를 적용해야 한다. 빈 배열 전용 스키마에 느슨한 객체 item을 두어 `invalid_json_schema`가 발생한 회귀를 scalar item과 `maxItems: 0` 조합으로 수정했다.
  - https://developers.openai.com/api/reference/ruby#structured-outputs-and-function-calling
- OpenDART 공식: 공시검색은 8자리 `corp_code`를 선택적으로 사용하며, 회사 고유번호 자료는 6자리 KRX 종목코드와 별도다. 제품은 DART 공식 회사검색의 정확한 종목코드 결과에서 고유번호를 해석한 뒤 기존 OpenDART 공시검색을 호출한다.
  - https://opendart.fss.or.kr/guide/detail.do?apiGrpCd=DS001&apiId=2019018
  - https://opendart.fss.or.kr/guide/detail.do?apiGrpCd=DS001&apiId=2019001
- GitHub 조사: `dart-fin-utils`, `disclosures`의 종목코드→고유번호 선행 해석, 실패를 구조화 결과로 반환하고 자격정보를 로그에서 제외하는 패턴을 참고했다. 외부 패키지와 코드는 가져오지 않고 기존 Rust `reqwest`와 React 도구 브로커만 사용했다.
  - https://github.com/shinkangteam/dart-fin-utils
  - https://github.com/carrotly-ai/disclosures
- Kaggle 조사: 기업행위 최신 공시·뉴스의 기준 시각, 정정 공시와 기사 중복을 보장하는 적합한 데이터셋이 없어 런타임 근거로 채택하지 않았다.
- 적용: 펀더멘털 직원의 `analysis.fundamentals_filings`와 뉴스 직원의 `analysis.disclosure_news`가 국내 단일 종목에서 OpenDART를 실제 호출한다. 뉴스 직원은 설정된 경우 네이버 뉴스도 함께 검색한다. 공급자별 실패는 해당 항목만 `unavailable`로 기록하며 가격·보유수량 등 다른 근거를 지우지 않는다. 다종목 요청은 비용·응답 길이를 통제하기 위해 한 번에 최대 3종목만 외부 조회하고 생략 수를 명시한다.
- 실제 검수와 실패 처리: 한화(000880) 기업행위 안건에서 뉴스 직원이 `analysis.disclosure_news`와 `analysis.telegram_news`를 선택해 이전 `invalid_json_schema`가 제거된 것을 확인했다. 후속 역할 보고 한 건이 7분 이상 종료되지 않아 회의가 정지한 사례는 직원별 도구 선택·역할 보고 각 3분 timeout과 실제 `codex_cancel_turn`으로 fail-closed 처리한다. 초과 직원은 성공으로 표시하지 않고 근거 공백으로 남기며 다음 직원과 부서 취합을 계속한다. 현재 PC에는 OpenDART·네이버 뉴스 자격정보가 저장되지 않아 공급자 본문 왕복은 미검증으로 유지한다.
- 전체 회의 종료 보장: Codex App Server 공식 계약에서 `turn/interrupt` 성공 뒤 `turn/completed`가 `interrupted` 상태로 도착하는 흐름을 채택했다. OpenAI Agents SDK upstream은 무기한 작업을 외부 timeout으로 감싸고 진행 중 작업을 취소하는 회귀검사를 제공하며, Investa는 새 SDK를 도입하지 않고 기존 `codex_cancel_turn`을 안건 분류·직원 계획·역할 보고·부서 취합·본부장 종합의 각 3분 감시에 재사용한다. 각 단계는 재시도 없이 실패로 닫고 완료 이벤트에서 타이머를 제거한다. 도구 계획의 완료 이벤트가 다음 역할 보고 타이머를 지우지 않도록 두 turn의 수명주기를 별도로 처리한다. Kaggle에는 로컬 App Server turn 수명주기와 취소 계약을 검증할 적합한 데이터셋·Notebook이 없어 적용 가능한 결과 없음으로 기록한다.
  - https://developers.openai.com/codex/app-server/
  - https://github.com/openai/openai-agents-python/blob/main/tests/test_guardrails.py

### 전략운용·리스크관리 직원별 Agent 전환 (2026-09-04)

- 보안 사전 검토: 전략·리스크 직원에게 네트워크·파일·명령·주문·정책 변경 권한을 주지 않는다. `analysis.position_portfolio`는 기존 토스 읽기 전용 스냅샷에서 계좌 식별자를 제거한 종목·통화·수량·가격·현금과 사용자가 저장한 운용 원칙만 제공한다. 통화 간 금액을 환율 근거 없이 합산하지 않고, 직원별 도구는 최대 3개로 제한하며 Rust와 React가 호출 전후 동일 allowlist를 검증한다.
- OpenAI 공식: manager-style orchestration은 한 관리자가 여러 전문 Agent의 제한된 결과를 결합하고 공통 guardrail을 소유하는 경우에 적합하다. 각 Agent는 전문 역할·도구·구조화 출력으로 한정하고 도구 guardrail은 호출 전후에 적용해야 한다는 원칙을 채택했다. SDK나 외부 런타임은 추가하지 않고 기존 Codex App Server와 로컬 브로커를 재사용한다.
  - https://openai.github.io/openai-agents-python/multi_agent/
  - https://openai.github.io/openai-agents-python/guardrails/
- GitHub/upstream: `openai-agents-python`의 agents-as-tools, 구조화 specialist, 실행 trace와 guardrail 패턴을 설계 근거로만 사용했다. 외부 코드를 복사하거나 패키지를 설치하지 않았다.
  - https://github.com/openai/openai-agents-python
  - https://github.com/openai/openai-agents-python/blob/main/examples/agent_patterns/README.md
- Kaggle 조사: 공개 multi-agent 금융 Notebook은 임의 데이터·API key·사후 성과 예시를 사용해 Investa의 PIT 근거, 계좌 비식별화, 역할별 권한과 호출 예산을 검증하지 못하므로 적용 가능한 결과 없음으로 기록한다.
- 적용: 전략운용부 4명과 리스크관리부 5명을 직원별 `AgentToolPlan → 로컬 읽기 전용 브로커 → RoleReport → 부서장 DepartmentReport` 경로로 전환했다. 직원별 비용은 2회, 부장 취합은 1회로 계산한다. 복합 회의는 먼저 모든 소집 부서의 1회 배치 보고 비용을 확보하고 남는 13회 예산에서만 가능한 부서를 직원별 실행으로 승격한다. 예산 부족을 이유로 필수 리스크·운영 부서 자체를 제거하지 않는다.

### 운영·디지털자산·홍보·투자공학·준법 직원별 Agent 전환 (2026-09-04)

- 보안 사전 검토: 남은 직원에게도 네트워크·파일·명령·게시·정책 변경·킬 스위치·주문 권한을 직접 주지 않는다. 운영 도구는 기존 Tauri 읽기 전용 명령에서 계좌 ID, 감사 상세 원문, 자격정보와 공급자 오류 원문을 제거한 요약만 반환한다. 내부 모의원장과 실제 계좌를 구분하고 모든 결과에 `liveOrderEnabled=false` 및 `SHADOW ONLY` 경계를 유지한다.
- OpenAI 공식: manager가 최종 응답과 공통 guardrail을 소유하고 전문 Agent를 제한된 도구처럼 호출하는 manager-style 구조, 함수 도구 호출 전후 검증, timeout·동시성·승인 경계를 채택했다. 외부 SDK나 런타임은 추가하지 않고 기존 Codex App Server와 Rust·React 이중 allowlist를 유지한다.
  - https://openai.github.io/openai-agents-python/multi_agent/
  - https://openai.github.io/openai-agents-python/tools/
  - https://openai.github.io/openai-agents-python/guardrails/
- GitHub/upstream: `openai/openai-agents-python`의 deterministic flow, agents-as-tools, 구조화 입력과 human-in-the-loop 예제를 설계 근거로만 사용한다. 코드를 복사하거나 패키지를 설치하지 않고 현재 `AgentToolPlan → 로컬 도구 → RoleReport → DepartmentReport` 계약을 확장한다.
  - https://github.com/openai/openai-agents-python/blob/main/docs/examples.md
  - https://github.com/openai/openai-agents-python/blob/main/examples/agent_patterns/README.md
- Google 검색·공식: 검색은 발견 수단으로만 사용했으며, 고위험 AI 운영의 추적성·책임·위험 측정에는 NIST AI RMF 생성형 AI 프로파일을 보조 근거로 확인했다. 법규 준수 완료나 투자 적합성 인증으로 해석하지 않는다.
  - https://nvlpubs.nist.gov/nistpubs/ai/NIST.AI.600-1.pdf
- Kaggle 조사: 공개 multi-agent 금융 Notebook은 제품의 로컬 감사 원장, 재시작 대사, 역할별 권한, 게시 금지와 PIT 근거 계약을 재현하지 못하므로 런타임 데이터·성능 근거·코드로 채택하지 않는다. 적용 가능한 결과 없음.
- 채택 범위: `operations.runtime_snapshot`, `operations.paper_ledger_snapshot`, `operations.audit_snapshot`, `analysis.evidence_manifest` 네 읽기 전용 도구만 추가한다. 운영 상태는 요약·건수·관측 시각만, 모의원장은 익명화 통화·현금·손익·포지션 요약만, 감사 도구는 action·시각만, 근거 manifest는 제공자·결측·근거 ID 목록만 전달한다.
- 검증 방법: 모든 직원의 허용 도구를 Rust와 TypeScript에서 동일하게 검증하고, 미등록 도구·중복·3개 초과·권한 초과를 거부한다. 호출 예산은 모든 소집 부서의 배치 보고를 먼저 보존한 뒤 직원형 승격 비용만 추가하며, 실패한 도구는 해당 직원의 근거 공백으로 닫는다.

### 복합 회의 전 직원 독립 실행·기업분할 공식 근거·신뢰도 보정 (2026-09-04)

- 보안 사전 검토: 호출 상한 확대는 Codex 사용량과 장시간 작업을 늘리지만 직원에게 네트워크·파일·명령·주문 권한을 추가하지 않는다. 외부 조회는 기존 Rust 고정 호스트·10초 timeout·1MiB 응답 제한과 Windows 자격 증명 관리자를 유지한다. OpenDART 키, 계좌 식별자와 공급자 오류 원문은 Agent 입력·SQLite·로그에 넣지 않는다. KRX 시장정보의 무단 재배포·비공식 scraping은 도입하지 않는다.
- OpenAI 공식 — **채택**: manager 방식은 전문 Agent가 한정된 하위 업무를 실제 수행하고 관리자가 구조화 결과를 종합할 때 적합하며, 코드 오케스트레이션은 실행 순서·동시성·비용을 결정론적으로 통제할 수 있다. 따라서 부서장이 직원 이름의 결과를 대신 작성하는 배치 프롬프트를 제거하고, 소집된 8개 전문 부서는 직원별 `AgentToolPlan → RoleReport`를 실제 실행한 뒤에만 부서 보고를 만든다.
  - https://openai.github.io/openai-agents-python/multi_agent/
  - https://github.com/openai/openai-agents-python/blob/main/docs/running_agents.md
- OpenDART 공식 — **채택**: 일반 공시목록 외에 `회사분할 결정(cmpDvDecsn.json)`이 분할방법·비율·존속/신설회사·재상장 신청·신주배정 기준일·신주 상장 예정일·거래정지 예정기간을 구조화해 제공한다. 기업행위 키워드가 있는 국내 종목 안건에서 이 읽기 전용 endpoint를 추가 조회하고 접수번호 기반 근거 ID로 추적한다. 공시 추출값은 정확성·완전성이 보장되지 않으므로 원 공시 링크와 결측을 함께 보존한다.
  - https://opendart.fss.or.kr/guide/detail.do?apiGrpCd=DS005&apiId=2020051
  - https://opendart.fss.or.kr/guide/detail.do?apiGrpCd=DS001&apiId=2019001
- KRX 공식 — **기각/보류**: KRX 시장정보 이용정책은 계약·승인 없는 데이터피드와 재배포를 제한한다. 공개 문서로 확인되지 않은 내부 endpoint나 HTML scraping을 공식 기업행위 근거로 추가하지 않는다. 현 단계의 공식 기업분할 사실은 OpenDART 원 공시로 제공하고, KRX 상장·거래 시점의 별도 구조화 피드는 계약 가능한 공급자를 선정한 뒤 연결한다.
  - https://data.krx.co.kr/inc/datasale/Market%20Data%20Usage%20Polices_ko.pdf
- GitHub/upstream — **부분 채택**: `openai/openai-agents-python`의 specialist-as-tool, 구조화 출력, 최대 turn·timeout과 실행 메타데이터 원칙만 설계 근거로 사용한다. 새 SDK나 코드는 도입하지 않고 기존 Codex App Server·Rust 검증·2명 동시 실행 큐를 유지한다.
  - https://github.com/openai/openai-agents-python
- Kaggle — **적용 가능한 결과 없음**: 공개 금융 multi-agent Notebook과 기업행위 데이터셋은 Investa의 실제 직원 turn, OpenDART PIT 접수 시각, 정정 공시, 역할 권한과 근거 ID 계보를 검증하지 못한다. 런타임 근거나 성능 주장에 사용하지 않는다.
- 신뢰도 정책: `confidencePercent`는 수익 확률이 아니라 근거 충족도다. 국내 기업분할·신주·재상장처럼 공식 공시가 핵심인 안건에서 허용된 `opendart-corporate-action-*` 근거가 하나도 없으면 역할·부서 보고의 근거 충족도를 결정론적으로 35% 이하로 제한하고, 공식 공시 연결 또는 재조회 조건을 명시한다. LLM이 제시한 높은 숫자를 그대로 노출하지 않는다.
- 호출 예산: 본부장 분류 1회, 35명 전원의 계획·보고 70회, 8개 부서장 취합 8회, 본부장 종합 1회를 합친 최대 80회를 중요 안건 상한으로 둔다. 실제 라우팅된 부서 비용만 계획에 표시하고 동시 실행은 2명, 사용량 80% 중단선을 유지한다. 실제 한화 기업행위 E2E에서 high reasoning 직원 보고와 부서 취합이 3분을 넘겨 실패한 결과를 반영해 역할 보고·부서 취합·최종 종합은 5분, 분류·도구 선택은 3분으로 분리했다. 직원 결측은 부서장 입력 전에 500자 계약 상한으로 제한한다. 부서장은 새 근거를 조사하지 않고 직원의 구조화 결과만 취합하므로 `medium` 추론을 사용하고, 역할·판단·반대근거·근거 공백·근거 ID를 계약 상한에 맞춰 결정론적으로 축약한 입력만 받는다.
- 2026-09-04 한화 기업행위 재검수에서 웹 연구원은 KRX·한화 공식 자료와 논문 근거 10개를 수집했지만, 오래 재사용된 리서치부장 thread가 5분 동안 첫 응답을 만들지 못해 유효한 직원 근거가 최종 보고에서 누락됐다. OpenAI Codex upstream에는 오래되거나 context-full인 App Server thread의 resume·무응답·지연 사례가 공개돼 있다. 따라서 안건 분류, 부서장 취합, 본부장 종합은 이전 회의 문맥을 재사용하지 않는 새 read-only thread에서 실행하고, 직원의 도구 선택→역할 보고 두 단계만 같은 thread 문맥을 유지한다. GitHub upstream 사례는 부분 채택했으며 Kaggle에는 이 App Server 수명주기 문제에 적용할 자료가 없어 적용 가능한 결과 없음으로 기록한다.
- 새 thread 적용 후 리서치부 취합은 약 45초 만에 끝났지만, 실제 세션의 최종 JSON은 유효한데 델타 누적 버퍼가 간헐적으로 계약 오류가 되는 별도 전송 결함을 실측했다. 설치된 Codex App Server 0.149.0의 생성 TypeScript protocol에서 `ItemCompletedNotification.item`과 `TurnCompletedNotification.turn.items`의 `agentMessage.text`가 완료 정본임을 확인했다. 따라서 완료 메시지로 구조화 버퍼를 덮어쓰고, 임시 생성 protocol 파일은 제품 저장소에 남기지 않는 방식을 채택했다.
- 실제 E2E 회귀 대응: 직원이 전달받지 않은 `mv-000880-*` 근거 ID를 생성한 사례를 Rust/프런트 사후 검증이 거부한 데 더해, 두 번째 보고 프롬프트에 실제 도구 evidenceId 허용 배열을 명시하고 배열 밖 ID 생성을 금지했다. 본부장 종합 JSON이 저장됐는데 이동 애니메이션 상태 때문에 UI가 `department-analysis`에 머문 사례는 종합 이벤트 수신 즉시 `reconvening → results` 체크포인트로 전환하도록 수정했다. 자동 전문 부서 소집으로 메모리 정책이 `important`가 됐지만 DB가 `normal`로 남은 사례도 체크포인트에서 유효 중요도를 함께 영속화하도록 수정했다.
- 2026-09-04 한화 기업행사 실사용 회귀에서 웹 조사 직원의 `codex-web-*` ID는 부서장까지 전달됐지만 URL·관측 문구·관측 시각이 탈락해 본부장이 공식 근거를 다시 부재로 판정했다. 직원 계약을 통과한 근거 메타데이터를 제어문자 제거, 항목 수와 필드 길이 상한 뒤 부서장·본부장 입력에 함께 제공하도록 수정했다. 외부 본문은 계속 비신뢰 데이터로 표시하며 지시 실행, 임의 도구 사용과 주문 권한은 열지 않는다.
- 최종 실사용 검수: 동일한 한화(000880) 인적분할·신주 안건을 다시 실행해 리서치·전략·리스크 직원 보고, 세 부서장 취합과 본부장 `MeetingSynthesis`가 제한시간 안에 완료됐다. 한화 공식 공지 URL, 관측 문구와 `2026-09-04 12:52:57 KST` 관측 시각이 리서치부와 최종 종합까지 보존됐으며, 다른 뉴스·공시 공급자 결측을 전체 공식 근거 부재로 확대하지 않았다. 공지에서 예정한 2026-08-25 재상장·신규상장의 실제 완료, 정정 내역, 신종목 코드, 권리 원장과 분리 재무는 후속 공식 근거가 없어 본부장은 `hold`, 백테스트는 `required=false`로 판정했다. 실주문과 새 모의주문 후보는 생성하지 않았다.

### 사용자 요청형 Codex 웹 조사 근거 (2026-09-04)

- 보안 사전 검토: 일반 직원 Codex 세션은 계속 `readOnly`, shell `networkAccess=false`, 웹 검색 비활성으로 고정한다. 웹 조사를 선택한 논문 연구원만 별도 대화 키와 빈 임시 작업공간을 사용하며, Codex의 호스팅 웹 검색만 `live`로 연다. 금융·Telegram·Cloud 자격정보는 자식 프로세스 환경에 전달하지 않고 파일·명령·주문·로그인·게시 도구는 제공하지 않는다. 검색 결과는 외부의 신뢰할 수 없는 입력이며 웹 페이지의 지시를 실행하지 않는다.
- OpenAI 공식 — **채택**: Responses/Codex 모델은 웹 검색을 내장 도구로 제공하며, App Server는 `thread/start`의 격리된 설정과 turn 이벤트를 지원한다. Investa 기본 번들 Codex App Server `0.149.0`과 현재 데스크톱 설치본 `0.151.0-alpha.7.2`의 생성 프로토콜 스키마에서 `web_search=live`, 웹 검색 항목과 `readOnly + networkAccess=false` 조합을 확인했다. 기본 번들 `0.149.0`에서도 실제 canary로 웹 검색 항목과 HTTPS 응답을 확인했다. API key 기반 Responses 웹 검색은 별도 호출 사용량이므로 도입하지 않고, 로그인된 로컬 Codex 계정 경로만 사용한다.
  - https://developers.openai.com/api/reference/cli/resources/responses/methods/create
  - https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md
- GitHub/upstream — **부분 채택**: `openai/codex`의 App Server thread 격리와 웹 검색 모드 계약만 사용한다. upstream 코드를 복사하거나 새 SDK·검색 패키지를 설치하지 않는다. 기존 직원 thread와 웹 조사 thread를 분리해 웹 권한이 다른 역할이나 다음 일반 turn으로 전파되지 않게 한다.
  - https://github.com/openai/codex
- Google 검색 — **발견·교차검증 수단**: Google 검색으로 App Server와 웹 검색 설정 후보를 찾은 뒤 OpenAI 공식 문서, 현재 설치 버전의 생성 스키마와 upstream으로 다시 확인했다. Google Custom Search·Vertex Grounding 같은 유료 검색 API는 도입하지 않는다.
- Kaggle — **적용 가능한 결과 없음**: 공개 검색·금융 뉴스 Notebook은 Codex App Server 권한 격리, 구독 사용량, 출처 URL 계보와 prompt injection 경계를 검증하지 못한다. 데이터셋·Notebook·런타임 의존성을 채택하지 않는다.
- prompt injection 대응: OpenAI는 웹 콘텐츠가 제3자의 악성 지시를 포함할 수 있고 단순 필터만으로 충분하지 않다고 설명한다. 따라서 검색 Agent는 계정·파일·명령·주문 권한을 갖지 않고, 최종 근거를 `codex-web-1..10` 고정 ID, 전체 HTTPS URL, 관측 내용과 관측 시각으로만 반환한다. URL·ID 계약을 위반한 결과는 직원 보고 경계에서 실패로 닫는다.
  - https://openai.com/index/designing-agents-to-resist-prompt-injection/
- 비용·운영 경계: 별도 Google/OpenAI API key와 종량제 검색 API를 사용하지 않는다. 사용자가 분석을 요청하고 논문 연구원이 해당 도구를 선택한 경우에만 실행하며, 24시간 자동 수집에는 사용하지 않는다. 호출은 로그인된 Codex 구독 사용량과 한도의 적용을 받으며 무제한·무비용을 보장하지 않는다.

## 부서별 상세 보고·공식 근거 보강 (2026-09-04)

- 보안 사전 검토: Codex 웹 조사는 기존 읽기 전용 격리 thread와 명시적 도구 승인 경계를 유지한다. 재무·뉴스 직원만 웹 조사 도구를 선택할 수 있고, 계좌·보유수량·현금·자격정보는 검색어에 포함하지 않는다. 기업행위 공식 근거 판정은 임의 문자열 포함 검사가 아니라 HTTPS와 정확한 허용 호스트(`dart.fss.or.kr`, `opendart.fss.or.kr`, `kind.krx.co.kr`, `data.krx.co.kr`, `hanwhacorp.co.kr`)를 모두 만족해야 한다.
- W3C APG Disclosure Pattern — **채택**: 부서별 장문 보고는 네이티브 `details/summary`로 접고 펼친다. 키보드와 브라우저 기본 의미 구조를 유지하고, 열린 상태에서도 제목·결론·근거 충족도를 먼저 스캔할 수 있게 했다. https://www.w3.org/WAI/ARIA/apg/patterns/disclosure/
- OpenDART 공식 API 목록 — **채택**: 국내 기업행위·재무·공시 근거는 OpenDART 공식 원문을 우선한다. 기존 내부 공급자에 결측이 있을 때 재무·뉴스 직원이 공식 원문을 교차 확인할 수 있도록 도구 선택 범위를 넓혔다. https://opendart.fss.or.kr/intro/infoApiList.do
- GitHub 공개 구현 — **부분 채택**: `fin-research-agent`, `openfr`, `TradingAgents`에서 전문 직원의 중간 산출물을 최종 보고 전에 보존하는 정보 구조를 확인했다. 코드·프롬프트·의존성은 복사하지 않고 기존 Rust 계약과 React 오케스트레이션에서 `summary + findings + evidence + counterevidence + gap`을 손실 없이 전달하는 원칙만 반영했다. `FinSight`는 GPL 범위를 고려해 코드 채택 없이 구조만 비교했다.
  - https://github.com/Schadenfreunde/fin-research-agent
  - https://github.com/oujingzhou/openfr
  - https://github.com/TauricResearch/TradingAgents
  - https://github.com/RUC-NLPIR/FinSight
- 기존 FinPilot — **부분 채택**: 사용자 저장소의 장문 보고 템플릿에서 결론·기술·재무·뉴스·위험·실행 조건을 분리하는 정보 위계를 확인했다. 동일 문구나 코드를 복사하지 않고, 부서 종합·직원별 상세 분석·반대 근거·근거 공백·후속 조치를 별도 구역으로 보존했다.
- Google 검색 — **발견 수단**: 후보를 찾은 뒤 W3C, OpenDART와 upstream 저장소로 다시 검증했다. 검색 결과 문구 자체는 제품 근거로 사용하지 않았다.
- Kaggle — **적용 가능한 결과 없음**: 공개 금융 Notebook과 데이터셋은 저장된 Agent 보고의 가독성, 근거 계보, 도구 권한과 공식 원문 판정을 검증하지 못하므로 이번 UI·오케스트레이션 변경에는 채택하지 않았다.
- 근거 충족도: 80%를 UI 목표값으로 강제하지 않는다. 역할 핵심 사실을 최신 공식 원문과 독립 근거로 교차 확인한 경우에만 80~100, 중요한 공백이 남으면 60~79, 주요 공급자나 핵심 사실이 비면 35~59, 핵심 사실을 확인하지 못하면 0~34로 보고하도록 계약 지침을 명시했다.
## Windows 바탕화면 런처 중첩 pnpm 복구 (2026-09-04)

- 보안 사전 검토: 바로가기나 환경변수에서 임의 실행 경로를 받지 않는다. 기존 `Find-PnpmCommand`가 확인한 `pnpm.cmd`의 부모 디렉터리만 현재 런처 PATH에 추가하고, 자식 빌드가 그 환경을 상속하게 한다. 외부 다운로드·레지스트리 변경·자격정보 로깅은 없다.
- Tauri 공식 설정 — **채택**: `beforeBuildCommand`는 Tauri CLI가 별도 hook 명령으로 실행하므로 해당 자식 환경에서도 `pnpm`을 해석할 수 있어야 한다. 기존 `beforeBuildCommand: pnpm build` 계약은 유지한다. https://v2.tauri.app/develop/configuration-files/
- Microsoft PowerShell — **채택**: `Start-Process`가 기본적으로 현재 프로세스의 환경변수를 상속하는 계약을 사용해, 절대 경로로 검증된 pnpm 디렉터리를 중첩 빌드에 전달한다. https://learn.microsoft.com/powershell/module/microsoft.powershell.management/start-process
- GitHub/upstream — **부분 채택**: Tauri 공식 예제에서도 `beforeBuildCommand: pnpm build`를 사용한다. 코드는 복사하지 않고 현재 런처의 PATH 전달 결함을 수정하는 근거로만 사용했다. https://github.com/tauri-apps/tauri/blob/dev/examples/api/src-tauri/tauri.conf.json
- Google 검색은 후보 발견에만 사용했고 Tauri·Microsoft 공식 문서와 upstream으로 다시 검증했다.
- Kaggle — **적용 가능한 결과 없음**: Tauri 데스크톱 사례는 확인됐지만 Windows 바로가기의 중첩 `beforeBuildCommand` PATH 계약을 검증하는 자료가 아니므로 채택하지 않았다.
# 보유 포트폴리오 분석 기록·차트 근거 보강 (2026-09-04)

- 보안 사전 검토: 분석 기록에는 익명화된 종목·수량·현재가·평단가와 통화별 매수 가능 금액만 저장한다. 계좌번호·별칭·자격정보·Telegram 사용자 ID는 저장하거나 차트에 표시하지 않는다. 기존 append-only 기록은 현재 계좌값으로 소급 덮어쓰지 않는다.
- GitHub 검토: TradingView `lightweight-charts` upstream은 가격선·마커와 공개 좌표 변환 API를 제공하지만, 현재 제품은 이미 의존성 없는 불변 SVG 차트와 결정론적 주석 계약을 갖고 있다. 새 패키지를 추가하지 않고 종목별 차트 저장·MA·거래량 표시 원칙만 부분 채택했다. 저장소: `tradingview/lightweight-charts`, Apache-2.0.
- Google 검토: 공식 Lightweight Charts 문서의 price line·series marker·autoscale 동작을 확인했다. 분석 당시 좌표와 완료 봉을 함께 보존하고, 주석이 잘리지 않도록 가격 범위 안의 관측선만 사용하는 기존 방식을 유지했다.
- Kaggle 검토: 검색 결과는 개별 지지·저항 Notebook과 데이터셋 중심이며 제품의 불변 기록, 계좌 비식별화, 시점 정합성과 UI 계약을 보증하지 않았다. 적용 가능한 결과 없음.
- 채택: 기존 `TechnicalChartEvidence`에 지표 스냅샷을 포함하고 실제 완료 OHLCV에서 MA5·20·60과 거래량을 렌더링한다. 회의·승인형 부서 종합에는 포트폴리오와 종목별 차트를 함께 저장한다.
- 검증: 중복 차트 제거, 20봉 미만 차단, 미래 봉·역순 봉 거부, 포트폴리오 통화 분리, 프론트 테스트·빌드와 Rust 단위 테스트로 검증한다.

## 장문 회의 단계형 종합과 실패 보존 (2026-09-04)

- 보안 사전 검토: 종합 입력에는 기존에 비식별화된 포지션·통화별 매수 가능 금액과 검증된 근거 메타데이터만 사용한다. 계좌번호·별칭·자격정보를 추가하지 않으며 외부 `source`·`observation`은 비신뢰 데이터로 표시한다. 이 변경은 주문 도구나 네트워크 권한을 열지 않고 `SHADOW ONLY`를 유지한다.
- OpenAI 공식 — **채택**: 장시간 Agent 작업은 결과 중심 지시, 구조화 출력과 명시적인 문맥 관리가 필요하며 입력이 모델 문맥을 넘으면 자동 절단되거나 실패할 수 있다. 따라서 무제한 입력 대신 직원→부서→본부장 단계형 산출물과 결정론적 입력 예산을 사용한다.
  - https://developers.openai.com/api/docs/guides/latest-model?model=gpt-5.5
  - https://developers.openai.com/api/reference/cli/resources/responses/methods/create
- GitHub/upstream — **부분 채택**: `openai/openai-agents-python`의 context 지침에서 로컬 상태와 모델에 전달할 문맥을 분리하고 필요한 정보만 메시지·도구·검색으로 주입하는 원칙을 확인했다. SDK와 코드는 추가하지 않고 기존 React/Rust 오케스트레이션에 중복 근거 제거와 단계형 패킷만 적용한다.
  - https://github.com/openai/openai-agents-python/blob/main/docs/context.md
- Google 검색 — **발견 수단**: 장문 Agent 문맥 관리 후보를 찾은 뒤 OpenAI 공식 문서와 upstream 저장소로 다시 확인했다. 검색 결과 문구 자체는 제품 계약 근거로 사용하지 않는다.
- Kaggle — **적용 가능한 결과 없음**: 공개 금융 Notebook은 Codex App Server의 문맥 상한, 계층형 보고 계약, 근거 ID 계보와 부분 실패 저장을 검증하지 못하므로 코드·데이터·성능 근거로 채택하지 않는다.
- 구현 결정: 직원·부서 원문은 분석 기록에 보존한다. 본부장 입력만 부서별 중간 산출물과 `evidenceId` 기준 고유 근거로 조립하고 44,000자 예산을 적용한다. 발생·고유·포함·제외 근거 수를 기록하며 최종 종합 실패도 완료 부서 원문과 실패 이유가 있는 `hold` 기록으로 남긴다.

## 분석 근거 가용성 게이트와 공급자 계약 분리 (2026-09-04)

- 보안 사전 검토: 기존 읽기 전용 스냅샷, 비식별 포지션과 허용된 근거 메타데이터만 재사용한다. 새 자격정보·외부 호출·주문 권한은 추가하지 않으며 외부 뉴스 본문은 계속 비신뢰 입력으로 취급한다.
- 실제 회귀 근거: 한화 전체 포트폴리오 분석에서 기술 담당은 TOSS 가격·기술 근거를, 뉴스 담당은 Codex 웹 10건과 Telegram 2건을 제출했지만 다른 직원의 역할 밖 결측 문장이 부서 전체 결측으로 합쳐졌다. 기술 계산 코드에 존재하는 MACD·Bollinger와 차트 주석도 당시 직원 도구 결과와 개별 보고 저장물에 완전하게 전달되지 않았다.
- GitHub/upstream — **부분 채택**: TradingView Lightweight Charts의 공식 plugin 예제는 trend line, rectangle과 bands indicator를 시계열에 결합된 구조화 primitive로 다룬다. 새 차트 패키지를 도입하지 않고 기존 `TechnicalChartEvidence`의 지표·주석 객체를 직원 도구 결과에도 전달하는 원칙만 채택했다. https://github.com/tradingview/lightweight-charts/blob/master/plugin-examples/src/index.html
- OpenDART 공식 — **유지**: 국내 재무·공시가 실제 0건인 경우에만 결측으로 남기며, 설정된 공식 API 조회 결과와 접수번호 근거를 우선한다. https://opendart.fss.or.kr/intro/infoApiList.do
- Google 검색 — **발견 수단**: 구조화 시계열 주석과 출처 메타데이터 후보를 찾은 뒤 upstream 및 공급자 공식 문서로 다시 확인했다. 검색 결과 자체는 투자 근거나 제품 데이터로 사용하지 않는다.
- Kaggle — **적용 가능한 결과 없음**: 공개 기술지표 Notebook은 앱의 공급자별 계약, 직원 역할 경계, evidenceId 계보와 Telegram 시점 정합성을 보증하지 않아 채택하지 않았다.
- 구현 결정: 기술·재무·뉴스 담당에게 필요한 최소 읽기 전용 도구를 자동 보완한다. 프로그램이 공급자 계약, 유한 기술지표, 차트 주석 수, 재무·공시·일반 뉴스·Telegram 근거 수와 HTTPS 출처 URL 수를 가용성 매니페스트로 계산한다. HTTPS 여부를 공식 출처 판정으로 오인하지 않으며, 직원 서술은 존재 근거를 결측으로 뒤집지 못하고 TOSS 자료에 KIS 고유 계약을 요구하지 않는다.

## 회의 시작 사용량 차단 피드백 (2026-09-04)

- 원인: Codex 1차 사용량이 안전 중단선 80% 이상이면 `agenda_execution_policy`가 회의 기록 생성 전에 정상 차단한다. 기존 UI는 실패 사유를 본부장 대화에만 기록해 소집 창에서는 버튼이 반응하지 않는 것처럼 보였다.
- 보안 사전 검토: 사용량과 초기화 시각만 표시하며 계정 ID·세션·토큰·원문 공급자 응답은 노출하지 않는다. 안전 중단선을 우회하거나 호출 예산을 늘리지 않는다.
- React 공식·GitHub upstream — **부분 채택**: 폼의 pending 상태 동안 중복 제출을 막고 오류를 폼 가까이에 표시하는 원칙만 기존 로컬 상태 구조에 적용했다. 새 라이브러리는 추가하지 않았다. https://react.dev/reference/react-dom/components/form https://github.com/facebook/react
- Google 검색 — **발견 수단**: React 폼 pending·오류 처리 자료를 찾은 뒤 React 공식 문서와 upstream으로 다시 확인했다.
- Kaggle — **적용 가능한 결과 없음**: 금융 Notebook과 데이터셋은 데스크톱 폼의 사용량 차단·접근성·중복 제출 문제와 관련이 없어 채택하지 않았다.

## Codex 80% 경고·95% 차단과 외부 근거 계보 (2026-09-04)

- 보안 사전 검토: 기사·메시지는 비신뢰 외부 입력이며 제목·출처·시각·URL만 화면에 텍스트로 렌더링한다. Telegram 사용자 ID, bot token, Naver Client Secret과 계좌 식별정보는 분석 기록·프롬프트·로그에 추가하지 않는다. 임의 채널명으로 URL을 만들지 않고 공개 username만 엄격한 문자 규칙으로 검증한다.
- Naver 공식 문서 — **채택**: 뉴스 검색 응답의 `title`, `originallink`, `link`, `description`, `pubDate` 계약을 출처 계보에 적용한다. 원문 언론사와 Naver 링크를 구분해 기록한다. https://developers.naver.com/docs/serviceapi/search/news/news.md
- GitHub/upstream — **부분 채택**: 현재 Telegram 계층이 사용하는 `grammers`의 공개 메시지 식별 구조를 유지한다. upstream 저장소는 2026-02-10 archive 후 Codeberg로 이전됐으므로 새 결합이나 버전 변경 없이 기존 의존성 범위만 사용하고 후속 공급자 교체 위험을 남긴다. https://github.com/Lonami/grammers
- Google 검색 — **발견 수단**: Naver 검색 API와 Telegram 클라이언트 후보를 찾은 뒤 공급자 공식 문서와 upstream 상태로 다시 검증했다. 검색 결과 문구 자체는 제품 계약 근거로 사용하지 않는다.
- Kaggle — **기각**: 공개 금융 뉴스 데이터셋은 과거 정적 데이터이고 Naver·Telegram 런타임 출처 계보, 라이선스별 원문 링크와 실제 보고 인용 여부를 검증하지 못한다.
- 구현 결정: 분석 후보 수, 도구 조회 수, 최종 보고 인용 수를 분리하고, 실제 `evidenceId`가 보고서에 존재할 때만 `cited=true`로 기록한다. Codex는 80%부터 경고하며 계속 실행하고 95%부터 매 turn 직전 새 호출을 차단한다.
