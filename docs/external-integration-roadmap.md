# Investa 외부 연동 우선순위

기준일: 2026-08-21

## 확정 순서

1. Codex App Server 연결
2. 연구원 레퍼런스 분석과 `StrategySpec` 생성
3. 시점 정합 데이터·전략 플러그인·백테스트
4. 토스증권 Open API 시세·종목·계좌 조회
5. 내부 섀도우 원장과 재시작 복구
6. 토스증권 주문은 별도 승인 전까지 비활성화
7. 티스토리 초안 내보내기·대표 승인형 수동 발행
8. 티스토리 화면 자동화 발행은 마지막 선택 기능

코인 현물은 위 주식 섀도우·원장 흐름이 안정된 뒤 `업비트 공개 시세 → 개인 잔고 조회 → 주문 생성 테스트 → 사용자 승인형 현물 주문` 순서로 연결한다. 케이뱅크 직접 API는 범위에 포함하지 않는다.

## 1. Codex App Server

Investa처럼 로그인, 대화 이력, 승인과 스트리밍 작업 이벤트가 필요한 로컬 데스크톱 제품은 Codex SDK보다 `codex app-server`를 우선한다.

- Tauri Rust가 사용자가 지정한 `codex` 실행 파일을 자식 프로세스로 시작한다.
- 기본 전송은 외부 포트를 열지 않는 stdio JSONL이다.
- `initialize`/`initialized` 이후 `account/read`, `thread/start`, `turn/start`와 스트리밍 notification을 처리한다.
- 기본 로그인은 ChatGPT browser 또는 device-code 방식이며 API key는 선택형 후속 방식이다.
- ChatGPT 로그인은 구독 사용량을 따르고 API key 로그인만 표준 API 사용량 과금으로 분리한다.
- 연구원 thread는 처음에 읽기 전용 sandbox, 제한된 작업 폴더, 네트워크 OFF와 구조화 `outputSchema`로 시작한다.
- GitHub·논문 수집 단계에서만 목적지 allowlist와 사용자 승인을 거쳐 네트워크를 연다.
- Codex에는 토스증권 자격증명, 계좌 식별자, 주문 함수와 위험 정책 변경 함수를 제공하지 않는다.

현재 PC의 WindowsApps 번들 대신 `%LOCALAPPDATA%\Investa\codex-cli`에 별도 Codex CLI 0.149.0을 설치했다. Investa는 `INVESTA_CODEX_PATH` 설정을 우선하고, 없으면 이 사용자 설치 폴더 안에서 실행 파일을 탐색한다. `--version`, `login status`, app-server handshake, ChatGPT 계정 확인과 실제 한글 delta·완료 이벤트를 검증했다.

현재 완료: 역할별 thread 생성, 실제 delta·완료·오류·취소 UI 연결, 연구원 `outputSchema` 검증, 사용 한도 표시, 직원별 thread ID SQLite 저장과 재시작 복원, 공개 GitHub 저장소 2개까지의 allowlist 기반 메타데이터·HEAD·README 수집, 앱 종료 시 자식 프로세스 정리, 본부장 안건 자동 분류와 선택 부서별 구조화 보고·최종 종합 오케스트레이션. 남은 범위: 논문 공급자 추가, 한도 도달 상세 복구와 전체 회의 장시간 검수.

공식 근거:

- https://learn.chatgpt.com/docs/app-server
- https://learn.chatgpt.com/docs/auth

## 2. 토스증권 Open API

토스증권 Open API는 국내·미국 주식의 시세, 종목, 환율, 장 운영 시간, 계좌·자산, 주문과 조건주문을 제공한다. 인증은 OAuth 2.0 Client Credentials이며 WTS에서 client ID·secret과 허용 IP를 설정해야 한다. 공식 명세 1.2.14는 REST와 실시간 체결·호가용 WebSocket을 함께 제공한다. 현재 지수 전광판에 필요한 KOSPI·KOSDAQ 시장 지표는 REST endpoint를 15초 간격으로 조회한다.

- 2026-08-21 1차 읽기 전용 어댑터를 구현했다. 설정에서 연결을 검증한 뒤 Windows 자격 증명 관리자에만 저장한다.
- 2026-08-21 연구원 전략 후보에 대해 최신 최대 200개 수정주가 일봉을 읽어 탐색 백테스트하는 수직 흐름을 연결했다. 진행 중 봉은 제외하고 원천 이용 가능 시각과 수집 시각을 분리한다.
- 1차는 종목·현재가·캔들·시장 캘린더 등 비주문 조회다.
- 2차 계좌·보유 자산 조회를 구현했다. 비밀값은 OS 보안 저장소, 계좌 식별자는 Rust 요청 경계에만 두고 화면에는 마스킹 계좌번호와 허용된 자산 요약만 반환한다.
- 공식 문서에서 별도 모의투자·sandbox 환경을 확인하지 못했으므로 주문 endpoint는 내부 섀도우·복구·안전 검증과 사용자 별도 승인이 끝날 때까지 호출하지 않는다.
- 고정 IP, 429 응답, `Retry-After`, 토큰 만료와 REST polling 한계를 설계에 포함한다.

