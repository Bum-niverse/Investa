# 기술적 분석 차트 주석

## 담당 직원

리서치부 `기술적 분석가(technical-analyst)`가 담당한다. 이 직원은 제공된 OHLCV와 계산 지표만 사용하며 재무·뉴스·최종 매매 판단을 대신하지 않는다.

## 참고 자료

- 키움 영웅문 사용 사례: 추세선·수평선·수직선으로 지지와 저항을 표시하고 사각형으로 가격 범위를 표시하며, 종목과 봉 주기별로 그림을 보존하는 사용 흐름을 참고했다.
  - https://cowcow.tistory.com/entry/키움-영웅문-HTS-주식-차트-툴바-사용법-선긋기-박스권-그리기-방법
- TradingView Lightweight Charts 공식 예제: 시간·가격 데이터 좌표에 고정되는 trend line, rectangle, vertical line primitive 구조를 참고했다. 라이브러리 코드는 복사하거나 의존성으로 추가하지 않았다.
  - https://github.com/tradingview/lightweight-charts/tree/master/plugin-examples/src/plugins
- Upbit 공식 일봉 API: 디지털자산 봉은 거래가 발생한 구간에만 생성되므로 빈 봉을 임의 보간하지 않는 규칙을 반영했다.
  - https://global-docs.upbit.com/reference/list-candles-days
- Binance USDⓈ-M Futures 공식 API: 일반 Kline과 별도로 Mark Price, Index Price, Funding Rate 및 Continuous Contract Kline을 제공하므로 코인 무기한선물 주석 근거를 체결가와 분리했다.
  - https://developers.binance.com/en/docs/catalog/core-trading-derivatives-trading-usd-s-m-futures/api/rest-api/market-data
- 한국투자증권 Open Trading API 공식 예제: 국내선물옵션 기간별시세의 계약별 일봉과 모의 서버 규격을 적용했다. 공식 응답에 없는 정산가·근월물 연결 정보는 만들지 않는다.
  - https://github.com/koreainvestment/open-trading-api
- CME Group 공식 교육 자료: 선물은 공식 일일 정산가로 mark-to-market되며 만기와 롤오버가 존재하므로 현 계약 구간 밖으로 추세선을 연장하지 않는다.
  - https://www.cmegroup.com/education/courses/introduction-to-futures/mark-to-market
  - https://www.cmegroup.com/education/courses/introduction-to-futures/understanding-futures-expiration-contract-roll
- `deepentropy/lightweight-charts-drawing`: drawing manager가 선택·삭제·JSON export/import를 관리하는 구조와 도구 분류를 비교했다. 현재 Investa는 자체 SVG 차트를 유지하고 이 패키지를 설치하지 않는다.
  - https://github.com/deepentropy/lightweight-charts-drawing

## 구현 계약

기술적 분석가의 개별 소견이 완료되면, 그 직원에게 전달된 동일 `AnalysisSnapshot`의 완료 봉을 최대 120개까지 분석 기록에 함께 저장한다. 차트 주석은 Codex가 임의 가격을 만들어 반환하지 않고 다음 결정론적 규칙으로 산출한다.

- 주식: 최근 최대 60봉의 관측 고·저점과 저점 연결선, 최근 20봉 가격 범위
- 코인 현물: 거래가 발생한 완료 봉만 쓰는 24시간 관측 고·저점·저점 연결선과 최근 30봉 범위
- 증권 선물: `contractCode`가 같은 현재 만기 안에서만 계산한 고·저점·추세선, 공식 정산가, 만기 롤 경계 수직선
- 코인 무기한선물: 24시간 가격 구조와 별도 `markPrice`, `indexPrice`, `fundingTime` 근거선

모든 비주식 봉은 `availableAtMs`와 `ingestedAtMs`를 보존한다. 분석 기준 시각 뒤에 끝나거나 공개·수집된 봉, 겹친 봉과 중복 봉은 거부한다. 증권 선물은 계약코드가 없으면, 코인 무기한선물은 마크가격 또는 지수가격이 없으면 차트 근거를 생성하지 않는다.

## 실제 공급자 연결 상태

- Upbit 원화 현물: 공개 일봉을 공통 `AnalysisSnapshot`으로 정규화하고 `crypto_spot`으로 라우팅한다. `KRW-BTC`와 비트코인·이더리움·리플 별칭을 지원하며 완료·공개·수집 시각을 보존한다.
- Binance USDⓈ-M 무기한선물: 4시간 체결 봉, 마크가격 봉, 지수가격 봉을 동일 시작 시각으로 교차 검증하고 실제 펀딩 관측만 붙여 `crypto_perpetual`로 라우팅한다.
- 두 공급자 모두 자격정보가 필요 없는 공개 API만 사용한다. 호출 실패·심볼 불확정·시계열 불일치 시 값을 만들지 않고 차트 생성을 중단한다.
- 증권선물은 KIS 공식 모의 서버의 국내선물 계약별 일봉 어댑터와 결정론적 라우팅을 구현했다. 현재 만기 계약코드를 직접 입력해야 하고 별도 정산가·근월물 연결 정보는 생성하지 않는다. 이 PC에는 KIS 모의 자격정보가 없어 실제 공급자 왕복만 미검증이다.

분석 보관함은 저장된 OHLCV와 주석을 SVG로 다시 그린다. 따라서 현재 시세가 변해도 과거 리포트의 선 위치와 캔들이 바뀌지 않는다. 최소 20개의 완료 봉이 없으면 차트를 만들지 않으며, 표시선은 미래 예측이나 주문 신호가 아니라 관측 구간의 시각 설명이라는 경고를 함께 저장한다.

## 기존 수동 차트와의 경계

모의투자 차트의 사용자가 직접 그린 선은 `localStorage`에 종목·공급자·봉 주기·수정주가 여부별로 저장된다. 분석 리포트 차트는 SQLite 분석 기록에 들어가는 불변 근거이며 개인 수동 선을 자동으로 가져오지 않는다. 사용자의 사후 그림이 과거 분석 근거로 섞이는 것을 막기 위한 분리다.
