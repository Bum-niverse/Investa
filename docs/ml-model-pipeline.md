# ML 모델 파이프라인 기반

기준일: 2026-08-30

## 구현 범위

`investa-ml-worker-v1`은 모델 라이브러리를 Tauri 앱 프로세스와 분리하고 데이터와 외부 worker 사이의 경계를 고정한다.

1. 기존 `forecast_dataset_audits`의 통과 결과와 동일한 자산·데이터셋만 ML 매니페스트로 만들 수 있다.
2. 표본과 피처는 안정적으로 정렬한 뒤 전체 payload와 피처 스키마를 각각 SHA-256으로 고정한다.
3. train·validation·test에 최소 한 표본을 요구하고, 학습·검증 타깃 관측 시각이 다음 구간으로 넘어가는 경우 누수로 거부한다.
4. worker 요청은 알고리즘 enum, 코드 버전, seed, horizon, 자원 상한과 scalar 하이퍼파라미터만 포함한다. 실행 파일 경로·쉘 명령·자격정보는 포함하지 않는다.
5. worker bundle을 읽을 때 저장 payload 해시를 다시 계산한다. 변조되었거나 준비 상태가 아니면 내보내지 않는다.
6. 작업 결과는 실패 또는 완료로 불변 저장한다. 성공 결과도 모델 레지스트리의 `candidate_review`일 뿐 자동 승격되지 않는다.
7. 모델 파일은 basename, 알고리즘별 허용 포맷, 최대 2GiB, SHA-256 메타데이터를 요구한다. Python worker는 임시 파일 학습이 끝난 뒤 원자적으로 최종 이름으로 옮기고 실제 파일 SHA-256을 계산한다.
8. 모든 응답은 `liveOrderAllowed=false`이며 모델이 주문이나 위험 정책을 직접 변경하는 경로는 없다.

## 지원 예정 알고리즘과 포맷

| 알고리즘 | 허용 결과 포맷 | 현재 상태 |
| --- | --- | --- |
| LightGBM | text, ONNX | 4.7.0 CPU 기준 worker·text 아티팩트 실제 학습 완료 |
| XGBoost | JSON, ONNX | 3.4.1 CPU 기준 worker·JSON 아티팩트 실제 학습 완료 |
| Chronos | safetensors, ONNX | 비교 어댑터 계약만 준비 |
| TimesFM | safetensors, ONNX | 비교 어댑터 계약만 준비 |

pickle과 임의 실행 포맷은 허용하지 않는다. 외부에서 받은 모델은 신뢰하지 않으며, 실제 아티팩트 연결 단계에서 파일 해시 재계산, 격리 실행, 자원 제한과 출처 검증을 추가해야 한다.

## 구현된 Python 기준 worker

- Python 3.14 전용 venv를 저장소 밖 `%LOCALAPPDATA%\Investa\ml-worker-venv`에 두고 버전을 `ml-worker/requirements.lock`에 고정했다.
- payload·feature schema·job 입력 해시를 Python에서 다시 계산하고 `liveOrderAllowed=false`를 강제한다.
- feature의 `availableAtMs <= decisionTimeMs`, 타깃 관측의 split 경계, sample×feature 완전성과 세 방향 학습 클래스 포함 여부를 재검사한다.
- LightGBM·XGBoost가 공유하지 않는 하이퍼파라미터와 비수치·범위 밖 값을 거부한다.
- train으로 학습하고 validation은 early stopping에만 사용하며 log loss·Brier·ECE·balanced accuracy는 test에서만 계산한다.
- test 표본별 하락·횡보·상승 확률을 백만분율 정수로 양자화해 합계를 정확히 1,000,000으로 고정한다. Rust는 원시 확률에서 log loss·Brier·ECE·balanced accuracy와 fold 수를 다시 계산하고 worker 보고값과 한 항목이라도 다르면 모델 등록을 거부한다.
- Python과 Rust는 동일 JSON fixture를 사용해 반올림·동점 분류·ECE bin 경계가 일치하는지 회귀검사한다.
- worker 원시 결과는 확률 보정 전 기준선이다. 실제 검증 runner는 현재 fold 이전 OOS 확률만 쓰는 `rolling-oos-temperature-v1`을 별도 진단으로 계산하지만 원시 아티팩트를 덮어쓰지 않는다. synthetic 회귀검사 통과는 실제 시장 예측 성능을 의미하지 않는다.

