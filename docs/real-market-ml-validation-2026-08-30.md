# 공식 실제시장 ML 기준 검증 (2026-08-30)

## 목적

합성 데이터로 검증한 ML worker를 공식 공개 실제 시장 데이터에 연결해 시점 누수, shard 계약, OOS 확률 지표와 단순 기준선 비교가 끝까지 재현되는지 확인한다. 이어서 OOS 확률을 고정 규칙의 비중첩 거래로 변환하고 명시적 수수료·슬리피지 가정과 실제 funding 이력을 적용한다. 이 검사는 모델 후보의 초기 진단이며 투자 권유, 모델 승격 또는 주문 허가가 아니다.

## 데이터와 계약

- 공급자: Binance 공식 공개 REST API
- 자산: BTCUSDT·ETHUSDT 현물, BTCUSDT·ETHUSDT USDⓈ-M 무기한선물
- 기간: 2026-03-02 17:00 UTC 이상, 2026-08-29 17:00 UTC 미만
- 주기: 완료된 1시간봉 4,320개씩
- 예측 horizon: 4시간 뒤
- 라벨: 수익률 -35bp 미만 `하락`, ±35bp `횡보`, +35bp 초과 `상승`
- 현물 피처: 1·4시간 수익률, 20시간 실현 변동성, 20시간 이동평균 괴리, 고저 범위, 20시간 거래량 비율
- 선물 추가 피처: 결정 시각까지 공개된 funding rate, mark-index basis, mark-trade gap
- 시간 경계: 봉 open을 event time, 봉 close를 available time으로 사용하며 target 관측 시각이 다음 split으로 넘어가는 학습·검증 표본은 제외한다.
- 계보: 자산·split·피처 스키마·각 shard·worker 작업을 SHA-256으로 고정한다.

공개 데이터 수집에는 API 키, 계좌, 주문 또는 출금 권한을 사용하지 않는다. 생성한 원시 shard와 모델은 저장소가 아니라 `%LOCALAPPDATA%\Investa\validation` 아래에 둔다.

## 검증 방식

전체 시간축의 50/10/10, 60/10/10, 70/10/10, 80/10/10 비율로 네 개 expanding walk-forward fold를 만든다. 각 fold의 마지막 10%만 OOS test로 사용하며 네 test 구간은 서로 겹치지 않는다. XGBoost worker 결과는 각 fold의 학습 라벨 분포에 Laplace smoothing을 적용한 고정 확률 기준선과 비교한다.

실행 명령:

```powershell
$python = "$env:LOCALAPPDATA\Investa\ml-worker-venv\Scripts\python.exe"
& $python scripts/run_real_ml_validation.py --days 180
```

## 결과

각 조합의 OOS 표본은 1,719개다. 수치는 백만분율 또는 bp 정수 저장값을 사람이 읽을 수 있게 환산했다.

| 시장 | 종목 | 모델 log loss | 사전확률 기준선 | Brier | ECE | Balanced accuracy |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 현물 | BTCUSDT | 1.017826 | 1.032061 | 0.202671 | 2.01%p | 34.88% |
| 현물 | ETHUSDT | 1.070739 | 1.085773 | 0.215282 | 2.61%p | 37.16% |
| USDⓈ-M | BTCUSDT | 1.019102 | 1.030947 | 0.202733 | 3.06%p | 33.76% |
| USDⓈ-M | ETHUSDT | 1.072943 | 1.086672 | 0.215741 | 2.57%p | 37.70% |

네 조합 모두 단순 기준선보다 log loss는 낮았다. 그러나 balanced accuracy가 무작위 3분류 수준에 가깝고, 단일 180일·1시간봉 구간만 사용했으므로 유효한 거래 모델로 판정하지 않는다.

## 비용 포함 OOS 거래 평가

- 신호: 세 방향 확률의 argmax. 현물은 상승만 long, 선물은 상승 long·하락 short, 횡보는 미진입
- 진입: 판단 시각 다음 봉 시가
- 청산: 고정 4시간 뒤 종가
- 크기: 거래마다 당시 연구 자본 100%, 레버리지 1배
- 중복: 포지션 보유 중 발생한 신호는 건너뜀
- 기본 연구 비용: 현물 taker 10bp/편도, USDⓈ-M taker 5bp/편도, 슬리피지 2bp/편도
- 스트레스: 수수료와 슬리피지를 1배·1.5배·2배 적용
- funding: USDⓈ-M 공식 시점 이력을 long 지급·short 수취 방향으로 적용

정확한 거래 수수료는 사용자 VIP 등급, maker/taker, 할인 상태에 따라 달라진다. 위 값은 공개 계정정보를 조회해 확정한 공식 사용자 수수료가 아니라 명시적 연구 가정이다.

