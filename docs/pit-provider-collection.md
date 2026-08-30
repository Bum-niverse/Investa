# 공식 공급자 PIT 페이지 수집

기준일: 2026-08-28

## 목적

`pit-dataset-builder-v1`에 실제 공식 가격 이력을 넣기 위한 첫 번째 공급자 계층이다. 공개 읽기 전용 시세만 사용하며 API Key, 계좌, 주문·출금 권한은 요구하지 않는다. 이 계층의 출력은 모델 입력 후보일 뿐 주문 신호가 아니다.

## 공식 계약

- Upbit KRW 현물: [Minute Candles](https://global-docs.upbit.com/reference/list-candles-minutes), [Day Candles](https://global-docs.upbit.com/reference/list-candles-days)
  - 요청당 최대 200봉
  - `to` 이전의 봉을 반환하는 배타적·역방향 페이지
  - 거래가 없는 구간은 봉이 생성되지 않으므로 gap을 임의 보간하지 않는다.
- Binance 현물: [Kline/Candlestick data](https://developers.binance.com/en/docs/catalog/core-trading-spot-trading/api/rest-api/market)
  - UTC `startTime`·`endTime`, 시간순 반환, 최대 1,000봉
- Binance USDⓈ-M: [Kline/Candlestick Data](https://developers.binance.com/en/docs/products/derivatives-trading-usds-futures/market-data/rest-api/Kline-Candlestick-Data)
- Binance COIN-M: [Kline/Candlestick Data](https://developers.binance.com/en/docs/products/derivatives-trading-coin-futures/market-data/rest-api/Kline-Candlestick-Data)
  - Investa는 선물 공급자별 최대값 차이에 기대지 않고 페이지당 1,000봉으로 보수적으로 제한한다.

## 구현 계약

- `pit_provider_page_fetch`: 한 페이지를 네트워크에서 읽어 정규화만 한다.
- `pit_provider_page_fetch_store`: 같은 수집 후 SQLite에 불변 저장한다.
- `pit_provider_stored_range`: 여러 페이지의 저장 관측을 시간순으로 병합해 최대 20,001봉을 반환한다.
- `pit_collection_job_create`: 작업 ID와 멱등성 키를 고정하고 장기 범위를 `queued` 상태로 등록한다. 같은 키·같은 요청은 기존 작업을 반환하고, 같은 키의 변조된 요청은 거부한다.
- `pit_collection_job_run`: 한 호출에서 최대 5페이지만 수집한다. 작업 실행권을 원자적으로 획득하고 각 페이지 저장 뒤 커서를 체크포인트한다.
- `pit_collection_job_cancel`: 대기·실행·재시도 작업을 취소한다. 이미 진행 중인 네트워크 요청이 늦게 끝나도 취소 상태를 덮어쓰지 않는다.
- `pit_collection_job_detail`·`pit_collection_job_history`: 현재 커서·페이지/관측 수·재시도 시각과 append-only 상태 이벤트를 조회한다.
- 앱 프로세스의 로컬 스케줄러는 5초마다 due 작업을 확인해 한 tick에 최대 2개, 작업당 최대 5페이지만 실행한다. UI 화면을 닫아도 계속되지만 Investa 프로세스를 완전히 종료하면 멈추고 다음 실행 때 SQLite 체크포인트에서 재개한다.
- `pit_stored_dataset_build_preview`·`pit_stored_dataset_build_commit`: 완료된 수집 작업이 덮는 저장 범위만 기존 `pit-dataset-builder-v1`으로 조립한다. 1봉·5봉 수익률과 5봉 이동평균 괴리는 현재·과거 봉만 사용하며 첫 5봉은 warmup으로 제외한다.
- 가격은 부동소수점 재계산을 피하기 위해 `priceScale=100000000` 고정소수점 정수로 저장한다.
- Binance의 포함형 close time은 1ms를 더해 배타적 `barEndMs`로 통일한다.
- 현재 시각보다 늦게 닫히는 미완료 봉은 저장하지 않는다.
- `recordId`는 공급자·심볼·종료 시각으로 고정하고 값이 바뀌면 덮어쓰지 않고 실패한다.
- `sourceRevision`은 공급자·심볼·종료 시각·가격의 정규화 표현을 SHA-256으로 고정한다.
- 저장 범위는 내부 gap 수와 결과 절단 여부를 함께 반환한다. Upbit gap은 무거래 가능성을 보존하고 Binance gap은 재수집·장애 조사 대상으로 둔다.
- 모든 응답은 `liveOrderAllowed=false`다.

## 장기 수집 작업 상태 계약

- 상태는 `queued → running → queued/completed` 또는 `retry_wait/failed/cancelled`만 사용한다.
- 실행 중 작업은 60초 lease를 갖는다. 프로세스가 비정상 종료된 뒤 lease가 만료되면 같은 체크포인트에서 `recovered` 이벤트와 함께 재개한다.
- 공급자별 다음 호출 가능 시각을 SQLite에 원자적으로 예약한다. Upbit는 초당 8회 이하, Binance 각 시장은 초당 10회 이하로 보수적으로 직렬화해 여러 작업의 동시 burst를 막는다.
- 연결 실패, 공급자 429, 공급자 5xx만 1·2·4초 지수 백오프로 재시도하고 네 번째 실패는 `failed`로 종결한다. 심볼·형식·무결성 오류는 즉시 실패한다.
- 페이지 저장은 기존 불변 관측을 재사용할 수 있으므로, 페이지 저장 직후 프로세스가 종료돼 같은 페이지를 다시 읽어도 가격 관측을 중복 생성하지 않는다.
- 실행·복구·페이지 저장·재시도·취소·완료는 `pit_collection_job_events`에 순서대로 남는다. 이벤트에는 자격정보나 계좌 정보가 들어가지 않는다.
- Upbit는 종료 커서를 과거 방향으로, Binance는 시작 커서를 미래 방향으로만 이동한다.

## 실제 확인

2026-08-28에 동일한 2024년 1분봉 구간을 공개 REST로 조회해 Upbit, Binance 현물, Binance USDⓈ-M, Binance COIN-M이 모두 데이터를 반환하는 것을 확인했다. 자격정보와 주문 권한은 사용하지 않았다.

## 아직 남은 범위

- 토스증권 공식 주식 수년 이력의 페이지·수정주가 리비전 계약 확인
- 앱 프로세스 밖에서도 24시간 실행할 OS 작업 또는 원격 worker와 수집 운영 화면
- 주식 기업행사 원천과 선물 만기·롤·펀딩 원천의 동시 수집
- XGBoost 외부 메모리 worker의 실제 장기 데이터 메모리·시간 soak와 LightGBM shard 지원 여부 결정
