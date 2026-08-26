# 기술적 분석 차트 주석

## 담당 직원

리서치부 `기술적 분석가(technical-analyst)`가 담당한다. 이 직원은 제공된 OHLCV와 계산 지표만 사용하며 재무·뉴스·최종 매매 판단을 대신하지 않는다.

## 참고 자료

- 키움 영웅문 사용 사례: 추세선·수평선·수직선으로 지지와 저항을 표시하고 사각형으로 가격 범위를 표시하며, 종목과 봉 주기별로 그림을 보존하는 사용 흐름을 참고했다.
  - https://cowcow.tistory.com/entry/키움-영웅문-HTS-주식-차트-툴바-사용법-선긋기-박스권-그리기-방법
- TradingView Lightweight Charts 공식 예제: 시간·가격 데이터 좌표에 고정되는 trend line, rectangle, vertical line primitive 구조를 참고했다. 라이브러리 코드는 복사하거나 의존성으로 추가하지 않았다.
  - https://github.com/tradingview/lightweight-charts/tree/master/plugin-examples/src/plugins
- `deepentropy/lightweight-charts-drawing`: drawing manager가 선택·삭제·JSON export/import를 관리하는 구조와 도구 분류를 비교했다. 현재 Investa는 자체 SVG 차트를 유지하고 이 패키지를 설치하지 않는다.
  - https://github.com/deepentropy/lightweight-charts-drawing

## 구현 계약

기술적 분석가의 개별 소견이 완료되면, 그 직원에게 전달된 동일 `AnalysisSnapshot`의 완료 봉을 최대 120개까지 분석 기록에 함께 저장한다. 차트 주석은 Codex가 임의 가격을 만들어 반환하지 않고 다음 결정론적 규칙으로 산출한다.

- 최근 최대 60봉의 관측 저점 수평선
- 최근 최대 60봉의 관측 고점 수평선
- 최근 최대 80봉을 앞·뒤 시간 구간으로 나눈 뒤 각 구간 저점을 잇는 선
- 최근 최대 20봉의 실제 고·저 가격 범위 사각형

분석 보관함은 저장된 OHLCV와 주석을 SVG로 다시 그린다. 따라서 현재 시세가 변해도 과거 리포트의 선 위치와 캔들이 바뀌지 않는다. 최소 20개의 완료 봉이 없으면 차트를 만들지 않으며, 표시선은 미래 예측이나 주문 신호가 아니라 관측 구간의 시각 설명이라는 경고를 함께 저장한다.

## 기존 수동 차트와의 경계

모의투자 차트의 사용자가 직접 그린 선은 `localStorage`에 종목·공급자·봉 주기·수정주가 여부별로 저장된다. 분석 리포트 차트는 SQLite 분석 기록에 들어가는 불변 근거이며 개인 수동 선을 자동으로 가져오지 않는다. 사용자의 사후 그림이 과거 분석 근거로 섞이는 것을 막기 위한 분리다.