## 구현된 PIT 데이터·라벨 빌더

`pit-dataset-builder-v1`은 공급자별 다운로드 코드와 학습 데이터 계약을 분리한다.

1. `pit_collection_plan_create`는 1·3·5·15·30분, 1·4시간, 일봉 기간을 `[start, end)` 비중첩 창으로 나눈다. 공급자 자격정보나 주문 권한은 받지 않는다.
2. 라벨 가격 기준은 주식 `adjusted_close`, 코인 현물 `close`, 증권 선물 `settlement`, 코인 무기한선물 `mark`로 고정한다. horizon과 상승·하락 임계값은 매니페스트 입력에 명시한다.
3. 주식 기업행사, 증권 선물 만기·롤오버를 가로지르는 표본은 제외한다. 무기한선물 펀딩은 `exclude_crossing` 또는 펀딩 손익을 포함하지 않는 `price_return_only`를 명시해야 한다.
4. 완료 가격봉만 사용한다. 코인 24시간 봉의 gap, 중복 bar end, 가격 단위 혼합과 잘못된 `barEnd ≤ availableAt ≤ ingestedAt` 순서를 거부한다.
5. 각 결정 시각에는 `eventTime ≤ barEnd` 및 `availableAt ≤ decisionTime`인 피처 중 가장 늦게 이용 가능해진 리비전만 as-of 조인한다. 미래 수정본, 중복 리비전, 필수 피처 결측은 감사에서 차단한다.
6. preview는 표본 원문을 UI로 반환하지 않고 건수·제외 사유·감사 결과만 제공한다. commit은 유효한 결과만 기존 `forecast_dataset_audits`와 `ml_dataset_manifests`에 불변 저장한다.
7. 현재 한 매니페스트는 기존 안전 상한인 표본 20,000개·피처 200,000개·직렬화 64MiB를 유지한다.

## 구현된 장기 분봉 shard set

`shard-set-v1`은 단일 매니페스트 안전 상한을 없애지 않고 기존 불변 매니페스트를 논리 데이터셋으로 묶는다.

1. 한 집합에는 기존 PIT 감사와 불변 저장을 통과한 매니페스트 2~64개만 넣을 수 있다.
2. 모든 shard는 동일한 자산 계약, train·validation·test 경계와 피처 스키마 SHA-256을 사용해야 한다.
3. 요청에 적힌 순서대로 train·validation·test 각 구간의 시간이 엄격히 증가해야 하며 shard 사이 표본 ID 중복을 거부한다.
4. 전체 구성과 자식 content SHA-256을 결합 해시로 고정한다. 동일 ID·동일 내용 재시도는 멱등이며 다른 내용 덮어쓰기는 거부한다.
5. 상세·이력 조회 때 결합 해시뿐 아니라 현재 SQLite에 저장된 각 자식 payload 해시를 다시 계산한다. 자식 누락이나 사후 변조가 있으면 재생을 중단한다.
6. 논리 상한은 64 shard, 표본 1,000,000개, 피처 값 10,000,000개다. 이는 저장·검증 계약의 상한이지 한 프로세스 메모리에 모두 적재해 학습한다는 뜻이 아니다.
7. XGBoost shard 작업은 `investa-ml-worker-sharded-v1` 계약으로만 준비한다. Rust runner가 검증된 payload를 작업별 `shards/shard-0000.json` 고정 목록에 `create_new`로 staging하고 실행 직후 입력을 제거한다.
8. Python은 결합·자식·피처 스키마 해시, 파일명과 경로 경계, 파일 크기, split별 시간 순서와 표본 ID 중복을 다시 검사한다. 그 뒤 XGBoost `DataIter`와 `ExtMemQuantileDMatrix`, CPU `hist`로 한 shard씩 소비한다.
9. LightGBM shard 입력은 안전한 streaming 경로와 동일성 검증이 마련될 때까지 명시적으로 거부한다. XGBoost 성공 결과도 기존 Rust OOS 재계산을 통과한 `candidate_review`일 뿐 자동 승격·외부 주문 권한은 없다.

