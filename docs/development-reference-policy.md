# 개발 레퍼런스 적용 규칙

Investa의 기능을 새로 만들거나 기존 엔진을 바꿀 때 다음 순서를 고정한다.

1. 저장소의 기존 구현·테스트·기획 문서를 먼저 확인한다.
2. 동일 문제를 다루는 공식 문서, 원 논문, 공식 API 명세와 공개 저장소를 찾는다.
3. 레퍼런스의 적용 범위, 데이터 가정, 라이선스, 시점 누수, 유지보수 상태를 기록한다.
4. 외부 코드를 그대로 실행하지 않고 Investa의 타입·불변 원장·보안 경계에 맞는 최소 로직만 재구현한다.
5. 채택하지 않은 방식도 이유를 남긴다. 필요한 입력이 없는 통계량이나 성과 수치는 임의로 만들지 않는다.
6. 레퍼런스 적용 뒤 정상·경계·실패·미래정보 누수 회귀검사를 실행한다.
7. 검증된 범위만 ProjectStudio 기능명세에서 완료 처리한다.

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

이 문서는 투자 성과를 보장하거나 특정 전략을 추천하는 문서가 아니다.