| 시장 | 종목 | 거래 수 | 시장 노출 | 비용 전 | 1배 비용 후 | MDD | 같은 기간 long | 1.5배 | 2배 |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 현물 | BTCUSDT | 51 | 12.04% | +9.85% | -2.80% | 7.05% | +23.31% | -8.58% | -14.02% |
| 현물 | ETHUSDT | 133 | 31.55% | +1.88% | -25.99% | 26.55% | +42.74% | -36.94% | -46.28% |
| USDⓈ-M | BTCUSDT | 65 | 15.60% | -0.74% | -9.29% | 12.19% | +20.27% | -13.33% | -17.20% |
| USDⓈ-M | ETHUSDT | 251 | 58.82% | -7.00% | -34.42% | 34.88% | +41.56% | -45.01% | -53.89% |

같은 기간 long 기준선은 모델 전략과 시장 노출량이 다르므로 승격 기준이 아니라 방향 확인용이다. 현재 규칙은 1배 비용부터 네 조합 모두 순손실이며 비용 스트레스에서 더 악화됐다. 따라서 현재 모델은 전략 후보로 기각하고 실제 주문은 계속 잠근다.

## 730일·다중 주기·관측 레짐 확장

동일 worker와 비용 계약을 730일의 `1h`·`4h`·`1d`로 확장했다. 12개 시장 조합에서 각 4개 expanding walk-forward fold, 총 48개 모델을 실행했다. `1h`와 `4h`는 4시간 horizon, `1d`는 1일 horizon이며 일봉 라벨 임계값은 ±100bp다.

레짐 기준은 전체 기간을 보고 정하지 않는다. 각 fold의 학습 구간에서 타깃까지 이미 관측된 표본만 사용해 `return_4` 절대값 중앙값과 `realized_volatility_20` 75분위수를 산출한다. OOS 표본은 상승·하락·횡보와 정상·고변동을 조합한 6개 상태로 분류한다. 이 분류는 설명용 진단이며 주문 신호 또는 성과 통과 기준이 아니다.

실행 명령:

```powershell
$python = "$env:LOCALAPPDATA\Investa\ml-worker-venv\Scripts\python.exe"
& $python scripts/run_real_ml_validation.py --days 730 --intervals 1h,4h,1d
```

| 주기 | 시장 | 종목 | OOS | Balanced accuracy | 거래 수 | 1배 비용 후 | MDD |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| 1h | 현물 | BTCUSDT | 6,999 | 38.66% | 611 | -74.98% | 74.98% |
| 1h | 현물 | ETHUSDT | 6,999 | 39.92% | 1,116 | -94.74% | 95.00% |
| 1h | USDⓈ-M | BTCUSDT | 6,999 | 38.66% | 725 | -77.09% | 77.09% |
| 1h | USDⓈ-M | ETHUSDT | 6,999 | 40.97% | 1,376 | -77.33% | 79.54% |
| 4h | 현물 | BTCUSDT | 1,744 | 37.48% | 295 | -58.37% | 58.67% |
| 4h | 현물 | ETHUSDT | 1,744 | 39.08% | 898 | -90.01% | 91.42% |
| 4h | USDⓈ-M | BTCUSDT | 1,744 | 37.69% | 500 | -65.22% | 65.94% |
| 4h | USDⓈ-M | ETHUSDT | 1,744 | 39.28% | 1,361 | -43.20% | 63.78% |
| 1d | 현물 | BTCUSDT | 284 | 37.82% | 35 | -9.42% | 21.32% |
| 1d | 현물 | ETHUSDT | 284 | 36.63% | 121 | -23.26% | 43.64% |
| 1d | USDⓈ-M | BTCUSDT | 284 | 34.33% | 58 | -36.90% | 43.52% |
| 1d | USDⓈ-M | ETHUSDT | 284 | 34.55% | 176 | -29.04% | 41.50% |

210개 공식 공개 요청을 사용했고 OOS 표본 중복, 레짐별 거래 합계, 비용 1배·1.5배·2배의 단조 악화와 `liveOrderAllowed=false`를 독립 검사했다. 12개 조합 모두 1배 비용에서 순손실이므로 모델 기각 결론은 더 강해졌다. 일부 세부 레짐의 양수 결과는 사후 하위집단 진단이므로 별도 OOS 재검증 전에는 선택하거나 승격하지 않는다.

## Rolling OOS 온도 보정 실험

다중분류 온도 보정은 확률의 argmax와 순위를 유지하면서 확신의 강도만 조절한다. 각 fold의 test 확률을 같은 fold나 미래 데이터로 보정하지 않았다. 첫 OOS fold는 cold-start로 제외하고, fold 1은 fold 0의 OOS 확률만, fold 2는 fold 0~1만, fold 3은 fold 0~2만 사용해 단일 temperature를 결정했다. 보정값은 고정된 log-space 격자 `0.25~4.0`에서 과거 OOS log loss가 가장 낮은 값을 선택한다.