### 부서별 시장 지수 전광판

일반 부서 8곳의 벽시계를 KOSPI·KOSDAQ·NASDAQ 3행 픽셀 전광판으로 교체했다. React는 `market_indices_snapshot` Tauri 명령을 공급자가 지정한 주기(현재 기본 15초)로 다시 호출한다. 스냅샷 계약은 지수 코드, 현재값, 등락률, 원천 관측시각, 수집 상태, 공급자와 수집시각을 분리한다.

- 토스증권의 시장 지표 현재가 endpoint는 국내 지수·국채를 지원하므로 KOSPI·KOSDAQ 1차 공급자 후보로 사용한다.
- 토스증권 Open API는 모든 시세 호출에 OAuth 2.0 access token이 필요하다. 체결·호가는 WebSocket을 지원하지만 시장 지표 endpoint는 REST이므로, 지수 전광판은 초 단위 스트리밍이라고 과장하지 않고 polling 주기와 원천 관측시각을 함께 유지한다.
- 토스증권 공식 시장 지표 범위는 국내 지수이므로 NASDAQ Composite를 ETF나 다른 종목으로 대체하지 않는다.
- NASDAQ 공식 실시간/지연 지수 데이터는 Global Index Data Service 등 별도 상품·자격증명 영역이다. 비용과 라이선스에 영향이 있으므로 공급자는 사용자 승인 후 결정한다.
- 공급자 연결 전에는 가격 필드를 `null`로 유지하고 `FEED WAIT · 대기`를 표시한다. 예시 숫자, 직전 하드코딩 값과 임의 변동 애니메이션은 실제 시세로 표시하지 않는다.
- 자격증명은 React, SQLite, 로그와 ProjectStudio 문서에 저장하지 않고 Rust 계층과 OS 보안 저장소 안에서만 사용한다.
- 공식 시장 지표 현재가 응답에는 등락률이 없으므로 값을 추정하지 않고 `null`/`대기`로 표시한다.
- 401은 캐시 토큰을 한 번 폐기하고 재발급하며, 429·서버 장애는 마지막 정상값을 `delayed`로 표시한다. 외부 호출 타임아웃은 8초다.

공식 근거:

- https://developers.tossinvest.com/docs
- https://developers.tossinvest.com/docs/market-data
- https://p.tossinvest.com/ko/open-api
- https://openapi.tossinvest.com/openapi-docs/latest/openapi.json
- https://openapi.tossinvest.com/openapi-docs/latest/asyncapi.json
- https://docs.data.nasdaq.com/docs/api-for-real-time-or-delayed-data-1

## 3. 티스토리

티스토리는 공식 Open API를 2023년 말부터 2024년 2월 말까지 순차 종료했으며 글 작성·수정과 이미지·파일 첨부를 외부 API로 처리할 수 없다고 공지했다. 따라서 존재하지 않는 공식 발행 API를 전제로 구현하지 않는다.

- 홍보부가 ProjectStudio 기록·Git 변경·테스트·실제 화면으로 Markdown·HTML 초안을 만든다.

## 기록 분리 원칙

- 내부 모의 원장은 `내부 체결`, `수동 주문`, `섀도우 자동매매`, `KIS 모의`, `백테스트 재생` 보기로 구분한다.
- 백테스트 재생 체결은 과거 데이터 검증용 불변 기록이며 내부 모의계좌의 현금·포지션·실현손익을 변경하지 않는다.
- 분석 기록은 `시스템 검사`, `연구 실험`, `승격 후보`로 분류한다. 시스템 검사의 수익률과 승률은 전략 채택 근거로 자동 승격하지 않는다.
- 사실·비밀정보·사진·캡션을 검수하고 사용자가 대표 승인한다.
- 1차는 클립보드 복사·HTML/Markdown 내보내기와 티스토리 에디터 수동 발행이다.
- 브라우저 화면 자동화는 세션·에디터 변경에 취약하고 공식 API가 아니므로 마지막 선택 기능으로만 검토한다.
- 자동화하더라도 최종 `공개 발행` 클릭은 별도 명시 승인을 요구하고 중복 발행 방지 ID를 기록한다.

공식 근거:

- https://notice.tistory.com/2664