공식 공개 PIT 가격 계층은 Upbit 현물과 Binance 현물·USDⓈ-M·COIN-M의 pagination·불변 저장을 지원한다. 완료된 수집 범위는 원천 계보를 보존한 1봉·5봉 수익률과 5봉 이동평균 괴리 피처로 조립해 같은 감사·매니페스트 경로에 넣는다. 다만 토스 주식 수정주가, 주식 기업행사와 증권 선물 상품 수명주기 원천은 별도 미완료다.

## 다음 구현

1. 완료한 730일·1h·4h·1d 기준을 3~5년·더 짧은 분봉으로 확장하고 독립 calibration 구간의 보정 drift를 비교한다.
2. 토스 주식 수정주가와 기업행사·증권 선물 수명주기 원천을 같은 PIT 계약으로 연결한다.
3. LightGBM의 안전한 out-of-core 경로를 검증하거나 지원 제외 결정을 확정하고, 실제 장기 데이터에서 메모리·시간 상한 soak를 수행한다.
4. Chronos·TimesFM은 동일 데이터·동일 horizon의 비교 후보로만 실행하며 모델 크기와 추론 시간을 함께 기록한다.
5. 검증된 후보만 기존 전략 승격 흐름의 SHADOW Canary 입력으로 연결한다.

## 구현된 ML worker 실행 격리

`ml-worker-runner-v1`은 다음 경계를 구현한다.

1. 앱이 저장된 `prepared` 작업 ID만 받아 bundle을 다시 구성하고 임의 실행 파일·스크립트·인자를 입력으로 받지 않는다.
2. 개발 환경과 패키지 환경의 worker 스크립트, 저장소 밖 전용 Python venv를 각각 고정된 후보 경로에서만 해석한다.
3. 하위 프로세스에는 금융·Cloud·GitHub·Telegram 자격정보 환경변수를 상속하지 않고 필요한 Windows·Python 실행 환경만 전달한다.
4. 작업별 timeout, stdout·stderr 상한, 결과 파일 크기와 JSON 계약을 강제한다.
5. Windows에서는 Job Object의 process memory limit와 `KILL_ON_JOB_CLOSE`를 적용해 앱 종료·timeout 때 자식 프로세스를 남기지 않는다.
6. timeout·출력 초과·비정상 종료는 제한된 실패 코드로 불변 저장하며 모델을 등록하지 않는다.
7. 성공 결과도 기존 Rust OOS 재계산과 아티팩트 계약을 통과해야 `candidate_review`로 등록한다.
8. 프로세스 내 작업 claim으로 같은 prepared 작업의 동시 실행을 거부하고 앱 재시작 뒤에는 새 attempt로 복구할 수 있다.

정상·비정상 프로세스, timeout 강제 종료, 출력 상한, 잘못된 결과 JSON과 아티팩트 변조를 회귀검사한다. 실행 입력 복제본은 프로세스 시작 뒤 제거하고, 성공 아티팩트와 제한된 결과만 앱 데이터의 작업별 attempt 폴더에 보존한다. 실제 수년치 시장 모델 성능과 외부 주문 연결은 이 묶음에 포함하지 않는다.

## 공식 실제시장 기준 검증