2026-08-30 공식 Binance 180일 `1h` 데이터를 다시 수집해 42개 공개 요청, 네 시장·종목 조합과 16개 XGBoost 모델을 실행했다. 비교 표본은 cold-start를 제외한 조합별 1,289개로 동일하다.

| 시장 | 종목 | 원시 log loss | 보정 log loss | 원시 ECE | 보정 ECE | 판정 |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| 현물 | BTCUSDT | 0.990716 | 1.007444 | 3.72%p | 7.59%p | 악화 |
| 현물 | ETHUSDT | 1.053812 | 1.065212 | 2.54%p | 4.37%p | 악화 |
| USDⓈ-M | BTCUSDT | 0.997933 | 1.010409 | 3.67%p | 6.25%p | 악화 |
| USDⓈ-M | ETHUSDT | 1.055142 | 1.067536 | 2.72%p | 2.53%p | ECE만 개선, log loss 악화 |

네 조합 모두 log loss가 나빠졌고 ECE도 세 조합에서 악화됐다. 따라서 이 보정 방식은 현재 모델에 채택하지 않는다. 온도 보정은 argmax를 바꾸지 않으므로 기존 매매 신호와 비용 포함 전략 손익도 개선하지 않는다. 구현 완료는 보정 모델 승인이나 전략 승격을 의미하지 않는다.

## 동일 OOS 모델 비교

`--compare-models`를 켜면 기존 shard XGBoost와 단일 manifest LightGBM, 학습구간 모멘텀 상태 사전확률, 전체 학습 클래스 사전확률을 같은 fold·피처·horizon·OOS sample ID에서 비교한다. 단순 모멘텀 기준선은 각 fold의 과거 학습 구간에서 `return_4` 절대값 중앙값으로 하락·횡보·상승 상태를 만들고, 상태별 실제 클래스 빈도에 Laplace smoothing을 적용한다. validation과 test 정답은 기준선 학습에 쓰지 않는다.

2026-08-30 Binance 공식 공개 180일 `1h` 네 조합을 42개 요청으로 다시 수집했다. XGBoost 16개와 LightGBM 16개를 동일한 4개 expanding walk-forward fold에서 학습했고, 각 조합의 OOS 표본은 1,719개다.

| 시장 | 종목 | 클래스 사전확률 | 모멘텀 상태 | LightGBM | XGBoost | 최저 log loss |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| 현물 | BTCUSDT | 1.032414 | 1.026647 | 1.022225 | 1.019235 | XGBoost |
| 현물 | ETHUSDT | 1.085831 | 1.074575 | 1.069379 | 1.068339 | XGBoost |
| USDⓈ-M | BTCUSDT | 1.031309 | 1.025890 | 1.021141 | 1.021215 | LightGBM |
| USDⓈ-M | ETHUSDT | 1.086731 | 1.075304 | 1.070409 | 1.070875 | LightGBM |

현물은 XGBoost, 선물은 LightGBM의 log loss가 가장 낮았지만 차이는 작다. LightGBM balanced accuracy는 33.08~36.71%, XGBoost는 33.55~37.46%다. 단순 모멘텀 상태 기준선도 네 조합 모두 클래스 사전확률보다 log loss가 낮아 피처 방향성이 완전히 무의미하다고 볼 수는 없지만, 거래 비용 후 수익성을 뜻하지 않는다. 비교 결과는 설명용이며 자동 winner 선택, 모델 등록, 전략 승격과 주문 권한을 만들지 않는다.

## 남은 검증

1. 3~5년 이상과 1분·15분 등 더 짧은 분봉 표본
2. 현재 6개 관측 레짐의 독립 기간 재검증과 레짐 임계값 안정성
3. 실제 계정 등급 수수료·호가 스프레드·시장 충격을 포함한 체결 민감도 검증
4. 국내·미국 주식과 증권선물의 공식 PIT 데이터
5. 독립 calibration 구간·더 긴 기간에서 sigmoid·temperature 후보의 drift 비교
6. 완료한 단순 모멘텀·LightGBM·XGBoost 비교를 3~5년·다주기로 확장하고 Chronos·TimesFM 후보를 동일 조건에 추가

## 공식 참고

- [Binance Spot API 문서](https://developers.binance.com/docs/binance-spot-api-docs)
- [Binance USDⓈ-M Kline 문서](https://developers.binance.com/docs/derivatives/usds-margined-futures/market-data/rest-api/Kline-Candlestick-Data)
- [XGBoost External Memory 문서](https://xgboost.readthedocs.io/en/latest/tutorials/external_memory.html)
- [scikit-learn Probability calibration](https://scikit-learn.org/stable/modules/calibration.html)
- [scikit-learn TimeSeriesSplit](https://scikit-learn.org/stable/modules/generated/sklearn.model_selection.TimeSeriesSplit.html)
- [Guo et al., On Calibration of Modern Neural Networks](https://proceedings.mlr.press/v70/guo17a.html)