2026-08-30 `scripts/run_real_ml_validation.py`로 Binance 공식 공개 BTC·ETH 현물과 USDⓈ-M 180일 1시간봉을 수집해 XGBoost shard worker를 네 개 expanding walk-forward OOS 구간에서 실행했다. OOS test 표본은 fold 사이 중복을 거부하고, 각 shard·결합 데이터·작업 입력 해시와 worker 예측 수를 다시 확인한다.

네 조합 모두 학습 클래스 사전확률 기준선보다 log loss는 낮았지만 balanced accuracy는 33.76~37.70%였다. OOS argmax를 다음 봉 시가 진입·4시간 뒤 종가 청산의 비중첩 신호로 바꾸고, 명시적 taker 수수료·슬리피지 가정과 공식 funding 이력을 적용한 결과 1배 비용부터 네 조합 모두 순손실이었다. 따라서 초기 실제 데이터 경로 검증만 통과하고 현재 기준 모델은 전략 후보로 기각한다. 모델 승격이나 주문 근거로 사용하지 않으며 상세 조건과 수치는 [공식 실제시장 ML 기준 검증](real-market-ml-validation-2026-08-30.md)에 기록한다.

같은 날 730일 `1h`·`4h`·`1d`로 확장해 12개 시장 조합과 48개 walk-forward 모델을 실행했다. 각 fold의 레짐 임계값은 타깃까지 관측된 과거 학습 표본만으로 산출하며 OOS 거래를 상승·하락·횡보 × 정상·고변동 6개 설명 상태로 분리한다. 210개 공식 요청과 레짐별 거래 대사는 통과했지만 12개 조합 모두 기본 비용에서 순손실이므로 현재 모델 기각 상태를 유지한다.

이후 180일 `1h` 네 조합을 다시 실행해 첫 fold를 제외하고 현재 fold 이전 OOS 확률만으로 다중분류 temperature를 맞췄다. 동일한 조합별 1,289개 표본에서 네 조합 모두 log loss가 악화했고 ECE도 세 조합에서 나빠졌다. 따라서 `rolling-oos-temperature-v1`은 누수 없는 진단 경로 구현만 완료하고 현재 모델에는 적용하지 않는다. 원시 worker 결과와 전략 신호, 모델 등록 상태는 변경하지 않는다.

같은 데이터에서 `same-oos-model-comparison-v1`도 실행했다. 기존 shard XGBoost와 단일 manifest LightGBM, 학습구간의 `return_4` 상태별 클래스 빈도 기준선, 전체 클래스 사전확률을 같은 4개 fold·피처·horizon·1,719개 OOS ID로 비교한다. 현물 두 조합은 XGBoost, USDⓈ-M 두 조합은 LightGBM의 log loss가 가장 낮았지만 차이가 작고 balanced accuracy는 두 모델 모두 약 33~37%였다. `lowestLogLossLabel`은 설명용이며 자동 winner 선택·모델 등록·전략 승격에 사용하지 않는다.

## 검증 근거

- scikit-learn `TimeSeriesSplit`: 시간순 분할과 gap의 근거
- MLflow Model Registry: run·model version·alias/tag 계보의 근거
- XGBoost Model IO: pickle 대신 JSON/UBJSON 안정 포맷 사용 근거
- XGBoost External Memory: custom `DataIter`와 `ExtMemQuantileDMatrix`를 사용하되 공식 문서가 실험적 기능으로 명시하므로 버전을 3.4.1로 고정하고 저장 계약·학습 지원·실제 성능 검증을 분리한 근거
- ONNX Security: 외부 모델과 입력을 신뢰하지 않고 출처·격리·자원 제한을 적용하는 근거

외부 저장소 코드는 복사하지 않았다. 승인된 공식 PyPI wheel만 저장소 밖 venv에 설치했고 앱의 Node·Rust 의존성에는 추가하지 않았다.
